use super::*;

pub fn read_loop<F, G>(terminal: Arc<Mutex<TerminalState>>, request_redraw: F, integration_event: G)
where
    F: Fn() + Send + 'static,
    G: Fn(IntegrationEvent) + Send + 'static,
{
    let mut reader = {
        let mut term = terminal.lock().unwrap();
        match term.take_reader() {
            Some(r) => r,
            None => {
                term.finish_session();
                return;
            }
        }
    };

    let mut buf = [0u8; 8192];
    let mut parser = Processor::new();

    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                log::error!("读取错误: {}", e);
                break;
            }
        };

        let events = {
            let mut term_state = terminal.lock().unwrap();
            term_state.process_pty_output(&mut parser, &buf[..n])
        };

        for event in events {
            integration_event(event);
        }
        request_redraw();
    }

    terminal.lock().unwrap().finish_session();
}

enum ReaderPumpEvent {
    Data { sequence: u64, bytes: Vec<u8> },
    Eof,
    Error(std::io::Error),
}

/// Enhanced reader entry point used by the ZMODEM integration layer.
///
/// The legacy `read_loop` remains available for existing callers. This entry
/// moves the potentially-blocking reader into a detached bounded pump so
/// cancel/timeout/shutdown commands are polled at a fixed interval even while
/// the underlying transport is blocked in `read`.
pub fn read_loop_with_zmodem<F, G, H, V>(
    terminal: Arc<Mutex<TerminalState>>,
    request_redraw: F,
    integration_event: G,
    visible_output: V,
    config: crate::zmodem::runtime::RuntimeConfig,
    zmodem_event: H,
) where
    F: Fn() + Send + 'static,
    G: Fn(IntegrationEvent) + Send + 'static,
    H: Fn(crate::zmodem::runtime::RuntimeEvent) + Send + 'static,
    V: Fn(&[u8]) + Send + 'static,
{
    let (mut reader, protocol_writer, input_gate) = {
        let mut term = terminal.lock().unwrap();
        let Some(reader) = term.take_reader() else {
            term.finish_session();
            return;
        };
        (
            reader,
            term.zmodem_protocol_writer(),
            term.zmodem_input_gate(),
        )
    };
    let (pump_tx, pump_rx) = mpsc::sync_channel(crate::zmodem::runtime::READER_PUMP_CAPACITY);
    // A read is ordered at completion, not when the blocking syscall starts.
    // Cancellation records the latest completed sequence. This discards bytes
    // already cached by the pump while allowing a read that was blocked during
    // cancellation to deliver the later normal prompt.
    let reader_sequence = Arc::new(Mutex::new(0u64));
    let pump_sequence = Arc::clone(&reader_sequence);
    std::thread::Builder::new()
        .name("terminal-reader-pump".into())
        .spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                let event = match reader.read(&mut buffer) {
                    Ok(0) => ReaderPumpEvent::Eof,
                    Ok(read) => {
                        let sequence = {
                            let mut next = pump_sequence.lock().unwrap();
                            *next = next.saturating_add(1);
                            *next
                        };
                        ReaderPumpEvent::Data {
                            sequence,
                            bytes: buffer[..read].to_vec(),
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => ReaderPumpEvent::Error(error),
                };
                let terminal = !matches!(event, ReaderPumpEvent::Data { .. });
                if pump_tx.send(event).is_err() || terminal {
                    break;
                }
            }
        })
        .expect("创建终端 reader pump 失败");

    let mut runtime =
        crate::zmodem::runtime::ZmodemRuntime::new(config, protocol_writer, input_gate);
    let mut parser = Processor::new();
    let mut stopped = false;
    let mut discard_through_sequence = 0;

    while !stopped {
        let control = runtime.poll();
        if control.discard_reader_epoch {
            discard_through_sequence =
                discard_through_sequence.max(*reader_sequence.lock().unwrap());
        }
        for event in control.events {
            zmodem_event(event);
        }
        if control.shutdown {
            break;
        }
        if !control.replay.is_empty() {
            visible_output(&control.replay);
            let events = terminal
                .lock()
                .unwrap()
                .process_pty_output(&mut parser, &control.replay);
            for event in events {
                integration_event(event);
            }
            request_redraw();
        }

        let poll_interval = if runtime.active() {
            crate::zmodem::runtime::ACTIVE_READER_POLL_INTERVAL
        } else {
            crate::zmodem::runtime::READER_POLL_INTERVAL
        };
        match pump_rx.recv_timeout(poll_interval) {
            Ok(ReaderPumpEvent::Data { sequence, bytes }) => {
                if sequence <= discard_through_sequence {
                    continue;
                }
                let output = runtime.feed(&bytes);
                if output.discard_reader_epoch {
                    discard_through_sequence =
                        discard_through_sequence.max(*reader_sequence.lock().unwrap());
                } else if !output.replay.is_empty() {
                    visible_output(&output.replay);
                    let events = terminal
                        .lock()
                        .unwrap()
                        .process_pty_output(&mut parser, &output.replay);
                    for event in events {
                        integration_event(event);
                    }
                    request_redraw();
                }
                for event in output.events {
                    zmodem_event(event);
                }
                stopped = output.shutdown;
            }
            Ok(ReaderPumpEvent::Eof) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let output = runtime.reader_eof();
                for event in output.events {
                    zmodem_event(event);
                }
                break;
            }
            Ok(ReaderPumpEvent::Error(error)) => {
                log::error!("读取错误: {error}");
                let output = runtime.reader_error(&error);
                for event in output.events {
                    zmodem_event(event);
                }
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }

    drop(runtime);
    terminal.lock().unwrap().finish_session();
}
