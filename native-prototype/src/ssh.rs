use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::bash_integration::{
    build_bash_rc, is_safe_remote_bash_path, widget_sequence, RemoteBashPaths, RemoteBashRuntime,
};
use crate::smart_completion::CompletionSessionKey;

const SSH_IO_TIMEOUT: Duration = Duration::from_secs(10);
const SSH_IO_TIMEOUT_MS: u32 = 10_000;

trait SessionTimeoutControl {
    fn timeout_ms(&self) -> u32;
    fn set_timeout_ms(&self, timeout_ms: u32);
}

impl SessionTimeoutControl for ssh2::Session {
    fn timeout_ms(&self) -> u32 {
        self.timeout()
    }

    fn set_timeout_ms(&self, timeout_ms: u32) {
        self.set_timeout(timeout_ms);
    }
}

struct SessionTimeoutRestore<'session, Session: SessionTimeoutControl> {
    session: &'session Session,
    previous_timeout_ms: u32,
}

impl<Session: SessionTimeoutControl> Drop for SessionTimeoutRestore<'_, Session> {
    fn drop(&mut self) {
        self.session.set_timeout_ms(self.previous_timeout_ms);
    }
}

fn with_temporary_ssh_timeout<Session, Output>(
    session: &Session,
    timeout_ms: u32,
    operation: impl FnOnce() -> Output,
) -> Output
where
    Session: SessionTimeoutControl,
{
    let previous_timeout_ms = session.timeout_ms();
    session.set_timeout_ms(timeout_ms);
    let restore = SessionTimeoutRestore {
        session,
        previous_timeout_ms,
    };
    let output = operation();
    drop(restore);
    output
}

fn configure_tcp_timeouts(tcp: &TcpStream, timeout: Duration) -> Result<(), String> {
    tcp.set_read_timeout(Some(timeout))
        .map_err(|error| format!("设置 SSH TCP 读超时失败: {error}"))?;
    tcp.set_write_timeout(Some(timeout))
        .map_err(|error| format!("设置 SSH TCP 写超时失败: {error}"))
}

fn resolve_ssh_addresses(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| format!("DNS 解析失败: {error}"))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        Err("DNS 解析没有返回可用地址".to_string())
    } else {
        Ok(addresses)
    }
}

fn connect_resolved<Connected>(
    addresses: &[SocketAddr],
    timeout: Duration,
    connector: impl FnMut(SocketAddr, Duration) -> std::io::Result<Connected>,
) -> Result<Connected, String> {
    connect_resolved_with_clock(addresses, timeout, Instant::now, connector)
}

fn connect_resolved_with_clock<Connected>(
    addresses: &[SocketAddr],
    timeout: Duration,
    mut now: impl FnMut() -> Instant,
    mut connector: impl FnMut(SocketAddr, Duration) -> std::io::Result<Connected>,
) -> Result<Connected, String> {
    let deadline = now()
        .checked_add(timeout)
        .ok_or_else(|| "SSH TCP 连接超时无效".to_string())?;
    let mut last_error = None;

    for (index, &address) in addresses.iter().enumerate() {
        let remaining = deadline.saturating_duration_since(now());
        if remaining.is_zero() {
            return Err("SSH TCP 连接超时".to_string());
        }
        let addresses_left = u32::try_from(addresses.len() - index)
            .map_err(|_| "SSH 地址数量超出支持范围".to_string())?;
        let attempt_timeout = remaining / addresses_left;
        if attempt_timeout.is_zero() {
            return Err("SSH TCP 连接超时".to_string());
        }
        match connector(address, attempt_timeout) {
            Ok(connected) => return Ok(connected),
            Err(error) => last_error = Some(error),
        }
    }

    match last_error {
        Some(error) => Err(format!("TCP 连接失败: {error}")),
        None => Err("没有可用的 SSH 地址".to_string()),
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: String,
    pub key_path: String,
    pub password: String,
}

impl std::fmt::Debug for ConnectionParams {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectionParams")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("auth", &self.auth)
            .finish_non_exhaustive()
    }
}

