use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::bash_integration::RemoteBashRuntime;

const SSH_IO_TIMEOUT: Duration = Duration::from_secs(10);
const SSH_IO_TIMEOUT_MS: u32 = 10_000;

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
    host: &str,
    port: u16,
    user: &str,
    auth: &str,
    key_path: Option<&str>,
    cols: u16,
    rows: u16,
) -> Result<SshHandle, String> {
    log::info!("SSH connecting to {}:{} user={} auth={}", host, port, user, auth);

    // 1. TCP connect
    let addr = format!("{}:{}", host, port);
    let tcp = TcpStream::connect_timeout(
        &addr.parse().map_err(|e| format!("地址解析失败: {}", e))?,
        Duration::from_secs(10),
    ).map_err(|e| format!("TCP 连接失败: {}", e))?;

    log::info!("SSH TCP connected to {}", addr);

    // 2. SSH session + handshake
    let mut session = ssh2::Session::new()
        .map_err(|e| format!("SSH session 创建失败: {}", e))?;
    session.set_tcp_stream(tcp);
    session.handshake()
        .map_err(|e| format!("SSH 握手失败: {}", e))?;

    log::info!("SSH handshake ok");

    // 3. Authenticate
    // 尝试顺序：显式指定 > agent > 默认密钥
    let auth_ok = match auth {
        "key" => {
            let kp = key_path.unwrap_or("~/.ssh/id_rsa");
            let expanded = shellexpand::tilde(kp);
            session.userauth_pubkey_file(
                user, None,
                std::path::Path::new(expanded.as_ref()),
                None,
            ).is_ok()
        }
        "agent" => {
            session.userauth_agent(user).is_ok()
        }
        _ => {
            // keyring 或其他：依次尝试 agent → key → 带密码的 key
            let mut ok = session.userauth_agent(user).is_ok();
            if !ok {
                // 尝试默认密钥
                let default_keys = ["~/.ssh/id_rsa", "~/.ssh/id_ed25519", "~/.ssh/id_ecdsa"];
                for kp in &default_keys {
                    let expanded = shellexpand::tilde(kp);
                    let path = std::path::Path::new(expanded.as_ref());
                    if path.exists() {
                        if session.userauth_pubkey_file(user, None, path, None).is_ok() {
                            ok = true;
                            break;
                        }
                    }
                }
            }
            ok
        }
    };

    if !auth_ok || !session.authenticated() {
        return Err(format!("SSH 认证失败 (auth={}, user={})", auth, user));
    }

    log::info!("SSH auth ok");

    // 4. Open channel + PTY + shell
    let mut channel = session.channel_session()
        .map_err(|e| format!("打开 channel 失败: {}", e))?;
    channel.request_pty(
        "xterm-256color",
        None,
        Some((cols as u32, rows as u32, 0, 0)),
    ).map_err(|e| format!("PTY 请求失败: {}", e))?;
    channel.shell()
        .map_err(|e| format!("Shell 请求失败: {}", e))?;

    session.set_keepalive(true, 30);
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
        bash_runtime: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        connect_resolved, connect_resolved_with_clock, resolve_ssh_addresses, ConnectionParams,
    };

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
        let elapsed =
            std::rc::Rc::new(std::cell::Cell::new(std::time::Duration::from_secs(0)));
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
