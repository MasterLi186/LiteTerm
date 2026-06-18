use std::io::Read;
use std::sync::atomic::Ordering;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::app_log;
use crate::core::monitor::{
    collect_command, parse_default_iface, parse_df_output, parse_loadavg, parse_proc_meminfo,
    parse_proc_net_dev, parse_proc_stat_cpu, parse_proc_uptime, parse_ps_aux, parse_swap_info,
};
use crate::state::AppState;

#[derive(Serialize, Clone)]
pub struct DiskItem {
    pub mount: String,
    pub avail: String,
    pub size: String,
    pub percent: u8,
}

#[derive(Serialize, Clone)]
pub struct ProcessInfo {
    pub mem: String,
    pub cpu: f32,
    pub command: String,
}

#[derive(Serialize, Clone)]
pub struct NetIfaceRate {
    pub name: String,
    pub rx_rate: u64,
    pub tx_rate: u64,
}

#[derive(Serialize, Clone)]
pub struct MonitorPayload {
    pub session_id: String,
    pub cpu_percent: f64,
    pub memory_used_percent: f64,
    pub memory_text: String,
    pub swap_text: String,
    pub swap_percent: f64,
    pub uptime_text: String,
    pub load_text: String,
    pub disk_items: Vec<DiskItem>,
    pub net_rx_rate: u64,
    pub net_tx_rate: u64,
    pub net_interface: String,
    pub net_interfaces: Vec<String>,
    pub net_per_iface: Vec<NetIfaceRate>,
    pub cpu_info: String,
    pub processes: Vec<ProcessInfo>,
}

fn format_kb_to_human(kb: u64) -> String {
    let gb = kb as f64 / 1_048_576.0;
    if gb >= 1.0 {
        format!("{:.1}G", gb)
    } else {
        let mb = kb as f64 / 1024.0;
        format!("{:.0}M", mb)
    }
}

/// Split the combined output by sentinel lines and parse each section.
fn parse_sections(output: &str) -> (String, String, String, String, String, String, String, String, String) {
    let mut stat = String::new();
    let mut mem = String::new();
    let mut disk = String::new();
    let mut net = String::new();
    let mut load = String::new();
    let mut uptime = String::new();
    let mut ps = String::new();
    let mut cpuinfo = String::new();
    let mut route = String::new();

    let mut current_section = "";
    for line in output.lines() {
        let trimmed = line.trim();
        match trimmed {
            "===STAT===" => { current_section = "stat"; continue; }
            "===MEM===" => { current_section = "mem"; continue; }
            "===DISK===" => { current_section = "disk"; continue; }
            "===NET===" => { current_section = "net"; continue; }
            "===LOAD===" => { current_section = "load"; continue; }
            "===UPTIME===" => { current_section = "uptime"; continue; }
            "===PS===" => { current_section = "ps"; continue; }
            "===CPUINFO===" => { current_section = "cpuinfo"; continue; }
            "===ROUTE===" => { current_section = "route"; continue; }
            "===END===" => { current_section = ""; continue; }
            _ => {}
        }
        match current_section {
            "stat" => { stat.push_str(line); stat.push('\n'); }
            "mem" => { mem.push_str(line); mem.push('\n'); }
            "disk" => { disk.push_str(line); disk.push('\n'); }
            "net" => { net.push_str(line); net.push('\n'); }
            "load" => { load.push_str(line); load.push('\n'); }
            "uptime" => { uptime.push_str(line); uptime.push('\n'); }
            "ps" => { ps.push_str(line); ps.push('\n'); }
            "cpuinfo" => { cpuinfo.push_str(line); cpuinfo.push('\n'); }
            "route" => { route.push_str(line); route.push('\n'); }
            _ => {}
        }
    }

    (stat, mem, disk, net, load, uptime, ps, cpuinfo, route)
}

fn parse_cpuinfo(cpuinfo_text: &str) -> (String, u32) {
    let mut model = String::new();
    let mut cores: u32 = 0;
    for line in cpuinfo_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("model name") {
            if let Some(name) = trimmed.split(':').nth(1) {
                model = name.trim().to_string();
            }
        } else if cores == 0 {
            if let Ok(n) = trimmed.parse::<u32>() {
                if cores == 0 { cores = n; }
            }
        }
    }
    (model, cores)
}

