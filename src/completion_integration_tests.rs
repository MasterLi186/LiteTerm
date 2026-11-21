use crate::smart_completion::{parse_bash_history, CompletionSessionKey, CompletionState};
use crate::terminal::CompletionHarness;

#[test]
fn local_bash_prompt_history_selection_and_submission_flow() {
    let session = CompletionSessionKey::new_for_test(1, "abc");
    let mut terminal = CompletionHarness::new(40, 6, session.clone());
    terminal.feed(b"$ \x1b]777;LiteTerm;abc;1;P\x07git");
    let mut completion = CompletionState::new(session);
    completion.replace_history(parse_bash_history(b"git log\ngit status\n"));
    completion.refresh(terminal.input().as_deref().unwrap());
    assert_eq!(completion.selected_candidate(), Some("git status"));
    assert_eq!(terminal.submit().as_deref(), Some("git"));
    completion.merge_executed("git");
    assert_eq!(completion.history()[0], "git");
    assert_eq!(terminal.input(), None);
}

#[test]
fn first_and_second_command_cycles_both_offer_visible_candidates() {
    let session = CompletionSessionKey::new_for_test(1, "abc");
    let mut terminal = CompletionHarness::new(40, 6, session.clone());
    let mut completion = CompletionState::new(session);
    completion.replace_history(vec!["ls -al".into(), "git status".into()]);

    terminal.feed(b"$ \x1b]777;LiteTerm;abc;1;P\x07ls");
    completion.track_user_input("ls");
    let first_input = completion.current_input(terminal.input().as_deref());
    completion.refresh(first_input.as_deref().unwrap());
    assert_eq!(completion.selected_candidate(), Some("ls -al"));
    assert!(completion.is_popup_visible());

    let fallback = terminal.submit();
    assert_eq!(
        completion
            .complete_submission(fallback.as_deref())
            .as_deref(),
        Some("ls")
    );
    terminal.feed(b"\r\nfile\r\n$ \x1b]777;LiteTerm;abc;1;P\x07git");
    completion.track_user_input("git");
    let second_input = completion.current_input(terminal.input().as_deref());
    completion.refresh(second_input.as_deref().unwrap());

    assert_eq!(completion.selected_candidate(), Some("git status"));
    assert!(completion.is_popup_visible());
}

#[test]
fn tracked_submission_survives_missing_grid_and_opens_second_cycle_candidate() {
    let session = CompletionSessionKey::new_for_test(1, "abc");
    let mut terminal = CompletionHarness::new(40, 6, session.clone());
    let mut completion = CompletionState::new(session);
    completion.replace_history(vec!["ls -al".into(), "git status".into()]);

    terminal.feed(b"$ \x1b]777;LiteTerm;abc;1;P\x07ls");
    completion.track_user_input("ls");
    let first_input = completion.tracked_input().unwrap().to_owned();
    completion.refresh(&first_input);
    assert_eq!(completion.selected_candidate(), Some("ls -al"));
    assert!(completion.is_popup_visible());

    terminal.resize(41, 6);
    assert_eq!(terminal.input(), None);
    let unavailable_fallback = terminal.submit();
    assert_eq!(unavailable_fallback, None);
    assert_eq!(
        completion
            .complete_submission(unavailable_fallback.as_deref())
            .as_deref(),
        Some("ls")
    );
    assert_eq!(completion.history()[0], "ls");
    assert_eq!(completion.tracked_input(), Some(""));
    assert!(completion.candidates().is_empty());

    terminal.feed(b"\r\nfile\r\n$ \x1b]777;LiteTerm;abc;1;P\x07git");
    completion.track_user_input("git");
    let second_input = completion.current_input(terminal.input().as_deref());
    completion.refresh(second_input.as_deref().unwrap());

    assert_eq!(completion.selected_candidate(), Some("git status"));
    assert!(completion.is_popup_visible());
}

