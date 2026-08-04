use super::{
    bootstrap_shell, cleanup_targets, configure_tcp_timeouts, connect, connect_resolved,
    connect_resolved_with_clock, read_probe_shell, resolve_ssh_addresses, shutdown_requested,
    with_temporary_ssh_timeout, write_channel_once, ConnectionParams, PendingChannelWrite,
    SessionTimeoutControl, ShellBootstrap, SshHandle,
};
use crate::bash_integration::RemoteBashRuntime;
use crate::sidebar::SshConnection;
use crate::smart_completion::CompletionSessionKey;

#[test]
fn resolver_accepts_bare_ipv6_host() {
    let addresses = resolve_ssh_addresses("::1", 2222).unwrap();

    assert!(!addresses.is_empty());
    assert!(addresses
        .iter()
        .all(|address| address.is_ipv6() && address.port() == 2222));
}

#[test]
fn connector_tries_later_address_after_first_failure() {
    let first = "127.0.0.1:2201".parse().unwrap();
    let second = "127.0.0.1:2202".parse().unwrap();
    let mut attempts = Vec::new();

    let connected = connect_resolved(
        &[first, second],
        std::time::Duration::from_secs(1),
        |address, remaining| {
            attempts.push((address, remaining));
            if address == first {
                Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    "first refused",
                ))
            } else {
                Ok(address)
            }
        },
    )
    .unwrap();

    assert_eq!(connected, second);
    assert_eq!(
        attempts
            .iter()
            .map(|(address, _)| *address)
            .collect::<Vec<_>>(),
        [first, second]
    );
    assert!(attempts
        .iter()
        .all(|(_, remaining)| *remaining <= std::time::Duration::from_secs(1)));
}

#[test]
fn connector_reserves_deadline_budget_for_later_addresses() {
    let first = "127.0.0.1:2201".parse().unwrap();
    let second = "127.0.0.1:2202".parse().unwrap();
    let started = std::time::Instant::now();
    let elapsed = std::rc::Rc::new(std::cell::Cell::new(std::time::Duration::from_secs(0)));
    let clock_elapsed = elapsed.clone();
    let connector_elapsed = elapsed.clone();
    let mut attempts = Vec::new();

    let connected = connect_resolved_with_clock(
        &[first, second],
        std::time::Duration::from_secs(10),
        move || started + clock_elapsed.get(),
        |address, attempt_timeout| {
            attempts.push((address, attempt_timeout));
            if address == first {
                connector_elapsed.set(connector_elapsed.get() + attempt_timeout);
                Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "first timed out",
                ))
            } else {
                Ok(address)
            }
        },
    )
    .unwrap();

    assert_eq!(connected, second);
    assert_eq!(
        attempts,
        [
            (first, std::time::Duration::from_secs(5)),
            (second, std::time::Duration::from_secs(5)),
        ]
    );
}

#[test]
fn connection_params_preserve_all_auth_fields() {
    let source = SshConnection {
        label: "生产机".into(),
        host: "server.example.com".into(),
        port: 2222,
        user: "deploy".into(),
        auth: "key".into(),
        key_path: "~/.ssh/id_ed25519".into(),
        password: "passphrase".into(),
        group: "prod".into(),
        group_color: [1, 2, 3],
    };

    let params = ConnectionParams::from(&source);
    assert_eq!(params.host, "server.example.com");
    assert_eq!(params.port, 2222);
    assert_eq!(params.user, "deploy");
    assert_eq!(params.auth, "key");
    assert_eq!(params.key_path, "~/.ssh/id_ed25519");
    assert_eq!(params.password, "passphrase");
}

#[test]
fn connection_params_debug_redacts_key_path_and_password() {
    let params = ConnectionParams {
        host: "server.example.com".into(),
        port: 2222,
        user: "deploy".into(),
        auth: "key".into(),
        key_path: "KEY_PATH_SENTINEL".into(),
        password: "PASSWORD_SENTINEL".into(),
    };

    let debug = format!("{params:?}");

    assert!(debug.contains("server.example.com"));
    assert!(debug.contains("deploy"));
    assert!(debug.contains("key"));
    assert!(!debug.contains("KEY_PATH_SENTINEL"));
    assert!(!debug.contains("PASSWORD_SENTINEL"));
}

#[test]
fn tcp_timeouts_cover_ssh_handshake_and_auth_io() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let timeout = std::time::Duration::from_millis(321);

    configure_tcp_timeouts(&stream, timeout).unwrap();

    for configured in [
        stream.read_timeout().unwrap().unwrap(),
        stream.write_timeout().unwrap().unwrap(),
    ] {
        assert!(configured >= timeout);
        assert!(configured <= timeout + std::time::Duration::from_millis(20));
    }
}

