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
        let line = stat
            .lines()
            .find(|line| line.split_whitespace().next() == Some("cpu"))
            .ok_or_else(|| "远端监控数据缺少有效 STAT CPU 数据".to_string())?;
        let values = line
            .split_whitespace()
            .skip(1)
            .map(|field| field.parse::<u64>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "远端监控数据缺少有效 STAT CPU 数据".to_string())?;
        if values.len() < 4 {
            return Err("远端监控数据缺少有效 STAT CPU 数据".to_string());
        }
        let total = values
            .iter()
            .try_fold(0_u64, |sum, value| sum.checked_add(*value))
            .ok_or_else(|| "远端监控数据缺少有效 STAT CPU 数据".to_string())?;
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
            if marker == "END" {
                break;
            }
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
        let name = name.trim();
        if let Some(value) = value
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<u64>().ok())
        {
            if let Some(value) = value.checked_mul(1024) {
                fields.insert(name, value);
            } else if name == "MemTotal" {
                return Err("远端监控数据缺少有效 MEM 内存总量".to_string());
            }
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
            let total = fields[1].parse::<u64>().ok()?.checked_mul(1024)?;
            if total == 0 {
                return None;
            }
            let available = fields[3].parse::<u64>().ok()?.checked_mul(1024)?;
            let percent = fields[4].strip_suffix('%')?.parse::<u64>().ok()?.min(100) as u8;
            Some(DiskItem {
                mount: fields[5..].join(" "),
                avail: format_bytes(available),
                size: format_bytes(total),
                percent,
            })
        })
        .collect()
}

