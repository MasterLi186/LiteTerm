use std::fmt;

use sysinfo::{
    Disks, Networks, Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind,
};

const LOCAL_DETAIL_FIELD_BYTES: usize = 8 * 1024;
const LOCAL_ENVIRONMENT_BYTES: usize = 64 * 1024;
const LOCAL_ENVIRONMENT_ENTRIES: usize = 50;
const LOCAL_ANCESTOR_COMMAND_BYTES: usize = 4 * 1024;
const LOCAL_ANCESTORS: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MonitorKey {
    Local,
    Remote {
        user: String,
        host: String,
        port: u16,
    },
}

impl MonitorKey {
    pub fn remote(user: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        Self::Remote {
            user: user.into(),
            host: host.into(),
            port,
        }
    }

    pub fn from_ssh(params: &crate::ssh::ConnectionParams) -> Self {
        Self::remote(&params.user, &params.host, params.port)
    }

    pub fn status_text(&self) -> String {
        match self {
            Self::Local => "本机".to_string(),
            Self::Remote { user, host, port } => {
                let host = if host.contains(':') && !(host.starts_with('[') && host.ends_with(']'))
                {
                    format!("[{host}]")
                } else {
                    host.clone()
                };
                format!("{user}@{host}:{port}")
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct DiskItem {
    pub mount: String,
    pub avail: String,
    pub size: String,
    pub percent: u8,
}

#[derive(Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub user: String,
    pub state: String,
    /// Application-owned memory used for the compact monitor view. On Linux this is RssAnon,
    /// which excludes reclaimable file-backed mappings. A dash is shown when unavailable.
    pub mem_mb: String,
    pub mem_bytes: u64,
    /// Full resident set size, retained separately for diagnostics in the process manager.
    pub resident_mem_mb: String,
    pub resident_mem_bytes: u64,
    pub cpu: f32,
    pub name: String,
    pub command: String,
    pub start_time: String,
}

pub(crate) fn normalize_process_start_time(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn format_process_start_time(epoch_seconds: u64) -> String {
    i64::try_from(epoch_seconds)
        .ok()
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .map(|timestamp| {
            timestamp
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| epoch_seconds.to_string())
}

impl fmt::Debug for ProcessInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessInfo")
            .field("pid", &self.pid)
            .field("user", &self.user)
            .field("state", &self.state)
            .field("mem_bytes", &self.mem_bytes)
            .field("resident_mem_bytes", &self.resident_mem_bytes)
            .field("cpu", &self.cpu)
            .field("name", &self.name)
            .field("command", &"<redacted>")
            .field("start_time", &self.start_time)
            .finish()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessStats {
    pub total: u32,
    pub running: u32,
    pub sleeping: u32,
    pub zombie: u32,
    pub stopped: u32,
}

impl ProcessStats {
    pub(crate) fn record_state(&mut self, state: &str) {
        let Some(state) = state.chars().next() else {
            return;
        };
        self.total = self.total.saturating_add(1);
        match state {
            'R' => self.running = self.running.saturating_add(1),
            'S' | 'D' | 'I' => self.sleeping = self.sleeping.saturating_add(1),
            'Z' => self.zombie = self.zombie.saturating_add(1),
            'T' | 't' => self.stopped = self.stopped.saturating_add(1),
            _ => {}
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_ticks: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProcessEnvironment {
    pub key: String,
    pub value: String,
}

impl fmt::Debug for ProcessEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessEnvironment")
            .field("key", &"<redacted>")
            .field("value", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProcessAncestor {
    pub pid: u32,
    pub name: String,
    pub command: String,
}

impl fmt::Debug for ProcessAncestor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessAncestor")
            .field("pid", &self.pid)
            .field("name", &self.name)
            .field("command", &"<redacted>")
            .finish()
    }
}

#[derive(Clone)]
pub struct ProcessDetail {
    pub identity: ProcessIdentity,
    pub user: String,
    pub state: String,
    pub mem_mb: String,
    pub mem_bytes: u64,
    pub platform_memory: Option<ProcessMemoryMetric>,
    pub cpu: f32,
    pub name: String,
    pub command: String,
    pub executable: String,
    pub working_dir: String,
    pub start_time: String,
    pub environ: Vec<ProcessEnvironment>,
    pub ancestors: Vec<ProcessAncestor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessMemoryMetric {
    pub label: &'static str,
    pub bytes: u64,
    pub text: String,
}

impl fmt::Debug for ProcessDetail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessDetail")
            .field("identity", &self.identity)
            .field("user", &self.user)
            .field("state", &self.state)
            .field("mem_bytes", &self.mem_bytes)
            .field("platform_memory", &self.platform_memory)
            .field("cpu", &self.cpu)
            .field("name", &self.name)
            .field("command", &"<redacted>")
            .field("executable", &"<redacted>")
            .field("working_dir", &"<redacted>")
            .field("start_time", &self.start_time)
            .field("environment_entries", &self.environ.len())
            .field("ancestor_count", &self.ancestors.len())
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct NetIfaceInfo {
    pub name: String,
    pub rx_rate: u64,
    pub tx_rate: u64,
}

#[derive(Clone, Debug)]
pub struct MonitorData {
    pub cpu_percent: f32,
    pub cpu_name: String,
    pub memory_used: u64,
    pub memory_total: u64,
    pub memory_text: String,
    pub memory_percent: f32,
    pub swap_used: u64,
    pub swap_total: u64,
    pub swap_text: String,
    pub swap_percent: f32,
    pub uptime_text: String,
    pub load_text: String,
    pub disk_items: Vec<DiskItem>,
    pub processes: Vec<ProcessInfo>,
    /// Bounded zombie list retained independently from the CPU-sorted process table.
    pub zombie_processes: Vec<ProcessInfo>,
    pub process_stats: ProcessStats,
    pub net_interfaces: Vec<NetIfaceInfo>,
    /// Interface carrying the default route, or the best active physical fallback.
    pub preferred_net_interface: Option<String>,
}

pub(crate) struct MonitorEvent {
    pub(crate) key: MonitorKey,
    pub(crate) generation: u64,
    pub(crate) result: Result<Box<MonitorData>, String>,
}

impl fmt::Debug for MonitorEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = if self.result.is_ok() { "Ok" } else { "Err" };
        f.debug_struct("MonitorEvent")
            .field("key", &self.key)
            .field("generation", &self.generation)
            .field("result", &status)
            .finish()
    }
}

#[derive(Default)]
pub(crate) struct MonitorSlot {
    pub(crate) data: Option<MonitorData>,
    pub(crate) error: Option<String>,
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1_024;
    const MIB: u64 = 1_048_576;
    const GIB: u64 = 1_073_741_824;
    const TIB: u64 = 1_099_511_627_776;

    if bytes >= TIB {
        format!("{:.1}T", bytes as f64 / TIB as f64)
    } else if bytes >= GIB {
        format!("{:.1}G", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.0}M", bytes as f64 / MIB as f64)
    } else {
        format!("{}K", bytes / KIB)
    }
}

pub(crate) fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{}天{}小时{}分钟", days, hours, mins)
    } else if hours > 0 {
        format!("{}小时{}分钟", hours, mins)
    } else {
        format!("{}分钟", mins)
    }
}

fn process_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .without_tasks()
}

fn local_process_detail_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_user(UpdateKind::OnlyIfNotSet)
        .with_cwd(UpdateKind::OnlyIfNotSet)
        .with_environ(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet)
        .with_exe(UpdateKind::OnlyIfNotSet)
        .without_tasks()
}

/// Collect bounded details for one local process.
///
/// The target is refreshed again after reading optional fields and its ancestor chain. This
/// catches a process that exits or a PID that is reused while the snapshot is being assembled.
pub fn collect_local_process_detail(pid: u32) -> Result<ProcessDetail, String> {
    let target = Pid::from_u32(pid);
    let mut system = System::new();
    let targets = [target];
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&targets),
        true,
        local_process_detail_refresh_kind(),
    );
    let process = system
        .process(target)
        .ok_or_else(|| "本机进程不存在或无权读取".to_string())?;
    let start_ticks = process.start_time();
    let parent = process.parent();
    let mem_bytes = process.memory();
    let platform_memory = collect_platform_memory(pid, process);
    let name = bounded_process_text(&process.name().to_string_lossy(), 1024);
    let command = bounded_process_text(
        &process
            .cmd()
            .iter()
            .map(|part| part.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" "),
        LOCAL_DETAIL_FIELD_BYTES,
    );
    let user = bounded_process_text(
        &process
            .user_id()
            .map(|user| format!("{user:?}"))
            .unwrap_or_default(),
        256,
    );
    let state = local_process_state(process.status()).to_string();
    let cpu = process.cpu_usage();
    let cpu = if cpu.is_finite() && cpu >= 0.0 {
        cpu
    } else {
        0.0
    };
    let executable = bounded_process_text(
        &process
            .exe()
            .map(|path| path.to_string_lossy())
            .unwrap_or_default(),
        LOCAL_DETAIL_FIELD_BYTES,
    );
    let working_dir = bounded_process_text(
        &process
            .cwd()
            .map(|path| path.to_string_lossy())
            .unwrap_or_default(),
        LOCAL_DETAIL_FIELD_BYTES,
    );
    let environ = bounded_local_environment(process.environ());
    let mut ancestors = Vec::with_capacity(LOCAL_ANCESTORS.min(8));
    ancestors.push(ProcessAncestor {
        pid,
        name: name.clone(),
        command: bounded_process_text(&command, LOCAL_ANCESTOR_COMMAND_BYTES),
    });

