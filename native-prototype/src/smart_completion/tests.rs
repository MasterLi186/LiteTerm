use super::*;

struct TemporaryHistoryFile {
    path: std::path::PathBuf,
}

impl TemporaryHistoryFile {
    fn new(contents: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "liteterm-smart-completion-{}.history",
            uuid::Uuid::new_v4().simple()
        ));
        let file = Self { path };
        std::fs::write(&file.path, contents).unwrap();
        file
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryHistoryFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn timestamped_multiline_entries_are_dropped() {
    let data = b"#100\nls -la\n#101\nprintf 'a\nb'\n#102\ngit status\n";
    assert_eq!(parse_bash_history(data), ["git status", "ls -la"]);
}

#[test]
fn timestamped_history_ignores_lines_before_the_first_timestamp() {
    let data = b"garbage\n#100\nls\n";
    assert_eq!(parse_bash_history(data), ["ls"]);
}

#[test]
fn untimestamped_lines_are_independent_and_control_bytes_are_rejected() {
    let data = b"echo one\n\nprintf two\nbad\tcommand\n";
    assert_eq!(parse_bash_history(data), ["printf two", "echo one"]);
}

#[test]
fn ranking_is_strict_prefix_recent_first_deduplicated_and_capped_at_five() {
    let history = vec![
        "strace -o /tmp/liteterm-fish-child.strace fish".into(),
        "fish --help".into(),
        "fish --help".into(),
        "fish -c one".into(),
        "fish -c two".into(),
        "fish -c three".into(),
        "fish -c four".into(),
        "fish -c five".into(),
        "fish -c six".into(),
        "fish".into(),
    ];
    assert_eq!(
        rank_candidates(&history, "fish"),
        [
            "fish --help",
            "fish -c one",
            "fish -c two",
            "fish -c three",
            "fish -c four"
        ]
    );
}

#[test]
fn exact_prefix_is_excluded_and_results_are_capped() {
    let history = (0..20)
        .map(|index| format!("echo {index}"))
        .collect::<Vec<_>>();
    let candidates = rank_candidates(&history, "echo");
    assert_eq!(candidates.len(), MAX_CANDIDATES);
    assert!(!rank_candidates(&["echo".into()], "echo").contains(&"echo".into()));
}

#[test]
fn executed_command_moves_to_front_without_duplicates() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(7, "a"));
    state.replace_history(vec!["ls".into(), "pwd".into(), "ls".into()]);
    state.merge_executed("pwd");
    assert_eq!(state.history(), ["pwd", "ls"]);
}

#[test]
fn first_fill_does_not_merge_history_but_submission_does() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "x"));
    state.replace_history(vec!["git status".into()]);
    assert!(state.begin_fill(10, "git log"));
    assert_eq!(state.history(), ["git status"]);
    state.finish_fill(10);
    state.complete_submission(Some("stale terminal input"));
    assert_eq!(state.history(), ["git log", "git status"]);
}

#[test]
fn submission_reset_clears_suppression_when_terminal_fallback_is_unavailable() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "cycle"));
    state.replace_history(vec!["git status".into()]);
    state.track_user_input("git");
    state.refresh("git");
    state.dismiss();
    assert!(!state.is_popup_visible());

    assert_eq!(state.complete_submission(None).as_deref(), Some("git"));
    assert_eq!(state.tracked_input(), Some(""));
    assert!(!state.fill_pending());

    state.track_user_input("git");
    state.refresh("git");
    assert!(state.is_popup_visible());
}

#[test]
fn successful_fill_tracks_candidate_without_recording_execution() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "fill"));
    state.replace_history(vec!["git status".into()]);
    state.track_user_input("git");

    assert!(state.begin_fill(10, "git status"));
    assert!(state.finish_fill(10));

    assert_eq!(state.tracked_input(), Some("git status"));
    assert_eq!(state.history(), ["git status"]);
    assert!(state.host_session_history.is_empty());
}

