use std::io::Read as IoRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use gtk::glib;
use gtk::prelude::*;
use gtk::Box as GtkBox;

use crate::core::monitor::{
    collect_command, parse_df_output, parse_loadavg, parse_proc_meminfo, parse_proc_net_dev,
    parse_proc_stat_cpu, CpuMetric, MetricBuffer, NetworkMetric,
};
use crate::ui::monitor::MonitorChart;

/// Capacity of the ring buffers (number of data points / chart width).
const BUFFER_CAPACITY: usize = 60;

/// Parsed snapshot from one collection round.
struct MetricSnapshot {
    cpu_percent: f64,
    mem_used_percent: f64,
    mem_text: String,
    disk_items: Vec<(String, f64)>,
    net_rx_rate: f64,
    net_tx_rate: f64,
    net_text: String,
    load_text: String,
}

/// A live monitoring session that collects metrics from a remote host
/// over SSH and updates chart widgets in the sidebar.
pub struct MonitorSession {
    stop_flag: Arc<AtomicBool>,
}

impl MonitorSession {
    /// Start a monitoring session.
    ///
    /// Creates chart widgets inside `monitor_area`, spawns a background
    /// thread that periodically runs the collection command via SSH exec
    /// channels on the provided session, and feeds parsed data to the
    /// charts through a channel polled on the GTK main thread.
    ///
    /// The SSH connection parameters are re-used to open a **second**
    /// independent TCP+SSH session dedicated to monitoring so that the
    /// terminal session is not blocked by mutex contention.
    pub fn start(
        monitor_area: &GtkBox,
        host: &str,
        port: u16,
        user: &str,
        password: Option<&str>,
        key_path: Option<&str>,
    ) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));

        // --- Build chart widgets ---
        let cpu_chart = MonitorChart::new("CPU", 50);
        let mem_chart = MonitorChart::new("Memory", 50);
        let disk_chart = MonitorChart::new("Disk", 60);
        let net_chart = MonitorChart::new("Network", 50);

        let load_label = gtk::Label::builder()
            .label("Load: --")
            .halign(gtk::Align::Start)
            .margin_start(4)
            .margin_top(4)
            .build();
        load_label.add_css_class("caption");

        // Add widgets to the monitor area
        monitor_area.append(cpu_chart.widget());
        monitor_area.append(mem_chart.widget());
        monitor_area.append(disk_chart.widget());
        monitor_area.append(net_chart.widget());
        monitor_area.append(&load_label);

        // --- Set up data channel ---
        let (tx, rx) = mpsc::channel::<MetricSnapshot>();

        // --- Background collection thread ---
        let stop = Arc::clone(&stop_flag);
        let addr = format!("{}:{}", host, port);
        let user = user.to_string();
        let password = password.map(|s| s.to_string());
        let key_path = key_path.map(|s| s.to_string());

        std::thread::spawn(move || {
            // Open a dedicated SSH session for monitoring
            let session = match open_monitor_session(&addr, &user, password.as_deref(), key_path.as_deref()) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("Monitor SSH connect failed: {}", e);
                    return;
                }
            };

            let mut prev_cpu: Option<CpuMetric> = None;
            let mut prev_net: Option<NetworkMetric> = None;

            while !stop.load(Ordering::Relaxed) {
                // Execute the collection command via an exec channel
                let output = match exec_command(&session, collect_command()) {
                    Ok(o) => o,
                    Err(e) => {
                        log::warn!("Monitor exec failed: {}", e);
                        // If the session is dead, stop
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        continue;
                    }
                };

                // Split output by sentinel markers
                let sections = split_sections(&output);

                // --- CPU ---
                let cpu_percent = if let Some(stat_text) = sections.get("STAT") {
                    let cpus = parse_proc_stat_cpu(stat_text);
                    // Use the aggregate "cpu" line (first entry)
                    let pct = if let (Some(curr), Some(prev)) = (cpus.first(), prev_cpu.as_ref()) {
                        curr.usage_percent(prev)
                    } else {
                        0.0
                    };
                    prev_cpu = cpus.into_iter().next();
                    pct
                } else {
                    0.0
                };

                // --- Memory ---
                let (mem_used_pct, mem_text) = if let Some(mem_text) = sections.get("MEM") {
                    if let Some(mem) = parse_proc_meminfo(mem_text) {
                        let used = mem.total_kb.saturating_sub(mem.available_kb);
                        let pct = if mem.total_kb > 0 {
                            (used as f64 / mem.total_kb as f64) * 100.0
                        } else {
                            0.0
                        };
                        let text = format!(
                            "{:.0} / {:.0} MB",
                            used as f64 / 1024.0,
                            mem.total_kb as f64 / 1024.0
                        );
                        (pct, text)
                    } else {
                        (0.0, String::new())
                    }
                } else {
                    (0.0, String::new())
                };

                // --- Disk ---
                let disk_items = if let Some(disk_text) = sections.get("DISK") {
                    parse_df_output(disk_text)
                        .into_iter()
                        .filter(|d| {
                            d.mount_point == "/"
                                || d.mount_point.starts_with("/home")
                                || d.mount_point.starts_with("/data")
                        })
                        .map(|d| {
                            let label = format!("{} ({})", d.mount_point, d.size);
                            (label, d.use_percent as f64 / 100.0)
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![]
                };

                // --- Network ---
                let (net_rx_rate, net_tx_rate, net_text) = if let Some(net_text) = sections.get("NET") {
                    let nets = parse_proc_net_dev(net_text);
                    // Sum all non-loopback interfaces
                    let total_rx: u64 = nets.iter().map(|n| n.rx_bytes).sum();
                    let total_tx: u64 = nets.iter().map(|n| n.tx_bytes).sum();

                    let (rx_rate, tx_rate) = if let Some(prev) = prev_net.as_ref() {
                        let dt = 2.0; // collection interval in seconds
                        let rx_d = total_rx.saturating_sub(prev.rx_bytes) as f64 / dt;
                        let tx_d = total_tx.saturating_sub(prev.tx_bytes) as f64 / dt;
                        (rx_d, tx_d)
                    } else {
                        (0.0, 0.0)
                    };

                    prev_net = Some(NetworkMetric {
                        interface: "all".to_string(),
                        rx_bytes: total_rx,
                        tx_bytes: total_tx,
                    });

                    let text = format!(
                        "RX {}/s  TX {}/s",
                        human_bytes(rx_rate),
                        human_bytes(tx_rate)
                    );
                    (rx_rate, tx_rate, text)
                } else {
                    (0.0, 0.0, String::new())
                };

                // --- Load ---
                let load_text = if let Some(load_raw) = sections.get("LOAD") {
                    if let Some(load) = parse_loadavg(load_raw) {
                        format!(
                            "Load: {:.2}  {:.2}  {:.2}",
                            load.load_1m, load.load_5m, load.load_15m
                        )
                    } else {
                        "Load: --".to_string()
                    }
                } else {
                    "Load: --".to_string()
                };

                let snapshot = MetricSnapshot {
                    cpu_percent,
                    mem_used_percent: mem_used_pct,
                    mem_text,
                    disk_items,
                    net_rx_rate,
                    net_tx_rate,
                    net_text,
                    load_text,
                };

                if tx.send(snapshot).is_err() {
                    break; // receiver dropped, UI is gone
                }

                std::thread::sleep(std::time::Duration::from_secs(2));
            }

            log::info!("Monitor collection thread stopped.");
        });

        // --- Main-thread timer: drain channel and update charts ---
        let cpu_buf = Arc::new(Mutex::new(MetricBuffer::<f64>::new(BUFFER_CAPACITY)));
        let mem_buf = Arc::new(Mutex::new(MetricBuffer::<f64>::new(BUFFER_CAPACITY)));
        let net_rx_buf = Arc::new(Mutex::new(MetricBuffer::<f64>::new(BUFFER_CAPACITY)));
        let net_tx_buf = Arc::new(Mutex::new(MetricBuffer::<f64>::new(BUFFER_CAPACITY)));

        let stop_ui = Arc::clone(&stop_flag);
        let rx = Mutex::new(rx);

        glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
            if stop_ui.load(Ordering::Relaxed) {
                return glib::ControlFlow::Break;
            }

            let rx = rx.lock().unwrap();
            let mut latest: Option<MetricSnapshot> = None;

            // Drain all pending snapshots, keep the latest for disk/load text
            while let Ok(snap) = rx.try_recv() {
                {
                    let mut buf = cpu_buf.lock().unwrap();
                    buf.push(snap.cpu_percent);
                }
                {
                    let mut buf = mem_buf.lock().unwrap();
                    buf.push(snap.mem_used_percent);
                }
                {
                    let mut buf = net_rx_buf.lock().unwrap();
                    buf.push(snap.net_rx_rate);
                }
                {
                    let mut buf = net_tx_buf.lock().unwrap();
                    buf.push(snap.net_tx_rate);
                }
                latest = Some(snap);
            }

            if let Some(snap) = latest {
                // Update CPU chart
                {
                    let buf = cpu_buf.lock().unwrap();
                    let data = buf.as_slice();
                    cpu_chart.set_value_text(&format!("{:.1}%", snap.cpu_percent));
                    cpu_chart.update_line_chart(&data, 100.0, (0.2, 0.6, 1.0));
                }

                // Update Memory chart
                {
                    let buf = mem_buf.lock().unwrap();
                    let data = buf.as_slice();
                    mem_chart.set_value_text(&snap.mem_text);
                    mem_chart.update_line_chart(&data, 100.0, (0.4, 0.8, 0.3));
                }

                // Update Disk chart (bar chart via draw function)
                {
                    let items = snap.disk_items.clone();
                    let height = if items.is_empty() { 30 } else { items.len() as i32 * 32 + 8 };
                    disk_chart.widget().set_height_request(height);
                    disk_chart.set_value_text("");
                    // Set a custom draw func for the bar chart
                    disk_chart.set_bar_chart(&items, (0.9, 0.5, 0.2));
                }

                // Update Network chart
                {
                    let rx_data = net_rx_buf.lock().unwrap().as_slice();
                    let tx_data = net_tx_buf.lock().unwrap().as_slice();
                    // Use the max of both series for the Y-axis scale
                    let max_val = rx_data
                        .iter()
                        .chain(tx_data.iter())
                        .cloned()
                        .fold(1.0_f64, f64::max);
                    net_chart.set_value_text(&snap.net_text);
                    // Show RX as primary line
                    net_chart.update_line_chart(&rx_data, max_val, (0.3, 0.8, 0.9));
                }

                // Update Load label
                load_label.set_label(&snap.load_text);
            }

            glib::ControlFlow::Continue
        });

        Self { stop_flag }
    }

    /// Stop the monitoring session. The background thread and UI timer will
    /// exit on their next iteration.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