    let mut current = parent;
    while let Some(ancestor_pid) = current {
        if ancestors.len() >= LOCAL_ANCESTORS
            || ancestors
                .iter()
                .any(|ancestor| ancestor.pid == ancestor_pid.as_u32())
        {
            break;
        }
        let ancestor_targets = [ancestor_pid];
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&ancestor_targets),
            true,
            ProcessRefreshKind::nothing()
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .without_tasks(),
        );
        let Some(ancestor) = system.process(ancestor_pid) else {
            break;
        };
        let ancestor_name = bounded_process_text(&ancestor.name().to_string_lossy(), 1024);
        let ancestor_command = bounded_process_text(
            &ancestor
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" "),
            LOCAL_ANCESTOR_COMMAND_BYTES,
        );
        current = ancestor.parent();
        ancestors.push(ProcessAncestor {
            pid: ancestor_pid.as_u32(),
            name: ancestor_name,
            command: ancestor_command,
        });
    }

    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&targets),
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
    if system
        .process(target)
        .map_or(true, |process| process.start_time() != start_ticks)
    {
        return Err("本机进程已退出或 PID 已被复用".to_string());
    }

    Ok(ProcessDetail {
        identity: ProcessIdentity { pid, start_ticks },
        user,
        state,
        mem_mb: format_bytes(mem_bytes),
        mem_bytes,
        platform_memory,
        cpu,
        name,
        command,
        executable,
        working_dir,
        start_time: format_process_start_time(start_ticks),
        environ,
        ancestors,
    })
}

