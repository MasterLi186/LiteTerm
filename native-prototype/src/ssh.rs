use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::bash_integration::{
    build_bash_rc, is_safe_remote_bash_path, snapshot_sequence, widget_sequence, RemoteBashPaths,
    RemoteBashRuntime,
};
use crate::smart_completion::CompletionSessionKey;

const SSH_IO_TIMEOUT: Duration = Duration::from_secs(10);
const SSH_IO_TIMEOUT_MS: u32 = 10_000;
const SSH_PENDING_WRITE_CAPACITY: usize = 32;

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
    fn from(conn: &crate::sidebar::SshConnection) -> Self {
        Self {
            host: conn.host.clone(),
            port: conn.port,
            user: conn.user.clone(),
            auth: conn.auth.clone(),
            key_path: conn.key_path.clone(),
            password: conn.password.clone(),
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
            let fill_sequence = widget_sequence(session);
            let snapshot_sequence = snapshot_sequence(session);
            let rc_contents = build_bash_rc(
                session,
                std::path::Path::new(&paths.candidate),
                &fill_sequence,
                &snapshot_sequence,
            );
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
                widget_sequence: fill_sequence,
                snapshot_sequence,
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
    let mut session = ssh2::Session::new().map_err(|e| format!("SSH session 创建失败: {e}"))?;
    session.set_tcp_stream(tcp);
    session.set_timeout(SSH_IO_TIMEOUT_MS);
    session
        .handshake()
        .map_err(|e| format!("SSH 握手失败: {e}"))?;
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
    pub write_tx: crate::zmodem::runtime::TransportWriter,
    pub resize_tx: mpsc::SyncSender<(u16, u16)>,
    pub shutdown_tx: mpsc::Sender<()>,
    pub io_done_rx: mpsc::Receiver<()>,
    pub bash_runtime: Option<RemoteBashRuntime>,
}

impl SshHandle {
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }

    #[cfg(test)]
    fn shutdown_and_wait(&self, timeout: Duration) -> Result<(), String> {
        let _ = self.shutdown_tx.send(());
        self.io_done_rx
            .recv_timeout(timeout)
            .map_err(|error| format!("等待 SSH I/O 线程退出失败: {error}"))
    }
}

fn shutdown_requested<T>(result: &Result<T, mpsc::TryRecvError>) -> bool {
    !matches!(result, Err(mpsc::TryRecvError::Empty))
}

struct SshOutputReader {
    receiver: mpsc::Receiver<Vec<u8>>,
    current: Vec<u8>,
    offset: usize,
}

impl SshOutputReader {
    fn new(receiver: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            receiver,
            current: Vec::new(),
            offset: 0,
        }
    }
}

impl Read for SshOutputReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        while self.offset == self.current.len() {
            self.current = match self.receiver.recv() {
                Ok(bytes) => bytes,
                Err(_) => return Ok(0),
            };
            self.offset = 0;
        }
        let take = buffer.len().min(self.current.len() - self.offset);
        buffer[..take].copy_from_slice(&self.current[self.offset..self.offset + take]);
        self.offset += take;
        Ok(take)
    }
}

struct PendingChannelWrite {
    bytes: Vec<u8>,
    offset: usize,
    protocol: Option<crate::zmodem::runtime::ProtocolWriteRequest>,
    normal_epoch: Option<u64>,
    started: bool,
}

impl PendingChannelWrite {
    fn from_transport(message: crate::zmodem::runtime::TransportWrite) -> Self {
        match message {
            crate::zmodem::runtime::TransportWrite::Normal { bytes, epoch } => Self {
                bytes,
                offset: 0,
                protocol: None,
                normal_epoch: Some(epoch),
                started: true,
            },
            crate::zmodem::runtime::TransportWrite::Protocol(protocol) => Self {
                bytes: protocol.bytes().to_vec(),
                offset: 0,
                protocol: Some(protocol),
                normal_epoch: None,
                started: false,
            },
        }
    }

    fn begin(&mut self) -> bool {
        if self.started {
            return true;
        }
        self.started = self
            .protocol
            .as_ref()
            .is_some_and(|request| request.begin());
        self.started
    }

    fn may_continue(&self) -> bool {
        self.protocol
            .as_ref()
            .is_none_or(|request| request.may_continue())
    }

    fn complete(self, result: std::io::Result<()>) {
        if let Some(protocol) = self.protocol {
            protocol.complete(result);
        }
    }
}