#[test]
fn adb_shell_completion_yields_to_fish_and_restores_each_parent_layer() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "nested"));

    state.track_user_input("adb -s CR10M02RK2M202312151 shell");
    let submission = state.complete_submission(None);
    state.observe_submission(submission.as_deref(), true);
    state.track_user_input("ls");
    assert_eq!(state.direct_fill_prefix(), Some("ls"));

    let submission = state.complete_submission(None);
    state.observe_submission(submission.as_deref(), false);
    assert_eq!(state.foreground, ForegroundCompletion::AdbShell);

    state.track_user_input("fish");
    let submission = state.complete_submission(None);
    state.observe_submission(submission.as_deref(), false);
    state.track_user_input("git");
    assert_eq!(state.foreground, ForegroundCompletion::FishInAdb);
    assert_eq!(state.direct_fill_prefix(), None);

    state.complete_submission(None);
    state.track_user_input("exit");
    let submission = state.complete_submission(None);
    state.observe_submission(submission.as_deref(), false);
    assert_eq!(state.foreground, ForegroundCompletion::AdbShell);

    state.track_user_input("exit");
    let submission = state.complete_submission(None);
    state.observe_submission(submission.as_deref(), false);
    assert_eq!(state.foreground, ForegroundCompletion::AwaitingBashPrompt);
    state.track_user_input("ls");
    assert_eq!(state.direct_fill_prefix(), None);

    state.observe_authenticated_prompt();
    assert_eq!(state.foreground, ForegroundCompletion::IntegratedBash);
    assert_eq!(state.tracked_input(), Some(""));
}

#[test]
fn nested_shell_detection_rejects_noninteractive_and_compound_commands() {
    for command in [
        "adb -s SERIAL shell ls",
        "adb -s SERIAL shell; echo wrong",
        "adb shell",
        "adb -s FIRST -s SECOND shell",
        "echo adb -s SERIAL shell",
        "adb -s \"$SERIAL\" shell",
    ] {
        let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "not-adb"));
        state.observe_submission(Some(command), true);
        assert_eq!(
            state.foreground,
            ForegroundCompletion::IntegratedBash,
            "{command}"
        );
    }

    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "fish"));
    state.observe_submission(Some("adb -s SERIAL shell"), true);
    for command in ["fish -c pwd", "fish --command pwd", "fish --help"] {
        state.observe_submission(Some(command), false);
        assert_eq!(
            state.foreground,
            ForegroundCompletion::AdbShell,
            "{command}"
        );
    }
}

fn enter_adb(
    state: &mut CompletionState,
    serial: &str,
) -> (AdbHistoryIdentity, AdbHistoryLoadRequest) {
    if state.foreground == ForegroundCompletion::AwaitingBashPrompt {
        state.observe_authenticated_prompt();
    }
    let command = format!("adb -s {serial} shell");
    assert_eq!(
        state.observe_submission(Some(&command), true).as_deref(),
        Some(serial)
    );
    let identity = AdbHistoryIdentity::new(crate::adb_history::HostScope::Local, serial).unwrap();
    assert!(state.activate_adb_history(identity.clone()));
    let request = state.mark_adb_history_loading().unwrap();
    (identity, request)
}

#[test]
fn adb_history_layers_device_session_persisted_and_host_without_leaking_on_exit() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "layers"));
    state.replace_history(vec!["host loaded".into(), "shared".into()]);
    state.merge_executed("host session");
    let (_, request) = enter_adb(&mut state, "SERIAL");

    state.track_user_input("device current");
    assert_eq!(
        state.complete_submission(None).as_deref(),
        Some("device current")
    );
    assert!(state.apply_adb_history_result(
        &request,
        Ok::<_, std::io::Error>(vec!["device saved".into(), "shared".into()])
    ));
    assert_eq!(
        state.history(),
        [
            "device current",
            "device saved",
            "shared",
            "host session",
            "host loaded"
        ]
    );

    state.track_user_input("exit");
    let submission = state.complete_submission(None);
    state.observe_submission(submission.as_deref(), false);
    assert_eq!(state.history(), ["host session", "host loaded", "shared"]);
    assert!(state.active_adb_identity().is_none());
}

