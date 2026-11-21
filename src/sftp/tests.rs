use super::*;
use crate::smart_completion::{CompletionSessionKey, CompletionState};

#[test]
fn upload_batch_debug_only_reports_item_count() {
    let command = SftpCommand::UploadBatch {
        uploads: vec![SftpUploadRequest {
            transfer_id: "private-transfer-id".into(),
            local_path: "/home/user/private.txt".into(),
            remote_path: "/srv/private.txt".into(),
        }],
    };

    let debug = format!("{command:?}");
    assert!(debug.contains("uploads: 1"));
    assert!(!debug.contains("private"));
    assert!(!debug.contains("/home"));
    assert!(!debug.contains("/srv"));
}

#[test]
fn upload_batch_send_failure_does_not_deliver_a_partial_batch() {
    let (handle, receiver) = test_handle();
    drop(receiver);
    let result = handle.send(SftpCommand::UploadBatch {
        uploads: vec![
            SftpUploadRequest {
                transfer_id: "first".into(),
                local_path: "/tmp/first".into(),
                remote_path: "/srv/first".into(),
            },
            SftpUploadRequest {
                transfer_id: "second".into(),
                local_path: "/tmp/second".into(),
                remote_path: "/srv/second".into(),
            },
        ],
    });

    assert_eq!(result.unwrap_err(), "SFTP worker 已停止");
}

fn history_request(session: CompletionSessionKey) -> crate::smart_completion::HistoryLoadRequest {
    CompletionState::new(session).mark_history_loading()
}

#[derive(Default)]
struct FakeRemoteCandidateOps {
    calls: Vec<String>,
    open_flags: Option<ssh2::OpenFlags>,
    open_mode: Option<i32>,
    fail_write: bool,
    fail_rename: bool,
}

impl RemoteCandidateOps for FakeRemoteCandidateOps {
    fn open(&mut self, path: &Path, flags: ssh2::OpenFlags, mode: i32) -> Result<(), String> {
        self.calls.push(format!("open:{}", path.display()));
        self.open_flags = Some(flags);
        self.open_mode = Some(mode);
        Ok(())
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.calls.push(format!("write:{}", bytes.len()));
        (!self.fail_write)
            .then_some(())
            .ok_or_else(|| "write failed".to_string())
    }

    fn close(&mut self) -> Result<(), String> {
        self.calls.push("close".into());
        Ok(())
    }

    fn rename(&mut self, from: &Path, to: &Path) -> Result<(), String> {
        self.calls
            .push(format!("rename:{}:{}", from.display(), to.display()));
        (!self.fail_rename)
            .then_some(())
            .ok_or_else(|| "rename failed".to_string())
    }

    fn unlink(&mut self, path: &Path) -> Result<(), String> {
        self.calls.push(format!("unlink:{}", path.display()));
        Ok(())
    }
}

#[test]
fn remote_tail_start_is_bounded() {
    assert_eq!(history_tail_start(100, 20), 80);
    assert_eq!(history_tail_start(10, 20), 0);
}

#[test]
fn remote_tail_reader_stays_bounded_if_file_grows_after_stat() {
    let mut source = std::io::Cursor::new(b"partial\nkept\ngrowth-after-stat");

    let bytes = read_bounded_history_tail(&mut source, 13, 10).unwrap();

    assert_eq!(bytes, b"kept\n");
    assert_eq!(source.position(), 13);
}

#[test]
fn candidate_temporary_path_stays_beside_target() {
    assert_eq!(
        candidate_temporary_path("/tmp/liteterm.candidate", 42).unwrap(),
        "/tmp/.liteterm.candidate.42.tmp"
    );
    assert!(candidate_temporary_path("/", 1).is_err());
}

#[test]
fn completion_event_keeps_session_and_request_identity() {
    let session = CompletionSessionKey::new_for_test(3, "token");
    let event = SftpEvent::CompletionCandidateWritten {
        tab_id: "tab".into(),
        session: session.clone(),
        request_id: 9,
        result: Ok(()),
    };
    assert_eq!(event.completion_session(), Some(&session));
}

