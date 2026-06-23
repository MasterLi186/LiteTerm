use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, State};

use crate::app_log;
use crate::core::zmodem::decode::ZmodemDecoder;
use crate::core::zmodem::encode::zcancel;
use crate::core::zmodem::sender::{FileInfo, SenderAction, ZmodemSender};
use crate::core::zmodem::DecodedFrame;
use crate::state::{AppState, ZmodemSendRequest};

/// ZMODEM(rz) 上传：把请求交给该会话的 reader 线程（独占 SSH channel）执行，阻塞等待
/// 结果。用于 SFTP 不可用（如交互式堡垒机跳到目标机——独立 SFTP 连接到不了目标机）时
/// 的拖拽上传回退：rz 在终端会话内运行，能穿透堡垒机菜单把文件传到目标机。
/// 进度由 reader 线程内的 run_zmodem_send 用自己的 AppHandle 上报，故此处无需 app。
pub async fn run_zmodem_upload(
    state: &State<'_, AppState>,
    session_id: String,
    files: Vec<String>,
) -> Result<(), String> {
    app_log!("ZMODEM", "SEND START: session={}, files={}", session_id, files.len());

    // 收集文件信息
    let mut file_infos = Vec::new();
    for path_str in &files {
        let expanded = shellexpand::tilde(path_str).to_string();
        let path = PathBuf::from(&expanded);
        let meta = std::fs::metadata(&path)
            .map_err(|e| format!("无法读取文件: {} - {}", path_str, e))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path_str.clone());
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        file_infos.push(FileInfo { path, name, size: meta.len(), mtime });
    }

    // 获取会话资源
    let (request_slot, zmodem_active) = {
        let sessions = state.sessions.lock().unwrap();
        let session = sessions
            .get(&session_id)
            .ok_or_else(|| "会话未找到（仅直连 SSH 终端支持拖拽上传）".to_string())?;
        if session.zmodem_active.load(Ordering::Acquire) {
            return Err("ZMODEM 传输已在进行中".to_string());
        }
        (session.zmodem_request.clone(), session.zmodem_active.clone())
    };

    // 取消标志 —— 以前端进度面板使用的同一个 key（`zmodem-upload-<文件名>`）
    // 注册进 transfer_cancel，这样 cancel_transfer 命令能直接翻转它。
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut cancels = state.transfer_cancel.lock().unwrap();
        for f in &file_infos {
            cancels.insert(format!("zmodem-upload-{}", f.name), cancel.clone());
        }
    }
    let cancel_keys: Vec<String> = file_infos
        .iter()
        .map(|f| format!("zmodem-upload-{}", f.name))
        .collect();

    // 提交请求并通知 reader 线程
    let (result_tx, result_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    *request_slot.lock().unwrap() = Some(ZmodemSendRequest {
        files: file_infos,
        result_tx,
        cancel,
    });
    zmodem_active.store(true, Ordering::Release);

    // 在阻塞线程上等待 reader 线程返回结果
    let result = tokio::task::spawn_blocking(move || {
        result_rx
            .recv()
            .unwrap_or_else(|_| Err("ZMODEM 处理线程无响应".into()))
    })
    .await
    .map_err(|e| format!("ZMODEM 线程异常: {}", e))?;

    // 清理取消标志
    {
        let mut cancels = state.transfer_cancel.lock().unwrap();
        for k in &cancel_keys {
            cancels.remove(k);
        }
    }

    app_log!("ZMODEM", "SEND END: session={}, ok={}", session_id, result.is_ok());
    result
}

/// 把整个缓冲区写入 SSH channel；遇到 WouldBlock 时穿插读取，避免单线程
/// 读写循环死锁。读到的字节喂给同一个解码器（不存在跨线程乱序），解析出
/// 的帧入队待处理。
fn zm_write(
    channel: &mut ssh2::Channel,
    decoder: &mut ZmodemDecoder,
    pending: &mut Vec<DecodedFrame>,
    data: &[u8],
) -> Result<(), String> {
    use std::io::{Read, Write};
    let mut off = 0usize;
    let mut last = Instant::now();
    while off < data.len() {
        match channel.write(&data[off..]) {
            Ok(0) => {
                if last.elapsed() > Duration::from_secs(60) {
                    return Err("写入停滞超时".into());
                }
            }
            Ok(n) => {
                off += n;
                last = Instant::now();
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // 读取远端数据腾出 SSH 窗口，并喂给同一个解码器
                let mut rb = [0u8; 8192];
                match channel.read(&mut rb) {
                    Ok(n) if n > 0 => {
                        last = Instant::now();
                        if ZmodemDecoder::detect_cancel(&rb[..n]) {
                            return Err("远端取消传输".into());
                        }
                        for f in decoder.feed(&rb[..n]) {
                            pending.push(f);
                        }
                    }
                    Ok(_) => {}
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(e) => return Err(format!("读取错误: {}", e)),
                }
                if last.elapsed() > Duration::from_secs(60) {
                    return Err("写入停滞超时".into());
                }
            }
            Err(e) => return Err(format!("写入错误: {}", e)),
        }
    }
    let _ = channel.flush();
    Ok(())
}

