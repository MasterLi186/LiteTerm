    #[test]
    fn real_local_bash_pty_emits_authenticated_prompt_without_modifying_bashrc() {
        use portable_pty::{native_pty_system, CommandBuilder, PtySize};
        use std::io::Read;
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        assert!(std::path::Path::new("/bin/bash").exists());
        let home = tempfile::tempdir().unwrap();
        let bashrc = home.path().join(".bashrc");
        let history = home.path().join(".bash_history");
        let inputrc = home.path().join(".inputrc");
        let bash_env = home.path().join(".bash_env");
        let observed_environment = home.path().join("observed-environment");
        let isolated_original = b"isolated-history-sentinel\n";
        let original = b"PS1='integration$ '\n\
printf '%s\\n%s\\n%s\\n%s\\n' \"$HOME\" \"$HISTFILE\" \"$INPUTRC\" \"$BASH_ENV\" \
> \"$HOME/observed-environment\"\n";
        std::fs::write(&bashrc, original).unwrap();
        std::fs::write(&history, isolated_original).unwrap();

        let session = CompletionSessionKey::new_for_test(11, "abcdef12");
        let runtime = LocalBashRuntime::create(session.clone()).unwrap();
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 8,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new("/bin/bash");
        command.arg("--rcfile");
        command.arg(runtime.rc_path());
        command.arg("-i");
        command.env("HOME", home.path());
        command.env("HISTFILE", &history);
        command.env("INPUTRC", &inputrc);
        command.env("BASH_ENV", &bash_env);
        command.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut process = BashPtyGuard::new(child);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let (output_tx, output_rx) = mpsc::channel();
        let (reader_done_tx, reader_done_rx) = mpsc::channel();
        let reader_thread = std::thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            while let Ok(length) = reader.read(&mut buffer) {
                if length == 0 || output_tx.send(buffer[..length].to_vec()).is_err() {
                    break;
                }
            }
            let _ = reader_done_tx.send(());
        });
        process.attach_reader_and_writer(writer, pair.master, reader_thread, reader_done_rx);

        let mut decoder = MarkerDecoder::new(session);
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut prompt_seen = false;
        while Instant::now() < deadline {
            if let Ok(bytes) = output_rx.recv_timeout(Duration::from_millis(100)) {
                prompt_seen |= decoder
                    .scan(&bytes)
                    .iter()
                    .any(|boundary| boundary.kind == MarkerKind::Prompt);
                if prompt_seen {
                    break;
                }
            }
        }
        process.finish().unwrap();

        assert!(prompt_seen, "真实 Bash PTY 应输出认证提示符标记");
        assert_eq!(std::fs::read(&bashrc).unwrap(), original);
        assert!(std::fs::read(&history)
            .unwrap()
            .starts_with(isolated_original));
        assert_eq!(
            std::fs::read_to_string(observed_environment)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            [
                home.path().to_str().unwrap(),
                history.to_str().unwrap(),
                inputrc.to_str().unwrap(),
                bash_env.to_str().unwrap(),
            ]
        );
    }

    #[test]
    fn only_an_absolute_shell_safe_bash_path_enables_integration() {
        assert!(is_safe_remote_bash_path("/bin/bash"));
        assert!(is_safe_remote_bash_path("/usr/local/bin/bash"));
        assert!(!is_safe_remote_bash_path("bash"));
        assert!(!is_safe_remote_bash_path("/usr/bin/fish"));
        assert!(!is_safe_remote_bash_path("bash -l"));
        assert!(!is_safe_remote_bash_path("/bin/'bash'"));
        assert!(!is_safe_remote_bash_path("/bin/ba\nsh"));
    }

    #[test]
    fn remote_paths_are_token_scoped_and_shell_safe() {
        let paths = RemoteBashPaths::new(&CompletionSessionKey::new_for_test(7, "abcdef"));

        assert_eq!(paths.rc, "/tmp/liteterm-native-abcdef-7.rc");
        assert_eq!(paths.candidate, "/tmp/liteterm-native-abcdef-7.candidate");
        let command = paths.launch_command("/bin/bash");
        assert!(command.contains("--rcfile"));
        assert!(!command.contains('\n'));
    }

    #[test]
    fn remote_runtime_and_paths_debug_redact_token_scoped_paths() {
        let session = CompletionSessionKey::new_for_test(7, "secret-token");
        let paths = RemoteBashPaths::new(&session);
        let runtime = RemoteBashRuntime {
            session,
            bash_path: "/bin/bash".into(),
            rc_path: paths.rc.clone(),
            candidate_path: paths.candidate.clone(),
            widget_sequence: "\x1b[777;42~".into(),
            snapshot_sequence: "\x1b[778;stored-value~".into(),
        };

        for debug in [format!("{paths:?}"), format!("{runtime:?}")] {
            assert!(!debug.contains("secret-token"));
            assert!(!debug.contains(&paths.rc));
            assert!(!debug.contains(&paths.candidate));
            assert!(!debug.contains("\x1b[777;42~"));
            assert!(!debug.contains("\x1b[778;stored-value~"));
        }
        assert!(format!("{runtime:?}").contains("snapshot_sequence: \"<redacted>\""));
        assert_eq!(runtime.snapshot_sequence(), "\x1b[778;stored-value~");
    }

    fn prompt_body() -> String {
        format!("777;LiteTerm;{TOKEN};{GENERATION};P")
    }

    fn history_body(path: &[u8]) -> String {
        format!(
            "777;LiteTerm;{TOKEN};{GENERATION};H;{}",
            URL_SAFE_NO_PAD.encode(path)
        )
    }

    fn snapshot_body(line: &[u8], point: impl std::fmt::Display) -> String {
        format!(
            "777;LiteTerm;{TOKEN};{GENERATION};I;{point};{}",
            URL_SAFE_NO_PAD.encode(line)
        )
    }

    fn run_bash_with_locale(
        runtime: &LocalBashRuntime,
        home: &Path,
        script: &str,
        locale: &str,
    ) -> Output {
        Command::new("bash")
            .args(["--noprofile", "--norc", "-c", script, "liteterm-test"])
            .arg(runtime.rc_path())
            .env("HOME", home)
            .env("LC_ALL", locale)
            .output()
            .unwrap()
    }

    fn run_bash(runtime: &LocalBashRuntime, home: &Path, script: &str) -> Output {
        run_bash_with_locale(runtime, home, script, "C")
    }

    #[test]
    fn bash_path_requires_bash_basename() {
        assert!(is_bash_path("bash"));
        assert!(is_bash_path("/bin/bash"));
        assert!(is_bash_path("/usr/local/bin/bash"));
        assert!(!is_bash_path("/bin/sh"));
        assert!(is_bash_path("/tmp/bash/"));
        assert!(is_bash_path("bash.exe"));
    }

    #[cfg(not(windows))]
    #[test]
    fn local_history_path_requires_an_absolute_host_path() {
        assert_eq!(local_history_path("relative/.bash_history"), None);
        assert_eq!(
            local_history_path("/tmp/.bash_history").as_deref(),
            Some(std::path::Path::new("/tmp/.bash_history"))
        );
        assert_eq!(local_history_path("/tmp/bad\npath"), None);
    }

    #[cfg(windows)]
    #[test]
    fn local_history_path_converts_git_bash_msys_drive_paths() {
        assert_eq!(local_history_path("relative/.bash_history"), None);
        assert_eq!(local_history_path("C:/tmp/bad\npath"), None);
        assert_eq!(
            local_history_path("/c/Users/lfl/.bash_history").as_deref(),
            Some(std::path::Path::new(r"C:\Users\lfl\.bash_history")),
        );
        assert_eq!(
            local_history_path("C:/Users/lfl/.bash_history").as_deref(),
            Some(std::path::Path::new(r"C:\Users\lfl\.bash_history")),
        );
    }

    #[test]
    fn widget_sequence_is_stable_and_token_specific() {
        let first = CompletionSessionKey::new_for_test(1, "01234567aaaaaaaa");
        let same_prefix = CompletionSessionKey::new_for_test(99, "01234567bbbbbbbb");
        let different = CompletionSessionKey::new_for_test(1, "89abcdefaaaaaaaa");
        let malformed = CompletionSessionKey::new_for_test(1, "not-hex");

        assert_eq!(widget_sequence(&first), "\x1b[777;19088743~");
        assert_eq!(widget_sequence(&same_prefix), widget_sequence(&first));
        assert_ne!(widget_sequence(&different), widget_sequence(&first));
        assert_eq!(widget_sequence(&malformed), "\x1b[777;777~");
    }

    #[test]
    fn fill_and_snapshot_sequences_are_distinct_token_specific_and_non_executing() {
        let first = CompletionSessionKey::new_for_test(1, "01234567aaaaaaaa");
        let different = CompletionSessionKey::new_for_test(1, "89abcdefaaaaaaaa");
        let fill = widget_sequence(&first);
        let snapshot = snapshot_sequence(&first);

        assert_ne!(snapshot, fill);
        assert_ne!(snapshot, snapshot_sequence(&different));
        assert!(!snapshot.bytes().any(|byte| matches!(byte, b'\r' | b'\n')));
    }

    #[test]
    fn readline_literal_only_escapes_escape_bytes() {
        assert_eq!(readline_literal("\x1b[777;42~\\\"'$"), "\\e[777;42~\\\"'$");
    }

    #[test]
    fn bash_rc_sources_user_rc_preserves_prompt_command_and_installs_widget() {
        let session = CompletionSessionKey::new_for_test(42, TOKEN);
        let sequence = widget_sequence(&session);
        let snapshot = snapshot_sequence(&session);
        let rc = build_bash_rc(
            &session,
            std::path::Path::new("/tmp/candidate"),
            &sequence,
            &snapshot,
        );

        assert!(rc.contains("source \"$HOME/.bashrc\""));
        assert!(rc.contains("declare -p PROMPT_COMMAND"));
        assert!(rc.contains("PROMPT_COMMAND+=("));
        assert!(rc.contains("PROMPT_COMMAND=\"${PROMPT_COMMAND%;};__liteterm_prompt_hook\""));
        for keymap in ["emacs-standard", "vi-insert", "vi-command"] {
            assert_eq!(rc.matches(&format!("bind -m {keymap} -x")).count(), 2);
        }
        assert!(rc.contains("READLINE_LINE=$(<\"$__liteterm_candidate\")"));
        assert!(rc.contains("READLINE_POINT=${#READLINE_LINE}"));
        assert!(rc.contains(";I;%s;%s\\a' \"$__liteterm_input_point\""));
        assert!(!rc.contains("eval"));
        assert!(!rc.contains("accept-line"));
        assert!(rc.contains(&format!("777;LiteTerm;{TOKEN};42;P")));
        assert!(rc.contains("${HISTFILE:-\"$HOME/.bash_history\"}"));
        assert!(rc.contains("tr '+/' '-_'"));
        assert!(rc.contains("tr -d '=\\n'"));
    }

    #[test]
    fn bash_rc_keeps_prompt_marker_as_readline_nonprinting_literal() {
        let session = CompletionSessionKey::new_for_test(42, TOKEN);
        let rc = build_bash_rc(
            &session,
            std::path::Path::new("/tmp/candidate"),
            &widget_sequence(&session),
            &snapshot_sequence(&session),
        );
        let marker =
            format!("readonly __liteterm_prompt_marker='\\[\\e]777;LiteTerm;{TOKEN};42;P\\a\\]'");

        assert!(rc.contains(&marker));
        assert!(!rc.contains("readonly __liteterm_prompt_marker=$'\\e"));
        assert!(rc.contains("*) PS1=\"${PS1-}${__liteterm_prompt_marker}\" ;;"));
    }

    #[test]
    fn bash_rc_guards_history_marker_on_both_encoder_commands() {
        let session = CompletionSessionKey::new_for_test(42, TOKEN);
        let rc = build_bash_rc(
            &session,
            std::path::Path::new("/tmp/candidate"),
            &widget_sequence(&session),
            &snapshot_sequence(&session),
        );
        let guard = "if command -v base64 >/dev/null 2>&1 && command -v tr >/dev/null 2>&1; then";
        let prompt_hook_start = rc
            .find("__liteterm_prompt_hook()")
            .expect("prompt hook declaration");
        let guarded_start = rc[prompt_hook_start..]
            .find(guard)
            .map(|offset| prompt_hook_start + offset)
            .expect("history encoder guard");
        let guarded_end = rc[guarded_start..]
            .find("\n        fi")
            .map(|offset| guarded_start + offset)
            .expect("history encoder guard end");
        let guarded_block = &rc[guarded_start..guarded_end];

        assert!(guarded_block.contains(";H;%s\\a'"));
        assert!(guarded_block.contains("__liteterm_history_sent=1"));
    }

    #[test]
    fn generated_widget_is_bound_in_real_bash_vi_insert_keymap() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join(".bashrc"), "").unwrap();

        let output = run_bash(
            &runtime,
            home.path(),
            r#"
source "$1"
__liteterm_install_bindings
bind -m vi-insert -X
"#,
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let expected = format!(
            "\"{}\": \"__liteterm_fill\"",
            readline_literal(runtime.widget_sequence())
        );

        assert!(output.status.success(), "stderr: {stderr}");
        assert!(!stderr.contains("invalid keymap"), "stderr: {stderr}");
        assert!(stdout.contains(&expected), "stdout: {stdout}");
    }

    #[test]
    fn generated_snapshot_widget_is_bound_in_all_keymaps_and_does_not_mutate_or_execute() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join(".bashrc"), "").unwrap();
        let rc = fs::read_to_string(runtime.rc_path()).unwrap();
        let start = rc.find("__liteterm_snapshot()").unwrap();
        let end = rc[start..].find("\n}\n").unwrap() + start;
        let snapshot_widget = &rc[start..end];

        assert!(!snapshot_widget.contains("READLINE_LINE="));
        assert!(!snapshot_widget.contains("READLINE_POINT="));
        assert!(!snapshot_widget.contains("printf '\\r"));
        assert!(!snapshot_widget.contains("printf '\\n"));

        let output = run_bash(
            &runtime,
            home.path(),
            r#"
source "$1"
__liteterm_install_bindings
for __test_map in emacs-standard vi-insert vi-command; do
    builtin bind -m "$__test_map" -X
done
"#,
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let expected = format!(
            "\"{}\": \"__liteterm_snapshot\"",
            readline_literal(runtime.snapshot_sequence())
        );

        assert!(output.status.success(), "stderr: {stderr}");
        assert_eq!(stdout.matches(&expected).count(), 3, "stdout: {stdout}");


}

    #[test]
    fn generated_snapshot_widget_converts_utf8_character_point_to_byte_offset() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join(".bashrc"), "").unwrap();
        let output = run_bash_with_locale(
            &runtime,
            home.path(),
            r#"
source "$1"
READLINE_LINE='你好'
READLINE_POINT=2
__liteterm_snapshot
"#,
            "C.UTF-8",
        );
        let expected = bel_frame(snapshot_body("你好".as_bytes(), 6));
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(output.status.success(), "stderr: {stderr}");
        assert_eq!(output.stdout, expected);
        assert!(!output
            .stdout
            .iter()
            .any(|byte| matches!(byte, b'\r' | b'\n')));
        assert_eq!(
            MarkerDecoder::new(session()).scan(&output.stdout)[0].kind,
            MarkerKind::InputSnapshot {
                line: "你好".into(),
                point: 6,
            }
        );
    }

    #[test]
    fn generated_snapshot_widget_drops_oversized_readline_line_before_encoding() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join(".bashrc"), "").unwrap();
        let script = format!(
            r#"
source "$1"
printf -v READLINE_LINE '%*s' {} ''
READLINE_POINT=0
__liteterm_snapshot
"#,
            MAX_SNAPSHOT_INPUT_BYTES + 1
        );
        let output = run_bash_with_locale(&runtime, home.path(), &script, "C.UTF-8");
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(output.status.success(), "stderr: {stderr}");
        assert!(output.stdout.is_empty(), "stdout: {:?}", output.stdout);
    }

    #[test]
    fn generated_widget_uses_builtin_bind_when_user_rc_shadows_bind() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join(".bashrc"), "bind() { return 99; }\n").unwrap();

        let output = run_bash(
            &runtime,
            home.path(),
            r#"
source "$1"
__liteterm_install_bindings
builtin bind -m vi-insert -X
"#,
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let expected = format!(
            "\"{}\": \"__liteterm_fill\"",
            readline_literal(runtime.widget_sequence())
        );

        assert!(output.status.success(), "stderr: {stderr}");
        assert!(stdout.contains(&expected), "stdout: {stdout}");
    }

    #[test]
    fn prompt_hook_runs_after_user_array_hooks_and_restores_marker_once() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(
            home.path().join(".bashrc"),
            "PROMPT_COMMAND=('PS1=dynamic')\n",
        )
        .unwrap();

        let output = run_bash(
            &runtime,
            home.path(),
            r#"
source "$1"
for __test_prompt in 1 2; do
    for __test_command in "${PROMPT_COMMAND[@]}"; do
        eval "$__test_command"
    done
done
printf '%s' "$PS1"
"#,
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let marker = format!("\\[\\e]777;LiteTerm;{TOKEN};{GENERATION};P\\a\\]");

        assert!(output.status.success(), "stderr: {stderr}");
        assert!(stdout.ends_with(&marker), "stdout: {stdout:?}");
        assert_eq!(stdout.matches(&marker).count(), 1, "stdout: {stdout:?}");
    }

    #[test]
    fn sourcing_generated_rc_twice_keeps_readonly_values_and_history_state() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join(".bashrc"), "").unwrap();

        let output = run_bash(
            &runtime,
            home.path(),
            r#"
source "$1"
__liteterm_history_sent=1
source "$1"
printf '%s' "$__liteterm_history_sent"
"#,
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(output.status.success(), "stderr: {stderr}");
        assert!(!stderr.contains("readonly variable"), "stderr: {stderr}");
        assert_eq!(stdout, "1");
    }

    #[test]
    fn local_runtime_creates_private_files_and_atomically_updates_candidate() {
        let runtime = LocalBashRuntime::create(session()).unwrap();

        assert_eq!(
            fs::metadata(runtime.temp_dir())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(runtime.rc_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(runtime.candidate_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(fs::read_to_string(runtime.rc_path())
            .unwrap()
            .contains("READLINE_LINE=$(<\"$__liteterm_candidate\")"));
        assert_eq!(runtime.session(), &session());
        assert_eq!(runtime.widget_sequence(), widget_sequence(&session()));
        assert_eq!(runtime.snapshot_sequence(), snapshot_sequence(&session()));

        runtime.write_candidate("git status").unwrap();
        assert_eq!(
            fs::read_to_string(runtime.candidate_path()).unwrap(),
            "git status"
        );
        assert!(!runtime.candidate_path().with_extension("next").exists());
    }

    #[test]
    fn local_runtime_debug_redacts_private_paths_and_sequences() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        let debug = format!("{runtime:?}");

        assert!(!debug.contains(runtime.rc_path().to_string_lossy().as_ref()));
        assert!(!debug.contains(runtime.candidate_path().to_string_lossy().as_ref()));
        assert!(!debug.contains(runtime.widget_sequence()));
        assert!(!debug.contains(runtime.snapshot_sequence()));
        assert!(debug.contains("widget_sequence: \"<redacted>\""));
        assert!(debug.contains("snapshot_sequence: \"<redacted>\""));
    }

    #[test]
    fn invalid_candidate_does_not_replace_existing_candidate() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        runtime.write_candidate("cargo test").unwrap();

        for invalid in ["", "line\nbreak", "tab\tvalue", "\u{7f}"] {
            assert!(runtime.write_candidate(invalid).is_err());
            assert_eq!(
                fs::read_to_string(runtime.candidate_path()).unwrap(),
                "cargo test"
            );
            assert!(!runtime.candidate_path().with_extension("next").exists());
        }
    }

    #[test]
    fn local_runtime_rejects_unicode_c1_control_candidate() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        runtime.write_candidate("cargo test").unwrap();

        assert!(runtime.write_candidate("x\u{0085}y").is_err());
        assert_eq!(
            fs::read_to_string(runtime.candidate_path()).unwrap(),
            "cargo test"
        );
    }

    #[test]
    fn concurrent_local_candidate_writers_all_succeed_without_mixed_content_or_temp_files() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("candidate");
        fs::write(&target, b"initial").unwrap();
        let writers = 12;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(writers));
        let candidates = (0..writers)
            .map(|index| format!("candidate-{index}-{}", "x".repeat(512 * 1024)))
            .collect::<Vec<_>>();
        let handles = candidates
            .iter()
            .cloned()
            .map(|candidate| {
                let barrier = barrier.clone();
                let target = target.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    write_local_candidate_atomic(&target, candidate.as_bytes())
                })
            })
            .collect::<Vec<_>>();

        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert!(results.iter().all(Result::is_ok), "{results:?}");

        let final_bytes = fs::read(&target).unwrap();
        assert!(candidates
            .iter()
            .any(|candidate| candidate.as_bytes() == final_bytes));
        let remaining_names = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(remaining_names, [std::ffi::OsString::from("candidate")]);
    }

    #[test]
    fn local_candidate_temporary_paths_are_unique_siblings() {
        let target = Path::new("/tmp/liteterm/candidate");
        let first = local_candidate_temporary_path(target, 10).unwrap();
        let second = local_candidate_temporary_path(target, 11).unwrap();

        assert_eq!(first.parent(), target.parent());
        assert_eq!(second.parent(), target.parent());
        assert_ne!(first, second);
        assert!(first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".candidate."));
        assert!(second
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".tmp"));
    }

    fn bel_frame(body: impl AsRef<[u8]>) -> Vec<u8> {
        let mut frame = b"\x1b]".to_vec();
        frame.extend_from_slice(body.as_ref());
        frame.push(b'\x07');
        frame
    }

    fn st_frame(body: impl AsRef<[u8]>) -> Vec<u8> {
        let mut frame = b"\x1b]".to_vec();
        frame.extend_from_slice(body.as_ref());
        frame.extend_from_slice(b"\x1b\\");
        frame
    }

    #[test]
    fn bel_marker_reports_chunk_exclusive_end_offset() {
        let mut decoder = MarkerDecoder::new(session());
        let frame = bel_frame(prompt_body());
        let mut chunk = b"ordinary-before".to_vec();
        chunk.extend_from_slice(&frame);
        let expected_end = chunk.len();
        chunk.extend_from_slice(b"ordinary-after");

        assert_eq!(
            decoder.scan(&chunk),
            vec![MarkerBoundary {
                end_offset: expected_end,
                kind: MarkerKind::Prompt,
            }]
        );
    }

    #[test]
    fn st_split_across_chunks_decodes_absolute_history_path() {
        let mut decoder = MarkerDecoder::new(session());
        let path = b"/home/test/.bash_history";
        let frame = st_frame(history_body(path));
        let split = frame.len() - 1;
        let mut first = b"before".to_vec();
        first.extend_from_slice(&frame[..split]);

        assert!(decoder.scan(&first).is_empty());
        assert_eq!(
            decoder.scan(&[frame[split], b'a', b'f', b't', b'e', b'r']),
            vec![MarkerBoundary {
                end_offset: 1,
                kind: MarkerKind::HistoryPath("/home/test/.bash_history".to_owned()),
            }]
        );
    }

    #[test]
    fn input_snapshot_marker_decodes_line_and_cursor() {
        let mut decoder = MarkerDecoder::new(session());

        assert_eq!(
            decoder.scan(&bel_frame(snapshot_body(b"git status", 3))),
            vec![MarkerBoundary {
                end_offset: bel_frame(snapshot_body(b"git status", 3)).len(),
                kind: MarkerKind::InputSnapshot {
                    line: "git status".into(),
                    point: 3,
                },
            }]
        );
    }

    #[test]
    fn input_snapshot_rejects_malformed_point_payload_and_controls() {
        let mut decoder = MarkerDecoder::new(session());
        let invalid = [
            snapshot_body(b"git", "three"),
            format!("777;LiteTerm;{TOKEN};{GENERATION};I;3;Z2l0="),
            format!("777;LiteTerm;{TOKEN};{GENERATION};I;3;***"),
            snapshot_body(b"git\nstatus", 3),
        ];

        for body in invalid {
            assert!(decoder.scan(&bel_frame(body)).is_empty());
        }
    }

    #[test]
    fn input_snapshot_rejects_stale_authentication_and_invalid_utf8_points() {
        let mut decoder = MarkerDecoder::new(session());
        let stale_token = format!(
            "777;LiteTerm;stale-token;{GENERATION};I;3;{}",
            URL_SAFE_NO_PAD.encode("git")
        );
        let stale_generation = format!(
            "777;LiteTerm;{TOKEN};{};I;3;{}",
            GENERATION + 1,
            URL_SAFE_NO_PAD.encode("git")
        );

        for body in [
            stale_token,
            stale_generation,
            snapshot_body("你好".as_bytes(), 1),
            snapshot_body("你好".as_bytes(), 7),
        ] {
            assert!(decoder.scan(&bel_frame(body)).is_empty());
        }
    }

    #[test]
    fn input_snapshot_enforces_line_and_frame_bounds_then_recovers() {
        let mut decoder = MarkerDecoder::new(session());
        let too_long = snapshot_body(&vec![b'x'; MAX_SNAPSHOT_INPUT_BYTES + 1], 0);
        let oversized = bel_frame(vec![b'x'; MAX_OSC_FRAME + 1]);

        assert!(decoder.scan(&bel_frame(too_long)).is_empty());
        assert!(decoder.scan(&oversized).is_empty());
        assert_eq!(
            decoder.scan(&bel_frame(snapshot_body(b"ok", 2)))[0].kind,
            MarkerKind::InputSnapshot {
                line: "ok".into(),
                point: 2,
            }
        );
    }
