use std::collections::HashMap;
use std::time::Duration;

use crate::monitor::{
    format_bytes, format_uptime, DiskItem, MonitorData, NetIfaceInfo, ProcessInfo,
};

pub const REMOTE_SNAPSHOT_COMMAND: &str = "LC_ALL=C; export LC_ALL; printf '%s\\n' STAT; cat /proc/stat; printf '%s\\n' MEM; cat /proc/meminfo; printf '%s\\n' DISK; df -Pk; printf '%s\\n' NET; cat /proc/net/dev; printf '%s\\n' LOAD; cat /proc/loadavg; printf '%s\\n' UPTIME; cat /proc/uptime; printf '%s\\n' PS; ps -eo rss=,pcpu=,comm= --sort=-pcpu | head -n 24; printf '%s\\n' CPUINFO; (grep -m1 -E '^(model name|Hardware|Processor)[[:space:]]*:' /proc/cpuinfo || true); printf '%s\\n' END";

#[derive(Default)]
pub struct RemoteSnapshotParser {
    previous_cpu: Option<(u64, u64)>,
    previous_network: HashMap<String, (u64, u64)>,
}

impl RemoteSnapshotParser {
    pub fn parse(&mut self, output: &str, elapsed: Duration) -> Result<MonitorData, String> {
        let sections = split_sections(output);
        let (cpu_percent, cpu_name) = (
            self.parse_cpu(
                sections
                    .get("STAT")
                    .ok_or_else(|| "远端监控数据缺少 STAT 段".to_string())?,
            )?,
            parse_cpu_name(sections.get("CPUINFO").map(String::as_str).unwrap_or("")),
        );
        let memory = parse_memory(
            sections
                .get("MEM")
                .ok_or_else(|| "远端监控数据缺少 MEM 段".to_string())?,
        )?;
        let disk_items = parse_disks(sections.get("DISK").map(String::as_str).unwrap_or(""));
        let net_interfaces = self.parse_network(
            sections.get("NET").map(String::as_str).unwrap_or(""),
            elapsed,
        );

        Ok(MonitorData {
            cpu_percent,
            cpu_name,
            memory_used: memory.used,
            memory_total: memory.total,
            memory_text: format!(
                "{} / {}",
                format_bytes(memory.used),
                format_bytes(memory.total)
            ),
            memory_percent: percent(memory.used, memory.total),
            swap_used: memory.swap_used,
            swap_total: memory.swap_total,
            swap_text: format!(
                "{} / {}",
                format_bytes(memory.swap_used),
                format_bytes(memory.swap_total)
            ),
            swap_percent: percent(memory.swap_used, memory.swap_total),
            uptime_text: parse_uptime(sections.get("UPTIME").map(String::as_str).unwrap_or("")),
            load_text: parse_load(sections.get("LOAD").map(String::as_str).unwrap_or("")),
            disk_items,
            processes: parse_processes(sections.get("PS").map(String::as_str).unwrap_or("")),
            net_interfaces,
        })
    }

    fn parse_cpu(&mut self, stat: &str) -> Result<f32, String> {
        let values = stat
            .lines()
            .find_map(|line| {
                let mut fields = line.split_whitespace();
                (fields.next() == Some("cpu")).then(|| {
                    fields
                        .filter_map(|field| field.parse::<u64>().ok())
                        .collect::<Vec<_>>()
                })
            })
            .filter(|values| !values.is_empty())
            .ok_or_else(|| "远端监控数据缺少有效 STAT CPU 数据".to_string())?;
        let total = values
            .iter()
            .fold(0_u64, |sum, value| sum.saturating_add(*value));
        let idle = values
            .get(3)
            .copied()
            .unwrap_or(0)
            .saturating_add(values.get(4).copied().unwrap_or(0));
        let cpu_percent = self
            .previous_cpu
            .map(|(previous_total, previous_idle)| {
                let total_delta = total.saturating_sub(previous_total);
                let idle_delta = idle.saturating_sub(previous_idle);
                if total_delta == 0 {
                    0.0
                } else {
                    ((total_delta.saturating_sub(idle_delta) as f64 / total_delta as f64) * 100.0)
                        as f32
                }
            })
            .unwrap_or(0.0);
        self.previous_cpu = Some((total, idle));
        Ok(cpu_percent)
    }

