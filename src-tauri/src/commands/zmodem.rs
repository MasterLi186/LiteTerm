use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, State};

use crate::app_log;
use crate::core::zmodem::decode::ZmodemDecoder;
use crate::core::zmodem::sender::{FileInfo, SenderAction, ZmodemSender};
use crate::state::AppState;

#[tauri::command]
pub async fn zmodem_send(
    state: State<'_, AppState>,
    app: AppHandle,
    session_id: String,
    files: Vec<String>,
) -> Result<(), String> {
    app_log!("ZMODEM", "SEND START: session={}, files={}", session_id, files.len());

    // Collect file info
    let mut file_infos = Vec::new();
    for path_str in &files {
        let expanded = shellexpand::tilde(path_str).to_string();
        let path = PathBuf::from(&expanded);
        let meta = std::fs::metadata(&path)
            .map_err(|e| format!("无法读取文件: {} - {}", path_str, e))?;
        let name = path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path_str.clone());
        let mtime = meta.modified().ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        file_infos.push(FileInfo {
            path,
            name,
            size: meta.len(),
            mtime,
        });
    }

    // Get session resources
    let (input_tx, zmodem_active, zmodem_tx_holder) = {
        let sessions = state.sessions.lock().unwrap();
        let session = sessions.get(&session_id)
            .ok_or_else(|| "会话未找到".to_string())?;
        // Guard against concurrent invocations on the same session
        if session.zmodem_active.load(Ordering::Acquire) {
            return Err("ZMODEM 传输已在进行中".to_string());
        }
        (
            session.input_tx.clone(),
            session.zmodem_active.clone(),
            session.zmodem_tx.clone(),
        )
    };

    // Create channel for receiving terminal output from the reader thread
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();

    // Activate ZMODEM mode — store tx first (Release on Mutex unlock), then set flag (Release)
    *zmodem_tx_holder.lock().unwrap() = Some(tx);
    zmodem_active.store(true, Ordering::Release);

    // Send "rz\r" to start rz on the remote
    let _ = input_tx.send(b"rz\r".to_vec());

    // Run the protocol on a blocking thread
    let app_clone = app.clone();
    let session_id_clone = session_id.clone();
    let join_result = tokio::task::spawn_blocking(move || {
        run_zmodem_protocol(file_infos, input_tx, rx, app_clone, &session_id_clone)
    }).await;

    // Always deactivate ZMODEM mode regardless of outcome (including panics)
    zmodem_active.store(false, Ordering::Release);
    *zmodem_tx_holder.lock().unwrap() = None;

    let result = join_result.map_err(|e| format!("ZMODEM 线程异常: {}", e))?;

    app_log!("ZMODEM", "SEND END: session={}, result={:?}", session_id, result.is_ok());

    // Send a newline to refresh the prompt
    {
        let sessions = state.sessions.lock().unwrap();
        if let Some(session) = sessions.get(&session_id) {
            let _ = session.input_tx.send(b"\r".to_vec());
        }
    }

    result
}

fn run_zmodem_protocol(
    files: Vec<FileInfo>,
    input_tx: std::sync::mpsc::Sender<Vec<u8>>,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    app: AppHandle,
    session_id: &str,
) -> Result<(), String> {
    let _ = session_id; // used for logging context
    let mut sender = ZmodemSender::new(files);
    let mut decoder = ZmodemDecoder::new();

    // Send ZRQINIT to initiate
    match sender.start() {
        SenderAction::Send(data) => { let _ = input_tx.send(data); }
        _ => {}
    }

    let mut last_progress = Instant::now();
    let timeout = Duration::from_secs(30);
    let mut last_activity = Instant::now();

    loop {
        if sender.is_done() {
            break;
        }

        // Check for timeout
        if last_activity.elapsed() > timeout {
            app_log!("ZMODEM", "TIMEOUT: 30 秒无响应");
            let _ = input_tx.send(crate::core::zmodem::encode::zcancel());
            return Err("ZMODEM 超时".into());
        }

        // Try to receive data from the reader thread (non-blocking with short timeout)
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(data) => {
                last_activity = Instant::now();

                // Check for cancel (5+ CAN bytes)
                if ZmodemDecoder::detect_cancel(&data) {
                    app_log!("ZMODEM", "远端取消");
                    return Err("远端取消传输".into());
                }

                // Parse frames
                let frames = decoder.feed(&data);
                for frame in frames {
                    app_log!("ZMODEM", "收到帧: {:?} offset={}", frame.frame_type, frame.offset());
                    match sender.handle_frame(&frame) {
                        SenderAction::Send(out) => { let _ = input_tx.send(out); }
                        SenderAction::Error(e) => {
                            app_log!("ZMODEM", "ERROR: {}", e);
                            let _ = input_tx.send(crate::core::zmodem::encode::zcancel());
                            return Err(e);
                        }
                        SenderAction::FileComplete(name) => {
                            app_log!("ZMODEM", "文件完成: {}", name);
                        }
                        SenderAction::AllComplete => {
                            app_log!("ZMODEM", "所有文件传输完成");
                        }
                        SenderAction::Progress { .. } | SenderAction::None => {}
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("终端连接断开".into());
            }
        }

        // Pump data while in SendData state
        while let Some(action) = sender.next_data_chunk() {
            match action {
                SenderAction::Send(data) => {
                    let _ = input_tx.send(data);
                    last_activity = Instant::now();
                }
                SenderAction::Error(e) => {
                    let _ = input_tx.send(crate::core::zmodem::encode::zcancel());
                    return Err(e);
                }
                _ => break,
            }

            // Emit progress every 200ms
            if last_progress.elapsed() > Duration::from_millis(200) {
                if let Some(SenderAction::Progress { bytes_sent, total, filename }) = sender.progress() {
                    let _ = app.emit("transfer-progress", serde_json::json!({
                        "filename": filename,
                        "bytes_transferred": bytes_sent,
                        "total_bytes": total,
                        "direction": "zmodem-upload"
                    }));
                }
                last_progress = Instant::now();
            }

            // Check for incoming frames during data sending (non-blocking)
            if let Ok(data) = rx.try_recv() {
                last_activity = Instant::now();
                if ZmodemDecoder::detect_cancel(&data) {
                    return Err("远端取消传输".into());
                }
                let frames = decoder.feed(&data);
                for frame in frames {
                    app_log!("ZMODEM", "数据发送中收到帧: {:?}", frame.frame_type);
                    match sender.handle_frame(&frame) {
                        SenderAction::Send(out) => { let _ = input_tx.send(out); }
                        SenderAction::Error(e) => {
                            let _ = input_tx.send(crate::core::zmodem::encode::zcancel());
                            return Err(e);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Final progress
    if let Some(SenderAction::Progress { bytes_sent, total, filename }) = sender.progress() {
        let _ = app.emit("transfer-progress", serde_json::json!({
            "filename": filename,
            "bytes_transferred": bytes_sent,
            "total_bytes": total,
            "direction": "zmodem-upload"
        }));
    }

    Ok(())
}
