use super::*;

fn params() -> ConnectionParams {
    ConnectionParams {
        host: "example.test".to_string(),
        port: 22,
        user: "tester".to_string(),
        auth: "password".to_string(),
        key_path: "/secret/id".to_string(),
        password: "super-secret".to_string(),
    }
}

fn spec() -> TunnelSpec {
    TunnelSpec {
        connection: params(),
        local_port: 8080,
        remote_host: "127.0.0.1".to_string(),
        remote_port: 80,
    }
}

#[test]
fn tunnel_only_listens_on_loopback() {
    assert_eq!(
        spec().local_addr(),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080)
    );
}

#[test]
fn spec_rejects_empty_hosts_and_zero_ports() {
    let mut value = spec();
    value.local_port = 0;
    assert!(value.validate().unwrap_err().contains("本地端口"));
    value = spec();
    value.remote_port = 0;
    assert!(value.validate().unwrap_err().contains("远端端口"));
    value = spec();
    value.remote_host = "  ".to_string();
    assert!(value.validate().unwrap_err().contains("远端主机"));
}

#[test]
fn status_machine_allows_only_lifecycle_transitions() {
    assert!(status_transition_allowed(
        &TunnelStatus::Connecting,
        &TunnelStatus::Active
    ));
    assert!(status_transition_allowed(
        &TunnelStatus::Active,
        &TunnelStatus::Closing
    ));
    assert!(status_transition_allowed(
        &TunnelStatus::Closing,
        &TunnelStatus::Stopped
    ));
    assert!(!status_transition_allowed(
        &TunnelStatus::Closing,
        &TunnelStatus::Active
    ));
    assert!(!status_transition_allowed(
        &TunnelStatus::Stopped,
        &TunnelStatus::Active
    ));
}

fn registry_with_record() -> TunnelRegistry {
    let mut registry = TunnelRegistry::new();
    let (command_tx, _command_rx) = mpsc::channel();
    registry.records.insert(
        7,
        TunnelRecord {
            info: TunnelInfo {
                id: 7,
                generation: 11,
                spec: spec(),
                status: TunnelStatus::Active,
            },
            command_tx,
            worker: None,
            exited: false,
        },
    );
    registry
}

#[test]
fn close_is_idempotent() {
    let mut registry = registry_with_record();
    assert!(registry.close(7));
    assert!(registry.close(7));
    assert_eq!(
        registry.info(7).map(|info| &info.status),
        Some(&TunnelStatus::Closing)
    );
}

#[test]
fn stale_event_does_not_change_current_generation() {
    let mut registry = registry_with_record();
    assert!(!registry.apply_event(&TunnelEvent {
        id: 7,
        generation: 10,
        status: TunnelStatus::Failed("old".to_string()),
    }));
    assert_eq!(
        registry.info(7).map(|info| &info.status),
        Some(&TunnelStatus::Active)
    );
}

struct ScriptedWriter {
    writes: VecDeque<io::Result<usize>>,
    bytes: Vec<u8>,
}

impl Write for ScriptedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self.writes.pop_front().expect("missing scripted write") {
            Ok(count) => {
                let count = count.min(buffer.len());
                self.bytes.extend_from_slice(&buffer[..count]);
                Ok(count)
            }
            Err(error) => Err(error),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn pending_buffer_preserves_partial_write_and_would_block() {
    let mut pending = PendingBuffer::default();
    pending.append(b"abcdef");
    let mut writer = ScriptedWriter {
        writes: VecDeque::from([
            Ok(2),
            Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Ok(4),
        ]),
        bytes: Vec::new(),
    };
    assert_eq!(
        write_pending(&mut writer, &mut pending).unwrap(),
        WriteProgress::Progress(2)
    );
    assert_eq!(
        write_pending(&mut writer, &mut pending).unwrap(),
        WriteProgress::WouldBlock
    );
    assert_eq!(pending.pending(), b"cdef");
    assert_eq!(
        write_pending(&mut writer, &mut pending).unwrap(),
        WriteProgress::Progress(4)
    );
    assert!(pending.is_empty());
    assert_eq!(writer.bytes, b"abcdef");
}

struct CountingReader {
    bytes: VecDeque<u8>,
    reads: usize,
}

impl Read for CountingReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        let count = buffer.len().min(self.bytes.len());
        for target in &mut buffer[..count] {
            *target = self.bytes.pop_front().expect("count was bounded");
        }
        Ok(count)
    }
}

#[test]
fn full_blocked_direction_does_not_starve_opposite_direction() {
    let mut to_remote = PendingBuffer::default();
    to_remote.append(&vec![1; MAX_PENDING_BYTES]);
    let mut blocked_remote_writer = ScriptedWriter {
        writes: VecDeque::from([Err(io::Error::from(io::ErrorKind::WouldBlock))]),
        bytes: Vec::new(),
    };
    assert_eq!(
        write_pending(&mut blocked_remote_writer, &mut to_remote).unwrap(),
        WriteProgress::WouldBlock
    );

    let mut local_reader = CountingReader {
        bytes: VecDeque::from(b"must-not-read".to_vec()),
        reads: 0,
    };
    assert_eq!(
        read_into_pending(&mut local_reader, &mut to_remote).unwrap(),
        ReadProgress::Backpressured
    );
    assert_eq!(local_reader.reads, 0);

    let mut remote_reader = CountingReader {
        bytes: VecDeque::from(b"response".to_vec()),
        reads: 0,
    };
    let mut to_local = PendingBuffer::default();
    assert_eq!(
        read_into_pending(&mut remote_reader, &mut to_local).unwrap(),
        ReadProgress::Progress(8)
    );
    let mut local_writer = ScriptedWriter {
        writes: VecDeque::from([Ok(8)]),
        bytes: Vec::new(),
    };
    assert_eq!(
        write_pending(&mut local_writer, &mut to_local).unwrap(),
        WriteProgress::Progress(8)
    );
    assert_eq!(local_writer.bytes, b"response");
    assert_eq!(to_remote.len(), MAX_PENDING_BYTES);
}

#[test]
fn half_close_actions_wait_for_each_direction_to_drain() {
    let mut state = HalfCloseState {
        local_read_eof: true,
        remote_read_eof: false,
        remote_write_eof_sent: false,
        local_write_shutdown: false,
    };
    assert_eq!(
        state.actions(false, true),
        HalfCloseActions {
            send_remote_eof: false,
            shutdown_local_write: false,
            complete: false,
        }
    );
    assert!(state.actions(true, true).send_remote_eof);
    state.remote_write_eof_sent = true;
    state.remote_read_eof = true;
    assert_eq!(
        state.actions(true, false),
        HalfCloseActions {
            send_remote_eof: false,
            shutdown_local_write: false,
            complete: false,
        }
    );
    assert!(state.actions(true, true).shutdown_local_write);
    state.local_write_shutdown = true;
    assert!(state.actions(true, true).complete);
}

#[test]
fn debug_output_redacts_credentials_and_failure_details() {
    let debug = format!("{:?}", spec());
    assert!(!debug.contains("super-secret"));
    assert!(!debug.contains("/secret/id"));
    let failed = format!("{:?}", TunnelStatus::Failed("/secret/id".to_string()));
    assert!(!failed.contains("/secret/id"));
}