#[test]
fn completion_history_event_keeps_requested_path_identity() {
    let session = CompletionSessionKey::new_for_test(3, "token");
    let request = history_request(session.clone());
    let event = SftpEvent::CompletionHistoryRead {
        tab_id: "tab".into(),
        session,
        request: request.clone(),
        path: "/tmp/history-a".into(),
        result: Ok(Vec::new()),
    };

    let SftpEvent::CompletionHistoryRead {
        path,
        request: event_request,
        ..
    } = event
    else {
        panic!("expected completion history event");
    };
    assert_eq!(path, "/tmp/history-a");
    assert_eq!(event_request, request);
}

#[test]
fn completion_history_command_keeps_load_request_identity() {
    let session = CompletionSessionKey::new_for_test(3, "secret-token");
    let request = history_request(session.clone());
    let command = SftpCommand::ReadCompletionHistory {
        session,
        request: request.clone(),
        path: "/tmp/history-a".into(),
        max_bytes: 1024,
    };

    let SftpCommand::ReadCompletionHistory {
        request: command_request,
        ..
    } = command
    else {
        panic!("expected history read command");
    };
    assert_eq!(command_request, request);
}

#[test]
fn completion_worker_io_gate_requires_current_session() {
    let worker_session = CompletionSessionKey::new_for_test(3, "current");
    let old_generation = CompletionSessionKey::new_for_test(2, "current");
    let stale_token = CompletionSessionKey::new_for_test(3, "stale");

    assert!(completion_command_session_is_current(
        &worker_session,
        &worker_session
    ));
    assert!(!completion_command_session_is_current(
        &worker_session,
        &old_generation
    ));
    assert!(!completion_command_session_is_current(
        &worker_session,
        &stale_token
    ));
}

#[test]
fn completion_worker_io_gate_executes_side_effect_only_for_current_session() {
    let worker_session = CompletionSessionKey::new_for_test(3, "current");
    let stale_session = CompletionSessionKey::new_for_test(3, "stale");
    let calls = std::cell::Cell::new(0);

    let stale_result = with_current_completion_session(&worker_session, &stale_session, || {
        calls.set(calls.get() + 1);
        Ok::<_, String>("stale")
    });
    assert!(stale_result.is_err());
    assert_eq!(calls.get(), 0);

    let current_result = with_current_completion_session(&worker_session, &worker_session, || {
        calls.set(calls.get() + 1);
        Ok::<_, String>("current")
    });
    assert_eq!(current_result.unwrap(), "current");
    assert_eq!(calls.get(), 1);
}

#[test]
fn completion_command_debug_redacts_candidate_bytes_and_session_token() {
    let bytes = b"password=remote-secret".to_vec();
    let command = SftpCommand::WriteCompletionCandidate {
        session: CompletionSessionKey::new_for_test(3, "secret-token"),
        request_id: 9,
        path: "/tmp/candidate".into(),
        bytes: bytes.clone(),
    };

    let debug = format!("{command:?}");

    assert!(debug.contains("WriteCompletionCandidate"));
    assert!(!debug.contains("secret-token"));
    assert!(!debug.contains(&format!("{bytes:?}")));
    assert!(!debug.contains("remote-secret"));
}

#[test]
fn worker_event_debug_redacts_remote_history_bytes_and_session_token() {
    let bytes = b"export PASSWORD=history-secret".to_vec();
    let session = CompletionSessionKey::new_for_test(3, "secret-token");
    let request = history_request(session.clone());
    let (worker, _commands) = test_handle();
    let event = SftpWorkerEvent {
        worker_id: worker.id(),
        tab_id: "tab".into(),
        pane_id: "pane".into(),
        session: session.clone(),
        event: SftpEvent::CompletionHistoryRead {
            tab_id: "tab".into(),
            session,
            request,
            path: "/tmp/history".into(),
            result: Ok(bytes.clone()),
        },
    };

    let debug = format!("{event:?}");

    assert!(debug.contains("CompletionHistoryRead"));
    assert!(!debug.contains("secret-token"));
    assert!(!debug.contains(&format!("{bytes:?}")));
    assert!(!debug.contains("history-secret"));
}

