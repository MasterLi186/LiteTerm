use super::*;
use crate::zmodem::encode::encode_zhex_header;
use crate::zmodem::FrameType;
use std::io::Write;
use std::sync::Mutex;

struct PartialWriter {
    output: Arc<Mutex<Vec<u8>>>,
    fail: bool,
}

impl Write for PartialWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.fail {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "failed"));
        }
        let take = bytes.len().min(2);
        self.output
            .lock()
            .unwrap()
            .extend_from_slice(&bytes[..take]);
        Ok(take)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "failed"))
        } else {
            Ok(())
        }
    }
}

fn writer_pair(writer: PartialWriter) -> (TransportWriter, ProtocolWriter, TerminalReplyWriter) {
    let gate = Arc::new(ProtocolGate::new());
    let (transport, receiver) = transport_write_channel(gate);
    std::thread::spawn(move || {
        let mut writer = writer;
        while let Ok(message) = receiver.recv() {
            match message {
                TransportWrite::Normal { bytes, .. } => {
                    let _ = writer.write_all(&bytes).and_then(|_| writer.flush());
                }
                TransportWrite::Protocol(request) => {
                    if !request.begin() {
                        request.complete(Err(io::Error::new(io::ErrorKind::TimedOut, "expired")));
                        continue;
                    }
                    let result = writer
                        .write_all(request.bytes())
                        .and_then(|_| writer.flush());
                    request.complete(result);
                }
                TransportWrite::TerminalReply(request) => {
                    if !request.begin() {
                        request.complete(Err(io::Error::new(io::ErrorKind::TimedOut, "expired")));
                        continue;
                    }
                    let result = writer
                        .write_all(request.bytes())
                        .and_then(|_| writer.flush());
                    request.complete(result);
                }
            }
        }
    });
    let protocol = ProtocolWriter::from_transport_writer(transport.clone());
    let terminal_reply = TerminalReplyWriter::from_transport_writer(transport.clone());
    (transport, protocol, terminal_reply)
}

fn writer_worker(writer: PartialWriter) -> ProtocolWriter {
    writer_pair(writer).1
}

#[test]
fn terminal_reply_acks_only_after_partial_write_and_flush() {
    let output = Arc::new(Mutex::new(Vec::new()));
    let (_, _, writer) = writer_pair(PartialWriter {
        output: Arc::clone(&output),
        fail: false,
    });

    writer
        .write_and_flush(b"\x1b[1;212R", Duration::from_secs(1))
        .unwrap();

    assert_eq!(*output.lock().unwrap(), b"\x1b[1;212R");
}

#[test]
fn terminal_reply_does_not_activate_or_obey_zmodem_exclusivity_gate() {
    let output = Arc::new(Mutex::new(Vec::new()));
    let (transport, _, writer) = writer_pair(PartialWriter {
        output: Arc::clone(&output),
        fail: false,
    });
    let gate = transport.protocol_active_gate();
    gate.activate();

    writer
        .write_and_flush(b"\x1b[?6c", Duration::from_secs(1))
        .unwrap();

    assert!(gate.is_active());
    assert_eq!(*output.lock().unwrap(), b"\x1b[?6c");
    gate.deactivate();
}

#[test]
fn queued_terminal_reply_that_times_out_never_starts() {
    let gate = Arc::new(ProtocolGate::new());
    let (transport, receiver) = transport_write_channel(gate);
    let writer = TerminalReplyWriter::from_transport_writer(transport);

    let error = writer
        .write_and_flush(b"must-not-write", Duration::from_millis(5))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);

    let request = receiver.try_recv().unwrap();
    let TransportWrite::TerminalReply(request) = request else {
        panic!("expected typed terminal reply request");
    };
    assert!(
        !request.begin(),
        "timed-out queued terminal reply must stay cancelled"
    );
}

#[test]
fn terminal_reply_partial_progress_only_gets_a_finite_hard_deadline_grace() {
    let now = Instant::now();
    let request = TerminalReplyRequest::with_deadlines_for_test(
        b"after-progress",
        now - Duration::from_secs(2),
        now - Duration::from_secs(1),
    );

    request.mark_progress();

    assert!(
        !request.may_continue_at(now),
        "partial progress must not keep a terminal reply alive forever"
    );
    assert!(request.hard_deadline_expired_at(now));
}