impl From<&crate::sidebar::SshConnection> for ConnectionParams {
    fn from(connection: &crate::sidebar::SshConnection) -> Self {
        Self {
            host: connection.host.clone(),
            port: connection.port,
            user: connection.user.clone(),
            auth: connection.auth.clone(),
            key_path: connection.key_path.clone(),
            password: connection.password.clone(),
        }
    }
}

trait ShellBootstrap {
    fn probe_login_shell(&mut self) -> Result<String, String>;
    fn deploy_bash_runtime(
        &mut self,
        session: &CompletionSessionKey,
        bash_path: &str,
    ) -> Result<RemoteBashRuntime, String>;
    fn open_integrated_bash(&mut self, runtime: &RemoteBashRuntime) -> Result<(), String>;
    fn cleanup_bash_runtime(&mut self, runtime: &RemoteBashRuntime);
    fn open_plain_shell(&mut self) -> Result<(), String>;
}

fn bootstrap_shell<B: ShellBootstrap>(
    bootstrap: &mut B,
    session: CompletionSessionKey,
) -> Result<Option<RemoteBashRuntime>, String> {
    let shell = match bootstrap.probe_login_shell() {
        Ok(shell) if is_safe_remote_bash_path(&shell) => shell,
        _ => {
            bootstrap.open_plain_shell()?;
            return Ok(None);
        }
    };

    let runtime = match bootstrap.deploy_bash_runtime(&session, &shell) {
        Ok(runtime) => runtime,
        Err(_) => {
            bootstrap.open_plain_shell()?;
            return Ok(None);
        }
    };

    if bootstrap.open_integrated_bash(&runtime).is_err() {
        bootstrap.cleanup_bash_runtime(&runtime);
        bootstrap.open_plain_shell()?;
        return Ok(None);
    }
    Ok(Some(runtime))
}

fn read_probe_shell(reader: &mut impl Read) -> Result<String, String> {
    let mut bytes = Vec::new();
    reader
        .take(4096)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取登录 shell 失败: {error}"))?;
    String::from_utf8(bytes).map_err(|_| "登录 shell 路径不是 UTF-8".to_string())
}

fn cleanup_targets(
    paths: &RemoteBashPaths,
    created_rc: bool,
    created_candidate: bool,
) -> Vec<&str> {
    let mut targets = Vec::with_capacity(2);
    if created_rc {
        targets.push(paths.rc.as_str());
    }
    if created_candidate {
        targets.push(paths.candidate.as_str());
    }
    targets
}

struct Ssh2Bootstrap<'session> {
    session: &'session ssh2::Session,
    cols: u16,
    rows: u16,
    channel: Option<ssh2::Channel>,
}

impl<'session> Ssh2Bootstrap<'session> {
    fn new(session: &'session ssh2::Session, cols: u16, rows: u16) -> Self {
        Self {
            session,
            cols,
            rows,
            channel: None,
        }
    }

    fn open_pty_channel(&self) -> Result<ssh2::Channel, String> {
        let mut channel = self
            .session
            .channel_session()
            .map_err(|error| format!("打开 channel 失败: {error}"))?;
        channel
            .request_pty(
                "xterm-256color",
                None,
                Some((self.cols as u32, self.rows as u32, 0, 0)),
            )
            .map_err(|error| format!("PTY 请求失败: {error}"))?;
        Ok(channel)
    }
}

