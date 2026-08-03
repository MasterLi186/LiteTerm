use crate::bash_integration::is_bash_path;
use crate::monitor::MonitorKey;
use crate::sidebar::SshConnection;
use crate::smart_completion::{CompletionSessionKey, CompletionState};
use crate::split::{ClosePaneResult, LayoutSnapshot, PaneId, PaneTree, SplitDirection, SplitId};
use crate::terminal::TerminalState;
use egui::Rect;
use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex};

mod model;

pub use model::*;

pub struct TabManager {
    pub tabs: Vec<Tab>,
    pub active_idx: usize,
    local_counter: usize,
}

pub struct SshReconnectPlan {
    pub tab_id: String,
    pub pane_id: PaneId,
    pub params: crate::ssh::ConnectionParams,
    pub session: CompletionSessionKey,
    pub old_terminal: Arc<Mutex<TerminalState>>,
}

pub struct SerialReconnectPlan {
    pub open: SerialOpenPlan,
    pub old_terminal: Arc<Mutex<TerminalState>>,
}

pub struct SerialDisconnectPlan {
    pub pane_id: PaneId,
    pub old_terminal: Arc<Mutex<TerminalState>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialOpenPlan {
    pub tab_id: String,
    pub pane_id: PaneId,
    pub generation: u64,
    pub spec: crate::serial::SerialSpec,
}

pub enum SplitPanePlan {
    Local {
        tab_id: String,
        pane_id: PaneId,
        terminal: Arc<Mutex<TerminalState>>,
        session: CompletionSessionKey,
        history_path: Option<std::path::PathBuf>,
    },
    Ssh {
        tab_id: String,
        pane_id: PaneId,
        params: crate::ssh::ConnectionParams,
        session: CompletionSessionKey,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseActivePaneResult {
    NotTerminal,
    CloseTab,
    Closed {
        tab_id: String,
        pane_id: PaneId,
        active_pane_id: PaneId,
    },
}

fn default_bash_history_path(home: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    let home = home.filter(|path| path.is_absolute())?;
    Some(home.join(".bash_history"))
}

fn shutdown_terminal(terminal: &Arc<Mutex<TerminalState>>) {
    terminal
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .shutdown();
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
    pub fn new_local(
        &mut self,
        shell: &str,
        cols: u16,
        rows: u16,
    ) -> (String, Arc<Mutex<TerminalState>>) {
        self.local_counter += 1;
        let id = uuid::Uuid::new_v4().to_string();
        let label = format!("终端 {}", self.local_counter);
        let terminal = Arc::new(Mutex::new(TerminalState::new()));
        let session = CompletionSessionKey::new(1);
        let mut completion = CompletionState::new(session.clone());
        if is_bash_path(shell) {
            if let Some(path) = default_bash_history_path(dirs::home_dir()) {
                completion.set_history_path(path.to_string_lossy().into());
            }
        }

        {
            let mut term = terminal.lock().unwrap();
            term.spawn_shell_with_path(shell, cols, rows, session);
        }

        let pane = TerminalPane {
            id: id.clone(),
            terminal: terminal.clone(),
            read_thread_started: false,
            completion,
            ssh_connected: false,
            status: PaneStatus::Connected,
            serial_generation: 0,
            search: crate::terminal_search::TerminalSearchState::default(),
        };
        let tab = Tab {
            id: id.clone(),
            label,
            tab_type: TabType::Local {
                shell_path: shell.to_string(),
            },
            layout: TerminalLayout::new(pane),
            remote_monitor_leased: false,
        };
        self.tabs.push(tab);
        self.active_idx = self.tabs.len() - 1;
        (id, terminal)
    }

    /// Try to create a local terminal from an untrusted absolute shell path.
    ///
    /// The manager is updated only after the PTY and child process are fully ready.
    pub fn try_new_local(
        &mut self,
        shell: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(String, Arc<Mutex<TerminalState>>), String> {
        let next_counter = self
            .local_counter
            .checked_add(1)
            .ok_or_else(|| "本地终端计数器已溢出".to_string())?;
        let id = uuid::Uuid::new_v4().to_string();
        let label = format!("终端 {next_counter}");
        let terminal = Arc::new(Mutex::new(TerminalState::new()));
        let session = CompletionSessionKey::new(1);
        let mut completion = CompletionState::new(session.clone());
        if is_bash_path(shell) {
            if let Some(path) = default_bash_history_path(dirs::home_dir()) {
                completion.set_history_path(path.to_string_lossy().into());
            }
        }

        terminal
            .lock()
            .map_err(|_| "本地终端状态锁已损坏".to_string())?
            .try_spawn_shell_with_path(shell, cols, rows, session)?;

        let pane = TerminalPane {
            id: id.clone(),
            terminal: terminal.clone(),
            read_thread_started: false,
            completion,
            ssh_connected: false,
            status: PaneStatus::Connected,
            serial_generation: 0,
            search: crate::terminal_search::TerminalSearchState::default(),
        };
        let tab = Tab {
            id: id.clone(),
            label,
            tab_type: TabType::Local {
                shell_path: shell.to_string(),
            },
            layout: TerminalLayout::new(pane),
            remote_monitor_leased: false,
        };
        self.tabs.push(tab);
        self.local_counter = next_counter;
        self.active_idx = self.tabs.len() - 1;
        Ok((id, terminal))
    }

    /// Create a placeholder tab for an SSH connection (terminal not yet ready).
    pub fn new_ssh_placeholder(&mut self, conn: &SshConnection) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let mut pane = TerminalPane::empty(id.clone());
        pane.status = PaneStatus::Connecting;
        let tab = Tab {
            id: id.clone(),
            label: format!("{} (连接中...)", conn.label),
            tab_type: TabType::Ssh {
                label: conn.label.clone(),
                params: crate::ssh::ConnectionParams::from(conn),
            },
            layout: TerminalLayout::new(pane),
            remote_monitor_leased: false,
        };
        self.tabs.push(tab);
        self.active_idx = self.tabs.len() - 1;
        id
    }

    /// Open the process page for a monitor identity, or activate the existing page.
    ///
    /// Process pages are unique by `MonitorKey`, not by their display label or source SSH tab.
    /// Remote pages own an independent monitor lease so their worker remains alive after the
    /// source terminal closes. Local pages never own a remote lease.
    pub fn open_process(
        &mut self,
        label: impl Into<String>,
        key: MonitorKey,
        params: Option<crate::ssh::ConnectionParams>,
    ) -> String {
        debug_assert!(match (&key, params.as_ref()) {
            (MonitorKey::Local, None) => true,
            (MonitorKey::Remote { .. }, Some(params)) => MonitorKey::from_ssh(params) == key,
            _ => false,
        });
        if let Some(index) = self.tabs.iter().position(
            |tab| matches!(&tab.tab_type, TabType::Process { key: existing, .. } if existing == &key),
        ) {
            self.active_idx = index;
            return self.tabs[index].id.clone();
        }

        let id = uuid::Uuid::new_v4().to_string();
        let label = label.into();
        let remote_monitor_leased = matches!(key, MonitorKey::Remote { .. });
        let tab = Tab {
            id: id.clone(),
            label: label.clone(),
            tab_type: TabType::Process { label, key, params },
            layout: TerminalLayout::new(TerminalPane::empty(id.clone())),
            remote_monitor_leased,
        };
        self.tabs.push(tab);
        self.active_idx = self.tabs.len() - 1;
        id
    }

    pub fn open_network(
        &mut self,
        key: MonitorKey,
        params: Option<crate::ssh::ConnectionParams>,
        initial_iface: Option<String>,
    ) -> String {
        if let Some(index) = self.tabs.iter().position(
            |tab| matches!(&tab.tab_type, TabType::Network { key: existing, .. } if existing == &key),
        ) {
            self.active_idx = index;
            return self.tabs[index].id.clone();
        }
        let id = uuid::Uuid::new_v4().to_string();
        let label = format!("网络 - {}", key.status_text());
        let remote_monitor_leased = matches!(key, MonitorKey::Remote { .. });
        self.tabs.push(Tab {
            id: id.clone(),
            label: label.clone(),
            tab_type: TabType::Network {
                label,
                key,
                params,
                initial_iface,
            },
            layout: TerminalLayout::new(TerminalPane::empty(id.clone())),
            remote_monitor_leased,
        });
        self.active_idx = self.tabs.len() - 1;
        id
    }

    /// Open the singleton settings page, or focus it when it already exists.
    /// Returns `(tab_id, created)` so the caller only resets the editor for a new page.
    pub fn open_settings(&mut self) -> (String, bool) {
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| matches!(tab.tab_type, TabType::Settings))
        {
            self.active_idx = index;
            return (self.tabs[index].id.clone(), false);
        }

        let id = uuid::Uuid::new_v4().to_string();
        self.tabs.push(Tab {
            id: id.clone(),
            label: "设置".into(),
            tab_type: TabType::Settings,
            layout: TerminalLayout::new(TerminalPane::empty(id.clone())),
            remote_monitor_leased: false,
        });
        self.active_idx = self.tabs.len() - 1;
        (id, true)
    }

    pub fn new_serial_placeholder(&mut self, spec: crate::serial::SerialSpec) -> SerialOpenPlan {
        let id = uuid::Uuid::new_v4().to_string();
        let mut pane = TerminalPane::empty(id.clone());
        pane.serial_generation = 1;
        pane.status = PaneStatus::Connecting;
        let tab_label = spec.tab_label();
        self.tabs.push(Tab {
            id: id.clone(),
            label: format!("{tab_label} (打开中...)"),
            tab_type: TabType::Serial { spec: spec.clone() },
            layout: TerminalLayout::new(pane),
            remote_monitor_leased: false,
        });
        self.active_idx = self.tabs.len() - 1;
        SerialOpenPlan {
            tab_id: id.clone(),
            pane_id: id,
            generation: 1,
            spec,
        }
    }

    pub fn new_recording(
        &mut self,
        path: std::path::PathBuf,
        cols: u16,
        rows: u16,
    ) -> (String, Arc<Mutex<TerminalState>>) {
        let id = uuid::Uuid::new_v4().to_string();
        let label = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("回放: {name}"))
            .unwrap_or_else(|| "录屏回放".into());
        let terminal = Arc::new(Mutex::new(TerminalState::new_replay(cols, rows)));
        let mut pane = TerminalPane::empty(id.clone());
        pane.terminal = terminal.clone();
        pane.status = PaneStatus::Connected;
        self.tabs.push(Tab {
            id: id.clone(),
            label,
            tab_type: TabType::Recording { path },
            layout: TerminalLayout::new(pane),
            remote_monitor_leased: false,
        });
        self.active_idx = self.tabs.len() - 1;
        (id, terminal)
    }