#[test]
fn bounded_writer_acks_only_after_partial_write_and_flush() {
    let output = Arc::new(Mutex::new(Vec::new()));
    let writer = writer_worker(PartialWriter {
        output: Arc::clone(&output),
        fail: false,
    });
    writer
        .write_and_flush(b"abcdef", Duration::from_secs(1))
        .unwrap();
    assert_eq!(*output.lock().unwrap(), b"abcdef");
}

#[test]
fn bounded_writer_returns_actual_transport_failure() {
    let writer = writer_worker(PartialWriter {
        output: Arc::new(Mutex::new(Vec::new())),
        fail: true,
    });
    let error = writer
        .write_and_flush(b"failure", Duration::from_secs(1))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);
}

#[test]
fn typed_protocol_acks_do_not_cross_between_terminals() {
    let first_output = Arc::new(Mutex::new(Vec::new()));
    let second_output = Arc::new(Mutex::new(Vec::new()));
    let first = writer_worker(PartialWriter {
        output: Arc::clone(&first_output),
        fail: false,
    });
    let second = writer_worker(PartialWriter {
        output: Arc::clone(&second_output),
        fail: false,
    });

    let first_thread =
        std::thread::spawn(move || first.write_and_flush(b"first", Duration::from_secs(1)));
    let second_thread =
        std::thread::spawn(move || second.write_and_flush(b"second", Duration::from_secs(1)));
    first_thread.join().unwrap().unwrap();
    second_thread.join().unwrap().unwrap();

    assert_eq!(*first_output.lock().unwrap(), b"first");
    assert_eq!(*second_output.lock().unwrap(), b"second");
}

#[test]
fn queued_protocol_request_that_times_out_never_starts() {
    let gate = Arc::new(ProtocolGate::new());
    let (transport, receiver) = transport_write_channel(gate);
    let protocol = ProtocolWriter::from_transport_writer(transport);

    let error = protocol
        .write_and_flush(b"must-not-write", Duration::from_millis(5))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);

    let request = receiver.try_recv().unwrap();
    let TransportWrite::Protocol(request) = request else {
        panic!("expected typed protocol request");
    };
    assert!(
        !request.begin(),
        "timed-out queued request must stay cancelled"
    );
}

#[test]
fn normal_transport_ingress_is_bounded_and_reports_full() {
    let gate = Arc::new(ProtocolGate::new());
    let (transport, _receiver) = transport_write_channel(gate);
    for index in 0..TRANSPORT_WRITE_QUEUE_CAPACITY {
        transport.try_send_normal(vec![index as u8]).unwrap();
    }
    let error = transport.try_send_normal(vec![0xff]).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
}

#[test]
fn terminal_reply_ingress_is_bounded_and_reports_full_without_waiting() {
    let gate = Arc::new(ProtocolGate::new());
    let (transport, _receiver) = transport_write_channel(gate);
    let writer = TerminalReplyWriter::from_transport_writer(transport);
    for _ in 0..TRANSPORT_WRITE_QUEUE_CAPACITY {
        writer
            .try_enqueue(b"reply", Duration::from_secs(1))
            .unwrap();
    }

    let error = writer
        .try_enqueue(b"overflow", Duration::from_secs(1))
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
}

#[test]
fn capability_explicitly_disables_serial() {
    assert!(RuntimeCapability::Local.supports_zmodem());
    assert!(RuntimeCapability::DirectSsh.supports_zmodem());
    assert!(!RuntimeCapability::SerialDisabled.supports_zmodem());
}

