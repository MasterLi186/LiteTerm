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
    /// Set true by zmodem_send to tell the reader thread to run a ZMODEM transfer.
    pub zmodem_active: Arc<AtomicBool>,
    /// The pending ZMODEM send request, consumed by the reader thread.
    pub zmodem_request: Arc<Mutex<Option<ZmodemSendRequest>>>,
}

/// A ZMODEM upload request handed from the `zmodem_send` command to the SSH
/// reader thread (which owns the channel).
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
