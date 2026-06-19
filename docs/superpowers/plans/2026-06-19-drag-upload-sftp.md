# 拖拽上传重新设计 实现计划（独立 SFTP + OSC7 cwd + ZMODEM 门控）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 拖拽上传脱离终端通道，改走独立 SFTP 连接；终端 cwd 由 OSC7 钩子被动跟踪；ZMODEM 用 cargo feature 门控（默认不编译）。

**Architecture:** 终端 reader 线程额外被动解析 OSC7 维护每会话 `osc7_cwd`；新 `drag_upload` 命令解析目标目录（osc7_cwd → 文件管理器目录 → home）后在自己的异步任务里做阻塞 SFTP 上传（终端 reader 是独立 OS 线程，故终端键盘不受影响）；ZMODEM 整套代码用 `#[cfg(feature = "zmodem")]` 包起来。

**Tech Stack:** Rust (Tauri 2, ssh2/libssh2, crc32fast), TypeScript/React。注释一律中文。

---

### Task 1: 加 cargo feature `zmodem`，把 ZMODEM 代码门控起来

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/core/mod.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/commands/ssh.rs`

- [ ] **Step 1: Cargo.toml 增加 features 段**

在 `src-tauri/Cargo.toml` 的 `[dependencies]` 之前（第 13 行 `[dependencies]` 上方）插入：

```toml
[features]
default = []
# ZMODEM 发送（rz）支持，默认关闭。启用：cargo build --features zmodem
zmodem = []

```

- [ ] **Step 2: 门控 core/zmodem 模块声明**

`src-tauri/src/core/mod.rs` 把 `pub mod zmodem;` 改为：

```rust
#[cfg(feature = "zmodem")]
pub mod zmodem;
```

- [ ] **Step 3: 门控 commands/zmodem 模块声明**

`src-tauri/src/commands/mod.rs` 把 `pub mod zmodem;` 改为：

```rust
#[cfg(feature = "zmodem")]
pub mod zmodem;
```

- [ ] **Step 4: 门控 lib.rs 里 zmodem_send 命令注册**

`src-tauri/src/lib.rs` 第 85 行附近的 `commands::zmodem::zmodem_send,`。`tauri::generate_handler!` 宏内不能直接用 `#[cfg]` 属性可靠地条件包含一项，因此改为不在宏内注册该命令——删除该行：

```rust
            // 删除这一行：
            // commands::zmodem::zmodem_send,
```

（ZMODEM 启用时通过 `cargo build --features zmodem` 仍会编译模块；其命令注册留待将来需要时单独处理。当前需求不依赖前端调用 zmodem_send。）

- [ ] **Step 5: 门控 state.rs 的 ZMODEM 字段与结构体**

`src-tauri/src/state.rs`：把 `ManagedSession` 里的两个 zmodem 字段和 `ZmodemSendRequest` 结构体各自加 `#[cfg(feature = "zmodem")]`。改后相关片段：

```rust
pub struct ManagedSession {
    pub id: String,
    pub label: String,
    pub input_tx: std::sync::mpsc::Sender<Vec<u8>>,
    pub resize_tx: std::sync::mpsc::Sender<(u32, u32)>,
    pub monitor_stop: Arc<AtomicBool>,
    pub sftp_request_tx: std::sync::mpsc::Sender<SftpRequest>,
    /// zmodem_send 置为 true，通知 reader 线程执行一次 ZMODEM 传输。
    #[cfg(feature = "zmodem")]
    pub zmodem_active: Arc<AtomicBool>,
    /// 待处理的 ZMODEM 发送请求，由 reader 线程取走执行。
    #[cfg(feature = "zmodem")]
    pub zmodem_request: Arc<Mutex<Option<ZmodemSendRequest>>>,
}

/// 由 zmodem_send 命令交给 SSH reader 线程（独占 channel）的 ZMODEM 上传请求。
#[cfg(feature = "zmodem")]
pub struct ZmodemSendRequest {
    pub files: Vec<crate::core::zmodem::sender::FileInfo>,
    pub result_tx: std::sync::mpsc::Sender<Result<(), String>>,
    pub cancel: Arc<AtomicBool>,
}
```

- [ ] **Step 6: 门控 ssh.rs 的 zmodem 变量、reader 线程块、构造**

`src-tauri/src/commands/ssh.rs` 第 142-145 行的 zmodem 变量创建，加门控：