#[test]
fn false_positive_is_replayed_and_trigger_is_intercepted() {
    let directory = tempfile::tempdir().unwrap();
    let output_bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = writer_worker(PartialWriter {
        output: Arc::clone(&output_bytes),
        fail: false,
    });
    let (_commands, receiver) = runtime_command_channel();
    let mut runtime = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::Local,
            directory.path(),
            TransferIdentity {
                transfer_id: 3,
                generation: 7,
            },
            receiver,
        ),
        Some(writer),
        Arc::new(ProtocolGate::new()),
    );
    let false_positive = b"text **\x18B00000000000001zz";
    let replay = runtime.feed(false_positive);
    assert_eq!(replay.replay, false_positive);

    let trigger = encode_zhex_header(FrameType::Zrqinit, [0; 4]);
    let detected = runtime.feed(&trigger);
    assert!(detected.replay.is_empty());
    assert!(detected.events.iter().any(|event| matches!(
        event.kind,
        RuntimeEventKind::Started {
            direction: TransferDirection::Receive,
            ..
        }
    )));
    assert!(!output_bytes.lock().unwrap().is_empty());
}

#[test]
fn consumed_trigger_discards_reader_boundary_on_all_initialization_failures() {
    let directory = tempfile::tempdir().unwrap();
    let trigger = encode_zhex_header(FrameType::Zrqinit, [0; 4]);

    let (_commands, receiver) = runtime_command_channel();
    let mut missing_writer = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::Local,
            directory.path(),
            TransferIdentity {
                transfer_id: 1,
                generation: 1,
            },
            receiver,
        ),
        None,
        Arc::new(ProtocolGate::new()),
    );
    assert!(missing_writer.feed(&trigger).discard_reader_epoch);

    let writer = writer_worker(PartialWriter {
        output: Arc::new(Mutex::new(Vec::new())),
        fail: false,
    });
    let (_commands, receiver) = runtime_command_channel();
    let mut invalid_directory = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::Local,
            directory.path().join("missing"),
            TransferIdentity {
                transfer_id: 2,
                generation: 1,
            },
            receiver,
        ),
        Some(writer),
        Arc::new(ProtocolGate::new()),
    );
    assert!(invalid_directory.feed(&trigger).discard_reader_epoch);

    let failing_writer = writer_worker(PartialWriter {
        output: Arc::new(Mutex::new(Vec::new())),
        fail: true,
    });
    let (_commands, receiver) = runtime_command_channel();
    let mut failed_start = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::Local,
            directory.path(),
            TransferIdentity {
                transfer_id: 3,
                generation: 1,
            },
            receiver,
        ),
        Some(failing_writer),
        Arc::new(ProtocolGate::new()),
    );
    assert!(failed_start.feed(&trigger).discard_reader_epoch);
}

#[test]
fn stale_generation_does_not_cancel_active_transfer() {
    let directory = tempfile::tempdir().unwrap();
    let writer = writer_worker(PartialWriter {
        output: Arc::new(Mutex::new(Vec::new())),
        fail: false,
    });
    let (commands, receiver) = runtime_command_channel();
    let identity = TransferIdentity {
        transfer_id: 5,
        generation: 9,
    };
    let mut runtime = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::DirectSsh,
            directory.path(),
            identity,
            receiver,
        ),
        Some(writer),
        Arc::new(ProtocolGate::new()),
    );
    runtime.feed(&encode_zhex_header(FrameType::Zrqinit, [0; 4]));
    commands
        .send(RuntimeCommand::Cancel(TransferIdentity {
            transfer_id: 5,
            generation: 8,
        }))
        .unwrap();
    let output = runtime.poll();
    assert!(runtime.active());
    assert!(output
        .events
        .iter()
        .any(|event| event.kind == RuntimeEventKind::StaleCommand));
}

#[test]
fn cancel_and_timeout_finish_active_runtime_and_open_input_gate() {
    for timeout in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let writer = writer_worker(PartialWriter {
            output: Arc::new(Mutex::new(Vec::new())),
            fail: false,
        });
        let (commands, receiver) = runtime_command_channel();
        let identity = TransferIdentity {
            transfer_id: 8,
            generation: 12,
        };
        let gate = Arc::new(ProtocolGate::new());
        let mut runtime = ZmodemRuntime::new(
            RuntimeConfig::new(
                RuntimeCapability::Local,
                directory.path(),
                identity,
                receiver,
            ),
            Some(writer),
            Arc::clone(&gate),
        );
        runtime.feed(&encode_zhex_header(FrameType::Zrqinit, [0; 4]));
        assert!(gate.is_active());
        commands
            .send(if timeout {
                RuntimeCommand::Timeout(identity)
            } else {
                RuntimeCommand::Cancel(identity)
            })
            .unwrap();

        let output = runtime.poll();
        assert!(!runtime.active());
        assert!(!gate.is_active());
        assert!(output
            .events
            .iter()
            .any(|event| event.kind == RuntimeEventKind::Finished));
    }
}

