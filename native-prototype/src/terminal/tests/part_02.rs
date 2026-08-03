    #[test]
    fn recoverable_edit_rejects_initial_and_retry_responses_before_post_edit_snapshot() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, expected_snapshot) = install_local_snapshot_runtime(&mut terminal);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice(b"git");
        terminal.process_pty_output(&mut parser, &output);

        terminal.resize(81, 24);
        assert_eq!(write_rx.try_recv().unwrap(), expected_snapshot);
        let retry_at = Instant::now();
        terminal
            .prompt_tracking
            .as_mut()
            .unwrap()
            .snapshot_requested_at =
            Some(retry_at - SNAPSHOT_RETRY_TIMEOUT - Duration::from_millis(1));
        assert_eq!(
            terminal.current_bash_input_or_request_snapshot(retry_at),
            None
        );
        assert_eq!(write_rx.try_recv().unwrap(), expected_snapshot);

        terminal.invalidate_readline_geometry();
        terminal.write_input("\x1b[D");
        assert_eq!(
            terminal.current_bash_input_or_request_snapshot(Instant::now()),
            None
        );
        assert_eq!(write_rx.try_recv().unwrap(), b"\x1b[D");
        assert_eq!(write_rx.try_recv().unwrap(), expected_snapshot);

        for old_input in ["git status", "git log"] {
            terminal.process_pty_output(
                &mut parser,
                &input_snapshot_marker(&completion_session(), old_input, 3),
            );
            assert_eq!(
                terminal.current_bash_input_or_request_snapshot(Instant::now()),
                None,
                "{old_input:?} must remain rejected"
            );
            assert!(terminal
                .prompt_tracking
                .as_ref()
                .unwrap()
                .snapshot_requested_at
                .is_some());
        }

        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "gi", 2),
        );
        assert_eq!(terminal.current_bash_input().as_deref(), Some("gi"));
    }

    #[test]
    fn first_valid_retry_response_prevents_later_same_state_response_from_overwriting_input() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, expected_snapshot) = install_local_snapshot_runtime(&mut terminal);
        let mut parser = TestProcessor::new();
        terminal.process_pty_output(&mut parser, &prompt_marker());
        terminal.resize(81, 24);
        assert_eq!(write_rx.try_recv().unwrap(), expected_snapshot);
        let retry_at = Instant::now();
        terminal
            .prompt_tracking
            .as_mut()
            .unwrap()
            .snapshot_requested_at =
            Some(retry_at - SNAPSHOT_RETRY_TIMEOUT - Duration::from_millis(1));
        assert_eq!(
            terminal.current_bash_input_or_request_snapshot(retry_at),
            None
        );
        assert_eq!(write_rx.try_recv().unwrap(), expected_snapshot);

        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "git", 3),
        );
        assert_eq!(terminal.current_bash_input().as_deref(), Some("git"));
        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "stale", 5),
        );

        assert_eq!(terminal.current_bash_input().as_deref(), Some("git"));
    }

    #[test]
    fn new_prompt_clears_stale_snapshot_debt_before_the_next_resize_response() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, expected_snapshot) = install_local_snapshot_runtime(&mut terminal);
        let mut parser = TestProcessor::new();
        terminal.process_pty_output(&mut parser, &prompt_marker());
        terminal.resize(81, 24);
        assert_eq!(write_rx.try_recv().unwrap(), expected_snapshot);

        terminal.invalidate_readline_geometry();
        assert_eq!(
            terminal.current_bash_input_or_request_snapshot(Instant::now()),
            None
        );
        assert_eq!(write_rx.try_recv().unwrap(), expected_snapshot);

        terminal.process_pty_output(&mut parser, &prompt_marker());
        terminal.resize(82, 24);
        assert_eq!(write_rx.try_recv().unwrap(), expected_snapshot);
        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "cargo", 5),
        );

        assert_eq!(terminal.current_bash_input().as_deref(), Some("cargo"));
    }

    #[test]
    fn real_resize_requests_one_snapshot_and_valid_snapshot_restores_prefix() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, expected) = install_local_snapshot_runtime(&mut terminal);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice(b"git");
        terminal.process_pty_output(&mut parser, &output);

        terminal.resize(81, 24);

        assert_eq!(write_rx.try_recv().unwrap(), expected);
        assert!(matches!(
            write_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(terminal.current_bash_input(), None);

        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "git status", 3),
        );
        assert_eq!(terminal.current_bash_input().as_deref(), Some("git"));

        terminal.process_pty_output(&mut parser, b"x");
        assert_eq!(terminal.current_bash_input().as_deref(), Some("gitx"));
    }

    #[test]
    fn backward_edit_before_snapshot_base_requests_and_accepts_fresh_snapshot() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, expected) = install_local_snapshot_runtime(&mut terminal);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice(b"git");
        terminal.process_pty_output(&mut parser, &output);
        terminal.resize(81, 24);
        assert_eq!(write_rx.try_recv().unwrap(), expected);
        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "git", 3),
        );
        assert_eq!(terminal.current_bash_input().as_deref(), Some("git"));

        terminal.process_pty_output(&mut parser, b"\x1b[D");

        assert_eq!(terminal.current_bash_input(), None);
        let tracking = terminal.prompt_tracking.as_ref().unwrap();
        assert!(tracking.active);
        assert!(tracking.snapshot_base.is_none());

        let now = Instant::now();
        assert_eq!(terminal.current_bash_input_or_request_snapshot(now), None);
        assert_eq!(write_rx.try_recv().unwrap(), expected);
        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "gi", 2),
        );
        assert_eq!(terminal.current_bash_input().as_deref(), Some("gi"));
    }

    #[test]
    fn soft_wrapped_input_is_bounded_by_snapshot_protocol_limit() {
        let mut terminal = tracked_terminal(64, 140);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend(std::iter::repeat_n(
            b'x',
            crate::bash_integration::MAX_SNAPSHOT_INPUT_BYTES,
        ));
        terminal.process_pty_output(&mut parser, &output);

        assert_eq!(
            terminal.current_bash_input().as_ref().map(String::len),
            Some(crate::bash_integration::MAX_SNAPSHOT_INPUT_BYTES)
        );

        terminal.process_pty_output(&mut parser, b"x");

        assert_eq!(terminal.current_bash_input(), None);
    }

    #[test]
    fn stale_snapshot_authentication_cannot_restore_input() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, _) = install_local_snapshot_runtime(&mut terminal);
        let mut parser = TestProcessor::new();
        terminal.process_pty_output(&mut parser, &prompt_marker());
        terminal.resize(81, 24);
        let _ = write_rx.try_recv().unwrap();
        let stale = CompletionSessionKey::new_for_test(GENERATION + 1, TOKEN);

        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&stale, "stale command", 5),
        );

        assert_eq!(terminal.current_bash_input(), None);
    }

    #[test]
    fn no_op_resize_does_not_request_snapshot() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, _) = install_local_snapshot_runtime(&mut terminal);
        let mut parser = TestProcessor::new();
        terminal.process_pty_output(&mut parser, &prompt_marker());

        terminal.resize(80, 24);

        assert!(matches!(
            write_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(terminal.current_bash_input().as_deref(), Some(""));
    }

    #[test]
    fn inactive_prompt_never_requests_or_retries_snapshot() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, _) = install_local_snapshot_runtime(&mut terminal);
        let now = std::time::Instant::now();

        assert_eq!(terminal.current_bash_input_or_request_snapshot(now), None);
        assert_eq!(
            terminal.current_bash_input_or_request_snapshot(now + Duration::from_secs(10)),
            None
        );
        assert!(matches!(
            write_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn snapshot_request_retries_only_after_timeout() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, expected) = install_local_snapshot_runtime(&mut terminal);
        let mut parser = TestProcessor::new();
        terminal.process_pty_output(&mut parser, &prompt_marker());
        terminal.resize(81, 24);
        assert_eq!(write_rx.try_recv().unwrap(), expected);
        let requested_at = terminal
            .prompt_tracking
            .as_ref()
            .unwrap()
            .snapshot_requested_at
            .unwrap();

        assert_eq!(
            terminal.current_bash_input_or_request_snapshot(
                requested_at + SNAPSHOT_RETRY_TIMEOUT - Duration::from_millis(1)
            ),
            None
        );
        assert!(matches!(
            write_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));

        assert_eq!(
            terminal.current_bash_input_or_request_snapshot(requested_at + SNAPSHOT_RETRY_TIMEOUT),
            None
        );
        assert_eq!(write_rx.try_recv().unwrap(), expected);
    }

    #[test]
    fn repeated_resize_keeps_one_pending_snapshot_request_until_timeout() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, expected) = install_local_snapshot_runtime(&mut terminal);
        let mut parser = TestProcessor::new();
        terminal.process_pty_output(&mut parser, &prompt_marker());
        terminal.resize(81, 24);
        assert_eq!(write_rx.try_recv().unwrap(), expected);
        let requested_at = terminal
            .prompt_tracking
            .as_ref()
            .unwrap()
            .snapshot_requested_at
            .unwrap();

        terminal.resize(82, 24);

        assert!(matches!(
            write_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(
            terminal
                .prompt_tracking
                .as_ref()
                .unwrap()
                .snapshot_requested_at,
            Some(requested_at)
        );
    }

    #[test]
    fn newer_prompt_rejects_delayed_same_session_snapshot() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, _) = install_local_snapshot_runtime(&mut terminal);
        let mut parser = TestProcessor::new();
        terminal.process_pty_output(&mut parser, &prompt_marker());
        terminal.resize(81, 24);
        let _ = write_rx.try_recv().unwrap();

        terminal.process_pty_output(&mut parser, &prompt_marker());
        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "delayed", 7),
        );

        assert_eq!(terminal.current_bash_input().as_deref(), Some(""));
        assert!(terminal
            .prompt_tracking
            .as_ref()
            .unwrap()
            .snapshot_base
            .is_none());
    }

    #[test]
    fn utf8_snapshot_point_restores_complete_character_prefix() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, _) = install_local_snapshot_runtime(&mut terminal);
        let mut parser = TestProcessor::new();
        terminal.process_pty_output(&mut parser, &prompt_marker());
        terminal.resize(81, 24);
        let _ = write_rx.try_recv().unwrap();

        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "你好abc", 6),
        );

        assert_eq!(terminal.current_bash_input().as_deref(), Some("你好"));
    }

    #[test]
    fn submission_and_alternate_screen_clear_snapshot_recovery() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, _) = install_local_snapshot_runtime(&mut terminal);
        let mut parser = TestProcessor::new();
        terminal.process_pty_output(&mut parser, &prompt_marker());
        terminal.resize(81, 24);
        let _ = write_rx.try_recv().unwrap();
        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "git status", 3),
        );
        assert_eq!(terminal.take_bash_submission().as_deref(), Some("git"));
        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "late", 4),
        );
        assert_eq!(terminal.current_bash_input(), None);

        terminal.process_pty_output(&mut parser, &prompt_marker());
        terminal.resize(82, 24);
        let _ = write_rx.try_recv().unwrap();
        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "cargo", 5),
        );
        assert_eq!(terminal.current_bash_input().as_deref(), Some("cargo"));
        terminal.process_pty_output(&mut parser, b"\x1b[?1049h");
        terminal.process_pty_output(&mut parser, b"\x1b[?1049l");
        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "late", 4),
        );
        assert_eq!(terminal.current_bash_input(), None);
    }

    #[test]
    fn history_path_event_carries_tracking_session() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();

        let events = terminal.process_pty_output(&mut parser, &history_marker("L3RtcC9oaXN0b3J5"));

        assert_eq!(
            events,
            vec![IntegrationEvent::HistoryPath {
                session: completion_session(),
                path: "/tmp/history".to_owned(),
            }]
        );
    }

    #[test]
    fn taking_submission_returns_input_and_clears_anchor() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice(b"git");
        terminal.process_pty_output(&mut parser, &output);

        assert_eq!(terminal.take_bash_submission().as_deref(), Some("git"));
        assert_eq!(terminal.current_bash_input(), None);
    }

    #[test]
    fn finish_session_drops_runtime_temp_directory_and_tracking() {
        let runtime = LocalBashRuntime::create(completion_session()).unwrap();
        let temp_dir = runtime.temp_dir().to_owned();
        let mut terminal = tracked_terminal(80, 24);
        terminal.local_bash_runtime = Some(runtime);

        terminal.finish_session();

        assert!(!temp_dir.exists());
        assert!(terminal.local_bash_runtime.is_none());
        assert!(terminal.prompt_tracking.is_none());
    }

    #[test]
    fn read_loop_callbacks_run_unlocked_and_eof_finishes_session() {
        let runtime = LocalBashRuntime::create(completion_session()).unwrap();
        let temp_dir = runtime.temp_dir().to_owned();
        let mut state = tracked_terminal(80, 24);
        state.local_bash_runtime = Some(runtime);
        let mut output = prompt_marker();
        output.extend_from_slice(b"git");
        output.extend_from_slice(&history_marker("L3RtcC9oaXN0b3J5"));
        state.pty_reader = Some(Box::new(std::io::Cursor::new(output)));
        let terminal = Arc::new(Mutex::new(state));
        let redraw_terminal = Arc::clone(&terminal);
        let event_terminal = Arc::clone(&terminal);
        let redraw_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let redraw_count_callback = Arc::clone(&redraw_count);
        let event_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let event_count_callback = Arc::clone(&event_count);

        read_loop(
            Arc::clone(&terminal),
            move || {
                assert!(redraw_terminal.try_lock().is_ok());
                redraw_count_callback.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
            move |_event| {
                assert!(event_terminal.try_lock().is_ok());
                event_count_callback.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
        );

        assert_eq!(redraw_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(event_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!temp_dir.exists());
        let state = terminal.lock().unwrap();
        assert!(state.local_bash_runtime.is_none());
        assert!(state.prompt_tracking.is_none());
    }

    #[test]
    fn read_loop_without_reader_still_finishes_session() {
        let runtime = LocalBashRuntime::create(completion_session()).unwrap();
        let temp_dir = runtime.temp_dir().to_owned();
        let mut state = tracked_terminal(80, 24);
        state.local_bash_runtime = Some(runtime);
        let terminal = Arc::new(Mutex::new(state));

        read_loop(Arc::clone(&terminal), || {}, |_event| {});

        assert!(!temp_dir.exists());
        assert!(terminal.lock().unwrap().prompt_tracking.is_none());
    }

    #[test]
    fn read_loop_unrecoverable_error_finishes_session_without_callbacks() {
        let runtime = LocalBashRuntime::create(completion_session()).unwrap();
        let temp_dir = runtime.temp_dir().to_owned();
        let mut state = tracked_terminal(80, 24);
        state.local_bash_runtime = Some(runtime);
        state.pty_reader = Some(Box::new(BrokenReader));
        let terminal = Arc::new(Mutex::new(state));
        let callback_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let redraw_count = Arc::clone(&callback_count);
        let event_count = Arc::clone(&callback_count);

        read_loop(
            Arc::clone(&terminal),
            move || {
                redraw_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
            move |_event| {
                event_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
        );

        assert_eq!(callback_count.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(!temp_dir.exists());
        let state = terminal.lock().unwrap();
        assert!(state.local_bash_runtime.is_none());
        assert!(state.prompt_tracking.is_none());
    }

    #[test]
    fn fish_cursor_forward_is_not_a_text_area_query() {
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        let mut parser = TestProcessor::new();

        assert!(advance_and_take(&mut terminal, &mut parser, b"\x1b[18C").is_empty());
        assert!(advance_and_take(&mut terminal, &mut parser, b"\x1b[18~").is_empty());
    }

    #[test]
    fn text_area_query_uses_alacritty_reply_and_survives_chunk_split() {
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        let mut parser = TestProcessor::new();

        assert!(advance_and_take(&mut terminal, &mut parser, b"\x1b[18").is_empty());
        assert_eq!(
            advance_and_take(&mut terminal, &mut parser, b"t"),
            vec!["\x1b[8;48;180t"]
        );
        assert!(terminal.take_pty_write_events().is_empty());
    }

    #[test]
    fn cursor_position_query_still_uses_alacritty_reply() {
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        let mut parser = TestProcessor::new();

        assert!(advance_and_take(&mut terminal, &mut parser, b"\x1b[3;5H").is_empty());
        assert_eq!(
            advance_and_take(&mut terminal, &mut parser, b"\x1b[6n"),
            vec!["\x1b[3;5R"]
        );
    }

    #[test]
    fn listener_ignores_a_disconnected_event_receiver() {
        let (pty_write_tx, pty_write_rx) = mpsc::channel();
        drop(pty_write_rx);
        let listener = Listener { pty_write_tx };

        listener.send_event(Event::PtyWrite("reply".to_owned()));
    }

    #[test]
    fn listener_discards_non_pty_write_events_at_the_boundary() {
        let (pty_write_tx, pty_write_rx) = mpsc::channel();
        let listener = Listener { pty_write_tx };

        listener.send_event(Event::Bell);

        assert!(matches!(
            pty_write_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn local_writer_does_not_receive_reply_for_fish_cursor_forward() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (completed_tx, completed_rx) = mpsc::channel();
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        terminal.writer = Some(spawn_writer_worker(Box::new(SharedWriter {
            captured: Arc::clone(&captured),
            completed_tx,
        })));
        let mut parser = TestProcessor::new();

        terminal.process_pty_output(&mut parser, b"\x1b[18C");

        assert!(matches!(
            completed_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(captured.lock().unwrap().is_empty());
    }

    #[test]
    fn local_and_ssh_writers_receive_the_same_alacritty_reply() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (completed_tx, completed_rx) = mpsc::channel();
        let mut local = TerminalState::new();
        local.init_term(180, 48);
        local.writer = Some(spawn_writer_worker(Box::new(SharedWriter {
            captured: Arc::clone(&captured),
            completed_tx,
        })));
        let mut local_parser = TestProcessor::new();

        let (write_tx, write_rx) = test_transport_capture();
        let mut ssh = TerminalState::new();
        ssh.init_term(180, 48);
        ssh.writer = Some(write_tx);
        let mut ssh_parser = TestProcessor::new();

        local.process_pty_output(&mut local_parser, b"\x1b[18t");
        ssh.process_pty_output(&mut ssh_parser, b"\x1b[18t");

        completed_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(&*captured.lock().unwrap(), b"\x1b[8;48;180t");
        assert_eq!(write_rx.try_recv().unwrap(), b"\x1b[8;48;180t");
    }

    #[test]
    fn protocol_reply_write_failure_does_not_panic() {
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (dropped_tx, dropped_rx) = mpsc::channel();
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        terminal.writer = Some(spawn_writer_worker(Box::new(FailingWriter {
            attempt_tx,
            dropped_tx,
        })));
        let mut parser = TestProcessor::new();

        terminal.process_pty_output(&mut parser, b"\x1b[18t");

        attempt_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        dropped_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        terminal.write_input("after failure");
    }

    #[test]
    fn local_writer_worker_keeps_protocol_processing_nonblocking() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        terminal.writer = Some(spawn_writer_worker(Box::new(BlockingWriter {
            entered_tx,
            release_rx,
        })));
        let mut parser = TestProcessor::new();

        terminal.process_pty_output(&mut parser, b"\x1b[18t");

        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        release_tx.send(()).unwrap();
    }

    #[test]
    fn completion_stage_only_clones_local_write_metadata() {
        let runtime = LocalBashRuntime::create(completion_session()).unwrap();
        let expected_path = runtime.candidate_path().to_path_buf();
        let mut terminal = TerminalState::new();
        terminal.local_bash_runtime = Some(runtime);

        let request = terminal.stage_completion_fill("git status", None).unwrap();

        assert_eq!(request.session, completion_session());
        assert_eq!(request.bytes, b"git status");
        assert_eq!(request.target, CandidateWriteTarget::Local(expected_path));
        assert!(terminal.stage_completion_fill("", None).is_err());
        assert!(terminal
            .stage_completion_fill("git status\n", None)
            .is_err());
    }

    #[test]
    fn completion_stage_only_clones_remote_write_metadata() {
        let runtime = crate::bash_integration::RemoteBashRuntime {
            session: completion_session(),
            bash_path: "/bin/bash".into(),
            rc_path: "/tmp/session.rc".into(),
            candidate_path: "/tmp/session.candidate".into(),
            widget_sequence: "\x1b[777;1~".into(),
            snapshot_sequence: "\x1b[778;1~".into(),
        };
        let mut terminal = TerminalState::new();
        terminal.remote_bash_runtime = Some(runtime);

        let request = terminal.stage_completion_fill("cargo test", None).unwrap();

        assert_eq!(request.session, completion_session());
        assert_eq!(request.bytes, b"cargo test");
        assert_eq!(
            request.target,
            CandidateWriteTarget::Remote("/tmp/session.candidate".into())
        );
    }

    #[test]
    fn direct_completion_stage_and_commit_write_only_the_candidate_suffix() {
        let runtime = LocalBashRuntime::create(completion_session()).unwrap();
        let (write_tx, write_rx) = test_transport_capture();
        let mut terminal = TerminalState::new();
        terminal.init_term(80, 24);
        terminal.local_bash_runtime = Some(runtime);
        terminal.writer = Some(write_tx);

        let request = terminal
            .stage_completion_fill("echo 你好", Some("echo 你"))
            .unwrap();

        assert_eq!(request.session, completion_session());
        assert_eq!(request.target, CandidateWriteTarget::Direct);
        assert_eq!(request.bytes, "好".as_bytes());
        assert!(terminal.commit_direct_completion_fill(&request.bytes));
        assert_eq!(write_rx.try_recv().unwrap(), "好".as_bytes());
        assert!(!request.bytes.contains(&b'\r'));
        assert!(!request.bytes.contains(&b'\n'));
    }

    #[test]
    fn direct_completion_rejects_stale_prefix_controls_and_unsafe_surfaces() {
        let runtime = LocalBashRuntime::create(completion_session()).unwrap();
        let mut terminal = TerminalState::new();
        terminal.init_term(80, 24);
        terminal.local_bash_runtime = Some(runtime);

        assert!(terminal
            .stage_completion_fill("git status", Some("fish"))
            .is_err());
        assert!(terminal
            .stage_completion_fill("git status", Some("git status"))
            .is_err());
        assert!(!terminal.commit_direct_completion_fill(b""));
        assert!(!terminal.commit_direct_completion_fill(b" status\r"));
        assert!(!terminal.commit_direct_completion_fill("\u{85}".as_bytes()));

        let mut parser = TestProcessor::new();
        terminal.process_pty_output(&mut parser, b"\x1b[?1049h");
        assert!(!terminal.completion_surface_safe());
        assert!(terminal
            .stage_completion_fill("git status", Some("git"))
            .is_err());
        terminal.process_pty_output(&mut parser, b"\x1b[?1049l\x1b[?1000h");
        assert!(!terminal.completion_surface_safe());
    }