impl ShellBootstrap for Ssh2Bootstrap<'_> {
    fn probe_login_shell(&mut self) -> Result<String, String> {
        with_temporary_ssh_timeout(self.session, 3_000, || {
            let mut channel = self
                .session
                .channel_session()
                .map_err(|error| format!("打开 shell 探测 channel 失败: {error}"))?;
            let probe = channel
                .exec("printf '%s' \"$SHELL\"")
                .map_err(|error| format!("执行 shell 探测失败: {error}"))
                .and_then(|_| read_probe_shell(&mut channel));
            let _ = channel.close();
            let _ = channel.wait_close();
            probe
        })
    }

    fn deploy_bash_runtime(
        &mut self,
        session: &CompletionSessionKey,
        bash_path: &str,
    ) -> Result<RemoteBashRuntime, String> {
        let paths = RemoteBashPaths::new(session);
        let sftp = self
            .session
            .sftp()
            .map_err(|error| format!("打开 SFTP 失败: {error}"))?;
        let flags = ssh2::OpenFlags::WRITE | ssh2::OpenFlags::CREATE | ssh2::OpenFlags::EXCLUSIVE;
        let mut created_rc = false;
        let mut created_candidate = false;
        let result = (|| {
            let sequence = widget_sequence(session);
            let rc_contents =
                build_bash_rc(session, std::path::Path::new(&paths.candidate), &sequence);
            let mut rc_file = sftp
                .open_mode(&paths.rc, flags, 0o600, ssh2::OpenType::File)
                .map_err(|error| format!("创建远端 Bash RC 失败: {error}"))?;
            created_rc = true;
            rc_file
                .write_all(rc_contents.as_bytes())
                .map_err(|error| format!("写入远端 Bash RC 失败: {error}"))?;
            rc_file
                .fsync()
                .map_err(|error| format!("同步远端 Bash RC 失败: {error}"))?;
            rc_file
                .close()
                .map_err(|error| format!("关闭远端 Bash RC 失败: {error}"))?;

            let mut candidate_file = sftp
                .open_mode(&paths.candidate, flags, 0o600, ssh2::OpenType::File)
                .map_err(|error| format!("创建远端候选文件失败: {error}"))?;
            created_candidate = true;
            candidate_file
                .fsync()
                .map_err(|error| format!("同步远端候选文件失败: {error}"))?;
            candidate_file
                .close()
                .map_err(|error| format!("关闭远端候选文件失败: {error}"))?;

            Ok(RemoteBashRuntime {
                session: session.clone(),
                bash_path: bash_path.to_owned(),
                rc_path: paths.rc.clone(),
                candidate_path: paths.candidate.clone(),
                widget_sequence: sequence,
            })
        })();

        if result.is_err() {
            for path in cleanup_targets(&paths, created_rc, created_candidate) {
                let _ = sftp.unlink(std::path::Path::new(path));
            }
        }
        result
    }

    fn open_integrated_bash(&mut self, runtime: &RemoteBashRuntime) -> Result<(), String> {
        let mut channel = self.open_pty_channel()?;
        let paths = RemoteBashPaths {
            rc: runtime.rc_path.clone(),
            candidate: runtime.candidate_path.clone(),
        };
        channel
            .exec(&paths.launch_command(&runtime.bash_path))
            .map_err(|error| format!("启动远端 Bash 集成失败: {error}"))?;
        self.channel = Some(channel);
        Ok(())
    }

    fn cleanup_bash_runtime(&mut self, runtime: &RemoteBashRuntime) {
        if let Ok(sftp) = self.session.sftp() {
            let _ = sftp.unlink(std::path::Path::new(&runtime.rc_path));
            let _ = sftp.unlink(std::path::Path::new(&runtime.candidate_path));
        }
    }

    fn open_plain_shell(&mut self) -> Result<(), String> {
        let mut channel = self.open_pty_channel()?;
        channel
            .shell()
            .map_err(|error| format!("Shell 请求失败: {error}"))?;
        self.channel = Some(channel);
        Ok(())
    }
}

