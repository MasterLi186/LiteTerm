use std::io::Read;

use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct ProcessDetail {
    pub pid: u32,
    pub user: String,
    pub mem: String,
    pub cpu: f32,
    pub command: String,
    pub full_command: String,
    pub location: String,
}

#[derive(Serialize, Clone)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
}

#[derive(Serialize, Clone)]
pub struct ProcessFullDetail {
    pub pid: u32,
    pub user: String,
    pub mem: String,
    pub cpu: f32,
    pub command: String,
    pub full_command: String,
    pub location: String,
    pub working_dir: String,
    pub environ: Vec<EnvVar>,
}

/// Open an SSH connection, execute a command, and return stdout as a String.
fn open_ssh_and_exec(
    host: &str,
    port: u16,
    user: &str,
    password: Option<&str>,
    auth_method: &str,
    key_path: Option<&str>,
    command: &str,
) -> Result<String, String> {
    let addr = format!("{}:{}", host, port);
    let sock_addr: std::net::SocketAddr = addr.parse().map_err(|e| format!("Invalid address: {}", e))?;

    let tcp = std::net::TcpStream::connect_timeout(&sock_addr, std::time::Duration::from_secs(10))
        .map_err(|e| format!("TCP connect failed: {}", e))?;

    let mut session = ssh2::Session::new().map_err(|e| format!("SSH session failed: {}", e))?;
    session.set_tcp_stream(tcp);
    session.handshake().map_err(|e| format!("SSH handshake failed: {}", e))?;

    match auth_method {
        "agent" => session
            .userauth_agent(user)
            .map_err(|e| format!("Agent auth failed: {}", e))?,
        "key" => {
            let key = key_path.unwrap_or_default();
            let expanded = shellexpand::tilde(key);
            session
                .userauth_pubkey_file(
                    user,
                    None,
                    std::path::Path::new(expanded.as_ref()),
                    password,
                )
                .map_err(|e| format!("Key auth failed: {}", e))?;
        }
        _ => {
            let pw = password.unwrap_or_default();
            session
                .userauth_password(user, pw)
                .map_err(|e| format!("Password auth failed: {}", e))?;
        }
    }

    session.set_blocking(true);

    let mut channel = session.channel_session().map_err(|e| format!("Channel failed: {}", e))?;
    channel.exec(command).map_err(|e| format!("Exec failed: {}", e))?;

    let mut output = String::new();
    channel.read_to_string(&mut output).map_err(|e| format!("Read failed: {}", e))?;
    channel.wait_close().ok();
    Ok(output)
}

/// Format RSS in kilobytes to a human-readable string.
fn format_rss_kb(kb: u64) -> String {
    if kb >= 1_048_576 {
        format!("{:.1}G", kb as f64 / 1_048_576.0)
    } else if kb >= 1024 {
        format!("{:.1}M", kb as f64 / 1024.0)
    } else {
        format!("{}K", kb)
    }
}

/// Parse one line of `ps aux` output into a ProcessDetail.
/// Columns: USER PID %CPU %MEM VSZ RSS TTY STAT START TIME COMMAND...
fn parse_ps_line(line: &str) -> Option<ProcessDetail> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 11 {
        return None;
    }

    let user = parts[0].to_string();
    let pid: u32 = parts[1].parse().ok()?;
    let cpu: f32 = parts[2].parse().ok()?;
    // parts[3] = %MEM, parts[4] = VSZ
    let rss_kb: u64 = parts[5].parse().ok()?;
    let mem = format_rss_kb(rss_kb);
    let full_command = parts[10..].join(" ");
    let command = parts[10]
        .rsplit('/')
        .next()
        .unwrap_or(parts[10])
        .to_string();

    Some(ProcessDetail {
        pid,
        user,
        mem,
        cpu,
        command,
        full_command,
        location: String::new(),
    })
}