#[test]
fn command_channel_is_bounded_and_reports_disconnect() {
    let (sender, receiver) = runtime_command_channel();
    for _ in 0..RUNTIME_COMMAND_CAPACITY {
        sender.try_send(RuntimeCommand::Shutdown).unwrap();
    }
    assert!(matches!(
        sender.try_send(RuntimeCommand::Shutdown),
        Err(mpsc::TrySendError::Full(_))
    ));
    drop(receiver);
    assert!(matches!(
        sender.try_send(RuntimeCommand::Shutdown),
        Err(mpsc::TrySendError::Disconnected(_))
    ));
}

#[test]
fn poll_processes_at_most_the_bounded_command_capacity() {
    let directory = tempfile::tempdir().unwrap();
    let writer = writer_worker(PartialWriter {
        output: Arc::new(Mutex::new(Vec::new())),
        fail: false,
    });
    let (commands, receiver) = runtime_command_channel();
    let mut runtime = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::Local,
            directory.path(),
            TransferIdentity {
                transfer_id: 20,
                generation: 2,
            },
            receiver,
        ),
        Some(writer),
        Arc::new(ProtocolGate::new()),
    );
    for transfer_id in 0..RUNTIME_COMMAND_CAPACITY as u64 {
        commands
            .try_send(RuntimeCommand::Cancel(TransferIdentity {
                transfer_id,
                generation: 2,
            }))
            .unwrap();
    }
    let output = runtime.poll();
    assert_eq!(output.events.len(), RUNTIME_COMMAND_CAPACITY);
}

#[test]
fn auto_detect_off_replays_trigger_but_explicit_send_still_works() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("send.bin");
    std::fs::write(&path, b"abc").unwrap();
    let writer = writer_worker(PartialWriter {
        output: Arc::new(Mutex::new(Vec::new())),
        fail: false,
    });
    let (commands, receiver) = runtime_command_channel();
    let identity = TransferIdentity {
        transfer_id: 30,
        generation: 4,
    };
    let mut config = RuntimeConfig::new(
        RuntimeCapability::Local,
        directory.path(),
        identity,
        receiver,
    );
    config.auto_detect = false;
    let mut runtime = ZmodemRuntime::new(config, Some(writer), Arc::new(ProtocolGate::new()));
    let trigger = encode_zhex_header(FrameType::Zrqinit, [0; 4]);
    assert_eq!(runtime.feed(&trigger).replay, trigger);
    assert!(!runtime.active());

    commands
        .try_send(RuntimeCommand::StartSend {
            identity,
            paths: vec![path],
        })
        .unwrap();
    let output = runtime.poll();
    assert!(runtime.active());
    assert!(output.events.iter().any(|event| matches!(
        event.kind,
        RuntimeEventKind::Started {
            direction: TransferDirection::Send,
            ..
        }
    )));
}

