use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::app_log;
use crate::state::{AppState, LocalTerminal};

#[derive(Clone, Serialize)]
pub struct AdbSibling {
    pub serial: String,
    pub product: String,
    pub manufacturer: String,
    pub port: Option<String>,
}

#[derive(Serialize)]
pub struct SerialPortInfo {
    pub name: String,
    pub path: String,
    pub port_type: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub serial_number: Option<String>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub usb_path: Option<String>,
    pub usb_speed: Option<String>,
    pub devpath: Option<String>,
    pub vendor_full: Option<String>,
    pub adb_siblings: Vec<AdbSibling>,
}

#[cfg(target_os = "linux")]
struct SysfsUsbInfo {
    usb_path: Option<String>,
    speed: Option<String>,
    devpath: Option<String>,
    vendor_full: Option<String>,
    adb_siblings: Vec<AdbSibling>,
}

/// 从 sysfs 向上找到包含 idVendor 的 USB 设备目录
#[cfg(target_os = "linux")]
fn find_usb_device_dir(dev_name: &str) -> Option<std::path::PathBuf> {
    let tty_device = std::fs::canonicalize(
        format!("/sys/class/tty/{}/device", dev_name)
    ).ok()?;
    let mut dir = tty_device.as_path();
    loop {
        if dir.join("idVendor").exists() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// 扫描同 hub 下所有 ADB 设备（class ff:42:01），标注端口号
#[cfg(target_os = "linux")]
fn find_adb_siblings(usb_dev_dir: &std::path::Path) -> Vec<AdbSibling> {
    let hub = match usb_dev_dir.parent() {
        Some(h) => h,
        None => return vec![],
    };
    let hub_entries = match std::fs::read_dir(hub) {
        Ok(e) => e,
        Err(_) => return vec![],
    };
    let self_name = usb_dev_dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let mut siblings = Vec::new();

    for entry in hub_entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == self_name || name_str.contains(':') || !entry.path().join("idVendor").exists() {
            continue;
        }
        let sibling = entry.path();
        let has_adb = std::fs::read_dir(&sibling).into_iter().flatten().flatten().any(|iface| {
            let p = iface.path();
            let class = std::fs::read_to_string(p.join("bInterfaceClass")).unwrap_or_default();
            let sub = std::fs::read_to_string(p.join("bInterfaceSubClass")).unwrap_or_default();
            let proto = std::fs::read_to_string(p.join("bInterfaceProtocol")).unwrap_or_default();
            class.trim() == "ff" && sub.trim() == "42" && proto.trim() == "01"
        });
        if has_adb {
            let serial = std::fs::read_to_string(sibling.join("serial"))
                .unwrap_or_default().trim().to_string();
            let product = std::fs::read_to_string(sibling.join("product"))
                .unwrap_or_default().trim().to_string();
            let manufacturer = std::fs::read_to_string(sibling.join("manufacturer"))
                .unwrap_or_default().trim().to_string();
            let port = std::fs::read_to_string(sibling.join("devpath"))
                .ok().map(|s| s.trim().to_string());
            if !serial.is_empty() {
                siblings.push(AdbSibling { serial, product, manufacturer, port });
            }
        }
    }
    siblings.sort_by(|a, b| a.port.cmp(&b.port));
    siblings
}

/// 从 udevadm 读取 USB ID 数据库中的厂商全名
#[cfg(target_os = "linux")]
fn read_vendor_from_database(port_name: &str) -> Option<String> {
    let output = std::process::Command::new("udevadm")
        .args(["info", "--query=property", &format!("--name={}", port_name)])
        .output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(val) = line.strip_prefix("ID_VENDOR_FROM_DATABASE=") {
            return Some(val.to_string());
        }
    }
    None
}