#[tauri::command]
pub async fn get_process_list(
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    auth_method: String,
    key_path: Option<String>,
) -> Result<Vec<ProcessDetail>, String> {
    // Run on a blocking thread since ssh2 is !Send
    let result = tokio::task::spawn_blocking(move || {
        // Get process list sorted by CPU
        let ps_output = open_ssh_and_exec(
            &host,
            port,
            &user,
            password.as_deref(),
            &auth_method,
            key_path.as_deref(),
            "ps aux --sort=-%cpu",
        )?;

        let mut processes: Vec<ProcessDetail> = Vec::new();
        for line in ps_output.lines().skip(1) {
            // skip header
            if let Some(p) = parse_ps_line(line) {
                processes.push(p);
            }
        }

        // For the top 50 processes, batch-fetch exe locations
        if !processes.is_empty() {
            let top_n = std::cmp::min(50, processes.len());
            let pids: Vec<String> = processes[..top_n]
                .iter()
                .map(|p| p.pid.to_string())
                .collect();

            // Build a single command that reads all exe links
            let readlink_cmd = pids
                .iter()
                .map(|pid| format!("echo \"{}:$(readlink /proc/{}/exe 2>/dev/null)\"", pid, pid))
                .collect::<Vec<_>>()
                .join("; ");

            let link_output = open_ssh_and_exec(
                &host,
                port,
                &user,
                password.as_deref(),
                &auth_method,
                key_path.as_deref(),
                &readlink_cmd,
            )
            .unwrap_or_default();

            // Parse "PID:/path/to/exe" lines into a map
            let mut loc_map = std::collections::HashMap::new();
            for line in link_output.lines() {
                if let Some((pid_str, path)) = line.split_once(':') {
                    if let Ok(pid) = pid_str.parse::<u32>() {
                        loc_map.insert(pid, path.to_string());
                    }
                }
            }

            for p in &mut processes[..top_n] {
                if let Some(loc) = loc_map.get(&p.pid) {
                    p.location = loc.clone();
                }
            }
        }

        Ok(processes)
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?;

    result
}

#[tauri::command]
pub async fn get_process_detail(
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    auth_method: String,
    key_path: Option<String>,
    pid: u32,
) -> Result<ProcessFullDetail, String> {
    let result = tokio::task::spawn_blocking(move || {
        // Build a combined command to collect all details in one SSH round-trip
        let cmd = format!(
            concat!(
                "echo '===EXE==='; readlink /proc/{pid}/exe 2>/dev/null; ",
                "echo '===CWD==='; readlink /proc/{pid}/cwd 2>/dev/null; ",
                "echo '===CMD==='; cat /proc/{pid}/cmdline 2>/dev/null | tr '\\0' ' '; echo; ",
                "echo '===ENV==='; cat /proc/{pid}/environ 2>/dev/null | tr '\\0' '\\n'; ",
                "echo '===PS==='; ps -p {pid} -o user=,rss=,%cpu=,%mem=,comm= 2>/dev/null; ",
                "echo '===END==='"
            ),
            pid = pid
        );

        let output = open_ssh_and_exec(
            &host,
            port,
            &user,
            password.as_deref(),
            &auth_method,
            key_path.as_deref(),
            &cmd,
        )?;

        let mut exe = String::new();
        let mut cwd = String::new();
        let mut cmdline = String::new();
        let mut environ_lines: Vec<String> = Vec::new();
        let mut ps_line = String::new();

        let mut section = "";
        for line in output.lines() {
            let trimmed = line.trim();
            match trimmed {
                "===EXE===" => { section = "exe"; continue; }
                "===CWD===" => { section = "cwd"; continue; }
                "===CMD===" => { section = "cmd"; continue; }
                "===ENV===" => { section = "env"; continue; }
                "===PS===" => { section = "ps"; continue; }
                "===END===" => { section = ""; continue; }
                _ => {}
            }
            match section {
                "exe" => { exe = trimmed.to_string(); }
                "cwd" => { cwd = trimmed.to_string(); }
                "cmd" => { cmdline = trimmed.to_string(); }
                "env" => {
                    if !trimmed.is_empty() {
                        environ_lines.push(trimmed.to_string());
                    }
                }
                "ps" => {
                    if !trimmed.is_empty() {
                        ps_line = trimmed.to_string();
                    }
                }
                _ => {}
            }
        }

        // Parse ps line: USER RSS %CPU %MEM COMMAND
        let ps_parts: Vec<&str> = ps_line.split_whitespace().collect();
        let (ps_user, rss_kb, cpu, command) = if ps_parts.len() >= 5 {
            let u = ps_parts[0].to_string();
            let rss: u64 = ps_parts[1].parse().unwrap_or(0);
            let c: f32 = ps_parts[2].parse().unwrap_or(0.0);
            let cmd = ps_parts[4].to_string();
            (u, rss, c, cmd)
        } else {
            (String::new(), 0, 0.0, String::new())
        };

        // Parse environ
        let environ: Vec<EnvVar> = environ_lines
            .iter()
            .filter_map(|line| {
                let (key, value) = line.split_once('=')?;
                Some(EnvVar {
                    key: key.to_string(),
                    value: value.to_string(),
                })
            })
            .collect();

        Ok(ProcessFullDetail {
            pid,
            user: ps_user,
            mem: format_rss_kb(rss_kb),
            cpu,
            command,
            full_command: cmdline.trim().to_string(),
            location: exe,
            working_dir: cwd,
            environ,
        })
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?;

    result
}
