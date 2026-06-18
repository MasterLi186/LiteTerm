use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::app_log;
use crate::state::AppState;

#[derive(Serialize, Clone)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
    pub permissions: String,
    pub owner: String,
    pub group: String,
}

fn mode_to_string(mode: u32, is_dir: bool) -> String {
    let mut s = String::with_capacity(10);
    s.push(if is_dir { 'd' } else { '-' });
    s.push(if mode & 0o400 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o200 != 0 { 'w' } else { '-' });
    s.push(if mode & 0o100 != 0 { 'x' } else { '-' });
    s.push(if mode & 0o040 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o020 != 0 { 'w' } else { '-' });
    s.push(if mode & 0o010 != 0 { 'x' } else { '-' });
    s.push(if mode & 0o004 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o002 != 0 { 'w' } else { '-' });
    s.push(if mode & 0o001 != 0 { 'x' } else { '-' });
    s
}

/// List files in a local directory.
#[tauri::command]
pub async fn list_local_dir(path: String) -> Result<Vec<FileEntry>, String> {
    let expanded = shellexpand::tilde(&path).to_string();
    let mut entries = Vec::new();

    let read_dir = std::fs::read_dir(&expanded).map_err(|e| format!("无法读取目录: {}", e))?;

    for entry in read_dir {
        let entry = entry.map_err(|e| e.to_string())?;
        let meta = entry.metadata().map_err(|e| e.to_string())?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        #[cfg(unix)]
        let (permissions, owner, group) = {
            let mode = meta.mode();
            (mode_to_string(mode, meta.is_dir()), meta.uid().to_string(), meta.gid().to_string())
        };
        #[cfg(not(unix))]
        let (permissions, owner, group) = {
            let perm = if meta.permissions().readonly() { "r--r--r--" } else { "rw-rw-rw-" };
            let p = format!("{}{}", if meta.is_dir() { "d" } else { "-" }, perm);
            (p, String::new(), String::new())
        };

        entries.push(FileEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            is_dir: meta.is_dir(),
            size: meta.len(),
            mtime,
            permissions,
            owner,
            group,
        });
    }

    // Sort: directories first, then alphabetical
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(entries)
}

/// Start an SFTP session over a new SSH connection and store it in app state.
#[tauri::command]
pub async fn start_sftp_session(
    state: State<'_, AppState>,
    session_id: String,
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    auth_method: String,
    key_path: Option<String>,
) -> Result<(), String> {
    app_log!("SFTP", "SFTP SESSION START: {}:{} user={} auth={} session_id={}", host, port, user, auth_method, session_id);

    // Open a separate SSH connection for SFTP
    let addr = format!("{}:{}", host, port);
    let sock_addr: std::net::SocketAddr = addr.parse().map_err(|e: std::net::AddrParseError| {
        app_log!("SFTP", "ERROR: 无效地址: {}", e);
        format!("无效地址: {}", e)
    })?;

    let tcp = std::net::TcpStream::connect_timeout(
        &sock_addr,
        std::time::Duration::from_secs(10),
    )
    .map_err(|e| {
        app_log!("SFTP", "ERROR: SFTP连接失败: {} ({}:{})", e, host, port);
        format!("SFTP连接失败: {}", e)
    })?;

    tcp.set_nodelay(true).ok();
    tcp.set_write_timeout(None).ok();
    tcp.set_read_timeout(None).ok();

    let mut session = ssh2::Session::new().map_err(|e| {
        app_log!("SFTP", "ERROR: SSH会话创建失败: {}", e);
        format!("SSH会话创建失败: {}", e)
    })?;
    session.set_tcp_stream(tcp);
    session.handshake().map_err(|e| {
        app_log!("SFTP", "ERROR: SSH握手失败: {}", e);
        format!("SSH握手失败: {}", e)
    })?;
    session.set_keepalive(true, 30);
    session.set_timeout(0);

    // Authenticate
    match auth_method.as_str() {
        "agent" => {
            session.userauth_agent(&user).map_err(|e| {
                app_log!("SFTP", "ERROR: Agent认证失败: {}", e);
                format!("认证失败: {}", e)
            })?;
        }
        "key" => {
            let key = key_path.unwrap_or_default();
            let expanded = shellexpand::tilde(&key);
            session
                .userauth_pubkey_file(
                    &user,
                    None,
                    Path::new(expanded.as_ref()),
                    password.as_deref(),
                )
                .map_err(|e| {
                    app_log!("SFTP", "ERROR: 密钥认证失败: {}", e);
                    format!("密钥认证失败: {}", e)
                })?;
        }
        _ => {
            let pw = password.unwrap_or_default();
            session
                .userauth_password(&user, &pw)
                .map_err(|e| {
                    app_log!("SFTP", "ERROR: 密码认证失败: {}", e);
                    format!("密码认证失败: {}", e)
                })?;
        }
    }
    app_log!("SFTP", "SFTP认证成功: {}:{}", host, port);

    let sftp = session.sftp().map_err(|e| {
        app_log!("SFTP", "ERROR: SFTP会话启动失败: {}", e);
        format!("SFTP会话启动失败: {}", e)
    })?;

    app_log!("SFTP", "SFTP会话已建立: session_id={}", session_id);

    // Store the SFTP session (session must live as long as sftp)
    state
        .sftp_sessions
        .lock()
        .unwrap()
        .insert(session_id, SftpHandle { _session: session, sftp });

    Ok(())
}