#[test]
fn late_host_history_load_rebuilds_beneath_active_adb_layers() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "late-host"));
    let host_request = state.mark_history_loading();
    let (_, adb_request) = enter_adb(&mut state, "SERIAL");
    state.track_user_input("device current");
    state.complete_submission(None);
    assert!(state.apply_adb_history_result(
        &adb_request,
        Ok::<_, std::io::Error>(vec!["device saved".into()])
    ));

    assert!(state.apply_history_result(
        &host_request,
        Ok::<_, std::io::Error>(vec!["host late".into()])
    ));
    assert_eq!(
        state.history(),
        ["device current", "device saved", "host late"]
    );
}

#[test]
fn stale_device_load_is_rejected_after_exit_and_same_device_reentry() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "epoch"));
    let (_, stale) = enter_adb(&mut state, "SERIAL");
    state.observe_submission(Some("exit"), false);
    let (_, current) = enter_adb(&mut state, "SERIAL");

    assert!(!state.apply_adb_history_result(&stale, Ok::<_, std::io::Error>(vec!["stale".into()])));
    assert!(
        state.apply_adb_history_result(&current, Ok::<_, std::io::Error>(vec!["current".into()]))
    );
    assert_eq!(state.history(), ["current"]);
}

#[test]
fn device_load_for_a_different_identity_is_rejected() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "identity"));
    let (_, first) = enter_adb(&mut state, "DEVICE-A");
    state.observe_submission(Some("exit"), false);
    let (_, second) = enter_adb(&mut state, "DEVICE-B");

    assert!(!state.apply_adb_history_result(&first, Ok::<_, std::io::Error>(vec!["from a".into()])));
    assert!(state.apply_adb_history_result(&second, Ok::<_, std::io::Error>(vec!["from b".into()])));
    assert_eq!(state.history(), ["from b"]);
}

#[test]
fn fish_temporarily_yields_completion_without_discarding_device_history() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "fish-adb"));
    let (identity, request) = enter_adb(&mut state, "SERIAL");
    assert!(
        state.apply_adb_history_result(&request, Ok::<_, std::io::Error>(vec!["getprop".into()]))
    );

    state.observe_submission(Some("fish"), false);
    assert_eq!(state.active_adb_identity(), Some(&identity));
    assert_eq!(state.direct_fill_prefix(), None);
    state.observe_submission(Some("exit"), false);
    state.track_user_input("get");
    assert_eq!(state.direct_fill_prefix(), Some("get"));
    assert_eq!(state.history(), ["getprop"]);
}

#[test]
fn adb_load_request_debug_redacts_session_and_identity() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "secret-session"));
    let (_, request) = enter_adb(&mut state, "SECRET-SERIAL");
    let debug = format!("{request:?}");
    assert!(!debug.contains("secret-session"));
    assert!(!debug.contains("SECRET-SERIAL"));
}

#[test]
fn device_load_failure_keeps_current_device_and_host_session_commands() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "failure"));
    state.merge_executed("host command");
    let (_, request) = enter_adb(&mut state, "SERIAL");
    state.track_user_input("device command");
    state.complete_submission(None);

    assert!(state.apply_adb_history_result(&request, Err::<Vec<String>, _>("failure")));
    assert_eq!(state.history(), ["device command", "host command"]);
    assert!(matches!(
        state.history_status(),
        HistoryStatus::Error { .. }
    ));
}

#[test]
fn session_reset_rejects_pending_device_load() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "old"));
    let (_, request) = enter_adb(&mut state, "SERIAL");
    state.reset_session(CompletionSessionKey::new_for_test(2, "new"));

    assert!(
        !state.apply_adb_history_result(&request, Ok::<_, std::io::Error>(vec!["stale".into()]))
    );
    assert!(state.history().is_empty());
    assert!(state.active_adb_identity().is_none());
}

