# LiteTerm HTTP API 自动化交互系统 — 设计文档

> 日期: 2026-07-10
> 状态: 已批准，待实现

## 1. 目标

为 LiteTerm 添加本地 HTTP API，使外部工具（Claude Code、脚本等）能够程序化地操作终端标签页：打开/关闭标签、发送命令、捕获输出。形成"发命令 → 读输出 → 分析 → 再发命令"的自动化闭环，提升开发调试效率。

## 2. 非目标

- 不内嵌 AI 推理引擎，智能分析由调用方负责
- 不自动判断命令是否执行完毕，由调用方控制读取节奏
- 第一期不覆盖 SFTP、串口、resize、隧道等操作（记入 TODO）

## 3. 架构

```
┌─────────────┐     curl / HTTP      ┌──────────────────┐
│  Claude Code │ ──────────────────→  │  HTTP API Server  │
│  (Bash tool) │ ←──────────────────  │  127.0.0.1:19526  │
└─────────────┘     JSON response    │  (axum, Rust)     │
                                      └────────┬─────────┘
                                               │ 共享 AppState (Arc)
                                      ┌────────▼─────────┐
                                      │  Tauri 后端核心    │
                                      │  (PTY/SSH/State)  │
                                      └────────┬─────────┘
                                               │ emit events
                                      ┌────────▼─────────┐
                                      │  React 前端       │
                                      │  (xterm.js)       │
                                      └──────────────────┘
```

### 3.1 HTTP 服务器

- 框架: axum（共享 Tauri 进程内的 tokio runtime）
- 绑定: `127.0.0.1:19526`（固定端口，LiteTerm 谐音）
- 认证: 启动时生成随机 token 写入 `~/.config/guishell/api_token`（权限 0600），请求需带 `Authorization: Bearer <token>`
- 启动时机: `tauri::Builder::setup()` 阶段 `tokio::spawn`
- 端口冲突处理: log 警告，不阻塞 Tauri 正常启动
- AppState 共享: `lib.rs` 中将 AppState 包装为 `Arc<AppState>`，Tauri `.manage()` 和 axum 路由 state 共用同一个 Arc 实例。AppHandle 也通过 `Arc` 传给 axum（用于 emit 前端事件和调用 SSH 连接逻辑）

### 3.2 输出捕获 — 环形缓冲区

每个终端（本地 PTY / SSH）在 AppState 中维护一个输出缓冲区：

```rust
pub struct TerminalOutputBuffer {
    buf: Vec<u8>,        // 固定 1MB
    capacity: usize,     // 1_048_576
    head: usize,         // 环形写入位置（buf 内偏移）
    write_pos: u64,      // 全局写入位置（单调递增，不回绕）
}
```

- PTY reader 线程现有逻辑不变（emit 给前端），额外把输出追加到缓冲区
- SSH reader 线程同理
- `write_pos` 作为游标，客户端每次 read 传入上次的 cursor，服务端返回增量
- cursor 过旧（数据已被覆盖）时返回缓冲区当前全部内容 + `truncated: true`

### 3.3 Tab 注册表

当前 AppState 有 `local_terminals` 和 `sessions` 两个 map，但不含标签元数据（label、type）。这些信息只在前端 React state 中。

新增 `tab_registry: Mutex<HashMap<String, TabInfo>>` 到 AppState：

```rust
pub struct TabInfo {
    pub id: String,
    pub label: String,
    pub tab_type: String,  // "local" / "ssh" / "serial"
}
```

- 现有 Tauri 命令（`open_local_terminal`、`ssh_connect`等）不改动，前端打开标签时通过新的 Tauri 命令 `register_tab(id, label, type)` 注册
- HTTP API 打开标签时由 handler 直接注册
- HTTP `GET /tabs` 从 `tab_registry` 读取
- 关闭标签时同步移除注册

### 3.4 前端联动

后端操作完成后通过 Tauri emit 通知前端同步 React state：

| 操作 | 事件名 | payload |
|------|--------|---------|
| 打开标签 | `api-tab-opened` | `{id, label, type, sshParams?}` |
| 关闭标签 | `api-tab-closed` | `{id}` |
| 切换焦点 | `api-tab-focus` | `{id}` |

前端新增一个 `useEffect` 监听这三个事件，约 30 行代码。

### 3.4 ANSI 过滤

`read` API 的 `data` 字段默认用 `strip-ansi-escapes` crate 剥离 ANSI 转义码，返回纯文本。
可通过 `?raw=true` 查询参数获取包含转义码的原始输出。
过滤在返回时执行，缓冲区始终存储原始数据。

## 4. API 接口总览

| 方法 | 路径 | 功能 |
|------|------|------|
| GET | `/api/v1/tabs` | 列出所有标签页 |
| POST | `/api/v1/tabs/local` | 打开本地终端 |
| POST | `/api/v1/tabs/ssh` | 打开 SSH 连接 |
| PUT | `/api/v1/tabs/:id/focus` | 切换活跃标签页 |
| POST | `/api/v1/tabs/:id/write` | 写入数据 |
| GET | `/api/v1/tabs/:id/read` | 读取新增输出 |
| DELETE | `/api/v1/tabs/:id` | 关闭标签页 |

详见 [API 文档](../api/http-api.md)。

## 5. 安全

- Token: 32 字节随机值 hex 编码（64 字符），每次启动重新生成
- 文件: `~/.config/guishell/api_token`，权限 `0600`
- 网络: 仅绑定 `127.0.0.1`，拒绝外部访问
- 认证失败返回 `401 Unauthorized`

## 6. 依赖变更

| crate | 用途 |
|-------|------|
| `axum` | HTTP 服务器 |
| `strip-ansi-escapes` | ANSI 转义码过滤 |
| `rand` | token 随机生成 |

`tokio` 已有，无需新增。

## 7. 文件改动范围

| 文件 | 改动 |
|------|------|
| 新增 `src-tauri/src/commands/api_server.rs` | HTTP 路由 + handler |
| 修改 `src-tauri/src/state.rs` | 添加 `TerminalOutputBuffer` + `TabInfo` + `tab_registry` |
| 修改 `src-tauri/src/commands/terminal.rs` | reader 线程追加写缓冲区 + `register_tab` 命令 |
| 修改 `src-tauri/src/commands/ssh.rs` | SSH reader 线程追加写缓冲区 |
| 修改 `src-tauri/src/lib.rs` | AppState 改为 Arc 包装 + setup 阶段启动 HTTP 服务器 |
| 修改 `src-tauri/Cargo.toml` | 新增 axum/strip-ansi-escapes/rand 依赖 |
| 修改 `src/App.tsx` | 监听 api-tab-* 事件 + 打开标签时调用 register_tab |
| 修改 `src-tauri/src/commands/mod.rs` | 导出 api_server 模块 |

## 8. 后续扩展（TODO）

以下操作留待后续版本：

- 终端 resize
- SFTP 文件操作
- 串口终端操作
- SSH 隧道管理
- WebSocket 实时输出推送（替代轮询 read）
- CLI 薄壳工具 `liteterm-cli`