#[test]
fn test_worker_handles_have_unique_nonzero_ids() {
    let (first, _first_commands) = test_handle();
    let (second, _second_commands) = test_handle();

    assert_ne!(first.id(), 0);
    assert_ne!(second.id(), 0);
    assert_ne!(first.id(), second.id());
}

#[test]
fn sftp_event_debug_redacts_remote_history_bytes_and_session_token() {
    let bytes = b"export PASSWORD=event-secret".to_vec();
    let session = CompletionSessionKey::new_for_test(3, "secret-token");
    let event = SftpEvent::CompletionHistoryRead {
        tab_id: "tab".into(),
        request: history_request(session.clone()),
        session,
        path: "/tmp/history".into(),
        result: Ok(bytes.clone()),
    };

    let debug = format!("{event:?}");

    assert!(debug.contains("CompletionHistoryRead"));
    assert!(!debug.contains("secret-token"));
    assert!(!debug.contains(&format!("{bytes:?}")));
    assert!(!debug.contains("event-secret"));
}

#[test]
fn remote_candidate_creation_is_exclusive_private_and_not_truncating() {
    let flags = remote_candidate_create_flags();
    assert!(flags.contains(ssh2::OpenFlags::WRITE));
    assert!(flags.contains(ssh2::OpenFlags::CREATE));
    assert!(flags.contains(ssh2::OpenFlags::EXCLUSIVE));
    assert!(!flags.contains(ssh2::OpenFlags::TRUNCATE));
    assert_eq!(REMOTE_CANDIDATE_MODE, 0o600);
}

#[test]
fn remote_candidate_algorithm_opens_writes_closes_then_renames() {
    let mut ops = FakeRemoteCandidateOps::default();

    write_remote_candidate_with_ops(&mut ops, "/tmp/liteterm.candidate", 42, b"git status")
        .unwrap();

    assert_eq!(
        ops.calls,
        [
            "open:/tmp/.liteterm.candidate.42.tmp",
            "write:10",
            "close",
            "rename:/tmp/.liteterm.candidate.42.tmp:/tmp/liteterm.candidate",
        ]
    );
    let flags = ops.open_flags.unwrap();
    assert!(flags.contains(ssh2::OpenFlags::WRITE));
    assert!(flags.contains(ssh2::OpenFlags::CREATE));
    assert!(flags.contains(ssh2::OpenFlags::EXCLUSIVE));
    assert!(!flags.contains(ssh2::OpenFlags::TRUNCATE));
    assert_eq!(ops.open_mode, Some(0o600));
}

#[test]
fn remote_candidate_failures_unlink_only_their_own_temporary_file() {
    let expected_temporary = "unlink:/tmp/.liteterm.candidate.42.tmp";

    let mut write_failure = FakeRemoteCandidateOps {
        fail_write: true,
        ..Default::default()
    };
    assert!(write_remote_candidate_with_ops(
        &mut write_failure,
        "/tmp/liteterm.candidate",
        42,
        b"git status",
    )
    .is_err());
    assert_eq!(
        write_failure.calls,
        [
            "open:/tmp/.liteterm.candidate.42.tmp",
            "write:10",
            expected_temporary,
        ]
    );

    let mut rename_failure = FakeRemoteCandidateOps {
        fail_rename: true,
        ..Default::default()
    };
    assert!(write_remote_candidate_with_ops(
        &mut rename_failure,
        "/tmp/liteterm.candidate",
        42,
        b"git status",
    )
    .is_err());
    assert_eq!(
        rename_failure.calls,
        [
            "open:/tmp/.liteterm.candidate.42.tmp",
            "write:10",
            "close",
            "rename:/tmp/.liteterm.candidate.42.tmp:/tmp/liteterm.candidate",
            expected_temporary,
        ]
    );
    assert!(!rename_failure
        .calls
        .contains(&"unlink:/tmp/liteterm.candidate".to_string()));
}

#[test]
fn remote_candidate_byte_writer_rejects_utf8_c1_control_before_open() {
    let mut ops = FakeRemoteCandidateOps::default();

    assert!(write_remote_candidate_with_ops(
        &mut ops,
        "/tmp/liteterm.candidate",
        42,
        "x\u{0085}y".as_bytes(),
    )
    .is_err());
    assert!(ops.calls.is_empty());
}