struct FakeSessionTimeout(std::cell::Cell<u32>);

impl SessionTimeoutControl for FakeSessionTimeout {
    fn timeout_ms(&self) -> u32 {
        self.0.get()
    }

    fn set_timeout_ms(&self, timeout_ms: u32) {
        self.0.set(timeout_ms);
    }
}

#[test]
fn temporary_ssh_timeout_restores_previous_value_after_failure() {
    let session = FakeSessionTimeout(std::cell::Cell::new(9_000));

    let result = with_temporary_ssh_timeout(&session, 3_000, || {
        assert_eq!(session.0.get(), 3_000);
        Err::<(), _>("probe failed")
    });

    assert_eq!(result, Err("probe failed"));
    assert_eq!(session.0.get(), 9_000);
}

struct FixtureProcess {
    child: Option<std::process::Child>,
}

impl FixtureProcess {
    fn spawn(test_name: &str) -> Result<Self, String> {
        let child = std::process::Command::new(
            std::env::current_exe()
                .map_err(|error| format!("读取当前测试程序路径失败: {error}"))?,
        )
        .arg("--exact")
        .arg(test_name)
        .arg("--ignored")
        .arg("--nocapture")
        .env("LITETERM_TEST_SSH_WORKER", "1")
        .spawn()
        .map_err(|error| format!("启动 SSH fixture 子进程失败: {error}"))?;
        Ok(Self { child: Some(child) })
    }