fn write_channel_once(
    writer: &mut impl Write,
    pending: &mut PendingChannelWrite,
) -> std::io::Result<bool> {
    if pending.offset < pending.bytes.len() {
        match writer.write(&pending.bytes[pending.offset..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "SSH channel write returned zero",
                ));
            }
            Ok(written) => {
                pending.offset += written;
                if pending.offset < pending.bytes.len() {
                    return Ok(false);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    match writer.flush() {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
        Err(error) => Err(error),
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

    // ssh2::Channel is !Send, so all I/O remains on one non-blocking worker.
    let protocol_active = std::sync::Arc::new(crate::zmodem::runtime::ProtocolGate::new());
    let (write_tx, write_rx) =
        crate::zmodem::runtime::transport_write_channel(std::sync::Arc::clone(&protocol_active));
    let (resize_tx, resize_rx) = mpsc::sync_channel::<(u16, u16)>(8);
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    let (io_done_tx, io_done_rx) = mpsc::channel::<()>();

    let (output_tx, output_rx) = mpsc::sync_channel::<Vec<u8>>(16);

    // SSH I/O thread: reads channel → pipe, reads mpsc → channel
    std::thread::spawn(move || {
        session.set_blocking(false);
        let mut buf = [0u8; 4096];
        let mut output_pending: Option<Vec<u8>> = None;
        let mut writes = VecDeque::<PendingChannelWrite>::new();
        let mut failure: Option<String> = None;
        loop {
            let shutdown_request = shutdown_rx.try_recv();
            if shutdown_requested(&shutdown_request) {
                let _ = channel.close();
                break;
            }

            if let Some(bytes) = output_pending.take() {
                match output_tx.try_send(bytes) {
                    Ok(()) => {}
                    Err(mpsc::TrySendError::Full(bytes)) => output_pending = Some(bytes),
                    Err(mpsc::TrySendError::Disconnected(_)) => break,
                }
            }
            if output_pending.is_none() {
                match channel.read(&mut buf) {
                    Ok(0) => {
                        if channel.eof() {
                            log::info!("SSH channel EOF");
                            break;
                        }
                    }
                    Ok(n) => match output_tx.try_send(buf[..n].to_vec()) {
                        Ok(()) => {}
                        Err(mpsc::TrySendError::Full(bytes)) => output_pending = Some(bytes),
                        Err(mpsc::TrySendError::Disconnected(_)) => break,
                    },
                    Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => {
                        failure = Some(format!("SSH channel read error: {error}"));
                        break;
                    }
                }
            }

            for _ in 0..4.min(SSH_PENDING_WRITE_CAPACITY.saturating_sub(writes.len())) {
                match write_rx.try_recv() {
                    Ok(message) => {
                        if matches!(
                            message,
                            crate::zmodem::runtime::TransportWrite::Normal { .. }
                        ) && protocol_active.is_active()
                        {
                            continue;
                        }
                        writes.push_back(PendingChannelWrite::from_transport(message));
                    }
                    Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
                }
            }

            if let Some(pending) = writes.front_mut() {
                if pending.protocol.is_none()
                    && (protocol_active.is_active()
                        || pending.normal_epoch != Some(protocol_active.epoch()))
                {
                    writes.pop_front();
                    continue;
                }
                if !pending.begin() || !pending.may_continue() {
                    let expired = writes.pop_front().unwrap();
                    expired.complete(Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "协议写请求在 SSH 写入前已取消或超时",
                    )));
                    continue;
                }
                match write_channel_once(&mut channel, pending) {
                    Ok(true) => {
                        let completed = writes.pop_front().unwrap();
                        completed.complete(Ok(()));
                    }
                    Ok(false) => {}
                    Err(error) => {
                        let failed = writes.pop_front().unwrap();
                        failed.complete(Err(std::io::Error::new(error.kind(), error.to_string())));
                        failure = Some(format!("SSH channel write error: {error}"));
                        break;
                    }
                }
            }

            if let Ok((new_cols, new_rows)) = resize_rx.try_recv() {
                if let Err(error) =
                    channel.request_pty_size(new_cols as u32, new_rows as u32, None, None)
                {
                    log::warn!("SSH resize failed: {error}");
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let reason = failure.unwrap_or_else(|| "SSH I/O 已关闭".into());
        for pending in writes {
            pending.complete(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                reason.clone(),
            )));
        }
        while let Ok(message) = write_rx.try_recv() {
            let pending = PendingChannelWrite::from_transport(message);
            pending.complete(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                reason.clone(),
            )));
        }
        if reason != "SSH I/O 已关闭" {
            log::error!("{reason}");
        }
        let _ = channel.close();
        drop(output_tx);
        drop(channel);
        drop(session);
        let _ = io_done_tx.send(());
    });

    Ok(SshHandle {
        reader: Box::new(SshOutputReader::new(output_rx)),
        write_tx,
        resize_tx,
        shutdown_tx,
        io_done_rx,
        bash_runtime,
    })
}

#[cfg(test)]
#[path = "ssh/tests.rs"]
mod tests;