#[test]
fn path_helpers_handle_root_and_nested_paths() {
    assert_eq!(join_path("/", "etc"), "/etc");
    assert_eq!(join_path("/var/log/", "app.log"), "/var/log/app.log");
    assert_eq!(parent_path("/var/log"), "/var");
    assert_eq!(parent_path("/"), "/");
}

#[test]
fn remote_plan_path_joins_each_relative_component() {
    assert_eq!(
        remote_plan_path("/srv/release", Path::new("assets/icon.png")).unwrap(),
        "/srv/release/assets/icon.png"
    );
    assert_eq!(
        remote_plan_path("/srv/release/", Path::new("README.md")).unwrap(),
        "/srv/release/README.md"
    );
    assert_eq!(
        remote_plan_path("/", Path::new("opt/app")).unwrap(),
        "/opt/app"
    );
    assert!(remote_plan_path("/srv/release", Path::new("../escape")).is_err());
}

#[test]
fn local_listing_puts_directories_before_files() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("z.txt"), b"123").unwrap();
    std::fs::create_dir(temp.path().join("a-dir")).unwrap();

    let entries = list_local_dir(temp.path().to_str().unwrap()).unwrap();
    assert_eq!(entries[0].name, "a-dir");
    assert!(entries[0].is_dir);
    assert_eq!(entries[1].name, "z.txt");
    assert_eq!(entries[1].size, 3);
}

#[test]
fn local_rename_and_recursive_delete_match_main_behavior() {
    let temp = tempfile::tempdir().unwrap();
    let old_path = temp.path().join("old.txt");
    let new_path = temp.path().join("new.txt");
    std::fs::write(&old_path, b"content").unwrap();

    rename_local(&old_path, &new_path).unwrap();
    assert!(!old_path.exists());
    assert_eq!(std::fs::read(&new_path).unwrap(), b"content");

    let directory = temp.path().join("nested");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("child.txt"), b"child").unwrap();
    delete_local(&directory, true).unwrap();
    assert!(!directory.exists());

    delete_local(&new_path, false).unwrap();
    assert!(!new_path.exists());
}

#[test]
fn local_create_makes_empty_file_and_directory_without_overwriting() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("new.txt");
    let directory = temp.path().join("new-dir");

    create_local(&file, CreateKind::File).unwrap();
    create_local(&directory, CreateKind::Directory).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"");
    assert!(directory.is_dir());
    assert!(create_local(&file, CreateKind::File).is_err());
    assert!(create_local(&directory, CreateKind::Directory).is_err());
}

#[test]
fn remote_file_creation_flags_are_exclusive() {
    let flags = remote_file_create_flags();
    assert!(flags.contains(ssh2::OpenFlags::WRITE));
    assert!(flags.contains(ssh2::OpenFlags::CREATE));
    assert!(flags.contains(ssh2::OpenFlags::EXCLUSIVE));
    assert!(!flags.contains(ssh2::OpenFlags::TRUNCATE));
}

#[test]
fn local_upload_plan_keeps_nested_and_empty_directories_and_totals_files() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("empty")).unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("root.txt"), b"abc").unwrap();
    std::fs::write(temp.path().join("src/main.rs"), b"rust").unwrap();

    let plan = build_local_upload_plan(temp.path()).unwrap();
    let entries = plan
        .entries
        .iter()
        .map(|entry| {
            (
                entry.relative.to_string_lossy().into_owned(),
                entry.is_dir,
                entry.size,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        entries,
        vec![
            ("empty".into(), true, 0),
            ("root.txt".into(), false, 3),
            ("src".into(), true, 0),
            ("src/main.rs".into(), false, 4),
        ]
    );
    assert_eq!(plan.total_bytes, 7);
}

#[cfg(unix)]
#[test]
fn local_upload_plan_rejects_symbolic_links() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("target.txt"), b"target").unwrap();
    symlink("target.txt", temp.path().join("link.txt")).unwrap();

    let error = build_local_upload_plan(temp.path()).unwrap_err();
    assert!(error.contains("不支持上传符号链接"), "{error}");
}