#[test]
fn empty_ctrl_d_pops_fish_then_waits_for_the_parent_bash_prompt() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "ctrl-d"));
    state.observe_submission(Some("adb -s SERIAL shell"), true);
    state.observe_submission(Some("fish"), false);

    assert!(state.observe_empty_ctrl_d());
    assert_eq!(state.foreground, ForegroundCompletion::AdbShell);
    assert!(state.observe_empty_ctrl_d());
    assert_eq!(state.foreground, ForegroundCompletion::AwaitingBashPrompt);
    assert!(!state.observe_empty_ctrl_d());
}

#[test]
fn unsafe_surface_pause_resumes_nested_tracking_from_a_clean_line() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "surface"));
    state.observe_submission(Some("adb -s SERIAL shell"), true);
    state.track_user_input("stale");

    state.pause_surface_tracking();
    assert_eq!(state.direct_fill_prefix(), None);
    state.track_user_input("ignored");
    assert_eq!(state.tracked_input(), None);

    state.resume_surface_tracking();
    state.track_user_input("free");
    assert_eq!(state.direct_fill_prefix(), Some("free"));
}

#[test]
fn tracked_input_handles_unicode_backspace_ctrl_u_and_ambiguous_edits() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "input"));

    state.track_user_input("echo 你好");
    state.track_user_input("\x7f");
    assert_eq!(state.tracked_input(), Some("echo 你"));

    state.track_user_input("\x1b[D");
    assert_eq!(state.tracked_input(), None);
    state.track_user_input("x");
    assert_eq!(state.tracked_input(), None);

    state.track_user_input("\x15");
    assert_eq!(state.tracked_input(), Some(""));
}

#[test]
fn begin_fill_does_not_replace_an_existing_request() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "pending"));

    assert!(state.begin_fill(1, "git status"));
    assert!(!state.begin_fill(2, "git log"));

    assert!(state.pending_fill_matches(1));
    assert!(!state.pending_fill_matches(2));
}

#[test]
fn user_edit_cancels_and_returns_only_the_pending_request() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "pending"));
    state.begin_fill(41, "git status");

    assert_eq!(state.on_user_edit(), Some(41));
    assert_eq!(state.on_user_edit(), None);
    assert!(!state.fill_pending());
}

#[test]
fn stale_fill_request_cannot_finish_or_fail_the_current_request() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "pending"));
    state.begin_fill(42, "git status");

    assert!(!state.pending_fill_matches(41));
    assert!(!state.finish_fill(41));
    state.fail_fill(41);
    assert!(state.pending_fill_matches(42));
}

#[test]
fn fill_request_from_a_reset_session_is_rejected() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "old-token"));
    state.begin_fill(43, "git status");
    state.reset_session(CompletionSessionKey::new_for_test(2, "new-token"));

    assert!(!state.pending_fill_matches(43));
    assert!(!state.finish_fill(43));
    state.fail_fill(43);
    assert!(!state.fill_pending());
}

#[test]
fn fill_request_requires_the_session_captured_when_it_began() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "old-token"));
    state.begin_fill(44, "git status");
    state.session = CompletionSessionKey::new_for_test(2, "new-token");

    assert!(!state.pending_fill_matches(44));
    assert!(!state.finish_fill(44));
    state.fail_fill(44);
    assert!(state.fill_pending());
}

#[test]
fn empty_successful_history_is_ready() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "history"));
    assert!(matches!(
        state.history_status(),
        HistoryStatus::Disabled { .. }
    ));
    let request = state.mark_history_loading();
    assert_eq!(state.history_status(), HistoryStatus::Loading);

    assert!(state.apply_history_result(&request, Ok::<_, std::io::Error>(Vec::new())));

    assert_eq!(state.history_status(), HistoryStatus::Ready { items: 0 });
    assert!(state.history().is_empty());
}

