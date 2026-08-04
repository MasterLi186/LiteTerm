    fn native_selection_text(
        output: &str,
        start: (usize, usize),
        end: (usize, usize),
    ) -> String {
        let mut terminal = selection_fixture(output);
        let start = terminal.visual_point_to_grid_point(start).unwrap();
        let end = terminal.visual_point_to_grid_point(end).unwrap();
        let kind = if start == end {
            TerminalSelectionKind::Semantic
        } else {
            TerminalSelectionKind::Simple
        };
        terminal.begin_selection(start, kind);
        if start != end {
            terminal.update_selection(end);
        }
        terminal.current_selection_text()
    }

    #[test]
    fn selection_text_keeps_adjacent_cjk_without_spacer_spaces() {
        assert_eq!(native_selection_text("中文", (0, 1), (3, 1)), "中文");
    }

    #[test]
    fn selection_text_keeps_mixed_ascii_and_cjk_compact() {
        assert_eq!(
            native_selection_text("A中B", (usize::MAX, 1), (0, 1)),
            "A中B"
        );
    }

    #[test]
    fn selection_text_preserves_real_internal_space() {
        assert_eq!(native_selection_text("中 文", (0, 1), (4, 1)), "中 文");
    }

    #[test]
    fn selection_text_keeps_emoji_without_spacer_space() {
        assert_eq!(native_selection_text("A😀B", (0, 1), (3, 1)), "A😀B");
    }

    #[test]
    fn selection_text_preserves_zero_width_combining_character() {
        assert_eq!(
            native_selection_text("e\u{301}", (0, 1), (0, 1)),
            "e\u{301}"
        );
    }

    #[test]
    fn selection_text_starting_at_wide_spacer_includes_primary_character() {
        assert_eq!(native_selection_text("中", (1, 1), (1, 1)), "中");
    }

    #[test]
    fn absolute_selection_keeps_the_same_text_after_viewport_scroll() {
        use alacritty_terminal::grid::Scroll;

        let mut terminal = selection_fixture("one\r\ntwo\r\nthree\r\nfour\r\nfive");
        let start = terminal
            .visual_point_to_grid_point((0, 1))
            .expect("visible start must map into scrollback");
        let end = terminal
            .visual_point_to_grid_point((3, 1))
            .expect("visible end must map into scrollback");
        terminal.begin_selection(start, TerminalSelectionKind::Simple);
        terminal.update_selection(end);
        let selected = terminal.current_selection_text();
        assert!(!selected.is_empty());

        terminal
            .term_mut()
            .expect("fixture must initialize term")
            .scroll_display(Scroll::Delta(2));

        assert_ne!(terminal.visual_point_to_grid_point((0, 1)), Some(start));
        assert_eq!(terminal.current_selection_text(), selected);
    }

    #[test]
    fn wheel_scrolling_during_drag_can_extend_selection_beyond_one_viewport() {
        use alacritty_terminal::grid::Scroll;

        let mut terminal = selection_fixture(
            "line-00\r\nline-01\r\nline-02\r\nline-03\r\nline-04\r\nline-05\r\nline-06\r\nline-07\r\nline-08\r\nline-09",
        );
        let anchor = terminal
            .visual_point_to_grid_point((7, 3))
            .expect("drag anchor must be visible");

        terminal
            .term_mut()
            .expect("fixture must initialize term")
            .scroll_display(Scroll::Top);
        let current = terminal
            .visual_point_to_grid_point((0, 1))
            .expect("scrolled drag point must map into history");

        assert!(
            anchor.1.abs_diff(current.1) > u32::from(terminal.rows()),
            "wheel-assisted drag should span more lines than the visible viewport"
        );
        terminal.begin_selection(anchor, TerminalSelectionKind::Simple);
        terminal.update_selection(current);
        assert!(terminal.current_selection_text().lines().count() > 4);
    }

    #[test]
    fn shift_click_without_visible_selection_uses_live_cursor_not_stale_mouse_anchor() {
        use alacritty_terminal::grid::Scroll;

        let output = (0..340)
            .map(|line| format!("line-{line:03}\r\n"))
            .collect::<String>();
        let mut terminal = selection_fixture(&output);
        terminal.term_mut().unwrap().scroll_display(Scroll::Top);
        let stale_mouse_anchor = terminal.visual_point_to_grid_point((0, 1)).unwrap();
        let clicked = terminal.visual_point_to_grid_point((7, 2)).unwrap();
        let live_cursor = terminal.term().unwrap().grid().cursor.point;

        assert!(terminal.begin_selection(stale_mouse_anchor, TerminalSelectionKind::Simple));
        assert!(terminal.has_selection_anchor());
        assert!(terminal.selection_range().is_none());
        assert!(terminal.current_selection_text().is_empty());

        assert!(terminal.shift_extend_selection(clicked));

        let range = terminal.selection_range().expect("Shift extension must become visible");
        assert!(range.start.line.0.abs_diff(range.end.line.0) >= 300);
        assert!(terminal.current_selection_text().lines().count() >= 300);
        assert!(range.contains(live_cursor));
    }

    #[test]
    fn shift_click_extends_an_existing_visible_selection_instead_of_live_cursor() {
        let mut terminal = selection_fixture("alpha beta\r\ntail");
        let alpha = terminal.visual_point_to_grid_point((2, 1)).unwrap();
        let beta = terminal.visual_point_to_grid_point((8, 1)).unwrap();
        terminal.begin_selection(alpha, TerminalSelectionKind::Semantic);
        assert_eq!(terminal.current_selection_text(), "alpha");

        assert!(terminal.shift_extend_selection(beta));
        assert_eq!(terminal.current_selection_text(), "alpha beta");
    }

    #[test]
    fn same_cell_pointer_jitter_keeps_a_plain_click_anchor_invisible() {
        let mut terminal = selection_fixture("alpha");
        let point = terminal.visual_point_to_grid_point((2, 1)).unwrap();
        terminal.begin_selection(point, TerminalSelectionKind::Simple);

        assert!(!terminal.update_selection(point));
        assert!(terminal.has_selection_anchor());
        assert!(terminal.selection_range().is_none());
        assert!(terminal.current_selection_text().is_empty());
    }

    #[test]
    fn semantic_and_line_selections_keep_their_mode_while_dragging() {
        let mut terminal = selection_fixture("alpha beta\r\ngamma delta");
        let alpha = terminal.visual_point_to_grid_point((2, 1)).unwrap();
        let beta = terminal.visual_point_to_grid_point((8, 1)).unwrap();
        terminal.begin_selection(alpha, TerminalSelectionKind::Semantic);
        assert_eq!(terminal.current_selection_text(), "alpha");
        terminal.update_selection(beta);
        assert_eq!(terminal.current_selection_text(), "alpha beta");

        let first_line = terminal.visual_point_to_grid_point((2, 1)).unwrap();
        let second_line = terminal.visual_point_to_grid_point((2, 2)).unwrap();
        terminal.begin_selection(first_line, TerminalSelectionKind::Lines);
        terminal.update_selection(second_line);
        let selected = terminal.current_selection_text();
        assert!(selected.contains("alpha beta"));
        assert!(selected.contains("gamma delta"));
    }

    #[test]
    fn block_selection_extracts_only_the_selected_columns() {
        let mut terminal = selection_fixture("abcd\r\nefgh\r\nijkl");
        let start = terminal.visual_point_to_grid_point((1, 1)).unwrap();
        let end = terminal.visual_point_to_grid_point((3, 2)).unwrap();
        terminal.begin_selection(start, TerminalSelectionKind::Block);
        terminal.update_selection(end);

        assert_eq!(terminal.current_selection_text(), "bcd\nfgh");
        assert!(terminal.selection_range().unwrap().is_block);
    }

    #[test]
    fn native_selection_tracks_output_scroll_and_clears_on_column_reflow() {
        let mut terminal = selection_fixture("anchor word\r\nsecond\r\nthird");
        let start = terminal.visual_point_to_grid_point((0, 1)).unwrap();
        terminal.begin_selection(start, TerminalSelectionKind::Semantic);
        assert_eq!(terminal.current_selection_text(), "anchor");

        let mut parser = TestProcessor::new();
        terminal.process_pty_output(&mut parser, b"\r\nfourth\r\nfifth");
        assert_eq!(terminal.current_selection_text(), "anchor");

        terminal.resize(30, 4);
        assert!(!terminal.has_selection_anchor());
        assert!(terminal.current_selection_text().is_empty());
    }

    #[test]
    fn history_eviction_drops_even_an_invisible_single_click_anchor() {
        use alacritty_terminal::grid::Scroll;

        let mut terminal = selection_fixture("");
        let mut config = TermConfig::default();
        config.scrolling_history = 8;
        config.semantic_escape_chars = SEMANTIC_SELECTION_DELIMITERS.to_owned();
        terminal.term_mut().unwrap().set_options(config);
        let mut parser = TestProcessor::new();
        let initial = (0..12)
            .map(|line| format!("old-{line}\r\n"))
            .collect::<String>();
        terminal.process_pty_output(&mut parser, initial.as_bytes());
        terminal.term_mut().unwrap().scroll_display(Scroll::Top);
        let anchor = terminal.visual_point_to_grid_point((0, 1)).unwrap();
        terminal.begin_selection(anchor, TerminalSelectionKind::Simple);
        assert!(terminal.has_selection_anchor());

        terminal.term_mut().unwrap().scroll_display(Scroll::Bottom);
        let replacement = (0..20)
            .map(|line| format!("new-{line}\r\n"))
            .collect::<String>();
        terminal.process_pty_output(&mut parser, replacement.as_bytes());

        assert!(!terminal.has_selection_anchor());
    }

    #[test]
    fn alternate_screen_and_clear_scrollback_invalidate_native_selection() {
        let mut terminal = selection_fixture("anchor word");
        let point = terminal.visual_point_to_grid_point((0, 1)).unwrap();
        terminal.begin_selection(point, TerminalSelectionKind::Semantic);
        let mut parser = TestProcessor::new();

        terminal.process_pty_output(&mut parser, b"\x1b[?1049h");
        assert!(!terminal.has_selection_anchor());

        terminal.process_pty_output(&mut parser, b"\x1b[?1049lmore");
        let point = terminal.visual_point_to_grid_point((0, 1)).unwrap();
        terminal.begin_selection(point, TerminalSelectionKind::Simple);
        terminal.update_selection(point);
        terminal.clear_display(true);
        assert!(!terminal.has_selection_anchor());
    }

    #[test]
    fn native_selection_of_a_wide_spacer_copies_the_primary_cjk_glyph() {
        let mut terminal = selection_fixture("中");
        let primary = terminal.visual_point_to_grid_point((0, 1)).unwrap();
        let spacer = terminal.visual_point_to_grid_point((1, 1)).unwrap();
        terminal.begin_selection(primary, TerminalSelectionKind::Simple);
        terminal.update_selection(spacer);

        assert_eq!(terminal.current_selection_text(), "中");
    }

    fn install_local_snapshot_runtime(
        terminal: &mut TerminalState,
    ) -> (TestTransportCapture, Vec<u8>) {
        let runtime = LocalBashRuntime::create(completion_session()).unwrap();
        let expected = runtime.snapshot_sequence().as_bytes().to_vec();
        let (write_tx, write_rx) = test_transport_capture();
        terminal.writer = Some(write_tx);
        terminal.local_bash_runtime = Some(runtime);
        (write_rx, expected)
    }

    struct SharedWriter {
        captured: Arc<Mutex<Vec<u8>>>,
        completed_tx: mpsc::Sender<()>,
    }

    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.captured.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            let _ = self.completed_tx.send(());
            Ok(())
        }
    }

    struct FailingWriter {
        attempt_tx: mpsc::Sender<()>,
        dropped_tx: mpsc::Sender<()>,
    }

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            let _ = self.attempt_tx.send(());
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "writer disconnected",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "writer disconnected",
            ))
        }
    }

    impl Drop for FailingWriter {
        fn drop(&mut self) {
            let _ = self.dropped_tx.send(());
        }
    }

    struct BlockingWriter {
        entered_tx: mpsc::Sender<()>,
        release_rx: mpsc::Receiver<()>,
    }

    struct BarrierCaptureWriter {
        captured: Arc<Mutex<Vec<u8>>>,
        entered_tx: Option<mpsc::Sender<()>>,
        release_rx: mpsc::Receiver<()>,
    }

    impl Write for BarrierCaptureWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if let Some(entered_tx) = self.entered_tx.take() {
                let _ = entered_tx.send(());
                let _ = self.release_rx.recv();
            }
            self.captured.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Write for BlockingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let _ = self.entered_tx.send(());
            let _ = self.release_rx.recv();
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct BrokenReader;

    impl Read for BrokenReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "reader failed"))
        }
    }

    #[derive(Debug)]
    struct SlowReapChild {
        polls: usize,
    }

    impl portable_pty::ChildKiller for SlowReapChild {
        fn kill(&mut self) -> std::io::Result<()> {
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
            Box::new(Self { polls: self.polls })
        }
    }

    impl portable_pty::Child for SlowReapChild {
        fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
            self.polls += 1;
            Ok((self.polls >= 30).then(|| portable_pty::ExitStatus::with_exit_code(0)))
        }

        fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
            Ok(portable_pty::ExitStatus::with_exit_code(0))
        }

        fn process_id(&self) -> Option<u32> {
            Some(1234)
        }
    }

    #[derive(Debug)]
    struct InitialWaitErrorChild {
        wait_calls: usize,
        killed: Arc<std::sync::atomic::AtomicBool>,
        reaped: Arc<std::sync::atomic::AtomicBool>,
    }

    impl portable_pty::ChildKiller for InitialWaitErrorChild {
        fn kill(&mut self) -> std::io::Result<()> {
            self.killed.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
            Box::new(Self {
                wait_calls: self.wait_calls,
                killed: Arc::clone(&self.killed),
                reaped: Arc::clone(&self.reaped),
            })
        }
    }

    impl portable_pty::Child for InitialWaitErrorChild {
        fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
            self.wait_calls += 1;
            if self.wait_calls == 1 {
                return Err(std::io::Error::other("transient wait failure"));
            }
            if self.killed.load(std::sync::atomic::Ordering::SeqCst) {
                self.reaped.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(Some(portable_pty::ExitStatus::with_exit_code(0)))
            } else {
                Ok(None)
            }
        }

        fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
            self.reaped.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(portable_pty::ExitStatus::with_exit_code(0))
        }

        fn process_id(&self) -> Option<u32> {
            Some(5678)
        }
    }

    #[derive(Debug)]
    struct ReaperFallbackChild {
        kill_fails: bool,
        killed: Arc<std::sync::atomic::AtomicBool>,
        waited: Arc<std::sync::atomic::AtomicBool>,
    }

    impl portable_pty::ChildKiller for ReaperFallbackChild {
        fn kill(&mut self) -> std::io::Result<()> {
            self.killed.store(true, std::sync::atomic::Ordering::SeqCst);
            if self.kill_fails {
                Err(std::io::Error::other("kill failed"))
            } else {
                Ok(())
            }
        }

        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
            Box::new(Self {
                kill_fails: self.kill_fails,
                killed: Arc::clone(&self.killed),
                waited: Arc::clone(&self.waited),
            })
        }
    }

    impl portable_pty::Child for ReaperFallbackChild {
        fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
            Ok(None)
        }

        fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
            self.waited.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(portable_pty::ExitStatus::with_exit_code(0))
        }

        fn process_id(&self) -> Option<u32> {
            Some(9012)
        }
    }

    fn reaper_fallback_child(
        kill_fails: bool,
    ) -> (
        Box<dyn portable_pty::Child + Send + Sync>,
        Arc<std::sync::atomic::AtomicBool>,
        Arc<std::sync::atomic::AtomicBool>,
    ) {
        let killed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let waited = Arc::new(std::sync::atomic::AtomicBool::new(false));
        (
            Box::new(ReaperFallbackChild {
                kill_fails,
                killed: Arc::clone(&killed),
                waited: Arc::clone(&waited),
            }),
            killed,
            waited,
        )
    }

    fn advance_and_take(
        terminal: &mut TerminalState,
        parser: &mut TestProcessor,
        bytes: &[u8],
    ) -> Vec<String> {
        parser.advance(terminal.term.as_mut().unwrap(), bytes);
        terminal.take_pty_write_events()
    }

    #[test]
    fn new_terminal_has_no_local_bash_runtime() {
        assert!(TerminalState::new().local_bash_runtime.is_none());
    }

    #[test]
    fn try_write_input_distinguishes_missing_and_disconnected_writer() {
        let mut terminal = TerminalState::new();
        assert!(terminal
            .try_write_input("echo")
            .unwrap_err()
            .contains("尚未就绪"));
        let (sender, receiver) = crate::zmodem::runtime::transport_write_channel(Arc::new(
            crate::zmodem::runtime::ProtocolGate::new(),
        ));
        drop(receiver);
        terminal.writer = Some(sender);
        assert!(terminal
            .try_write_input("echo")
            .unwrap_err()
            .contains("已断开"));
    }

    #[test]
    fn link_lookup_uses_visual_row_mapping_and_rejects_remote_paths() {
        let mut terminal = TerminalState::new();
        terminal.init_term(80, 4);
        let mut parser = TestProcessor::new();
        terminal.process_pty_output(&mut parser, b"https://example.com /tmp/test.txt");

        assert!(matches!(
            terminal.link_at_visual(1, 3, false).unwrap().target,
            crate::terminal_links::LinkTarget::Url(_)
        ));
        assert!(terminal.link_at_visual(1, 22, false).is_none());
        assert!(matches!(
            terminal.link_at_visual(1, 22, true).unwrap().target,
            crate::terminal_links::LinkTarget::LocalPath { .. }
        ));
    }

    #[test]
    fn test_bash_child_environment_stays_inside_runtime_private_directory() {
        let runtime = LocalBashRuntime::create(completion_session()).unwrap();

        let environment = isolated_test_bash_environment(&runtime);
        let mut command = CommandBuilder::new("/bin/bash");
        configure_isolated_test_bash_environment(&mut command, &runtime);

        assert_eq!(environment.home.as_path(), runtime.temp_dir());
        for (name, path) in [
            ("HOME", &environment.home),
            ("HISTFILE", &environment.histfile),
            ("INPUTRC", &environment.inputrc),
            ("BASH_ENV", &environment.bash_env),
        ] {
            assert!(path.starts_with(runtime.temp_dir()));
            assert_eq!(command.get_env(name), Some(path.as_os_str()));
        }
    }

    #[test]
    fn local_bash_shutdown_reaps_child_releases_runtime_and_stops_read_loop() {
        let terminal = Arc::new(Mutex::new(TerminalState::new()));
        {
            let mut state = terminal.lock().unwrap();
            state.spawn_shell_with_path("/bin/bash", 80, 24, completion_session());
        }
        let (pid, temp_dir) = {
            let state = terminal.lock().unwrap();
            (
                state
                    .local_child
                    .as_ref()
                    .and_then(|child| child.process_id())
                    .expect("本地 Bash 应有 PID"),
                state
                    .local_bash_runtime
                    .as_ref()
                    .unwrap()
                    .temp_dir()
                    .to_path_buf(),
            )
        };
        let (done_tx, done_rx) = mpsc::channel();
        let read_terminal = Arc::clone(&terminal);
        let read_thread = std::thread::spawn(move || {
            read_loop(read_terminal, || {}, |_| {});
            let _ = done_tx.send(());
        });
        let reader_deadline = std::time::Instant::now() + Duration::from_secs(1);
        while terminal.lock().unwrap().pty_reader.is_some()
            && std::time::Instant::now() < reader_deadline
        {
            std::thread::yield_now();
        }
        assert!(
            terminal.lock().unwrap().pty_reader.is_none(),
            "read loop 应先取得 reader"
        );

        terminal.lock().unwrap().shutdown();

        done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("shutdown 后 read loop 应有界退出");
        read_thread.join().unwrap();
        assert!(!temp_dir.exists(), "shutdown 应释放本地 runtime 私有目录");
        let process_deadline = std::time::Instant::now() + Duration::from_secs(2);
        let process_exists = loop {
            let exists = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0
                || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH);
            if !exists || std::time::Instant::now() >= process_deadline {
                break exists;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(!process_exists, "shutdown 应 kill 并 reap 本地 Bash");
    }

    #[test]
    fn real_bash_large_transcript_terminal_queries_do_not_pollute_next_command() {
        let terminal = Arc::new(Mutex::new(TerminalState::new()));
        let session = completion_session();
        {
            terminal
                .lock()
                .unwrap()
                .spawn_shell_with_path("/bin/bash", 212, 48, session.clone());
        }
        let probe = terminal
            .lock()
            .unwrap()
            .local_bash_runtime
            .as_ref()
            .unwrap()
            .temp_dir()
            .join("terminal-reply-probe");
        let mut reader = terminal.lock().unwrap().take_reader().unwrap();
        let (output_tx, output_rx) = mpsc::channel();
        let reader_thread = std::thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            while let Ok(length) = reader.read(&mut buffer) {
                if length == 0 || output_tx.send(buffer[..length].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut marker_decoder = MarkerDecoder::new(session);
        let mut parser = TestProcessor::new();
        let mut prompts = 0;
        let mut observed = Vec::new();
        let mut transcript_sent = false;
        let mut probe_sent = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(8);

        while prompts < 3 && std::time::Instant::now() < deadline {
            let Ok(bytes) = output_rx.recv_timeout(Duration::from_millis(250)) else {
                continue;
            };
            observed.extend_from_slice(&bytes);
            if observed.len() > 4096 {
                observed.drain(..observed.len() - 4096);
            }
            prompts += marker_decoder
                .scan(&bytes)
                .iter()
                .filter(|boundary| boundary.kind == MarkerKind::Prompt)
                .count();
            terminal
                .lock()
                .unwrap()
                .process_pty_output(&mut parser, &bytes);

            if prompts == 1 && !transcript_sent {
                transcript_sent = true;
                let mut terminal = terminal.lock().unwrap();
                terminal.take_bash_submission();
                terminal.write_input(
                    "bash --noprofile --norc -c \"stty raw -echo; head -c 131072 /dev/zero | tr '\\0' x; printf '\\033[c\\033[c\\033[c\\033[c\\033[1;212H\\033[6n'; sleep 0.05; stty sane\"\r",
                );
            } else if prompts == 2 && !probe_sent {
                probe_sent = true;
                let probe = probe.to_string_lossy();
                assert!(!probe.contains('\''));
                let mut terminal = terminal.lock().unwrap();
                terminal.take_bash_submission();
                terminal.write_input(&format!("printf __TERMINAL_REPLY_OK__ > '{probe}'\r"));
            }
        }

        assert_eq!(prompts, 3, "真实 Bash PTY 应完成测试命令并返回提示符");
        assert_eq!(
            std::fs::read_to_string(&probe).unwrap_or_else(|error| {
                panic!(
                    "探针命令未执行（{error}），PTY 输出：{}",
                    String::from_utf8_lossy(&observed).escape_debug()
                )
            }),
            "__TERMINAL_REPLY_OK__",
            "raw 子会话日志中的 DA/DSR 应答不得残留并污染下一条 Bash 命令"
        );
        terminal.lock().unwrap().shutdown();
        reader_thread.join().unwrap();
    }

    #[test]
    fn shutdown_hands_slow_child_reaping_off_without_blocking_ui_mutex() {
        let mut terminal = TerminalState::new();
        terminal.local_child = Some(Box::new(SlowReapChild { polls: 0 }));
        let started = std::time::Instant::now();

        terminal.shutdown();

        assert!(
            started.elapsed() < Duration::from_millis(200),
            "shutdown 不得在 UI mutex 内等待 child reap"
        );
        assert!(terminal.local_child.is_none());
    }

    #[test]
    fn shutdown_still_kills_and_reaps_after_initial_try_wait_error() {
        let killed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reaped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut terminal = TerminalState::new();
        terminal.local_child = Some(Box::new(InitialWaitErrorChild {
            wait_calls: 0,
            killed: Arc::clone(&killed),
            reaped: Arc::clone(&reaped),
        }));

        terminal.shutdown();

        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !reaped.load(std::sync::atomic::Ordering::SeqCst)
            && std::time::Instant::now() < deadline
        {
            std::thread::yield_now();
        }
        assert!(killed.load(std::sync::atomic::Ordering::SeqCst));
        assert!(reaped.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn local_reaper_unavailable_kills_and_waits_synchronously() {
        let (child, killed, waited) = reaper_fallback_child(false);

        enqueue_local_child_or_wait(child, None);

        assert!(killed.load(std::sync::atomic::Ordering::SeqCst));
        assert!(waited.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn local_reaper_disconnected_queue_recovers_child_and_waits() {
        let (sender, receiver) = mpsc::channel();
        drop(receiver);
        let (child, killed, waited) = reaper_fallback_child(false);

        enqueue_local_child_or_wait(child, Some(&sender));

        assert!(killed.load(std::sync::atomic::Ordering::SeqCst));
        assert!(waited.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn local_reaper_kill_error_still_reaches_final_wait() {
        let (child, killed, waited) = reaper_fallback_child(true);

        enqueue_local_child_or_wait(child, None);

        assert!(killed.load(std::sync::atomic::Ordering::SeqCst));
        assert!(waited.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn prompt_marker_boundary_makes_same_chunk_suffix_current_input() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice(b"git");

        let events = terminal.process_pty_output(&mut parser, &output);

        assert!(events.is_empty());
        assert_eq!(terminal.current_bash_input().as_deref(), Some("git"));
    }

    #[test]
    fn exact_width_input_includes_the_pending_wrap_cell() {
        let mut terminal = tracked_terminal(3, 4);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice(b"git");

        terminal.process_pty_output(&mut parser, &output);

        assert_eq!(terminal.current_bash_input().as_deref(), Some("git"));
    }

    #[test]
    fn exact_width_prompt_does_not_become_part_of_wrapped_input() {
        let mut terminal = tracked_terminal(3, 4);
        let mut parser = TestProcessor::new();
        let mut output = b"abc".to_vec();
        output.extend_from_slice(&prompt_marker());
        output.extend_from_slice(b"git");

        terminal.process_pty_output(&mut parser, &output);

        assert_eq!(terminal.current_bash_input().as_deref(), Some("git"));
    }

    #[test]
    fn prompt_marker_split_across_chunks_still_anchors_before_suffix() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();
        let marker = prompt_marker();
        let split = marker.len() - 2;

        assert!(terminal
            .process_pty_output(&mut parser, &marker[..split])
            .is_empty());
        assert_eq!(terminal.current_bash_input(), None);

        let mut tail = marker[split..].to_vec();
        tail.extend_from_slice(b"git");
        assert!(terminal.process_pty_output(&mut parser, &tail).is_empty());
        assert_eq!(terminal.current_bash_input().as_deref(), Some("git"));
    }

    #[test]
    fn segmented_marker_processing_matches_unmodified_parser_input() {
        let mut tracked = tracked_terminal(80, 24);
        let mut baseline = TerminalState::new();
        baseline.init_term(80, 24);
        let mut tracked_parser = TestProcessor::new();
        let mut baseline_parser = TestProcessor::new();
        let mut output = b"before".to_vec();
        output.extend_from_slice(&prompt_marker());
        output.extend_from_slice(b"git");
        output.extend_from_slice(&history_marker("L3RtcC9oaXN0b3J5"));
        output.extend_from_slice(&prompt_marker());
        output.extend_from_slice("你好abc".as_bytes());
        output.extend_from_slice(b"\x1b[3");

        tracked.process_pty_output(&mut tracked_parser, &output);
        baseline_parser.advance(baseline.term.as_mut().unwrap(), &output);

        let tracked_term = tracked.term().unwrap();
        let baseline_term = baseline.term().unwrap();
        assert_eq!(tracked_term.grid(), baseline_term.grid());
        assert_eq!(
            tracked_term.grid().cursor.point,
            baseline_term.grid().cursor.point
        );
        assert_eq!(
            tracked_term.grid().cursor.input_needs_wrap,
            baseline_term.grid().cursor.input_needs_wrap
        );
        assert_eq!(tracked_term.mode(), baseline_term.mode());

        tracked.process_pty_output(&mut tracked_parser, b"1mX");
        baseline_parser.advance(baseline.term.as_mut().unwrap(), b"1mX");
        let tracked_term = tracked.term().unwrap();
        let baseline_term = baseline.term().unwrap();
        assert_eq!(tracked_term.grid(), baseline_term.grid());
        assert_eq!(
            tracked_term.grid().cursor.point,
            baseline_term.grid().cursor.point
        );
    }

    #[test]
    fn soft_wrap_with_wide_spacers_extracts_cjk_and_ascii() {
        let mut terminal = tracked_terminal(5, 4);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice("你好abc".as_bytes());

        terminal.process_pty_output(&mut parser, &output);

        assert_eq!(terminal.current_bash_input().as_deref(), Some("你好abc"));
    }

    #[test]
    fn wide_cell_and_zerowidth_combining_character_are_preserved() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice("好e\u{301}".as_bytes());

        terminal.process_pty_output(&mut parser, &output);

        assert_eq!(terminal.current_bash_input().as_deref(), Some("好e\u{301}"));
    }

    #[test]
    fn hard_newline_invalidates_prompt_anchor() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice(b"git\r\n");

        terminal.process_pty_output(&mut parser, &output);

        assert_eq!(terminal.current_bash_input(), None);
        assert!(terminal.prompt_tracking.as_ref().unwrap().anchor.is_none());
    }

    #[test]
    fn alternate_screen_invalidates_prompt_anchor_permanently() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();

        terminal.process_pty_output(&mut parser, &prompt_marker());
        assert_eq!(terminal.current_bash_input().as_deref(), Some(""));

        terminal.process_pty_output(&mut parser, b"\x1b[?1049h");
        assert!(terminal.prompt_tracking.as_ref().unwrap().anchor.is_none());
        terminal.process_pty_output(&mut parser, b"\x1b[?1049l");
        assert_eq!(terminal.current_bash_input(), None);
    }

    #[test]
    fn only_real_resize_invalidates_prompt_anchor() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();

        terminal.process_pty_output(&mut parser, &prompt_marker());
        terminal.resize(80, 24);
        assert_eq!(terminal.current_bash_input().as_deref(), Some(""));

        terminal.resize(81, 24);
        assert_eq!(terminal.current_bash_input(), None);
        assert!(terminal.prompt_tracking.as_ref().unwrap().anchor.is_none());
    }

    #[test]
    fn recoverable_readline_edit_drops_stale_grid_and_queues_snapshot_after_edit() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, expected_snapshot) = install_local_snapshot_runtime(&mut terminal);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice(b"git");
        terminal.process_pty_output(&mut parser, &output);
        let mut completion = crate::smart_completion::CompletionState::new(completion_session());
        completion.track_user_input("git");
        completion.track_user_input("\x1b[D");

        terminal.invalidate_readline_geometry();
        terminal.write_input("\x1b[D");

        assert!(terminal.has_authenticated_active_bash_prompt());
        assert_eq!(
            crate::completion_input_for_render(&mut completion, &mut terminal, Instant::now(),),
            None
        );
        assert_eq!(write_rx.try_recv().unwrap(), b"\x1b[D");
        assert_eq!(write_rx.try_recv().unwrap(), expected_snapshot);
        assert!(matches!(
            write_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn recoverable_edit_rejects_queued_pre_edit_snapshot_before_accepting_post_edit_response() {
        let mut terminal = tracked_terminal(80, 24);
        let (write_rx, expected_snapshot) = install_local_snapshot_runtime(&mut terminal);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice(b"git");
        terminal.process_pty_output(&mut parser, &output);
        let mut completion = crate::smart_completion::CompletionState::new(completion_session());
        completion.replace_history(vec!["git status".into()]);
        completion.track_user_input("git");

        terminal.resize(81, 24);
        assert_eq!(write_rx.try_recv().unwrap(), expected_snapshot);
        let mut popup_snapshot = None;
        let effect = crate::apply_completion_user_input_state(
            &mut completion,
            &mut popup_snapshot,
            "\x1b[D",
        );
        crate::apply_completion_prompt_effect(&mut terminal, effect);
        terminal.write_input("\x1b[D");
        assert_eq!(
            crate::completion_input_for_render(&mut completion, &mut terminal, Instant::now()),
            None
        );
        assert_eq!(write_rx.try_recv().unwrap(), b"\x1b[D");
        assert_eq!(write_rx.try_recv().unwrap(), expected_snapshot);

        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "git status", 3),
        );
        let stale_input =
            crate::completion_input_for_render(&mut completion, &mut terminal, Instant::now());
        if let Some(input) = stale_input.as_deref() {
            completion.refresh(input);
        }
        assert_eq!(stale_input, None);
        assert!(completion.candidates().is_empty());
        assert!(terminal
            .prompt_tracking
            .as_ref()
            .unwrap()
            .snapshot_requested_at
            .is_some());

        terminal.process_pty_output(
            &mut parser,
            &input_snapshot_marker(&completion_session(), "gi", 2),
        );
        assert_eq!(terminal.current_bash_input().as_deref(), Some("gi"));
    }
