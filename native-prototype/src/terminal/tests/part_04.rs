    #[test]
    fn timed_out_started_protocol_write_keeps_normal_gate_closed_until_return() {
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
        gate.activate();
        let protocol_thread = std::thread::spawn(move || {
            protocol.write_and_flush(b"late-protocol", Duration::from_millis(20))
        });
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            protocol_thread.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );

        gate.deactivate();
        assert!(
            gate.is_active(),
            "in-flight syscall must keep the gate closed"
        );
        assert_eq!(
            writer
                .try_send_normal(b"normal".to_vec())
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );

        release_tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while gate.is_active() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(!gate.is_active());
        writer.try_send_normal(b"normal".to_vec()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while captured.lock().unwrap().len() < b"late-protocolnormal".len()
            && Instant::now() < deadline
        {
            std::thread::yield_now();
        }
        assert_eq!(&*captured.lock().unwrap(), b"late-protocolnormal");
    }

    #[test]
    fn enhanced_read_loop_replays_false_bytes_but_intercepts_real_trigger() {
        let directory = tempfile::tempdir().unwrap();
        let mut terminal = TerminalState::new();
        terminal.init_term(120, 8);
        let mut wire = b"visible false **\x18Bnot-a-header ".to_vec();
        wire.extend(crate::zmodem::encode::encode_zhex_header(
            crate::zmodem::FrameType::Zrqinit,
            [0; 4],
        ));
        terminal.pty_reader = Some(Box::new(std::io::Cursor::new(wire)));
        let gate = Arc::clone(&terminal.zmodem_input_gate);
        let (writer, protocol_writer) =
            spawn_writer_worker_with_protocol(Box::new(std::io::sink()), gate);
        terminal.writer = Some(writer);
        terminal.zmodem_protocol_writer = Some(protocol_writer);
        let terminal = Arc::new(Mutex::new(terminal));
        let (_commands, receiver) = crate::zmodem::runtime::runtime_command_channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&events);
        let visible = Arc::new(Mutex::new(Vec::new()));
        let captured_visible = Arc::clone(&visible);

        read_loop_with_zmodem(
            Arc::clone(&terminal),
            || {},
            |_| {},
            move |bytes| captured_visible.lock().unwrap().extend_from_slice(bytes),
            crate::zmodem::runtime::RuntimeConfig::new(
                crate::zmodem::runtime::RuntimeCapability::Local,
                directory.path(),
                crate::zmodem::runtime::TransferIdentity {
                    transfer_id: 44,
                    generation: 2,
                },
                receiver,
            ),
            move |event| captured_events.lock().unwrap().push(event),
        );

        let text = terminal
            .lock()
            .unwrap()
            .search_lines()
            .iter()
            .map(search_line_haystack)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("visible false"));
        assert!(text.contains("not-a-header"));
        assert!(!text.contains("00000000000000"));
        assert_eq!(
            &*visible.lock().unwrap(),
            b"visible false **\x18Bnot-a-header "
        );
        assert!(events.lock().unwrap().iter().any(|event| matches!(
            event.kind,
            crate::zmodem::runtime::RuntimeEventKind::Started {
                direction: crate::zmodem::runtime::TransferDirection::Receive,
                ..
            }
        )));
    }

    #[test]
    fn visible_output_excludes_fragmented_receive_wire_and_resumes_after_transfer() {
        let directory = tempfile::tempdir().unwrap();
        let mut wire =
            crate::zmodem::encode::encode_zbin32_header(crate::zmodem::FrameType::Zrqinit, [0; 4]);
        wire.extend(crate::zmodem::encode::encode_zbin32_header(
            crate::zmodem::FrameType::Zfile,
            [0; 4],
        ));
        wire.extend(
            crate::zmodem::encode::encode_data_subpacket(
                &crate::zmodem::encode::encode_zfile_metadata("visible-hook-received", 3, 0, 1),
                crate::zmodem::ZCRCW,
                true,
            )
            .unwrap(),
        );
        wire.extend(crate::zmodem::encode::encode_zbin32_header(
            crate::zmodem::FrameType::Zdata,
            [0; 4],
        ));
        wire.extend(
            crate::zmodem::encode::encode_data_subpacket(b"abc", crate::zmodem::ZCRCE, true)
                .unwrap(),
        );
        wire.extend(crate::zmodem::encode::encode_zbin32_header(
            crate::zmodem::FrameType::Zeof,
            3u32.to_le_bytes(),
        ));
        wire.extend(crate::zmodem::encode::encode_zbin32_header(
            crate::zmodem::FrameType::Zfin,
            [0; 4],
        ));
        wire.extend_from_slice(b"OO");
        let prompt = b"NORMAL-PROMPT-AFTER-TRANSFER";
        wire.extend_from_slice(prompt);

        let mut terminal = TerminalState::new();
        terminal.init_term(80, 8);
        terminal.pty_reader = Some(Box::new(ChunkReader {
            chunks: wire.into_iter().map(|byte| vec![byte]).collect(),
        }));
        let gate = Arc::clone(&terminal.zmodem_input_gate);
        let (writer, protocol_writer) =
            spawn_writer_worker_with_protocol(Box::new(std::io::sink()), gate);
        terminal.writer = Some(writer);
        terminal.zmodem_protocol_writer = Some(protocol_writer);
        let terminal = Arc::new(Mutex::new(terminal));
        let (_commands, receiver) = crate::zmodem::runtime::runtime_command_channel();
        let visible = Arc::new(Mutex::new(Vec::new()));
        let captured_visible = Arc::clone(&visible);

        read_loop_with_zmodem(
            Arc::clone(&terminal),
            || {},
            |_| {},
            move |bytes| captured_visible.lock().unwrap().extend_from_slice(bytes),
            crate::zmodem::runtime::RuntimeConfig::new(
                crate::zmodem::runtime::RuntimeCapability::Local,
                directory.path(),
                crate::zmodem::runtime::TransferIdentity {
                    transfer_id: 45,
                    generation: 2,
                },
                receiver,
            ),
            |_| {},
        );

        assert_eq!(&*visible.lock().unwrap(), prompt);
        assert_eq!(
            std::fs::read(directory.path().join("visible-hook-received")).unwrap(),
            b"abc"
        );
    }

    struct CommandBlockedReader {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }

    struct ChunkReader {
        chunks: std::collections::VecDeque<Vec<u8>>,
    }

    struct CachedThenBlockedPromptReader {
        chunks: std::collections::VecDeque<Vec<u8>>,
        release_prompt: mpsc::Receiver<()>,
    }

    impl Read for ChunkReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let Some(chunk) = self.chunks.pop_front() else {
                return Ok(0);
            };
            buffer[..chunk.len()].copy_from_slice(&chunk);
            Ok(chunk.len())
        }
    }

    impl Read for CachedThenBlockedPromptReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let Some(chunk) = self.chunks.pop_front() else {
                return Ok(0);
            };
            if self.chunks.is_empty() {
                let _ = self.release_prompt.recv();
            }
            buffer[..chunk.len()].copy_from_slice(&chunk);
            Ok(chunk.len())
        }
    }

    #[test]
    fn cancelled_transfer_discards_already_cached_reader_epoch() {
        let directory = tempfile::tempdir().unwrap();
        let trigger =
            crate::zmodem::encode::encode_zhex_header(crate::zmodem::FrameType::Zrqinit, [0; 4]);
        let mut terminal = TerminalState::new();
        terminal.init_term(80, 8);
        terminal.pty_reader = Some(Box::new(ChunkReader {
            chunks: std::collections::VecDeque::from([
                trigger,
                b"CACHED-PROTOCOL-MUST-NOT-PARSE".to_vec(),
            ]),
        }));
        let gate = Arc::clone(&terminal.zmodem_input_gate);
        let (writer, protocol_writer) =
            spawn_writer_worker_with_protocol(Box::new(std::io::sink()), gate);
        terminal.writer = Some(writer);
        terminal.zmodem_protocol_writer = Some(protocol_writer);
        let terminal = Arc::new(Mutex::new(terminal));
        let (commands, receiver) = crate::zmodem::runtime::runtime_command_channel();
        let cancel_commands = commands.clone();

        read_loop_with_zmodem(
            Arc::clone(&terminal),
            || {},
            |_| {},
            |_| {},
            crate::zmodem::runtime::RuntimeConfig::new(
                crate::zmodem::runtime::RuntimeCapability::Local,
                directory.path(),
                crate::zmodem::runtime::TransferIdentity {
                    transfer_id: 56,
                    generation: 3,
                },
                receiver,
            ),
            move |event| {
                if matches!(
                    event.kind,
                    crate::zmodem::runtime::RuntimeEventKind::Started { .. }
                ) {
                    let _ = cancel_commands.try_send(
                        crate::zmodem::runtime::RuntimeCommand::Cancel(event.identity),
                    );
                }
            },
        );

        let text = terminal
            .lock()
            .unwrap()
            .search_lines()
            .iter()
            .map(search_line_haystack)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains("CACHED-PROTOCOL-MUST-NOT-PARSE"));
    }

    #[test]
    fn cancel_drops_cached_protocol_but_preserves_later_blocked_read_prompt() {
        let directory = tempfile::tempdir().unwrap();
        let trigger =
            crate::zmodem::encode::encode_zhex_header(crate::zmodem::FrameType::Zrqinit, [0; 4]);
        let (release_tx, release_rx) = mpsc::channel();
        let mut terminal = TerminalState::new();
        terminal.init_term(80, 8);
        terminal.pty_reader = Some(Box::new(CachedThenBlockedPromptReader {
            chunks: std::collections::VecDeque::from([
                trigger,
                b"CACHED-PROTOCOL-MUST-NOT-PARSE".to_vec(),
                b"NORMAL-PROMPT-AFTER-CANCEL".to_vec(),
            ]),
            release_prompt: release_rx,
        }));
        let gate = Arc::clone(&terminal.zmodem_input_gate);
        let (writer, protocol_writer) =
            spawn_writer_worker_with_protocol(Box::new(std::io::sink()), gate);
        terminal.writer = Some(writer);
        terminal.zmodem_protocol_writer = Some(protocol_writer);
        let terminal = Arc::new(Mutex::new(terminal));
        let (commands, receiver) = crate::zmodem::runtime::runtime_command_channel();
        let cancel_commands = commands.clone();

        read_loop_with_zmodem(
            Arc::clone(&terminal),
            || {},
            |_| {},
            |_| {},
            crate::zmodem::runtime::RuntimeConfig::new(
                crate::zmodem::runtime::RuntimeCapability::Local,
                directory.path(),
                crate::zmodem::runtime::TransferIdentity {
                    transfer_id: 57,
                    generation: 3,
                },
                receiver,
            ),
            move |event| match event.kind {
                crate::zmodem::runtime::RuntimeEventKind::Started { .. } => {
                    let _ = cancel_commands.try_send(
                        crate::zmodem::runtime::RuntimeCommand::Cancel(event.identity),
                    );
                }
                crate::zmodem::runtime::RuntimeEventKind::Finished => {
                    let _ = release_tx.send(());
                }
                _ => {}
            },
        );

        let text = terminal
            .lock()
            .unwrap()
            .search_lines()
            .iter()
            .map(search_line_haystack)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains("CACHED-PROTOCOL-MUST-NOT-PARSE"));
        assert!(text.contains("NORMAL-PROMPT-AFTER-CANCEL"));
    }

    impl Read for CommandBlockedReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            let _ = self.entered.send(());
            let _ = self.release.recv();
            Ok(0)
        }
    }