fn memory_metric(label: &'static str, bytes: u64) -> Option<ProcessMemoryMetric> {
    (bytes > 0).then(|| ProcessMemoryMetric {
        label,
        bytes,
        text: format_bytes(bytes),
    })
}

#[cfg(target_os = "linux")]
fn collect_platform_memory(pid: u32, _process: &sysinfo::Process) -> Option<ProcessMemoryMetric> {
    let contents = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
    let bytes = parse_linux_pss_bytes(&contents)?;
    memory_metric("平台占用（PSS）", bytes)
}

#[cfg(target_os = "linux")]
fn parse_linux_pss_bytes(contents: &str) -> Option<u64> {
    contents.lines().find_map(|line| {
        let value = line.strip_prefix("Pss:")?.split_whitespace().next()?;
        value.parse::<u64>().ok()?.checked_mul(1024)
    })
}

#[cfg(target_os = "windows")]
fn collect_platform_memory(_pid: u32, process: &sysinfo::Process) -> Option<ProcessMemoryMetric> {
    // sysinfo 0.34 maps this value to PROCESS_MEMORY_COUNTERS_EX::PrivateUsage on Windows.
    memory_metric("平台占用（私有提交）", process.virtual_memory())
}

#[cfg(target_os = "macos")]
fn collect_platform_memory(pid: u32, _process: &sysinfo::Process) -> Option<ProcessMemoryMetric> {
    let pid = i32::try_from(pid).ok()?;
    // SAFETY: `info` is initialized storage of the exact version requested from libproc. The
    // pointer is valid for the duration of the call and the return code is checked before use.
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V2,
            &mut info as *mut libc::rusage_info_v2 as *mut libc::rusage_info_t,
        )
    };
    (result == 0)
        .then_some(info.ri_phys_footprint)
        .and_then(|bytes| memory_metric("平台占用（物理足迹）", bytes))
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn collect_platform_memory(_pid: u32, _process: &sysinfo::Process) -> Option<ProcessMemoryMetric> {
    None
}