    pub fn apply_serial(
        &mut self,
        tab_id: &str,
        pane_id: &str,
        generation: u64,
        handle: crate::serial::SerialHandle,
        cols: u16,
        rows: u16,
    ) -> Option<Arc<Mutex<TerminalState>>> {
        let opened_device = handle.opened_device().to_owned();
        let Some(index) = self.find_by_id(tab_id) else {
            crate::terminal::shutdown_serial_handle(handle);
            return None;
        };
        let tab = &mut self.tabs[index];
        if !matches!(tab.tab_type, TabType::Serial { .. }) {
            crate::terminal::shutdown_serial_handle(handle);
            return None;
        }
        let terminal = {
            let Some(pane) = tab.pane_mut(pane_id) else {
                crate::terminal::shutdown_serial_handle(handle);
                return None;
            };
            if pane.serial_generation != generation || pane.status != PaneStatus::Connecting {
                crate::terminal::shutdown_serial_handle(handle);
                return None;
            }
            pane.terminal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .apply_serial_handle(handle, cols, rows);
            pane.status = PaneStatus::Connected;
            pane.terminal.clone()
        };
        if let TabType::Serial { spec } = &mut tab.tab_type {
            spec.device = opened_device;
            tab.label = spec.tab_label();
        }
        Some(terminal)
    }

