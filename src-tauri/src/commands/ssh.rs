use std::io::{Read, Write};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use portable_pty::{CommandBuilder, PtySize};
use tauri::{AppHandle, Emitter, State};

use crate::app_log;
use crate::state::{AppState, LocalTerminal, ManagedSession, SftpRequest};

/// SSH 连接核心逻辑(供 Tauri 命令和 HTTP API 共用)
#[allow(clippy::too_many_arguments)]
pub async fn do_ssh_connect(
    state: &AppState,
    app: &AppHandle,
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    auth_method: String,
    key_path: Option<String>,
    label: String,
    proxy_jump: Option<String>,
    cols: Option<u32>,
    rows: Option<u32>,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    // 初始化输出缓冲区供 HTTP API 增量拉取
    state.output_buffers.lock().unwrap().insert(id.clone(), crate::state::TerminalOutputBuffer::new(1_048_576));
    let timeout = state.settings.lock().unwrap().ssh.connect_timeout_secs;

    // ProxyJump path: use system SSH client via PTY
    if let Some(ref proxy) = proxy_jump {
        if !proxy.is_empty() {
            let pty_system = portable_pty::native_pty_system();
            let pair = pty_system
                .openpty(PtySize {
                    rows: rows.unwrap_or(36) as u16,
                    cols: cols.unwrap_or(120) as u16,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| format!("PTY open failed: {}", e))?;

            let mut cmd = CommandBuilder::new("ssh");
            cmd.arg("-J");
            cmd.arg(proxy);
            cmd.arg("-p");
            cmd.arg(port.to_string());
            cmd.arg("-o");
            cmd.arg("StrictHostKeyChecking=no");
            if let Some(ref kp) = key_path {
                if !kp.is_empty() {
                    let expanded = shellexpand::tilde(kp);
                    cmd.arg("-i");
                    cmd.arg(expanded.as_ref());
                }
            }
            cmd.arg(format!("{}@{}", user, host));

            let _child = pair
                .slave
                .spawn_command(cmd)
                .map_err(|e| format!("SSH spawn failed: {}", e))?;
            drop(pair.slave);

            let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
            let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

            let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();

            let id_clone = id.clone();
            let app_clone = app.clone();
            let output_bufs = state.output_buffers.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let mut reader = reader;
                let mut read_buf = [0u8; 4096];
                loop {
                    match reader.read(&mut read_buf) {
                        Ok(0) => {
                            let _ = app_clone.emit(
                                "terminal-closed",
                                serde_json::json!({"id": id_clone}),
                            );
                            break;
                        }
                        Ok(n) => {
                            let _ = app_clone.emit(
                                "terminal-output",
                                serde_json::json!({"id": id_clone, "data": &read_buf[..n]}),
                            );
                            // 写入输出缓冲区供 HTTP API 读取
                            if let Ok(mut bufs) = output_bufs.lock() {
                                if let Some(ob) = bufs.get_mut(&id_clone) {
                                    ob.write(&read_buf[..n]);
                                }
                            }
                        }
                        Err(_) => {
                            let _ = app_clone.emit(
                                "terminal-closed",
                                serde_json::json!({"id": id_clone}),
                            );
                            break;
                        }
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

            state.local_terminals.lock().unwrap().insert(
                id.clone(),
                LocalTerminal {
                    id: id.clone(),
                    input_tx,
                    resize_tx,
                    stop: Arc::new(AtomicBool::new(false)),
                },
            );

            return Ok(id);
        }
    }

    let id_clone = id.clone();
    let app_clone = app.clone();
    let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (resize_tx, resize_rx) = std::sync::mpsc::channel::<(u32, u32)>();
    let (status_tx, status_rx) = std::sync::mpsc::channel::<Result<(), String>>();

    // OSC7 cwd 跟踪（不门控）：原件存进 ManagedSession，克隆交给 reader 线程更新。
    let osc7_cwd: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let osc7_cwd_clone = osc7_cwd.clone();

    let zmodem_active = Arc::new(AtomicBool::new(false));
    let zmodem_request: Arc<Mutex<Option<crate::state::ZmodemSendRequest>>> = Arc::new(Mutex::new(None));
    let zmodem_active_clone = zmodem_active.clone();
    let zmodem_request_clone = zmodem_request.clone();
    let output_bufs = state.output_buffers.clone();

    std::thread::spawn(move || {
        app_log!("SSH", "SSH CONNECT START: {}:{} user={} auth={}", host, port, user, auth_method);

        // 1. TCP connect + SSH handshake
        let addr = format!("{}:{}", host, port);
        let sock_addr = match crate::core::net::resolve_addr(&addr) {
            Ok(a) => a,
            Err(e) => {
                app_log!("SSH", "ERROR: {}", e);
                let _ = status_tx.send(Err(e));
                return;
            }
        };
        let tcp = match std::net::TcpStream::connect_timeout(
            &sock_addr,
            std::time::Duration::from_secs(timeout as u64),
        ) {
            Ok(tcp) => { app_log!("SSH", "TCP connected to {}", addr); tcp },
            Err(e) => {
                app_log!("SSH", "ERROR: TCP connect failed: {}", e);
                let _ = status_tx.send(Err(format!("TCP connect failed: {}", e)));
                return;
            }
        };

        let mut session = match ssh2::Session::new() {
            Ok(s) => s,
            Err(e) => {
                app_log!("SSH", "ERROR: SSH session create failed: {}", e);
                let _ = status_tx.send(Err(format!("SSH session failed: {}", e)));
                return;
            }
        };

        // 记录客户端支持的加密算法（握手前）
        let alg_info = |mt: ssh2::MethodType, label: &str| -> String {
            match session.supported_algs(mt) {
                Ok(algs) => format!("{}: {}", label, algs.join(", ")),
                Err(_) => format!("{}: (无法获取)", label),
            }
        };
        let supported = format!(
            "客户端支持的算法:\n  {}\n  {}\n  {}\n  {}",
            alg_info(ssh2::MethodType::Kex, "密钥交换"),
            alg_info(ssh2::MethodType::HostKey, "主机密钥"),
            alg_info(ssh2::MethodType::CryptCs, "加密(C→S)"),
            alg_info(ssh2::MethodType::CryptSc, "加密(S→C)"),
        );
        app_log!("SSH", "{}", supported);

        session.set_tcp_stream(tcp);
        app_log!("SSH", "开始 SSH 握手...");
        if let Err(e) = session.handshake() {
            app_log!("SSH", "ERROR: SSH handshake failed: {}", e);
            app_log!("SSH", "{}", supported);
            let _ = status_tx.send(Err(format!("SSH handshake failed: {}", e)));
            return;
        }
        app_log!("SSH", "SSH 握手成功");

        // 记录协商后实际使用的算法
        let active_algs = |mt: ssh2::MethodType, label: &str| -> String {
            match session.methods(mt) {
                Some(m) => format!("{}: {}", label, m),
                None => format!("{}: (未知)", label),
            }
        };
        app_log!("SSH", "协商结果: {} | {} | {}",
            active_algs(ssh2::MethodType::Kex, "Kex"),
            active_algs(ssh2::MethodType::CryptCs, "Cipher"),
            active_algs(ssh2::MethodType::MacCs, "MAC"),
        );

        // 2. Authenticate
        app_log!("SSH", "开始认证: method={}", auth_method);
        let auth_result = match auth_method.as_str() {
            "agent" => session
                .userauth_agent(&user)
                .map_err(|e| format!("Agent auth failed: {}", e)),
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
                    .map_err(|e| format!("Key auth failed: {}", e))
            }
            _ => {
                let pw = password.unwrap_or_default();
                session
                    .userauth_password(&user, &pw)
                    .map_err(|e| format!("Password auth failed: {}", e))
            }
        };

        if let Err(e) = auth_result {
            app_log!("SSH", "ERROR: 认证失败: {}", e);
            let _ = status_tx.send(Err(e));
            return;
        }
        app_log!("SSH", "认证成功");

        // 3. Open shell channel with PTY
        let mut channel = match session.channel_session() {
            Ok(ch) => ch,
            Err(e) => {
                let _ = status_tx.send(Err(e.to_string()));
                return;
            }
        };
        let pty_cols = cols.unwrap_or(120);
        let pty_rows = rows.unwrap_or(36);
        // 标准 PTY 终端模式(对齐 OpenSSH 行为,解决 fish/zsh 切换时 ^J/⏎ 残留)
        // ECHO=false 使随后注入的 bash/zsh OSC7 钩子不回显;注入完成后 stty echo 恢复
        let mut pty_modes = ssh2::PtyModes::new();
        // 输入模式
        pty_modes.set_boolean(ssh2::PtyModeOpcode::ICRNL, true);  // CR→NL(输入)
        pty_modes.set_boolean(ssh2::PtyModeOpcode::IXON, true);   // 输出流控
        pty_modes.set_boolean(ssh2::PtyModeOpcode::IXANY, true);  // 任意键恢复输出
        pty_modes.set_boolean(ssh2::PtyModeOpcode::IMAXBEL, true); // 输入队列满时响铃
        // 输出模式
        pty_modes.set_boolean(ssh2::PtyModeOpcode::OPOST, true);  // 启用输出处理
        pty_modes.set_boolean(ssh2::PtyModeOpcode::ONLCR, true);  // NL→CR-NL(输出)
        // 本地模式
        pty_modes.set_boolean(ssh2::PtyModeOpcode::ISIG, true);   // 信号(Ctrl+C等)
        pty_modes.set_boolean(ssh2::PtyModeOpcode::ICANON, true); // 规范输入
        pty_modes.set_boolean(ssh2::PtyModeOpcode::ECHO, false);  // 注入期间不回显
        pty_modes.set_boolean(ssh2::PtyModeOpcode::ECHOE, true);  // 退格可视擦除
        pty_modes.set_boolean(ssh2::PtyModeOpcode::ECHOK, true);  // kill 后回显 NL
        pty_modes.set_boolean(ssh2::PtyModeOpcode::ECHOCTL, true); // 控制字符显示为 ^X
        pty_modes.set_boolean(ssh2::PtyModeOpcode::ECHOKE, true); // kill 可视擦除
        pty_modes.set_boolean(ssh2::PtyModeOpcode::IEXTEN, true); // 扩展输入处理
        if let Err(e) = channel.request_pty("xterm-256color", Some(pty_modes), Some((pty_cols, pty_rows, 0, 0))) {
            let _ = status_tx.send(Err(format!("PTY request failed: {}", e)));
            return;
        }
        if let Err(e) = channel.shell() {
            let _ = status_tx.send(Err(format!("Shell request failed: {}", e)));
            return;
        }
        session.set_keepalive(true, 30);

        // Signal success
        let _ = status_tx.send(Ok(()));

        // 4. Read loop: SSH channel -> terminal-output event (ZMODEM handled on frontend)
        // Delay to let frontend register event listener
        std::thread::sleep(std::time::Duration::from_millis(500));
        // 给 bash/zsh 登录 shell 注入一次性 OSC7 cwd 上报钩子，让“当前这个会话”立刻
        // 能跟随目录——rc 配置只对之后新开的 shell 生效，覆盖不到已启动的本会话。
        // 隐藏处理：PTY 已 ECHO=0 → 注入命令不回显；每条命令以 printf '\r\033[2K' 自清
        // 当前提示符行，避免 ECHO=0 下回车不换行导致单行提示符叠行（多行/powerline
        // 提示符上半部分仍可能残留）。fish 等子 shell 不走这里（由 rc/config.fish 上报）。
        // host 用占位 h，解析器接受任意 host。
        {
            // 注入 OSC7 钩子(ECHO=false 不可见)。stty echo 延迟到首次 resize 时一起注入
            let hook = " if [ -n \"$BASH_VERSION\" ]; then shopt -s checkwinsize; PROMPT_COMMAND='printf \"\\033]7;file://h%s\\007\\r\" \"$PWD\"'\"${PROMPT_COMMAND:+;$PROMPT_COMMAND}\"; elif [ -n \"$ZSH_VERSION\" ]; then __lt_cwd(){ printf '\\033]7;file://h%s\\007\\r' \"$PWD\"; }; typeset -ga precmd_functions; precmd_functions+=(__lt_cwd); fi; printf '\\r\\033[2K\\r'\r";
            let _ = channel.write_all(hook.as_bytes());
            let _ = channel.flush();
            app_log!("SSH-HOOK", "注入 hook(ECHO 延迟到首次 resize)");
        }
        session.set_blocking(false);
        let id_for_read = id_clone.clone();
        app_log!("SSH", "reader LOOP START id={} host={}:{} user={} (把 uuid 锚定到主机,便于按主机对齐断连时刻)", id_for_read, host, port, user);
        // OSC7 cwd 解析器（接受任意 host，含 shell 原生 OSC7，如 fish）
        let mut osc7_parser = crate::core::osc7::Osc7Parser::new();
        let mut last_keepalive = std::time::Instant::now();
        let mut need_stty_echo = true; // ECHO=false 状态,等首次 resize 时恢复
        loop {
            // ZMODEM 上传：收到信号时在本线程内联运行整个协议（本线程独占
            // channel）。单线程、单解码器、按网络速度推进。
            // ZMODEM 上传（feature 门控，默认不编译）
            if zmodem_active_clone.load(std::sync::atomic::Ordering::Acquire) {
                let req = zmodem_request_clone.lock().unwrap().take();
                if let Some(req) = req {
                    // 用 catch_unwind 隔离 panic，避免某个 bug 静默杀死 reader
                    // 线程（那会冻结终端并让等待中的命令永久挂起）。即使 panic
                    // 也要回应命令。
                    let result_tx = req.result_tx;
                    let files = req.files;
                    let cancel = req.cancel;
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        crate::commands::zmodem::run_zmodem_send(
                            &mut channel,
                            &session,
                            files,
                            &app_clone,
                            &cancel,
                            &input_rx,
                        )
                    }))
                    .unwrap_or_else(|_| {
                        app_log!("ZMODEM", "run_zmodem_send panic");
                        Err("ZMODEM 内部错误".to_string())
                    });
                    zmodem_active_clone.store(false, std::sync::atomic::Ordering::Release);
                    let _ = result_tx.send(r);
                    // 丢弃传输期间堆积的键盘输入，避免结束后一次性回放到 shell
                    while input_rx.try_recv().is_ok() {}
                    // 传输结束后刷新 shell 提示符
                    session.set_blocking(false);
                    let _ = channel.write(b"\r");
                    let _ = channel.flush();
                    last_keepalive = std::time::Instant::now();
                    continue;
                }
            }

            // 线程退出前回应可能遗留的 ZMODEM 请求，避免命令在连接断开时永久阻塞
            let answer_orphan = || {
                if let Some(req) = zmodem_request_clone.lock().unwrap().take() {
                    zmodem_active_clone.store(false, std::sync::atomic::Ordering::Release);
                    let _ = req.result_tx.send(Err("连接已关闭".to_string()));
                }
            };

            let mut buf = [0u8; 4096];
            match channel.read(&mut buf) {
                Ok(0) => {
                    answer_orphan();
                    app_log!("SSH", "terminal-closed via EOF id={} host={}:{} user={} channel_eof={} (服务端关闭/shell 退出,非客户端误判)", id_for_read, host, port, user, channel.eof());
                    let _ = app_clone.emit(
                        "terminal-closed",
                        serde_json::json!({"id": id_for_read}),
                    );
                    break;
                }
                Ok(n) => {
                    // 被动解析 shell 自行上报的 OSC7，更新 cwd（只读，不影响终端）
                    if let Some(cwd) = osc7_parser.feed(&buf[..n]) {
                        let mut guard = osc7_cwd_clone.lock().unwrap();
                        if guard.as_deref() != Some(cwd.as_str()) {
                            app_log!("OSC7", "cwd 更新 -> {}", cwd);
                            *guard = Some(cwd);
                        }
                    }
                    let _ = app_clone.emit(
                        "terminal-output",
                        serde_json::json!({
                            "id": id_for_read,
                            "data": &buf[..n],
                        }),
                    );
                    // 写入输出缓冲区供 HTTP API 读取
                    if let Ok(mut bufs) = output_bufs.lock() {
                        if let Some(ob) = bufs.get_mut(&id_for_read) {
                            ob.write(&buf[..n]);
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(ref e) => {
                    answer_orphan();
                    // 记录断连真实原因:raw_os=104(ECONNRESET 远端/中间设备断)、113(EHOSTUNREACH
                    // 本机到对端链路/路由断=本机网络共因)、110(ETIMEDOUT)、32(EPIPE);kind=Other
                    // 且 raw_os=None 则为 libssh2 协议错(看 err 文本)。区分'本机网络事件'与'误判'。
                    app_log!("SSH", "terminal-closed via READ-ERR id={} host={}:{} user={} kind={:?} raw_os={:?} err={}", id_for_read, host, port, user, e.kind(), e.raw_os_error(), e);
                    let _ = app_clone.emit(
                        "terminal-closed",
                        serde_json::json!({"id": id_for_read}),
                    );
                    break;
                }
            }

            // 写入键盘输入（非阻塞）
            while let Ok(data) = input_rx.try_recv() {
                if data.len() <= 10 {
                    app_log!("SSH-IO", "WRITE {} bytes: {:02x?}", data.len(), &data);
                }
                let _ = channel.write_all(&data);
                let _ = channel.flush();
            }

            // 处理窗口大小变化
            while let Ok((cols, rows)) = resize_rx.try_recv() {
                session.set_blocking(true);
                let _ = channel.request_pty_size(cols, rows, None, None);
                // 首次 resize: 注入 stty 强制更新 $COLUMNS(request_pty_size 的 SIGWINCH 在部分服务器不更新)
                // 用 \033[A 上移光标 + \033[2K 擦除行 隐藏 stty 回显
                // ECHO=false 期间注入 stty cols/rows + echo(全部不可见)
                if need_stty_echo {
                    need_stty_echo = false;
                    let stty = format!(" stty cols {} rows {} echo 2>/dev/null; command stty cols {} rows {} echo < /dev/tty 2>/dev/null; printf '\\r\\033[2K\\r'\r", cols, rows, cols, rows);
                    let _ = channel.write_all(stty.as_bytes());
                    let _ = channel.flush();
                    app_log!("SSH", "resize 时注入 stty cols={} rows={} + echo (ECHO=false 不可见)", cols, rows);
                }
                session.set_blocking(false);
            }

            // 每 15 秒发送 SSH keepalive 防止服务端超时断连
            if last_keepalive.elapsed() >= std::time::Duration::from_secs(15) {
                // keepalive 失败是 socket 死亡的早期信号(通常紧邻随后的 READ-ERR);只记录不视为致命。
                if let Err(e) = session.keepalive_send() {
                    app_log!("SSH", "keepalive_send 失败 id={} host={}:{} err={} (连接可能正在劣化)", id_for_read, host, port, e);
                }
                last_keepalive = std::time::Instant::now();
            }

            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });

    // Wait for connection result
    match status_rx.recv() {
        Ok(Ok(())) => {
            let monitor_stop = Arc::new(AtomicBool::new(false));
            let (sftp_tx, _sftp_rx) = std::sync::mpsc::channel::<SftpRequest>();
            state.sessions.lock().unwrap().insert(
                id.clone(),
                ManagedSession {
                    id: id.clone(),
                    label,
                    input_tx,
                    resize_tx,
                    monitor_stop,
                    sftp_request_tx: sftp_tx,
                    osc7_cwd,
                    zmodem_active,
                    zmodem_request,
                },
            );
            Ok(id)
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Connection thread died".to_string()),
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ssh_connect(
    state: State<'_, AppState>,
    app: AppHandle,
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    auth_method: String,
    key_path: Option<String>,
    label: String,
    proxy_jump: Option<String>,
    cols: Option<u32>,
    rows: Option<u32>,
) -> Result<String, String> {
    do_ssh_connect(&state, &app, host, port, user, password, auth_method, key_path, label, proxy_jump, cols, rows).await
}

/// 返回客户端支持的 SSH 加密算法列表，用于连接失败时诊断
#[tauri::command]
pub async fn ssh_supported_algs() -> Result<String, String> {
    let session = ssh2::Session::new().map_err(|e| format!("{}", e))?;
    let mut info = String::new();
    let types = [
        (ssh2::MethodType::Kex, "密钥交换(Kex)"),
        (ssh2::MethodType::HostKey, "主机密钥(HostKey)"),
        (ssh2::MethodType::CryptCs, "加密(Client→Server)"),
        (ssh2::MethodType::CryptSc, "加密(Server→Client)"),
        (ssh2::MethodType::MacCs, "MAC(Client→Server)"),
        (ssh2::MethodType::MacSc, "MAC(Server→Client)"),
    ];
    for (mt, label) in types {
        match session.supported_algs(mt) {
            Ok(algs) => info.push_str(&format!("{}: {}\n", label, algs.join(", "))),
            Err(_) => info.push_str(&format!("{}: (无法获取)\n", label)),
        }
    }
    Ok(info)
}
