use super::*;

pub fn read_loop<F, G>(terminal: Arc<Mutex<TerminalState>>, request_redraw: F, integration_event: G)
where
    F: Fn() + Send + 'static,
    G: Fn(IntegrationEvent) + Send + 'static,
{
    let mut reader = {
        let mut term = terminal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            let mut term_state = terminal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            term_state.process_pty_output(&mut parser, &buf[..n])
        };

        for event in events {
            integration_event(event);
        }
        request_redraw();
    }

    terminal
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .finish_session();
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
pub fn read_loop_with_zmodem<F, G, H, V, S>(
    terminal: Arc<Mutex<TerminalState>>,
    request_redraw: F,
    integration_event: G,
    visible_output: V,
    config: crate::zmodem::runtime::RuntimeConfig,
    zmodem_event: H,
    session_end: S,
) where
    F: Fn() + Send + 'static,
    G: Fn(IntegrationEvent) + Send + 'static,
    H: Fn(crate::zmodem::runtime::RuntimeEvent) + Send + 'static,
    V: Fn(&[u8]) + Send + 'static,
    S: Fn(Option<String>) + Send + 'static,
{
    let (mut reader, protocol_writer, input_gate) = {
        let mut term = terminal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(reader) = term.take_reader() else {
            // 正常情况下启动读线程时 reader 一定刚由 spawn/apply_*_handle 装上；
            // 走到这里说明会话已被主动关闭或误重复启动，不能贸然发出“已结束”事件
            // （重复启动时另一条读线程可能仍在正常工作），只记录并清理。
            log::warn!("终端读线程启动时 reader 已被取走，跳过本次读循环");
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
    // 结束原因只用于 UI 提示：正常 EOF（进程退出/连接关闭）为 None，
    // 读取错误时记录底层错误文本。
    let mut end_reason: Option<String> = None;

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
                // Linux 上本地 PTY 的子进程退出后，master 读取常返回 EIO 而不是 EOF，
                // 因此错误同样视为传输结束，只是把错误文本留给 UI。
                log::error!("读取错误: {error}");
                end_reason = Some(error.to_string());
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
    terminal
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .finish_session();
    // 先完成本地清理再通知：主线程可能立刻在同一面板上重启 shell。
    session_end(end_reason);
}
