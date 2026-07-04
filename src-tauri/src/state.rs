use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub sessions: Mutex<HashMap<String, ManagedSession>>,
    pub local_terminals: Mutex<HashMap<String, LocalTerminal>>,
    pub connections: Mutex<crate::config::connections::ConnectionStore>,
    pub settings: Mutex<crate::config::settings::Settings>,
    pub sftp_sessions: Mutex<HashMap<String, crate::commands::sftp::SftpHandle>>,
    pub tunnels: Mutex<HashMap<String, crate::commands::tunnel::TunnelHandle>>,
    pub recordings: Mutex<HashMap<String, crate::commands::recording::Recording>>,
    pub transfer_cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
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
