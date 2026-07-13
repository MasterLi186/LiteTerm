use std::io::{Read, Write};

use portable_pty::{CommandBuilder, PtySize};
use tauri::{AppHandle, Emitter, State};

use crate::app_log;
use crate::state::{AppState, LocalTerminal};

fn default_shell() -> String {
    #[cfg(unix)]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
    #[cfg(windows)]
    {
        // 优先 Git Bash → MSYS2 Bash → PowerShell 7 → Windows PowerShell → CMD
        let candidates = [
            r"C:\Program Files\Git\bin\bash.exe",
            r"C:\Program Files (x86)\Git\bin\bash.exe",
            r"C:\msys64\usr\bin\bash.exe",
            r"C:\Program Files\PowerShell\7\pwsh.exe",
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            r"C:\Windows\System32\cmd.exe",
        ];
        for p in &candidates {
            if std::path::Path::new(p).exists() {
                return p.to_string();
            }
        }
        "cmd.exe".to_string()
    }
}

#[derive(serde::Serialize)]
pub struct ShellInfo {
    pub name: String,
    pub path: String,
}

/// 本地终端核心逻辑(供 Tauri 命令和 HTTP API 共用)
pub fn do_open_terminal(state: &AppState, app: &AppHandle, shell_path: Option<String>) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();

    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| {
            app_log!("TERM", "ERROR: PTY open failed: {}", e);
            e.to_string()
        })?;

    let shell = shell_path.unwrap_or_else(default_shell);
    let cmd = CommandBuilder::new(&shell);
    let _child = pair.slave.spawn_command(cmd).map_err(|e| {
        app_log!("TERM", "ERROR: spawn shell failed: {} (shell={})", e, shell);
        e.to_string()
    })?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| {
            app_log!("TERM", "ERROR: clone reader failed: {}", e);
            e.to_string()
        })?;
    let writer = pair.master.take_writer().map_err(|e| {
        app_log!("TERM", "ERROR: take writer failed: {}", e);
        e.to_string()
    })?;

    // 输入通道: 前端/API -> writer 线程
    let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();

    // PTY + shell 创建成功后才初始化输出缓冲区（避免失败路径泄漏 1MB）
    state.output_buffers.lock().unwrap().insert(id.clone(), crate::state::TerminalOutputBuffer::new(1_048_576));

    // Reader 线程: PTY 输出 -> Tauri 事件 + 输出缓冲区
    let id_clone = id.clone();
    let app_clone = app.clone();
    let output_bufs = state.output_buffers.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let mut reader = reader;
        let mut read_buf = [0u8; 4096];
        loop {
            match reader.read(&mut read_buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = app_clone.emit(
                        "terminal-output",
                        serde_json::json!({
                            "id": id_clone,
                            "data": &read_buf[..n],
                        }),
                    );
                    // 写入输出缓冲区供 HTTP API 读取
                    if let Ok(mut bufs) = output_bufs.lock() {
                        if let Some(ob) = bufs.get_mut(&id_clone) {
                            ob.write(&read_buf[..n]);
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });
    std::thread::spawn(move || {
        let mut writer = writer;
        while let Ok(data) = input_rx.recv() {
            let _ = writer.write_all(&data);
            let _ = writer.flush();
        }
    });

    // Resize 通道
    let (resize_tx, resize_rx) = std::sync::mpsc::channel::<(u32, u32)>();
    let master = pair.master;
    std::thread::spawn(move || {
        while let Ok((cols, rows)) = resize_rx.recv() {
            let _ = master.resize(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    });

    state
        .local_terminals
        .lock()
        .unwrap()
        .insert(id.clone(), LocalTerminal {
            id: id.clone(),
            input_tx,
            resize_tx,
            stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });

    Ok(id)
}

#[tauri::command]
pub async fn open_local_terminal(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<String, String> {
    do_open_terminal(&state, &app, None)
}

#[tauri::command]
pub async fn list_shells() -> Result<Vec<ShellInfo>, String> {
    let mut shells = Vec::new();

    #[cfg(unix)]
    {
        if let Ok(content) = std::fs::read_to_string("/etc/shells") {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('/') && std::path::Path::new(line).exists() {
                    let name = std::path::Path::new(line)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    shells.push(ShellInfo { name, path: line.to_string() });
                }
            }
        }
    }

    #[cfg(windows)]
    {
        // Git Bash（优先）
        let git_bash_paths = [
            r"C:\Program Files\Git\bin\bash.exe",
            r"C:\Program Files (x86)\Git\bin\bash.exe",
        ];
        for p in &git_bash_paths {
            if std::path::Path::new(p).exists() {
                shells.push(ShellInfo { name: "Git Bash".to_string(), path: p.to_string() });
                break;
            }
        }
        // MSYS2 Bash
        let msys2_path = r"C:\msys64\usr\bin\bash.exe";
        if std::path::Path::new(msys2_path).exists() {
            shells.push(ShellInfo { name: "MSYS2 Bash".to_string(), path: msys2_path.to_string() });
        }
        // WSL
        let wsl_path = r"C:\Windows\System32\wsl.exe";
        if std::path::Path::new(wsl_path).exists() {
            shells.push(ShellInfo { name: "WSL".to_string(), path: wsl_path.to_string() });
        }
        // PowerShell 7+
        let pwsh_paths = [
            r"C:\Program Files\PowerShell\7\pwsh.exe",
            r"C:\Program Files (x86)\PowerShell\7\pwsh.exe",
        ];
        for p in &pwsh_paths {
            if std::path::Path::new(p).exists() {
                shells.push(ShellInfo { name: "PowerShell 7".to_string(), path: p.to_string() });
                break;
            }
        }
        // Windows PowerShell（兜底）
        let win_ps = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
        if std::path::Path::new(win_ps).exists() {
            shells.push(ShellInfo { name: "PowerShell".to_string(), path: win_ps.to_string() });
        }
        // CMD（最后兜底）
        let cmd_path = r"C:\Windows\System32\cmd.exe";
        if std::path::Path::new(cmd_path).exists() {
            shells.push(ShellInfo { name: "CMD".to_string(), path: cmd_path.to_string() });
        }
    }

    shells.dedup_by(|a, b| a.name == b.name);
    Ok(shells)
}

#[tauri::command]
pub async fn open_shell_terminal(
    state: State<'_, AppState>,
    app: AppHandle,
    shell_path: String,
) -> Result<String, String> {
    do_open_terminal(&state, &app, Some(shell_path))
}

#[tauri::command]
pub async fn terminal_write(
    state: State<'_, AppState>,
    id: String,
    data: Vec<u8>,
) -> Result<(), String> {
    if let Some(term) = state.local_terminals.lock().unwrap().get(&id) {
        term.input_tx.send(data).map_err(|e| e.to_string())?;
        return Ok(());
    }
    if let Some(session) = state.sessions.lock().unwrap().get(&id) {
        session.input_tx.send(data).map_err(|e| e.to_string())?;
        return Ok(());
    }
    Err("terminal not found".to_string())
}

#[tauri::command]
pub async fn terminal_resize(
    state: State<'_, AppState>,
    id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    if let Some(term) = state.local_terminals.lock().unwrap().get(&id) {
        term.resize_tx
            .send((cols, rows))
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    if let Some(session) = state.sessions.lock().unwrap().get(&id) {
        session
            .resize_tx
            .send((cols, rows))
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    Err("terminal not found".to_string())
}

#[tauri::command]
pub async fn close_terminal(state: State<'_, AppState>, id: String) -> Result<(), String> {
    if let Some(term) = state.local_terminals.lock().unwrap().remove(&id) {
        term.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        // drop input_tx 解除 writer 线程阻塞，drop resize_tx 解除 resize 线程阻塞
        // resize 线程持有 PTY master，释放后 reader 线程会收到 EOF 退出
        drop(term.input_tx);
        drop(term.resize_tx);
    }
    if let Some(session) = state.sessions.lock().unwrap().remove(&id) {
        session
            .monitor_stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    // SFTP 会话与终端共用同一 id;关终端时一并回收。否则 sftp_sessions 只 insert 从不
    // remove —— 每次关闭/掉线重连都会泄漏一条活的 SFTP 连接(TcpStream + libssh2 Session
    // + 30s keepalive + 1 个 fd),长跑会单调耗尽文件描述符/连接数。
    state.sftp_sessions.lock().unwrap().remove(&id);
    // 清理 HTTP API 相关资源
    state.output_buffers.lock().unwrap().remove(&id);
    state.tab_registry.lock().unwrap().remove(&id);
    Ok(())
}

/// 本地进程列表(sysinfo,跨平台)
#[tauri::command]
pub async fn get_local_processes() -> Result<serde_json::Value, String> {
    use sysinfo::{System, ProcessesToUpdate};
    let mut sys = System::new();
    sys.refresh_cpu_all();
    std::thread::sleep(std::time::Duration::from_millis(200));
    sys.refresh_cpu_all();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let num_cpus = sys.cpus().len().max(1) as f32;
    let mut procs: Vec<(&sysinfo::Pid, &sysinfo::Process)> = sys.processes().iter().collect();
    procs.sort_by(|a, b| b.1.cpu_usage().partial_cmp(&a.1.cpu_usage()).unwrap_or(std::cmp::Ordering::Equal));
    let list: Vec<serde_json::Value> = procs.iter().take(100).map(|(pid, p)| {
        let mem = p.memory();
        let mem_str = if mem >= 1_073_741_824 { format!("{:.1}G", mem as f64 / 1_073_741_824.0) }
            else if mem >= 1_048_576 { format!("{:.1}M", mem as f64 / 1_048_576.0) }
            else { format!("{}K", mem / 1024) };
        serde_json::json!({
            "pid": pid.as_u32(),
            "user": "",
            "cpu": p.cpu_usage() / num_cpus,
            "mem": mem_str,
            "command": p.name().to_string_lossy(),
            "full_command": p.cmd().iter().map(|s| s.to_string_lossy().to_string()).collect::<Vec<_>>().join(" "),
            "start_time": p.start_time(),
        })
    }).collect();
    Ok(serde_json::json!(list))
}

/// 本地进程详情(sysinfo)
#[tauri::command]
pub async fn get_local_process_detail(pid: u32) -> Result<serde_json::Value, String> {
    use sysinfo::{System, ProcessesToUpdate, Pid};
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let target = Pid::from_u32(pid);
    let p = sys.process(target).ok_or("进程未找到")?;
    let mem = p.memory();
    let mem_str = if mem >= 1_073_741_824 { format!("{:.1}G", mem as f64 / 1_073_741_824.0) }
        else if mem >= 1_048_576 { format!("{:.1}M", mem as f64 / 1_048_576.0) }
        else { format!("{}K", mem / 1024) };
    let parent_pid = p.parent().map(|pp| pp.as_u32());
    // 向上追溯进程树
    let mut ancestors = Vec::new();
    let mut cur = Some(target);
    for _ in 0..50 {
        match cur {
            Some(cp) if cp.as_u32() > 1 => {
                if let Some(proc) = sys.process(cp) {
                    ancestors.push(serde_json::json!({
                        "pid": cp.as_u32(),
                        "name": proc.name().to_string_lossy(),
                        "cmdline": proc.cmd().iter().map(|s| s.to_string_lossy().to_string()).collect::<Vec<_>>().join(" "),
                    }));
                    cur = proc.parent();
                } else { break; }
            }
            _ => break,
        }
    }
    Ok(serde_json::json!({
        "pid": pid,
        "user": "",
        "cpu": 0,
        "mem": mem_str,
        "command": p.name().to_string_lossy(),
        "full_command": p.cmd().iter().map(|s| s.to_string_lossy().to_string()).collect::<Vec<_>>().join(" "),
        "location": p.exe().map(|e| e.to_string_lossy().to_string()).unwrap_or_default(),
        "working_dir": p.cwd().map(|c| c.to_string_lossy().to_string()).unwrap_or_default(),
        "start_time": p.start_time(),
        "parent_pid": parent_pid,
        "environ": p.environ().iter().take(50).map(|s| s.to_string_lossy().to_string()).collect::<Vec<String>>(),
        "ancestors": ancestors,
    }))
}

/// 返回系统信息(关于对话框用)
#[tauri::command]
pub async fn get_system_info(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let username = whoami::username();

    Ok(serde_json::json!({
        "app_version": app.package_info().version.to_string(),
        "os": os,
        "arch": arch,
        "hostname": hostname,
        "username": username,
    }))
}

/// 返回当前用户的默认 shell(SHELL 环境变量)
#[tauri::command]
pub async fn get_default_shell() -> Result<String, String> {
    std::env::var("SHELL").map_err(|e| format!("获取 SHELL 失败: {}", e))
}

/// 前端日志桥:把前端的 appLog 写入 ~/guishell.log
#[tauri::command]
pub async fn frontend_log(category: String, message: String) -> Result<(), String> {
    crate::app_log!("FE", "[{}] {}", category, message);
    Ok(())
}

/// 判断目标是否为带 scheme 的 URI（http/https/mailto/tel/file 等）。
/// 用于终端链接：URI 直接交给系统默认程序，路径则先解析再打开。
pub fn is_external_uri(target: &str) -> bool {
    let t = target.trim();
    if t.is_empty() || t.starts_with('/') || t.starts_with('.') || t.starts_with('~') {
        return false;
    }
    // scheme: 以字母开头，后接字母/数字/+/-. ，再跟 :
    let bytes = t.as_bytes();
    if !bytes[0].is_ascii_alphabetic() {
        return false;
    }
    let mut i = 1;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b':' {
            // 至少形如 "a:"，且常见为 "http:" / "mailto:" 等
            return i >= 2;
        }
        if !(c.is_ascii_alphanumeric() || c == b'+' || c == b'-' || c == b'.') {
            return false;
        }
        i += 1;
    }
    false
}

/// 用系统默认程序打开 URL 或本地路径（按后缀/协议关联）。
/// Linux: xdg-open；macOS: open；Windows: cmd start。
fn open_with_system_default(target: &str) -> Result<(), String> {
    app_log!("OPEN", "系统默认程序打开: {}", target);

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(target)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("打开失败 (xdg-open): {}", e))?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(target)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("打开失败 (open): {}", e))?;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        // start 的第一个引号参数是窗口标题，必须传空串，否则 URL/路径会被当成标题
        std::process::Command::new("cmd")
            .args(["/C", "start", "", target])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("打开失败 (start): {}", e))?;
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err(format!("当前平台不支持用默认程序打开: {}", target))
    }
}

/// 解析终端中的本地文件路径（~、相对路径基于 OSC7 cwd 或 HOME）。
fn resolve_local_path(state: &AppState, terminal_id: Option<&str>, path: &str) -> String {
    let expanded = shellexpand::tilde(path).to_string();

    if std::path::Path::new(&expanded).is_absolute() {
        return expanded;
    }

    let cwd = terminal_id.and_then(|id| {
        state
            .sessions
            .lock()
            .unwrap()
            .get(id)
            .and_then(|s| s.osc7_cwd.lock().unwrap().clone())
    });

    if let Some(cwd) = cwd {
        std::path::Path::new(&cwd)
            .join(&expanded)
            .to_string_lossy()
            .to_string()
    } else {
        let home = dirs::home_dir().unwrap_or_default();
        home.join(&expanded).to_string_lossy().to_string()
    }
}

/// 核心逻辑：用系统默认程序打开 URI 或本地文件路径。
fn do_open_with_default_app(
    state: &AppState,
    target: &str,
    terminal_id: Option<&str>,
) -> Result<(), String> {
    let target = target.trim();
    if target.is_empty() {
        return Err("打开目标为空".into());
    }

    if is_external_uri(target) {
        return open_with_system_default(target);
    }

    let resolved = resolve_local_path(state, terminal_id, target);
    if !std::path::Path::new(&resolved).exists() {
        return Err(format!("文件不存在: {}", resolved));
    }
    open_with_system_default(&resolved)
}

/// 用系统默认程序打开终端里的 URL 或本地文件。
/// - URL（http/https/mailto/…）：直接交给系统协议处理器（浏览器等）
/// - 文件路径：解析相对路径后按后缀用默认应用打开
#[tauri::command]
pub async fn open_with_default_app(
    state: State<'_, AppState>,
    target: String,
    id: Option<String>,
) -> Result<(), String> {
    do_open_with_default_app(&state, &target, id.as_deref())
}

/// 打开终端里的文件路径（兼容旧接口）。
#[tauri::command]
pub async fn open_file_path(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> Result<(), String> {
    do_open_with_default_app(&state, &path, Some(&id))
}

#[cfg(test)]
mod open_uri_tests {
    use super::is_external_uri;

    #[test]
    fn detects_http_urls() {
        assert!(is_external_uri("http://192.168.110.14:8000/admin/login"));
        assert!(is_external_uri("https://example.com/a?b=1"));
        assert!(is_external_uri("  HTTP://HOST/path  "));
        assert!(is_external_uri("mailto:user@example.com"));
        assert!(is_external_uri("file:///tmp/a.txt"));
    }

    #[test]
    fn rejects_file_paths() {
        assert!(!is_external_uri("/tmp/a.txt"));
        assert!(!is_external_uri("./rel/path"));
        assert!(!is_external_uri("../up"));
        assert!(!is_external_uri("~/Downloads/x.pdf"));
        assert!(!is_external_uri("readme.md"));
        assert!(!is_external_uri(""));
    }
}

/// HTTP API 标签页注册(前端打开标签时调用)
#[tauri::command]
pub async fn register_tab(state: State<'_, AppState>, id: String, label: String, tab_type: String) -> Result<(), String> {
    state.tab_registry.lock().unwrap().insert(id.clone(), crate::state::TabInfo { id, label, tab_type });
    Ok(())
}

/// HTTP API 标签页注销(前端关闭标签时调用)
#[tauri::command]
pub async fn unregister_tab(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.tab_registry.lock().unwrap().remove(&id);
    Ok(())
}

#[tauri::command]
pub fn force_quit() {
    crate::log_util::app_log("关闭", "force_quit: destroy 超时或失败,强制退出进程");
    std::process::exit(0);
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let settings = state.settings.lock().unwrap();
    serde_json::to_value(&*settings).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_settings(state: State<'_, AppState>, patch: serde_json::Value) -> Result<(), String> {
    let mut settings = state.settings.lock().unwrap();
    let mut current = serde_json::to_value(&*settings).map_err(|e| e.to_string())?;
    if let (Some(cur_obj), Some(patch_obj)) = (current.as_object_mut(), patch.as_object()) {
        for (key, val) in patch_obj {
            if let Some(existing) = cur_obj.get_mut(key) {
                if let (Some(e), Some(v)) = (existing.as_object_mut(), val.as_object()) {
                    for (k2, v2) in v {
                        e.insert(k2.clone(), v2.clone());
                    }
                } else {
                    cur_obj.insert(key.clone(), val.clone());
                }
            } else {
                cur_obj.insert(key.clone(), val.clone());
            }
        }
    }
    let updated: crate::config::settings::Settings = serde_json::from_value(current).map_err(|e| e.to_string())?;
    updated.save().map_err(|e| e.to_string())?;
    *settings = updated;
    Ok(())
}