#[test]
fn history_snapshot_keeps_commands_executed_after_loading_started() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "history"));
    state.replace_history(vec!["cached old".into()]);
    let request = state.mark_history_loading();
    state.merge_executed("git status");

    assert!(state.apply_history_result(
        &request,
        Ok::<_, std::io::Error>(vec!["ls".into(), "git status".into(), "pwd".into(),]),
    ));

    assert_eq!(state.history(), ["git status", "ls", "pwd"]);
    assert_eq!(state.history_status(), HistoryStatus::Ready { items: 3 });
}

#[test]
fn stale_history_request_cannot_replace_the_current_load() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "history"));
    let stale = state.mark_history_loading();
    let current = state.mark_history_loading();

    assert!(!state.apply_history_result(&stale, Ok::<_, std::io::Error>(vec!["stale".into()])));
    assert_eq!(state.history_status(), HistoryStatus::Loading);
    assert!(state.apply_history_result(&current, Ok::<_, std::io::Error>(vec!["current".into()])));
    assert_eq!(state.history(), ["current"]);
}

#[test]
fn cancelled_history_request_cannot_apply_after_reconnect() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "history"));
    let request = state.mark_history_loading();

    state.cancel_history_load();

    assert!(matches!(
        state.history_status(),
        HistoryStatus::Disabled { .. }
    ));
    assert!(!state.apply_history_result(&request, Ok::<_, std::io::Error>(vec!["stale".into()])));
    assert!(state.history().is_empty());
}

#[test]
fn history_error_diagnostics_are_fixed_and_safe() {
    let session_token = "secret-session-token";
    let raw_error = "/home/alice/.bash_history: permission denied; token=abc; command=rm";
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, session_token));
    state.begin_fill(45, "git status");
    assert!(!format!("{:?}", state.pending_fill).contains(session_token));

    let command = "secret command --token abc";
    let request = state.mark_history_loading();
    assert!(!format!("{request:?}").contains(session_token));
    state.apply_history_result(&request, Ok::<_, std::io::Error>(vec![command.into()]));
    assert!(!format!("{:?}", state.history_status()).contains(command));

    let request = state.mark_history_loading();
    state.apply_history_result(&request, Err::<Vec<String>, _>(raw_error));

    let status = state.history_status();
    let diagnostic = format!("{status:?}");
    assert!(matches!(status, HistoryStatus::Error { .. }));
    assert!(!diagnostic.contains(raw_error));
    assert!(!diagnostic.contains("/home/alice"));
    assert!(!diagnostic.contains(session_token));
    assert!(!diagnostic.contains("token=abc"));
    assert!(!diagnostic.contains("command=rm"));
}

#[test]
fn successful_history_replace_preserves_merge_behavior() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "history"));
    let request = state.mark_history_loading();
    state.apply_history_result(
        &request,
        Ok::<_, std::io::Error>(vec!["ls".into(), "pwd".into(), "ls".into()]),
    );
    state.merge_executed("pwd");

    assert_eq!(state.history(), ["pwd", "ls"]);
    assert_eq!(state.history_status(), HistoryStatus::Ready { items: 2 });
}

#[test]
fn clearing_candidates_also_resets_selection() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "clear"));
    state.replace_history(vec!["git a".into(), "git b".into()]);
    state.refresh("git");
    state.move_selection(1);
    assert_eq!(state.selected(), 1);

    state.clear_candidates();

    assert!(state.candidates().is_empty());
    assert_eq!(state.selected(), 0);
}

#[test]
fn refresh_reuses_ranked_snapshot_for_the_same_key() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "cache"));
    state.replace_history(vec!["git status".into(), "git log".into()]);

    assert!(state.refresh("git"));
    let key_builds = state.candidate_cache_key_build_count();
    state.move_selection(1);
    let candidates = state.candidates().to_vec();
    assert!(!state.refresh("git"));

    assert_eq!(state.candidates(), candidates);
    assert_eq!(state.selected(), 1);
    assert_eq!(state.candidate_cache_key_build_count(), key_builds);
}