    pub fn serial_failed(
        &mut self,
        tab_id: &str,
        pane_id: &str,
        generation: u64,
        error: &str,
    ) -> bool {
        let Some(index) = self.find_by_id(tab_id) else {
            return false;
        };
        let tab = &mut self.tabs[index];
        if !matches!(tab.tab_type, TabType::Serial { .. }) {
            return false;
        }
        let Some(pane) = tab.pane_mut(pane_id) else {
            return false;
        };
        if pane.serial_generation != generation || pane.status != PaneStatus::Connecting {
            return false;
        }
        pane.status = PaneStatus::Failed(error.to_string());
        if let TabType::Serial { spec } = &tab.tab_type {
            tab.label = format!("{} (打开失败)", spec.tab_label());
        }
        true
    }

    pub fn reset_serial_for_reconnect(&mut self, index: usize) -> Option<SerialReconnectPlan> {
        let tab = self.tabs.get_mut(index)?;
        let spec = match &tab.tab_type {
            TabType::Serial { spec } => spec.clone(),
            _ => return None,
        };
        let pane_id = tab.active_pane_id().to_string();
        let next_generation = tab.serial_generation.checked_add(1)?;
        let session = tab.completion.session().successor();
        tab.completion.reset_session(session);
        tab.status = PaneStatus::Connecting;
        tab.serial_generation = next_generation;
        shutdown_terminal(&tab.terminal);
        let old_terminal = std::mem::replace(
            &mut tab.terminal,
            Arc::new(Mutex::new(TerminalState::new())),
        );
        tab.read_thread_started = false;
        tab.label = format!("{} (打开中...)", spec.tab_label());

        Some(SerialReconnectPlan {
            open: SerialOpenPlan {
                tab_id: tab.id.clone(),
                pane_id,
                generation: next_generation,
                spec,
            },
            old_terminal,
        })
    }