#[test]
fn ambiguous_readline_edit_uses_authenticated_terminal_input() {
    let session = CompletionSessionKey::new_for_test(1, "abc");
    let mut terminal = CompletionHarness::new(40, 6, session.clone());
    terminal.feed(b"$ \x1b]777;LiteTerm;abc;1;P\x07git status");
    let mut completion = CompletionState::new(session);
    completion.track_user_input("git");

    completion.track_user_input("\x1b[D");

    assert_eq!(completion.tracked_input(), None);
    assert_eq!(
        completion
            .current_input(terminal.input().as_deref())
            .as_deref(),
        Some("git status"),
    );
}

#[test]
fn prompt_submission_output_and_next_prompt_activate_second_input() {
    let session = CompletionSessionKey::new_for_test(1, "abc");
    let mut terminal = CompletionHarness::new(40, 6, session);
    terminal.feed(b"$ \x1b]777;LiteTerm;abc;1;P\x07ls");
    assert!(terminal.authenticated_prompt_active());
    assert_eq!(terminal.submit().as_deref(), Some("ls"));
    assert!(!terminal.authenticated_prompt_active());

    terminal.feed(b"\r\nfile\r\n$ \x1b]777;LiteTerm;abc;1;P\x07git");

    assert!(terminal.authenticated_prompt_active());
    assert_eq!(terminal.input().as_deref(), Some("git"));
}

#[test]
fn shell_switch_stays_disabled_without_a_new_authenticated_prompt() {
    let session = CompletionSessionKey::new_for_test(1, "abc");
    let mut terminal = CompletionHarness::new(40, 6, session);
    terminal.feed(b"$ \x1b]777;LiteTerm;abc;1;P\x07fish");
    assert_eq!(terminal.submit().as_deref(), Some("fish"));
    terminal.feed(b"fish> ");
    assert_eq!(terminal.input(), None);
}

#[test]
fn missing_history_and_long_commands_degrade_safely() {
    let session = CompletionSessionKey::new_for_test(1, "abc");
    let mut completion = CompletionState::new(session);
    assert!(parse_bash_history(b"").is_empty());
    let long = format!("echo {}", "x".repeat(16_384));
    completion.replace_history(vec![long.clone()]);
    completion.refresh("echo ");
    assert_eq!(completion.selected_candidate(), Some(long.as_str()));
}

#[test]
fn ssh_reconnect_and_failed_sftp_fill_reject_old_events() {
    let previous = CompletionSessionKey::new_for_test(2, "old");
    let current = previous.successor();
    let mut completion = CompletionState::new(current.clone());
    completion.begin_fill(8, "git status");
    assert!(!crate::completion_fill_may_commit(
        &completion,
        &current,
        &previous,
        8,
        &Ok(()),
    ));
    assert!(!crate::completion_fill_may_commit(
        &completion,
        &current,
        &current,
        8,
        &Err("SFTP unavailable".into()),
    ));
}

#[test]
fn local_and_ssh_snapshot_recovery_requests_never_execute() {
    let session = CompletionSessionKey::new_for_test(1, "abc");
    let mut local = CompletionHarness::new(40, 6, session.clone());
    let (local_requests, local_sequence) = local.enable_local_snapshot_requests();
    local.feed(b"$ \x1b]777;LiteTerm;abc;1;P\x07git");
    local.resize(41, 6);
    assert_eq!(local_requests.try_recv().unwrap(), local_sequence);
    assert!(!local_sequence
        .iter()
        .any(|byte| matches!(byte, b'\r' | b'\n')));
    local.feed(b"\x1b]777;LiteTerm;abc;1;I;3;Z2l0IHN0YXR1cw\x07");
    assert_eq!(local.input().as_deref(), Some("git"));

    let mut ssh = CompletionHarness::new(40, 6, session);
    let ssh_sequence = b"\x1b[778;123~".to_vec();
    let ssh_requests = ssh.enable_remote_snapshot_requests(&ssh_sequence);
    ssh.feed(b"$ \x1b]777;LiteTerm;abc;1;P\x07git");
    ssh.resize(41, 6);
    assert_eq!(ssh_requests.try_recv().unwrap(), ssh_sequence);
    assert!(!ssh_sequence
        .iter()
        .any(|byte| matches!(byte, b'\r' | b'\n')));
    ssh.feed(b"\x1b]777;LiteTerm;abc;1;I;3;Z2l0IHN0YXR1cw\x07");
    assert_eq!(ssh.input().as_deref(), Some("git"));
}
