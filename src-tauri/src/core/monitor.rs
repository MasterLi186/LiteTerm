use std::collections::VecDeque;

/// CPU metrics parsed from /proc/stat.
#[derive(Debug, Clone)]
pub struct CpuMetric {
    pub label: String,
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
}

impl CpuMetric {
    /// Calculate overall CPU usage percentage compared to a previous sample.
    pub fn usage_percent(&self, prev: &CpuMetric) -> f64 {
        let prev_total = prev.user + prev.nice + prev.system + prev.idle + prev.iowait;
        let curr_total = self.user + self.nice + self.system + self.idle + self.iowait;
        let total_diff = curr_total.saturating_sub(prev_total) as f64;
        if total_diff == 0.0 {
            return 0.0;
        }
        let idle_diff = self.idle.saturating_sub(prev.idle) as f64;
        ((total_diff - idle_diff) / total_diff) * 100.0
    }

    /// Calculate IO-wait percentage compared to a previous sample.
    pub fn iowait_percent(&self, prev: &CpuMetric) -> f64 {
        let prev_total = prev.user + prev.nice + prev.system + prev.idle + prev.iowait;
        let curr_total = self.user + self.nice + self.system + self.idle + self.iowait;
        let total_diff = curr_total.saturating_sub(prev_total) as f64;
        if total_diff == 0.0 {
            return 0.0;
        }
        let iowait_diff = self.iowait.saturating_sub(prev.iowait) as f64;
        (iowait_diff / total_diff) * 100.0
    }
}

/// Memory metrics parsed from /proc/meminfo.
#[derive(Debug, Clone)]
pub struct MemoryMetric {
    pub total_kb: u64,
    pub free_kb: u64,
    pub available_kb: u64,
    pub buffers_kb: u64,
    pub cached_kb: u64,
}

impl MemoryMetric {
    /// Compute used memory (total minus free).
    pub fn used_kb(&self) -> u64 {
        self.total_kb.saturating_sub(self.free_kb)
    }
}

/// Disk metrics parsed from `df` output.
#[derive(Debug, Clone)]
pub struct DiskMetric {
    pub filesystem: String,
    pub size: String,
    pub used: String,
    pub avail: String,
    pub use_percent: u8,
    pub mount_point: String,
}

/// Network metrics parsed from /proc/net/dev.
#[derive(Debug, Clone)]
pub struct NetworkMetric {
    pub interface: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// Load average parsed from /proc/loadavg.
#[derive(Debug, Clone)]
pub struct LoadMetric {
    pub load_1m: f64,
    pub load_5m: f64,
    pub load_15m: f64,
}

/// A single process entry parsed from `ps aux` output.
#[derive(Debug, Clone)]
pub struct ProcessMetric {
    pub pid: u32,
    pub user: String,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    pub rss_kb: u64,
    pub command: String,
}

/// A fixed-capacity ring buffer for time-series metric data.
#[derive(Debug, Clone)]
pub struct MetricBuffer<T: Clone> {
    buf: VecDeque<T>,
    capacity: usize,
}

impl<T: Clone> MetricBuffer<T> {
    /// Create a new ring buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push a value, evicting the oldest if at capacity.
    pub fn push(&mut self, value: T) {
        if self.buf.len() == self.capacity {
            self.buf.pop_front();
        }
        self.buf.push_back(value);
    }

    /// Return contents as a contiguous Vec in insertion order.
    pub fn as_slice(&self) -> Vec<T> {
        self.buf.iter().cloned().collect()
    }

    /// Number of elements currently stored.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Return a reference to the most recent element.
    pub fn last(&self) -> Option<&T> {
        self.buf.back()
    }
}

// ---------------------------------------------------------------------------
// Parsing functions
// ---------------------------------------------------------------------------

/// Parse /proc/stat CPU lines into CpuMetric entries.
///
/// Expected format per line:
///   cpu  <user> <nice> <system> <idle> <iowait> ...
///   cpu0 <user> <nice> <system> <idle> <iowait> ...
pub fn parse_proc_stat_cpu(input: &str) -> Vec<CpuMetric> {
    let mut metrics = Vec::new();
    for line in input.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("cpu") {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 6 {
            continue;
        }
        let label = parts[0].to_string();
        let user = parts[1].parse::<u64>().unwrap_or(0);
        let nice = parts[2].parse::<u64>().unwrap_or(0);
        let system = parts[3].parse::<u64>().unwrap_or(0);
        let idle = parts[4].parse::<u64>().unwrap_or(0);
        let iowait = parts[5].parse::<u64>().unwrap_or(0);
        metrics.push(CpuMetric {
            label,
            user,
            nice,
            system,
            idle,
            iowait,
        });
    }
    metrics
}

/// Parse /proc/meminfo into a MemoryMetric.
pub fn parse_proc_meminfo(input: &str) -> Option<MemoryMetric> {
    let mut total_kb = None;
    let mut free_kb = None;
    let mut available_kb = None;
    let mut buffers_kb = None;
    let mut cached_kb = None;

    for line in input.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("MemTotal:") {
            total_kb = parse_kb_value(rest);
        } else if let Some(rest) = trimmed.strip_prefix("MemFree:") {
            free_kb = parse_kb_value(rest);
        } else if let Some(rest) = trimmed.strip_prefix("MemAvailable:") {
            available_kb = parse_kb_value(rest);
        } else if let Some(rest) = trimmed.strip_prefix("Buffers:") {
            buffers_kb = parse_kb_value(rest);
        } else if let Some(rest) = trimmed.strip_prefix("Cached:") {
            cached_kb = parse_kb_value(rest);
        }
    }

