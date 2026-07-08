use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::app_log;
use crate::state::AppState;

#[derive(Serialize, Clone)]
pub struct TunnelInfo {
    pub id: String,
    pub tunnel_type: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub status: String,
}

pub struct TunnelHandle {
    pub info: TunnelInfo,
    pub stop: Arc<AtomicBool>,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_tunnel(
    state: State<'_, AppState>,
    app: AppHandle,
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    auth_method: String,
    key_path: Option<String>,
    tunnel_type: String,
    local_port: u16,
    remote_host: String,
    remote_port: u16,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let timeout = state.settings.lock().unwrap().ssh.connect_timeout_secs;
    let stop = Arc::new(AtomicBool::new(false));

    let id_clone = id.clone();
    let stop_clone = stop.clone();
    let app_clone = app.clone();
    let remote_host_clone = remote_host.clone();
    let tunnel_type_clone = tunnel_type.clone();
    let (status_tx, status_rx) = std::sync::mpsc::channel::<Result<(), String>>();

    std::thread::spawn(move || {
        app_log!("TUNNEL", "TUNNEL START: {}:{} -> {}:{} type={} local_port={}", host, port, remote_host_clone, remote_port, tunnel_type_clone, local_port);

        // 1. TCP connect + SSH handshake
        let addr = format!("{}:{}", host, port);
        let sock_addr = match crate::core::net::resolve_addr(&addr) {
            Ok(a) => a,
            Err(e) => {
                app_log!("TUNNEL", "ERROR: {}", e);
                let _ = status_tx.send(Err(e));
                return;
            }
        };
        let tcp = match std::net::TcpStream::connect_timeout(
            &sock_addr,
            std::time::Duration::from_secs(timeout as u64),
        ) {
            Ok(tcp) => tcp,
            Err(e) => {
                app_log!("TUNNEL", "ERROR: TCP connect failed: {} ({})", e, addr);
                let _ = status_tx.send(Err(format!("TCP connect failed: {}", e)));
                return;
            }
        };

        let mut session = match ssh2::Session::new() {
            Ok(s) => s,
            Err(e) => {
                app_log!("TUNNEL", "ERROR: SSH session failed: {}", e);
                let _ = status_tx.send(Err(format!("SSH session failed: {}", e)));
                return;
            }
        };
        session.set_tcp_stream(tcp);
        if let Err(e) = session.handshake() {
            app_log!("TUNNEL", "ERROR: SSH handshake failed: {}", e);
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
            app_log!("TUNNEL", "ERROR: 认证失败: {}", e);
            let _ = status_tx.send(Err(e));
            return;
        }

        session.set_keepalive(true, 30);

        // 3. Bind local listener
        let listener = match std::net::TcpListener::bind(format!("0.0.0.0:{}", local_port)) {
            Ok(l) => l,
            Err(e) => {
                app_log!("TUNNEL", "ERROR: Bind port {} failed: {}", local_port, e);
                let _ = status_tx.send(Err(format!("Bind port {} failed: {}", local_port, e)));
                return;
            }
        };
        // SO_REUSEADDR is set by default on TcpListener::bind on Linux,
        // but set nonblocking so we can check the stop flag.
        let _ = listener.set_nonblocking(true);

        app_log!("TUNNEL", "Tunnel listening on 0.0.0.0:{}", local_port);

        // Signal success
        let _ = status_tx.send(Ok(()));

        // 4. Accept loop
        let id_for_event = id_clone.clone();
        loop {
            if stop_clone.load(Ordering::Relaxed) {
                break;
            }

            match listener.accept() {
                Ok((local_stream, peer_addr)) => {
                    app_log!("TUNNEL", "Accepted connection from {}", peer_addr);
                    // ssh2::Session is !Send, so channel_direct_tcpip must happen
                    // on this thread. We open the channel here, then hand both
                    // the local stream and the channel to a new thread for copying.
                    let channel = match session.channel_direct_tcpip(
                        &remote_host_clone,
                        remote_port,
                        None,
                    ) {
                        Ok(ch) => ch,
                        Err(e) => {
                            app_log!("TUNNEL", "ERROR: channel_direct_tcpip failed: {}", e);
                            continue;
                        }
                    };

                    let stop_for_copy = stop_clone.clone();
                    let _ = local_stream.set_nonblocking(true);

                    // Bidirectional copy thread
                    std::thread::spawn(move || {
                        let mut local = local_stream;
                        let mut chan = channel;
                        let mut buf = [0u8; 8192];

                        loop {
                            if stop_for_copy.load(Ordering::Relaxed) {
                                break;
                            }

                            // local -> channel
                            match local.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => {
                                    if chan.write_all(&buf[..n]).is_err() {
                                        app_log!("TUNNEL", "ERROR: write to channel failed");
                                        break;
                                    }
                                    let _ = chan.flush();
                                }
                                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                                Err(e) => {
                                    app_log!("TUNNEL", "ERROR: read from local failed: {}", e);
                                    break;
                                }
                            }

                            // channel -> local
                            match chan.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => {
                                    if local.write_all(&buf[..n]).is_err() {
                                        app_log!("TUNNEL", "ERROR: write to local failed");
                                        break;
                                    }
                                    let _ = local.flush();
                                }
                                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                                Err(e) => {
                                    app_log!("TUNNEL", "ERROR: read from channel failed: {}", e);
                                    break;
                                }
                            }

                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => {
                    app_log!("TUNNEL", "ERROR: accept failed: {}", e);
                    break;
                }
            }
        }

        app_log!("TUNNEL", "Tunnel stopped: id={}", id_for_event);

        // Tunnel stopped — notify frontend
        let _ = app_clone.emit(
            "tunnel-closed",
            serde_json::json!({
                "id": id_for_event,
                "tunnel_type": tunnel_type_clone,
            }),
        );
    });

    // Wait for connection result
    match status_rx.recv() {
        Ok(Ok(())) => {
            let info = TunnelInfo {
                id: id.clone(),
                tunnel_type,
                local_port,
                remote_host,
                remote_port,
                status: "active".to_string(),
            };
            state.tunnels.lock().unwrap().insert(
                id.clone(),
                TunnelHandle {
                    info,
                    stop,
                },
            );
            Ok(id)
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Tunnel thread died".to_string()),
    }
}

#[tauri::command]
pub async fn list_tunnels(state: State<'_, AppState>) -> Result<Vec<TunnelInfo>, String> {
    let tunnels = state.tunnels.lock().unwrap();
    let list: Vec<TunnelInfo> = tunnels.values().map(|h| h.info.clone()).collect();
    Ok(list)
}

#[tauri::command]
pub async fn close_tunnel(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut tunnels = state.tunnels.lock().unwrap();
    match tunnels.remove(&id) {
        Some(handle) => {
            handle.stop.store(true, Ordering::Relaxed);
            Ok(())
        }
        None => Err(format!("Tunnel {} not found", id)),
    }
}
