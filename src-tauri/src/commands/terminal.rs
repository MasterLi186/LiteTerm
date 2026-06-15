use std::io::{Read, Write};

use portable_pty::{CommandBuilder, PtySize};
use tauri::{AppHandle, Emitter, State};

use crate::state::{AppState, LocalTerminal};

#[derive(serde::Serialize)]
pub struct ShellInfo {
    pub name: String,
    pub path: String,
}

#[tauri::command]
pub async fn open_local_terminal(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();

    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let cmd = CommandBuilder::new(shell);
    let _child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    // Input channel: frontend -> writer thread
    let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();

    // Reader thread: PTY output -> Tauri event (ZMODEM handled on frontend)
    // Delay start to let frontend register event listener first
    let id_clone = id.clone();
    let app_clone = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = app_clone.emit(
                        "terminal-output",
                        serde_json::json!({
                            "id": id_clone,
                            "data": &buf[..n],
                        }),
                    );
                }
                Err(_) => break,
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

    // Resize channel
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

    state
        .local_terminals
        .lock()
        .unwrap()
        .insert(id.clone(), LocalTerminal {
            id: id.clone(),
            input_tx,
            resize_tx,
            stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });

    Ok(id)
}

#[tauri::command]
pub async fn list_shells() -> Result<Vec<ShellInfo>, String> {
    let mut shells = Vec::new();
    if let Ok(content) = std::fs::read_to_string("/etc/shells") {
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('/') && std::path::Path::new(line).exists() {
                let name = std::path::Path::new(line)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                shells.push(ShellInfo { name, path: line.to_string() });
            }
        }
    }
    shells.dedup_by(|a, b| a.name == b.name);
    Ok(shells)
}

#[tauri::command]
pub async fn open_shell_terminal(
    state: State<'_, AppState>,
    app: AppHandle,
    shell_path: String,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();

    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let cmd = CommandBuilder::new(&shell_path);
    let _child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| e.to_string())?;
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
                Ok(0) => break,
                Ok(n) => {
                    let _ = app_clone.emit(
                        "terminal-output",
                        serde_json::json!({
                            "id": id_clone,
                            "data": &buf[..n],
                        }),
                    );
                }
                Err(_) => break,
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

    state
        .local_terminals
        .lock()
        .unwrap()
        .insert(id.clone(), LocalTerminal {
            id: id.clone(),
            input_tx,
            resize_tx,
            stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });

    Ok(id)
}

#[tauri::command]
pub async fn terminal_write(
    state: State<'_, AppState>,
    id: String,
    data: Vec<u8>,
) -> Result<(), String> {
    if let Some(term) = state.local_terminals.lock().unwrap().get(&id) {
        term.input_tx.send(data).map_err(|e| e.to_string())?;
        return Ok(());
    }
    if let Some(session) = state.sessions.lock().unwrap().get(&id) {
        session.input_tx.send(data).map_err(|e| e.to_string())?;
        return Ok(());
    }
    Err("terminal not found".to_string())
}

#[tauri::command]
pub async fn terminal_resize(
    state: State<'_, AppState>,
    id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    if let Some(term) = state.local_terminals.lock().unwrap().get(&id) {
        term.resize_tx
            .send((cols, rows))
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    if let Some(session) = state.sessions.lock().unwrap().get(&id) {
        session
            .resize_tx
            .send((cols, rows))
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    Err("terminal not found".to_string())
}

#[tauri::command]
pub async fn close_terminal(state: State<'_, AppState>, id: String) -> Result<(), String> {
    if let Some(term) = state.local_terminals.lock().unwrap().remove(&id) {
        // Signal reader/writer threads to stop, releasing serial port fd
        term.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        // Drop input_tx to unblock writer thread's recv()
        drop(term.input_tx);
    }
    if let Some(session) = state.sessions.lock().unwrap().remove(&id) {
        session
            .monitor_stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}
