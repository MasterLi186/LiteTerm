use super::*;

pub const MAX_TAB_LABEL_CHARS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabPlacement {
    Before,
    After,
}

#[derive(Clone)]
pub enum TabType {
    Local {
        shell_path: String,
    },
    Ssh {
        label: String,
        params: crate::ssh::ConnectionParams,
    },
    Process {
        label: String,
        key: MonitorKey,
        params: Option<crate::ssh::ConnectionParams>,
    },
    Network {
        label: String,
        key: MonitorKey,
        params: Option<crate::ssh::ConnectionParams>,
        initial_iface: Option<String>,
    },
    Serial {
        spec: crate::serial::SerialSpec,
    },
    Recording {
        path: std::path::PathBuf,
    },
    Settings,
}

impl TabType {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Local { .. } | Self::Ssh { .. } | Self::Serial { .. } | Self::Recording { .. }
        )
    }

    pub fn remote_params(&self) -> Option<&crate::ssh::ConnectionParams> {
        match self {
            Self::Ssh { params, .. } => Some(params),
            Self::Process { params, .. } | Self::Network { params, .. } => params.as_ref(),
            Self::Local { .. } | Self::Serial { .. } | Self::Recording { .. } | Self::Settings => {
                None
            }
        }
    }

    pub fn zmodem_capability(&self) -> crate::zmodem::runtime::RuntimeCapability {
        match self {
            Self::Local { .. } => crate::zmodem::runtime::RuntimeCapability::Local,
            Self::Ssh { .. } => crate::zmodem::runtime::RuntimeCapability::DirectSsh,
            Self::Serial { .. }
            | Self::Recording { .. }
            | Self::Process { .. }
            | Self::Network { .. }
            | Self::Settings => crate::zmodem::runtime::RuntimeCapability::SerialDisabled,
        }
    }
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
            Self::Process { label, key, .. } => formatter
                .debug_struct("Process")
                .field("label", label)
                .field("key", key)
                .finish_non_exhaustive(),
            Self::Network {
                label,
                key,
                initial_iface,
                ..
            } => formatter
                .debug_struct("Network")
                .field("label", label)
                .field("key", key)
                .field("initial_iface", initial_iface)
                .finish_non_exhaustive(),
            Self::Serial { spec } => formatter.debug_tuple("Serial").field(spec).finish(),
            Self::Recording { path } => formatter
                .debug_struct("Recording")
                .field("path", path)
                .finish(),
            Self::Settings => formatter.write_str("Settings"),
        }
    }
}

