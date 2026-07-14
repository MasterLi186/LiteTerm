# Native Prototype: 读取连接 + SSH 会话

## 目标

在 Rust 原生原型中实现：从 `~/.config/guishell/connections.toml` 读取真实连接配置，侧边栏显示，双击发起 SSH 连接，终端显示 SSH 会话。

## 架构

```
connections.toml → ConnectionStore（TOML 解析）
                        ↓
              Sidebar（egui）显示真实连接列表（分组 + 折叠）
                        ↓ 双击
              ssh::connect() → libssh2 Session → Channel → PTY
                        ↓
              reader thread → mpsc → alacritty_terminal → GPU 渲染
              writer thread ← keyboard input
```

## 模块设计

### 1. connections.rs（新模块）

从 `src-tauri/src/config/connections.rs` 提取 `ConnectionStore`、`GroupConfig`、`HostConfig` 结构体和 TOML 解析。去掉 Tauri 依赖。

```rust
pub struct HostConfig {
    pub label: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: String,        // "password" | "key"
    pub key_path: Option<String>,
    pub proxy_jump: Option<String>,
}

pub struct GroupConfig {
    pub label: String,
    pub color: String,       // "#58a6ff"
    pub hosts: BTreeMap<String, HostConfig>,
}

pub struct ConnectionStore {
    pub groups: BTreeMap<String, GroupConfig>,
}

impl ConnectionStore {
    pub fn load() -> Self { /* 从 ~/.config/guishell/connections.toml 读取 */ }
}
```

### 2. ssh.rs（新模块）

从 `src-tauri/src/commands/ssh.rs` 提取 `do_ssh_connect` 核心逻辑：

- TCP 连接 → `ssh2::Session` 握手 → 认证（密码/密钥）→ `channel_session()` → 请求 PTY → `shell()`
- 返回 `SshSession { reader: Box<dyn Read+Send>, writer: Box<dyn Write+Send>, resize_tx }`
- 去掉 `Tauri State`、`AppHandle`、`emit` 依赖
- ProxyJump 暂不实现（直连优先）

```rust
pub struct SshSession {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
}

pub fn connect(host: &str, port: u16, user: &str, auth: &str, key_path: Option<&str>, cols: u16, rows: u16) -> Result<SshSession, String> {
    // TCP → libssh2 → auth → channel → pty → shell
}
```

### 3. sidebar.rs（修改）

- `Sidebar::new()` 调用 `ConnectionStore::load()` 填充连接列表
- 分组折叠/展开
- 双击设置 `on_connect = Some(连接信息)`

### 4. terminal.rs（修改）

- 新增 `spawn_ssh(host, port, user, auth, key_path, cols, rows)` 方法
- 内部调用 `ssh::connect()` 获取 reader/writer
- reader/writer 接入 alacritty_terminal（复用现有的 read_loop 模式）

### 5. main.rs（修改）

- `resumed()` 中检查 `sidebar.on_connect`，触发 `terminal.spawn_ssh()`
- 用户事件循环中同样检查

## 依赖

native-prototype/Cargo.toml 新增：
- `ssh2 = "0.9"` （libssh2 绑定）
- `toml = "0.8"` （TOML 解析）
- `serde = { version = "1", features = ["derive"] }`
- `dirs = "6"` （~/.config 路径）

## 不做

- 多标签（下一个子系统）
- 密码输入对话框（先用 key 认证）
- ProxyJump（直连优先）
- 监控面板 / 命令栏
- SFTP / 端口转发