#[cfg(target_os = "linux")]
fn collect_process_application_memory(pid: u32, _process: &sysinfo::Process) -> Option<u64> {
    let contents = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    parse_linux_rss_anon_bytes(&contents)
}

#[cfg(target_os = "linux")]
fn parse_linux_rss_anon_bytes(contents: &str) -> Option<u64> {
    contents.lines().find_map(|line| {
        let value = line.strip_prefix("RssAnon:")?.split_whitespace().next()?;
        value.parse::<u64>().ok()?.checked_mul(1024)
    })
}

#[cfg(target_os = "windows")]
fn collect_process_application_memory(_pid: u32, process: &sysinfo::Process) -> Option<u64> {
    // sysinfo exposes PROCESS_MEMORY_COUNTERS_EX::PrivateUsage here.
    Some(process.virtual_memory())
}

#[cfg(target_os = "macos")]
fn collect_process_application_memory(pid: u32, _process: &sysinfo::Process) -> Option<u64> {
    let pid = i32::try_from(pid).ok()?;
    // SAFETY: `info` has the exact layout requested from libproc and remains valid for the call.
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V2,
            &mut info as *mut libc::rusage_info_v2 as *mut libc::rusage_info_t,
        )
    };
    (result == 0).then_some(info.ri_phys_footprint)
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn collect_process_application_memory(_pid: u32, _process: &sysinfo::Process) -> Option<u64> {
    None
}

fn bounded_local_environment(environment: &[std::ffi::OsString]) -> Vec<ProcessEnvironment> {
    let mut remaining_bytes = LOCAL_ENVIRONMENT_BYTES;
    environment
        .iter()
        .filter_map(|entry| {
            if remaining_bytes == 0 {
                return None;
            }
            let entry = entry.to_string_lossy();
            let (key, value) = entry.split_once('=')?;
            if key.is_empty() {
                return None;
            }
            let key = bounded_process_text(key, 1024.min(remaining_bytes));
            remaining_bytes = remaining_bytes.saturating_sub(key.len());
            let value = bounded_process_text(value, (8 * 1024).min(remaining_bytes));
            remaining_bytes = remaining_bytes.saturating_sub(value.len());
            Some(ProcessEnvironment { key, value })
        })
        .take(LOCAL_ENVIRONMENT_ENTRIES)
        .collect()
}

pub struct MonitorCollector {
    sys: System,
    networks: Networks,
    disks: Disks,
    prev_rx: std::collections::HashMap<String, u64>,
    prev_tx: std::collections::HashMap<String, u64>,
}

impl MonitorCollector {
    pub fn new() -> Self {
        // sysinfo caches one /proc/*/stat descriptor per process/task by default.
        // This monitor refreshes every two seconds, so reopening those files is
        // preferable to retaining thousands of descriptors on large systems.
        sysinfo::set_open_files_limit(0);

        let mut sys = System::new();
        sys.refresh_cpu_all();
        sys.refresh_memory();
        let networks = Networks::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();
        Self {
            sys,
            networks,
            disks,
            prev_rx: std::collections::HashMap::new(),
            prev_tx: std::collections::HashMap::new(),
        }
    }