impl Drop for MonitorSession {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Open a dedicated SSH session for monitoring using the same credentials.
fn open_monitor_session(
    addr: &str,
    user: &str,
    password: Option<&str>,
    key_path: Option<&str>,
) -> Result<ssh2::Session, String> {
    use std::net::TcpStream;

    let tcp = TcpStream::connect(addr).map_err(|e| format!("Monitor TCP connect: {}", e))?;
    let mut sess = ssh2::Session::new().map_err(|e| format!("Session create: {}", e))?;
    sess.set_tcp_stream(tcp);
    sess.handshake().map_err(|e| format!("SSH handshake: {}", e))?;

    if let Some(pw) = password {
        sess.userauth_password(user, pw)
            .map_err(|e| format!("Password auth: {}", e))?;
    } else if let Some(kp) = key_path {
        let expanded = shellexpand::tilde(kp).to_string();
        let path = std::path::Path::new(&expanded);
        sess.userauth_pubkey_file(user, None, path, None)
            .map_err(|e| format!("Key auth: {}", e))?;
    } else {
        let mut agent = sess.agent().map_err(|e| format!("SSH agent: {}", e))?;
        agent.connect().map_err(|e| format!("Agent connect: {}", e))?;
        agent.list_identities().map_err(|e| format!("Agent list: {}", e))?;
        let mut authenticated = false;
        for identity in agent.identities().unwrap_or_default() {
            if agent.userauth(user, &identity).is_ok() {
                authenticated = true;
                break;
            }
        }
        if !authenticated {
            return Err("SSH agent authentication failed".to_string());
        }
    }

    if !sess.authenticated() {
        return Err("Authentication failed".to_string());
    }

    Ok(sess)
}

/// Execute a command on the SSH session via an exec channel and return stdout.
fn exec_command(session: &ssh2::Session, command: &str) -> Result<String, String> {
    let mut channel = session
        .channel_session()
        .map_err(|e| format!("channel_session: {}", e))?;
    channel
        .exec(command)
        .map_err(|e| format!("exec: {}", e))?;

    let mut output = String::new();
    channel
        .read_to_string(&mut output)
        .map_err(|e| format!("read: {}", e))?;
    channel.wait_close().ok();
    Ok(output)
}

/// Split the combined output from `collect_command()` into named sections.
fn split_sections(output: &str) -> std::collections::HashMap<&str, &str> {
    let mut map = std::collections::HashMap::new();
    let markers = ["===STAT===", "===MEM===", "===DISK===", "===NET===", "===LOAD===", "===PS===", "===END==="];

    for i in 0..markers.len() - 1 {
        let start_marker = markers[i];
        let end_marker = markers[i + 1];

        if let Some(start_pos) = output.find(start_marker) {
            let content_start = start_pos + start_marker.len();
            if let Some(end_pos) = output[content_start..].find(end_marker) {
                let key = start_marker
                    .trim_start_matches("===")
                    .trim_end_matches("===");
                map.insert(key, &output[content_start..content_start + end_pos]);
            }
        }
    }

    map
}

/// Format a byte rate as a human-readable string (B, KB, MB).
fn human_bytes(bytes: f64) -> String {
    if bytes < 1024.0 {
        format!("{:.0} B", bytes)
    } else if bytes < 1024.0 * 1024.0 {
        format!("{:.1} KB", bytes / 1024.0)
    } else {
        format!("{:.1} MB", bytes / (1024.0 * 1024.0))
    }
}
