use std::io::{self, Read, Write};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

pub const DEFAULT_BAUD_RATE: u32 = 115_200;
pub const BAUD_RATES: [u32; 8] = [
    9_600, 19_200, 38_400, 57_600, 115_200, 230_400, 460_800, 921_600,
];
const IO_TIMEOUT: Duration = Duration::from_millis(80);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialPortInfo {
    pub name: String,
    pub path: String,
    pub port_type: String,
    pub serial_number: Option<String>,
}

impl SerialPortInfo {
    pub fn device_label(&self) -> String {
        device_name(&self.path)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialSpec {
    pub device: String,
    pub display_name: String,
    pub serial_number: Option<String>,
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
    let mut ports = serialport::available_ports()
        .map_err(|error| format!("枚举串口失败：{error}"))?
        .into_iter()
        .map(|port| {
            let (name, port_type, serial_number) = match port.port_type {
                serialport::SerialPortType::UsbPort(usb) => {
                    let name = usb
                        .product
                        .or(usb.manufacturer)
                        .unwrap_or_else(|| device_name(&port.port_name));
                    let serial_number = usb
                        .serial_number
                        .map(|serial_number| serial_number.trim().to_owned())
                        .filter(|serial_number| !serial_number.is_empty());
                    (name, "USB".to_owned(), serial_number)
                }
                serialport::SerialPortType::PciPort => {
                    (device_name(&port.port_name), "PCI".to_owned(), None)
                }
                serialport::SerialPortType::BluetoothPort => {
                    (device_name(&port.port_name), "Bluetooth".to_owned(), None)
                }
                serialport::SerialPortType::Unknown => {
                    (device_name(&port.port_name), "Unknown".to_owned(), None)
                }
            };
            SerialPortInfo {
                name,
                path: port.port_name,
                port_type,
                serial_number,
            }
        })
        .collect::<Vec<_>>();
    ports.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(ports)
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

pub fn user_open_error_message(error: &str) -> String {
    let normalized = error.to_ascii_lowercase();
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

fn resolve_device_for_open(spec: &SerialSpec) -> Result<String, String> {
    let Some(expected_serial) = spec
        .serial_number
        .as_deref()
        .map(str::trim)
        .filter(|serial| !serial.is_empty())
    else {
        return Ok(spec.device.clone());
    };
    let candidates = serialport::available_ports()
        .map_err(|error| format!("按硬件 SN 查找串口失败：{error}"))?
        .into_iter()
        .filter_map(|port| match port.port_type {
            serialport::SerialPortType::UsbPort(usb)
                if usb.serial_number.as_deref().map(str::trim) == Some(expected_serial) =>
            {
                Some(port.port_name)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    choose_serial_device(&spec.device, expected_serial, &candidates)
}

fn choose_serial_device(
    original_device: &str,
    expected_serial: &str,
    candidates: &[String],
) -> Result<String, String> {
    if candidates.iter().any(|device| device == original_device) {
        return Ok(original_device.to_owned());
    }
    match candidates {
        [device] => Ok(device.clone()),
        [] => Err(format!("未找到硬件 SN 为 {expected_serial} 的串口设备")),
        _ => Err(format!(
            "硬件 SN {expected_serial} 对应多个串口接口，请重新选择具体设备"
        )),
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
            serial_number: None,
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
            serial_number: None,
            baud_rate: DEFAULT_BAUD_RATE,
        };
        let windows = SerialSpec {
            device: "COM12".into(),
            display_name: "USB Serial Port".into(),
            serial_number: None,
            baud_rate: DEFAULT_BAUD_RATE,
        };
        let macos = SerialSpec {
            device: "/dev/tty.usbserial-A10K9".into(),
            display_name: "FTDI".into(),
            serial_number: None,
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
            serial_number: Some(" A10LCL3D ".into()),
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
