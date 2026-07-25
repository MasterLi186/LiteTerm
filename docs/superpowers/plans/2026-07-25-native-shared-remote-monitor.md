# Native Shared Remote Monitor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Native sidebar follow the active local or SSH tab while sharing one monitor connection for equal `user@host:port` identities and preserving per-tab SFTP/file-browser isolation.

**Architecture:** `MonitorKey` identifies the machine whose metrics are displayed. A dedicated `remote_monitor` worker owns one SSH connection per remote key, emits generation-tagged snapshots, and is reconciled against the SSH keys still referenced by tabs. `Sidebar` keeps presentation state per monitor key, while existing SFTP workers and file-browser state remain keyed only by `TabId`.

**Tech Stack:** Rust 2021, winit 0.30 user events, ssh2 0.9, egui 0.31, sysinfo 0.34, standard `mpsc`, existing Native test harness.

---

## File Structure

- Create `native-prototype/src/remote_monitor.rs`: remote command, parsing state, SSH worker, redacted events and shutdown handle.
- Modify `native-prototype/src/monitor.rs`: shared monitor identity, event envelope, byte/uptime formatting exposed to the remote parser.
- Modify `native-prototype/src/tab_manager.rs`: derive monitor requirements from current tabs without changing tab-local identity.
- Modify `native-prototype/src/main.rs`: monitor registry/cache, generation gate, worker reconciliation and active-tab selection.
- Modify `native-prototype/src/sidebar.rs`: per-key network/chart state and local/remote status rendering.

### Task 1: Define Stable Monitor Identity

**Files:**
- Modify: `native-prototype/src/monitor.rs`
- Modify: `native-prototype/src/tab_manager.rs`

- [ ] **Step 1: Write failing identity and tab-mapping tests**

```rust
#[test]
fn equal_user_host_and_port_share_one_remote_monitor_key() {
    let first = MonitorKey::remote("lfl", "10.0.0.8", 22);
    let duplicate = MonitorKey::remote("lfl", "10.0.0.8", 22);
    assert_eq!(first, duplicate);
}

#[test]
fn user_host_or_port_difference_creates_an_independent_key() {
    let base = MonitorKey::remote("lfl", "10.0.0.8", 22);
    assert_ne!(base, MonitorKey::remote("root", "10.0.0.8", 22));
    assert_ne!(base, MonitorKey::remote("lfl", "10.0.0.9", 22));
    assert_ne!(base, MonitorKey::remote("lfl", "10.0.0.8", 2222));
}

#[test]
fn duplicate_ssh_tabs_share_a_monitor_key_but_keep_distinct_tab_ids() {
    let mut manager = TabManager::new();
    let first = manager.new_ssh_placeholder(&test_ssh_connection());
    let second = manager.new_ssh_placeholder(&test_ssh_connection());
    assert_ne!(first, second);
    assert_eq!(
        manager.tabs[0].monitor_key(),
        manager.tabs[1].monitor_key()
    );
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
cd native-prototype
cargo test equal_user_host_and_port_share_one_remote_monitor_key
cargo test duplicate_ssh_tabs_have_one_monitor_requirement_but_distinct_tab_ids
```

Expected: compilation fails because `MonitorKey` and `Tab::monitor_key` do not exist.

- [ ] **Step 3: Add the identity and requirement mapping**

```rust
// monitor.rs
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MonitorKey {
    Local,
    Remote { user: String, host: String, port: u16 },
}

impl MonitorKey {
    pub fn remote(user: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        Self::Remote { user: user.into(), host: host.into(), port }
    }

    pub fn from_ssh(params: &crate::ssh::ConnectionParams) -> Self {
        Self::remote(&params.user, &params.host, params.port)
    }

    pub fn status_text(&self) -> String {
        match self {
            Self::Local => "本机".to_owned(),
            Self::Remote { user, host, port } => format!("{user}@{host}:{port}"),
        }
    }
}

// tab_manager.rs
impl Tab {
    pub fn monitor_key(&self) -> crate::monitor::MonitorKey {
        match &self.tab_type {
            TabType::Local { .. } => crate::monitor::MonitorKey::Local,
            TabType::Ssh { params, .. } => crate::monitor::MonitorKey::from_ssh(params),
        }
    }
}

pub fn active_monitor_key(&self) -> crate::monitor::MonitorKey {
    self.active().map_or(crate::monitor::MonitorKey::Local, |tab| match &tab.tab_type {
        TabType::Local { .. } => crate::monitor::MonitorKey::Local,
        TabType::Ssh { params, .. } => crate::monitor::MonitorKey::from_ssh(params),
    })
}
```