#[test]
fn refresh_recomputes_only_after_prefix_history_or_session_changes() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "first"));
    state.replace_history(vec!["git status".into(), "git log".into()]);

    assert!(state.refresh("git"));
    assert!(state.refresh("git s"));
    assert!(!state.refresh("git s"));

    state.replace_history(vec!["git show".into()]);
    assert!(state.refresh("git s"));
    assert!(!state.refresh("git s"));

    state.reset_session(CompletionSessionKey::new_for_test(2, "second"));
    state.replace_history(vec!["git stash".into()]);
    assert!(state.refresh("git s"));
}

#[test]
fn finish_fill_reuses_the_candidate_clear_selection_invariant() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "fill"));
    state.replace_history(vec!["git a".into(), "git b".into()]);
    state.refresh("git");
    state.move_selection(1);
    state.begin_fill(10, "git b");
    assert_eq!(state.selected(), 1);

    assert!(state.finish_fill(10));

    assert!(state.candidates().is_empty());
    assert_eq!(state.selected(), 0);
    assert!(state.refresh("git"));
}

#[test]
fn state_rejects_commands_with_trailing_carriage_returns() {
    let mut replaced = CompletionState::new(CompletionSessionKey::new_for_test(1, "replace"));
    replaced.replace_history(vec!["safe".into(), "bad\r".into()]);

    let mut merged = CompletionState::new(CompletionSessionKey::new_for_test(1, "merge"));
    merged.replace_history(vec!["safe".into()]);
    merged.merge_executed("bad\r");

    assert_eq!(
        (replaced.history(), merged.history()),
        (&["safe".to_owned()][..], &["safe".to_owned()][..])
    );
}

#[test]
fn escape_suppresses_until_the_next_user_edit() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "b"));
    state.replace_history(vec!["git status".into()]);
    state.refresh("git");
    assert!(state.is_popup_visible());
    state.dismiss();
    state.refresh("git");
    assert!(!state.is_popup_visible());
    state.on_user_edit();
    state.refresh("git");
    assert!(state.is_popup_visible());
}

#[test]
fn selection_wraps_for_large_delta_without_overflow() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "c"));
    state.replace_history(vec!["git a".into(), "git b".into(), "git c".into()]);
    state.refresh("git");
    state.move_selection(1);
    state.move_selection(isize::MAX);
    assert_eq!(state.selected(), 2);
}

#[test]
fn selection_wraps_for_minimum_delta_without_overflow() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "d"));
    state.replace_history(vec!["git a".into(), "git b".into(), "git c".into()]);
    state.refresh("git");
    state.move_selection(isize::MIN);
    assert_eq!(state.selected(), 1);
}

#[test]
fn successor_rotates_token_and_increments_generation() {
    let current = CompletionSessionKey::new_for_test(7, "old");
    let next = current.successor();
    assert_eq!(next.generation, 8);
    assert_ne!(next.token(), current.token());
}

#[test]
fn successor_wraps_max_generation_to_one_and_rotates_token() {
    let current = CompletionSessionKey::new_for_test(u64::MAX, "old");
    let next = current.successor();
    assert_eq!(next.generation, 1);
    assert_ne!(next.token(), current.token());
}

#[test]
fn history_tail_drops_a_partial_first_line() {
    let file = TemporaryHistoryFile::new(b"old-command\nnew-command\n");
    assert_eq!(
        read_history_tail(file.path(), 15).unwrap(),
        b"new-command\n"
    );
}

#[test]
fn history_tail_keeps_a_complete_first_line_at_an_exact_boundary() {
    let file = TemporaryHistoryFile::new(b"old-command\nnew-command\n");
    assert_eq!(
        read_history_tail(file.path(), 12).unwrap(),
        b"new-command\n"
    );
}

#[test]
fn history_tail_of_an_empty_file_is_empty() {
    let file = TemporaryHistoryFile::new(b"");
    assert!(read_history_tail(file.path(), 12).unwrap().is_empty());
}

#[test]
fn history_tail_with_zero_byte_limit_is_empty() {
    let file = TemporaryHistoryFile::new(b"command\n");
    assert!(read_history_tail(file.path(), 0).unwrap().is_empty());
}
