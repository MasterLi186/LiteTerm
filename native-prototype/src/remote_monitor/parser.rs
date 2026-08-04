use super::*;

#[derive(Default)]
pub struct RemoteSnapshotParser {
    previous_cpu: Option<(u64, u64)>,
    previous_network: HashMap<String, (u64, u64)>,
}

impl RemoteSnapshotParser {
    pub fn parse(&mut self, output: &str, elapsed: Duration) -> Result<MonitorData, String> {
        let sections = split_sections(output)?;
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
        let preferred_net_interface = parse_preferred_network_interface(
            sections.get("NETDEFAULT").map(String::as_str).unwrap_or(""),
            &net_interfaces,
        );

        let mut processes = parse_processes(sections.get("PS").map(String::as_str).unwrap_or(""));
        let zombie_processes = parse_processes_with_limit(
            sections.get("PSZOMBIE").map(String::as_str).unwrap_or(""),
            200,
        )
        .into_iter()
        .filter(|process| process.state.starts_with('Z'))
        .collect();
        apply_process_application_memory(
            &mut processes,
            sections.get("PSANON").map(String::as_str).unwrap_or(""),
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
            processes,
            zombie_processes,
            process_stats: parse_process_stats(
                sections.get("PSSTATS").map(String::as_str).unwrap_or(""),
            ),
            net_interfaces,
            preferred_net_interface,
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

pub(super) struct MemoryValues {
    pub(super) used: u64,
    pub(super) total: u64,
    pub(super) swap_used: u64,
    pub(super) swap_total: u64,
}

pub(super) fn split_sections(output: &str) -> Result<HashMap<&'static str, String>, String> {
    let mut sections = HashMap::new();
    let mut current = None;
    let mut saw_end = false;
    for line in output.lines() {
        if let Some(marker) = section_marker(line.trim()) {
            if marker == "END" {
                saw_end = true;
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
    saw_end
        .then_some(sections)
        .ok_or_else(|| "远端监控数据缺少 END 结束标记".to_string())
}

pub(super) fn section_marker(line: &str) -> Option<&'static str> {
    match line {
        "STAT" => Some("STAT"),
        "MEM" => Some("MEM"),
        "DISK" => Some("DISK"),
        "NETDEFAULT" => Some("NETDEFAULT"),
        "NET" => Some("NET"),
        "LOAD" => Some("LOAD"),
        "UPTIME" => Some("UPTIME"),
        "PS" => Some("PS"),
        "PSANON" => Some("PSANON"),
        "PSZOMBIE" => Some("PSZOMBIE"),
        "PSSTATS" => Some("PSSTATS"),
        "CPUINFO" => Some("CPUINFO"),
        "END" => Some("END"),
        _ => None,
    }
}

fn parse_preferred_network_interface(value: &str, interfaces: &[NetIfaceInfo]) -> Option<String> {
    let candidate = value.lines().map(str::trim).find(|line| !line.is_empty())?;
    interfaces
        .iter()
        .any(|interface| interface.name == candidate)
        .then(|| candidate.to_string())
}

pub(super) fn parse_memory(memory: &str) -> Result<MemoryValues, String> {
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

pub(super) fn parse_disks(disks: &str) -> Vec<DiskItem> {
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

pub(super) fn parse_uptime(uptime: &str) -> String {
    let seconds = uptime
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(0.0) as u64;
    format_uptime(seconds)
}

pub(super) fn parse_load(load: &str) -> String {
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

pub(super) fn parse_cpu_name(cpu_info: &str) -> String {
    cpu_info
        .lines()
        .find_map(|line| line.split_once(':').map(|(_, value)| value.trim()))
        .filter(|name| !name.is_empty())
        .unwrap_or("Unknown CPU")
        .to_string()
}

pub(super) fn parse_processes(processes: &str) -> Vec<ProcessInfo> {
    parse_processes_with_limit(processes, 100)
}

fn parse_processes_with_limit(processes: &str, limit: usize) -> Vec<ProcessInfo> {
    processes
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse::<u32>().ok()?;
            let user = truncate_utf8(fields.next()?, 256);
            let state = truncate_utf8(fields.next()?, 16);
            let rss = fields.next()?.parse::<u64>().ok()?.checked_mul(1024)?;
            let cpu = fields.next()?.parse::<f32>().ok()?;
            if !cpu.is_finite() || cpu < 0.0 {
                return None;
            }
            let name = truncate_utf8(fields.next()?, 1024);
            let start_time = normalize_process_start_time(
                &(0..5)
                    .map(|_| fields.next())
                    .collect::<Option<Vec<_>>>()?
                    .join(" "),
            );
            let command = truncate_utf8(&fields.collect::<Vec<_>>().join(" "), 8 * 1024);
            (pid > 0 && !name.is_empty()).then(|| ProcessInfo {
                pid,
                user,
                state,
                mem_mb: "—".to_string(),
                mem_bytes: 0,
                resident_mem_mb: format_bytes(rss),
                resident_mem_bytes: rss,
                cpu,
                command: if command.is_empty() {
                    name.clone()
                } else {
                    command
                },
                name,
                start_time: truncate_utf8(&start_time, 256),
            })
        })
        .take(limit)
        .collect()
}

pub(super) fn apply_process_application_memory(
    processes: &mut [ProcessInfo],
    application_memory: &str,
) {
    let values = application_memory
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse::<u32>().ok()?;
            let bytes = fields.next()?.parse::<u64>().ok()?.checked_mul(1024)?;
            (fields.next().is_none()).then_some((pid, bytes))
        })
        .collect::<HashMap<_, _>>();

    for process in processes {
        if let Some(bytes) = values.get(&process.pid).copied() {
            process.mem_mb = format_bytes(bytes);
            process.mem_bytes = bytes;
        }
    }
}

pub(super) fn parse_process_stats(stats: &str) -> ProcessStats {
    let mut parsed = ProcessStats::default();
    for line in stats.lines() {
        let mut fields = line.split_whitespace();
        let (Some(count), Some(state)) = (fields.next(), fields.next()) else {
            continue;
        };
        let Ok(count) = count.parse::<u32>() else {
            continue;
        };
        let Some(state) = state.chars().next() else {
            continue;
        };
        parsed.total = parsed.total.saturating_add(count);
        match state {
            'R' => parsed.running = parsed.running.saturating_add(count),
            'S' | 'D' | 'I' => parsed.sleeping = parsed.sleeping.saturating_add(count),
            'Z' => parsed.zombie = parsed.zombie.saturating_add(count),
            'T' | 't' => parsed.stopped = parsed.stopped.saturating_add(count),
            _ => {}
        }
    }
    parsed
}

pub(super) fn process_detail_command(pid: u32) -> String {
    format!(
        "LC_ALL=C; export LC_ALL; pid={pid}; \
         hex() {{ od -An -v -tx1 | tr -d ' \\n'; printf '\\n'; }}; \
         printf '%s\\n' DETAIL_V1; \
         if [ ! -r /proc/$pid/stat ]; then printf '%s\\n' ERROR; printf '%s' '进程不存在或无权读取' | hex; printf '%s\\n' DETAIL_END; exit 0; fi; \
         printf '%s\\n' PROCSTAT; head -c {MAX_DETAIL_FIELD_BYTES} /proc/$pid/stat 2>/dev/null | hex; \
         printf '%s\\n' STATUS; head -c {MAX_STATUS_BYTES} /proc/$pid/status 2>/dev/null | hex; \
         printf '%s\\n' PSS; grep -m1 '^Pss:' /proc/$pid/smaps_rollup 2>/dev/null | head -c {MAX_PSS_BYTES} | hex; \
         printf '%s\\n' USER; ps -p $pid -o user= 2>/dev/null | head -n 1 | hex; \
         printf '%s\\n' CPU; ps -p $pid -o pcpu= 2>/dev/null | head -n 1 | hex; \
         printf '%s\\n' CMDLINE; head -c {MAX_DETAIL_FIELD_BYTES} /proc/$pid/cmdline 2>/dev/null | hex; \
         printf '%s\\n' EXE; readlink /proc/$pid/exe 2>/dev/null | head -c {MAX_DETAIL_FIELD_BYTES} | hex; \
         printf '%s\\n' CWD; readlink /proc/$pid/cwd 2>/dev/null | head -c {MAX_DETAIL_FIELD_BYTES} | hex; \
         printf '%s\\n' START; ps -p $pid -o lstart= 2>/dev/null | head -n 1 | hex; \
         printf '%s\\n' ENV; head -c {MAX_ENVIRONMENT_BYTES} /proc/$pid/environ 2>/dev/null | hex; \
         printf '%s\\n' ANCESTORS; {{ p=$pid; i=0; \
           while [ \"$p\" -gt 0 ] 2>/dev/null && [ $i -lt {MAX_ANCESTORS} ]; do \
             stat=$(head -c {MAX_DETAIL_FIELD_BYTES} /proc/$p/stat 2>/dev/null) || break; \
             [ -n \"$stat\" ] || break; \
             comm=${{stat#*(}}; comm=${{comm%)*}}; rest=${{stat##*) }}; set -- $rest; pp=$2; \
             cmd=$(head -c {MAX_ANCESTOR_COMMAND_BYTES} /proc/$p/cmdline 2>/dev/null | tr '\\000\\n|' '   '); \
             printf '%s|%s|%s\\n' \"$p\" \"$comm\" \"$cmd\"; \
             [ \"$p\" = \"1\" ] && break; p=$pp; i=$((i+1)); \
           done; \
         }} | hex; \
         printf '%s\\n' DETAIL_END"
    )
}

pub(super) fn parse_process_detail(pid: u32, output: &str) -> Result<ProcessDetail, String> {
    let sections = split_detail_sections(output)?;
    if sections.contains_key("ERROR") {
        return Err("远端进程不存在或无权读取".to_string());
    }
    let stat = decode_text_section(&sections, "PROCSTAT", MAX_DETAIL_FIELD_BYTES)?;
    let (stat_name, state, start_ticks) = parse_proc_stat(stat.trim())?;
    let status = decode_text_section(&sections, "STATUS", MAX_STATUS_BYTES)?;
    let name = detail_status_value(&status, "Name")
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or(stat_name);
    let state = detail_status_value(&status, "State")
        .and_then(|value| value.chars().next())
        .map(|value| value.to_string())
        .unwrap_or(state);
    let mem_bytes = detail_status_value(&status, "VmRSS")
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok())
        .and_then(|value| value.checked_mul(1024))
        .unwrap_or(0);
    let platform_memory = if sections.contains_key("PSS") {
        let pss = decode_text_section(&sections, "PSS", MAX_PSS_BYTES)?;
        detail_status_value(&pss, "Pss")
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
            .and_then(|value| value.checked_mul(1024))
            .filter(|value| *value > 0)
            .map(|bytes| ProcessMemoryMetric {
                label: "平台占用（PSS）",
                bytes,
                text: format_bytes(bytes),
            })
    } else {
        None
    };
    let user = decode_text_section(&sections, "USER", 256)?
        .trim()
        .to_string();
    let cpu = decode_text_section(&sections, "CPU", 64)?
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(0.0);
    let command_bytes = decode_hex_section(&sections, "CMDLINE", MAX_DETAIL_FIELD_BYTES)?;
    let command = String::from_utf8_lossy(
        &command_bytes
            .into_iter()
            .map(|byte| if byte == 0 { b' ' } else { byte })
            .collect::<Vec<_>>(),
    )
    .trim()
    .to_string();
    let executable = decode_text_section(&sections, "EXE", MAX_DETAIL_FIELD_BYTES)?
        .trim()
        .to_string();
    let working_dir = decode_text_section(&sections, "CWD", MAX_DETAIL_FIELD_BYTES)?
        .trim()
        .to_string();
    let start_time = normalize_process_start_time(&decode_text_section(&sections, "START", 256)?);
    let environment_bytes = decode_hex_section(&sections, "ENV", MAX_ENVIRONMENT_BYTES)?;
    let environ = environment_bytes
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let separator = entry.iter().position(|byte| *byte == b'=')?;
            let key = String::from_utf8_lossy(&entry[..separator]);
            let value = String::from_utf8_lossy(&entry[separator + 1..]);
            (!key.is_empty()).then(|| ProcessEnvironment {
                key: truncate_utf8(key.trim(), 1024),
                value: truncate_utf8(&value, 8 * 1024),
            })
        })
        .take(MAX_ENVIRONMENT_ENTRIES)
        .collect();
    let ancestor_text = decode_text_section(
        &sections,
        "ANCESTORS",
        MAX_ANCESTORS * (MAX_ANCESTOR_COMMAND_BYTES + 1024),
    )?;
    let ancestors = ancestor_text
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '|');
            let pid = fields.next()?.parse::<u32>().ok()?;
            let name = truncate_utf8(fields.next()?.trim(), 1024);
            let command = truncate_utf8(fields.next().unwrap_or("").trim(), 4096);
            Some(ProcessAncestor { pid, name, command })
        })
        .take(MAX_ANCESTORS)
        .collect();

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
        start_time,
        environ,
        ancestors,
    })
}