    Some(MemoryMetric {
        total_kb: total_kb?,
        free_kb: free_kb?,
        available_kb: available_kb?,
        buffers_kb: buffers_kb?,
        cached_kb: cached_kb?,
    })
}

/// Helper: extract the numeric value from "   12345 kB".
fn parse_kb_value(s: &str) -> Option<u64> {
    s.trim().split_whitespace().next()?.parse().ok()
}

/// Parse `df -h` output into DiskMetric entries (skips the header line).
pub fn parse_df_output(input: &str) -> Vec<DiskMetric> {
    let mut disks = Vec::new();
    for line in input.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 {
            continue;
        }
        let use_pct_str = parts[4].trim_end_matches('%');
        let use_percent = use_pct_str.parse::<u8>().unwrap_or(0);
        disks.push(DiskMetric {
            filesystem: parts[0].to_string(),
            size: parts[1].to_string(),
            used: parts[2].to_string(),
            avail: parts[3].to_string(),
            use_percent,
            mount_point: parts[5].to_string(),
        });
    }
    disks
}

/// Parse /proc/net/dev into NetworkMetric entries.
///
/// Skips the two header lines and the loopback (`lo`) interface.
/// Format: "  iface: rx_bytes rx_packets ... tx_bytes tx_packets ..."
pub fn parse_proc_net_dev(input: &str) -> Vec<NetworkMetric> {
    let mut nets = Vec::new();
    for line in input.lines() {
        let trimmed = line.trim();
        // Skip header lines (contain "|" or don't contain ":")
        if !trimmed.contains(':') || trimmed.contains('|') {
            continue;
        }
        let (iface, rest) = match trimmed.split_once(':') {
            Some(pair) => pair,
            None => continue,
        };
        let iface = iface.trim();
        // Skip loopback
        if iface == "lo" {
            continue;
        }
        let nums: Vec<u64> = rest
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        // rx_bytes is field 0, tx_bytes is field 8
        if nums.len() < 9 {
            continue;
        }
        nets.push(NetworkMetric {
            interface: iface.to_string(),
            rx_bytes: nums[0],
            tx_bytes: nums[8],
        });
    }
    nets
}

/// Parse /proc/loadavg into a LoadMetric.
pub fn parse_loadavg(input: &str) -> Option<LoadMetric> {
    let parts: Vec<&str> = input.trim().split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    Some(LoadMetric {
        load_1m: parts[0].parse().ok()?,
        load_5m: parts[1].parse().ok()?,
        load_15m: parts[2].parse().ok()?,
    })
}

/// Parse `ps aux --sort=-%cpu` output into ProcessMetric entries.
///
/// Expected columns: USER PID %CPU %MEM VSZ RSS TTY STAT START TIME COMMAND
pub fn parse_ps_aux(input: &str) -> Vec<ProcessMetric> {
    let mut procs = Vec::new();
    for line in input.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 11 {
            continue;
        }
        let pid = match parts[1].parse::<u32>() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let cpu_percent = parts[2].parse::<f64>().unwrap_or(0.0);
        let mem_percent = parts[3].parse::<f64>().unwrap_or(0.0);
        let rss_kb = parts[5].parse::<u64>().unwrap_or(0);
        let command = parts[10..].join(" ");
        procs.push(ProcessMetric {
            pid,
            user: parts[0].to_string(),
            cpu_percent,
            mem_percent,
            rss_kb,
            command,
        });
    }
    procs
}

/// Parse /proc/uptime into a human-readable Chinese string.
/// Format: "12345.67 98765.43" (uptime_seconds idle_seconds)
pub fn parse_proc_uptime(input: &str) -> String {
    let parts: Vec<&str> = input.trim().split_whitespace().collect();
    if parts.is_empty() {
        return "N/A".to_string();
    }
    let secs: f64 = parts[0].parse().unwrap_or(0.0);
    let total_secs = secs as u64;
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let mins = (total_secs % 3600) / 60;
    if days > 0 {
        format!("{}天 {}时 {}分", days, hours, mins)
    } else if hours > 0 {
        format!("{}时 {}分", hours, mins)
    } else {
        format!("{}分", mins)
    }
}

/// Parse SwapTotal and SwapFree from /proc/meminfo text.
/// Returns (swap_used_kb, swap_total_kb).
pub fn parse_swap_info(input: &str) -> (u64, u64) {
    let mut swap_total: u64 = 0;
    let mut swap_free: u64 = 0;
    for line in input.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("SwapTotal:") {
            swap_total = parse_kb_value(rest).unwrap_or(0);
        } else if let Some(rest) = trimmed.strip_prefix("SwapFree:") {
            swap_free = parse_kb_value(rest).unwrap_or(0);
        }
    }
    (swap_total.saturating_sub(swap_free), swap_total)
}

/// Returns the combined shell command string that collects all metrics in one
/// SSH round-trip. Each section is separated by a sentinel line.
pub fn collect_command() -> &'static str {
    concat!(
        "echo '===STAT==='; cat /proc/stat; ",
        "echo '===MEM==='; cat /proc/meminfo; ",
        "echo '===DISK==='; df -h; ",
        "echo '===NET==='; cat /proc/net/dev; ",
        "echo '===LOAD==='; cat /proc/loadavg; ",
        "echo '===UPTIME==='; cat /proc/uptime; ",
        "echo '===PS==='; ps aux --sort=-%cpu | head -20; ",
        "echo '===CPUINFO==='; grep -c ^processor /proc/cpuinfo; grep 'model name' /proc/cpuinfo | head -1; nproc; ",
        "echo '===END==='"
    )
}