    pub fn collect(&mut self) -> MonitorData {
        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();
        self.sys
            .refresh_processes_specifics(ProcessesToUpdate::All, true, process_refresh_kind());
        self.networks.refresh(true);
        self.disks.refresh(true);

        // CPU
        let cpu_percent = self.sys.global_cpu_usage();
        let cpu_name = self
            .sys
            .cpus()
            .first()
            .map(|c| c.brand().to_string())
            .unwrap_or_else(|| "Unknown CPU".to_string());

        // Memory
        let memory_used = self.sys.used_memory();
        let memory_total = self.sys.total_memory();
        let memory_percent = if memory_total > 0 {
            (memory_used as f32 / memory_total as f32) * 100.0
        } else {
            0.0
        };
        let memory_text = format!(
            "{} / {}",
            format_bytes(memory_used),
            format_bytes(memory_total)
        );

        // Swap
        let swap_used = self.sys.used_swap();
        let swap_total = self.sys.total_swap();
        let swap_percent = if swap_total > 0 {
            (swap_used as f32 / swap_total as f32) * 100.0
        } else {
            0.0
        };
        let swap_text = format!("{} / {}", format_bytes(swap_used), format_bytes(swap_total));

        // Uptime
        let uptime_text = format_uptime(System::uptime());

        // Load
        let load = System::load_average();
        let load_text = format!("{:.2}, {:.2}, {:.2}", load.one, load.five, load.fifteen);

        // Disks
        let mut disk_items = Vec::new();
        for disk in self.disks.list() {
            let mount = disk.mount_point().to_string_lossy().to_string();
            let total = disk.total_space();
            let avail = disk.available_space();
            if total == 0 {
                continue;
            }
            let used = total - avail;
            let percent = ((used as f64 / total as f64) * 100.0) as u8;
            disk_items.push(DiskItem {
                mount,
                avail: format_bytes(avail),
                size: format_bytes(total),
                percent,
            });
        }
        // Filter out tiny/virtual filesystems
        disk_items.retain(|d| {
            !d.mount.starts_with("/snap")
                && !d.mount.starts_with("/sys")
                && !d.mount.starts_with("/run")
                && d.size != "0K"
        });

        // Processes (keep a bounded list; the process page re-sorts locally).
        let mut process_stats = ProcessStats::default();
        for process in self.sys.processes().values() {
            process_stats.record_state(local_process_state(process.status()));
        }
        let mut processes: Vec<ProcessInfo> = self
            .sys
            .processes()
            .values()
            .map(|p| {
                let resident_mem = p.memory();
                let name = truncate_process_text(&p.name().to_string_lossy(), 1024);
                ProcessInfo {
                    pid: p.pid().as_u32(),
                    user: truncate_process_text(
                        &p.user_id()
                            .map(|user| format!("{user:?}"))
                            .unwrap_or_default(),
                        256,
                    ),
                    state: local_process_state(p.status()).to_string(),
                    mem_mb: "—".to_string(),
                    mem_bytes: 0,
                    resident_mem_mb: format_bytes(resident_mem),
                    resident_mem_bytes: resident_mem,
                    cpu: p.cpu_usage(),
                    name: name.clone(),
                    command: {
                        let command = p
                            .cmd()
                            .iter()
                            .map(|part| part.to_string_lossy())
                            .collect::<Vec<_>>()
                            .join(" ");
                        if command.is_empty() {
                            name
                        } else {
                            truncate_process_text(&command, 8 * 1024)
                        }
                    },
                    start_time: format_process_start_time(p.start_time()),
                }
            })
            .collect();
        let zombie_processes = processes
            .iter()
            .filter(|process| process.state.starts_with('Z'))
            .take(200)
            .cloned()
            .collect();
        processes.sort_by(|a, b| {
            b.cpu
                .partial_cmp(&a.cpu)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        processes.truncate(100);
        // Only enrich the bounded list. Reading one small status record per displayed process
        // avoids scanning smaps and keeps the two-second monitor refresh inexpensive.
        for info in &mut processes {
            let Some(process) = self.sys.process(Pid::from_u32(info.pid)) else {
                continue;
            };
            if let Some(bytes) = collect_process_application_memory(info.pid, process) {
                info.mem_mb = format_bytes(bytes);
                info.mem_bytes = bytes;
            }
        }

        // Network
        let mut net_interfaces = Vec::new();
        for (name, data) in self.networks.list() {
            let rx = data.total_received();
            let tx = data.total_transmitted();
            let prev_r = self.prev_rx.get(name).copied().unwrap_or(rx);
            let prev_t = self.prev_tx.get(name).copied().unwrap_or(tx);
            let rx_rate = rx.saturating_sub(prev_r) / 2; // 2 second interval
            let tx_rate = tx.saturating_sub(prev_t) / 2;
            self.prev_rx.insert(name.clone(), rx);
            self.prev_tx.insert(name.clone(), tx);
            net_interfaces.push(NetIfaceInfo {
                name: name.clone(),
                rx_rate,
                tx_rate,
            });
        }
        // Sort by name, filter out lo
        net_interfaces.retain(|n| n.name != "lo");
        net_interfaces.sort_by(|a, b| a.name.cmp(&b.name));
        let preferred_net_interface = detect_preferred_network_interface(&net_interfaces);

        MonitorData {
            cpu_percent,
            cpu_name,
            memory_used,
            memory_total,
            memory_text,
            memory_percent,
            swap_used,
            swap_total,
            swap_text,
            swap_percent,
            uptime_text,
            load_text,
            disk_items,
            processes,
            zombie_processes,
            process_stats,
            net_interfaces,
            preferred_net_interface,
        }
    }
}

fn is_virtual_network_interface(name: &str) -> bool {
    name == "lo"
        || name.starts_with("br-")
        || name.starts_with("docker")
        || name.starts_with("veth")
        || name.starts_with("virbr")
        || name.starts_with("tun")
        || name.starts_with("tap")
}

fn detect_preferred_network_interface(interfaces: &[NetIfaceInfo]) -> Option<String> {
    let contains = |name: &str| interfaces.iter().any(|interface| interface.name == name);
    let routed = platform_default_interface().filter(|name| contains(name));
    if routed.is_some() {
        return routed;
    }

    #[cfg(target_os = "linux")]
    if let Some(interface) = interfaces.iter().find(|interface| {
        !is_virtual_network_interface(&interface.name) && linux_interface_link_up(&interface.name)
    }) {
        return Some(interface.name.clone());
    }

    interfaces
        .iter()
        .find(|interface| !is_virtual_network_interface(&interface.name))
        .or_else(|| interfaces.first())
        .map(|interface| interface.name.clone())
}

#[cfg(target_os = "linux")]
fn platform_default_interface() -> Option<String> {
    let routes = std::fs::read_to_string("/proc/net/route").ok()?;
    routes.lines().skip(1).find_map(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        (fields.len() >= 4 && fields[1] == "00000000").then(|| fields[0].to_string())
    })
}

#[cfg(target_os = "linux")]
fn linux_interface_link_up(name: &str) -> bool {
    let root = std::path::Path::new("/sys/class/net").join(name);
    std::fs::read_to_string(root.join("carrier")).is_ok_and(|value| value.trim() == "1")
        || std::fs::read_to_string(root.join("operstate"))
            .is_ok_and(|value| matches!(value.trim(), "up" | "unknown"))
}

#[cfg(target_os = "macos")]
fn platform_default_interface() -> Option<String> {
    let output = std::process::Command::new("route")
        .args(["-n", "get", "default"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().strip_prefix("interface:").map(str::trim))
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

#[cfg(target_os = "windows")]
fn platform_default_interface() -> Option<String> {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-NetRoute -DestinationPrefix '0.0.0.0/0' | Sort-Object RouteMetric | Select-Object -First 1 -ExpandProperty InterfaceAlias",
        ])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_default_interface() -> Option<String> {
    None
}

fn local_process_state(status: ProcessStatus) -> &'static str {
    match status {
        ProcessStatus::Run => "R",
        ProcessStatus::Sleep | ProcessStatus::Idle => "S",
        ProcessStatus::Zombie => "Z",
        ProcessStatus::Stop | ProcessStatus::Tracing => "T",
        ProcessStatus::UninterruptibleDiskSleep => "D",
        ProcessStatus::Dead => "X",
        ProcessStatus::Wakekill | ProcessStatus::Waking => "W",
        ProcessStatus::Parked => "P",
        ProcessStatus::LockBlocked => "L",
        ProcessStatus::Unknown(_) => "?",
    }
}

fn truncate_process_text(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut boundary = max;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_string()
}

fn bounded_process_text(value: &str, max_bytes: usize) -> String {
    let sanitized = value
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    truncate_process_text(&sanitized, max_bytes)
}

#[cfg(test)]
#[path = "monitor/tests.rs"]
mod tests;
