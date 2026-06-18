# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概述

LiteTerm 是一个轻量级跨平台 SSH 客户端（FinalShell / Tabby / XShell 开源替代品），基于 Tauri 2 (Rust) + React (TypeScript) + xterm.js 构建。所有界面文本为中文。commit 信息也使用中文。

## 构建与运行

```bash
./build.sh      # 清理缓存 → 编译前端 → 编译后端（每次都全量重建）
./run.sh        # 直接运行已编译的二进制（不编译，需先 build）
```

手动步骤：
```bash
npm run build                      # 前端 → dist/
cd src-tauri && cargo build        # 后端 → src-tauri/target/debug/guishell-tauri
```

**禁止使用 `npx tauri dev`** — 系统文件描述符不足，file watcher 会崩溃。

发布构建：
```bash
npx tauri build                    # 生成 deb/rpm/AppImage/dmg/msi/exe
git tag v0.x.0 && git push origin v0.x.0  # 触发 CI 三平台构建 + Release
```

## 测试

Rust 核心层测试（使用项目根目录的 Cargo.toml）：
```bash
cargo test                                    # 全部测试
cargo test --test zmodem_frame_test           # 单个测试文件
cargo test --test monitor_parse_test -v       # 详细输出
```

测试文件在 `tests/`，测试 `src/` 下的旧 GTK4 代码模块（与 `src-tauri/src/` 中的核心逻辑相同）。

## 架构

### 两套代码共存

| 路径 | 用途 | 状态 |
|------|------|------|
| `src/*.rs` + 根 `Cargo.toml` | 旧 GTK4 版本 | 仅保留测试 |
| `src-tauri/` | Tauri 2 Rust 后端 | 活跃开发 |
| `src/*.tsx` + `index.html` | React 前端 | 活跃开发 |

### 后端 Rust (`src-tauri/src/`)

```
lib.rs            — Tauri 应用启动、命令注册、AppState 初始化
state.rs          — AppState（Mutex HashMap 管理会话、终端、连接、SFTP、隧道、录屏）
commands/
  terminal.rs     — 本地终端（portable-pty）、Shell 列表
  ssh.rs          — SSH 连接（libssh2 / ProxyJump 走系统 ssh -J）
  connection.rs   — 连接配置 CRUD（TOML 持久化）
  keyring.rs      — GNOME Keyring 密码存取
  monitor.rs      — 系统监控采集（SSH exec / 本地 /proc）
  sftp.rs         — SFTP 文件操作 + 下载上传 + save_file
  serial.rs       — 串口终端（serialport crate）
  tunnel.rs       — SSH 端口转发（本地隧道）
  process.rs      — 远程进程列表 + 详情
  recording.rs    — 终端录屏（asciicast v2 格式）
  config_io.rs    — 导入/导出配置
  ssh_keys.rs     — SSH 密钥管理
config/           — 数据模型（Settings, ConnectionStore, KeyringEntry）
core/             — 业务逻辑（无 UI 依赖）
  monitor.rs      — /proc 解析器、MetricBuffer 环形缓冲区
  zmodem/         — ZMODEM 协议（帧编解码、检测器、收发状态机）
```

### 关键线程模型

`ssh2::Session` 是 `!Send` — 每个 SSH 子系统（终端、监控、SFTP）在独立线程上创建自己的 TCP+SSH 连接。通信方式：
- `std::sync::mpsc` 通道：前端 → 后端输入
- `app_handle.emit("event-name", payload)`：后端 → 前端输出
- `AtomicBool` stop flag：用于终止线程（串口、隧道、监控）

Reader 线程有 500ms 启动延迟，等前端注册事件监听器。

### 前端 React (`src/`)

```
App.tsx                           — 主布局、标签管理、连接流程、快捷键、重连逻辑
components/Terminal/TerminalPane  — xterm.js 封装（双 div resize 架构、搜索、主题、录屏、ZMODEM）
components/Terminal/SplitContainer— 分屏递归渲染
components/Sidebar/SystemInfoPanel— 卡片式系统监控面板（CPU/内存/进程/网络/磁盘）
components/FileManager/FileBrowser— FileZilla 风格双栏文件管理器（本地+远程 SFTP）
components/ProcessManager/        — 进程表格 + 进程详情
components/ConnectionDialog       — SSH 连接新建/编辑对话框
components/NewTabSelector         — 新建标签选择器（Shell/SSH/串口）
components/TunnelManager          — SSH 端口转发管理
components/BatchCommand           — 批量命令执行
components/ShortcutSettings       — 快捷键自定义
components/SshKeyManager          — SSH 密钥管理
components/RecordingPlayer        — 终端录屏回放
```

### xterm.js Resize 架构

TerminalPane 使用双 div 模式防止 xterm 画布撑爆 flex 布局：
- **wrapperRef**（外层）：`position: absolute; inset: 0` — 由父级 flex 决定大小，ResizeObserver 监听此 div
- **containerRef**（内层）：在 fit() 前从 wrapper 的 `getBoundingClientRect()` 获取精确像素尺寸

Resize 在 50/150/400ms 三个延迟点触发。fit() 后必须调用 `term.refresh()`。

### SSH 连接初始 PTY 尺寸

`sshConnect()` 辅助函数从窗口尺寸估算 cols/rows 传给后端，避免硬编码 80x24。连接后 xterm.js 的 `onResize` 事件会持续同步实际尺寸。

## 配置文件

用户配置存储在 `~/.config/guishell/`：
- `connections.toml` — SSH 连接书签（密码在 GNOME Keyring，不写入磁盘）
- `settings.toml` — 终端字体、外观、传输、SSH keepalive 设置

前端本地存储（localStorage）：
- `guishell_terminal_theme` — 终端主题名称
- `guishell_cmd_history` — 命令历史
- `guishell_snippets` — 命令收藏
- `guishell_sessions` — 会话持久化（自动重连）
- `guishell_shortcuts` — 自定义快捷键

## 跨平台注意事项

- `secret-service` crate 仅 Linux（`cfg(target_os = "linux")`），其他平台 keyring 返回 stub
- `std::os::unix::fs::MetadataExt` 用 `cfg(unix)` 条件编译，Windows 回退
- `exec_local_command` 在非 Unix 平台返回错误（本地监控依赖 /proc）
- `.cargo/config.toml` 是本地开发用的 PKG_CONFIG_PATH，已加入 .gitignore 不提交

## CI/CD

GitHub Actions（`.github/workflows/build.yml`）：
- 触发条件：推送 `v*` tag 或手动触发
- 三平台构建：Ubuntu 22.04 / macOS-latest / Windows-latest
- 自动生成 changelog（上一个 tag 到当前的 commit 列表）
- 产物：deb/rpm/AppImage/dmg/msi/exe
