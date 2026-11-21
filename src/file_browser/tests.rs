use super::*;

fn entry(name: &str) -> FileEntry {
    FileEntry {
        name: name.into(),
        path: format!("/{name}"),
        is_dir: false,
        size: 1,
        mtime: 0,
    }
}

#[test]
fn stale_list_result_does_not_replace_newer_request() {
    let mut state = FileBrowserState::new("/tmp".into());
    state.local.request_id = 2;
    state.apply_event(&SftpEvent::Listed {
        tab_id: "tab".into(),
        request_id: 1,
        side: FileSide::Local,
        path: "/old".into(),
        result: Ok(vec![entry("old")]),
    });
    assert!(state.local.entries.is_empty());
    assert_eq!(state.local.path, "/tmp");
}

#[test]
fn ready_event_sets_remote_home_without_touching_local_path() {
    let mut state = FileBrowserState::new("/tmp".into());
    state.apply_event(&SftpEvent::Ready {
        tab_id: "tab".into(),
        home: "/home/deploy".into(),
    });
    assert!(state.ready);
    assert_eq!(state.remote.path, "/home/deploy");
    assert_eq!(state.local.path, "/tmp");
}

#[test]
fn failed_list_keeps_last_successful_path_and_entries() {
    let mut state = FileBrowserState::new("/tmp".into());
    state.local.entries = vec![entry("visible.txt")];
    let request_id = state.next_request(FileSide::Local, "/root".into());
    state.apply_event(&SftpEvent::Listed {
        tab_id: "tab".into(),
        request_id,
        side: FileSide::Local,
        path: "/root".into(),
        result: Err("权限不足".into()),
    });

    assert_eq!(state.local.path, "/tmp");
    assert_eq!(state.local.input, "/tmp");
    assert_eq!(state.local.entries[0].name, "visible.txt");
}

#[test]
fn new_transfer_clears_old_failures_and_preserves_filename() {
    let mut state = FileBrowserState::new("/tmp".into());
    state.start_transfer("old".into(), "old.bin".into(), TransferDirection::Upload);
    state.transfers[0].error = Some("failed".into());
    state.start_transfer(
        "new".into(),
        "release.tar".into(),
        TransferDirection::Download,
    );
    assert_eq!(state.transfers.len(), 1);
    assert_eq!(state.transfers[0].filename, "release.tar");
}

#[test]
fn mutation_result_updates_the_matching_pane_error() {
    let mut state = FileBrowserState::new("/tmp".into());
    state.remote.error = Some("旧错误".into());
    state.apply_event(&SftpEvent::MutationFinished {
        tab_id: "tab".into(),
        side: FileSide::Remote,
        operation: crate::sftp::FileOperation::Rename,
        result: Ok(()),
    });
    assert_eq!(state.remote.error, None);

    state.apply_event(&SftpEvent::MutationFinished {
        tab_id: "tab".into(),
        side: FileSide::Remote,
        operation: crate::sftp::FileOperation::Delete,
        result: Err("目录非空".into()),
    });
    assert_eq!(state.remote.error.as_deref(), Some("删除失败: 目录非空"));
    assert_eq!(state.local.error, None);

    state.apply_event(&SftpEvent::MutationFinished {
        tab_id: "tab".into(),
        side: FileSide::Local,
        operation: crate::sftp::FileOperation::Create,
        result: Err("目标已存在".into()),
    });
    assert_eq!(state.local.error.as_deref(), Some("创建失败: 目标已存在"));
}
