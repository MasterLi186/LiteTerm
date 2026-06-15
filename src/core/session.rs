use std::sync::{Arc, Mutex};

/// Current status of an SSH session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Disconnected,
    Connecting,
    Connected,
}

/// Events emitted during an SSH session lifecycle.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    Connecting,
    Connected,
    DataReceived(Vec<u8>),
    Disconnected(String),
    Error(String),
}

/// Tracks the current state of an SSH session.
#[derive(Debug)]
pub struct SessionState {
    status: SessionStatus,
    last_error: Option<String>,
}

impl SessionState {
    /// Create a new session state in the Disconnected status.
    pub fn new() -> Self {
        Self {
            status: SessionStatus::Disconnected,
            last_error: None,
        }
    }

    /// Return the current status.
    pub fn status(&self) -> &SessionStatus {
        &self.status
    }

    /// Transition to the Connecting status.
    pub fn set_connecting(&mut self) {
        self.status = SessionStatus::Connecting;
        self.last_error = None;
    }

    /// Transition to the Connected status.
    pub fn set_connected(&mut self) {
        self.status = SessionStatus::Connected;
        self.last_error = None;
    }

    /// Transition to the Disconnected status with a reason.
    pub fn set_disconnected(&mut self, reason: &str) {
        self.status = SessionStatus::Disconnected;
        self.last_error = Some(reason.to_string());
    }

    /// Return the last error message, if any.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

impl Default for SessionState {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe shared session state.
pub type SharedSessionState = Arc<Mutex<SessionState>>;

/// Create a new shared session state.
pub fn new_shared_state() -> SharedSessionState {
    Arc::new(Mutex::new(SessionState::new()))
}