/// A handle holding both the SSH session (to keep it alive) and the SFTP channel.
pub struct SftpHandle {
    _session: ssh2::Session,
    sftp: ssh2::Sftp,
}

// Safety: ssh2::Session and ssh2::Sftp use raw pointers internally but are
// effectively single-threaded. We guard access via Mutex<HashMap<..>>,
// ensuring only one thread touches a given SftpHandle at a time.
unsafe impl Send for SftpHandle {}
unsafe impl Sync for SftpHandle {}

/// Execute a command on the SFTP session's SSH connection and return output.
#[tauri::command]
pub async fn sftp_exec(
    state: State<'_, AppState>,
    session_id: String,
    command: String,
) -> Result<String, String> {
    let sftp_sessions = state.sftp_sessions.lock().unwrap();
    let handle = sftp_sessions
        .get(&session_id)
        .ok_or_else(|| "SFTP会话未找到".to_string())?;

    let mut channel = handle._session
        .channel_session()
        .map_err(|e| format!("打开通道失败: {}", e))?;

    channel.exec(&command)
        .map_err(|e| format!("执行命令失败: {}", e))?;

    let mut output = String::new();
    channel.read_to_string(&mut output)
        .map_err(|e| format!("读取输出失败: {}", e))?;

    channel.wait_close().ok();
    Ok(output.trim().to_string())
}

/// Flip the cancel flag for an in-progress transfer (keyed `<direction>-<filename>`).
/// Used by the progress panel's cancel button (e.g. ZMODEM upload).
#[tauri::command]
pub async fn cancel_transfer(state: State<'_, AppState>, transfer_key: String) -> Result<(), String> {
    if let Some(flag) = state.transfer_cancel.lock().unwrap().get(&transfer_key) {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
        app_log!("SFTP", "cancel_transfer: {}", transfer_key);
    }
    Ok(())
}