/// 在 SSH reader 线程上运行完整的 ZMODEM 发送协议，独占 channel 的读和写。
/// 单线程、单解码器、按网络速度推进。
pub fn run_zmodem_send(
    channel: &mut ssh2::Channel,
    session: &ssh2::Session,
    files: Vec<FileInfo>,
    app: &AppHandle,
    cancel: &AtomicBool,
    keyboard_rx: &std::sync::mpsc::Receiver<Vec<u8>>,
) -> Result<(), String> {
    use std::io::Read;

    session.set_blocking(false);

    let mut sender = ZmodemSender::new(files);
    let mut decoder = ZmodemDecoder::new();
    let mut pending: Vec<DecodedFrame> = Vec::new();

    app_log!("ZMODEM", "run_zmodem_send 启动");

    // 在远端启动 rz，再发送 ZRQINIT 宣告发送方就绪
    zm_write(channel, &mut decoder, &mut pending, b"rz\r")?;
    if let SenderAction::Send(d) = sender.start() {
        zm_write(channel, &mut decoder, &mut pending, &d)?;
    }

    let mut last_activity = Instant::now();
    let mut last_progress = Instant::now();
    let mut last_keepalive = Instant::now();

    let emit_progress = |sender: &ZmodemSender, app: &AppHandle| {
        if let Some(SenderAction::Progress { bytes_sent, total, filename }) = sender.progress() {
            let _ = app.emit(
                "transfer-progress",
                serde_json::json!({
                    "filename": filename,
                    "bytes_transferred": bytes_sent,
                    "total_bytes": total,
                    "direction": "zmodem-upload"
                }),
            );
        }
    };

    loop {
        if cancel.load(Ordering::Relaxed) {
            app_log!("ZMODEM", "用户取消");
            let _ = zm_write(channel, &mut decoder, &mut pending, &zcancel());
            return Err("传输已取消".into());
        }
        if sender.is_done() {
            break;
        }
        if last_activity.elapsed() > Duration::from_secs(60) {
            app_log!("ZMODEM", "TIMEOUT: 60 秒无网络活动");
            let _ = zm_write(channel, &mut decoder, &mut pending, &zcancel());
            return Err("ZMODEM 超时(60 秒无网络活动)".into());
        }

        // Discard any keyboard input that piled up while rz owns the terminal —
        // otherwise it replays in a burst when the transfer ends.
        while keyboard_rx.try_recv().is_ok() {}

        // 1. 读取远端字节 → 解码器 → 帧入队
        let mut rb = [0u8; 8192];
        match channel.read(&mut rb) {
            Ok(0) => return Err("连接已关闭".into()),
            Ok(n) => {
                last_activity = Instant::now();
                if ZmodemDecoder::detect_cancel(&rb[..n]) {
                    return Err("远端取消传输".into());
                }
                for f in decoder.feed(&rb[..n]) {
                    pending.push(f);
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(format!("读取错误: {}", e)),
        }

        // 2. 处理队列中的帧
        let frames: Vec<DecodedFrame> = pending.drain(..).collect();
        for frame in frames {
            app_log!("ZMODEM", "帧 {:?} off={}", frame.frame_type, frame.offset());
            match sender.handle_frame(&frame) {
                SenderAction::Send(out) => {
                    zm_write(channel, &mut decoder, &mut pending, &out)?;
                    last_activity = Instant::now();
                }
                SenderAction::Error(e) => {
                    let _ = zm_write(channel, &mut decoder, &mut pending, &zcancel());
                    return Err(e);
                }
                SenderAction::FileComplete(name) => {
                    app_log!("ZMODEM", "文件完成: {}", name);
                    emit_progress(&sender, app);
                }
                SenderAction::AllComplete => {
                    app_log!("ZMODEM", "全部文件完成");
                }
                _ => {}
            }
        }

        // 3. 流式发送阶段每轮只推一个数据块（zm_write 会阻塞到真正写上线路，
        //    天然按网络速度推进 → 真实进度、内存有界）
        if sender.in_send_data() {
            match sender.next_data_chunk() {
                Some(SenderAction::Send(chunk)) => {
                    zm_write(channel, &mut decoder, &mut pending, &chunk)?;
                    last_activity = Instant::now();
                    if last_progress.elapsed() > Duration::from_millis(150) {
                        emit_progress(&sender, app);
                        last_progress = Instant::now();
                    }
                }
                Some(SenderAction::Error(e)) => {
                    let _ = zm_write(channel, &mut decoder, &mut pending, &zcancel());
                    return Err(e);
                }
                _ => {}
            }
        } else if pending.is_empty() {
            // 在等待远端，短暂让出避免空转
            std::thread::sleep(Duration::from_millis(5));
        }

        // SSH keepalive（传输本身就是活动，但保持心跳计时正常）
        if last_keepalive.elapsed() >= Duration::from_secs(15) {
            let _ = session.keepalive_send();
            last_keepalive = Instant::now();
        }
    }

    emit_progress(&sender, app);
    app_log!("ZMODEM", "run_zmodem_send 完成");
    Ok(())
}
