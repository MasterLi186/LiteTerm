    #[test]
    fn enhanced_read_loop_shutdown_is_bounded_while_reader_is_blocked() {
        let directory = tempfile::tempdir().unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut terminal = TerminalState::new();
        terminal.init_term(80, 24);
        terminal.pty_reader = Some(Box::new(CommandBlockedReader {
            entered: entered_tx,
            release: release_rx,
        }));
        let terminal = Arc::new(Mutex::new(terminal));
        let (commands, receiver) = crate::zmodem::runtime::runtime_command_channel();
        let (done_tx, done_rx) = mpsc::channel();
        let read_terminal = Arc::clone(&terminal);
        let receive_path = directory.path().to_path_buf();
        let worker = std::thread::spawn(move || {
            read_loop_with_zmodem(
                read_terminal,
                || {},
                |_| {},
                |_| {},
                crate::zmodem::runtime::RuntimeConfig::new(
                    crate::zmodem::runtime::RuntimeCapability::Local,
                    receive_path,
                    crate::zmodem::runtime::TransferIdentity {
                        transfer_id: 55,
                        generation: 3,
                    },
                    receiver,
                ),
                |_| {},
            );
            let _ = done_tx.send(());
        });

        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        commands
            .send(crate::zmodem::runtime::RuntimeCommand::Shutdown)
            .unwrap();
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("控制循环必须在 reader 阻塞时有界退出");
        release_tx.send(()).unwrap();
        worker.join().unwrap();
    }