- [ ] **Step 4: Run focused tests and verify GREEN**

Run: `cd native-prototype && cargo test monitor::tests:: && cargo test tab_manager::tests::`

Expected: all monitor identity and tab-manager tests pass.

- [ ] **Step 5: Commit**

```bash
git add native-prototype/src/monitor.rs native-prototype/src/tab_manager.rs
git commit -m "feat: 添加 Native 监控身份映射"
```

### Task 2: Parse Remote Linux Snapshots

**Files:**
- Create: `native-prototype/src/remote_monitor.rs`
- Modify: `native-prototype/src/main.rs`
- Modify: `native-prototype/src/monitor.rs`

- [ ] **Step 1: Add representative parser tests**

```rust
const SAMPLE: &str = "\
===STAT===\ncpu  100 0 40 860 0 0 0 0\n\
===MEM===\nMemTotal: 2048000 kB\nMemAvailable: 1024000 kB\nSwapTotal: 512000 kB\nSwapFree: 256000 kB\n\
===DISK===\nFilesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda1 1048576 524288 524288 50% /\n\
===NET===\nInter-| Receive | Transmit\n eth0: 1000 0 0 0 0 0 0 0 2000 0 0 0 0 0 0 0\n\
===LOAD===\n0.10 0.20 0.30 1/100 1\n\
===UPTIME===\n90061.00 0.00\n\
===PS===\n20480 12.5 sshd\n10240 2.0 bash\n\
===CPUINFO===\nmodel name : Test CPU\n===END===\n";

#[test]
fn parser_builds_the_shared_monitor_shape() {
    let mut parser = RemoteSnapshotParser::default();
    let data = parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
    assert_eq!(data.cpu_name, "Test CPU");
    assert_eq!(data.memory_text, "1000M / 2.0G");
    assert_eq!(data.swap_text, "250M / 500M");
    assert_eq!(data.uptime_text, "1天1小时1分钟");
    assert_eq!(data.disk_items[0].mount, "/");
    assert_eq!(data.processes[0].name, "sshd");
    assert_eq!(data.net_interfaces[0].name, "eth0");
}

#[test]
fn second_sample_computes_cpu_and_network_rates_from_deltas() {
    let mut parser = RemoteSnapshotParser::default();
    parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
    let next = SAMPLE
        .replace("100 0 40 860", "120 0 50 930")
        .replace("eth0: 1000", "eth0: 5000")
        .replace("0 2000", "0 10000");
    let data = parser.parse(&next, Duration::from_secs(2)).unwrap();
    assert_eq!(data.net_interfaces[0].rx_rate, 2000);
    assert_eq!(data.net_interfaces[0].tx_rate, 4000);
    assert!((data.cpu_percent - 30.0).abs() < 0.1);
}

#[test]
fn parser_rejects_a_snapshot_missing_stat_or_memory_sections() {
    let mut parser = RemoteSnapshotParser::default();
    assert!(parser.parse("===LOAD===\n0 0 0\n", Duration::from_secs(2)).is_err());
}
```

- [ ] **Step 2: Run parser tests and verify RED**

Run: `cd native-prototype && cargo test remote_monitor::tests::parser_`

Expected: compilation fails because `remote_monitor` and `RemoteSnapshotParser` are absent.

- [ ] **Step 3: Add the fixed command and stateful parser**

Add `mod remote_monitor;` to `main.rs`. Expose `format_bytes` and `format_uptime` as `pub(crate)` in `monitor.rs`.