/// List files in a remote directory via SFTP.
#[tauri::command]
pub async fn sftp_list_dir(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> Result<Vec<FileEntry>, String> {
    let sftp_sessions = state.sftp_sessions.lock().unwrap();
    let handle = sftp_sessions
        .get(&session_id)
        .ok_or_else(|| "SFTP会话未找到".to_string())?;

    let dir = handle
        .sftp
        .readdir(Path::new(&path))
        .map_err(|e| format!("无法读取远程目录: {}", e))?;

    let mut entries: Vec<FileEntry> = dir
        .into_iter()
        .filter_map(|(pathbuf, stat)| {
            let name = pathbuf.file_name()?.to_string_lossy().into_owned();
            let perm = stat.perm.unwrap_or(0);
            let is_dir = stat.is_dir();
            Some(FileEntry {
                name,
                is_dir,
                size: stat.size.unwrap_or(0),
                mtime: stat.mtime.unwrap_or(0),
                permissions: mode_to_string(perm, is_dir),
                owner: stat.uid.map(|u| u.to_string()).unwrap_or_default(),
                group: stat.gid.map(|g| g.to_string()).unwrap_or_default(),
            })
        })
        .collect();

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(entries)
}

/// Download a remote file to a local path via SFTP, emitting progress events.
#[tauri::command]
pub async fn sftp_download(
    state: State<'_, AppState>,
    app: AppHandle,
    session_id: String,
    remote_path: String,
    local_path: String,
) -> Result<(), String> {
    app_log!("SFTP", "DOWNLOAD START: session={}, remote={}, local={}", session_id, remote_path, local_path);

    let sftp_sessions = state.sftp_sessions.lock().unwrap();
    let handle = match sftp_sessions.get(&session_id) {
        Some(h) => h,
        None => {
            let msg = format!("SFTP会话未找到, session_id={}", session_id);
            app_log!("SFTP", "ERROR: {}", msg);
            return Err(msg);
        }
    };

    app_log!("SFTP", "SFTP session found, stat remote file...");
    let stat = handle
        .sftp
        .stat(Path::new(&remote_path))
        .map_err(|e| {
            let msg = format!("无法获取远程文件信息: {} (path={})", e, remote_path);
            app_log!("SFTP", "ERROR: {}", msg);
            msg
        })?;
    let total = stat.size.unwrap_or(0);
    app_log!("SFTP", "Remote file size: {} bytes", total);

    app_log!("SFTP", "Opening remote file...");
    let mut remote_file = handle
        .sftp
        .open(Path::new(&remote_path))
        .map_err(|e| {
            let msg = format!("无法打开远程文件: {} (path={})", e, remote_path);
            app_log!("SFTP", "ERROR: {}", msg);
            msg
        })?;

    app_log!("SFTP", "Creating local file: {}", local_path);
    let expanded_local = shellexpand::tilde(&local_path).to_string();
    if let Some(parent) = Path::new(&expanded_local).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let mut local_file = std::fs::File::create(&expanded_local)
        .map_err(|e| {
            let msg = format!("无法创建本地文件: {} (path={})", e, expanded_local);
            app_log!("SFTP", "ERROR: {}", msg);
            msg
        })?;

    let filename = Path::new(&remote_path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    app_log!("SFTP", "Starting transfer: {}", filename);
    let mut buf = [0u8; 32768];
    let mut bytes_so_far: u64 = 0;
    loop {
        let n = remote_file
            .read(&mut buf)
            .map_err(|e| {
                let msg = format!("读取远程文件失败: {} (bytes_so_far={})", e, bytes_so_far);
                app_log!("SFTP", "ERROR: {}", msg);
                msg
            })?;
        if n == 0 {
            break;
        }
        local_file
            .write_all(&buf[..n])
            .map_err(|e| {
                let msg = format!("写入本地文件失败: {} (bytes_so_far={})", e, bytes_so_far);
                app_log!("SFTP", "ERROR: {}", msg);
                msg
            })?;
        bytes_so_far += n as u64;
        let _ = app.emit(
            "transfer-progress",
            serde_json::json!({
                "filename": filename,
                "bytes_transferred": bytes_so_far,
                "total_bytes": total,
                "direction": "download"
            }),
        );
    }

    app_log!("SFTP", "DOWNLOAD COMPLETE: {} bytes transferred", bytes_so_far);
    Ok(())
}

/// Upload a local file to a remote path via SFTP, emitting progress events.
#[tauri::command]
pub async fn sftp_upload(
    state: State<'_, AppState>,
    app: AppHandle,
    session_id: String,
    local_path: String,
    remote_path: String,
) -> Result<(), String> {
    app_log!("SFTP", "UPLOAD START: session={}, local={}, remote={}", session_id, local_path, remote_path);

    let expanded_local = shellexpand::tilde(&local_path).to_string();
    let meta =
        std::fs::metadata(&expanded_local).map_err(|e| {
            let msg = format!("无法读取本地文件信息: {} (path={})", e, expanded_local);
            app_log!("SFTP", "ERROR: {}", msg);
            msg
        })?;
    let total = meta.len();
    app_log!("SFTP", "Local file size: {} bytes", total);

    let sftp_sessions = state.sftp_sessions.lock().unwrap();
    let handle = sftp_sessions
        .get(&session_id)
        .ok_or_else(|| {
            let msg = format!("SFTP会话未找到, session_id={}", session_id);
            app_log!("SFTP", "ERROR: {}", msg);
            msg
        })?;

    app_log!("SFTP", "Creating remote file: {}", remote_path);
    let mut remote_file = handle
        .sftp
        .create(Path::new(&remote_path))
        .map_err(|e| {
            let msg = format!("无法创建远程文件: {} (path={})", e, remote_path);
            app_log!("SFTP", "ERROR: {}", msg);
            msg
        })?;

    let mut local_file =
        std::fs::File::open(&expanded_local).map_err(|e| {
            let msg = format!("无法打开本地文件: {} (path={})", e, expanded_local);
            app_log!("SFTP", "ERROR: {}", msg);
            msg
        })?;

    let filename = Path::new(&expanded_local)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    app_log!("SFTP", "Starting transfer: {}", filename);
    let mut buf = [0u8; 32768];
    let mut bytes_so_far: u64 = 0;
    let mut last_log_mb: u64 = 0;
    loop {
        let n = local_file
            .read(&mut buf)
            .map_err(|e| {
                let msg = format!("读取本地文件失败: {} (bytes_so_far={})", e, bytes_so_far);
                app_log!("SFTP", "ERROR: {}", msg);
                msg
            })?;
        if n == 0 {
            break;
        }
        remote_file
            .write_all(&buf[..n])
            .map_err(|e| {
                let msg = format!("写入远程文件失败: {} (bytes_so_far={})", e, bytes_so_far);
                app_log!("SFTP", "ERROR: {}", msg);
                msg
            })?;
        bytes_so_far += n as u64;
        // 每 10MB 记录一次进度
        let current_mb = bytes_so_far / (10 * 1024 * 1024);
        if current_mb > last_log_mb {
            last_log_mb = current_mb;
            app_log!("SFTP", "PROGRESS: {}MB / {}MB", bytes_so_far / (1024 * 1024), total / (1024 * 1024));
        }
        let _ = app.emit(
            "transfer-progress",
            serde_json::json!({
                "filename": filename,
                "bytes_transferred": bytes_so_far,
                "total_bytes": total,
                "direction": "upload"
            }),
        );
    }

    app_log!("SFTP", "UPLOAD COMPLETE: {} bytes transferred", bytes_so_far);
    Ok(())
}

/// Delete a remote file or directory via SFTP.
#[tauri::command]
pub async fn sftp_delete(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
    is_dir: bool,
) -> Result<(), String> {
    let sftp_sessions = state.sftp_sessions.lock().unwrap();
    let handle = sftp_sessions
        .get(&session_id)
        .ok_or_else(|| "SFTP会话未找到".to_string())?;

    if is_dir {
        handle
            .sftp
            .rmdir(Path::new(&path))
            .map_err(|e| format!("删除远程目录失败: {}", e))?;
    } else {
        handle
            .sftp
            .unlink(Path::new(&path))
            .map_err(|e| format!("删除远程文件失败: {}", e))?;
    }

    Ok(())
}

/// Rename (move) a remote file or directory via SFTP.
#[tauri::command]
pub async fn sftp_rename(
    state: State<'_, AppState>,
    session_id: String,
    old_path: String,
    new_path: String,
) -> Result<(), String> {
    let sftp_sessions = state.sftp_sessions.lock().unwrap();
    let handle = sftp_sessions
        .get(&session_id)
        .ok_or_else(|| "SFTP会话未找到".to_string())?;

    handle
        .sftp
        .rename(Path::new(&old_path), Path::new(&new_path), None)
        .map_err(|e| format!("重命名远程文件失败: {}", e))?;

    Ok(())
}

/// Save binary data to a local file (used by frontend ZMODEM receive).
#[tauri::command]
pub async fn save_file(path: String, data: Vec<u8>) -> Result<(), String> {
    let expanded = shellexpand::tilde(&path).to_string();
    if let Some(parent) = Path::new(&expanded).parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&expanded, &data).map_err(|e| format!("保存文件失败: {}", e))?;
    Ok(())
}

/// Delete a local file or directory.
#[tauri::command]
pub async fn local_delete(path: String) -> Result<(), String> {
    let expanded = shellexpand::tilde(&path).to_string();
    let p = Path::new(&expanded);

    if p.is_dir() {
        std::fs::remove_dir_all(p).map_err(|e| format!("删除本地目录失败: {}", e))?;
    } else {
        std::fs::remove_file(p).map_err(|e| format!("删除本地文件失败: {}", e))?;
    }

    Ok(())
}

/// Read a local file and return its contents as bytes.
#[tauri::command]
pub async fn read_local_file(path: String) -> Result<Vec<u8>, String> {
    let expanded = shellexpand::tilde(&path).to_string();
    std::fs::read(&expanded).map_err(|e| format!("读取文件失败: {}", e))
}

/// Rename a local file or directory.
#[tauri::command]
pub async fn local_rename(old_path: String, new_path: String) -> Result<(), String> {
    let old_expanded = shellexpand::tilde(&old_path).to_string();
    let new_expanded = shellexpand::tilde(&new_path).to_string();
    std::fs::rename(&old_expanded, &new_expanded)
        .map_err(|e| format!("重命名失败: {}", e))?;
    Ok(())
}
