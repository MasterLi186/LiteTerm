use sysinfo::{Disks, Networks, ProcessRefreshKind, ProcessesToUpdate, System};

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

#[derive(Clone, Debug)]
pub struct ProcessInfo {
    pub mem_mb: String,
    pub mem_bytes: u64,
    pub cpu: f32,
    pub name: String,
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
    pub net_interfaces: Vec<NetIfaceInfo>,
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

        // Processes (keep top candidates; UI re-sorts by tab)
        let mut processes: Vec<ProcessInfo> = self
            .sys
            .processes()
            .values()
            .map(|p| {
                let mem = p.memory();
                ProcessInfo {
                    mem_mb: format_bytes(mem),
                    mem_bytes: mem,
                    cpu: p.cpu_usage(),
                    name: p.name().to_string_lossy().to_string(),
                }
            })
            .filter(|p| p.cpu > 0.05 || p.mem_bytes > 10 * 1024 * 1024)
            .collect();
        processes.sort_by(|a, b| {
            b.cpu
                .partial_cmp(&a.cpu)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        processes.truncate(24);

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
            net_interfaces,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{format_bytes, process_refresh_kind, MonitorKey};

    const KIB: u64 = 1_024;
    const MIB: u64 = 1_048_576;
    const GIB: u64 = 1_073_741_824;
    const TIB: u64 = 1_099_511_627_776;

    #[test]
    fn disk_capacity_formats_existing_binary_units() {
        assert_eq!(format_bytes(0), "0K");
        assert_eq!(format_bytes(KIB), "1K");
        assert_eq!(format_bytes(MIB), "1M");
        assert_eq!(format_bytes(GIB), "1.0G");
    }

    #[test]
    fn disk_capacity_switches_to_tib_at_boundary() {
        let approximately_1033_7_gib = (1033.7 * GIB as f64) as u64;

        assert_eq!(format_bytes(TIB - 1), "1024.0G");
        assert_eq!(format_bytes(TIB), "1.0T");
        assert_eq!(format_bytes(TIB + TIB / 10), "1.1T");
        assert_eq!(format_bytes(approximately_1033_7_gib), "1.0T");
    }

    #[test]
    fn monitor_process_refresh_avoids_tasks() {
        let kind = process_refresh_kind();

        assert!(kind.cpu());
        assert!(kind.memory());
        assert!(!kind.tasks());
    }

    #[test]
    fn remote_monitor_keys_compare_by_user_host_and_port() {
        let key = MonitorKey::remote("alice", "server.example", 22);

        assert_eq!(key, MonitorKey::remote("alice", "server.example", 22));
        assert_ne!(key, MonitorKey::remote("bob", "server.example", 22));
        assert_ne!(key, MonitorKey::remote("alice", "other.example", 22));
        assert_ne!(key, MonitorKey::remote("alice", "server.example", 2200));
    }

    #[test]
    fn monitor_key_status_text_is_exact() {
        assert_eq!(MonitorKey::Local.status_text(), "本机");
        assert_eq!(
            MonitorKey::remote("alice", "server.example", 2200).status_text(),
            "alice@server.example:2200"
        );
    }

    #[test]
    fn monitor_key_status_text_brackets_unbracketed_ipv6_hosts() {
        assert_eq!(
            MonitorKey::remote("alice", "2001:db8::1", 2200).status_text(),
            "alice@[2001:db8::1]:2200"
        );
        assert_eq!(
            MonitorKey::remote("alice", "[2001:db8::1]", 2200).status_text(),
            "alice@[2001:db8::1]:2200"
        );
    }
}