```rust
pub const REMOTE_SNAPSHOT_COMMAND: &str = concat!(
    "LC_ALL=C; ",
    "printf '===STAT===\\n'; cat /proc/stat; ",
    "printf '===MEM===\\n'; cat /proc/meminfo; ",
    "printf '===DISK===\\n'; df -Pk; ",
    "printf '===NET===\\n'; cat /proc/net/dev; ",
    "printf '===LOAD===\\n'; cat /proc/loadavg; ",
    "printf '===UPTIME===\\n'; cat /proc/uptime; ",
    "printf '===PS===\\n'; ps -eo rss=,pcpu=,comm= --sort=-pcpu | head -n 24; ",
    "printf '===CPUINFO===\\n'; ",
    "grep -m1 -E '^(model name|Hardware|Processor)[[:space:]]*:' /proc/cpuinfo || true; ",
    "printf '===END===\\n'"
);

#[derive(Default)]
pub struct RemoteSnapshotParser {
    previous_cpu: Option<(u64, u64)>,
    previous_network: HashMap<String, (u64, u64)>,
}

impl RemoteSnapshotParser {
    pub fn parse(
        &mut self,
        output: &str,
        elapsed: Duration,
    ) -> Result<crate::monitor::MonitorData, String> {
        let sections = split_sections(output);
        let (cpu_total, cpu_idle) = parse_cpu(required(&sections, "STAT")?)?;
        let cpu_percent = self.previous_cpu.map_or(0.0, |(old_total, old_idle)| {
            let total = cpu_total.saturating_sub(old_total);
            let idle = cpu_idle.saturating_sub(old_idle);
            if total == 0 { 0.0 } else { 100.0 * (total - idle) as f32 / total as f32 }
        });
        self.previous_cpu = Some((cpu_total, cpu_idle));

        let memory = parse_memory(required(&sections, "MEM")?)?;
        let elapsed_secs = elapsed.as_secs_f64().max(0.001);
        let net_interfaces = parse_network(
            sections.get("NET").map(String::as_str).unwrap_or(""),
            elapsed_secs,
            &mut self.previous_network,
        );
        Ok(build_monitor_data(
            cpu_percent,
            memory,
            sections.get("DISK").map(String::as_str).unwrap_or(""),
            sections.get("LOAD").map(String::as_str).unwrap_or(""),
            sections.get("UPTIME").map(String::as_str).unwrap_or(""),
            sections.get("PS").map(String::as_str).unwrap_or(""),
            sections.get("CPUINFO").map(String::as_str).unwrap_or(""),
            net_interfaces,
        ))
    }
}
```

Implement the named pure helpers in the same file with these exact rules:

- `split_sections` accepts only markers `STAT`, `MEM`, `DISK`, `NET`, `LOAD`, `UPTIME`, `PS`, `CPUINFO`, and stops at `END`.
- `parse_cpu` sums all numeric fields on the aggregate `cpu ` line and treats idle plus iowait as idle.
- `parse_memory` requires `MemTotal`, uses `MemAvailable` with `MemFree` fallback, and computes swap used from total minus free.
- `parse_network` ignores `lo`, uses receive field 0 and transmit field 8, and emits zero for the first sample.
- `parse_disk` reads `df -Pk`, converts KiB to bytes, and fills `DiskItem`.
- `parse_processes` reads `rss pcpu comm`, converts RSS KiB to bytes, and keeps at most 24 rows.
- `build_monitor_data` uses the existing binary-unit formatters and never panics on malformed optional sections.

- [ ] **Step 4: Run parser and existing local-monitor tests**

Run:

```bash
cd native-prototype
cargo test remote_monitor::tests::
cargo test monitor::tests::
```

Expected: all tests pass; local `MonitorCollector` output remains unchanged.

- [ ] **Step 5: Commit**

```bash
git add native-prototype/src/remote_monitor.rs native-prototype/src/monitor.rs native-prototype/src/main.rs
git commit -m "feat: 解析 Native 远端监控快照"
```

### Task 3: Add a Bounded, Redacted Remote Worker

**Files:**
- Modify: `native-prototype/src/remote_monitor.rs`
- Modify: `native-prototype/src/main.rs`

- [ ] **Step 1: Add worker protocol and shutdown tests**

```rust
#[test]
fn worker_event_debug_never_contains_password_or_snapshot_body() {
    let event = RemoteMonitorEvent::Failed {
        key: MonitorKey::remote("lfl", "10.0.0.8", 22),
        generation: 7,
        error: "secret-password".into(),
    };
    let debug = format!("{event:?}");
    assert!(!debug.contains("secret-password"));
}

#[test]
fn shutdown_command_stops_a_fake_source_without_another_collection() {
    let source = FakeSource::new([Ok(SAMPLE.to_owned())]);
    let (handle, events, done) =
        start_test_worker(source, MonitorKey::remote("lfl", "h", 22), 3);
    handle.shutdown();
    assert!(done.recv_timeout(Duration::from_secs(1)).is_ok());
    assert!(events.len() <= 1);
}
```

