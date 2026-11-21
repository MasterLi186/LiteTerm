use std::io::{self, Read, Write};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

pub const DEFAULT_BAUD_RATE: u32 = 115_200;
pub const BAUD_RATES: [u32; 8] = [
    9_600, 19_200, 38_400, 57_600, 115_200, 230_400, 460_800, 921_600,
];
const IO_TIMEOUT: Duration = Duration::from_millis(80);
#[cfg(target_os = "linux")]
const UDEV_TIMEOUT: Duration = Duration::from_secs(1);
const ADB_TIMEOUT: Duration = Duration::from_millis(1_500);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdbSibling {
    pub serial: String,
    pub product: String,
    pub manufacturer: String,
    pub port: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

impl SerialPortInfo {
    pub fn device_label(&self) -> String {
        device_name(&self.path)
    }

    /// `VID:PID`（小写、四位十六进制），非 USB 设备返回 `None`。
    pub fn vid_pid(&self) -> Option<String> {
        Some(format!("{:04x}:{:04x}", self.vid?, self.pid?))
    }

    /// USB 接口号（`1-2.6.4:1.2` → `1.2`），用于区分复合设备上的多个串口。
    pub fn interface_number(&self) -> Option<&str> {
        self.devpath
            .as_deref()
            .and_then(interface_number_from_devpath)
    }

    /// 优先展示产品名，其次 udev 厂商全名和厂商名。
    pub fn display_model(&self) -> String {
        self.product
            .clone()
            .or_else(|| self.vendor_full.clone())
            .or_else(|| self.manufacturer.clone())
            .unwrap_or_else(|| self.name.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialSpec {
    pub device: String,
    pub display_name: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub serial_number: Option<String>,
    /// USB 复合设备接口号（如 `1.0` / `1.2`），用于重连时区分同一设备的多个串口。
    pub usb_interface: Option<String>,
    pub baud_rate: u32,
}

impl SerialSpec {
    /// Human-readable, unique tab label derived from the actual device node.
    ///
    /// Examples: `/dev/ttyUSB0` -> `ttyUSB0`, `COM3` -> `COM3`.
    pub fn tab_label(&self) -> String {
        let label = device_name(&self.device);
        let device_label = if label.trim().is_empty() {
            self.display_name.clone()
        } else {
            label
        };
        match self.serial_number.as_deref().map(str::trim) {
            Some(serial_number) if !serial_number.is_empty() => {
                format!("{device_label} · {serial_number}")
            }
            _ => device_label,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.device.trim().is_empty() {
            return Err("串口设备路径不能为空".into());
        }
        if !BAUD_RATES.contains(&self.baud_rate) {
            return Err(format!("不支持的波特率：{}", self.baud_rate));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SerialExit {
    Shutdown,
    DeviceEof,
    Error(String),
}

pub struct SerialHandle {
    opened_device: String,
    reader: Option<Box<dyn Read + Send>>,
    write_tx: Option<crate::zmodem::runtime::TransportWriter>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    io_done_rx: Option<mpsc::Receiver<SerialExit>>,
    join: Option<thread::JoinHandle<()>>,
}

pub struct SerialParts {
    pub reader: Box<dyn Read + Send>,
    pub write_tx: crate::zmodem::runtime::TransportWriter,
    pub shutdown_tx: mpsc::Sender<()>,
    pub io_done_rx: mpsc::Receiver<SerialExit>,
    pub join: Option<thread::JoinHandle<()>>,
}

impl std::fmt::Debug for SerialHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SerialHandle")
            .field("reader", &"<pipe>")
            .field("write_tx", &"<channel>")
            .field("shutdown_tx", &"<channel>")
            .field("io_done_rx", &"<channel>")
            .field("join", &self.join.is_some())
            .finish()
    }
}

impl SerialHandle {
    pub fn opened_device(&self) -> &str {
        &self.opened_device
    }

    pub fn into_parts(mut self) -> SerialParts {
        SerialParts {
            reader: self.reader.take().expect("serial reader is present"),
            write_tx: self.write_tx.take().expect("serial writer is present"),
            shutdown_tx: self
                .shutdown_tx
                .take()
                .expect("serial shutdown channel is present"),
            io_done_rx: self
                .io_done_rx
                .take()
                .expect("serial done channel is present"),
            join: self.join.take(),
        }
    }
}

impl Drop for SerialHandle {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.write_tx = None;
        self.reader = None;
        if let Some(join) = self.join.take() {
            let _ = thread::Builder::new()
                .name("liteterm-serial-reaper".into())
                .spawn(move || {
                    let _ = join.join();
                });
        }
    }
}

pub fn list_ports() -> Result<Vec<SerialPortInfo>, String> {
    let raw_ports =
        serialport::available_ports().map_err(|error| format!("枚举串口失败：{error}"))?;
    let has_usb = raw_ports
        .iter()
        .any(|port| matches!(&port.port_type, serialport::SerialPortType::UsbPort(_)));
    let adb_devices = if has_usb { adb_devices() } else { Vec::new() };
    let mut vendor_cache = std::collections::HashMap::new();
    let mut ports = raw_ports
        .into_iter()
        .map(|port| {
            let path = port.port_name;
            let mut info = SerialPortInfo {
                name: device_name(&path),
                path: path.clone(),
                port_type: "Unknown".to_owned(),
                vid: None,
                pid: None,
                serial_number: None,
                manufacturer: None,
                product: None,
                usb_path: None,
                usb_speed: None,
                devpath: None,
                vendor_full: None,
                adb_siblings: Vec::new(),
            };
            match port.port_type {
                serialport::SerialPortType::UsbPort(usb) => {
                    info.port_type = "USB".to_owned();
                    info.vid = Some(usb.vid);
                    info.pid = Some(usb.pid);
                    info.serial_number = normalize_text(usb.serial_number);
                    info.manufacturer = normalize_text(usb.manufacturer);
                    info.product = normalize_text(usb.product);
                }
                serialport::SerialPortType::PciPort => info.port_type = "PCI".to_owned(),
                serialport::SerialPortType::BluetoothPort => {
                    info.port_type = "Bluetooth".to_owned();
                }
                serialport::SerialPortType::Unknown => {}
            }
            if let Some(sysfs) = read_usb_sysfs(&path) {
                if info.vid.is_none() {
                    info.vid = sysfs.vid;
                }
                if info.pid.is_none() {
                    info.pid = sysfs.pid;
                }
                if info.serial_number.is_none() {
                    info.serial_number = sysfs.serial_number;
                }
                if info.manufacturer.is_none() {
                    info.manufacturer = sysfs.manufacturer;
                }
                if info.product.is_none() {
                    info.product = sysfs.product;
                }
                info.usb_path = sysfs.usb_path;
                info.usb_speed = sysfs.usb_speed;
                info.devpath = sysfs.devpath;
                info.adb_siblings = sysfs.adb_siblings;
            }
            info.vendor_full = info
                .vid
                .and_then(|vid| {
                    vendor_cache
                        .entry(vid)
                        .or_insert_with(|| udev_vendor_name(&path))
                        .clone()
                })
                .or_else(|| info.manufacturer.clone());
            info.name = info
                .product
                .clone()
                .or_else(|| info.manufacturer.clone())
                .unwrap_or_else(|| info.name.clone());
            if info.vid_pid().is_some() {
                let matched = adb_devices
                    .iter()
                    .filter(|adb| adb_matches(&info, adb))
                    .cloned()
                    .collect::<Vec<_>>();
                for adb in matched {
                    if !info
                        .adb_siblings
                        .iter()
                        .any(|existing| existing.serial == adb.serial)
                    {
                        info.adb_siblings.push(adb);
                    }
                }
            }
            info
        })
        .collect::<Vec<_>>();
    ports.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(ports)
}

fn normalize_text(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

fn interface_number_from_devpath(devpath: &str) -> Option<&str> {
    devpath
        .rsplit_once(':')
        .map(|(_, number)| number)
        .filter(|number| !number.is_empty())
}

pub fn open(spec: SerialSpec) -> Result<SerialHandle, String> {
    spec.validate()?;
    let device = resolve_device_for_open(&spec)?;
    let port = serialport::new(&device, spec.baud_rate)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .timeout(IO_TIMEOUT)
        .open()
        .map_err(|error| format!("打开串口 {device} 失败：{error}"))?;
    spawn_worker(port, device)
}

struct UsbSysfsInfo {
    vid: Option<u16>,
    pid: Option<u16>,
    serial_number: Option<String>,
    manufacturer: Option<String>,
    product: Option<String>,
    usb_path: Option<String>,
    usb_speed: Option<String>,
    devpath: Option<String>,
    adb_siblings: Vec<AdbSibling>,
}

/// 读取 Linux sysfs 中的 USB 设备信息。非 Linux 或非 USB 串口返回 `None`。
#[cfg(target_os = "linux")]
fn read_usb_sysfs(device_path: &str) -> Option<UsbSysfsInfo> {
    let tty_dir = std::path::Path::new("/sys/class/tty").join(device_name(device_path));
    let mut interface_dir = std::fs::canonicalize(tty_dir.join("device")).ok()?;
    for _ in 0..8 {
        if interface_dir.join("bInterfaceClass").exists() {
            break;
        }
        interface_dir = interface_dir.parent()?.to_path_buf();
    }
    if !interface_dir.join("bInterfaceClass").exists() {
        return None;
    }
    let interface_name = interface_dir.file_name()?.to_str()?.to_owned();
    let device_dir = interface_dir.parent()?;
    let sysfs_path = device_dir.file_name()?.to_str()?.to_owned();
    Some(UsbSysfsInfo {
        vid: sysfs_u16(device_dir, "idVendor"),
        pid: sysfs_u16(device_dir, "idProduct"),
        serial_number: sysfs_text(device_dir, "serial"),
        manufacturer: sysfs_text(device_dir, "manufacturer"),
        product: sysfs_text(device_dir, "product"),
        usb_speed: sysfs_text(device_dir, "speed").map(|speed| format_usb_speed(&speed)),
        usb_path: serial_by_path_name(device_path)
            .or_else(|| (!sysfs_path.is_empty()).then_some(sysfs_path)),
        devpath: Some(interface_name),
        adb_siblings: sysfs_adb_siblings(device_dir),
    })
}

#[cfg(not(target_os = "linux"))]
fn read_usb_sysfs(_device_path: &str) -> Option<UsbSysfsInfo> {
    None
}

/// 用 `/dev/serial/by-path` 的稳定物理路径名标识串口设备。
#[cfg(target_os = "linux")]
fn serial_by_path_name(device_path: &str) -> Option<String> {
    let device = device_name(device_path);
    let entries = std::fs::read_dir("/dev/serial/by-path").ok()?;
    for entry in entries.flatten() {
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        if target.file_name().and_then(|name| name.to_str()) == Some(device.as_str()) {
            return Some(entry.file_name().to_string_lossy().into_owned());
        }
    }
    None
}

/// 扫描同 hub 下带 ADB 接口（class ff:42:01）的 USB 设备。
#[cfg(target_os = "linux")]
fn sysfs_adb_siblings(usb_device_dir: &std::path::Path) -> Vec<AdbSibling> {
    let Some(hub) = usb_device_dir.parent() else {
        return Vec::new();
    };
    let self_name = usb_device_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let Ok(entries) = std::fs::read_dir(hub) else {
        return Vec::new();
    };
    let mut siblings = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == self_name || name.contains(':') || !path.join("idVendor").exists() {
            continue;
        }
        if !has_adb_interface(&path) {
            continue;
        }
        let Some(serial) = sysfs_text(&path, "serial") else {
            continue;
        };
        siblings.push(AdbSibling {
            serial,
            product: sysfs_text(&path, "product").unwrap_or_default(),
            manufacturer: sysfs_text(&path, "manufacturer").unwrap_or_default(),
            port: Some(name),
        });
    }
    if has_adb_interface(usb_device_dir) {
        if let Some(serial) = sysfs_text(usb_device_dir, "serial") {
            siblings.push(AdbSibling {
                serial,
                product: sysfs_text(usb_device_dir, "product").unwrap_or_default(),
                manufacturer: sysfs_text(usb_device_dir, "manufacturer").unwrap_or_default(),
                port: Some(self_name.to_owned()),
            });
        }
    }
    siblings.sort_by(|left, right| left.port.cmp(&right.port));
    let mut seen = std::collections::HashSet::new();
    siblings.retain(|sibling| seen.insert(sibling.serial.clone()));
    siblings
}

#[cfg(target_os = "linux")]
fn has_adb_interface(device_dir: &std::path::Path) -> bool {
    std::fs::read_dir(device_dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|interface| {
            let interface = interface.path();
            sysfs_text(&interface, "bInterfaceClass").as_deref() == Some("ff")
                && sysfs_text(&interface, "bInterfaceSubClass").as_deref() == Some("42")
                && sysfs_text(&interface, "bInterfaceProtocol").as_deref() == Some("01")
        })
}

#[cfg(target_os = "linux")]
fn sysfs_text(directory: &std::path::Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(directory.join(name)).ok()?;
    normalize_text(Some(text))
}

#[cfg(target_os = "linux")]
fn sysfs_u16(directory: &std::path::Path, name: &str) -> Option<u16> {
    let text = sysfs_text(directory, name)?;
    u16::from_str_radix(text.trim_start_matches("0x"), 16).ok()
}

#[cfg(target_os = "linux")]
fn format_usb_speed(raw: &str) -> String {
    match raw.trim().parse::<f64>() {
        Ok(speed) if speed >= 1000.0 => format!("{:.0} Gbps", speed / 1000.0),
        Ok(speed) if speed.fract() == 0.0 => format!("{speed:.0} Mbps"),
        Ok(speed) => format!("{speed:.1} Mbps"),
        Err(_) => raw.trim().to_owned(),
    }
}

/// 执行外部命令并限制等待时间，超时则终止，避免串口扫描被挂起的工具卡住。
fn command_output_with_timeout(
    command: &mut std::process::Command,
    timeout: Duration,
) -> Option<std::process::Output> {
    configure_hidden_command(command);
    // 用临时文件而不是管道承接 stdout：子进程的后代即使继承 stdout，也不会阻塞读取线程。
    let mut stdout_file = tempfile::tempfile().ok()?;
    let child_stdout = stdout_file.try_clone().ok()?;
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(child_stdout))
        .stderr(std::process::Stdio::null());
    let mut child = command.spawn().ok()?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                use std::io::{Read as _, Seek as _, SeekFrom};
                let mut stdout = Vec::new();
                let _ = stdout_file.seek(SeekFrom::Start(0));
                let _ = stdout_file.read_to_end(&mut stdout);
                return Some(std::process::Output {
                    status,
                    stdout,
                    stderr: Vec::new(),
                });
            }
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(windows)]
fn configure_hidden_command(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_hidden_command(_command: &mut std::process::Command) {}

/// 通过 udev 数据库把 USB VID 转成厂商全名；命令不可用时返回 `None`。
#[cfg(target_os = "linux")]
fn udev_vendor_name(device_path: &str) -> Option<String> {
    let output = command_output_with_timeout(
        std::process::Command::new("udevadm").args([
            "info",
            "--query=property",
            &format!("--name={device_path}"),
        ]),
        UDEV_TIMEOUT,
    )?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find_map(|line| line.strip_prefix("ID_VENDOR_FROM_DATABASE="))
        .map(str::trim)
        .filter(|vendor| !vendor.is_empty())
        .map(str::to_owned)
}

#[cfg(not(target_os = "linux"))]
fn udev_vendor_name(_device_path: &str) -> Option<String> {
    None
}

/// 读取 `adb devices -l`，一次枚举供所有串口匹配。
///
/// 只在 adb server 已监听 5037 时查询，避免仅打开串口列表就拉起 adb 守护进程。
fn adb_devices() -> Vec<AdbSibling> {
    let Ok(address) = "127.0.0.1:5037".parse::<std::net::SocketAddr>() else {
        return Vec::new();
    };
    if std::net::TcpStream::connect_timeout(&address, Duration::from_millis(30)).is_err() {
        return Vec::new();
    }
    let output = match command_output_with_timeout(
        std::process::Command::new("adb").args(["devices", "-l"]),
        ADB_TIMEOUT,
    ) {
        Some(output) if output.status.success() => output,
        _ => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut devices: Vec<AdbSibling> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("List of devices")
            || line.starts_with("* daemon")
            || line.starts_with("adb server")
        {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(serial) = parts.next() else {
            continue;
        };
        let Some(state) = parts.next() else {
            continue;
        };
        if state != "device" {
            continue;
        }
        let mut product = String::new();
        let mut manufacturer = String::new();
        let mut port = None;
        for property in parts {
            if let Some(value) = property.strip_prefix("product:") {
                product = value.to_owned();
            } else if let Some(value) = property.strip_prefix("manufacturer:") {
                manufacturer = value.to_owned();
            } else if let Some(value) = property.strip_prefix("usb:") {
                port = Some(value.to_owned());
            }
        }
        if devices.iter().any(|device| device.serial == serial) {
            continue;
        }
        devices.push(AdbSibling {
            serial: serial.to_owned(),
            product,
            manufacturer,
            port,
        });
    }
    devices
}

fn adb_matches(info: &SerialPortInfo, adb: &AdbSibling) -> bool {
    if info.serial_number.as_deref().map(str::trim) == Some(adb.serial.trim()) {
        return true;
    }
    if let (Some(adb_port), Some(devpath)) = (adb.port.as_deref(), info.devpath.as_deref()) {
        let usb_device = devpath.split(':').next().unwrap_or(devpath);
        return adb_port == usb_device;
    }
    info.vid_pid()
        .is_some_and(|vid_pid| adb.serial.to_ascii_lowercase().starts_with(&vid_pid))
}

pub fn user_open_error_message(error: &str) -> String {
    let normalized = error.to_ascii_lowercase();
    if crate::i18n::is_english() {
        let hint = if normalized.contains("resource busy")
            || normalized.contains("device busy")
            || normalized.contains("being used")
            || normalized.contains("access is denied")
        {
            "This serial port is in use by another application. Disconnect it there, then try again."
        } else if normalized.contains("permission denied") || normalized.contains("access denied") {
            "The current user does not have permission to access this serial port. Check the dialout group or device permissions."
        } else if normalized.contains("no such file")
            || normalized.contains("not found")
            || normalized.contains("cannot find")
        {
            "The serial device was removed or its path changed. Reconnect the device, then try again."
        } else {
            "Could not open this serial port. Check the device connection, whether it is in use, and access permissions."
        };
        return format!("{hint}\n\nSystem response: {error}");
    }
    let hint = if normalized.contains("resource busy")
        || normalized.contains("device busy")
        || normalized.contains("being used")
        || normalized.contains("access is denied")
    {
        "该串口正被其他程序占用，请先在 Tabby、串口助手等程序中断开后再重试。"
    } else if normalized.contains("permission denied") || normalized.contains("access denied") {
        "当前用户没有串口访问权限，请检查 dialout 组或系统设备权限。"
    } else if normalized.contains("no such file")
        || normalized.contains("not found")
        || normalized.contains("cannot find")
    {
        "串口设备已拔出或设备路径发生变化，请重新插入设备后重试。"
    } else {
        "无法打开该串口，请检查设备连接、占用状态和访问权限。"
    };
    format!("{hint}\n\n系统返回：{error}")
}

/// 判断枚举到的原路径设备是否与保存的 USB 身份冲突；缺失元数据视为无法判断（不冲突）。
fn port_matches_saved_identity(port: &serialport::SerialPortInfo, spec: &SerialSpec) -> bool {
    let serialport::SerialPortType::UsbPort(usb) = &port.port_type else {
        return true;
    };
    if let Some(expected) = spec
        .serial_number
        .as_deref()
        .map(str::trim)
        .filter(|serial| !serial.is_empty())
    {
        if let Some(actual) = usb
            .serial_number
            .as_deref()
            .map(str::trim)
            .filter(|serial| !serial.is_empty())
        {
            if actual != expected {
                return false;
            }
        }
    }
    spec.vid.map_or(true, |vid| usb.vid == vid) && spec.pid.map_or(true, |pid| usb.pid == pid)
}

fn resolve_device_for_open(spec: &SerialSpec) -> Result<String, String> {
    let expected_serial = spec
        .serial_number
        .as_deref()
        .map(str::trim)
        .filter(|serial| !serial.is_empty());
    let vid_pid = match (spec.vid, spec.pid) {
        (Some(vid), Some(pid)) => Some(format!("{vid:04x}:{pid:04x}")),
        _ => None,
    };
    if expected_serial.is_none() && vid_pid.is_none() {
        return Ok(spec.device.clone());
    }
    let identity = match (expected_serial, vid_pid.as_deref()) {
        (Some(serial), Some(vid_pid)) => format!("硬件 SN {serial}（VID:PID {vid_pid}）"),
        (Some(serial), None) => format!("硬件 SN {serial}"),
        (None, Some(vid_pid)) => format!("VID:PID {vid_pid}"),
        (None, None) => unreachable!(),
    };
    let available = serialport::available_ports()
        .map_err(|error| format!("按硬件标识查找串口失败：{error}"))?;
    // 原路径仍然存在且没有身份冲突时直接使用，避免驱动未上报 USB 元数据时误判为设备丢失。
    if available
        .iter()
        .find(|port| port.port_name == spec.device)
        .is_some_and(|port| port_matches_saved_identity(port, spec))
    {
        return Ok(spec.device.clone());
    }
    let candidates = available
        .into_iter()
        .filter_map(|port| match port.port_type {
            serialport::SerialPortType::UsbPort(usb) => {
                let actual_serial = usb
                    .serial_number
                    .as_deref()
                    .map(str::trim)
                    .filter(|serial| !serial.is_empty());
                let serial_matches = match (expected_serial, actual_serial) {
                    (Some(expected), Some(actual)) => actual == expected,
                    // 驱动未上报 SN 时不能据此判定为不匹配，交由 VID/PID 和接口号继续筛选。
                    _ => true,
                };
                let vid_matches = spec.vid.map(|vid| usb.vid == vid).unwrap_or(true);
                let pid_matches = spec.pid.map(|pid| usb.pid == pid).unwrap_or(true);
                if !(serial_matches && vid_matches && pid_matches) {
                    return None;
                }
                let device = port.port_name;
                let interface = read_usb_sysfs(&device)
                    .and_then(|sysfs| sysfs.devpath)
                    .and_then(|devpath| interface_number_from_devpath(&devpath).map(str::to_owned));
                Some(UsbCandidate { device, interface })
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if let Some(expected_interface) = spec.usb_interface.as_deref() {
        let mut matching = candidates
            .iter()
            .filter(|candidate| candidate.interface.as_deref() == Some(expected_interface));
        if let (Some(candidate), None) = (matching.next(), matching.next()) {
            return Ok(candidate.device.clone());
        }
    }
    let devices = candidates
        .iter()
        .map(|candidate| candidate.device.clone())
        .collect::<Vec<_>>();
    choose_serial_device(&spec.device, &identity, &devices)
}

struct UsbCandidate {
    device: String,
    interface: Option<String>,
}

fn choose_serial_device(
    original_device: &str,
    identity: &str,
    candidates: &[String],
) -> Result<String, String> {
    if candidates.iter().any(|device| device == original_device) {
        return Ok(original_device.to_owned());
    }
    match candidates {
        [device] => Ok(device.clone()),
        [] => Err(format!("未找到{identity}的串口设备")),
        _ => Err(format!("{identity} 对应多个串口接口，请重新选择具体设备")),
    }
}

fn spawn_worker(
    mut port: Box<dyn serialport::SerialPort>,
    opened_device: String,
) -> Result<SerialHandle, String> {
    let (reader, mut writer) =
        os_pipe::pipe().map_err(|error| format!("创建串口输出管道失败：{error}"))?;
    let protocol_gate = Arc::new(crate::zmodem::runtime::ProtocolGate::new());
    let (write_tx, write_rx) = crate::zmodem::runtime::transport_write_channel(protocol_gate);
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    let (io_done_tx, io_done_rx) = mpsc::channel::<SerialExit>();

    let join = thread::Builder::new()
        .name("liteterm-serial".into())
        .spawn(move || {
            let mut buffer = [0_u8; 4096];
            let exit = loop {
                match shutdown_rx.try_recv() {
                    Ok(()) | Err(mpsc::TryRecvError::Disconnected) => {
                        break SerialExit::Shutdown;
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }

                match drain_writes(port.as_mut(), &write_rx) {
                    Ok(WriteDrain::Continue) => {}
                    Ok(WriteDrain::Disconnected) => break SerialExit::Shutdown,
                    Err(error) => break SerialExit::Error(format!("串口写入失败：{error}")),
                }

                match port.read(&mut buffer) {
                    Ok(0) => break SerialExit::DeviceEof,
                    Ok(count) => {
                        if let Err(error) = writer.write_all(&buffer[..count]) {
                            let reason = if error.kind() == io::ErrorKind::BrokenPipe {
                                SerialExit::Shutdown
                            } else {
                                SerialExit::Error(format!("串口输出管道失败：{error}"))
                            };
                            break reason;
                        }
                    }
                    Err(error) if is_timeout(&error) => {}
                    Err(error) => break SerialExit::Error(format!("串口读取失败：{error}")),
                }
            };
            drop(writer);
            drop(port);
            let _ = io_done_tx.send(exit);
        })
        .map_err(|error| format!("启动串口线程失败：{error}"))?;

    Ok(SerialHandle {
        opened_device,
        reader: Some(Box::new(reader)),
        write_tx: Some(write_tx),
        shutdown_tx: Some(shutdown_tx),
        io_done_rx: Some(io_done_rx),
        join: Some(join),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteDrain {
    Continue,
    Disconnected,
}

fn drain_writes(
    writer: &mut dyn Write,
    receiver: &mpsc::Receiver<crate::zmodem::runtime::TransportWrite>,
) -> io::Result<WriteDrain> {
    loop {
        match receiver.try_recv() {
            Ok(crate::zmodem::runtime::TransportWrite::Normal { bytes, .. }) => {
                writer.write_all(&bytes).and_then(|_| writer.flush())?;
            }
            Ok(crate::zmodem::runtime::TransportWrite::Protocol(request)) => {
                request.complete(Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "串口不支持 ZMODEM 协议写入",
                )));
            }
            Ok(crate::zmodem::runtime::TransportWrite::TerminalReply(request)) => {
                if !request.begin() {
                    request.complete(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "终端应答在串口写入前已取消或超时",
                    )));
                    continue;
                }
                let mut offset = 0;
                let mut result = Ok(());
                while offset < request.bytes().len() {
                    if !request.may_continue() {
                        result = Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "串口终端应答写入已取消或超时",
                        ));
                        break;
                    }
                    match writer.write(&request.bytes()[offset..]) {
                        Ok(0) => {
                            result = Err(io::Error::new(
                                io::ErrorKind::WriteZero,
                                "串口 writer 返回零长度写入",
                            ));
                            break;
                        }
                        Ok(written) => {
                            request.mark_progress();
                            offset += written;
                        }
                        Err(error) => {
                            result = Err(error);
                            break;
                        }
                    }
                }
                if result.is_ok() {
                    if request.may_continue() {
                        result = writer.flush();
                        if result.is_ok() && !request.may_continue() {
                            result = Err(io::Error::new(
                                io::ErrorKind::TimedOut,
                                "串口终端应答 flush 后超过硬截止时间",
                            ));
                        }
                    } else {
                        result = Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "串口终端应答在完成前超过硬截止时间",
                        ));
                    }
                }
                let failed = result.is_err()
                    && (request.requires_transport_shutdown()
                        || result
                            .as_ref()
                            .is_err_and(|error| error.kind() != io::ErrorKind::TimedOut));
                request.complete(result);
                if failed {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "串口终端应答写入失败",
                    ));
                }
            }
            Err(mpsc::TryRecvError::Empty) => return Ok(WriteDrain::Continue),
            Err(mpsc::TryRecvError::Disconnected) => return Ok(WriteDrain::Disconnected),
        }
    }
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}

fn device_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct RecordingWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for RecordingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn serial_spec_requires_a_device_and_supported_baud_rate() {
        let mut spec = SerialSpec {
            device: " ".into(),
            display_name: "USB".into(),
            vid: None,
            pid: None,
            serial_number: None,
            usb_interface: None,
            baud_rate: DEFAULT_BAUD_RATE,
        };
        assert!(spec.validate().is_err());

        spec.device = "/dev/ttyUSB0".into();
        assert!(spec.validate().is_ok());

        spec.baud_rate = 12_345;
        assert!(spec.validate().is_err());
    }

    #[test]
    fn baud_rates_are_sorted_and_include_the_default() {
        assert!(BAUD_RATES.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(BAUD_RATES.contains(&DEFAULT_BAUD_RATE));
    }

    #[test]
    fn write_drain_preserves_message_order_and_bytes() {
        let gate = Arc::new(crate::zmodem::runtime::ProtocolGate::new());
        let (sender, receiver) = crate::zmodem::runtime::transport_write_channel(gate);
        sender.try_send_normal(vec![0, 1, 2]).unwrap();
        sender.try_send_normal("终端".as_bytes().to_vec()).unwrap();
        let mut writer = RecordingWriter::default();

        assert_eq!(
            drain_writes(&mut writer, &receiver).unwrap(),
            WriteDrain::Continue
        );
        assert_eq!(
            writer.0.lock().unwrap().as_slice(),
            [0, 1, 2]
                .into_iter()
                .chain("终端".as_bytes().iter().copied())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn disconnected_input_channel_stops_the_worker_side() {
        let gate = Arc::new(crate::zmodem::runtime::ProtocolGate::new());
        let (sender, receiver) = crate::zmodem::runtime::transport_write_channel(gate);
        drop(sender);
        let mut writer = RecordingWriter::default();
        assert_eq!(
            drain_writes(&mut writer, &receiver).unwrap(),
            WriteDrain::Disconnected
        );
    }

    #[test]
    fn serial_terminal_reply_ack_follows_device_write_and_flush() {
        let gate = Arc::new(crate::zmodem::runtime::ProtocolGate::new());
        let (transport, receiver) = crate::zmodem::runtime::transport_write_channel(gate);
        // Keep one sender alive while the test drains the worker queue.  Without
        // this, a loaded CI runner can let the reply thread time out and drop
        // its only sender before the queued request is observed, making the
        // test report `Disconnected` instead of exercising the ACK path.
        let _transport_keepalive = transport.clone();
        let reply_writer =
            crate::zmodem::runtime::TerminalReplyWriter::from_transport_writer(transport);
        let reply = std::thread::spawn(move || {
            reply_writer.write_and_flush(b"\x1b[?6c", Duration::from_secs(1))
        });
        let output = Arc::new(Mutex::new(Vec::new()));
        let mut writer = RecordingWriter(Arc::clone(&output));
        let deadline = std::time::Instant::now() + Duration::from_secs(1);

        while output.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
            assert_eq!(
                drain_writes(&mut writer, &receiver).unwrap(),
                WriteDrain::Continue
            );
            std::thread::yield_now();
        }

        reply.join().unwrap().unwrap();
        assert_eq!(&*output.lock().unwrap(), b"\x1b[?6c");
    }

    #[test]
    fn device_name_uses_the_final_path_component() {
        assert_eq!(device_name("/dev/ttyUSB0"), "ttyUSB0");
        assert_eq!(device_name("COM3"), "COM3");
    }

    #[test]
    fn serial_tab_label_uses_device_node_instead_of_usb_product_name() {
        let linux = SerialSpec {
            device: "/dev/ttyUSB7".into(),
            display_name: "FT232R USB UART".into(),
            vid: None,
            pid: None,
            serial_number: None,
            usb_interface: None,
            baud_rate: DEFAULT_BAUD_RATE,
        };
        let windows = SerialSpec {
            device: "COM12".into(),
            display_name: "USB Serial Port".into(),
            vid: None,
            pid: None,
            serial_number: None,
            usb_interface: None,
            baud_rate: DEFAULT_BAUD_RATE,
        };
        let macos = SerialSpec {
            device: "/dev/tty.usbserial-A10K9".into(),
            display_name: "FTDI".into(),
            vid: None,
            pid: None,
            serial_number: None,
            usb_interface: None,
            baud_rate: DEFAULT_BAUD_RATE,
        };

        assert_eq!(linux.tab_label(), "ttyUSB7");
        assert_eq!(windows.tab_label(), "COM12");
        assert_eq!(macos.tab_label(), "tty.usbserial-A10K9");
    }

    #[test]
    fn serial_tab_label_appends_trimmed_hardware_serial_number() {
        let spec = SerialSpec {
            device: "/dev/ttyUSB1".into(),
            display_name: "FT232R USB UART".into(),
            vid: Some(0x0403),
            pid: Some(0x6001),
            serial_number: Some(" A10LCL3D ".into()),
            usb_interface: Some("1.0".into()),
            baud_rate: DEFAULT_BAUD_RATE,
        };

        assert_eq!(spec.tab_label(), "ttyUSB1 · A10LCL3D");
    }

    #[test]
    fn serial_reconnect_resolves_a_unique_renumbered_device_by_hardware_serial() {
        let candidates = vec!["/dev/ttyUSB4".to_owned()];

        assert_eq!(
            choose_serial_device("/dev/ttyUSB1", "A10LCL3D", &candidates).unwrap(),
            "/dev/ttyUSB4"
        );
    }

    #[test]
    fn serial_reconnect_prefers_the_original_interface_and_rejects_ambiguity() {
        let existing = vec!["/dev/ttyACM2".to_owned(), "/dev/ttyACM3".to_owned()];
        assert_eq!(
            choose_serial_device("/dev/ttyACM3", "5A7E086941", &existing).unwrap(),
            "/dev/ttyACM3"
        );

        let renumbered = vec!["/dev/ttyACM4".to_owned(), "/dev/ttyACM5".to_owned()];
        assert!(choose_serial_device("/dev/ttyACM3", "5A7E086941", &renumbered).is_err());
    }

    #[test]
    fn timeout_classification_keeps_the_worker_alive() {
        for kind in [
            io::ErrorKind::TimedOut,
            io::ErrorKind::WouldBlock,
            io::ErrorKind::Interrupted,
        ] {
            assert!(is_timeout(&io::Error::from(kind)));
        }
        assert!(!is_timeout(&io::Error::from(io::ErrorKind::BrokenPipe)));
    }
}
