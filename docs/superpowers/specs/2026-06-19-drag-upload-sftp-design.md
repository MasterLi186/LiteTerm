# 拖拽上传重新设计（独立 SFTP 通道）

## 背景与目标

当前拖拽上传走 ZMODEM（`rz`），运行在终端 PTY 通道内、独占终端 stdin，导致：
- 传输期间当前标签页无法输入，键盘事件堆积、结束后一次性回放。
- 这是 ZMODEM 带内协议的固有限制，无法在同一通道上做到非阻塞。

**目标：** 拖拽上传脱离终端通道，改走已有的独立 SFTP 连接。终端在传输期间完全可用、键盘不受影响。上传到终端 shell 的当前工作目录（对齐 XShell rz 的直觉），拿不到时回退。

**成功标准：**
1. 上传期间当前终端标签页可正常输入和显示，键盘事件不堆积、不回放。
2. 文件默认上传到终端 shell 的当前工作目录。
3. 拿不到 cwd 时回退到文件管理器当前远程目录，再不行回退 home，并在上传浮窗显示实际目标路径。
4. 大文件（GB 级）稳定传输（复用已修好的 SFTP 无超时配置）。
5. 右上角浮窗显示进度、实时速率，支持取消。

## 架构

```
拖拽文件到终端区域
  → 前端 invoke('drag_upload', { sessionId, files, fallbackDir })
        fallbackDir = 文件管理器当前远程路径
  → 后端 drag_upload 命令（spawn_blocking，独立运行）
        目标目录 = osc7_cwd ?? fallbackDir ?? home
        逐个文件走 SFTP 上传（独立 SFTP 连接，emit transfer-progress）
  → 前端右上角浮窗显示进度/速率/取消；终端通道完全不参与

终端 reader 线程（独立）：
  - 正常读写终端
  - 额外被动解析 OSC7，更新 ManagedSession.osc7_cwd（不阻塞）
```

关键点：上传命令使用 `sftp_sessions` 里该会话的独立 SFTP 连接，与终端 reader 线程互不干扰。终端 reader 线程只新增"解析 OSC7"这一被动只读动作。

## 组件设计

### 1. cwd 跟踪

**状态：** `ManagedSession` 增加字段
```rust
pub osc7_cwd: Arc<Mutex<Option<String>>>,
```
reader 线程解析到 OSC7 时写入；`drag_upload` 命令读取。每会话独立，多标签页不串。

**注入钩子（连接后一次）：** 终端就绪后（首个提示符出现后，约连接后 800ms）静默发送一行兼容 bash/zsh 的 OSC7 上报钩子：
```sh
if [ -n "$BASH_VERSION" ]; then PROMPT_COMMAND='printf "\033]7;file://h%s\007" "$PWD"'"${PROMPT_COMMAND:+;$PROMPT_COMMAND}"; elif [ -n "$ZSH_VERSION" ]; then __lt_cwd(){ printf '\033]7;file://h%s\007' "$PWD"; }; precmd_functions+=(__lt_cwd); fi
```
- 发送后立即发 `\033[2K\r`（擦除当前行）尽量不留可见痕迹；命令本身不进历史（前导空格 + 依赖 `HISTCONTROL` 不强求）。
- 注入仅一次，记录在 `ManagedSession`（`osc7_injected: AtomicBool`）避免重复。
- 失败/不支持的 shell：钩子不生效，`osc7_cwd` 保持 None，走回退。

**OSC7 解析（reader 线程）：** 在终端输出字节流里识别序列
```
ESC ] 7 ; <payload> (BEL=0x07 或 ESC '\')
```
- `<payload>` 形如 `file://<host>/<path>`，取 `/<path>` 部分，URL 解码（`%XX`）。
- 序列可能跨多次 read 到达 → 维护一个小的滚动缓冲（仅缓存"疑似 OSC 开始后"的字节，最多几百字节，超长丢弃防止内存增长）。
- 解析成功后写入 `osc7_cwd`。解析逻辑放 `core/`（无 UI 依赖），便于单测。

### 2. 上传执行

