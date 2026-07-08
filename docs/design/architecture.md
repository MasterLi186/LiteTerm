# LiteTerm 架构设计文档

## 技术栈

| 层 | 技术 | 说明 |
|---|------|------|
| 前端 | React 18 + TypeScript + xterm.js | 终端渲染、UI 组件 |
| 后端 | Rust + Tauri 2 | SSH/SFTP/PTY/串口/隧道 |
| 构建 | Vite + Cargo | 前端打包 + Rust 编译 |
| 打包 | Tauri bundle | deb/rpm/AppImage/dmg/exe |
| CI | GitHub Actions | 三平台构建 + 质量门 |

## 进程模型

```
guishell-tauri (主进程, Rust)
├── WebKitWebProcess (渲染进程, 前端 UI)
│   ├── React App
│   ├── xterm.js × N (每个终端面板)
│   └── 各 UI 组件
├── WebKitNetworkProcess (网络进程)
├── SSH reader 线程 × N (每个 SSH 连接)
├── SSH monitor 线程 × N (每台主机一个,共享)
├── PTY reader 线程 × N (本地终端)
├── Tunnel listener 线程 × N
└── sysinfo 本地监控线程
```

## SSH 连接架构

每个 SSH 标签打开 **3 条独立 SSH 连接**:
1. **终端** — PTY channel,交互式 shell
2. **监控** — exec channel,每 2 秒执行 ps/stat/meminfo(同一主机共享)
3. **SFTP** — SFTP subsystem,文件管理+命令执行

每条连接独立线程 + 独立 TcpStream + 独立 libssh2 Session(`ssh2::Session` 是 `!Send`)。

## 数据流

```
用户键盘 → xterm onData → invoke('terminal_write') → mpsc channel → SSH channel.write
SSH channel.read → emit('terminal-output') → xterm.write (渲染)
SSH channel EOF/Error → emit('terminal-closed') → 前端显示断开提示
```

## 状态管理

### 后端 (AppState)
```rust
AppState {
    sessions:        HashMap<String, ManagedSession>,  // SSH 会话(终端+监控+ZMODEM)
    local_terminals: HashMap<String, LocalTerminal>,   // 本地/串口终端
    sftp_sessions:   HashMap<String, SftpHandle>,      // SFTP 连接
    tunnels:         HashMap<String, TunnelHandle>,     // 端口转发
    recordings:      HashMap<String, Recording>,       // 终端录屏
    transfer_cancel: HashMap<String, Arc<AtomicBool>>,  // 传输取消标志
    connections:     ConnectionStore,                   // 连接书签
    settings:        Settings,                         // 配置
}
```

### 前端 (React State)
- `tabs[]` — 标签列表(SSH/本地/串口/进程/录屏)
- `splitTrees{}` — 每个标签的分屏树
- `reconnecting{}` — 断开重连状态
- `monitorCache` — 监控数据缓存(按 host:port:user)
- `remoteFileCache` — 远端文件列表缓存
- `localStorage` — 会话持久化、命令历史、快捷键、主题

## 密码存储

- **Linux**: GNOME Secret Service (D-Bus) + AES-256-GCM 文件 fallback
- **Windows/macOS**: AES-256-GCM 加密文件(`~/.config/guishell/credentials.enc`)
- **密钥派生**: PBKDF2-HMAC-SHA256(hostname + username + app salt, 100000 次)
- **Machine-bound**: 文件拷到别的机器无法解密

## 日志系统

- 后端: `app_log!("TAG", "message")` → `~/guishell.log`(全局 Mutex 防交错)
- 前端: `log("TAG", "message")` → 内存 buffer + `invoke('frontend_log')` → 同一日志文件
- 日志格式: `[epoch.ms] [TAG] message`