- [ ] **Step 2: Run worker tests and verify RED**

Run: `cd native-prototype && cargo test remote_monitor::tests::worker_`

Expected: compilation fails because the worker protocol and test source do not exist.

- [ ] **Step 3: Implement worker protocol and production SSH source**

```rust
pub enum RemoteMonitorCommand { Shutdown }

pub enum RemoteMonitorEvent {
    Update {
        key: MonitorKey,
        generation: u64,
        data: Box<MonitorData>,
    },
    Failed {
        key: MonitorKey,
        generation: u64,
        error: String,
    },
}

impl std::fmt::Debug for RemoteMonitorEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Update { key, generation, .. } =>
                f.debug_struct("Update").field("key", key).field("generation", generation).finish(),
            Self::Failed { key, generation, .. } =>
                f.debug_struct("Failed").field("key", key).field("generation", generation).finish(),
        }
    }
}

pub struct RemoteMonitorHandle {
    generation: u64,
    tx: mpsc::Sender<RemoteMonitorCommand>,
}

impl RemoteMonitorHandle {
    pub fn generation(&self) -> u64 { self.generation }
    pub fn shutdown(&self) { let _ = self.tx.send(RemoteMonitorCommand::Shutdown); }
}
```

Define a private `SnapshotSource` trait with `collect(&mut self) -> Result<String, String>`. Its production implementation owns `ssh2::Session`, opens one session channel per sample, executes `REMOTE_SNAPSHOT_COMMAND`, reads through `Read::take(2 * 1024 * 1024)`, closes the channel, and relies on `ssh::connect_authenticated` TCP/session timeouts.

The worker keeps `Option<SshSnapshotSource>`. It connects and collects immediately, then uses `rx.recv_timeout(Duration::from_secs(2))` between healthy samples. A connection or sample failure emits `Failed`, drops the source and uses `recv_timeout(Duration::from_secs(5))` before reconnecting, so a transient failure cannot leave a permanently dead registry entry. `Shutdown` and channel disconnect exit from both waits. Each successful result emits `Update`; if `proxy.send_event` returns `EventLoopClosed`, drop the event and exit. The worker must never log `ConnectionParams`, passwords, raw snapshots or error text through `Debug`.

- [ ] **Step 4: Run worker tests, Clippy and format**

Run:

```bash
cd native-prototype
cargo test remote_monitor::tests::worker_
cargo fmt --check
cargo clippy --all-targets
```

Expected: tests pass; Clippy exits 0 with no new warning from `remote_monitor.rs`.

- [ ] **Step 5: Commit**

```bash
git add native-prototype/src/remote_monitor.rs native-prototype/src/main.rs
git commit -m "feat: 添加共享远端监控工作线程"
```

### Task 4: Reconcile One Worker Per Referenced Remote Key

**Files:**
- Modify: `native-prototype/src/main.rs`
- Modify: `native-prototype/src/tab_manager.rs`

- [ ] **Step 1: Add lifecycle action tests**

```rust
fn params(user: &str, host: &str, port: u16) -> ssh::ConnectionParams {
    ssh::ConnectionParams {
        host: host.to_owned(),
        port,
        user: user.to_owned(),
        auth: "agent".to_owned(),
        key_path: String::new(),
        password: String::new(),
    }
}

#[test]
fn two_duplicate_tabs_start_one_monitor_and_keep_two_file_identities() {
    let key = MonitorKey::remote("lfl", "10.0.0.8", 22);
    let requirements = HashMap::from([(key, params("lfl", "10.0.0.8", 22))]);
    let actions = reconcile_actions(&requirements, &HashSet::new());
    assert_eq!(actions.starts.len(), 1);
    assert_eq!(HashSet::from(["tab-a", "tab-b"]).len(), 2);
}

#[test]
fn different_remote_keys_start_independent_workers() {
    let a = MonitorKey::remote("lfl", "10.0.0.8", 22);
    let b = MonitorKey::remote("lfl", "10.0.0.9", 22);
    let requirements = HashMap::from([
        (a, params("lfl", "10.0.0.8", 22)),
        (b, params("lfl", "10.0.0.9", 22)),
    ]);
    assert_eq!(reconcile_actions(&requirements, &HashSet::new()).starts.len(), 2);
}

#[test]
fn last_reference_removal_stops_only_that_remote_key() {
    let a = MonitorKey::remote("lfl", "10.0.0.8", 22);
    let b = MonitorKey::remote("lfl", "10.0.0.9", 22);
    let running = HashSet::from([a.clone(), b.clone()]);
    let requirements = HashMap::from([(b, params("lfl", "10.0.0.9", 22))]);
    let actions = reconcile_actions(&requirements, &running);
    assert_eq!(actions.stops, vec![a]);
}
```