    fn wait_with_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<std::process::ExitStatus, String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let child = self
                .child
                .as_mut()
                .ok_or_else(|| "SSH fixture 子进程已被回收".to_string())?;
            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("查询 SSH fixture 子进程失败: {error}"))?
            {
                self.child.take();
                return Ok(status);
            }
            if std::time::Instant::now() >= deadline {
                self.stop();
                return Err(format!("SSH fixture 超过 {} 秒", timeout.as_secs()));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_ssh_fixture_with_total_timeout(test_name: &str, timeout: std::time::Duration) {
    let mut process = FixtureProcess::spawn(test_name).unwrap();
    let status = process.wait_with_timeout(timeout).unwrap();
    assert!(status.success(), "SSH fixture 子进程失败: {status}");
}

// Optional fixture command:
// cd native-prototype
// cargo test ssh::tests::real_ssh_bash_bootstrap_and_local_io_shutdown -- --ignored --nocapture
// The assertion covers local SSH I/O teardown only. Remote trap cleanup remains
// best-effort because this fixture does not open a second session to inspect it.
#[test]
#[ignore = "requires LITETERM_TEST_SSH_HOST/USER and key or password"]
fn real_ssh_bash_bootstrap_and_local_io_shutdown() {
    if std::env::var_os("LITETERM_TEST_SSH_WORKER").is_none() {
        run_ssh_fixture_with_total_timeout(
            "ssh::tests::real_ssh_bash_bootstrap_and_local_io_shutdown",
            std::time::Duration::from_secs(30),
        );
        return;
    }

    let params = ConnectionParams {
        host: std::env::var("LITETERM_TEST_SSH_HOST").unwrap(),
        port: std::env::var("LITETERM_TEST_SSH_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(22),
        user: std::env::var("LITETERM_TEST_SSH_USER").unwrap(),
        auth: std::env::var("LITETERM_TEST_SSH_AUTH").unwrap_or_else(|_| "key".into()),
        key_path: std::env::var("LITETERM_TEST_SSH_KEY").unwrap_or_default(),
        password: std::env::var("LITETERM_TEST_SSH_PASSWORD").unwrap_or_default(),
    };
    let session = CompletionSessionKey::new_for_test(1, "abcdef12");
    let handle = connect(&params, 80, 24, Some(session)).unwrap();
    assert!(
        handle.bash_runtime.is_some(),
        "fixture login shell must be Bash"
    );
    handle
        .shutdown_and_wait(std::time::Duration::from_secs(5))
        .unwrap();
}

#[derive(Clone, Copy)]
enum FakeFailure {
    None,
    Probe,
    Deploy,
    IntegratedOpen,
}

struct FakeBootstrap {
    failure: FakeFailure,
    shell: String,
    calls: Vec<&'static str>,
}

impl FakeBootstrap {
    fn bash_success() -> Self {
        Self {
            failure: FakeFailure::None,
            shell: "/bin/bash".into(),
            calls: Vec::new(),
        }
    }

    fn failing(failure: FakeFailure) -> Self {
        Self {
            failure,
            ..Self::bash_success()
        }
    }
}

fn test_session() -> CompletionSessionKey {
    CompletionSessionKey::new_for_test(1, "abcdef12")
}

impl ShellBootstrap for FakeBootstrap {
    fn probe_login_shell(&mut self) -> Result<String, String> {
        self.calls.push("probe");
        if matches!(self.failure, FakeFailure::Probe) {
            Err("probe failed".into())
        } else {
            Ok(self.shell.clone())
        }
    }

    fn deploy_bash_runtime(
        &mut self,
        session: &CompletionSessionKey,
        bash_path: &str,
    ) -> Result<RemoteBashRuntime, String> {
        self.calls.push("deploy");
        if matches!(self.failure, FakeFailure::Deploy) {
            return Err("deploy failed".into());
        }
        Ok(RemoteBashRuntime {
            session: session.clone(),
            bash_path: bash_path.into(),
            rc_path: "/tmp/session.bash".into(),
            candidate_path: "/tmp/candidate".into(),
            widget_sequence: "\x1b[777;1~".into(),
            snapshot_sequence: crate::bash_integration::snapshot_sequence(session),
        })
    }

    fn open_integrated_bash(&mut self, _: &RemoteBashRuntime) -> Result<(), String> {
        self.calls.push("open_integrated");
        if matches!(self.failure, FakeFailure::IntegratedOpen) {
            Err("exec failed".into())
        } else {
            Ok(())
        }
    }

    fn cleanup_bash_runtime(&mut self, _: &RemoteBashRuntime) {
        self.calls.push("cleanup");
    }

    fn open_plain_shell(&mut self) -> Result<(), String> {
        self.calls.push("open_plain");
        Ok(())
    }
}

#[test]
fn bootstrap_success_uses_integrated_bash_without_plain_fallback() {
    let mut bootstrap = FakeBootstrap::bash_success();

    let runtime = bootstrap_shell(&mut bootstrap, test_session()).unwrap();

    assert_eq!(runtime.unwrap().bash_path, "/bin/bash");
    assert_eq!(bootstrap.calls, ["probe", "deploy", "open_integrated"]);
}

#[test]
fn bootstrap_probe_failure_opens_plain_shell() {
    let mut bootstrap = FakeBootstrap::failing(FakeFailure::Probe);

    assert!(bootstrap_shell(&mut bootstrap, test_session())
        .unwrap()
        .is_none());
    assert_eq!(bootstrap.calls, ["probe", "open_plain"]);
}

#[test]
fn bootstrap_rejects_unsafe_or_non_bash_shell_before_deployment() {
    for shell in ["bash", "/bin/fish", "/bin/'bash'", "/bin/ba\nsh"] {
        let mut bootstrap = FakeBootstrap::bash_success();
        bootstrap.shell = shell.into();

        assert!(bootstrap_shell(&mut bootstrap, test_session())
            .unwrap()
            .is_none());
        assert_eq!(bootstrap.calls, ["probe", "open_plain"]);
    }
}

#[test]
fn bootstrap_deploy_failure_opens_plain_shell() {
    let mut bootstrap = FakeBootstrap::failing(FakeFailure::Deploy);

    assert!(bootstrap_shell(&mut bootstrap, test_session())
        .unwrap()
        .is_none());
    assert_eq!(bootstrap.calls, ["probe", "deploy", "open_plain"]);
}

#[test]
fn bootstrap_exec_failure_cleans_up_then_opens_plain_shell() {
    let mut bootstrap = FakeBootstrap::failing(FakeFailure::IntegratedOpen);

    assert!(bootstrap_shell(&mut bootstrap, test_session())
        .unwrap()
        .is_none());
    assert_eq!(
        bootstrap.calls,
        [
            "probe",
            "deploy",
            "open_integrated",
            "cleanup",
            "open_plain"
        ]
    );
}

#[test]
fn ssh_shutdown_signal_is_independent_from_terminal_input() {
    let (write_tx, write_rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();

    write_tx.send(vec![b'x']).unwrap();
    shutdown_tx.send(()).unwrap();

    assert_eq!(write_rx.try_recv().unwrap(), vec![b'x']);
    assert!(shutdown_rx.try_recv().is_ok());
}

struct NonblockingPartialWriter {
    output: Vec<u8>,
    calls: usize,
}

impl std::io::Write for NonblockingPartialWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.calls += 1;
        if self.calls == 1 {
            return Err(std::io::Error::from(std::io::ErrorKind::WouldBlock));
        }
        let take = bytes.len().min(2);
        self.output.extend_from_slice(&bytes[..take]);
        Ok(take)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn ssh_nonblocking_writer_preserves_partial_progress_and_yields() {
    let mut writer = NonblockingPartialWriter {
        output: Vec::new(),
        calls: 0,
    };
    let mut pending = PendingChannelWrite {
        bytes: b"abcdef".to_vec(),
        offset: 0,
        protocol: None,
        terminal_reply: None,
        normal_epoch: Some(0),
        started: true,
    };

    assert!(!write_channel_once(&mut writer, &mut pending).unwrap());
    assert_eq!(pending.offset, 0);
    while !write_channel_once(&mut writer, &mut pending).unwrap() {}
    assert_eq!(writer.output, b"abcdef");
    assert!(writer.calls >= 4);
}

#[test]
fn ssh_terminal_reply_ack_follows_channel_write_and_flush() {
    let gate = std::sync::Arc::new(crate::zmodem::runtime::ProtocolGate::new());
    let (transport, receiver) = crate::zmodem::runtime::transport_write_channel(gate);
    let reply_writer =
        crate::zmodem::runtime::TerminalReplyWriter::from_transport_writer(transport);
    let reply = std::thread::spawn(move || {
        reply_writer.write_and_flush(b"\x1b[1;212R", std::time::Duration::from_secs(1))
    });
    let message = receiver
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    let mut pending = PendingChannelWrite::from_transport(message);
    let mut writer = NonblockingPartialWriter {
        output: Vec::new(),
        calls: 0,
    };

    assert!(pending.begin());
    while !write_channel_once(&mut writer, &mut pending).unwrap() {}
    pending.complete(Ok(()));

    reply.join().unwrap().unwrap();
    assert_eq!(writer.output, b"\x1b[1;212R");
}

#[test]
fn ssh_handle_shutdown_and_wait_receives_io_completion() {
    let (write_tx, _write_rx) = crate::zmodem::runtime::transport_write_channel(
        std::sync::Arc::new(crate::zmodem::runtime::ProtocolGate::new()),
    );
    let (resize_tx, _resize_rx) = std::sync::mpsc::sync_channel(8);
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();
    let (io_done_tx, io_done_rx) = std::sync::mpsc::channel();
    let io_thread = std::thread::spawn(move || {
        shutdown_rx.recv().unwrap();
        io_done_tx.send(()).unwrap();
    });
    let handle = SshHandle {
        reader: Box::new(std::io::empty()),
        write_tx,
        resize_tx,
        shutdown_tx,
        io_done_rx,
        bash_runtime: None,
    };

    handle
        .shutdown_and_wait(std::time::Duration::from_secs(1))
        .unwrap();
    io_thread.join().unwrap();
}

#[test]
fn ssh_handle_shutdown_and_wait_succeeds_after_natural_io_exit() {
    let (write_tx, _write_rx) = crate::zmodem::runtime::transport_write_channel(
        std::sync::Arc::new(crate::zmodem::runtime::ProtocolGate::new()),
    );
    let (resize_tx, _resize_rx) = std::sync::mpsc::sync_channel(8);
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();
    let (io_done_tx, io_done_rx) = std::sync::mpsc::channel();
    drop(shutdown_rx);
    io_done_tx.send(()).unwrap();

    let handle = SshHandle {
        reader: Box::new(std::io::empty()),
        write_tx,
        resize_tx,
        shutdown_tx,
        io_done_rx,
        bash_runtime: None,
    };

    handle
        .shutdown_and_wait(std::time::Duration::from_secs(1))
        .expect("自然退出的 SSH I/O 完成通知必须保持可观察");
}

#[test]
fn dropped_shutdown_sender_still_requests_io_loop_shutdown() {
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();
    drop(shutdown_tx);

    assert!(shutdown_requested(&shutdown_rx.try_recv()));
}

#[test]
fn empty_shutdown_channel_keeps_io_loop_running() {
    let (_shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();

    assert!(!shutdown_requested(&shutdown_rx.try_recv()));
}

#[test]
fn login_shell_probe_is_bounded_to_4096_bytes() {
    let bytes = vec![b'x'; 5000];
    let mut reader = std::io::Cursor::new(bytes);

    let shell = read_probe_shell(&mut reader).unwrap();

    assert_eq!(shell.len(), 4096);
    assert_eq!(reader.position(), 4096);
}

#[test]
fn deployment_cleanup_targets_only_files_created_by_this_attempt() {
    let paths = crate::bash_integration::RemoteBashPaths {
        rc: "/tmp/session.rc".into(),
        candidate: "/tmp/session.candidate".into(),
    };

    assert!(cleanup_targets(&paths, false, false).is_empty());
    assert_eq!(cleanup_targets(&paths, true, false), ["/tmp/session.rc"]);
    assert_eq!(
        cleanup_targets(&paths, false, true),
        ["/tmp/session.candidate"]
    );
    assert_eq!(
        cleanup_targets(&paths, true, true),
        ["/tmp/session.rc", "/tmp/session.candidate"]
    );
}
