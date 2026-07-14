use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::time::Duration;

/// SSH connection that runs entirely on a single thread (ssh2::Session is !Send).
/// The reader stays on the SSH thread; writing is done via mpsc channel.
pub struct SshHandle {
    pub reader: Box<dyn Read + Send>,
    pub write_tx: mpsc::Sender<Vec<u8>>,
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
    match auth {
        "key" => {
            let kp = key_path.unwrap_or("~/.ssh/id_rsa");
            let expanded = shellexpand::tilde(kp);
            session.userauth_pubkey_file(
                user, None,
                std::path::Path::new(expanded.as_ref()),
                None,
            ).map_err(|e| format!("密钥认证失败: {}", e))?;
        }
        "agent" => {
            session.userauth_agent(user)
                .map_err(|e| format!("Agent 认证失败: {}", e))?;
        }
        _ => {
            return Err("暂不支持密码认证（原型阶段）".to_string());
        }
    }

    if !session.authenticated() {
        return Err("SSH 认证失败".to_string());
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

    // We'll use OS pipes for the reader side too — the SSH thread reads
    // from the channel and writes to a pipe, and the terminal thread
    // reads from the other end.
    let (mut pipe_read, mut pipe_write) = os_pipe::pipe()
        .map_err(|e| format!("创建管道失败: {}", e))?;

    // SSH I/O thread: reads channel → pipe, reads mpsc → channel
    std::thread::spawn(move || {
        session.set_blocking(false);
        let mut buf = [0u8; 4096];
        loop {
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

            // Small sleep to avoid busy loop (non-blocking mode)
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(pipe_write);
    });

    Ok(SshHandle {
        reader: Box::new(pipe_read),
        write_tx,
    })
}