#[test]
fn explicit_send_handshake_pumps_progress_and_rejects_concurrent_start() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("send.bin");
    std::fs::write(&path, b"abc").unwrap();
    let writer = writer_worker(PartialWriter {
        output: Arc::new(Mutex::new(Vec::new())),
        fail: false,
    });
    let (commands, receiver) = runtime_command_channel();
    let identity = TransferIdentity {
        transfer_id: 40,
        generation: 6,
    };
    let mut runtime = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::DirectSsh,
            directory.path(),
            identity,
            receiver,
        ),
        Some(writer),
        Arc::new(ProtocolGate::new()),
    );
    commands
        .try_send(RuntimeCommand::StartSend {
            identity,
            paths: vec![path.clone()],
        })
        .unwrap();
    let started = runtime.poll();
    assert!(started.events.iter().any(|event| matches!(
        &event.kind,
        RuntimeEventKind::Started {
            direction: TransferDirection::Send,
            filename: Some(name),
            total: Some(3),
        } if name == "send.bin"
    )));

    commands
        .try_send(RuntimeCommand::StartSend {
            identity: TransferIdentity {
                transfer_id: 41,
                generation: 6,
            },
            paths: vec![path],
        })
        .unwrap();
    assert!(runtime
        .poll()
        .events
        .iter()
        .any(|event| matches!(event.kind, RuntimeEventKind::Error(_))));

    runtime.feed(&encode_zhex_header(
        FrameType::Zrinit,
        [0, 0, 0, crate::zmodem::CANFC32],
    ));
    let progress = runtime.feed(&encode_zhex_header(FrameType::Zrpos, 0u32.to_le_bytes()));
    assert!(progress.events.iter().any(|event| matches!(
        event.kind,
        RuntimeEventKind::Sender(SenderAction::Progress {
            bytes_sent: 3,
            total: 3,
            ..
        })
    )));
}

#[test]
fn sender_runtime_pumps_one_chunk_per_control_turn() {
    assert_eq!(MAX_RUNTIME_PUMP_CHUNKS, 1);
    assert_eq!(ACTIVE_READER_POLL_INTERVAL, Duration::from_millis(1));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large-send.bin");
    const FILE_SIZE: usize = 16 * 1024;
    std::fs::write(&path, vec![0x5a; FILE_SIZE]).unwrap();
    let writer = writer_worker(PartialWriter {
        output: Arc::new(Mutex::new(Vec::new())),
        fail: false,
    });
    let (commands, receiver) = runtime_command_channel();
    let identity = TransferIdentity {
        transfer_id: 42,
        generation: 6,
    };
    let mut runtime = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::Local,
            directory.path(),
            identity,
            receiver,
        ),
        Some(writer),
        Arc::new(ProtocolGate::new()),
    );
    commands
        .try_send(RuntimeCommand::StartSend {
            identity,
            paths: vec![path],
        })
        .unwrap();
    runtime.poll();
    runtime.feed(&encode_zhex_header(
        FrameType::Zrinit,
        [0, 0, 0, crate::zmodem::CANFC32],
    ));
    let first_turn = runtime.feed(&encode_zhex_header(FrameType::Zrpos, 0u32.to_le_bytes()));
    let sent = first_turn
        .events
        .iter()
        .find_map(|event| match event.kind {
            RuntimeEventKind::Sender(SenderAction::Progress { bytes_sent, .. }) => Some(bytes_sent),
            _ => None,
        })
        .expect("first sender turn should make progress");
    assert!(sent > 0);
    assert!(
        sent < FILE_SIZE as u64,
        "a control turn must not drain the whole file"
    );
}

#[test]
fn repeated_auto_receives_allocate_distinct_transfer_ids() {
    let directory = tempfile::tempdir().unwrap();
    let writer = writer_worker(PartialWriter {
        output: Arc::new(Mutex::new(Vec::new())),
        fail: false,
    });
    let (commands, receiver) = runtime_command_channel();
    let mut runtime = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::Local,
            directory.path(),
            TransferIdentity {
                transfer_id: 90,
                generation: 7,
            },
            receiver,
        ),
        Some(writer),
        Arc::new(ProtocolGate::new()),
    );
    let trigger = encode_zhex_header(FrameType::Zrqinit, [0; 4]);
    let first = runtime.feed(&trigger).events[0].identity;
    commands.try_send(RuntimeCommand::Cancel(first)).unwrap();
    runtime.poll();
    let second = runtime.feed(&trigger).events[0].identity;
    assert_ne!(first.transfer_id, second.transfer_id);
    assert_eq!(first.generation, second.generation);
}