pub(super) fn decode_text_section(
    sections: &HashMap<&str, String>,
    marker: &str,
    max_bytes: usize,
) -> Result<String, String> {
    Ok(String::from_utf8_lossy(&decode_hex_section(sections, marker, max_bytes)?).into_owned())
}

pub(super) fn decode_hex_section(
    sections: &HashMap<&str, String>,
    marker: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let encoded = sections
        .get(marker)
        .ok_or_else(|| format!("远端进程详情缺少 {marker} 段"))?
        .trim();
    if encoded.len() % 2 != 0 {
        return Err(format!("远端进程详情 {marker} 段长度无效"));
    }
    let decoded_len = encoded.len() / 2;
    if decoded_len > max_bytes {
        return Err(format!("远端进程详情 {marker} 段超出限制"));
    }

    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = decode_hex_digit(pair[0])
                .ok_or_else(|| format!("远端进程详情 {marker} 段包含无效编码"))?;
            let low = decode_hex_digit(pair[1])
                .ok_or_else(|| format!("远端进程详情 {marker} 段包含无效编码"))?;
            Ok((high << 4) | low)
        })
        .collect()
}

pub(super) fn decode_hex_digit(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

pub(super) fn split_detail_sections(output: &str) -> Result<HashMap<&'static str, String>, String> {
    let mut sections = HashMap::new();
    let mut current = None;
    let mut saw_header = false;
    let mut saw_end = false;
    for line in output.lines() {
        match detail_marker(line.trim()) {
            Some("DETAIL_V1") => {
                saw_header = true;
                current = None;
            }
            Some("DETAIL_END") => {
                saw_end = true;
                break;
            }
            Some(marker) if saw_header => {
                current = Some(marker);
                sections.entry(marker).or_insert_with(String::new);
            }
            _ if saw_header => {
                if let Some(marker) = current {
                    let section = sections.entry(marker).or_default();
                    section.push_str(line);
                    section.push('\n');
                }
            }
            _ => {}
        }
    }
    if !saw_header || !saw_end {
        return Err("远端进程详情协议不完整".to_string());
    }
    Ok(sections)
}