    pub fn disconnect_serial(&mut self, index: usize) -> Option<SerialDisconnectPlan> {
        let tab = self.tabs.get_mut(index)?;
        let label = match &tab.tab_type {
            TabType::Serial { spec } => spec.tab_label(),
            _ => return None,
        };
        if tab.status != PaneStatus::Connected {
            return None;
        }

        let pane_id = tab.active_pane_id().to_string();
        let next_generation = tab.serial_generation.checked_add(1)?;
        let session = tab.completion.session().successor();
        tab.completion.reset_session(session);
        tab.status = PaneStatus::Idle;
        tab.serial_generation = next_generation;
        shutdown_terminal(&tab.terminal);
        let old_terminal = std::mem::replace(
            &mut tab.terminal,
            Arc::new(Mutex::new(TerminalState::new())),
        );
        tab.read_thread_started = false;
        tab.label = format!("{label} (已断开)");

        Some(SerialDisconnectPlan {
            pane_id,
            old_terminal,
        })
    }

    pub fn split_active_pane(
        &mut self,
        direction: SplitDirection,
        cols: u16,
        rows: u16,
    ) -> Result<SplitPanePlan, String> {
        let tab = self
            .tabs
            .get_mut(self.active_idx)
            .ok_or_else(|| "当前没有活动标签".to_string())?;
        let target_pane_id = tab.active_pane_id().to_string();
        let pane_id = uuid::Uuid::new_v4().to_string();
        let mut pane = TerminalPane::empty(pane_id.clone());

        let plan = match &tab.tab_type {
            TabType::Local { shell_path } => {
                let session = pane.completion.session().clone();
                if is_bash_path(shell_path) {
                    if let Some(path) = default_bash_history_path(dirs::home_dir()) {
                        pane.completion
                            .set_history_path(path.to_string_lossy().into());
                    }
                }
                pane.terminal
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .spawn_shell_with_path(shell_path, cols, rows, session.clone());
                SplitPanePlan::Local {
                    tab_id: tab.id.clone(),
                    pane_id: pane_id.clone(),
                    terminal: pane.terminal.clone(),
                    session,
                    history_path: pane.completion.history_path().map(std::path::PathBuf::from),
                }
            }
            TabType::Ssh { params, .. } => {
                pane.status = PaneStatus::Connecting;
                SplitPanePlan::Ssh {
                    tab_id: tab.id.clone(),
                    pane_id: pane_id.clone(),
                    params: params.clone(),
                    session: pane.completion.session().clone(),
                }
            }
            TabType::Serial { .. } => return Err("串口终端不支持分屏".into()),
            TabType::Recording { .. } => return Err("录屏回放不支持分屏".into()),
            TabType::Process { .. } | TabType::Network { .. } | TabType::Settings => {
                return Err("当前标签不是终端".into())
            }
        };

        if let Err(pane) = tab.layout.split(&target_pane_id, direction, pane) {
            pane.terminal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .shutdown();
            return Err("活动面板已失效".into());
        }
        Ok(plan)
    }