#[test]
fn repeated_and_regressing_send_ids_are_rejected_and_max_never_wraps() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("send.bin");
    std::fs::write(&path, b"abc").unwrap();
    let writer = writer_worker(PartialWriter {
        output: Arc::new(Mutex::new(Vec::new())),
        fail: false,
    });
    let (commands, receiver) = runtime_command_channel();
    let identity = TransferIdentity {
        transfer_id: 70,
        generation: 9,
    };
    let mut runtime = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::Local,
            directory.path(),
            identity,
            receiver,
        ),
        Some(writer),
        Arc::new(ProtocolGate::new()),
    );
    commands
        .try_send(RuntimeCommand::StartSend {
            identity,
            paths: vec![path.clone()],
        })
        .unwrap();
    runtime.poll();
    commands.try_send(RuntimeCommand::Cancel(identity)).unwrap();
    runtime.poll();

    for transfer_id in [70, 69] {
        commands
            .try_send(RuntimeCommand::StartSend {
                identity: TransferIdentity {
                    transfer_id,
                    generation: 9,
                },
                paths: vec![path.clone()],
            })
            .unwrap();
        assert!(runtime
            .poll()
            .events
            .iter()
            .any(|event| event.kind == RuntimeEventKind::StaleCommand));
        assert!(!runtime.active());
    }

    let max_identity = TransferIdentity {
        transfer_id: u64::MAX,
        generation: 10,
    };
    let max_writer = writer_worker(PartialWriter {
        output: Arc::new(Mutex::new(Vec::new())),
        fail: false,
    });
    let (max_commands, max_receiver) = runtime_command_channel();
    let mut max_runtime = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::Local,
            directory.path(),
            max_identity,
            max_receiver,
        ),
        Some(max_writer),
        Arc::new(ProtocolGate::new()),
    );
    let trigger = encode_zhex_header(FrameType::Zrqinit, [0; 4]);
    let first = max_runtime.feed(&trigger);
    assert_eq!(first.events[0].identity.transfer_id, u64::MAX);
    max_commands
        .try_send(RuntimeCommand::Cancel(max_identity))
        .unwrap();
    max_runtime.poll();
    let exhausted = max_runtime.feed(&trigger);
    assert!(exhausted.discard_reader_epoch);
    assert!(exhausted.events.iter().any(|event| matches!(
        &event.kind,
        RuntimeEventKind::Error(ZmodemError::Protocol(message))
            if message.contains("transfer_id 已耗尽")
    )));
    assert!(!max_runtime.active());
}

#[test]
fn runtime_settings_apply_immediately_when_idle_and_reset_detector() {
    let directory = tempfile::tempdir().unwrap();
    let next_directory = tempfile::tempdir().unwrap();
    let writer = writer_worker(PartialWriter {
        output: Arc::new(Mutex::new(Vec::new())),
        fail: false,
    });
    let (_commands, receiver) = runtime_command_channel();
    let mut runtime = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::Local,
            directory.path(),
            TransferIdentity {
                transfer_id: 100,
                generation: 5,
            },
            receiver,
        ),
        Some(writer),
        Arc::new(ProtocolGate::new()),
    );
    assert!(runtime.feed(b"prefix *").replay.starts_with(b"prefix"));

    let mut settings = RuntimeSettings::new(next_directory.path());
    settings.auto_detect = false;
    settings.transfer_timeout = Some(Duration::from_secs(9));
    runtime.settings_source.update(settings.clone());
    let output = runtime.poll();

    assert_eq!(output.replay, b"*");
    assert_eq!(runtime.config.settings(), settings);
    let trigger = encode_zhex_header(FrameType::Zrqinit, [0; 4]);
    assert_eq!(runtime.feed(&trigger).replay, trigger);
    assert!(!runtime.active());
}

