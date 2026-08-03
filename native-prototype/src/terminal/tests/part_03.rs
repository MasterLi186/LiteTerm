    #[test]
    fn completion_commit_writes_widget_without_any_execute_byte() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (completed_tx, completed_rx) = mpsc::channel();
        let mut state = TerminalState::new();
        state.init_term(80, 24);
        state.writer = Some(spawn_writer_worker(Box::new(SharedWriter {
            captured: captured.clone(),
            completed_tx,
        })));
        let runtime =
            LocalBashRuntime::create(CompletionSessionKey::new_for_test(1, "abcdef12")).unwrap();
        let expected = runtime.widget_sequence().as_bytes().to_vec();
        state.local_bash_runtime = Some(runtime);

        assert!(state.commit_completion_fill());

        completed_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let bytes = captured.lock().unwrap().clone();
        assert_eq!(bytes, expected);
        assert!(!bytes.contains(&b'\r'));
        assert!(!bytes.contains(&b'\n'));
    }

    #[test]
    fn remote_bash_handle_installs_matching_prompt_tracking_metadata() {
        let session = completion_session();
        let runtime = crate::bash_integration::RemoteBashRuntime {
            session: session.clone(),
            bash_path: "/bin/bash".into(),
            rc_path: "/tmp/session.rc".into(),
            candidate_path: "/tmp/session.candidate".into(),
            widget_sequence: "\x1b[777;1~".into(),
            snapshot_sequence: "\x1b[778;1~".into(),
        };
        let (write_tx, _write_rx) = crate::zmodem::runtime::transport_write_channel(Arc::new(
            crate::zmodem::runtime::ProtocolGate::new(),
        ));
        let (resize_tx, _resize_rx) = mpsc::sync_channel(8);
        let (shutdown_tx, _shutdown_rx) = mpsc::channel();
        let (_io_done_tx, io_done_rx) = mpsc::channel();
        let handle = crate::ssh::SshHandle {
            reader: Box::new(std::io::empty()),
            write_tx,
            resize_tx,
            shutdown_tx,
            io_done_rx,
            bash_runtime: Some(runtime.clone()),
        };
        let mut terminal = TerminalState::new();

        terminal.apply_ssh_handle(handle, 80, 24);

        assert_eq!(terminal.remote_bash_runtime.as_ref(), Some(&runtime));
        assert_eq!(
            terminal
                .prompt_tracking
                .as_ref()
                .map(|tracking| &tracking.session),
            Some(&session)
        );
        assert!(terminal.local_bash_runtime.is_none());
    }

    #[test]
    fn terminal_shutdown_signals_ssh_and_clears_io_channels() {
        let (write_tx, _write_rx) = crate::zmodem::runtime::transport_write_channel(Arc::new(
            crate::zmodem::runtime::ProtocolGate::new(),
        ));
        let (resize_tx, _resize_rx) = mpsc::sync_channel(8);
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let (_io_done_tx, io_done_rx) = mpsc::channel();
        let handle = crate::ssh::SshHandle {
            reader: Box::new(std::io::empty()),
            write_tx,
            resize_tx,
            shutdown_tx,
            io_done_rx,
            bash_runtime: None,
        };
        let mut terminal = TerminalState::new();
        terminal.apply_ssh_handle(handle, 80, 24);

        terminal.shutdown();

        assert!(shutdown_rx.try_recv().is_ok());
        assert!(terminal.writer.is_none());
        assert!(terminal.ssh_resize_tx.is_none());
        terminal.shutdown();
        assert!(matches!(
            shutdown_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn applied_ssh_handle_keeps_all_control_channels_alive() {
        let (write_tx, write_rx) = crate::zmodem::runtime::transport_write_channel(Arc::new(
            crate::zmodem::runtime::ProtocolGate::new(),
        ));
        let (resize_tx, resize_rx) = mpsc::sync_channel(8);
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let (io_done_tx, io_done_rx) = mpsc::channel();
        let mut terminal = TerminalState::new();

        terminal.apply_ssh_handle(
            crate::ssh::SshHandle {
                reader: Box::new(std::io::empty()),
                write_tx,
                resize_tx,
                shutdown_tx,
                io_done_rx,
                bash_runtime: None,
            },
            80,
            24,
        );

        assert!(matches!(
            write_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert!(matches!(
            resize_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert!(matches!(
            shutdown_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        io_done_tx
            .send(())
            .expect("TerminalState 必须持有 SSH I/O 完成通知接收端");
    }

    #[test]
    fn terminal_resize_forwards_dimensions_to_ssh_worker() {
        let (write_tx, _write_rx) = crate::zmodem::runtime::transport_write_channel(Arc::new(
            crate::zmodem::runtime::ProtocolGate::new(),
        ));
        let (resize_tx, resize_rx) = mpsc::sync_channel(8);
        let (shutdown_tx, _shutdown_rx) = mpsc::channel();
        let (_io_done_tx, io_done_rx) = mpsc::channel();
        let mut terminal = TerminalState::new();
        terminal.apply_ssh_handle(
            crate::ssh::SshHandle {
                reader: Box::new(std::io::empty()),
                write_tx,
                resize_tx,
                shutdown_tx,
                io_done_rx,
                bash_runtime: None,
            },
            80,
            24,
        );

        terminal.resize(132, 43);

        assert_eq!(resize_rx.try_recv().unwrap(), (132, 43));
    }

    #[test]
    fn dropping_terminal_signals_ssh_shutdown() {
        let (write_tx, _write_rx) = crate::zmodem::runtime::transport_write_channel(Arc::new(
            crate::zmodem::runtime::ProtocolGate::new(),
        ));
        let (resize_tx, _resize_rx) = mpsc::sync_channel(8);
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let (_io_done_tx, io_done_rx) = mpsc::channel();
        let mut terminal = TerminalState::new();
        terminal.apply_ssh_handle(
            crate::ssh::SshHandle {
                reader: Box::new(std::io::empty()),
                write_tx,
                resize_tx,
                shutdown_tx,
                io_done_rx,
                bash_runtime: None,
            },
            80,
            24,
        );

        drop(terminal);

        assert!(shutdown_rx.try_recv().is_ok());
    }

    #[test]
    fn finish_session_clears_remote_runtime_and_signals_shutdown() {
        let session = completion_session();
        let runtime = crate::bash_integration::RemoteBashRuntime {
            session,
            bash_path: "/bin/bash".into(),
            rc_path: "/tmp/session.rc".into(),
            candidate_path: "/tmp/session.candidate".into(),
            widget_sequence: "\x1b[777;1~".into(),
            snapshot_sequence: "\x1b[778;1~".into(),
        };
        let (write_tx, _write_rx) = crate::zmodem::runtime::transport_write_channel(Arc::new(
            crate::zmodem::runtime::ProtocolGate::new(),
        ));
        let (resize_tx, _resize_rx) = mpsc::sync_channel(8);
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let (_io_done_tx, io_done_rx) = mpsc::channel();
        let mut terminal = TerminalState::new();
        terminal.apply_ssh_handle(
            crate::ssh::SshHandle {
                reader: Box::new(std::io::empty()),
                write_tx,
                resize_tx,
                shutdown_tx,
                io_done_rx,
                bash_runtime: Some(runtime),
            },
            80,
            24,
        );

        terminal.finish_session();

        assert!(shutdown_rx.try_recv().is_ok());
        assert!(terminal.remote_bash_runtime.is_none());
        assert!(terminal.prompt_tracking.is_none());
    }

    // =========================================================================
    // TerminalState::search_lines / reveal_search_line (Task 4 RED-B)
    // Production methods are intentionally missing — these tests must fail to
    // compile until GREEN implements them. Filter: terminal::tests::search_
    // =========================================================================

    /// Build haystack text the same way production search does: skip spacers,
    /// keep primary + zerowidth. Used only to assert contracts without locking
    /// trailing-blank trimming policy.
    fn search_cells_haystack(cells: &[crate::terminal_search::SearchCell]) -> String {
        let mut text = String::new();
        for cell in cells {
            if cell.is_spacer {
                continue;
            }
            text.push(cell.ch);
            text.extend(cell.zerowidth.iter().copied());
        }
        text
    }

    fn search_line_haystack(line: &crate::terminal_search::SearchLine) -> String {
        search_cells_haystack(&line.cells)
    }

    /// Viewport absolute lines [view_top, view_bottom] for current display_offset.
    fn search_viewport_range(term: &alacritty_terminal::term::Term<Listener>) -> (i32, i32) {
        use alacritty_terminal::grid::Dimensions;
        let grid = term.grid();
        let d = grid.display_offset() as i32;
        let screen = grid.screen_lines() as i32;
        let view_top = -d;
        (view_top, view_top + screen - 1)
    }

    fn search_line_in_viewport(term: &alacritty_terminal::term::Term<Listener>, line: i32) -> bool {
        let (top, bottom) = search_viewport_range(term);
        line >= top && line <= bottom
    }

    /// Small grid + unique markers so history lines are easy to find.
    fn search_feed_scrollback_history(terminal: &mut TerminalState, parser: &mut TestProcessor) {
        // rows=3 → after several CRLFs, early markers land in history (Line.0 < 0).
        let payload = b"HIST_NEEDLE_A\r\n\
HIST_NEEDLE_B\r\n\
HIST_NEEDLE_C\r\n\
VIEW_NEEDLE_D\r\n\
VIEW_NEEDLE_E\r\n\
VIEW_NEEDLE_F\r\n";
        terminal.process_pty_output(parser, payload);
    }

    fn visible_grid_is_empty(terminal: &TerminalState) -> bool {
        let term = terminal.term().expect("fixture must initialize term");
        (0..terminal.rows() as i32)
            .all(|line| term.grid()[Line(line)][..].iter().all(|cell| cell.c == ' '))
    }

    #[test]
    fn clear_display_clears_viewport_without_writing_escape_bytes_to_shell() {
        let mut terminal = TerminalState::new();
        terminal.init_term(40, 3);
        let mut parser = TestProcessor::new();
        terminal.process_pty_output(&mut parser, b"VISIBLE\r\nCONTENT");

        terminal.clear_display(false);

        let term = terminal.term().expect("fixture must initialize term");
        assert!(visible_grid_is_empty(&terminal));
        assert_eq!(term.grid().cursor.point, Point::new(Line(0), Column(0)));
        assert!(
            term.grid().history_size() > 0,
            "plain clear must preserve visible content in scrollback"
        );
        assert!(
            terminal.take_pty_write_events().is_empty(),
            "clear control sequences must never be sent to the shell"
        );
    }

    #[test]
    fn clear_display_with_scrollback_removes_viewport_and_history() {
        let mut terminal = TerminalState::new();
        terminal.init_term(40, 3);
        let mut parser = TestProcessor::new();
        search_feed_scrollback_history(&mut terminal, &mut parser);
        assert!(
            terminal
                .term()
                .expect("fixture must initialize term")
                .grid()
                .history_size()
                > 0
        );

        terminal.scroll_offset = 2;
        terminal.clear_display(true);

        let term = terminal.term().expect("fixture must initialize term");
        assert!(visible_grid_is_empty(&terminal));
        assert_eq!(term.grid().history_size(), 0);
        assert_eq!(term.grid().cursor.point, Point::new(Line(0), Column(0)));
        assert_eq!(terminal.scroll_offset, 0);
        assert!(
            terminal.take_pty_write_events().is_empty(),
            "clear control sequences must never be sent to the shell"
        );
    }

    #[test]
    fn search_lines_uninitialized_returns_empty_and_reveal_does_not_panic() {
        let mut terminal = TerminalState::new();

        let lines = terminal.search_lines();
        assert!(
            lines.is_empty(),
            "uninitialized TerminalState (term=None) must yield empty search snapshot"
        );

        // reveal must be a no-op, never panic, for any absolute line.
        terminal.reveal_search_line(0);
        terminal.reveal_search_line(-1);
        terminal.reveal_search_line(i32::MIN);
        terminal.reveal_search_line(i32::MAX);
    }

    #[test]
    fn search_lines_covers_history_range_with_negative_line_and_find_matches() {
        use alacritty_terminal::grid::Dimensions;

        let mut terminal = TerminalState::new();
        terminal.init_term(40, 3);
        let mut parser = TestProcessor::new();
        search_feed_scrollback_history(&mut terminal, &mut parser);

        let term = terminal.term().expect("init_term must create term");
        let grid = term.grid();
        let top = grid.topmost_line().0;
        let bottom = grid.bottommost_line().0;
        assert!(
            top < 0,
            "fixture must produce scrollback history (topmost_line < 0), got top={top}"
        );
        assert!(grid.history_size() > 0);

        let lines = terminal.search_lines();
        assert!(
            !lines.is_empty(),
            "search_lines must cover history + visible screen"
        );
        assert_eq!(
            lines.first().map(|l| l.line),
            Some(top),
            "first SearchLine.line must equal grid.topmost_line().0"
        );
        assert_eq!(
            lines.last().map(|l| l.line),
            Some(bottom),
            "last SearchLine.line must equal grid.bottommost_line().0"
        );
        assert_eq!(
            lines.len(),
            (bottom - top + 1) as usize,
            "search_lines must be contiguous topmost..=bottommost"
        );
        for (i, sl) in lines.iter().enumerate() {
            assert_eq!(
                sl.line,
                top + i as i32,
                "SearchLine.line must equal alacritty Line.0 for each row"
            );
        }
        assert!(
            lines.iter().any(|l| l.line < 0),
            "snapshot must include at least one negative history line"
        );

        let matches = crate::terminal_search::find_matches(&lines, "HIST_NEEDLE_A", false);
        assert_eq!(
            matches.len(),
            1,
            "find_matches must locate the history needle via search_lines snapshot"
        );
        assert!(
            matches[0].line < 0,
            "HIST_NEEDLE_A must live on a negative history line, got {}",
            matches[0].line
        );
        assert_eq!(matches[0].start_col, 0);
        assert_eq!(matches[0].end_col, "HIST_NEEDLE_A".len());
    }

    #[test]
    fn search_lines_maps_cjk_wide_spacer_and_zerowidth_for_find_matches() {
        let mut terminal = TerminalState::new();
        terminal.init_term(20, 5);
        let mut parser = TestProcessor::new();
        // "a中b" → a@0, 中(WIDE_CHAR)@1, WIDE_CHAR_SPACER@2, b@3
        // then a combining sequence on the next line: Z + U+0301
        let acute = '\u{0301}';
        let mut payload = String::from("a中b\r\n");
        payload.push('Z');
        payload.push(acute);
        payload.push_str("tail\r\n");
        terminal.process_pty_output(&mut parser, payload.as_bytes());

        let lines = terminal.search_lines();
        assert!(!lines.is_empty());

        // --- CJK wide primary + spacer ---
        let cjk_line = lines
            .iter()
            .find(|l| search_line_haystack(l).contains('中'))
            .expect("search_lines must include the CJK fixture line");

        let wide = cjk_line
            .cells
            .iter()
            .find(|c| c.ch == '中' && !c.is_spacer)
            .expect("wide CJK primary cell");
        assert_eq!(
            wide.width, 2,
            "WIDE_CHAR primary must report display width 2"
        );
        assert_eq!(wide.col, 1, "fixture places 中 after leading 'a'");

        let spacer = cjk_line
            .cells
            .iter()
            .find(|c| c.col == wide.col + 1)
            .expect("column after wide primary must be present (WIDE_CHAR_SPACER)");
        assert!(
            spacer.is_spacer,
            "WIDE_CHAR_SPACER must be marked is_spacer so it is not haystack text"
        );

        let b_cell = cjk_line
            .cells
            .iter()
            .find(|c| c.ch == 'b' && !c.is_spacer)
            .expect("trailing ASCII 'b'");
        assert_eq!(b_cell.col, wide.col + 2);

        let cjk_matches =
            crate::terminal_search::find_matches(std::slice::from_ref(cjk_line), "中b", false);
        assert_eq!(
            cjk_matches,
            vec![crate::terminal_search::SearchMatch {
                line: cjk_line.line,
                start_col: 1,
                end_col: 4, // half-open: covers 中@1, spacer@2, b@3
            }],
            "query 中b column range must account for wide primary + spacer"
        );

        // --- combining zerowidth on primary ---
        let zw_line = lines
            .iter()
            .find(|l| search_line_haystack(l).contains('Z'))
            .expect("search_lines must include zerowidth fixture line");
        let primary = zw_line
            .cells
            .iter()
            .find(|c| c.ch == 'Z' && !c.is_spacer)
            .expect("primary cell for Z");
        assert!(
            primary.zerowidth.contains(&acute),
            "combining mark U+0301 must stay on the primary cell, got {:?}",
            primary.zerowidth
        );

        let query: String = ['Z', acute].into_iter().collect();
        let zw_matches =
            crate::terminal_search::find_matches(std::slice::from_ref(zw_line), &query, false);
        assert_eq!(
            zw_matches,
            vec![crate::terminal_search::SearchMatch {
                line: zw_line.line,
                start_col: primary.col,
                end_col: primary.col + primary.width.max(1),
            }],
            "zerowidth must map back to the primary column (no invented columns)"
        );
    }

    #[test]
    fn search_reveal_search_line_history_bottom_and_clamp_no_panic() {
        use alacritty_terminal::grid::Dimensions;

        let mut terminal = TerminalState::new();
        terminal.init_term(40, 3);
        let mut parser = TestProcessor::new();
        search_feed_scrollback_history(&mut terminal, &mut parser);

        {
            let grid = terminal.term().unwrap().grid();
            assert!(grid.history_size() > 0);
            assert_eq!(
                grid.display_offset(),
                0,
                "fresh scroll ends at bottom (display_offset=0)"
            );
        }

        let lines = terminal.search_lines();
        let hist_line = crate::terminal_search::find_matches(&lines, "HIST_NEEDLE_A", false)
            .into_iter()
            .next()
            .expect("history needle")
            .line;
        assert!(hist_line < 0);

        // 1) Reveal history match → scroll up so display_offset > 0 and line in viewport.
        terminal.reveal_search_line(hist_line);
        {
            let term = terminal.term().unwrap();
            let grid = term.grid();
            assert!(
                grid.display_offset() > 0,
                "revealing a history line must increase display_offset"
            );
            assert!(
                search_line_in_viewport(term, hist_line),
                "revealed history line {hist_line} must fall inside viewport {:?}",
                search_viewport_range(term)
            );
        }

        // 2) Reveal bottommost (currently off-screen after scrolling up) → scroll back down.
        let bottom = terminal.term().unwrap().grid().bottommost_line().0;
        {
            let term = terminal.term().unwrap();
            assert!(
                !search_line_in_viewport(term, bottom),
                "precondition: bottommost line {bottom} should be below history viewport {:?}",
                search_viewport_range(term)
            );
        }
        terminal.reveal_search_line(bottom);
        {
            let term = terminal.term().unwrap();
            assert!(
                search_line_in_viewport(term, bottom),
                "reveal bottommost must bring line {bottom} into viewport {:?}",
                search_viewport_range(term)
            );
            // Fully scrolled to live edge.
            assert_eq!(term.grid().display_offset(), 0);
        }

        // Also reveal a still-visible mid-screen line: must remain non-panicking.
        let mid = bottom; // already visible
        terminal.reveal_search_line(mid);
        assert!(search_line_in_viewport(terminal.term().unwrap(), mid));

        // 3) Extreme out-of-range absolute lines clamp; never panic.
        terminal.reveal_search_line(i32::MIN);
        terminal.reveal_search_line(i32::MAX);
        terminal.reveal_search_line(topmost_after_clamp_probe(&terminal) - 10_000);
        terminal.reveal_search_line(bottom + 10_000);
        {
            let grid = terminal.term().unwrap().grid();
            assert!(
                grid.display_offset() <= grid.history_size(),
                "clamp must keep display_offset within history"
            );
        }
    }

    fn topmost_after_clamp_probe(terminal: &TerminalState) -> i32 {
        use alacritty_terminal::grid::Dimensions;
        terminal.term().unwrap().grid().topmost_line().0
    }

    #[test]
    fn search_lines_trailing_blanks_must_not_treat_spacer_as_text() {
        let mut terminal = TerminalState::new();
        terminal.init_term(24, 4);
        let mut parser = TestProcessor::new();
        // Wide char near end of content; trailing columns may be blank.
        terminal.process_pty_output(&mut parser, "pre中end   \r\n".as_bytes());

        let lines = terminal.search_lines();
        let line = lines
            .iter()
            .find(|l| search_line_haystack(l).contains('中'))
            .expect("CJK line present");

        // Spacers must never appear in haystack text (whether trailing blanks are
        // kept or trimmed is intentionally unconstrained).
        let haystack = search_line_haystack(line);
        assert!(
            haystack.contains('中') && haystack.contains("pre") && haystack.contains("end"),
            "haystack should keep real text, got {haystack:?}"
        );
        assert!(
            !haystack.contains('\0'),
            "spacer placeholder must not leak into search text"
        );

        for cell in &line.cells {
            if cell.is_spacer {
                // Implementation may use any placeholder char on spacers; only
                // is_spacer matters for haystack construction.
                assert_eq!(
                    cell.width.max(1),
                    1,
                    "spacer occupies exactly one grid column"
                );
            }
        }

        // If spacer were treated as text (e.g. NUL or space at wrong col), column
        // mapping for 中+following ASCII would drift. Lock the half-open range.
        let matches =
            crate::terminal_search::find_matches(std::slice::from_ref(line), "中e", false);
        assert_eq!(matches.len(), 1, "中e must match across wide+spacer");
        let m = &matches[0];
        let wide = line
            .cells
            .iter()
            .find(|c| c.ch == '中' && !c.is_spacer)
            .expect("wide primary");
        let e_cell = line
            .cells
            .iter()
            .find(|c| c.ch == 'e' && !c.is_spacer && c.col > wide.col)
            .expect("'e' after the wide character");
        assert_eq!(m.start_col, wide.col);
        assert_eq!(
            m.end_col,
            e_cell.col + e_cell.width.max(1),
            "end_col half-open must cover final primary, not a spacer-as-text artifact"
        );
    }

    #[test]
    fn active_zmodem_rejects_input_without_delayed_replay() {
        let (writer, writes) = test_transport_capture();
        let mut terminal = TerminalState::new();
        terminal.writer = Some(writer);
        terminal.zmodem_input_gate.activate();

        terminal.write_input("discarded");
        assert!(terminal.try_write_input("rejected").is_err());
        assert!(writes.try_recv().is_err());

        terminal.zmodem_input_gate.deactivate();
        terminal.write_input("normal");
        assert_eq!(
            writes.recv_timeout(Duration::from_secs(1)).unwrap(),
            b"normal"
        );
        assert!(writes.try_recv().is_err());
    }

    #[test]
    fn exclusive_protocol_gate_drops_normal_backlog_without_late_replay() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let gate = Arc::new(crate::zmodem::runtime::ProtocolGate::new());
        let (writer, protocol) = spawn_writer_worker_with_protocol(
            Box::new(BarrierCaptureWriter {
                captured: Arc::clone(&captured),
                entered_tx: Some(entered_tx),
                release_rx,
            }),
            Arc::clone(&gate),
        );
        writer.try_send_normal(b"first".to_vec()).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        writer.try_send_normal(b"stale-backlog".to_vec()).unwrap();
        gate.activate();
        let protocol_thread = std::thread::spawn(move || {
            protocol.write_and_flush(b"protocol", Duration::from_secs(1))
        });
        release_tx.send(()).unwrap();
        protocol_thread.join().unwrap().unwrap();
        gate.deactivate();

        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(&*captured.lock().unwrap(), b"firstprotocol");
    }