- [ ] **Step 2: Run lifecycle tests and verify RED**

Run: `cd native-prototype && cargo test layout_tests::two_duplicate_tabs_start_one_monitor`

Expected: compilation fails because reconciliation helpers are missing.

- [ ] **Step 3: Implement pure reconciliation and App registry**

```rust
struct MonitorReconcileActions {
    starts: Vec<(MonitorKey, ssh::ConnectionParams)>,
    stops: Vec<MonitorKey>,
}

fn reconcile_actions(
    required: &HashMap<MonitorKey, ssh::ConnectionParams>,
    running: &HashSet<MonitorKey>,
) -> MonitorReconcileActions {
    let starts = required.iter()
        .filter(|(key, _)| !running.contains(*key))
        .map(|(key, params)| (key.clone(), params.clone()))
        .collect();
    let stops = running.iter()
        .filter(|key| !required.contains_key(*key))
        .cloned()
        .collect();
    MonitorReconcileActions { starts, stops }
}
```

Add to `App`:

```rust
remote_monitors: HashMap<MonitorKey, remote_monitor::RemoteMonitorHandle>,
remote_monitor_generations: HashMap<MonitorKey, u64>,
next_remote_monitor_generation: u64,
```

Add `ssh_connected: bool` to `Tab`, initialize it to `false`, set it to `true` only after `apply_ssh` succeeds, and reset it before reconnect. Implement `remote_monitor_requirements()` by collecting only connected SSH tabs into a `HashMap<MonitorKey, ConnectionParams>`.

Implement `reconcile_remote_monitors()` from those requirements, stopping and removing obsolete handles/generations first, then incrementing a nonzero wrapping generation and spawning each missing worker. Call it only after an SSH handle has been successfully applied, after `close_tab`, after `close_other_tabs`, and during application shutdown. A reconnect of an existing tab with the same key must keep the worker until the reconnect succeeds or the tab closes.

- [ ] **Step 4: Verify lifecycle tests**

Run: `cd native-prototype && cargo test layout_tests:: -- --nocapture`

Expected: duplicate, independent-key and last-reference tests pass.

- [ ] **Step 5: Commit**

```bash
git add native-prototype/src/main.rs native-prototype/src/tab_manager.rs
git commit -m "feat: 协调 Native 远端监控生命周期"
```

### Task 5: Gate Events and Select the Active Snapshot

**Files:**
- Modify: `native-prototype/src/monitor.rs`
- Modify: `native-prototype/src/main.rs`

- [ ] **Step 1: Add stale-event and active-selection tests**

```rust
#[test]
fn stale_remote_monitor_generation_cannot_replace_current_cache() {
    let key = MonitorKey::remote("lfl", "10.0.0.8", 22);
    let current = HashMap::from([(key.clone(), 9)]);
    assert!(!monitor_event_is_current(&key, 8, &current));
    assert!(monitor_event_is_current(&key, 9, &current));
}

#[test]
fn missing_remote_snapshot_never_falls_back_to_local_data() {
    let local = sample_monitor("local");
    let cache = HashMap::from([(MonitorKey::Local, local)]);
    let remote = MonitorKey::remote("lfl", "10.0.0.8", 22);
    assert!(active_monitor_snapshot(&cache, &remote).is_none());
}
```

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
cd native-prototype
cargo test stale_remote_monitor_generation_cannot_replace_current_cache
cargo test missing_remote_snapshot_never_falls_back_to_local_data
```

Expected: compilation fails because keyed event/cache helpers are absent.

- [ ] **Step 3: Replace the global snapshot with keyed state**

```rust
pub struct MonitorEvent {
    pub key: MonitorKey,
    pub generation: u64,
    pub result: Result<Box<MonitorData>, String>,
}