#[test]
fn active_settings_are_deferred_but_disabling_cancels_and_discards() {
    let directory = tempfile::tempdir().unwrap();
    let pending_directory = tempfile::tempdir().unwrap();
    let disabled_directory = tempfile::tempdir().unwrap();
    let writer = writer_worker(PartialWriter {
        output: Arc::new(Mutex::new(Vec::new())),
        fail: false,
    });
    let (_commands, receiver) = runtime_command_channel();
    let identity = TransferIdentity {
        transfer_id: 101,
        generation: 6,
    };
    let mut runtime = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::Local,
            directory.path(),
            identity,
            receiver,
        ),
        Some(writer),
        Arc::new(ProtocolGate::new()),
    );
    let trigger = encode_zhex_header(FrameType::Zrqinit, [0; 4]);
    runtime.feed(&trigger);
    assert!(runtime.active());

    let mut pending = RuntimeSettings::new(pending_directory.path());
    pending.auto_detect = false;
    runtime.settings_source.update(pending.clone());
    let deferred = runtime.poll();
    assert!(deferred.events.is_empty());
    assert_ne!(runtime.config.receive_directory, pending.receive_directory);
    assert_eq!(runtime.pending_settings, Some(pending));

    let mut disabled = RuntimeSettings::new(disabled_directory.path());
    disabled.enabled = false;
    runtime.settings_source.update(disabled.clone());
    let cancelled = runtime.poll();
    assert!(cancelled.discard_reader_epoch);
    assert!(!runtime.active());
    assert_eq!(runtime.config.settings(), disabled);
    assert!(cancelled
        .events
        .iter()
        .any(|event| event.kind == RuntimeEventKind::Finished));
    assert_eq!(runtime.feed(&trigger).replay, trigger);
}

#[test]
fn shared_settings_source_updates_every_runtime_on_its_next_poll() {
    let directory = tempfile::tempdir().unwrap();
    let next_directory = tempfile::tempdir().unwrap();
    let source = RuntimeSettingsSource::new(RuntimeSettings::new(directory.path()));
    let mut runtimes = Vec::new();
    for transfer_id in [110, 111] {
        let (_commands, receiver) = runtime_command_channel();
        let mut config = RuntimeConfig::new(
            RuntimeCapability::Local,
            directory.path(),
            TransferIdentity {
                transfer_id,
                generation: 9,
            },
            receiver,
        );
        config.use_settings_source(source.clone());
        runtimes.push(ZmodemRuntime::new(
            config,
            None,
            Arc::new(ProtocolGate::new()),
        ));
    }
    let mut updated = RuntimeSettings::new(next_directory.path());
    updated.auto_detect = false;
    updated.transfer_timeout = Some(Duration::from_secs(23));
    source.update(updated.clone());

    for runtime in &mut runtimes {
        runtime.poll();
        assert_eq!(runtime.config.settings(), updated);
    }
}

#[test]
fn settings_update_between_config_attachment_and_runtime_start_is_not_lost() {
    let directory = tempfile::tempdir().unwrap();
    let next_directory = tempfile::tempdir().unwrap();
    let source = RuntimeSettingsSource::new(RuntimeSettings::new(directory.path()));
    let (_commands, receiver) = runtime_command_channel();
    let mut config = RuntimeConfig::new(
        RuntimeCapability::Local,
        directory.path(),
        TransferIdentity {
            transfer_id: 113,
            generation: 9,
        },
        receiver,
    );
    config.use_settings_source(source.clone());
    let mut updated = RuntimeSettings::new(next_directory.path());
    updated.auto_detect = false;
    source.update(updated.clone());

    let mut runtime = ZmodemRuntime::new(config, None, Arc::new(ProtocolGate::new()));
    runtime.poll();

    assert_eq!(runtime.config.settings(), updated);
}

#[test]
fn settings_source_recovers_from_poison_and_update_never_fails() {
    let directory = tempfile::tempdir().unwrap();
    let source = RuntimeSettingsSource::new(RuntimeSettings::new(directory.path()));
    let poisoned = source.clone();
    assert!(std::thread::spawn(move || {
        let _guard = poisoned.inner.lock().unwrap();
        panic!("poison settings lock");
    })
    .join()
    .is_err());

    let mut updated = RuntimeSettings::new(directory.path());
    updated.enabled = false;
    source.update(updated.clone());
    assert_eq!(source.snapshot(), (1, updated));
}