pub(super) fn detail_marker(line: &str) -> Option<&'static str> {
    match line {
        "DETAIL_V1" => Some("DETAIL_V1"),
        "ERROR" => Some("ERROR"),
        "PROCSTAT" => Some("PROCSTAT"),
        "STATUS" => Some("STATUS"),
        "PSS" => Some("PSS"),
        "USER" => Some("USER"),
        "CPU" => Some("CPU"),
        "CMDLINE" => Some("CMDLINE"),
        "EXE" => Some("EXE"),
        "CWD" => Some("CWD"),
        "START" => Some("START"),
        "ENV" => Some("ENV"),
        "ANCESTORS" => Some("ANCESTORS"),
        "DETAIL_END" => Some("DETAIL_END"),
        _ => None,
    }
}

pub(super) fn parse_proc_stat(stat: &str) -> Result<(String, String, u64), String> {
    let open = stat
        .find('(')
        .ok_or_else(|| "远端进程详情包含无效 PROCSTAT".to_string())?;
    let close = stat
        .rfind(") ")
        .ok_or_else(|| "远端进程详情包含无效 PROCSTAT".to_string())?;
    if close <= open {
        return Err("远端进程详情包含无效 PROCSTAT".to_string());
    }
    let name = stat[open + 1..close].to_string();
    let fields = stat[close + 2..].split_whitespace().collect::<Vec<_>>();
    let state = fields
        .first()
        .filter(|value| value.len() == 1)
        .ok_or_else(|| "远端进程详情包含无效 PROCSTAT".to_string())?
        .to_string();
    let start_ticks = fields
        .get(19)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| "远端进程详情包含无效 PROCSTAT".to_string())?;
    Ok((name, state, start_ticks))
}

pub(super) fn detail_status_value<'a>(status: &'a str, key: &str) -> Option<&'a str> {
    status.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name == key).then_some(value.trim())
    })
}

pub(super) fn truncate_utf8(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut boundary = max;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_string()
}

pub(super) fn percent(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64 * 100.0) as f32
    }
}