pub struct TerminalPane {
    pub(super) id: PaneId,
    pub terminal: Arc<Mutex<TerminalState>>,
    pub read_thread_started: bool,
    pub completion: CompletionState,
    pub ssh_connected: bool,
    pub status: PaneStatus,
    pub serial_generation: u64,
    pub search: crate::terminal_search::TerminalSearchState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneStatus {
    Idle,
    Connecting,
    Connected,
    Failed(String),
}

impl TerminalPane {
    pub(super) fn empty(id: PaneId) -> Self {
        Self {
            id,
            terminal: Arc::new(Mutex::new(TerminalState::new())),
            read_thread_started: false,
            completion: CompletionState::new(CompletionSessionKey::new(1)),
            ssh_connected: false,
            status: PaneStatus::Idle,
            serial_generation: 0,
            search: crate::terminal_search::TerminalSearchState::default(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

pub struct TerminalLayout {
    tree: PaneTree,
    active_pane_id: PaneId,
    panes: HashMap<PaneId, TerminalPane>,
}

pub enum CloseLayoutPaneResult {
    NotFound,
    LastPane,
    Closed {
        removed: TerminalPane,
        suggested_active: PaneId,
    },
}

impl TerminalLayout {
    pub(super) fn new(pane: TerminalPane) -> Self {
        let pane_id = pane.id.clone();
        let mut panes = HashMap::new();
        panes.insert(pane_id.clone(), pane);
        let layout = Self {
            tree: PaneTree::new(pane_id.clone()),
            active_pane_id: pane_id,
            panes,
        };
        layout.debug_assert_invariants();
        layout
    }

    pub fn tree(&self) -> &PaneTree {
        &self.tree
    }

    pub fn layout(&self, rect: Rect) -> LayoutSnapshot {
        self.tree.layout(rect)
    }

    pub fn active_pane_id(&self) -> &str {
        &self.active_pane_id
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &TerminalPane> {
        self.panes.values()
    }

    pub fn get(&self, pane_id: &str) -> Option<&TerminalPane> {
        self.panes.get(pane_id)
    }

    pub fn get_mut(&mut self, pane_id: &str) -> Option<&mut TerminalPane> {
        self.panes.get_mut(pane_id)
    }

    pub fn len(&self) -> usize {
        self.panes.len()
    }

    pub fn set_active(&mut self, pane_id: &str) -> bool {
        if !self.panes.contains_key(pane_id) {
            return false;
        }
        self.active_pane_id.clear();
        self.active_pane_id.push_str(pane_id);
        self.debug_assert_invariants();
        true
    }

    pub fn split(
        &mut self,
        target_pane_id: &str,
        direction: SplitDirection,
        pane: TerminalPane,
    ) -> Result<(), TerminalPane> {
        if self.panes.contains_key(&pane.id) || !self.panes.contains_key(target_pane_id) {
            return Err(pane);
        }
        let mut next_tree = self.tree.clone();
        if !next_tree.split(target_pane_id, direction, pane.id.clone()) {
            return Err(pane);
        }
        self.active_pane_id.clone_from(&pane.id);
        self.panes.insert(pane.id.clone(), pane);
        self.tree = next_tree;
        self.debug_assert_invariants();
        Ok(())
    }

    pub fn close(&mut self, pane_id: &str) -> CloseLayoutPaneResult {
        if !self.panes.contains_key(pane_id) {
            return CloseLayoutPaneResult::NotFound;
        }
        let mut next_tree = self.tree.clone();
        match next_tree.close(pane_id) {
            ClosePaneResult::NotFound => CloseLayoutPaneResult::NotFound,
            ClosePaneResult::LastPane => CloseLayoutPaneResult::LastPane,
            ClosePaneResult::Closed { suggested_active } => {
                let Some(removed) = self.panes.remove(pane_id) else {
                    return CloseLayoutPaneResult::NotFound;
                };
                if !self.panes.contains_key(&suggested_active) {
                    self.panes.insert(removed.id.clone(), removed);
                    return CloseLayoutPaneResult::NotFound;
                }
                self.tree = next_tree;
                self.active_pane_id.clone_from(&suggested_active);
                self.debug_assert_invariants();
                CloseLayoutPaneResult::Closed {
                    removed,
                    suggested_active,
                }
            }
        }
    }

    pub fn set_split_ratio(&mut self, split_id: SplitId, ratio: f32) -> bool {
        let changed = self.tree.set_ratio(split_id, ratio);
        self.debug_assert_invariants();
        changed
    }

    pub(super) fn invariants_hold(&self) -> bool {
        let leaf_ids = self.tree.pane_ids();
        let leaf_set: HashSet<&str> = leaf_ids.iter().map(String::as_str).collect();
        leaf_ids.len() == leaf_set.len()
            && leaf_set.len() == self.panes.len()
            && self
                .panes
                .iter()
                .all(|(pane_id, pane)| pane_id == &pane.id && leaf_set.contains(pane_id.as_str()))
            && self.panes.contains_key(&self.active_pane_id)
    }

    fn debug_assert_invariants(&self) {
        debug_assert!(
            self.invariants_hold(),
            "terminal layout tree, pane map, and active pane diverged"
        );
    }

    pub fn active(&self) -> &TerminalPane {
        self.panes
            .get(&self.active_pane_id)
            .expect("active pane must exist in terminal layout")
    }

    pub fn active_mut(&mut self) -> &mut TerminalPane {
        self.panes
            .get_mut(&self.active_pane_id)
            .expect("active pane must exist in terminal layout")
    }
}

pub struct Tab {
    pub id: String,
    pub label: String,
    pub tab_type: TabType,
    pub layout: TerminalLayout,
    pub remote_monitor_leased: bool,
}

impl Deref for Tab {
    type Target = TerminalPane;

    fn deref(&self) -> &Self::Target {
        self.layout.active()
    }
}

impl DerefMut for Tab {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.layout.active_mut()
    }
}

impl Tab {
    pub fn active_pane(&self) -> &TerminalPane {
        self.layout.active()
    }

    pub fn active_pane_mut(&mut self) -> &mut TerminalPane {
        self.layout.active_mut()
    }

    pub fn pane(&self, pane_id: &str) -> Option<&TerminalPane> {
        self.layout.get(pane_id)
    }

    pub fn pane_mut(&mut self, pane_id: &str) -> Option<&mut TerminalPane> {
        self.layout.get_mut(pane_id)
    }

    pub fn pane_count(&self) -> usize {
        self.layout.len()
    }

    pub fn set_active_pane(&mut self, pane_id: &str) -> bool {
        self.layout.set_active(pane_id)
    }

    pub fn active_pane_id(&self) -> &str {
        self.layout.active_pane_id()
    }

    pub fn panes(&self) -> impl ExactSizeIterator<Item = &TerminalPane> {
        self.layout.iter()
    }

    pub fn monitor_key(&self) -> MonitorKey {
        match &self.tab_type {
            TabType::Local { .. } => MonitorKey::Local,
            TabType::Ssh { params, .. } => MonitorKey::from_ssh(params),
            TabType::Process { key, .. } | TabType::Network { key, .. } => key.clone(),
            TabType::Serial { .. } | TabType::Recording { .. } | TabType::Settings => {
                MonitorKey::Local
            }
        }
    }
}