#[test]
fn reader_eof_rejects_queued_send_before_runtime_consumes_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("private-name.bin");
    std::fs::write(&path, b"abc").unwrap();
    let (commands, receiver) = runtime_command_channel();
    let identity = TransferIdentity {
        transfer_id: 112,
        generation: 10,
    };
    let mut runtime = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::Local,
            directory.path(),
            identity,
            receiver,
        ),
        None,
        Arc::new(ProtocolGate::new()),
    );
    commands
        .try_send(RuntimeCommand::StartSend {
            identity,
            paths: vec![path.clone()],
        })
        .unwrap();

    let output = runtime.reader_eof();

    assert_eq!(output.events.len(), 1);
    assert_eq!(output.events[0].identity, identity);
    let RuntimeEventKind::Error(ZmodemError::Protocol(message)) = &output.events[0].kind else {
        panic!("queued send must receive a terminal runtime error");
    };
    assert!(message.contains("连接已结束"));
    assert!(!message.contains("private-name"));
    assert!(runtime.config.commands.try_recv().is_err());
}

#[test]
fn reader_eof_and_error_finish_active_transfer_and_clear_gate() {
    for reader_error in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let writer = writer_worker(PartialWriter {
            output: Arc::new(Mutex::new(Vec::new())),
            fail: false,
        });
        let (_commands, receiver) = runtime_command_channel();
        let gate = Arc::new(ProtocolGate::new());
        let mut runtime = ZmodemRuntime::new(
            RuntimeConfig::new(
                RuntimeCapability::Local,
                directory.path(),
                TransferIdentity {
                    transfer_id: 80,
                    generation: 4,
                },
                receiver,
            ),
            Some(writer),
            Arc::clone(&gate),
        );
        runtime.feed(&encode_zhex_header(FrameType::Zrqinit, [0; 4]));
        assert!(runtime.active());
        let output = if reader_error {
            runtime.reader_error(&io::Error::new(io::ErrorKind::Other, "read failed"))
        } else {
            runtime.reader_eof()
        };
        assert!(output.discard_reader_epoch);
        assert!(output
            .events
            .iter()
            .any(|event| matches!(event.kind, RuntimeEventKind::Error(_))));
        assert!(output
            .events
            .iter()
            .any(|event| event.kind == RuntimeEventKind::Finished));
        assert!(!runtime.active());
        assert!(!gate.is_active());
    }
}

#[test]
fn disabled_serial_and_stale_start_send_are_rejected_without_activation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("send.bin");
    std::fs::write(&path, b"abc").unwrap();
    for (capability, enabled) in [
        (RuntimeCapability::Local, false),
        (RuntimeCapability::SerialDisabled, true),
    ] {
        let writer = writer_worker(PartialWriter {
            output: Arc::new(Mutex::new(Vec::new())),
            fail: false,
        });
        let (commands, receiver) = runtime_command_channel();
        let identity = TransferIdentity {
            transfer_id: 60,
            generation: 10,
        };
        let mut config = RuntimeConfig::new(capability, directory.path(), identity, receiver);
        config.enabled = enabled;
        let mut runtime = ZmodemRuntime::new(config, Some(writer), Arc::new(ProtocolGate::new()));
        let trigger = encode_zhex_header(FrameType::Zrqinit, [0; 4]);
        assert_eq!(runtime.feed(&trigger).replay, trigger);
        commands
            .try_send(RuntimeCommand::StartSend {
                identity,
                paths: vec![path.clone()],
            })
            .unwrap();
        assert!(runtime
            .poll()
            .events
            .iter()
            .any(|event| matches!(event.kind, RuntimeEventKind::Error(_))));
        assert!(!runtime.active());
    }

    let writer = writer_worker(PartialWriter {
        output: Arc::new(Mutex::new(Vec::new())),
        fail: false,
    });
    let (commands, receiver) = runtime_command_channel();
    let identity = TransferIdentity {
        transfer_id: 61,
        generation: 11,
    };
    let mut runtime = ZmodemRuntime::new(
        RuntimeConfig::new(
            RuntimeCapability::Local,
            directory.path(),
            identity,
            receiver,
        ),
        Some(writer),
        Arc::new(ProtocolGate::new()),
    );
    commands
        .try_send(RuntimeCommand::StartSend {
            identity: TransferIdentity {
                transfer_id: 61,
                generation: 10,
            },
            paths: vec![path],
        })
        .unwrap();
    assert!(runtime
        .poll()
        .events
        .iter()
        .any(|event| event.kind == RuntimeEventKind::StaleCommand));
    assert!(!runtime.active());
}