fn exec_command(session: &ssh2::Session, cmd: &str) -> Result<String, String> {
    let mut channel = session.channel_session().map_err(|e| e.to_string())?;
    channel.exec(cmd).map_err(|e| e.to_string())?;

    let mut output = String::new();
    channel.read_to_string(&mut output).map_err(|e| e.to_string())?;
    channel.wait_close().ok();
    Ok(output)
}

#[cfg(unix)]
fn exec_local_command(cmd: &str) -> Result<String, String> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .map_err(|e| format!("执行本地命令失败: {}", e))?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(not(unix))]
fn exec_local_command(_cmd: &str) -> Result<String, String> {
    Err("本地监控暂不支持此平台".to_string())
}

#[tauri::command]
pub async fn start_monitor(
    state: State<'_, AppState>,
    app: AppHandle,
    session_id: String,
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    auth_method: String,
    key_path: Option<String>,
) -> Result<(), String> {
    // Get the monitor_stop flag from the session
    let monitor_stop = {
        let sessions = state.sessions.lock().unwrap();
        match sessions.get(&session_id) {
            Some(s) => s.monitor_stop.clone(),
            None => return Err("Session not found".to_string()),
        }
    };

    // Reset stop flag
    monitor_stop.store(false, Ordering::Relaxed);

    let session_id_clone = session_id.clone();
    let app_clone = app.clone();
    let stop_flag = monitor_stop.clone();

    std::thread::spawn(move || {
        // Open a separate SSH connection for monitoring
        let addr = format!("{}:{}", host, port);
        let sock_addr = match addr.parse::<std::net::SocketAddr>() {
            Ok(a) => a,
            Err(e) => {
                app_log!("MON", "Monitor: invalid address: {}", e);
                return;
            }
        };

        let tcp = match std::net::TcpStream::connect_timeout(
            &sock_addr,
            std::time::Duration::from_secs(10),
        ) {
            Ok(t) => t,
            Err(e) => {
                app_log!("MON", "Monitor: TCP connect failed: {}", e);
                return;
            }
        };

        let mut session = match ssh2::Session::new() {
            Ok(s) => s,
            Err(e) => {
                app_log!("MON", "Monitor: SSH session failed: {}", e);
                return;
            }
        };
        session.set_tcp_stream(tcp);
        if let Err(e) = session.handshake() {
            app_log!("MON", "Monitor: SSH handshake failed: {}", e);
            return;
        }

        // Authenticate
        let auth_result = match auth_method.as_str() {
            "agent" => session
                .userauth_agent(&user)
                .map_err(|e| format!("{}", e)),
            "key" => {
                let key = key_path.unwrap_or_default();
                let expanded = shellexpand::tilde(&key);
                session
                    .userauth_pubkey_file(
                        &user,
                        None,
                        std::path::Path::new(expanded.as_ref()),
                        password.as_deref(),
                    )
                    .map_err(|e| format!("{}", e))
            }
            _ => {
                let pw = password.unwrap_or_default();
                session
                    .userauth_password(&user, &pw)
                    .map_err(|e| format!("{}", e))
            }
        };

        if let Err(e) = auth_result {
            app_log!("MON", "Monitor: auth failed: {}", e);
            return;
        }

        session.set_blocking(true);

        let cmd = collect_command();
        let mut prev_cpu = None;
        let mut prev_net_rx: u64 = 0;
        let mut prev_net_tx: u64 = 0;
        let mut prev_per_iface: std::collections::HashMap<String, (u64, u64)> = std::collections::HashMap::new();
        let mut first_sample = true;

        loop {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            match exec_command(&session, cmd) {
                Ok(output) => {
                    let (stat_text, mem_text, disk_text, net_text, load_text, uptime_text, ps_text, cpuinfo_text, route_text) =
                        parse_sections(&output);

                    // CPU
                    let cpu_metrics = parse_proc_stat_cpu(&stat_text);
                    let cpu_percent = if let Some(ref prev) = prev_cpu {
                        if let Some(curr) = cpu_metrics.first() {
                            curr.usage_percent(prev)
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    };
                    prev_cpu = cpu_metrics.into_iter().next();

                    // Memory
                    let (memory_used_percent, memory_display) =
                        if let Some(mem) = parse_proc_meminfo(&mem_text) {
                            let used_kb = mem.total_kb.saturating_sub(mem.available_kb);
                            let pct = if mem.total_kb > 0 {
                                (used_kb as f64 / mem.total_kb as f64) * 100.0
                            } else {
                                0.0
                            };
                            let display = format!(
                                "{} / {}",
                                format_kb_to_human(used_kb),
                                format_kb_to_human(mem.total_kb)
                            );
                            (pct, display)
                        } else {
                            (0.0, "N/A".to_string())
                        };

                    // Swap
                    let (swap_used_kb, swap_total_kb) = parse_swap_info(&mem_text);
                    let swap_percent = if swap_total_kb > 0 {
                        (swap_used_kb as f64 / swap_total_kb as f64) * 100.0
                    } else {
                        0.0
                    };
                    let swap_display = format!(
                        "{} / {}",
                        format_kb_to_human(swap_used_kb),
                        format_kb_to_human(swap_total_kb)
                    );

                    // Uptime
                    let uptime_display = parse_proc_uptime(&uptime_text);

                    // Load
                    let load_display = if let Some(load) = parse_loadavg(&load_text) {
                        format!("{:.2}, {:.2}, {:.2}", load.load_1m, load.load_5m, load.load_15m)
                    } else {
                        "N/A".to_string()
                    };

                    // Disk
                    let disks = parse_df_output(&disk_text);
                    let disk_items: Vec<DiskItem> = disks
                        .iter()
                        .filter(|d| {
                            d.mount_point.starts_with("/")
                                && !d.filesystem.starts_with("tmpfs")
                                && !d.filesystem.starts_with("udev")
                                && !d.filesystem.starts_with("overlay")
                        })
                        .map(|d| DiskItem {
                            mount: d.mount_point.clone(),
                            avail: d.avail.clone(),
                            size: d.size.clone(),
                            percent: d.use_percent,
                        })
                        .collect();

                    // Network — collect all interfaces, default to route's iface
                    let net_metrics = parse_proc_net_dev(&net_text);
                    let all_ifaces: Vec<String> = net_metrics.iter().map(|n| n.interface.clone()).collect();
                    let total_rx: u64 = net_metrics.iter().map(|n| n.rx_bytes).sum();
                    let total_tx: u64 = net_metrics.iter().map(|n| n.tx_bytes).sum();
                    let default_iface = parse_default_iface(&route_text);
                    let net_iface = default_iface
                        .filter(|d| all_ifaces.contains(d))
                        .unwrap_or_else(|| net_metrics.first().map(|n| n.interface.clone()).unwrap_or_default());

                    let mut net_per_iface: Vec<NetIfaceRate> = Vec::new();
                    let (net_rx_rate, net_tx_rate) = if first_sample {
                        first_sample = false;
                        for m in &net_metrics {
                            prev_per_iface.insert(m.interface.clone(), (m.rx_bytes, m.tx_bytes));
                            net_per_iface.push(NetIfaceRate { name: m.interface.clone(), rx_rate: 0, tx_rate: 0 });
                        }
                        (0u64, 0u64)
                    } else {
                        let rx_rate = total_rx.saturating_sub(prev_net_rx) / 2;
                        let tx_rate = total_tx.saturating_sub(prev_net_tx) / 2;
                        for m in &net_metrics {
                            let (prx, ptx) = prev_per_iface.get(&m.interface).copied().unwrap_or((0, 0));
                            net_per_iface.push(NetIfaceRate {
                                name: m.interface.clone(),
                                rx_rate: m.rx_bytes.saturating_sub(prx) / 2,
                                tx_rate: m.tx_bytes.saturating_sub(ptx) / 2,
                            });
                            prev_per_iface.insert(m.interface.clone(), (m.rx_bytes, m.tx_bytes));
                        }
                        (rx_rate, tx_rate)
                    };
                    prev_net_rx = total_rx;
                    prev_net_tx = total_tx;

                    // Processes
                    let all_procs = parse_ps_aux(&ps_text);
                    let processes: Vec<ProcessInfo> = all_procs
                        .iter()
                        .take(10)
                        .map(|p| {
                            let mem = if p.rss_kb >= 1048576 {
                                format!("{:.1}G", p.rss_kb as f64 / 1048576.0)
                            } else if p.rss_kb >= 1024 {
                                format!("{:.1}M", p.rss_kb as f64 / 1024.0)
                            } else {
                                format!("{}K", p.rss_kb)
                            };
                            // Show only process name (like ps -A), not full cmdline
                            let short_name = p.command.split_whitespace().next()
                                .and_then(|s| s.rsplit('/').next())
                                .unwrap_or(&p.command)
                                .to_string();
                            ProcessInfo {
                                mem,
                                cpu: p.cpu_percent as f32,
                                command: short_name,
                            }
                        })
                        .collect();

                    let payload = MonitorPayload {
                        session_id: session_id_clone.clone(),
                        cpu_percent,
                        memory_used_percent,
                        memory_text: memory_display,
                        swap_text: swap_display,
                        swap_percent,
                        uptime_text: uptime_display,
                        load_text: load_display,
                        disk_items,
                        net_rx_rate,
                        net_tx_rate,
                        net_interface: net_iface,
                        net_interfaces: all_ifaces,
                        net_per_iface,
                        cpu_info: {
                            let (model, cores) = parse_cpuinfo(&cpuinfo_text);
                            if model.is_empty() {
                                format!("{}核", cores)
                            } else {
                                format!("{} ({}核)", model, cores)
                            }
                        },
                        processes,
                    };

                    let _ = app_clone.emit("monitor-data", payload);
                }
                Err(e) => {
                    app_log!("MON", "Monitor exec failed: {}", e);
                    break;
                }
            }

            // Sleep 2 seconds between samples
            for _ in 0..20 {
                if stop_flag.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    });

    Ok(())
}

/// Local system monitor — same data as SSH monitor but reads from local /proc.
#[tauri::command]
pub async fn start_local_monitor(
    app: AppHandle,
) -> Result<(), String> {
    let app_clone = app.clone();

    std::thread::spawn(move || {
        let cmd = collect_command();
        let mut prev_cpu = None;
        let mut prev_net_rx: u64 = 0;
        let mut prev_net_tx: u64 = 0;
        let mut prev_per_iface: std::collections::HashMap<String, (u64, u64)> = std::collections::HashMap::new();
        let mut first_sample = true;
        let session_id = "local".to_string();

        loop {
            match exec_local_command(cmd) {
                Ok(output) => {
                    let (stat_text, mem_text, disk_text, net_text, load_text, uptime_text, ps_text, cpuinfo_text, route_text) =
                        parse_sections(&output);

                    let cpu_metrics = parse_proc_stat_cpu(&stat_text);
                    let cpu_percent = if let Some(ref prev) = prev_cpu {
                        if let Some(curr) = cpu_metrics.first() {
                            curr.usage_percent(prev)
                        } else { 0.0 }
                    } else { 0.0 };
                    prev_cpu = cpu_metrics.into_iter().next();

                    let (memory_used_percent, memory_display) =
                        if let Some(mem) = parse_proc_meminfo(&mem_text) {
                            let used_kb = mem.total_kb.saturating_sub(mem.available_kb);
                            let pct = if mem.total_kb > 0 { (used_kb as f64 / mem.total_kb as f64) * 100.0 } else { 0.0 };
                            (pct, format!("{} / {}", format_kb_to_human(used_kb), format_kb_to_human(mem.total_kb)))
                        } else { (0.0, "N/A".to_string()) };

                    let (swap_used_kb, swap_total_kb) = parse_swap_info(&mem_text);
                    let swap_percent = if swap_total_kb > 0 { (swap_used_kb as f64 / swap_total_kb as f64) * 100.0 } else { 0.0 };
                    let swap_display = format!("{} / {}", format_kb_to_human(swap_used_kb), format_kb_to_human(swap_total_kb));

                    let uptime_display = parse_proc_uptime(&uptime_text);
                    let load_display = if let Some(load) = parse_loadavg(&load_text) {
                        format!("{:.2}, {:.2}, {:.2}", load.load_1m, load.load_5m, load.load_15m)
                    } else { "N/A".to_string() };

                    let disks = parse_df_output(&disk_text);
                    let disk_items: Vec<DiskItem> = disks.iter()
                        .filter(|d| d.mount_point.starts_with("/") && !d.filesystem.starts_with("tmpfs") && !d.filesystem.starts_with("udev") && !d.filesystem.starts_with("overlay"))
                        .map(|d| DiskItem { mount: d.mount_point.clone(), avail: d.avail.clone(), size: d.size.clone(), percent: d.use_percent })
                        .collect();

                    let net_metrics = parse_proc_net_dev(&net_text);
                    let all_ifaces: Vec<String> = net_metrics.iter().map(|n| n.interface.clone()).collect();
                    let total_rx: u64 = net_metrics.iter().map(|n| n.rx_bytes).sum();
                    let total_tx: u64 = net_metrics.iter().map(|n| n.tx_bytes).sum();
                    let default_iface = parse_default_iface(&route_text);
                    let net_iface = default_iface
                        .filter(|d| all_ifaces.contains(d))
                        .unwrap_or_else(|| net_metrics.first().map(|n| n.interface.clone()).unwrap_or_default());

                    let mut net_per_iface: Vec<NetIfaceRate> = Vec::new();
                    let (net_rx_rate, net_tx_rate) = if first_sample {
                        first_sample = false;
                        for m in &net_metrics {
                            prev_per_iface.insert(m.interface.clone(), (m.rx_bytes, m.tx_bytes));
                            net_per_iface.push(NetIfaceRate { name: m.interface.clone(), rx_rate: 0, tx_rate: 0 });
                        }
                        (0u64, 0u64)
                    } else {
                        for m in &net_metrics {
                            let (prx, ptx) = prev_per_iface.get(&m.interface).copied().unwrap_or((0, 0));
                            net_per_iface.push(NetIfaceRate {
                                name: m.interface.clone(),
                                rx_rate: m.rx_bytes.saturating_sub(prx) / 2,
                                tx_rate: m.tx_bytes.saturating_sub(ptx) / 2,
                            });
                            prev_per_iface.insert(m.interface.clone(), (m.rx_bytes, m.tx_bytes));
                        }
                        (total_rx.saturating_sub(prev_net_rx) / 2, total_tx.saturating_sub(prev_net_tx) / 2)
                    };
                    prev_net_rx = total_rx;
                    prev_net_tx = total_tx;

                    let all_procs = parse_ps_aux(&ps_text);
                    let processes: Vec<ProcessInfo> = all_procs.iter().take(10).map(|p| {
                        let mem = if p.rss_kb >= 1048576 { format!("{:.1}G", p.rss_kb as f64 / 1048576.0) }
                            else if p.rss_kb >= 1024 { format!("{:.1}M", p.rss_kb as f64 / 1024.0) }
                            else { format!("{}K", p.rss_kb) };
                        let short_name = p.command.split_whitespace().next()
                            .and_then(|s| s.rsplit('/').next()).unwrap_or(&p.command).to_string();
                        ProcessInfo { mem, cpu: p.cpu_percent as f32, command: short_name }
                    }).collect();

                    let payload = MonitorPayload {
                        session_id: session_id.clone(), cpu_percent, memory_used_percent,
                        memory_text: memory_display, swap_text: swap_display, swap_percent,
                        uptime_text: uptime_display, load_text: load_display, disk_items,
                        net_rx_rate, net_tx_rate, net_interface: net_iface, net_interfaces: all_ifaces, net_per_iface,
                        cpu_info: {
                            let (model, cores) = parse_cpuinfo(&cpuinfo_text);
                            if model.is_empty() { format!("{}核", cores) } else { format!("{} ({}核)", model, cores) }
                        },
                        processes,
                    };
                    let _ = app_clone.emit("monitor-data", payload);
                }
                Err(e) => { app_log!("MON", "Local monitor failed: {}", e); break; }
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });

    Ok(())
}