    pub fn close_active_pane(&mut self) -> CloseActivePaneResult {
        let Some(tab) = self.tabs.get_mut(self.active_idx) else {
            return CloseActivePaneResult::NotTerminal;
        };
        if !tab.tab_type.is_terminal() {
            return CloseActivePaneResult::NotTerminal;
        }
        let pane_id = tab.active_pane_id().to_string();
        match tab.layout.close(&pane_id) {
            CloseLayoutPaneResult::NotFound => CloseActivePaneResult::NotTerminal,
            CloseLayoutPaneResult::LastPane => CloseActivePaneResult::CloseTab,
            CloseLayoutPaneResult::Closed {
                mut removed,
                suggested_active,
            } => {
                removed.completion.cancel_pending_fill();
                shutdown_terminal(&removed.terminal);
                CloseActivePaneResult::Closed {
                    tab_id: tab.id.clone(),
                    pane_id,
                    active_pane_id: suggested_active,
                }
            }
        }
    }

    pub fn set_active_pane(&mut self, tab_id: &str, pane_id: &str) -> bool {
        let Some(index) = self.find_by_id(tab_id) else {
            return false;
        };
        self.tabs[index].set_active_pane(pane_id)
    }

    pub fn find_pane(&self, pane_id: &str) -> Option<(usize, &TerminalPane)> {
        self.tabs
            .iter()
            .enumerate()
            .find_map(|(index, tab)| tab.pane(pane_id).map(|pane| (index, pane)))
    }

    pub fn find_pane_mut(&mut self, pane_id: &str) -> Option<(usize, &mut TerminalPane)> {
        self.tabs
            .iter_mut()
            .enumerate()
            .find_map(|(index, tab)| tab.pane_mut(pane_id).map(|pane| (index, pane)))
    }

    /// Apply SSH handle to a placeholder tab after successful connection.
    /// Returns the terminal Arc so the caller can start the read_loop.
    pub fn apply_ssh(
        &mut self,
        tab_id: &str,
        expected_session: &CompletionSessionKey,
        handle: crate::ssh::SshHandle,
        cols: u16,
        rows: u16,
    ) -> Option<Arc<Mutex<TerminalState>>> {
        self.apply_ssh_pane(tab_id, tab_id, expected_session, handle, cols, rows)
    }