```rust
    #[cfg(feature = "zmodem")]
    let zmodem_active = Arc::new(AtomicBool::new(false));
    #[cfg(feature = "zmodem")]
    let zmodem_request: Arc<Mutex<Option<crate::state::ZmodemSendRequest>>> = Arc::new(Mutex::new(None));
    #[cfg(feature = "zmodem")]
    let zmodem_active_clone = zmodem_active.clone();
    #[cfg(feature = "zmodem")]
    let zmodem_request_clone = zmodem_request.clone();
```

reader 线程 loop 顶部的整段 ZMODEM 内联块（以 `if zmodem_active_clone.load(...)` 开头、到对应 `continue;` 后的 `}` 结束，即第 285-320 行那一整块 `if ... { ... continue; } }`）整体加门控。在该 `if` 块前加一行属性：

```rust
            // ZMODEM 上传（feature 门控，默认不编译）
            #[cfg(feature = "zmodem")]
            if zmodem_active_clone.load(std::sync::atomic::Ordering::Acquire) {
                // ……原有整块内容保持不变……
            }
```

`ManagedSession { ... }` 构造（第 362-371 行附近）里的两个字段加属性：

```rust
                ManagedSession {
                    id: id.clone(),
                    label,
                    input_tx,
                    resize_tx,
                    monitor_stop,
                    sftp_request_tx: sftp_tx,
                    #[cfg(feature = "zmodem")]
                    zmodem_active,
                    #[cfg(feature = "zmodem")]
                    zmodem_request,
                },
```

`answer_orphan` 闭包（reader 线程退出前回应 ZMODEM 请求，第 322-329 行）也整体加门控，并把它的两处调用 `answer_orphan();` 各自加 `#[cfg(feature = "zmodem")]`：

```rust
            #[cfg(feature = "zmodem")]
            let answer_orphan = || {
                if let Some(req) = zmodem_request_clone.lock().unwrap().take() {
                    zmodem_active_clone.store(false, std::sync::atomic::Ordering::Release);
                    let _ = req.result_tx.send(Err("连接已关闭".to_string()));
                }
            };
```

两处调用改为：
```rust
                Ok(0) => {
                    #[cfg(feature = "zmodem")]
                    answer_orphan();
                    let _ = app_clone.emit("terminal-closed", serde_json::json!({"id": id_for_read}));
                    break;
                }
```
和 `Err(_)` 分支同理（在 `let _ = app_clone.emit("terminal-closed", ...)` 前加 `#[cfg(feature = "zmodem")] answer_orphan();`）。

- [ ] **Step 7: 验证两种 feature 组合都能编译**

Run:
```bash
cd /home/lfl/ssd/code/guishell/src-tauri && cargo build 2>&1 | tail -3
```
Expected: 默认（无 zmodem）编译通过，无 error。

Run:
```bash
cd /home/lfl/ssd/code/guishell/src-tauri && cargo build --features zmodem 2>&1 | tail -3
```
Expected: 启用 zmodem 也编译通过，无 error。

- [ ] **Step 8: Commit**

```bash
cd /home/lfl/ssd/code/guishell
git add src-tauri/Cargo.toml src-tauri/src/core/mod.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src-tauri/src/state.rs src-tauri/src/commands/ssh.rs
git commit -m "refactor: ZMODEM 用 cargo feature 门控，默认不编译"
```

---

### Task 2: OSC7 解析器（core 纯模块 + 单测）

**Files:**
- Create: `src-tauri/src/core/osc7.rs`
- Modify: `src-tauri/src/core/mod.rs`

- [ ] **Step 1: 写解析器与单测**

创建 `src-tauri/src/core/osc7.rs`：

```rust
//! OSC 7 当前目录上报序列解析。
//!
//! shell 通过 `ESC ] 7 ; file://<host>/<path> (BEL | ESC \)` 上报 cwd。
//! 终端 reader 线程把所有输出字节喂给本解析器，解析出最近一次的远端路径。

/// 累积缓冲上限，防止异常流导致内存增长。
const MAX_PENDING: usize = 8192;

/// 增量 OSC7 解析器：跨多次 feed 处理可能被切分的序列。
pub struct Osc7Parser {
    pending: Vec<u8>,
}

impl Osc7Parser {
    pub fn new() -> Self {
        Self { pending: Vec::new() }
    }

