use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::app_log;
use crate::state::{AppState, LocalTerminal};

#[derive(Serialize)]
pub struct SerialPortInfo {
    pub name: String,
    pub path: String,
    pub port_type: String,
}

#[tauri::command]
pub async fn list_serial_ports() -> Result<Vec<SerialPortInfo>, String> {
    let ports = serialport::available_ports().map_err(|e| e.to_string())?;
    Ok(ports
        .iter()
        .map(|p| {
            let port_type = match &p.port_type {
                serialport::SerialPortType::UsbPort(_) => "USB",
                serialport::SerialPortType::PciPort => "PCI",
                serialport::SerialPortType::BluetoothPort => "Bluetooth",
                serialport::SerialPortType::Unknown => "Unknown",
            }
            .to_string();
            let name = std::path::Path::new(&p.port_name)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(p.port_name.clone());
            SerialPortInfo {
                name,
                path: p.port_name.clone(),
                port_type,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn open_serial_terminal(
    state: State<'_, AppState>,
    app: AppHandle,
    device: String,
    baud_rate: u32,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let stop = Arc::new(AtomicBool::new(false));

    app_log!("SERIAL", "打开串口: device={} baud_rate={}", device, baud_rate);
    let port = serialport::new(&device, baud_rate)
        .timeout(std::time::Duration::from_millis(100))
        .open()
        .map_err(|e| {
            app_log!("SERIAL", "ERROR: 打开串口失败: {} (device={})", e, device);
            format!("打开串口失败: {}", e)
        })?;

    let mut reader = port.try_clone().map_err(|e| e.to_string())?;
    let writer = port;

    // Reader thread with stop flag
    let id_clone = id.clone();
    let app_clone = app.clone();
    let stop_r = stop.clone();
    std::thread::spawn(move || {
        // Wait for frontend terminal component to mount and register event listener
        std::thread::sleep(std::time::Duration::from_millis(600));
        let mut buf = [0u8; 4096];
        loop {
            if stop_r.load(Ordering::Relaxed) {
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = app_clone.emit(
                        "terminal-output",
                        serde_json::json!({
                            "id": id_clone,
                            "data": buf[..n].to_vec(),
                        }),
                    );
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(e) => {
                    app_log!("SERIAL", "ERROR: 读取串口失败: {}", e);
                    break;
                }
            }
        }
        // reader drops here, releasing the fd clone
    });

    // Writer thread with stop flag
    let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let stop_w = stop.clone();
    std::thread::spawn(move || {
        let mut writer = writer;
        loop {
            if stop_w.load(Ordering::Relaxed) {
                break;
            }
            match input_rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(data) => {
                    use std::io::Write;
                    if let Err(e) = writer.write_all(&data) {
                        app_log!("SERIAL", "ERROR: 写入串口失败: {}", e);
                    }
                    let _ = writer.flush();
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        // writer (port) drops here, releasing the fd
    });

    // Send an initial CR after frontend has time to mount the terminal
    let init_tx = input_tx.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(800));
        let _ = init_tx.send(b"\r".to_vec());
    });

    let (resize_tx, _) = std::sync::mpsc::channel::<(u32, u32)>();

    state.local_terminals.lock().unwrap().insert(
        id.clone(),
        LocalTerminal {
            id: id.clone(),
            input_tx,
            resize_tx,
            stop,
        },
    );

    Ok(id)
}