    pub fn apply_ssh_pane(
        &mut self,
        tab_id: &str,
        pane_id: &str,
        expected_session: &CompletionSessionKey,
        handle: crate::ssh::SshHandle,
        cols: u16,
        rows: u16,
    ) -> Option<Arc<Mutex<TerminalState>>> {
        let Some(idx) = self.find_by_id(tab_id) else {
            handle.shutdown();
            return None;
        };
        let tab = &mut self.tabs[idx];
        if !matches!(tab.tab_type, TabType::Ssh { .. }) {
            handle.shutdown();
            return None;
        }
        let terminal = {
            let Some(pane) = tab.pane_mut(pane_id) else {
                handle.shutdown();
                return None;
            };
            if pane.completion.session() != expected_session
                || pane.status != PaneStatus::Connecting
                || handle
                    .bash_runtime
                    .as_ref()
                    .is_some_and(|runtime| runtime.session != *expected_session)
            {
                handle.shutdown();
                return None;
            }
            {
                let mut term = pane.terminal.lock().unwrap();
                term.apply_ssh_handle(handle, cols, rows);
            }
            pane.ssh_connected = true;
            pane.status = PaneStatus::Connected;
            pane.terminal.clone()
        };
        tab.remote_monitor_leased = true;
        // Update label to remove "(连接中...)"
        if let TabType::Ssh { ref label, .. } = tab.tab_type {
            tab.label = label.clone();
        }
        Some(terminal)
    }

    pub fn reset_ssh_for_reconnect(&mut self, index: usize) -> Option<SshReconnectPlan> {
        let tab = self.tabs.get_mut(index)?;
        let params = match &tab.tab_type {
            TabType::Ssh { params, .. } => params.clone(),
            TabType::Local { .. }
            | TabType::Process { .. }
            | TabType::Network { .. }
            | TabType::Serial { .. }
            | TabType::Recording { .. }
            | TabType::Settings => return None,
        };
        let session = tab.completion.session().successor();
        tab.completion.reset_session(session.clone());
        tab.ssh_connected = false;
        tab.status = PaneStatus::Connecting;
        shutdown_terminal(&tab.terminal);
        let old_terminal = std::mem::replace(
            &mut tab.terminal,
            Arc::new(Mutex::new(TerminalState::new())),
        );
        tab.read_thread_started = false;
        tab.label = format!("{} (连接中...)", tab.label.trim_end_matches(" (连接中...)"));
        Some(SshReconnectPlan {
            tab_id: tab.id.clone(),
            pane_id: tab.active_pane_id().to_string(),
            params,
            session,
            old_terminal,
        })
    }

    pub fn ssh_attempt_is_current(
        &self,
        tab_id: &str,
        pane_id: &str,
        expected_session: &CompletionSessionKey,
    ) -> bool {
        self.find_by_id(tab_id)
            .and_then(|index| self.tabs[index].pane(pane_id))
            .is_some_and(|pane| {
                pane.completion.session() == expected_session
                    && pane.status == PaneStatus::Connecting
            })
    }

    /// Mark one SSH pane's current connection attempt as failed.
    pub fn ssh_failed(
        &mut self,
        tab_id: &str,
        pane_id: &str,
        expected_session: &CompletionSessionKey,
        error: &str,
    ) -> bool {
        let Some(idx) = self.find_by_id(tab_id) else {
            return false;
        };
        let tab = &mut self.tabs[idx];
        if !matches!(tab.tab_type, TabType::Ssh { .. }) {
            return false;
        }
        let Some(pane) = tab.pane_mut(pane_id) else {
            return false;
        };
        if pane.completion.session() != expected_session || pane.status != PaneStatus::Connecting {
            return false;
        }
        pane.ssh_connected = false;
        pane.status = PaneStatus::Failed(error.to_string());
        log::warn!("[TAB] SSH failed for {tab_id}/{pane_id}: {error}");
        true
    }