pub(crate) fn connect_authenticated(params: &ConnectionParams) -> Result<ssh2::Session, String> {
    log::info!(
        "SSH connecting to {}:{} user={} auth={}",
        params.host,
        params.port,
        params.user,
        params.auth
    );
    let addresses = resolve_ssh_addresses(&params.host, params.port)?;
    let tcp = connect_resolved(&addresses, SSH_IO_TIMEOUT, |address, remaining| {
        TcpStream::connect_timeout(&address, remaining)
    })?;
    configure_tcp_timeouts(&tcp, SSH_IO_TIMEOUT)?;

    log::info!("SSH TCP connected to {}:{}", params.host, params.port);
    let mut session =
        ssh2::Session::new().map_err(|error| format!("SSH session 创建失败: {error}"))?;
    session.set_tcp_stream(tcp);
    session.set_timeout(SSH_IO_TIMEOUT_MS);
    session
        .handshake()
        .map_err(|error| format!("SSH 握手失败: {error}"))?;
    log::info!("SSH handshake ok");

    let key_path = (!params.key_path.is_empty()).then_some(params.key_path.as_str());
    let password = (!params.password.is_empty()).then_some(params.password.as_str());
    let mut authenticated = false;

    if params.auth == "key" || params.auth == "keyring" || params.auth.is_empty() {
        if let Some(path) = key_path {
            let expanded = shellexpand::tilde(path);
            let expanded = std::path::Path::new(expanded.as_ref());
            if expanded.exists() {
                authenticated = session
                    .userauth_pubkey_file(&params.user, None, expanded, password)
                    .is_ok();
            }
        }
    }

    if !authenticated {
        authenticated = session.userauth_agent(&params.user).is_ok();
    }

    if !authenticated {
        for path in ["~/.ssh/id_rsa", "~/.ssh/id_ed25519", "~/.ssh/id_ecdsa"] {
            let expanded = shellexpand::tilde(path);
            let expanded = std::path::Path::new(expanded.as_ref());
            if expanded.exists()
                && session
                    .userauth_pubkey_file(&params.user, None, expanded, password)
                    .is_ok()
            {
                authenticated = true;
                break;
            }
        }
    }

    if !authenticated {
        if let Some(password) = password {
            authenticated = session.userauth_password(&params.user, password).is_ok();
        }
    }

    if !authenticated || !session.authenticated() {
        return Err(format!(
            "SSH 认证失败 (auth={}, user={})",
            params.auth, params.user
        ));
    }

    session.set_keepalive(true, 30);
    log::info!("SSH auth ok");
    Ok(session)
}

/// SSH connection that runs entirely on a single thread (ssh2::Session is !Send).
/// The reader stays on the SSH thread; writing is done via mpsc channel.
pub struct SshHandle {
    pub reader: Box<dyn Read + Send>,
    pub write_tx: mpsc::Sender<Vec<u8>>,
    pub resize_tx: mpsc::Sender<(u16, u16)>,
    pub shutdown_tx: mpsc::Sender<()>,
    pub io_done_rx: mpsc::Receiver<()>,
    pub bash_runtime: Option<RemoteBashRuntime>,
}

