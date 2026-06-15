use std::io::{Read, Write};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::state::{AppState, ManagedSession, SftpRequest};

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
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let timeout = state.settings.lock().unwrap().ssh.connect_timeout_secs;

    let id_clone = id.clone();
    let app_clone = app.clone();
    let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (resize_tx, resize_rx) = std::sync::mpsc::channel::<(u32, u32)>();
    let (status_tx, status_rx) = std::sync::mpsc::channel::<Result<(), String>>();

    std::thread::spawn(move || {
        // 1. TCP connect + SSH handshake
        let addr = format!("{}:{}", host, port);
        let sock_addr = match addr.parse::<std::net::SocketAddr>() {
            Ok(a) => a,
            Err(e) => {
                let _ = status_tx.send(Err(format!("Invalid address: {}", e)));
                return;
            }
        };
        let tcp = match std::net::TcpStream::connect_timeout(
            &sock_addr,
            std::time::Duration::from_secs(timeout as u64),
        ) {
            Ok(tcp) => tcp,
            Err(e) => {
                let _ = status_tx.send(Err(format!("TCP connect failed: {}", e)));
                return;
            }
        };

        let mut session = match ssh2::Session::new() {
            Ok(s) => s,
            Err(e) => {
                let _ = status_tx.send(Err(format!("SSH session failed: {}", e)));
                return;
            }
        };
        session.set_tcp_stream(tcp);
        if let Err(e) = session.handshake() {
            let _ = status_tx.send(Err(format!("SSH handshake failed: {}", e)));
            return;
        }

        // 2. Authenticate
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
            let _ = status_tx.send(Err(e));
            return;
        }

        // 3. Open shell channel with PTY
        let mut channel = match session.channel_session() {
            Ok(ch) => ch,
            Err(e) => {
                let _ = status_tx.send(Err(e.to_string()));
                return;
            }
        };
        if let Err(e) = channel.request_pty("xterm-256color", None, Some((80, 24, 0, 0))) {
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
                    let _ = app_clone.emit(
                        "terminal-output",
                        serde_json::json!({
                            "id": id_for_read,
                            "data": &buf[..n],
                        }),
                    );
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

            // Write user input
            while let Ok(data) = input_rx.try_recv() {
                let _ = channel.write_all(&data);
                let _ = channel.flush();
            }

            // Handle resize
            while let Ok((cols, rows)) = resize_rx.try_recv() {
                let _ = channel.request_pty_size(cols, rows, None, None);
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
                },
            );
            Ok(id)
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Connection thread died".to_string()),
    }
}
