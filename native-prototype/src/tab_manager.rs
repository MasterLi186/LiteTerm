use crate::bash_integration::is_bash_path;
use crate::monitor::MonitorKey;
use crate::sidebar::SshConnection;
use crate::smart_completion::{CompletionSessionKey, CompletionState};
use crate::terminal::TerminalState;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub enum TabType {
    Local {
        shell_path: String,
    },
    Ssh {
        label: String,
        params: crate::ssh::ConnectionParams,
    },
}

impl std::fmt::Debug for TabType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local { shell_path } => formatter
                .debug_struct("Local")
                .field("shell_path", shell_path)
                .finish(),
            Self::Ssh { label, params } => formatter
                .debug_struct("Ssh")
                .field("label", label)
                .field("user", &params.user)
                .field("host", &params.host)
                .field("port", &params.port)
                .finish_non_exhaustive(),
        }
    }
}

pub struct Tab {
    pub id: String,
    pub label: String,
    pub tab_type: TabType,
    pub terminal: Arc<Mutex<TerminalState>>,
    pub read_thread_started: bool,
    pub completion: CompletionState,
}

impl Tab {
    pub fn monitor_key(&self) -> MonitorKey {
        match &self.tab_type {
            TabType::Local { .. } => MonitorKey::Local,
            TabType::Ssh { params, .. } => MonitorKey::from_ssh(params),
        }
    }
}

pub struct TabManager {
    pub tabs: Vec<Tab>,
    pub active_idx: usize,
    local_counter: usize,
}

pub struct SshReconnectPlan {
    pub tab_id: String,
    pub params: crate::ssh::ConnectionParams,
    pub session: CompletionSessionKey,
    pub old_terminal: Arc<Mutex<TerminalState>>,
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

