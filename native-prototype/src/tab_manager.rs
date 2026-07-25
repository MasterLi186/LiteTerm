use std::sync::{Arc, Mutex};
use crate::smart_completion::{CompletionSessionKey, CompletionState};
use crate::terminal::TerminalState;
use crate::sidebar::SshConnection;

#[derive(Clone, Debug)]
pub enum TabType {
    Local { shell_path: String },
    Ssh { host: String, port: u16, user: String, label: String },
}

pub struct Tab {
    pub id: String,
    pub label: String,
    pub tab_type: TabType,
    pub terminal: Arc<Mutex<TerminalState>>,
    pub read_thread_started: bool,
    pub completion: CompletionState,
}

pub struct TabManager {
    pub tabs: Vec<Tab>,
    pub active_idx: usize,
    local_counter: usize,
}

impl TabManager {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active_idx: 0,
            local_counter: 0,
        }
    }

    /// Create a new local terminal tab with the given shell, spawn it, and make it active.
    /// Returns (tab_id, terminal Arc) so the caller can start the read_loop.
    pub fn new_local(&mut self, shell: &str, cols: u16, rows: u16) -> (String, Arc<Mutex<TerminalState>>) {
        self.local_counter += 1;
        let id = uuid::Uuid::new_v4().to_string();
        let label = format!("终端 {}", self.local_counter);
        let terminal = Arc::new(Mutex::new(TerminalState::new()));
        let session = CompletionSessionKey::new(1);
        let completion = CompletionState::new(session.clone());

        {
            let mut term = terminal.lock().unwrap();
            term.spawn_shell_with_path(shell, cols, rows, session);
        }

        let tab = Tab {
            id: id.clone(),
            label,
            tab_type: TabType::Local { shell_path: shell.to_string() },
            terminal: terminal.clone(),
            read_thread_started: false,
            completion,
        };
        self.tabs.push(tab);
        self.active_idx = self.tabs.len() - 1;
        (id, terminal)
    }

    /// Create a placeholder tab for an SSH connection (terminal not yet ready).
    pub fn new_ssh_placeholder(&mut self, conn: &SshConnection) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let terminal = Arc::new(Mutex::new(TerminalState::new()));
        let tab = Tab {
            id: id.clone(),
            label: format!("{} (连接中...)", conn.label),
            tab_type: TabType::Ssh {
                host: conn.host.clone(),
                port: conn.port,
                user: conn.user.clone(),
                label: conn.label.clone(),
            },
            terminal,
            read_thread_started: false,
            completion: CompletionState::new(CompletionSessionKey::new(1)),
        };
        self.tabs.push(tab);
        self.active_idx = self.tabs.len() - 1;
        id
    }

    /// Apply SSH handle to a placeholder tab after successful connection.
    /// Returns the terminal Arc so the caller can start the read_loop.
    pub fn apply_ssh(&mut self, tab_id: &str, handle: crate::ssh::SshHandle, cols: u16, rows: u16) -> Option<Arc<Mutex<TerminalState>>> {
        let idx = self.find_by_id(tab_id)?;
        let tab = &mut self.tabs[idx];
        {
            let mut term = tab.terminal.lock().unwrap();
            term.apply_ssh_handle(handle, cols, rows);
        }
        // Update label to remove "(连接中...)"
        if let TabType::Ssh { ref label, .. } = tab.tab_type {
            tab.label = label.clone();
        }
        Some(tab.terminal.clone())
    }

    /// Mark SSH tab as failed
    pub fn ssh_failed(&mut self, tab_id: &str, error: &str) {
        if let Some(idx) = self.find_by_id(tab_id) {
            self.tabs[idx].label = format!("连接失败");
            eprintln!("[TAB] SSH failed for {}: {}", tab_id, error);
        }
    }

    pub fn close(&mut self, idx: usize) {
        if idx >= self.tabs.len() { return; }
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            // Never leave empty — will be handled by caller
            return;
        }
        if self.active_idx >= self.tabs.len() {
            self.active_idx = self.tabs.len() - 1;
        } else if self.active_idx > idx {
            self.active_idx -= 1;
        }
    }

    pub fn active(&self) -> Option<&Tab> {
        self.tabs.get(self.active_idx)
    }

    pub fn active_terminal(&self) -> Option<Arc<Mutex<TerminalState>>> {
        self.active().map(|t| t.terminal.clone())
    }

    pub fn switch_to(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active_idx = idx;
        }
    }

    pub fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active_idx = (self.active_idx + 1) % self.tabs.len();
        }
    }

    pub fn prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active_idx = if self.active_idx == 0 { self.tabs.len() - 1 } else { self.active_idx - 1 };
        }
    }

    pub fn find_by_id(&self, id: &str) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_bash_tab_shares_one_generation_one_session_with_runtime() {
        let mut manager = TabManager::new();
        let (_, terminal) = manager.new_local("bash", 80, 24);
        let terminal = terminal.lock().unwrap();
        let runtime = terminal.local_bash_runtime.as_ref().unwrap();

        assert_eq!(manager.tabs[0].completion.session().generation, 1);
        assert_eq!(manager.tabs[0].completion.session(), runtime.session());
    }

    #[test]
    fn ssh_placeholder_starts_with_generation_one_completion() {
        let connection = SshConnection {
            label: "测试主机".to_owned(),
            host: "127.0.0.1".to_owned(),
            port: 22,
            user: "tester".to_owned(),
            auth: "password".to_owned(),
            key_path: String::new(),
            group: String::new(),
            group_color: [0, 0, 0],
        };
        let mut manager = TabManager::new();

        manager.new_ssh_placeholder(&connection);

        assert_eq!(manager.tabs[0].completion.session().generation, 1);
        assert!(manager.tabs[0]
            .terminal
            .lock()
            .unwrap()
            .local_bash_runtime
            .is_none());
    }
}