struct MonitorSlot {
    data: Option<MonitorData>,
    error: Option<String>,
}
```

Replace `UserEvent::MonitorUpdate(Box<MonitorData>)` with `UserEvent::Monitor(MonitorEvent)`. Local collection emits key `Local`, generation `0`, and `Ok(data)`. Convert remote worker events into the same envelope.

Add to `App`:

```rust
monitor_slots: HashMap<MonitorKey, MonitorSlot>,
```

Use:

```rust
fn monitor_event_is_current(
    key: &MonitorKey,
    generation: u64,
    remote_generations: &HashMap<MonitorKey, u64>,
) -> bool {
    match key {
        MonitorKey::Local => generation == 0,
        MonitorKey::Remote { .. } =>
            remote_generations.get(key).is_some_and(|current| *current == generation),
    }
}
```

On success, replace only that key’s data and clear its error. On failure, retain its previous data and store a concise Chinese error. `do_render` must derive `active_key = tab_manager.active_monitor_key()` and perform an exact lookup; never use the local slot as a fallback for a remote key.

- [ ] **Step 4: Run event and full Native tests**

Run: `cd native-prototype && cargo test`

Expected: all tests pass, including stale SSH/SFTP events and new monitor generation gates.

- [ ] **Step 5: Commit**

```bash
git add native-prototype/src/monitor.rs native-prototype/src/main.rs
git commit -m "feat: 按活动标签选择监控快照"
```

### Task 6: Keep Sidebar Presentation State Per Monitor Key

**Files:**
- Modify: `native-prototype/src/sidebar.rs`
- Modify: `native-prototype/src/main.rs`

- [ ] **Step 1: Add sidebar isolation and status tests**

```rust
#[test]
fn network_history_is_isolated_between_remote_monitor_keys() {
    let mut sidebar = Sidebar::new_for_test();
    let a = MonitorKey::remote("lfl", "10.0.0.8", 22);
    let b = MonitorKey::remote("lfl", "10.0.0.9", 22);
    sidebar.on_monitor_update(&a, &monitor_with_rate("eth0", 100, 200));
    sidebar.on_monitor_update(&b, &monitor_with_rate("ens3", 300, 400));
    assert_eq!(sidebar.monitor_view(&a).net_rx_history, [100.0]);
    assert_eq!(sidebar.monitor_view(&b).net_rx_history, [300.0]);
}

#[test]
fn remote_header_never_uses_the_local_label() {
    let key = MonitorKey::remote("lfl", "10.0.0.8", 22);
    let presentation = monitor_presentation(&key, None, None);
    assert_eq!(presentation.title, "已连接");
    assert_eq!(presentation.detail, "lfl@10.0.0.8:22");
    assert_eq!(presentation.message, "正在采集");
}
```

- [ ] **Step 2: Run sidebar tests and verify RED**

Run: `cd native-prototype && cargo test sidebar::ui_tests::network_history_is_isolated`

Expected: compilation fails because sidebar state is still global.

- [ ] **Step 3: Key all host-specific sidebar state**

```rust
struct MonitorViewState {
    selected_iface: Option<String>,
    process_tab: u8,
    net_rx_history: Vec<f64>,
    net_tx_history: Vec<f64>,
    last_chart_iface: Option<String>,
}

impl Default for MonitorViewState {
    fn default() -> Self {
        Self {
            selected_iface: None,
            process_tab: 1,
            net_rx_history: Vec::new(),
            net_tx_history: Vec::new(),
            last_chart_iface: None,
        }
    }
}

pub struct MonitorPresentation {
    pub title: String,
    pub detail: String,
    pub message: String,
}
```

Replace the five global monitor presentation fields in `Sidebar` with:

```rust
monitor_views: HashMap<MonitorKey, MonitorViewState>,
```

Change `on_monitor_update` to accept `&MonitorKey` and append only to that key’s view. Change `ui_with_monitor` to accept the active key, snapshot and error. Render:

- `Local`: blue dot, title `本机`, empty detail.
- `Remote`: green dot, title `已连接`, detail `user@host:port`.
- no snapshot/no error: `正在采集`.
- error/no snapshot: `采集失败：{error}`.
- snapshot plus error: retain cards and show a one-line `监控暂时中断` warning.

When a remote key is pruned, call `sidebar.remove_monitor_view(&key)` so its chart vectors and selected interface are released.

- [ ] **Step 4: Run sidebar and layout tests**

Run:

```bash
cd native-prototype
cargo test sidebar::
cargo test layout_tests::
```

Expected: sidebar geometry tests and new per-key behavior tests pass.

- [ ] **Step 5: Commit**

```bash
git add native-prototype/src/sidebar.rs native-prototype/src/main.rs
git commit -m "feat: 让侧栏监控跟随活动标签"
```

### Task 7: Prove File Browser Isolation and Resource Release

**Files:**
- Modify: `native-prototype/src/main.rs`
- Modify: `native-prototype/src/file_browser.rs`
- Modify: `native-prototype/src/remote_monitor.rs`

- [ ] **Step 1: Add cross-subsystem integration tests**

```rust
#[test]
fn duplicate_tabs_share_monitor_but_not_file_browser_state() {
    let key = MonitorKey::remote("lfl", "10.0.0.8", 22);
    let mut browsers = HashMap::from([
        ("tab-a".to_owned(), FileBrowserState::new("/tmp/a".into())),
        ("tab-b".to_owned(), FileBrowserState::new("/tmp/b".into())),
    ]);
    browsers.get_mut("tab-a").unwrap().remote.path = "/srv/a".into();
    assert_eq!(browsers["tab-b"].remote.path, "/");
    assert_eq!(HashSet::from([key]).len(), 1);
}