fn parse_uptime(uptime: &str) -> String {
    let seconds = uptime
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
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
        Ok(values)
            if values.len() == 3
                && values
                    .iter()
                    .all(|value| value.is_finite() && *value >= 0.0) =>
        {
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
            let rss = fields.next()?.parse::<u64>().ok()?.checked_mul(1024)?;
            let cpu = fields.next()?.parse::<f32>().ok()?;
            if !cpu.is_finite() || cpu < 0.0 {
                return None;
            }
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
        assert_eq!(data.memory_used, 1_048_576_000);
        assert_eq!(data.memory_total, 2_147_483_648);
        assert_eq!(data.memory_text, "1000M / 2.0G");
        assert_eq!(data.memory_percent, 48.828125);
        assert_eq!(data.swap_used, 262_144_000);
        assert_eq!(data.swap_total, 524_288_000);
        assert_eq!(data.swap_text, "250M / 500M");
        assert_eq!(data.swap_percent, 50.0);
        assert_eq!(data.uptime_text, "1天1小时1分钟");
        assert_eq!(data.load_text, "0.10, 0.20, 0.30");
        assert_eq!(data.disk_items.len(), 1);
        assert_eq!(data.disk_items[0].mount, "/");
        assert_eq!(data.disk_items[0].avail, "512M");
        assert_eq!(data.disk_items[0].size, "1.0G");
        assert_eq!(data.disk_items[0].percent, 50);
        assert_eq!(data.processes[0].mem_bytes, 10_485_760);
        assert_eq!(data.processes[0].mem_mb, "10M");
        assert_eq!(data.processes[0].cpu, 12.5);
        assert_eq!(data.processes[0].name, "/usr/bin/test process");
        assert_eq!(data.net_interfaces.len(), 1);
        assert_eq!(data.net_interfaces[0].name, "eth0");
        assert_eq!(data.net_interfaces[0].rx_rate, 0);
        assert_eq!(data.net_interfaces[0].tx_rate, 0);
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
    fn rejects_required_sections_without_valid_keys() {
        let mut parser = RemoteSnapshotParser::default();
        assert!(parser
            .parse(
                "STAT\ncpu 1 nope 2 3\nMEM\nMemTotal: 1 kB\nEND\n",
                Duration::from_secs(1),
            )
            .is_err());
        assert!(parser
            .parse(
                "STAT\ncpu 1 0 2 3\nMEM\nMemTotal: nope kB\nEND\n",
                Duration::from_secs(1),
            )
            .is_err());
    }

    #[test]
    fn end_marker_discards_later_replacement_sections() {
        let output = format!("{SAMPLE}STAT\ncpu 1 nope 2 3\nMEM\nMemTotal: nope kB\n");

        let sections = super::split_sections(&output);
        assert_eq!(
            sections["STAT"],
            "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\n"
        );
        assert_eq!(sections["MEM"], "MemTotal:       2097152 kB\nMemAvailable:   1073152 kB\nSwapTotal:       512000 kB\nSwapFree:        256000 kB\n");
    }

    #[test]
    fn end_marker_cannot_be_followed_by_required_sections() {
        let mut parser = RemoteSnapshotParser::default();
        let output = "END\nSTAT\ncpu 1 0 2 3\nMEM\nMemTotal: 1 kB\n";

        assert!(parser.parse(output, Duration::from_secs(1)).is_err());
    }

    #[test]
    fn malformed_optional_sections_are_safe() {
        let mut parser = RemoteSnapshotParser::default();
        let output = "STAT\ncpu 1 0 2 3\nMEM\nMemTotal: 1 kB\nDISK\nbad\nNET\neth0: nope\nLOAD\nnot load\nUPTIME\nnope\nPS\nbad\nCPUINFO\ninvalid\nEND\n";

        let data = parser.parse(output, Duration::ZERO).unwrap();
        assert_eq!(data.cpu_name, "Unknown CPU");
        assert!(data.disk_items.is_empty());
        assert!(data.processes.is_empty());
        assert!(data.net_interfaces.is_empty());
    }

    #[test]
    fn mem_free_is_used_when_mem_available_is_absent() {
        let mut parser = RemoteSnapshotParser::default();
        let output = SAMPLE.replace("MemAvailable:   1073152 kB", "MemFree:        1048576 kB");

        let data = parser.parse(&output, Duration::from_secs(1)).unwrap();
        assert_eq!(data.memory_used, 1_073_741_824);
        assert_eq!(data.memory_text, "1.0G / 2.0G");
    }

    #[test]
    fn network_interfaces_are_sorted_and_loopback_is_ignored() {
        let mut parser = RemoteSnapshotParser::default();
        let output = SAMPLE.replace(
            " lo: 999 0 0 0 0 0 0 0 999 0 0 0 0 0 0 0",
            " zeta: 1 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0\n alpha: 1 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0\n lo: 999 0 0 0 0 0 0 0 999 0 0 0 0 0 0 0",
        );

        let data = parser.parse(&output, Duration::from_secs(1)).unwrap();
        let names: Vec<_> = data
            .net_interfaces
            .iter()
            .map(|item| item.name.as_str())
            .collect();
        assert_eq!(names, ["alpha", "eth0", "zeta"]);
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
    fn cpu_counter_reset_never_overflows() {
        let mut parser = RemoteSnapshotParser::default();
        parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        let reset = SAMPLE.replace("cpu  100 0 50 800 50 0 0 0 0 0", "cpu  1 0 1 1 0");

        let data = parser.parse(&reset, Duration::from_secs(2)).unwrap();
        assert_eq!(data.cpu_percent, 0.0);
    }

    #[test]
    fn uptime_rejects_non_finite_and_negative_values() {
        for value in ["NaN", "+inf", "-inf", "-1.0"] {
            assert_eq!(super::parse_uptime(value), "0分钟");
        }
    }

    #[test]
    fn load_rejects_non_finite_and_negative_values() {
        for load in ["NaN 0.2 0.3", "+inf 0.2 0.3", "-1.0 0.2 0.3"] {
            assert_eq!(super::parse_load(load), "");
        }
    }

    #[test]
    fn process_parser_rejects_invalid_cpu_and_overflowing_rss() {
        let output = format!(
            "1 NaN nan\n1 inf infinite\n1 -1 negative\n{} 1 overflow\n1024 2.5 valid\n",
            u64::MAX
        );

        let processes = super::parse_processes(&output);
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].name, "valid");
        assert_eq!(processes[0].mem_bytes, 1_048_576);
    }

    #[test]
    fn memory_parser_rejects_total_overflow_and_ignores_optional_overflow() {
        let total_overflow = format!("MemTotal: {} kB\n", u64::MAX);
        assert!(super::parse_memory(&total_overflow).is_err());

        let optional_overflow = format!("MemTotal: 1 kB\nMemAvailable: {} kB\n", u64::MAX);
        let memory = super::parse_memory(&optional_overflow).unwrap();
        assert_eq!(memory.used, 1024);
    }

    #[test]
    fn disk_parser_rejects_overflow_and_clamps_percentages() {
        let disks = format!(
            "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/overflow {} 0 1 1% /overflow\n/dev/avail-overflow 100 0 {} 1% /avail-overflow\n/dev/one 100 0 50 101% /one\n/dev/two 100 0 50 999% /two\n/dev/invalid 100 0 50 nope /invalid\n/dev/negative 100 0 50 -1% /negative\n",
            u64::MAX
            , u64::MAX
        );

        let parsed = super::parse_disks(&disks);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].mount, "/one");
        assert_eq!(parsed[0].percent, 100);
        assert_eq!(parsed[1].mount, "/two");
        assert_eq!(parsed[1].percent, 100);
    }

    #[test]
    fn cpu_parser_rejects_aggregate_counter_overflow() {
        let mut parser = RemoteSnapshotParser::default();
        let output = format!("STAT\ncpu {} 1 0 0\nMEM\nMemTotal: 1 kB\nEND\n", u64::MAX);

        assert!(parser.parse(&output, Duration::from_secs(1)).is_err());
    }

    #[test]
    fn reappearing_network_interface_starts_at_zero_rate() {
        let mut parser = RemoteSnapshotParser::default();
        parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        let absent = SAMPLE.replacen(" eth0: 1000 0 0 0 0 0 0 0 2000 0 0 0 0 0 0 0\n", "", 1);
        parser.parse(&absent, Duration::from_secs(2)).unwrap();

        let data = parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        assert_eq!(data.net_interfaces[0].name, "eth0");
        assert_eq!(data.net_interfaces[0].rx_rate, 0);
        assert_eq!(data.net_interfaces[0].tx_rate, 0);
    }

    #[test]
    fn command_is_fixed_and_contains_all_markers() {
        for fragment in [
            "LC_ALL=C",
            "cat /proc/stat",
            "cat /proc/meminfo",
            "df -Pk",
            "cat /proc/net/dev",
            "cat /proc/loadavg",
            "cat /proc/uptime",
            "ps -eo rss=,pcpu=,comm= --sort=-pcpu | head -n 24",
            "grep -m1 -E '^(model name|Hardware|Processor)[[:space:]]*:' /proc/cpuinfo",
        ] {
            assert!(REMOTE_SNAPSHOT_COMMAND.contains(fragment));
        }
        let markers = [
            "STAT", "MEM", "DISK", "NET", "LOAD", "UPTIME", "PS", "CPUINFO", "END",
        ];
        let mut previous = 0;
        for marker in markers {
            let position = REMOTE_SNAPSHOT_COMMAND.find(marker).unwrap();
            assert!(position >= previous);
            previous = position;
        }
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("{user}"));
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("{host}"));
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("$USER"));
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("$HOST"));
    }
}