/// 从 sysfs、/dev/serial/by-path/、udevadm 读取 USB 详细信息
#[cfg(target_os = "linux")]
fn read_usb_sysfs(port_name: &str) -> SysfsUsbInfo {
    let dev_name = std::path::Path::new(port_name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    let usb_path = (|| -> Option<String> {
        let by_path = std::path::Path::new("/dev/serial/by-path");
        for entry in std::fs::read_dir(by_path).ok()? {
            let entry = entry.ok()?;
            let target = std::fs::read_link(entry.path()).ok()?;
            if target.file_name()?.to_str()? == dev_name {
                return Some(entry.file_name().to_string_lossy().to_string());
            }
        }
        None
    })();

    let usb_dev_dir = find_usb_device_dir(dev_name);

    let (speed, devpath) = usb_dev_dir.as_ref()
        .map(|dir| {
            let speed = std::fs::read_to_string(dir.join("speed")).ok()
                .map(|s| format!("{}Mbps", s.trim()))
                .unwrap_or_default();
            let devpath = std::fs::read_to_string(dir.join("devpath")).ok()
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            (Some(speed), Some(devpath))
        })
        .unwrap_or((None, None));

    let adb_siblings = usb_dev_dir.as_ref()
        .map(|dir| find_adb_siblings(dir))
        .unwrap_or_default();

    let vendor_full = read_vendor_from_database(port_name);

    SysfsUsbInfo { usb_path, speed, devpath, vendor_full, adb_siblings }
}

#[cfg(not(target_os = "linux"))]
struct SysfsUsbInfo {
    usb_path: Option<String>,
    speed: Option<String>,
    devpath: Option<String>,
    vendor_full: Option<String>,
    adb_siblings: Vec<AdbSibling>,
}

#[cfg(not(target_os = "linux"))]
fn read_usb_sysfs(_port_name: &str) -> SysfsUsbInfo {
    SysfsUsbInfo { usb_path: None, speed: None, devpath: None, vendor_full: None, adb_siblings: vec![] }
}

#[tauri::command]
pub async fn list_serial_ports() -> Result<Vec<SerialPortInfo>, String> {
    let ports = serialport::available_ports().map_err(|e| e.to_string())?;
    Ok(ports
        .iter()
        .map(|p| {
            let (port_type, vid, pid, serial_number, manufacturer, product) = match &p.port_type {
                serialport::SerialPortType::UsbPort(usb) => (
                    "USB".to_string(),
                    Some(usb.vid),
                    Some(usb.pid),
                    usb.serial_number.clone(),
                    usb.manufacturer.clone(),
                    usb.product.clone(),
                ),
                serialport::SerialPortType::PciPort => ("PCI".to_string(), None, None, None, None, None),
                serialport::SerialPortType::BluetoothPort => ("Bluetooth".to_string(), None, None, None, None, None),
                serialport::SerialPortType::Unknown => ("Unknown".to_string(), None, None, None, None, None),
            };
            let name = product.clone()
                .or_else(|| manufacturer.clone())
                .unwrap_or_else(|| {
                    std::path::Path::new(&p.port_name)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or(p.port_name.clone())
                });
            let sysfs = read_usb_sysfs(&p.port_name);
            SerialPortInfo {
                name,
                path: p.port_name.clone(),
                port_type,
                vid,
                pid,
                serial_number,
                manufacturer,
                product,
                usb_path: sysfs.usb_path,
                usb_speed: sysfs.speed,
                devpath: sysfs.devpath,
                vendor_full: sysfs.vendor_full,
                adb_siblings: sysfs.adb_siblings,
            }
        })
        .collect())
}

/// 设备标识符：支持路径、VID:PID、VID:PID:SERIAL 三种格式
fn resolve_device_path(device: &str) -> Result<String, String> {
    // 纯路径：/dev/ttyUSB0 或 COM3
    if device.starts_with('/') || device.starts_with("COM") {
        return Ok(device.to_string());
    }

    // VID:PID 或 VID:PID:SERIAL
    let parts: Vec<&str> = device.split(':').collect();
    if parts.len() < 2 {
        return Err(format!("无法识别的设备标识: {}", device));
    }

    let vid = u16::from_str_radix(parts[0], 16)
        .map_err(|_| format!("无效的 VID: {}", parts[0]))?;
    let pid = u16::from_str_radix(parts[1], 16)
        .map_err(|_| format!("无效的 PID: {}", parts[1]))?;
    let serial_filter = parts.get(2).copied();

    let ports = serialport::available_ports()
        .map_err(|e| format!("枚举串口失败: {}", e))?;

    for p in &ports {
        if let serialport::SerialPortType::UsbPort(usb) = &p.port_type {
            if usb.vid == vid && usb.pid == pid {
                if let Some(sf) = serial_filter {
                    if usb.serial_number.as_deref() == Some(sf) {
                        return Ok(p.port_name.clone());
                    }
                } else {
                    return Ok(p.port_name.clone());
                }
            }
        }
    }

    Err(format!("未找到匹配的设备: {}", device))
}

#[derive(Deserialize)]
pub struct OpenSerialParams {
    pub device: String,
    pub baud_rate: u32,
}

#[tauri::command]
pub async fn open_serial_terminal(
    state: State<'_, AppState>,
    app: AppHandle,
    device: String,
    baud_rate: u32,
) -> Result<String, String> {
    let resolved = resolve_device_path(&device)?;
    app_log!("SERIAL", "打开串口: device={} (resolved={}) baud_rate={}", device, resolved, baud_rate);

    let id = uuid::Uuid::new_v4().to_string();
    let stop = Arc::new(AtomicBool::new(false));

    let port = serialport::new(&resolved, baud_rate)
        .timeout(std::time::Duration::from_millis(100))
        .open()
        .map_err(|e| {
            app_log!("SERIAL", "ERROR: 打开串口失败: {} (device={})", e, resolved);
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