    /// 喂入一段终端输出字节。返回本次累积流中最新一条完整 OSC7 解析出的路径
    /// （若有多条取最后一条）；没有完整序列时返回 None。
    pub fn feed(&mut self, data: &[u8]) -> Option<String> {
        self.pending.extend_from_slice(data);
        if self.pending.len() > MAX_PENDING {
            let cut = self.pending.len() - MAX_PENDING;
            self.pending.drain(..cut);
        }

        let prefix = [0x1b, b']', b'7', b';']; // ESC ] 7 ;
        let mut result = None;
        loop {
            let Some(s) = find_subseq(&self.pending, &prefix) else {
                // 没有起始标记：只保留末尾最多 3 字节（可能是被切断的 ESC ] 7 前缀）
                if self.pending.len() > 3 {
                    let cut = self.pending.len() - 3;
                    self.pending.drain(..cut);
                }
                break;
            };
            let payload_start = s + prefix.len();
            // 从 payload 起找终止符：BEL(0x07) 或 ST(ESC '\')
            let mut term: Option<(usize, usize)> = None; // (index, len)
            let mut i = payload_start;
            while i < self.pending.len() {
                if self.pending[i] == 0x07 {
                    term = Some((i, 1));
                    break;
                }
                if self.pending[i] == 0x1b
                    && i + 1 < self.pending.len()
                    && self.pending[i + 1] == 0x5c
                {
                    term = Some((i, 2));
                    break;
                }
                i += 1;
            }
            let Some((t, tlen)) = term else {
                // 序列未结束：丢掉起始标记之前的内容，等下次 feed 补齐
                self.pending.drain(..s);
                break;
            };
            let payload = self.pending[payload_start..t].to_vec();
            self.pending.drain(..t + tlen);
            if let Some(path) = parse_file_url(&payload) {
                result = Some(path);
            }
        }
        result
    }
}

/// 在 haystack 中查找子序列 needle 的起始下标。
fn find_subseq(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

/// 从 OSC7 payload 解析出路径。payload 形如 `file://host/path`，也兼容直接是路径。
/// 对 `%XX` 做 URL 解码。
fn parse_file_url(payload: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(payload);
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let path_part = if let Some(rest) = s.strip_prefix("file://") {
        // rest = host/path —— 从 host 之后的第一个 '/' 开始才是路径
        match rest.find('/') {
            Some(idx) => &rest[idx..],
            None => return None,
        }
    } else if s.starts_with('/') {
        s
    } else {
        return None;
    };
    let decoded = url_decode(path_part);
    if decoded.starts_with('/') {
        Some(decoded)
    } else {
        None
    }
}

/// 最小化 URL 解码：把 %XX 还原为字节，其余原样。
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_complete_bel() {
        let mut p = Osc7Parser::new();
        let seq = b"\x1b]7;file://host/home/bmc\x07";
        assert_eq!(p.feed(seq), Some("/home/bmc".to_string()));
    }

    #[test]
    fn test_complete_st() {
        let mut p = Osc7Parser::new();
        let seq = b"\x1b]7;file://host/var/log\x1b\\";
        assert_eq!(p.feed(seq), Some("/var/log".to_string()));
    }

    #[test]
    fn test_split_across_feeds() {
        let mut p = Osc7Parser::new();
        assert_eq!(p.feed(b"\x1b]7;file://h/ho"), None);
        assert_eq!(p.feed(b"me/lfl/work\x07"), Some("/home/lfl/work".to_string()));
    }

    #[test]
    fn test_url_decode() {
        let mut p = Osc7Parser::new();
        let seq = b"\x1b]7;file://h/home/a%20b/c\x07";
        assert_eq!(p.feed(seq), Some("/home/a b/c".to_string()));
    }

    #[test]
    fn test_surrounding_garbage() {
        let mut p = Osc7Parser::new();
        let seq = b"some prompt text\x1b]7;file://h/data/bmc\x07lfl@host:~$ ";
        assert_eq!(p.feed(seq), Some("/data/bmc".to_string()));
    }

    #[test]
    fn test_latest_of_multiple() {
        let mut p = Osc7Parser::new();
        let seq = b"\x1b]7;file://h/a\x07\x1b]7;file://h/b\x07";
        assert_eq!(p.feed(seq), Some("/b".to_string()));
    }

    #[test]
    fn test_no_osc7() {
        let mut p = Osc7Parser::new();
        assert_eq!(p.feed(b"just normal terminal output\r\n"), None);
    }

    #[test]
    fn test_oversized_does_not_grow_unbounded() {
        let mut p = Osc7Parser::new();
        // 一大段没有完整 OSC7 的数据
        let big = vec![b'x'; 20000];
        assert_eq!(p.feed(&big), None);
        // pending 不应超过上限
        assert!(p.pending.len() <= MAX_PENDING);
    }
}
```

- [ ] **Step 2: 注册模块**

`src-tauri/src/core/mod.rs` 顶部加一行：

```rust
pub mod osc7;
```

- [ ] **Step 3: 运行测试**

Run:
```bash
cd /home/lfl/ssd/code/guishell/src-tauri && cargo test --lib core::osc7 -v 2>&1 | tail -15
```
Expected: 8 个测试全部 PASS。

- [ ] **Step 4: Commit**

```bash
cd /home/lfl/ssd/code/guishell
git add src-tauri/src/core/osc7.rs src-tauri/src/core/mod.rs
git commit -m "feat(osc7): 新增 OSC7 当前目录上报序列解析器（含单测）"
```

---

### Task 3: reader 线程跟踪 cwd + 注入 OSC7 钩子 + osc7_cwd 状态

**Files:**
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/commands/ssh.rs`

