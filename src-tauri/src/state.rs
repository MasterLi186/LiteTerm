use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// 终端输出环形缓冲区，供 HTTP API 增量拉取会话输出使用。
///
/// 固定容量的字节环形缓冲区：写入超过容量时覆盖最旧数据，
/// `write_pos` 记录累计写入的总字节数（单调递增），作为游标基准。
pub struct TerminalOutputBuffer {
    buf: Vec<u8>,
    capacity: usize,
    /// 下一次写入的位置（buf 内的物理偏移）。
    head: usize,
    /// 累计写入的总字节数，单调递增，用作对外游标。
    write_pos: u64,
}

impl TerminalOutputBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0u8; capacity],
            capacity,
            head: 0,
            write_pos: 0,
        }
    }

    /// 写入数据到环形缓冲区，容量不足时自动覆盖最旧字节。
    pub fn write(&mut self, data: &[u8]) {
        for &byte in data {
            self.buf[self.head] = byte;
            self.head = (self.head + 1) % self.capacity;
        }
        self.write_pos += data.len() as u64;
    }

    /// 从 cursor 位置读取增量数据。返回 (data, new_cursor, truncated)。
    /// truncated 为 true 表示请求的起点已被覆盖，实际返回的数据从缓冲区最旧位置开始。
    pub fn read_from(&self, cursor: u64) -> (Vec<u8>, u64, bool) {
        if cursor >= self.write_pos {
            return (Vec::new(), self.write_pos, false);
        }

        let available = self.write_pos - cursor;
        let truncated = available > self.capacity as u64;

        let read_start = if truncated {
            self.write_pos - self.capacity as u64
        } else {
            cursor
        };

        let len = (self.write_pos - read_start) as usize;
        let mut data = Vec::with_capacity(len);

        let start_offset = self.capacity - (self.write_pos - read_start) as usize % self.capacity;
        let start_idx = (self.head + start_offset) % self.capacity;

        for i in 0..len {
            data.push(self.buf[(start_idx + i) % self.capacity]);
        }

        (data, self.write_pos, truncated)
    }
}

/// 标签页信息，供 HTTP API 查询当前打开的标签列表使用。
#[derive(Clone, serde::Serialize)]
pub struct TabInfo {
    pub id: String,
    pub label: String,
    pub tab_type: String,
}

pub struct AppState {
    pub sessions: Mutex<HashMap<String, ManagedSession>>,
    pub local_terminals: Mutex<HashMap<String, LocalTerminal>>,
    pub connections: Mutex<crate::config::connections::ConnectionStore>,
    pub settings: Mutex<crate::config::settings::Settings>,
    pub sftp_sessions: Mutex<HashMap<String, crate::commands::sftp::SftpHandle>>,
    pub tunnels: Mutex<HashMap<String, crate::commands::tunnel::TunnelHandle>>,
    pub recordings: Mutex<HashMap<String, crate::commands::recording::Recording>>,
    pub transfer_cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// 各会话的终端输出环形缓冲区，reader 线程克隆 Arc 后写入自己所在线程。
    pub output_buffers: Arc<Mutex<HashMap<String, TerminalOutputBuffer>>>,
    /// 当前打开标签页的注册表，供 HTTP API 查询标签列表。
    pub tab_registry: Mutex<HashMap<String, TabInfo>>,
}

pub struct ManagedSession {
    pub id: String,
    pub label: String,
    pub input_tx: std::sync::mpsc::Sender<Vec<u8>>,
    pub resize_tx: std::sync::mpsc::Sender<(u32, u32)>,
    pub monitor_stop: Arc<AtomicBool>,
    pub sftp_request_tx: std::sync::mpsc::Sender<SftpRequest>,
    /// 终端 shell 当前工作目录，由 reader 线程解析 OSC7 更新（每会话独立）。
    pub osc7_cwd: Arc<Mutex<Option<String>>>,
    /// run_zmodem_upload 置为 true，通知 reader 线程执行一次 ZMODEM 传输。
    pub zmodem_active: Arc<AtomicBool>,
    /// 待处理的 ZMODEM 发送请求，由 reader 线程取走执行。
    pub zmodem_request: Arc<Mutex<Option<ZmodemSendRequest>>>,
}

/// 由 run_zmodem_upload 交给 SSH reader 线程（独占 channel）的 ZMODEM 上传请求。
pub struct ZmodemSendRequest {
    pub files: Vec<crate::core::zmodem::sender::FileInfo>,
    pub result_tx: std::sync::mpsc::Sender<Result<(), String>>,
    pub cancel: Arc<AtomicBool>,
}

pub struct LocalTerminal {
    pub id: String,
    pub input_tx: std::sync::mpsc::Sender<Vec<u8>>,
    pub resize_tx: std::sync::mpsc::Sender<(u32, u32)>,
    pub stop: Arc<AtomicBool>,
}

pub enum SftpRequest {
    ListDir {
        path: String,
        reply: std::sync::mpsc::Sender<Result<Vec<crate::core::sftp::SftpEntry>, String>>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_empty_read() {
        let buf = TerminalOutputBuffer::new(1024);
        let (data, cursor, truncated) = buf.read_from(0);
        assert!(data.is_empty());
        assert_eq!(cursor, 0);
        assert!(!truncated);
    }

    #[test]
    fn test_buffer_write_and_read() {
        let mut buf = TerminalOutputBuffer::new(1024);
        buf.write(b"hello");
        let (data, cursor, truncated) = buf.read_from(0);
        assert_eq!(data, b"hello");
        assert_eq!(cursor, 5);
        assert!(!truncated);
    }

    #[test]
    fn test_buffer_incremental_read() {
        let mut buf = TerminalOutputBuffer::new(1024);
        buf.write(b"ab");
        buf.write(b"cd");
        let (data, cursor, truncated) = buf.read_from(2);
        assert_eq!(data, b"cd");
        assert_eq!(cursor, 4);
        assert!(!truncated);
    }

    #[test]
    fn test_buffer_no_new_data() {
        let mut buf = TerminalOutputBuffer::new(1024);
        buf.write(b"hello");
        let (data, cursor, _) = buf.read_from(5);
        assert!(data.is_empty());
        assert_eq!(cursor, 5);
    }

    #[test]
    fn test_buffer_wrap_around() {
        let mut buf = TerminalOutputBuffer::new(8);
        buf.write(b"12345678"); // 填满
        buf.write(b"ab");       // 覆盖前 2 字节
        let (data, cursor, truncated) = buf.read_from(0);
        // cursor=0 太旧，truncated
        assert!(truncated);
        assert_eq!(cursor, 10);
        assert_eq!(data, b"345678ab");
    }

    #[test]
    fn test_buffer_cursor_too_old() {
        let mut buf = TerminalOutputBuffer::new(8);
        for i in 0..20u8 {
            buf.write(&[i]);
        }
        let (data, cursor, truncated) = buf.read_from(5);
        assert!(truncated);
        assert_eq!(cursor, 20);
        assert_eq!(data.len(), 8);
    }
}