        let tab = Tab {
            id: id.clone(),
            label,
            tab_type: TabType::Local {
                shell_path: shell.to_string(),
            },
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
                label: conn.label.clone(),
                params: crate::ssh::ConnectionParams::from(conn),
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
    pub fn apply_ssh(
        &mut self,
        tab_id: &str,
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
        if tab.completion.session() != expected_session
            || handle
                .bash_runtime
                .as_ref()
                .is_some_and(|runtime| runtime.session != *expected_session)
        {
            handle.shutdown();
            return None;
        }
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

    pub fn reset_ssh_for_reconnect(&mut self, index: usize) -> Option<SshReconnectPlan> {
        let tab = self.tabs.get_mut(index)?;
        let params = match &tab.tab_type {
            TabType::Ssh { params, .. } => params.clone(),
            TabType::Local { .. } => return None,
        };
        let session = tab.completion.session().successor();
        tab.completion.reset_session(session.clone());
        shutdown_terminal(&tab.terminal);
        let old_terminal = std::mem::replace(
            &mut tab.terminal,
            Arc::new(Mutex::new(TerminalState::new())),
        );
        tab.read_thread_started = false;
        tab.label = format!("{} (连接中...)", tab.label.trim_end_matches(" (连接中...)"));
        Some(SshReconnectPlan {
            tab_id: tab.id.clone(),
            params,
            session,
            old_terminal,
        })
    }

    /// Mark SSH tab as failed
    pub fn ssh_failed(&mut self, tab_id: &str, error: &str) {
        if let Some(idx) = self.find_by_id(tab_id) {
            self.tabs[idx].label = format!("连接失败");
            eprintln!("[TAB] SSH failed for {}: {}", tab_id, error);
        }
    }

    pub fn close(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        shutdown_terminal(&self.tabs[idx].terminal);
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
            if index != keep_idx {
                shutdown_terminal(&tab.terminal);
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
    use crate::bash_integration::RemoteBashRuntime;
    use crate::monitor::MonitorKey;
    use std::io::{self, Read};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    fn test_ssh_connection() -> SshConnection {
        SshConnection {
            label: "测试".into(),
            host: "127.0.0.1".into(),
            port: 22,
            user: "test".into(),
            auth: "key".into(),
            key_path: String::new(),
            password: String::new(),
            group: String::new(),
            group_color: [0, 0, 0],
        }
    }

    fn test_ssh_connection_for(host: &str, user: &str, port: u16) -> SshConnection {
        SshConnection {
            label: format!("{user}@{host}"),
            host: host.into(),
            port,
            user: user.into(),
            auth: "key".into(),
            key_path: String::new(),
            password: String::new(),
            group: String::new(),
            group_color: [0, 0, 0],
        }
    }

    fn test_ssh_handle(
        bash_runtime: Option<RemoteBashRuntime>,
    ) -> (crate::ssh::SshHandle, mpsc::Receiver<()>) {
        let (write_tx, _write_rx) = mpsc::channel();
        let (resize_tx, _resize_rx) = mpsc::channel();
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let (_io_done_tx, io_done_rx) = mpsc::channel();
        (
            crate::ssh::SshHandle {
                reader: Box::new(std::io::empty()),
                write_tx,
                resize_tx,
                shutdown_tx,
                io_done_rx,
                bash_runtime,
            },
            shutdown_rx,
        )
    }

    struct ReadStarted<R> {
        reader: R,
        started_tx: Option<mpsc::Sender<()>>,
    }

    impl<R: Read> Read for ReadStarted<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if let Some(started_tx) = self.started_tx.take() {
                let _ = started_tx.send(());
            }
            self.reader.read(buffer)
        }
    }

    struct BlockingSshProbe {
        terminal: Arc<Mutex<TerminalState>>,
        shutdown_seen_rx: mpsc::Receiver<()>,
        release_worker_tx: mpsc::Sender<()>,
        read_done_rx: mpsc::Receiver<()>,
        worker_thread: thread::JoinHandle<()>,
        read_thread: thread::JoinHandle<()>,
    }

    impl BlockingSshProbe {
        fn wait_for_shutdown(&self) -> bool {
            let shutdown_requested = self
                .shutdown_seen_rx
                .recv_timeout(Duration::from_secs(1))
                .is_ok();
            if !shutdown_requested {
                self.terminal.lock().unwrap().shutdown();
                self.shutdown_seen_rx
                    .recv_timeout(Duration::from_secs(1))
                    .expect("测试清理必须送达 SSH shutdown");
            }
            shutdown_requested
        }

        fn release_and_wait(self) {
            let _ = self.release_worker_tx.send(());
            self.read_done_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("SSH pipe 关闭后 read_loop 必须有界退出");
            self.worker_thread.join().unwrap();
            self.read_thread.join().unwrap();
        }
    }

    fn add_blocked_ssh_tab(manager: &mut TabManager) -> BlockingSshProbe {
        let tab_id = manager.new_ssh_placeholder(&test_ssh_connection());
        let session = manager.tabs[0].completion.session().clone();
        let (pipe_read, pipe_write) = os_pipe::pipe().unwrap();
        let (read_started_tx, read_started_rx) = mpsc::channel();
        let (write_tx, write_rx) = mpsc::channel();
        let (resize_tx, resize_rx) = mpsc::channel();
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let (io_done_tx, io_done_rx) = mpsc::channel();
        let (shutdown_seen_tx, shutdown_seen_rx) = mpsc::channel();
        let (release_worker_tx, release_worker_rx) = mpsc::channel();
        let handle = crate::ssh::SshHandle {
            reader: Box::new(ReadStarted {
                reader: pipe_read,
                started_tx: Some(read_started_tx),
            }),
            write_tx,
            resize_tx,
            shutdown_tx,
            io_done_rx,
            bash_runtime: None,
        };
        let terminal = manager
            .apply_ssh(&tab_id, &session, handle, 80, 24)
            .unwrap();
        let read_terminal = terminal.clone();
        let (read_done_tx, read_done_rx) = mpsc::channel();
        let read_thread = thread::spawn(move || {
            crate::terminal::read_loop(read_terminal, || {}, |_| {});
            let _ = read_done_tx.send(());
        });
        let worker_thread = thread::spawn(move || {
            let _write_rx = write_rx;
            let _resize_rx = resize_rx;
            shutdown_rx.recv().unwrap();
            let _ = shutdown_seen_tx.send(());
            let _ = release_worker_rx.recv();
            drop(pipe_write);
            let _ = io_done_tx.send(());
        });
        read_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("read_loop 必须先进入阻塞 pipe read");

        BlockingSshProbe {
            terminal,
            shutdown_seen_rx,
            release_worker_tx,
            read_done_rx,
            worker_thread,
            read_thread,
        }
    }

    #[test]
    fn close_shuts_down_arc_held_ssh_before_removing_tab() {
        let mut manager = TabManager::new();
        let probe = add_blocked_ssh_tab(&mut manager);
        let (close_done_tx, close_done_rx) = mpsc::channel();
        let close_thread = thread::spawn(move || {
            manager.close(0);
            let _ = close_done_tx.send(());
        });

        let shutdown_requested = probe.wait_for_shutdown();
        close_done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("close 不得等待 SSH worker 或 read_loop");
        probe.release_and_wait();
        close_thread.join().unwrap();

        assert!(shutdown_requested);
    }

    #[test]
    fn close_others_shuts_down_removed_arc_held_ssh_tabs() {
        let mut manager = TabManager::new();
        let probe = add_blocked_ssh_tab(&mut manager);
        manager.new_ssh_placeholder(&test_ssh_connection_for("keep.example", "keeper", 22));
        let (close_done_tx, close_done_rx) = mpsc::channel();
        let close_thread = thread::spawn(move || {
            manager.close_others(1);
            let _ = close_done_tx.send(());
        });

        let shutdown_requested = probe.wait_for_shutdown();
        close_done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("close_others 不得等待 SSH worker 或 read_loop");
        probe.release_and_wait();
        close_thread.join().unwrap();

        assert!(shutdown_requested);
    }

    #[test]
    fn reconnect_reset_shuts_down_replaced_arc_held_ssh() {
        let mut manager = TabManager::new();
        let probe = add_blocked_ssh_tab(&mut manager);
        let (reset_done_tx, reset_done_rx) = mpsc::channel();
        let reset_thread = thread::spawn(move || {
            let plan = manager.reset_ssh_for_reconnect(0);
            let _ = reset_done_tx.send(plan.is_some());
        });

        let shutdown_requested = probe.wait_for_shutdown();
        assert!(reset_done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("reset 不得等待 SSH worker 或 read_loop"));
        probe.release_and_wait();
        reset_thread.join().unwrap();

        assert!(shutdown_requested);
    }

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
    fn local_bash_tab_starts_with_an_absolute_default_history_path() {
        let mut manager = TabManager::new();
        manager.new_local("bash", 80, 24);

        let path = std::path::Path::new(manager.tabs[0].completion.history_path().unwrap());
        assert!(path.is_absolute());
        assert!(path.ends_with(".bash_history"));
    }

    #[test]
    fn default_bash_history_path_requires_an_absolute_home() {
        assert_eq!(
            super::default_bash_history_path(Some(std::path::PathBuf::from("relative-home"))),
            None
        );
        assert_eq!(super::default_bash_history_path(None), None);

        let absolute =
            super::default_bash_history_path(Some(std::path::PathBuf::from("/home/test-user")))
                .unwrap();
        assert_eq!(
            std::path::Path::new(&absolute),
            std::path::Path::new("/home/test-user/.bash_history")
        );
        assert!(std::path::Path::new(&absolute).is_absolute());
    }

    #[test]
    fn non_bash_local_tab_does_not_get_a_bash_history_path() {
        let mut manager = TabManager::new();
        manager.new_local("fish", 80, 24);

        assert_eq!(manager.tabs[0].completion.history_path(), None);
    }

    #[test]
    fn ssh_placeholder_starts_with_generation_one_completion() {
        let connection = test_ssh_connection();
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

    #[test]
    fn ssh_reconnect_keeps_tab_id_and_rotates_session() {
        let mut manager = TabManager::new();
        let connection = test_ssh_connection();
        let tab_id = manager.new_ssh_placeholder(&connection);
        manager.tabs[0]
            .completion
            .replace_history(vec!["old history".into()]);
        let previous = manager.tabs[0].completion.session().clone();

        let plan = manager.reset_ssh_for_reconnect(0).unwrap();

        assert_eq!(plan.tab_id, tab_id);
        assert_eq!(plan.session.generation, previous.generation + 1);
        assert_ne!(plan.session.token(), previous.token());
        assert!(manager.tabs[0].completion.history().is_empty());
        assert_eq!(manager.tabs[0].completion.session(), &plan.session);
    }

    #[test]
    fn apply_ssh_rejects_mismatched_integrated_runtime_and_shuts_it_down() {
        let mut manager = TabManager::new();
        let tab_id = manager.new_ssh_placeholder(&test_ssh_connection());
        let current = manager.tabs[0].completion.session().clone();
        let stale = current.successor();
        let runtime = RemoteBashRuntime {
            session: stale,
            bash_path: "/bin/bash".into(),
            rc_path: "/tmp/stale.rc".into(),
            candidate_path: "/tmp/stale.candidate".into(),
            widget_sequence: "\x1b[777;1~".into(),
        };
        let (handle, shutdown_rx) = test_ssh_handle(Some(runtime));

        assert!(manager
            .apply_ssh(&tab_id, &current, handle, 80, 24)
            .is_none());
        assert!(shutdown_rx.try_recv().is_ok());
        assert_eq!(manager.tabs[0].completion.session(), &current);
    }

    #[test]
    fn apply_ssh_accepts_plain_shell_fallback_without_runtime() {
        let mut manager = TabManager::new();
        let tab_id = manager.new_ssh_placeholder(&test_ssh_connection());
        let session = manager.tabs[0].completion.session().clone();
        let (handle, _shutdown_rx) = test_ssh_handle(None);

        assert!(manager
            .apply_ssh(&tab_id, &session, handle, 80, 24)
            .is_some());
        assert_eq!(manager.tabs[0].label, "测试");
    }

    #[test]
    fn late_plain_shell_result_cannot_replace_newer_reconnect() {
        let mut manager = TabManager::new();
        let tab_id = manager.new_ssh_placeholder(&test_ssh_connection());
        let old_session = manager.tabs[0].completion.session().clone();
        let reconnect = manager.reset_ssh_for_reconnect(0).unwrap();
        let (new_handle, _new_shutdown_rx) = test_ssh_handle(None);
        let (old_handle, old_shutdown_rx) = test_ssh_handle(None);

        assert!(manager
            .apply_ssh(&tab_id, &reconnect.session, new_handle, 80, 24)
            .is_some());
        assert!(manager
            .apply_ssh(&tab_id, &old_session, old_handle, 80, 24)
            .is_none());

        assert!(old_shutdown_rx.try_recv().is_ok());
        assert_eq!(
            manager.tabs[0].completion.session(),
            &reconnect.session,
            "晚到的旧结果不得回退当前会话"
        );
    }

    #[test]
    fn ssh_placeholders_have_distinct_tab_ids_but_share_monitor_key() {
        let mut manager = TabManager::new();
        let connection = test_ssh_connection();

        let first_id = manager.new_ssh_placeholder(&connection);
        let second_id = manager.new_ssh_placeholder(&connection);

        assert_ne!(first_id, second_id);
        assert_eq!(manager.tabs[0].monitor_key(), manager.tabs[1].monitor_key());
        assert_eq!(
            manager.active_monitor_key(),
            MonitorKey::remote("test", "127.0.0.1", 22)
        );
    }

    #[test]
    fn local_and_empty_managers_use_the_local_monitor_key() {
        let mut manager = TabManager::new();

        assert_eq!(manager.active_monitor_key(), MonitorKey::Local);

        manager.new_local("sh", 80, 24);
        assert_eq!(manager.tabs[0].monitor_key(), MonitorKey::Local);
        assert_eq!(manager.active_monitor_key(), MonitorKey::Local);
    }

    #[test]
    fn tab_type_debug_does_not_expose_ssh_credentials() {
        let mut connection = test_ssh_connection();
        connection.password = "password-sentinel".into();
        connection.key_path = "key-path-sentinel".into();
        let mut manager = TabManager::new();
        manager.new_ssh_placeholder(&connection);

        let debug = format!("{:?}", manager.tabs[0].tab_type);

        assert!(!debug.contains("password-sentinel"));
        assert!(!debug.contains("key-path-sentinel"));
    }

    #[test]
    fn active_monitor_key_follows_the_active_tab_and_handles_invalid_indices() {
        let mut manager = TabManager::new();
        let ssh_a = test_ssh_connection_for("alpha.example", "alice", 22);
        let ssh_b = test_ssh_connection_for("beta.example", "bob", 2200);

        manager.new_local("sh", 80, 24);
        manager.new_ssh_placeholder(&ssh_a);
        manager.new_ssh_placeholder(&ssh_b);

        manager.switch_to(0);
        assert_eq!(manager.active_monitor_key(), MonitorKey::Local);
        manager.switch_to(1);
        assert_eq!(
            manager.active_monitor_key(),
            MonitorKey::remote("alice", "alpha.example", 22)
        );
        manager.switch_to(2);
        assert_eq!(
            manager.active_monitor_key(),
            MonitorKey::remote("bob", "beta.example", 2200)
        );
        manager.switch_to(99);
        assert_eq!(
            manager.active_monitor_key(),
            MonitorKey::remote("bob", "beta.example", 2200)
        );

        let empty = TabManager::new();
        assert_eq!(empty.active_monitor_key(), MonitorKey::Local);
    }
}