**新命令：**
```rust
#[tauri::command]
pub async fn drag_upload(
    state: State<'_, AppState>,
    app: AppHandle,
    session_id: String,
    files: Vec<String>,        // 本地绝对路径
    fallback_dir: Option<String>,
) -> Result<(), String>
```
流程：
1. 解析目标目录：`osc7_cwd`（读 ManagedSession） → 否则 `fallback_dir` → 否则用 sshUser 推断 home（`/root` 或 `/home/<user>`）。
2. 注册 cancel flag 到 `transfer_cancel`，key = `upload-<filename>`（与浮窗取消按钮一致）。
3. `spawn_blocking` 内逐个文件调用现有 SFTP 上传逻辑，目标 `target_dir/<basename>`。
4. emit `transfer-progress`（direction=`upload`，附带实际目标目录字段 `target`）。
5. 完成/失败/取消后清理 cancel keys。
6. 返回结果（成功数 / 错误信息）。

**sftp_upload 增强：** 在写循环中检查 cancel flag；取消时 `unlink` 远端半成品文件并返回"已取消"。（当前 `sftp_upload` 没有 cancel 检查，需补上。）

目标目录解析在主线程快速完成（读一次 Mutex），真正传输在 `spawn_blocking`，不阻塞异步运行时，也不碰终端。

### 3. 前端

- `App.tsx` 拖拽 handler：`invoke('drag_upload', { sessionId, files: paths, fallbackDir })`。
  - `fallbackDir`：需要拿到文件管理器当前远程路径。通过给 `FileBrowser` 加 `onRemotePathChange` 回调，把当前远程路径存到 `App` 的一个 ref，拖拽时取用。
- 右上角浮窗：进度项增加显示"目标目录"（来自 `transfer-progress` 的 `target` 字段），让用户清楚传到哪了。
- 拖拽覆盖层文案：保持"释放文件上传到远程"。

### 4. ZMODEM 门控（保留代码，默认不编译）

- `Cargo.toml` 增加 feature：
  ```toml
  [features]
  default = []
  zmodem = []
  ```
- 以下用 `#[cfg(feature = "zmodem")]` 门控：
  - `core/zmodem` 模块声明（`core/mod.rs` 里 `pub mod zmodem`）
  - `commands/zmodem.rs` 模块声明与 `zmodem_send` 命令注册（lib.rs）
  - `ssh.rs` reader 线程里的 ZMODEM 内联运行块、`ManagedSession` 的 `zmodem_active`/`zmodem_request` 字段、相关创建代码
- 默认构建（`./build.sh`、CI）不含 ZMODEM，drag 走 SFTP。
- `sz` 下载仍由前端 zmodem.js sentry 处理，不受影响。
- 想试 ZMODEM：`cargo build --features zmodem`。

实现注意：`ManagedSession` 的 zmodem 字段和 `ZmodemSendRequest` 用 `#[cfg(feature = "zmodem")]` 门控时，所有构造点和引用点都要同样门控，保证两种 feature 组合都能编译。

## 错误处理与边界

- 上传期间终端完全可用（独立 SFTP 通道）。
- 多文件：顺序上传，逐个进度；其中一个失败不影响其余，最后汇总。
- SFTP 连接断开：上传命令报错，终端 reader 线程不受影响。
- OSC7 拿不到：回退到 fallbackDir / home，浮窗显示实际目标。
- 注入钩子对 fish 等不生效：自动走回退，不报错。
- 取消：写循环检查 flag，删除半成品远端文件。
- OSC7 滚动缓冲设上限（如 1KB），防止异常流导致内存增长。

## 测试计划

1. 单元测试（core）：OSC7 解析 —— 完整序列、跨 read 分片、BEL 与 ESC\ 两种结束符、URL 解码、超长丢弃。
2. 手动测试：
   - bash/zsh：cd 到某目录后拖文件，确认传到该目录；传输中能正常敲命令。
   - fish/不支持 shell：确认回退到文件管理器目录，浮窗显示目标。
   - 大文件（>1GB）稳定传输 + 实时速率。
   - 传输中取消，半成品被删除。
   - 多标签页：A 传输时切到 B 正常使用，cwd 不串。
   - `cargo build --features zmodem` 能编译，ZMODEM 路径仍可用。

## 不做（YAGNI）

- 不做拖到文件管理器远程面板（只支持拖到终端区域；目标由 cwd/回退决定）。
- 不做并行多文件上传（顺序即可）。
- 不做断点续传（SFTP 覆盖写）。
