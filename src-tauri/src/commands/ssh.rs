use std::io::{Read, Write};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use portable_pty::{CommandBuilder, PtySize};
use tauri::{AppHandle, Emitter, State};

use crate::app_log;
use crate::state::{AppState, LocalTerminal, ManagedSession, SftpRequest};

#[tauri::command]
pub async fn ssh_connect(
    state: State<'_, AppState>,
    app: AppHandle,
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    auth_method: String,
    key_path: Option<String>,
    label: String,
    proxy_jump: Option<String>,
    cols: Option<u32>,
    rows: Option<u32>,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let timeout = state.settings.lock().unwrap().ssh.connect_timeout_secs;

    // ProxyJump path: use system SSH client via PTY
    if let Some(ref proxy) = proxy_jump {
        if !proxy.is_empty() {
            let pty_system = portable_pty::native_pty_system();
            let pair = pty_system
                .openpty(PtySize {
                    rows: rows.unwrap_or(36) as u16,
                    cols: cols.unwrap_or(120) as u16,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| format!("PTY open failed: {}", e))?;

            let mut cmd = CommandBuilder::new("ssh");
            cmd.arg("-J");
            cmd.arg(proxy);
            cmd.arg("-p");
            cmd.arg(&port.to_string());
            cmd.arg("-o");
            cmd.arg("StrictHostKeyChecking=no");
            if let Some(ref kp) = key_path {
                if !kp.is_empty() {
                    let expanded = shellexpand::tilde(kp);
                    cmd.arg("-i");
                    cmd.arg(expanded.as_ref());
                }
            }
            cmd.arg(&format!("{}@{}", user, host));

            let _child = pair
                .slave
                .spawn_command(cmd)
                .map_err(|e| format!("SSH spawn failed: {}", e))?;
            drop(pair.slave);

            let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
            let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

            let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();

            let id_clone = id.clone();
            let app_clone = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let mut reader = reader;
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => {
                            let _ = app_clone.emit(
                                "terminal-closed",
                                serde_json::json!({"id": id_clone}),
                            );
                            break;
                        }
                        Ok(n) => {
                            let _ = app_clone.emit(
                                "terminal-output",
                                serde_json::json!({"id": id_clone, "data": &buf[..n]}),
                            );
                        }
                        Err(_) => {
                            let _ = app_clone.emit(
                                "terminal-closed",
                                serde_json::json!({"id": id_clone}),
                            );
                            break;
                        }
                    }
                }
            });

            std::thread::spawn(move || {
                let mut writer = writer;
                while let Ok(data) = input_rx.recv() {
                    let _ = writer.write_all(&data);
                    let _ = writer.flush();
                }
            });

            let (resize_tx, resize_rx) = std::sync::mpsc::channel::<(u32, u32)>();
            let master = pair.master;
            std::thread::spawn(move || {
                while let Ok((cols, rows)) = resize_rx.recv() {
                    let _ = master.resize(PtySize {
                        rows: rows as u16,
                        cols: cols as u16,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
            });

            state.local_terminals.lock().unwrap().insert(
                id.clone(),
                LocalTerminal {
                    id: id.clone(),
                    input_tx,
                    resize_tx,
                    stop: Arc::new(AtomicBool::new(false)),
                },
            );

            return Ok(id);
        }
    }

    let id_clone = id.clone();
    let app_clone = app.clone();
    let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (resize_tx, resize_rx) = std::sync::mpsc::channel::<(u32, u32)>();
    let (status_tx, status_rx) = std::sync::mpsc::channel::<Result<(), String>>();

    let zmodem_active = Arc::new(AtomicBool::new(false));
    let zmodem_tx: Arc<Mutex<Option<std::sync::mpsc::Sender<Vec<u8>>>>> = Arc::new(Mutex::new(None));
    let zmodem_active_clone = zmodem_active.clone();
    let zmodem_tx_clone = zmodem_tx.clone();

    std::thread::spawn(move || {
        app_log!("SSH", "SSH CONNECT START: {}:{} user={} auth={}", host, port, user, auth_method);

        // 1. TCP connect + SSH handshake
        let addr = format!("{}:{}", host, port);
        let sock_addr = match addr.parse::<std::net::SocketAddr>() {
            Ok(a) => a,
            Err(e) => {
                app_log!("SSH", "ERROR: Invalid address: {}", e);
                let _ = status_tx.send(Err(format!("Invalid address: {}", e)));
                return;
            }
        };
        let tcp = match std::net::TcpStream::connect_timeout(
            &sock_addr,
            std::time::Duration::from_secs(timeout as u64),
        ) {
            Ok(tcp) => { app_log!("SSH", "TCP connected to {}", addr); tcp },
            Err(e) => {
                app_log!("SSH", "ERROR: TCP connect failed: {}", e);
                let _ = status_tx.send(Err(format!("TCP connect failed: {}", e)));
                return;
            }
        };

        let mut session = match ssh2::Session::new() {
            Ok(s) => s,
            Err(e) => {
                app_log!("SSH", "ERROR: SSH session create failed: {}", e);
                let _ = status_tx.send(Err(format!("SSH session failed: {}", e)));
                return;
            }
        };

        // 记录客户端支持的加密算法（握手前）
        let alg_info = |mt: ssh2::MethodType, label: &str| -> String {
            match session.supported_algs(mt) {
                Ok(algs) => format!("{}: {}", label, algs.join(", ")),
                Err(_) => format!("{}: (无法获取)", label),
            }
        };
        let supported = format!(
            "客户端支持的算法:\n  {}\n  {}\n  {}\n  {}",
            alg_info(ssh2::MethodType::Kex, "密钥交换"),
            alg_info(ssh2::MethodType::HostKey, "主机密钥"),
            alg_info(ssh2::MethodType::CryptCs, "加密(C→S)"),
            alg_info(ssh2::MethodType::CryptSc, "加密(S→C)"),
        );
        app_log!("SSH", "{}", supported);

        session.set_tcp_stream(tcp);
        app_log!("SSH", "开始 SSH 握手...");
        if let Err(e) = session.handshake() {
            app_log!("SSH", "ERROR: SSH handshake failed: {}", e);
            app_log!("SSH", "{}", supported);
            let _ = status_tx.send(Err(format!("SSH handshake failed: {}", e)));
            return;
        }
        app_log!("SSH", "SSH 握手成功");

        // 记录协商后实际使用的算法
        let active_algs = |mt: ssh2::MethodType, label: &str| -> String {
            match session.methods(mt) {
                Some(m) => format!("{}: {}", label, m),
                None => format!("{}: (未知)", label),
            }
        };
        app_log!("SSH", "协商结果: {} | {} | {}",
            active_algs(ssh2::MethodType::Kex, "Kex"),
            active_algs(ssh2::MethodType::CryptCs, "Cipher"),
            active_algs(ssh2::MethodType::MacCs, "MAC"),
        );

        // 2. Authenticate
        app_log!("SSH", "开始认证: method={}", auth_method);
        let auth_result = match auth_method.as_str() {
            "agent" => session
                .userauth_agent(&user)
                .map_err(|e| format!("Agent auth failed: {}", e)),
            "key" => {
                let key = key_path.unwrap_or_default();
                let expanded = shellexpand::tilde(&key);
                session
                    .userauth_pubkey_file(
                        &user,
                        None,
                        std::path::Path::new(expanded.as_ref()),
                        password.as_deref(),
                    )
                    .map_err(|e| format!("Key auth failed: {}", e))
            }
            _ => {
                let pw = password.unwrap_or_default();
                session
                    .userauth_password(&user, &pw)
                    .map_err(|e| format!("Password auth failed: {}", e))
            }
        };

        if let Err(e) = auth_result {
            app_log!("SSH", "ERROR: 认证失败: {}", e);
            let _ = status_tx.send(Err(e));
            return;
        }
        app_log!("SSH", "认证成功");

        // 3. Open shell channel with PTY
        let mut channel = match session.channel_session() {
            Ok(ch) => ch,
            Err(e) => {
                let _ = status_tx.send(Err(e.to_string()));
                return;
            }
        };
        let pty_cols = cols.unwrap_or(120);
        let pty_rows = rows.unwrap_or(36);
        if let Err(e) = channel.request_pty("xterm-256color", None, Some((pty_cols, pty_rows, 0, 0))) {
            let _ = status_tx.send(Err(format!("PTY request failed: {}", e)));
            return;
        }
        if let Err(e) = channel.shell() {
            let _ = status_tx.send(Err(format!("Shell request failed: {}", e)));
            return;
        }
        session.set_keepalive(true, 30);

        // Signal success
        let _ = status_tx.send(Ok(()));

        // 4. Read loop: SSH channel -> terminal-output event (ZMODEM handled on frontend)
        // Delay to let frontend register event listener
        std::thread::sleep(std::time::Duration::from_millis(500));
        session.set_blocking(false);
        let id_for_read = id_clone.clone();
        let mut last_keepalive = std::time::Instant::now();
        loop {
            let mut buf = [0u8; 4096];
            match channel.read(&mut buf) {
                Ok(0) => {
                    let _ = app_clone.emit(
                        "terminal-closed",
                        serde_json::json!({"id": id_for_read}),
                    );
                    break;
                }
                Ok(n) => {
                    if zmodem_active_clone.load(std::sync::atomic::Ordering::Acquire) {
                        let guard = zmodem_tx_clone.lock().unwrap();
                        if let Some(ref tx) = *guard {
                            let _ = tx.send(buf[..n].to_vec());
                        }
                    } else {
                        let _ = app_clone.emit(
                            "terminal-output",
                            serde_json::json!({
                                "id": id_for_read,
                                "data": &buf[..n],
                            }),
                        );
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => {
                    let _ = app_clone.emit(
                        "terminal-closed",
                        serde_json::json!({"id": id_for_read}),
                    );
                    break;
                }
            }

            // Write user input — non-blocking, drain reads on WouldBlock to avoid deadlock
            while let Ok(data) = input_rx.try_recv() {
                let mut offset = 0;
                let mut retries = 0;
                while offset < data.len() {
                    match channel.write(&data[offset..]) {
                        Ok(n) => { offset += n; retries = 0; }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            // Drain pending read data to free SSH window
                            let mut drain_buf = [0u8; 8192];
                            if let Ok(n) = channel.read(&mut drain_buf) {
                                if n > 0 {
                                    if zmodem_active_clone.load(std::sync::atomic::Ordering::Acquire) {
                                        if let Some(ref tx) = *zmodem_tx_clone.lock().unwrap() {
                                            let _ = tx.send(drain_buf[..n].to_vec());
                                        }
                                    } else {
                                        let _ = app_clone.emit("terminal-output", serde_json::json!({"id": id_for_read, "data": &drain_buf[..n]}));
                                    }
                                }
                            }
                            retries += 1;
                            if retries > 5000 { break; }
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }
                        Err(_) => break,
                    }
                }
                let _ = channel.flush();
            }

            // Handle resize
            while let Ok((cols, rows)) = resize_rx.try_recv() {
                let _ = channel.request_pty_size(cols, rows, None, None);
            }

            // 每 15 秒发送 SSH keepalive 防止服务端超时断连
            if last_keepalive.elapsed() >= std::time::Duration::from_secs(15) {
                let _ = session.keepalive_send();
                last_keepalive = std::time::Instant::now();
            }

            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });

    // Wait for connection result
    match status_rx.recv() {
        Ok(Ok(())) => {
            let monitor_stop = Arc::new(AtomicBool::new(false));
            let (sftp_tx, _sftp_rx) = std::sync::mpsc::channel::<SftpRequest>();
            state.sessions.lock().unwrap().insert(
                id.clone(),
                ManagedSession {
                    id: id.clone(),
                    label,
                    input_tx,
                    resize_tx,
                    monitor_stop,
                    sftp_request_tx: sftp_tx,
                    zmodem_active,
                    zmodem_tx,
                },
            );
            Ok(id)
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Connection thread died".to_string()),
    }
}

/// 返回客户端支持的 SSH 加密算法列表，用于连接失败时诊断
#[tauri::command]
pub async fn ssh_supported_algs() -> Result<String, String> {
    let session = ssh2::Session::new().map_err(|e| format!("{}", e))?;
    let mut info = String::new();
    let types = [
        (ssh2::MethodType::Kex, "密钥交换(Kex)"),
        (ssh2::MethodType::HostKey, "主机密钥(HostKey)"),
        (ssh2::MethodType::CryptCs, "加密(Client→Server)"),
        (ssh2::MethodType::CryptSc, "加密(Server→Client)"),
        (ssh2::MethodType::MacCs, "MAC(Client→Server)"),
        (ssh2::MethodType::MacSc, "MAC(Server→Client)"),
    ];
    for (mt, label) in types {
        match session.supported_algs(mt) {
            Ok(algs) => info.push_str(&format!("{}: {}\n", label, algs.join(", "))),
            Err(_) => info.push_str(&format!("{}: (无法获取)\n", label)),
        }
    }
    Ok(info)
}