- [ ] **Step 1: ManagedSession 增加 osc7_cwd 字段**

`src-tauri/src/state.rs` 的 `ManagedSession` 增加（放在 `sftp_request_tx` 之后、zmodem 字段之前）：

```rust
    /// 终端 shell 当前工作目录，由 reader 线程解析 OSC7 更新（每会话独立）。
    pub osc7_cwd: Arc<Mutex<Option<String>>>,
```

- [ ] **Step 2: ssh.rs 创建 osc7_cwd 并克隆进 reader 线程**

`src-tauri/src/commands/ssh.rs` 在第 142 行附近（zmodem 变量创建处旁边，但 osc7 不门控）加：

```rust
    let osc7_cwd: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let osc7_cwd_clone = osc7_cwd.clone();
```

- [ ] **Step 3: reader 线程解析 OSC7 + 一次性注入钩子**

在 reader 线程闭包内、`session.set_blocking(false);`（第 279 行）之后、`let mut last_keepalive = ...`（第 281 行）之前，加解析器与注入：

```rust
        // OSC7 cwd 解析器
        let mut osc7_parser = crate::core::osc7::Osc7Parser::new();
        // 注入一次性 OSC7 上报钩子（bash/zsh）。前导空格避免进 history；
        // 不支持的 shell 不生效，自动走上传时的回退逻辑。
        {
            let hook = " if [ -n \"$BASH_VERSION\" ]; then PROMPT_COMMAND='printf \"\\033]7;file://h%s\\007\" \"$PWD\"'\"${PROMPT_COMMAND:+;$PROMPT_COMMAND}\"; elif [ -n \"$ZSH_VERSION\" ]; then __lt_cwd(){ printf '\\033]7;file://h%s\\007' \"$PWD\"; }; typeset -ga precmd_functions; precmd_functions+=(__lt_cwd); fi\r";
            let _ = channel.write_all(hook.as_bytes());
            let _ = channel.flush();
            // 擦除刚回显的注入命令行，尽量不留痕迹
            let _ = channel.write_all(b"\x1b[2K\r");
            let _ = channel.flush();
        }
```

在 reader loop 内处理 `channel.read` 成功 `Ok(n)` 的分支里（第 339-347 行附近，`let _ = app_clone.emit("terminal-output", ...)` 之前），加 OSC7 解析：

```rust
                Ok(n) => {
                    // 被动解析 OSC7 更新 cwd（只读，不影响终端）
                    if let Some(cwd) = osc7_parser.feed(&buf[..n]) {
                        *osc7_cwd_clone.lock().unwrap() = Some(cwd);
                    }
                    let _ = app_clone.emit(
                        "terminal-output",
                        serde_json::json!({
                            "id": id_for_read,
                            "data": &buf[..n],
                        }),
                    );
                }
```

- [ ] **Step 4: ManagedSession 构造加 osc7_cwd**

`src-tauri/src/commands/ssh.rs` 的 `ManagedSession { ... }` 构造里加（在 `sftp_request_tx: sftp_tx,` 之后）：

