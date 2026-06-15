use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub sessions: Mutex<HashMap<String, ManagedSession>>,
    pub local_terminals: Mutex<HashMap<String, LocalTerminal>>,
    pub connections: Mutex<crate::config::connections::ConnectionStore>,
    pub settings: Mutex<crate::config::settings::Settings>,
    pub sftp_sessions: Mutex<HashMap<String, crate::commands::sftp::SftpHandle>>,
}

pub struct ManagedSession {
    pub id: String,
    pub label: String,
    pub input_tx: std::sync::mpsc::Sender<Vec<u8>>,
    pub resize_tx: std::sync::mpsc::Sender<(u32, u32)>,
    pub monitor_stop: Arc<AtomicBool>,
    pub sftp_request_tx: std::sync::mpsc::Sender<SftpRequest>,
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