    pub fn close(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        if self.tabs[idx].tab_type.is_terminal() {
            for pane in self.tabs[idx].panes() {
                shutdown_terminal(&pane.terminal);
            }
        }
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

    pub fn close_others(&mut self, keep_idx: usize) {
        if keep_idx >= self.tabs.len() {
            return;
        }
        for (index, tab) in self.tabs.iter().enumerate() {
            if index != keep_idx && tab.tab_type.is_terminal() {
                for pane in tab.panes() {
                    shutdown_terminal(&pane.terminal);
                }
            }
        }
        let kept = self.tabs.remove(keep_idx);
        self.tabs.clear();
        self.tabs.push(kept);
        self.active_idx = 0;
    }

    pub fn active(&self) -> Option<&Tab> {
        self.tabs.get(self.active_idx)
    }

    pub fn active_monitor_key(&self) -> MonitorKey {
        self.active()
            .map(Tab::monitor_key)
            .unwrap_or(MonitorKey::Local)
    }

    pub fn remote_monitor_requirements(&self) -> HashMap<MonitorKey, crate::ssh::ConnectionParams> {
        let mut requirements = HashMap::new();
        for tab in &self.tabs {
            if !tab.remote_monitor_leased {
                continue;
            }
            if let Some(params) = tab.tab_type.remote_params() {
                requirements
                    .entry(tab.monitor_key())
                    .or_insert_with(|| params.clone());
            }
        }
        requirements
    }

    pub fn active_terminal(&self) -> Option<Arc<Mutex<TerminalState>>> {
        self.active()
            .filter(|tab| tab.tab_type.is_terminal())
            .map(|tab| tab.active_pane().terminal.clone())
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
            self.active_idx = if self.active_idx == 0 {
                self.tabs.len() - 1
            } else {
                self.active_idx - 1
            };
        }
    }

    pub fn find_by_id(&self, id: &str) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub fn reorder_by_id(
        &mut self,
        dragged_id: &str,
        target_id: &str,
        placement: TabPlacement,
    ) -> bool {
        if dragged_id == target_id {
            return false;
        }
        let Some(dragged_index) = self.find_by_id(dragged_id) else {
            return false;
        };
        if self.find_by_id(target_id).is_none() {
            return false;
        }
        let active_id = self.active().map(|tab| tab.id.clone());
        let dragged = self.tabs.remove(dragged_index);
        let target_index = self
            .find_by_id(target_id)
            .expect("validated target must remain after removing a different tab");
        let insert_index = match placement {
            TabPlacement::Before => target_index,
            TabPlacement::After => target_index + 1,
        };
        self.tabs.insert(insert_index, dragged);
        if let Some(active_id) = active_id {
            self.active_idx = self.find_by_id(&active_id).unwrap_or(0);
        }
        dragged_index != insert_index
    }

    pub fn rename(&mut self, id: &str, label: &str) -> bool {
        let label = label.trim();
        if label.is_empty() {
            return false;
        }
        let label: String = label.chars().take(MAX_TAB_LABEL_CHARS).collect();
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return false;
        };

        tab.label.clone_from(&label);
        match &mut tab.tab_type {
            TabType::Ssh {
                label: type_label, ..
            }
            | TabType::Process {
                label: type_label, ..
            }
            | TabType::Network {
                label: type_label, ..
            } => type_label.clone_from(&label),
            TabType::Local { .. }
            | TabType::Serial { .. }
            | TabType::Recording { .. }
            | TabType::Settings => {}
        }
        true
    }

    /// 将全部标签页终端网格调整为给定尺寸。
    /// 使用 `PoisonError::into_inner` 恢复 poisoned lock，避免单一锁毒化中断全部 resize。
    pub fn resize_all(&mut self, cols: u16, rows: u16) {
        for tab in self.tabs.iter().filter(|tab| tab.tab_type.is_terminal()) {
            for pane in tab.panes() {
                let mut terminal = pane
                    .terminal
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                terminal.resize(cols, rows);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }
}

#[cfg(test)]
#[path = "tab_manager/tests.rs"]
mod tests;