impl SshHandle {
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

/// Connect to an SSH server and return a handle for reading/writing.
/// Everything SSH happens on the calling thread.
/// Returns (reader_for_terminal, write_sender).
pub fn connect(
    params: &ConnectionParams,
    cols: u16,
    rows: u16,
    integration: Option<CompletionSessionKey>,
) -> Result<SshHandle, String> {
    let session = connect_authenticated(params)?;
    let (mut channel, bash_runtime) = {
        let mut bootstrap = Ssh2Bootstrap::new(&session, cols, rows);
        let bash_runtime = match integration {
            Some(completion_session) => bootstrap_shell(&mut bootstrap, completion_session)?,
            None => {
                bootstrap.open_plain_shell()?;
                None
            }
        };
        let channel = bootstrap
            .channel
            .take()
            .ok_or_else(|| "SSH shell channel 未创建".to_string())?;
        (channel, bash_runtime)
    };

    log::info!("SSH shell opened, {}x{}", cols, rows);

    // 5. Set up writer thread via mpsc
    // ssh2::Channel is !Send, so we can't move it across threads.
    // Instead, we use a pipe: the writer thread writes to a pipe,
    // and the SSH thread reads from the pipe and forwards to the channel.
    let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>();
    let (resize_tx, resize_rx) = mpsc::channel::<(u16, u16)>();
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    let (io_done_tx, io_done_rx) = mpsc::channel::<()>();

    // We'll use OS pipes for the reader side too — the SSH thread reads
    // from the channel and writes to a pipe, and the terminal thread
    // reads from the other end.
    let (pipe_read, mut pipe_write) =
        os_pipe::pipe().map_err(|e| format!("创建管道失败: {}", e))?;

    // SSH I/O thread: reads channel → pipe, reads mpsc → channel
    std::thread::spawn(move || {
        session.set_blocking(false);
        let mut buf = [0u8; 4096];
        loop {
            if !matches!(shutdown_rx.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                session.set_blocking(true);
                let _ = channel.close();
                break;
            }

            // Read from channel (non-blocking)
            match channel.read(&mut buf) {
                Ok(0) => {
                    if channel.eof() {
                        log::info!("SSH channel EOF");
                        break;
                    }
                }
                Ok(n) => {
                    if pipe_write.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    let _ = pipe_write.flush();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    log::error!("SSH channel read error: {}", e);
                    break;
                }
            }

            // Write pending input to channel (non-blocking)
            while let Ok(data) = write_rx.try_recv() {
                session.set_blocking(true);
                let _ = channel.write_all(&data);
                let _ = channel.flush();
                session.set_blocking(false);
            }

            while let Ok((new_cols, new_rows)) = resize_rx.try_recv() {
                session.set_blocking(true);
                let _ = channel.request_pty_size(new_cols as u32, new_rows as u32, None, None);
                session.set_blocking(false);
            }

            // Small sleep to avoid busy loop (non-blocking mode)
            std::thread::sleep(Duration::from_millis(5));
        }
        session.set_blocking(true);
        let _ = channel.close();
        drop(pipe_write);
        drop(channel);
        drop(session);
        let _ = io_done_tx.send(());
    });

    Ok(SshHandle {
        reader: Box::new(pipe_read),
        write_tx,
        resize_tx,
        shutdown_tx,
        io_done_rx,
        bash_runtime,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        bootstrap_shell, cleanup_targets, configure_tcp_timeouts, connect_resolved,
        connect_resolved_with_clock, read_probe_shell, resolve_ssh_addresses,
        with_temporary_ssh_timeout, ConnectionParams, SessionTimeoutControl, ShellBootstrap,
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
    fn connection_params_preserve_password_and_key_passphrase() {
        let source = SshConnection {
            label: "生产机".into(),
            host: "server.example.com".into(),
            port: 2222,
            user: "deploy".into(),
            auth: "password".into(),
            key_path: "~/.ssh/id_ed25519".into(),
            password: "passphrase-or-password".into(),
            group: "prod".into(),
            group_color: [1, 2, 3],
        };

        let params = ConnectionParams::from(&source);

        assert_eq!(params.key_path, "~/.ssh/id_ed25519");
        assert_eq!(params.password, "passphrase-or-password");
        assert_eq!(params.auth, "password");
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
    fn bootstrap_failures_fall_back_and_cleanup_partial_runtime() {
        for (failure, expected) in [
            (FakeFailure::Probe, vec!["probe", "open_plain"]),
            (FakeFailure::Deploy, vec!["probe", "deploy", "open_plain"]),
            (
                FakeFailure::IntegratedOpen,
                vec![
                    "probe",
                    "deploy",
                    "open_integrated",
                    "cleanup",
                    "open_plain",
                ],
            ),
        ] {
            let mut bootstrap = FakeBootstrap::failing(failure);

            assert!(bootstrap_shell(&mut bootstrap, test_session())
                .unwrap()
                .is_none());
            assert_eq!(bootstrap.calls, expected);
        }
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
}