    fn parse_network(&mut self, network: &str, elapsed: Duration) -> Vec<NetIfaceInfo> {
        let seconds = elapsed.as_secs_f64().max(0.001);
        let mut current = HashMap::new();
        for line in network.lines() {
            let Some((name, counters)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() || name == "lo" {
                continue;
            }
            let fields: Vec<_> = counters.split_whitespace().collect();
            let (Some(rx), Some(tx)) = (fields.first(), fields.get(8)) else {
                continue;
            };
            let (Ok(rx), Ok(tx)) = (rx.parse::<u64>(), tx.parse::<u64>()) else {
                continue;
            };
            current.insert(name.to_string(), (rx, tx));
        }

        let mut interfaces = current
            .iter()
            .map(|(name, &(rx, tx))| {
                let (previous_rx, previous_tx) =
                    self.previous_network.get(name).copied().unwrap_or((rx, tx));
                NetIfaceInfo {
                    name: name.clone(),
                    rx_rate: (rx.saturating_sub(previous_rx) as f64 / seconds) as u64,
                    tx_rate: (tx.saturating_sub(previous_tx) as f64 / seconds) as u64,
                }
            })
            .collect::<Vec<_>>();
        interfaces.sort_by(|left, right| left.name.cmp(&right.name));
        self.previous_network = current;
        interfaces
    }
}

struct MemoryValues {
    used: u64,
    total: u64,
    swap_used: u64,
    swap_total: u64,
}

fn split_sections(output: &str) -> HashMap<&'static str, String> {
    let mut sections = HashMap::new();
    let mut current = None;
    for line in output.lines() {
        if let Some(marker) = section_marker(line.trim()) {
            current = Some(marker);
            sections.entry(marker).or_insert_with(String::new);
        } else if let Some(marker) = current {
            if marker != "END" {
                sections.entry(marker).or_default().push_str(line);
                sections.entry(marker).or_default().push('\n');
            }
        }
    }
    sections
}

fn section_marker(line: &str) -> Option<&'static str> {
    match line {
        "STAT" => Some("STAT"),
        "MEM" => Some("MEM"),
        "DISK" => Some("DISK"),
        "NET" => Some("NET"),
        "LOAD" => Some("LOAD"),
        "UPTIME" => Some("UPTIME"),
        "PS" => Some("PS"),
        "CPUINFO" => Some("CPUINFO"),
        "END" => Some("END"),
        _ => None,
    }
}

fn parse_memory(memory: &str) -> Result<MemoryValues, String> {
    let mut fields = HashMap::new();
    for line in memory.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if let Some(value) = value
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<u64>().ok())
        {
            fields.insert(name.trim(), value.saturating_mul(1024));
        }
    }
    let total = fields
        .get("MemTotal")
        .copied()
        .ok_or_else(|| "远端监控数据缺少有效 MEM 内存总量".to_string())?;
    let available = fields
        .get("MemAvailable")
        .or_else(|| fields.get("MemFree"))
        .copied()
        .unwrap_or(0);
    let swap_total = fields.get("SwapTotal").copied().unwrap_or(0);
    let swap_free = fields.get("SwapFree").copied().unwrap_or(0);
    Ok(MemoryValues {
        used: total.saturating_sub(available),
        total,
        swap_used: swap_total.saturating_sub(swap_free),
        swap_total,
    })
}

fn parse_disks(disks: &str) -> Vec<DiskItem> {
    disks
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 6 {
                return None;
            }
            let total = fields[1].parse::<u64>().ok()?.saturating_mul(1024);
            if total == 0 {
                return None;
            }
            let available = fields[3].parse::<u64>().ok()?.saturating_mul(1024);
            Some(DiskItem {
                mount: fields[5..].join(" "),
                avail: format_bytes(available),
                size: format_bytes(total),
                percent: fields[4].trim_end_matches('%').parse().unwrap_or(0),
            })
        })
        .collect()
}

fn parse_uptime(uptime: &str) -> String {
    let seconds = uptime
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| *value >= 0.0)
        .unwrap_or(0.0) as u64;
    format_uptime(seconds)
}

fn parse_load(load: &str) -> String {
    let values = load
        .split_whitespace()
        .take(3)
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>();
    match values {
        Ok(values) if values.len() == 3 => {
            format!("{:.2}, {:.2}, {:.2}", values[0], values[1], values[2])
        }
        _ => String::new(),
    }
}

fn parse_cpu_name(cpu_info: &str) -> String {
    cpu_info
        .lines()
        .find_map(|line| line.split_once(':').map(|(_, value)| value.trim()))
        .filter(|name| !name.is_empty())
        .unwrap_or("Unknown CPU")
        .to_string()
}

fn parse_processes(processes: &str) -> Vec<ProcessInfo> {
    processes
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let rss = fields.next()?.parse::<u64>().ok()?.saturating_mul(1024);
            let cpu = fields.next()?.parse::<f32>().ok()?;
            let name = fields.collect::<Vec<_>>().join(" ");
            (!name.is_empty()).then(|| ProcessInfo {
                mem_mb: format_bytes(rss),
                mem_bytes: rss,
                cpu,
                name,
            })
        })
        .take(24)
        .collect()
}