```rust
                    osc7_cwd,
```

- [ ] **Step 5: 编译验证**

Run:
```bash
cd /home/lfl/ssd/code/guishell && ./build.sh 2>&1 | tail -5
```
Expected: 构建完成，无 error。

- [ ] **Step 6: Commit**

```bash
cd /home/lfl/ssd/code/guishell
git add src-tauri/src/state.rs src-tauri/src/commands/ssh.rs
git commit -m "feat(osc7): reader 线程注入 OSC7 钩子并被动跟踪终端 cwd"
```

---

### Task 4: drag_upload 命令（目标解析 + SFTP 上传 + 取消 + 进度带目标目录）

**Files:**
- Modify: `src-tauri/src/commands/sftp.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 在 sftp.rs 末尾加 drag_upload 命令**

在 `src-tauri/src/commands/sftp.rs` 文件末尾追加：

```rust
/// 拖拽上传：把若干本地文件通过独立 SFTP 连接上传到终端当前目录。
///
/// 目标目录解析顺序：会话的 osc7_cwd（终端 cwd）→ fallback_dir（文件管理器
/// 当前远程目录）→ 相对路径（落到 SFTP 默认目录，即 home）。
/// 阻塞 I/O 跑在本命令自己的异步任务里；终端 reader 是独立 OS 线程，不受影响。
#[tauri::command]
pub async fn drag_upload(
    state: State<'_, AppState>,
    app: AppHandle,
    session_id: String,
    files: Vec<String>,
    fallback_dir: Option<String>,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;

    // 1. 解析目标目录
    let cwd = {
        let sessions = state.sessions.lock().unwrap();
        sessions
            .get(&session_id)
            .and_then(|s| s.osc7_cwd.lock().unwrap().clone())
    };
    let target_dir = cwd.or(fallback_dir); // None 时用相对路径（落到 home）
    let target_display = target_dir.clone().unwrap_or_else(|| "~".to_string());
    app_log!("SFTP", "DRAG UPLOAD: session={}, files={}, target={}", session_id, files.len(), target_display);

    // 2. 注册取消标志（key 与前端浮窗取消按钮一致：upload-<文件名>）
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut cancel_keys: Vec<String> = Vec::new();
    {
        let mut cancels = state.transfer_cancel.lock().unwrap();
        for f in &files {
            let name = Path::new(f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let key = format!("upload-{}", name);
            cancels.insert(key.clone(), cancel.clone());
            cancel_keys.push(key);
        }
    }

    // 3. 逐个上传
    let mut last_err: Option<String> = None;
    for local in &files {
        let expanded = shellexpand::tilde(local).to_string();
        let name = Path::new(&expanded)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let remote_path = match &target_dir {
            Some(d) => format!("{}/{}", d.trim_end_matches('/'), name),
            None => name.clone(), // 相对路径 → SFTP home
        };
        if let Err(e) = upload_one(&state, &app, &session_id, &expanded, &remote_path, &name, &target_display, &cancel) {
            app_log!("SFTP", "DRAG UPLOAD 失败: {} - {}", name, e);
            last_err = Some(format!("{}: {}", name, e));
            if cancel.load(Ordering::Relaxed) {
                break;
            }
        }
    }

    // 4. 清理取消标志
    {
        let mut cancels = state.transfer_cancel.lock().unwrap();
        for k in &cancel_keys {
            cancels.remove(k);
        }
    }

    match last_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// 单文件 SFTP 上传，支持取消，进度事件附带目标目录。
fn upload_one(
    state: &State<'_, AppState>,
    app: &AppHandle,
    session_id: &str,
    expanded_local: &str,
    remote_path: &str,
    filename: &str,
    target_display: &str,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;

    let meta = std::fs::metadata(expanded_local)
        .map_err(|e| format!("无法读取本地文件: {}", e))?;
    let total = meta.len();

    let sftp_sessions = state.sftp_sessions.lock().unwrap();
    let handle = sftp_sessions
        .get(session_id)
        .ok_or_else(|| "SFTP会话未找到".to_string())?;

    let mut remote_file = handle
        .sftp
        .create(Path::new(remote_path))
        .map_err(|e| format!("无法创建远程文件: {} (path={})", e, remote_path))?;
    let mut local_file =
        std::fs::File::open(expanded_local).map_err(|e| format!("无法打开本地文件: {}", e))?;

    let mut buf = [0u8; 32768];
    let mut bytes_so_far: u64 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(remote_file);
            let _ = handle.sftp.unlink(Path::new(remote_path));
            return Err("已取消".to_string());
        }
        let n = local_file
            .read(&mut buf)
            .map_err(|e| format!("读取本地文件失败: {}", e))?;
        if n == 0 {
            break;
        }
        remote_file
            .write_all(&buf[..n])
            .map_err(|e| format!("写入远程文件失败: {} (bytes_so_far={})", e, bytes_so_far))?;
        bytes_so_far += n as u64;
        let _ = app.emit(
            "transfer-progress",
            serde_json::json!({
                "filename": filename,
                "bytes_transferred": bytes_so_far,
                "total_bytes": total,
                "direction": "upload",
                "target": target_display
            }),
        );
    }
    app_log!("SFTP", "DRAG UPLOAD 完成: {} ({} bytes) -> {}", filename, bytes_so_far, remote_path);
    Ok(())
}
```

- [ ] **Step 2: 注册 drag_upload 命令**

`src-tauri/src/lib.rs` 的 `invoke_handler` 里，在 `commands::sftp::cancel_transfer,`（第 66 行）后加：

```rust
            commands::sftp::drag_upload,