#[test]
fn shutdown_is_nonblocking_and_worker_reports_done() {
    let (handle, done) = start_blocked_test_worker();
    let started = Instant::now();
    handle.shutdown();
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(done.recv_timeout(Duration::from_secs(1)).is_ok());
}
```

- [ ] **Step 2: Run integration tests and verify RED**

Run:

```bash
cd native-prototype
cargo test duplicate_tabs_share_monitor_but_not_file_browser_state
cargo test shutdown_is_nonblocking_and_worker_reports_done
```

Expected: at least the worker completion observation test fails until a done channel exists.

- [ ] **Step 3: Complete cleanup and isolation hooks**

Add an independent `done_rx` to `RemoteMonitorHandle` for tests and a singleton reaper queue for production handles if joining is needed. The UI shutdown path may send only `Shutdown`; it must not call `recv`, `join`, or sleep.

Keep these invariants explicit in production:

```rust
debug_assert_eq!(self.sftp_workers.keys().count(), self.file_browsers.keys().count());
// remote_monitors is keyed by MonitorKey; both maps above remain keyed by TabId.
```

Do not move `FileBrowserState` or `SftpHandle` into the monitor registry. Ensure `close_tab` removes only that tab’s SFTP/browser state, then reconciles monitor keys; a sibling duplicate must retain its SFTP/browser entries and shared monitor.

- [ ] **Step 4: Run full verification**

Run:

```bash
./native-prototype/build.sh
git diff --check
git diff --cached --check
git diff --name-only -- build.sh run.sh run-native.sh native-prototype/build.sh
```

Expected: Native build succeeds; all tests pass; no whitespace errors; protected scripts have no diff.

- [ ] **Step 5: Commit**

```bash
git add native-prototype/src/main.rs native-prototype/src/file_browser.rs native-prototype/src/remote_monitor.rs
git commit -m "test: 验证共享监控与文件会话隔离"
```

### Task 8: Manual Acceptance and Final Review

**Files:**
- Modify only if acceptance reveals a defect in the files already listed above.

- [ ] **Step 1: Launch only the isolated Native binary**

Run: `./run-native.sh`

Expected: a new `liteterm-native` instance launches; the existing GuiShell process is untouched.

- [ ] **Step 2: Verify the required tab matrix**

1. Local tab: sidebar title is `本机` and shows local metrics.
2. SSH A: sidebar changes to `user@host:port` and never shows the local snapshot while waiting.
3. Duplicate SSH A: sidebar immediately reuses A’s snapshot; both bottom file browsers can navigate to different paths.
4. SSH B with a different user, host or port: sidebar shows B’s independent snapshot and network history.
5. Return to A: A’s cached snapshot/history is restored.
6. Close one A tab: the other A tab keeps monitoring and its file browser.
7. Close the last A tab: the A monitor worker exits and its extra SSH socket/FD disappears.

- [ ] **Step 3: Run staged specification and quality reviews**

Dispatch a specification reviewer against `docs/superpowers/specs/2026-07-25-native-shared-remote-monitor-design.md`, then a quality reviewer after all specification issues are closed. Both must report no Critical or Important findings.

- [ ] **Step 4: Run final evidence commands**

```bash
./native-prototype/build.sh
git diff --check
git status --short
```

Expected: build and tests pass; status contains only intended repository work and pre-existing unrelated changes.