fn percent(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64 * 100.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{RemoteSnapshotParser, REMOTE_SNAPSHOT_COMMAND};

    const SAMPLE: &str = "STAT\ncpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\nMEM\nMemTotal:       2097152 kB\nMemAvailable:   1073152 kB\nSwapTotal:       512000 kB\nSwapFree:        256000 kB\nDISK\nFilesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda1 1048576 524288 524288 50% /\nNET\nInter-|   Receive                                                |  Transmit\n eth0: 1000 0 0 0 0 0 0 0 2000 0 0 0 0 0 0 0\n lo: 999 0 0 0 0 0 0 0 999 0 0 0 0 0 0 0\nLOAD\n0.10 0.20 0.30 1/100 100\nUPTIME\n90060.00 0.00\nPS\n10240 12.5 /usr/bin/test process\nCPUINFO\nmodel name : Example CPU\nEND\n";

    #[test]
    fn parses_complete_snapshot_into_monitor_shape() {
        let mut parser = RemoteSnapshotParser::default();
        let data = parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();

        assert_eq!(data.cpu_percent, 0.0);
        assert_eq!(data.cpu_name, "Example CPU");
        assert_eq!(data.memory_text, "1000M / 2.0G");
        assert_eq!(data.swap_text, "250M / 500M");
        assert_eq!(data.uptime_text, "1天1小时1分钟");
        assert_eq!(data.load_text, "0.10, 0.20, 0.30");
        assert_eq!(data.disk_items.len(), 1);
        assert_eq!(data.disk_items[0].mount, "/");
        assert_eq!(data.processes[0].name, "/usr/bin/test process");
        assert_eq!(data.net_interfaces.len(), 1);
        assert_eq!(data.net_interfaces[0].name, "eth0");
        assert_eq!(data.net_interfaces[0].rx_rate, 0);
    }

    #[test]
    fn computes_second_sample_cpu_and_network_rates() {
        let mut parser = RemoteSnapshotParser::default();
        parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        let second = SAMPLE
            .replace(
                "cpu  100 0 50 800 50 0 0 0 0 0",
                "cpu  120 0 60 870 50 0 0 0 0 0",
            )
            .replace("eth0: 1000", "eth0: 5000")
            .replace("0 0 0 0 2000", "0 0 0 0 10000");

        let data = parser.parse(&second, Duration::from_secs(2)).unwrap();

        assert_eq!(data.cpu_percent, 30.0);
        assert_eq!(data.net_interfaces[0].rx_rate, 2000);
        assert_eq!(data.net_interfaces[0].tx_rate, 4000);
    }

    #[test]
    fn first_network_sample_has_zero_rates() {
        let mut parser = RemoteSnapshotParser::default();
        let data = parser.parse(SAMPLE, Duration::from_millis(1)).unwrap();

        assert!(data
            .net_interfaces
            .iter()
            .all(|item| item.rx_rate == 0 && item.tx_rate == 0));
    }

    #[test]
    fn rejects_missing_required_stat_or_mem_sections() {
        let mut parser = RemoteSnapshotParser::default();
        assert!(parser
            .parse("MEM\nMemTotal: 1 kB\nEND\n", Duration::from_secs(1))
            .is_err());
        assert!(parser
            .parse("STAT\ncpu 1 0 0 1\nEND\n", Duration::from_secs(1))
            .is_err());
    }

    #[test]
    fn malformed_optional_sections_are_safe() {
        let mut parser = RemoteSnapshotParser::default();
        let output = "STAT\ncpu 1 xx 2 3\nMEM\nMemTotal: 1 kB\nDISK\nbad\nNET\neth0: nope\nLOAD\nnot load\nUPTIME\nnope\nPS\nbad\nCPUINFO\ninvalid\nEND\n";

        let data = parser.parse(output, Duration::ZERO).unwrap();
        assert_eq!(data.cpu_name, "Unknown CPU");
        assert!(data.disk_items.is_empty());
        assert!(data.processes.is_empty());
        assert!(data.net_interfaces.is_empty());
    }

    #[test]
    fn parses_space_padded_ps_rows() {
        let mut parser = RemoteSnapshotParser::default();
        let output = SAMPLE.replace(
            "10240 12.5 /usr/bin/test process",
            "  1024   3.5   worker --flag",
        );

        let data = parser.parse(&output, Duration::from_secs(1)).unwrap();
        assert_eq!(data.processes.len(), 1);
        assert_eq!(data.processes[0].name, "worker --flag");
    }

    #[test]
    fn network_counter_reset_never_overflows() {
        let mut parser = RemoteSnapshotParser::default();
        parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        let reset = SAMPLE
            .replace("eth0: 1000", "eth0: 1")
            .replace("0 0 0 0 2000", "0 0 0 0 1");

        let data = parser.parse(&reset, Duration::from_secs(2)).unwrap();
        assert_eq!(data.net_interfaces[0].rx_rate, 0);
        assert_eq!(data.net_interfaces[0].tx_rate, 0);
    }

    #[test]
    fn command_is_fixed_and_contains_all_markers() {
        for marker in [
            "STAT", "MEM", "DISK", "NET", "LOAD", "UPTIME", "PS", "CPUINFO", "END",
        ] {
            assert!(REMOTE_SNAPSHOT_COMMAND.contains(marker));
        }
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("{user}"));
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("{host}"));
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("$USER"));
    }
}