```

- [ ] **Step 3: 编译验证**

Run:
```bash
cd /home/lfl/ssd/code/guishell && ./build.sh 2>&1 | tail -5
```
Expected: 构建完成，无 error。

- [ ] **Step 4: Commit**

```bash
cd /home/lfl/ssd/code/guishell
git add src-tauri/src/commands/sftp.rs src-tauri/src/lib.rs
git commit -m "feat(sftp): 新增 drag_upload 命令（终端 cwd 目标解析 + 取消 + 进度带目标）"
```

---

### Task 5: 前端接线（拖拽走 drag_upload + fallbackDir + 浮窗显示目标目录）

**Files:**
- Modify: `src/components/FileManager/FileBrowser.tsx`
- Modify: `src/App.tsx`

- [ ] **Step 1: FileBrowser 暴露当前远程路径**

`src/components/FileManager/FileBrowser.tsx` 的 `interface Props` 增加一个回调：

```tsx
interface Props {
  sessionId: string | null;
  activeTerminalId?: string | null;
  sshUser?: string;
  sftpReady?: number;
  onRemotePathChange?: (path: string) => void;
}
```

组件签名同步加 `onRemotePathChange`：把 `export function FileBrowser({ sessionId, activeTerminalId, sshUser, sftpReady }: Props)` 改为：

```tsx
export function FileBrowser({ sessionId, activeTerminalId, sshUser, sftpReady, onRemotePathChange }: Props) {
```

在 `setRemotePath`（第 388 行附近 `const setRemotePath = (path: string) => {` 内）末尾加一行回调通知：

```tsx
  const setRemotePath = (path: string) => {
    setRemotePathRaw(path);
    if (sessionId) sessionPathsRef.current[sessionId] = path;
    if (onRemotePathChange) onRemotePathChange(path);
  };
```

- [ ] **Step 2: App.tsx 记录当前远程路径并接入 FileBrowser**

`src/App.tsx` 在 `transferStatsRef` 声明（第 1044 行附近）旁边加一个 ref：

```tsx
  const currentRemotePathRef = useRef<string>('');
```

第 1747 行的 `<FileBrowser .../>` 加 `onRemotePathChange`：

```tsx
            <FileBrowser sessionId={activeSshSessionId} activeTerminalId={activeTabId} sshUser={activeTab?.sshParams?.user} sftpReady={sftpReady} onRemotePathChange={(p) => { currentRemotePathRef.current = p; }} />
```

- [ ] **Step 3: 拖拽 handler 改为调用 drag_upload**

`src/App.tsx` 第 1055-1077 行的拖拽 useEffect 内，`drop` 分支里把对 `zmodem_send` 的调用替换为 `drag_upload`：

```tsx
      } else if (event.payload.type === 'drop') {
        setDragOverTerminal(false);
        const paths = event.payload.paths;
        if (!paths.length) return;
        const sid = activeSshSessionId;
        const fallbackDir = currentRemotePathRef.current || null;
        log('拖拽上传', `${paths.length} 个文件通过 SFTP 上传`, paths);
        invoke('drag_upload', { sessionId: sid, files: paths, fallbackDir })
          .then(() => log('拖拽上传', '完成'))
          .catch((e) => {
            log('拖拽上传', `失败: ${e}`);
            setError(`上传失败: ${e}`);
          });
      }
```

- [ ] **Step 4: globalTransfers 增加 target 字段**

`src/App.tsx` 第 1043 行 `globalTransfers` 的类型加 `target`：

```tsx
  const [globalTransfers, setGlobalTransfers] = useState<Record<string, { filename: string; direction: string; bytes: number; total: number; speed: number; target?: string }>>({});
```

transfer-progress 监听里（第 1084 行附近，解构事件 payload 处）取出 `target` 并存入：

```tsx
      const { filename, bytes_transferred, total_bytes, direction } = event.payload as any;
      const target = (event.payload as any).target as string | undefined;
      const key = `${direction}-${filename}`;
      const now = Date.now();
      const prev = transferStatsRef.current[key];
      let speed = 0;
      if (prev && now > prev.lastTime) {
        const inst = (bytes_transferred - prev.lastBytes) * 1000 / (now - prev.lastTime);
        speed = prev.ema > 0 ? prev.ema * 0.6 + inst * 0.4 : inst;
      }
      transferStatsRef.current[key] = { lastBytes: bytes_transferred, lastTime: now, ema: speed };
      setGlobalTransfers(prev => ({
        ...prev,
        [key]: { filename, direction, bytes: bytes_transferred, total: total_bytes, speed, target },
      }));
```

- [ ] **Step 5: 浮窗显示目标目录**

`src/App.tsx` 右上角浮窗每个传输项里（第 1853 行附近，文件名那行之后、进度条之前），在 `<div className="h-1.5 bg-surface ...">` 进度条上方加一行目标目录显示：

```tsx
                    {t.target && (
                      <div className="text-[10px] text-gray-500 truncate mb-0.5" title={t.target}>→ {t.target}</div>
                    )}
```

- [ ] **Step 6: 完整构建验证**

Run:
```bash
cd /home/lfl/ssd/code/guishell && ./build.sh 2>&1 | tail -5
```
Expected: 构建完成，无 error（tsc + vite + cargo 全过）。

- [ ] **Step 7: Commit**

```bash
cd /home/lfl/ssd/code/guishell
git add src/components/FileManager/FileBrowser.tsx src/App.tsx
git commit -m "feat(upload): 拖拽改走 drag_upload（SFTP 独立通道）+ 浮窗显示目标目录"
```

---

### Task 6: 手动联调验证

**Files:** 无（手动测试）

- [ ] **Step 1: 启动**

Run:
```bash
cd /home/lfl/ssd/code/guishell && ./run.sh
```

- [ ] **Step 2: bash/zsh 终端 cwd 跟随**

连接一台 Linux SSH（bash 或 zsh）。在终端 `cd /tmp` 回车，拖一个文件到终端区域。验证：
- 文件出现在远端 `/tmp`。
- 右上角浮窗显示 `→ /tmp`、进度、速率。
- **传输期间能正常在终端敲命令**（如 `ls`），键盘不卡、结束后无乱码回放。

- [ ] **Step 3: 回退路径**

把文件管理器远程面板导航到某目录，连一台无 OSC7 钩子生效的环境（或临时 `unset PROMPT_COMMAND`），拖文件，验证回退到文件管理器当前目录，浮窗显示该目标。

- [ ] **Step 4: 大文件 + 取消**

拖一个 >1GB 文件，验证实时速率稳定增长；中途点浮窗取消按钮，验证传输停止、远端半成品被删除（远端 `ls -l` 确认）。

- [ ] **Step 5: 多标签页隔离**

标签 A 传输大文件时切到标签 B，验证 B 终端可正常使用，且各自 cwd 不串。

- [ ] **Step 6: 查日志**

```bash
tail -30 ~/guishell.log | grep -E "SFTP|DRAG"
```
确认有 `DRAG UPLOAD: ... target=...` 与 `DRAG UPLOAD 完成` 记录。

- [ ] **Step 7: ZMODEM feature 仍可编译**

```bash
cd /home/lfl/ssd/code/guishell/src-tauri && cargo build --features zmodem 2>&1 | tail -3
```
Expected: 编译通过。
