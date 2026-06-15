# GuiShell Design Spec

FinalShell 替代品 — 轻量级原生 Linux SSH 客户端，对齐 FinalShell 核心功能，摒弃 Java 生态。

## 技术选型

| 层 | 选择 | 说明 |
|---|------|------|
| 语言 | Rust | 零 GC、内存安全、极低资源占用 |
| GUI | GTK4 + libadwaita (gtk4-rs) | Linux 原生，未来可跨平台 |
| 终端 | VTE4 | GNOME Terminal 同款引擎 |
| SSH/SFTP | libssh2 (ssh2-rs) | 成熟、SFTP 内置 |
| 密钥存储 | libsecret | GNOME Keyring 集成 |
| 图表绘制 | cairo-rs | 系统监控面板折线/面积/柱状图 |
| 异步运行时 | tokio | SSH 会话、SFTP 传输、监控采集 |
| 配置格式 | TOML (serde) | 可读性好，Rust 生态原生支持 |

目标平台：Linux 优先，保留 macOS/Windows 跨平台扩展能力。

## 整体架构

```
guishell/
├── src/
│   ├── main.rs              # 入口、GTK Application 初始化
│   ├── ui/                  # UI 层 — 纯展示逻辑
│   │   ├── window.rs        # 主窗口布局
│   │   ├── sidebar.rs       # 左侧栏（连接管理 + 监控）
│   │   ├── terminal.rs      # VTE 终端封装
│   │   ├── tabs.rs          # 标签页管理
│   │   ├── split.rs         # 分屏管理（二叉分割树）
│   │   ├── file_browser.rs  # 双栏文件浏览器
│   │   └── monitor.rs       # 系统监控图表绘制
│   ├── core/                # 业务逻辑 — 无 UI 依赖
│   │   ├── ssh.rs           # SSH 连接管理
│   │   ├── sftp.rs          # SFTP 文件操作
│   │   ├── session.rs       # 会话生命周期
│   │   ├── monitor.rs       # 远程指标采集与解析
│   │   └── transfer.rs      # 传输队列管理
│   ├── config/              # 配置与持久化
│   │   ├── connections.rs   # 连接信息存储
│   │   ├── settings.rs      # 全局设置
│   │   └── keyring.rs       # 系统密钥环集成
│   └── plugin/              # 监控指标插件系统
│       ├── registry.rs      # 插件注册与发现
│       └── builtin/         # 内置指标采集器
├── Cargo.toml
└── docs/
```

UI 层与 core 层严格分离：core 不依赖 GTK，可独立测试。

## 主窗口布局

```
┌─────────────────────────────────────────────────────────────┐
│  GuiShell                                                   │
│  ┌──────────┬──────────────────────────────────────────────┐ │
│  │ 左侧栏    │  主区域                                      │ │
│  │          │  ┌─ Tab1 ─┬─ Tab2 ─┬─ Tab3 ──────────────┐  │ │
│  │ ┌──────┐ │  │                                        │  │ │
│  │ │连接   │ │  │  ┌──────────────┬──────────────────┐   │  │ │
│  │ │管理器 │ │  │  │ 终端面板 A    │ 终端面板 B        │   │  │ │
│  │ │(分组  │ │  │  │ (VTE)       │ (VTE)            │   │  │ │
│  │ │树形)  │ │  │  │             │                  │   │  │ │
│  │ └──────┘ │  │  └──────────────┴──────────────────┘   │  │ │
│  │ ┌──────┐ │  └────────────────────────────────────────┘  │ │
│  │ │系统   │ │                                              │ │
│  │ │监控   │ │  ┌─ 底部面板（可折叠）─────────────────────┐  │ │
│  │ │面板   │ │  │  文件管理器（双栏：本地 | 远程）         │  │ │
│  │ │(图表) │ │  │  传输队列 / 进度条                      │  │ │
│  │ └──────┘ │  └──────────────────────────────────────────┘  │ │
│  └──────────┴──────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

- 左侧栏可折叠（Ctrl+B）
- 底部文件管理器可折叠（Ctrl+Shift+E）
- 各区域大小可拖拽调整，记忆用户偏好

## 连接管理

### 配置存储

路径：`~/.config/guishell/connections.toml`

```toml
[groups.production]
label = "生产环境"
color = "#e74c3c"

[groups.production.hosts.web01]
label = "Web Server 01"
host = "192.168.1.10"
port = 22
user = "root"
auth = "keyring"           # "keyring" | "key" | "agent"
key_path = ""              # auth=key 时使用
charset = "utf-8"

[groups.dev]
label = "开发环境"
color = "#2ecc71"

[groups.dev.hosts.local_vm]
label = "本地虚拟机"
host = "192.168.56.101"
port = 22
user = "lfl"
auth = "keyring"
```

### 密码存储

- 通过 libsecret API 存入 GNOME Keyring
- keyring key 格式：`guishell:ssh://{user}@{host}:{port}`
- 密码从不写入磁盘文件
- 支持 ssh-agent 转发，自动检测 `SSH_AUTH_SOCK`

### 认证流程

```
连接请求
  → ssh-agent (检测 SSH_AUTH_SOCK)
  → keyring 查询密码
  → 密钥文件 (key_path)
  → 全部失败 → 弹出认证对话框手动输入 → 可选保存到 keyring
```

### 分组管理

- 树形视图展示，按分组折叠/展开
- 分组可设置颜色标签，标签页继承分组颜色
- 支持拖拽移动主机到不同分组
- 双击主机快速连接，右键菜单编辑/删除/复制

## SSH 会话

### 生命周期

```
连接请求 → 创建 ssh2::Session（独立 tokio 任务）
         → 认证（agent → keyring → key 按顺序尝试）
         → 成功 → 打开 channel → 绑定到 VTE PTY
                → 启动监控采集任务
                → 启动 SFTP 子系统（按需）
         → 失败 → 弹出认证对话框 → 重试
```

### 多会话管理

- 每个 SSH 连接运行在独立 tokio 任务中，不阻塞 UI 线程
- 连接断开自动检测（keepalive 心跳），提示重连
- 支持同一主机开多个会话（多标签/多分屏）

## 终端系统

### VTE 终端

- 每个终端面板封装一个 `VteTerminal` 实例
- SSH channel 字节流通过 PTY 桥接到 VTE 渲染
- 支持 xterm-256color，滚动缓冲区可配置
- 字体、配色方案在设置中可配，实时生效
- 支持终端内搜索（Ctrl+Shift+F）

### 标签页

- 顶部标签栏，可拖拽排序
- 中键点击关闭，`+` 按钮新建
- 标签颜色继承连接分组颜色
- 右键菜单：重命名、复制会话、关闭、关闭其他

### 分屏

标签内维护一棵二叉分割树：

```
        Split(V)
       /        \
    Pane(A)   Split(H)
              /      \
          Pane(B)  Pane(C)
```

- `Ctrl+Shift+D` — 水平分屏
- `Ctrl+Shift+R` — 垂直分屏
- `Ctrl+Shift+W` — 关闭当前面板
- `Ctrl+Shift+方向键` — 焦点切换
- 拖拽分割条调整大小
- 每个面板可绑定独立 SSH 会话

### 完整快捷键表

| 操作 | 快捷键 |
|------|--------|
| 新建标签 | Ctrl+Shift+T |
| 关闭标签 | Ctrl+Shift+Q |
| 切换标签 | Ctrl+Tab / Ctrl+1~9 |
| 水平分屏 | Ctrl+Shift+D |
| 垂直分屏 | Ctrl+Shift+R |
| 关闭面板 | Ctrl+Shift+W |
| 焦点移动 | Ctrl+Shift+方向键 |
| 终端搜索 | Ctrl+Shift+F |
| 文件管理器 | Ctrl+Shift+E |
| 左侧栏 | Ctrl+B |
| 全屏 | F11 |

快捷键可在设置中自定义。

## 系统监控面板

### 数据采集

通过 SSH channel 执行一条合并命令，每次采集一次往返：

```bash
cat /proc/stat /proc/meminfo /proc/net/dev /proc/diskstats /proc/loadavg; \
df -h; \
cat /sys/class/thermal/thermal_zone*/temp 2>/dev/null; \
ps aux --sort=-%cpu | head -11
```

- 采集间隔：默认 2 秒，可配置 1~10 秒
- 环形缓冲区保留最近 60 个采样点
- 不依赖远程安装任何 agent

### 默认指标集

| 指标 | 数据源 | 展示方式 |
|------|--------|----------|
| CPU 使用率 | /proc/stat | 折线图（总/各核心） |
| 内存使用 | /proc/meminfo | 面积图（used/cached/free） |
| 磁盘用量 | df -h | 横向柱状图（各挂载点） |
| 网络流量 | /proc/net/dev | 双向折线图（上行/下行） |
| 系统负载 | /proc/loadavg | 数字 + 迷你趋势线 |
| IO 等待 | /proc/stat (iowait) | 折线图 |
| 温度 | thermal_zone*/temp | 数字 + 颜色指示 |
| Top 进程 | ps aux | 排序列表（CPU%/MEM%） |

### 左侧面板布局

```
┌──────────────┐
│ CPU ██████ 73%│
│ ╱╲╱╲╱╲_╱╲╱  │
├──────────────┤
│ MEM 2.1/7.8G │
│ ▓▓▓▓▓▒▒░░░░ │
├──────────────┤
│ Load 1.2 0.8 │
│ IO wait 3.2% │
├──────────────┤
│ NET ↑12K ↓85K│
│ ╱╲_╱╲╱╲╱╲╱  │
├──────────────┤
│ DISK         │
│ / ████░ 68%  │
│ /home █░ 23% │
├──────────────┤
│ TEMP 52°C    │
├──────────────┤
│ TOP PROCS    │
│ java   34.2% │
│ node   12.1% │
│ nginx   3.4% │
└──────────────┘
```

### 插件系统

```rust
pub trait MetricPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn collect_command(&self) -> &str;
    fn parse(&self, raw: &str) -> MetricData;
    fn render(&self, ctx: &cairo::Context, data: &MetricData, rect: Rect);
}
```

- 内置指标均实现此 trait，可通过配置文件逐项开关
- 未来可支持动态加载 .so 插件扩展自定义指标

### 监控配置

路径：`~/.config/guishell/monitor.toml`

```toml
interval_secs = 2
history_points = 60

[panels]
cpu = { enabled = true, show_per_core = false }
memory = { enabled = true }
disk = { enabled = true }
network = { enabled = true, interface = "auto" }
load = { enabled = true }
iowait = { enabled = true }
temperature = { enabled = true }
top_procs = { enabled = true, count = 5 }
```

## 文件管理器与传输系统

### 双栏文件浏览器

底部面板，可折叠：

```
┌─ 文件管理器 ──────────────────────────────────────────────┐
│ ┌─ 本地 ─────────────────┬─ 远程 (web01) ──────────────┐ │
│ │ /home/lfl/Downloads     │ /var/log                     │ │
│ │ ┌─────┬──────┬───────┐ │ ┌─────┬──────┬───────┐      │ │
│ │ │名称  │大小   │修改时间│ │ │名称  │大小   │修改时间│      │ │
│ │ ├─────┼──────┼───────┤ │ ├─────┼──────┼───────┤      │ │
│ │ │ ..  │      │       │ │ │ ..  │      │       │      │ │
│ │ │ img │ 4K   │06-12  │ │ │syslog│2.1M │06-13  │      │ │
│ │ │a.txt│ 12K  │06-13  │ │ │auth  │890K │06-13  │      │ │
│ │ └─────┴──────┴───────┘ │ └─────┴──────┴───────┘      │ │
│ └────────────────────────┴──────────────────────────────┘ │
│ ┌─ 传输队列 ────────────────────────────────────────────┐ │
│ │ ↓ syslog → ~/Downloads    2.1M  ████████░░ 78%  1.2M/s│ │
│ │ ⏳ auth.log → ~/Downloads  890K  等待中                  │ │
│ └───────────────────────────────────────────────────────┘ │
└───────────────────────────────────────────────────────────┘
```

### SFTP 实现

```
SSH Session
    ↓
ssh2::Sftp (子系统)    ← 复用已有 SSH TCP 连接
    ↓
SftpWorker (独立 tokio 任务)
    ↓
TransferQueue (MPSC channel)
    ├── Download { remote_path, local_path }
    ├── Upload { local_path, remote_path }
    └── ListDir { path, reply_tx }
```

### 交互操作

| 操作 | 方式 |
|------|------|
| 下载 | 远程栏双击 / 右键→下载 / 拖拽到本地栏 |
| 上传 | 本地栏双击 / 右键→上传 / 拖拽到远程栏 |
| 批量选择 | Ctrl+点击多选 / Ctrl+A 全选 |
| 新建文件夹 | 右键→新建文件夹 |
| 删除 | 右键→删除（二次确认） |
| 重命名 | F2 或右键→重命名 |
| 查看权限 | 右键→属性 |
| 快速跳转 | 路径栏直接输入路径回车 |

### 终端内快捷命令

在终端内用 `@` 前缀拦截，GuiShell 捕获后执行 SFTP 操作：

```bash
@get /var/log/syslog                    # 下载到默认目录
@get /var/log/syslog ~/Desktop/         # 下载到指定目录
@put ~/files/config.yaml /etc/app/      # 上传
@ls /var/log/                           # 在文件管理器中打开该目录
```

- 支持 Tab 补全远程路径
- 传输进度在终端内显示

### 传输可靠性

- 大文件（>10MB）支持断点续传
- 传输失败自动重试 3 次，间隔递增
- 队列支持暂停/恢复/取消单个任务
- 同名文件冲突：覆盖 / 重命名 / 跳过 / 全部应用

## ZMODEM 多跳传输

### 问题背景

企业堡垒机（如 JumpServer）不允许 ProxyJump 直连，只能交互式逐级跳转。
此时本地 SSH Session 只到堡垒机，SFTP 无法触达最终目标机。
ZMODEM 协议将文件编码进终端字节流，不依赖 SFTP 通道，穿透任意层跳转。

### 传输方式决策

```
连接建立时 GuiShell 自动判断：

直连主机 / ProxyJump ─→ SFTP（主通道，完整文件管理器可用）
交互式多跳（堡垒机）  ─→ ZMODEM（sz/rz 终端流拦截）
```

用户也可手动切换：在任何会话中执行 `sz`/`rz` 都会触发 ZMODEM 模式。

### ZMODEM 工作原理

```
目标机 shell                堡垒机              本地 GuiShell
    │                         │                      │
    │ sz filename             │                      │
    │ ──ZMODEM 帧──→          │                      │
    │   (编码进终端字节流)      │ ──透传字节流──→       │
    │                         │                      │ 检测 ZMODEM 头
    │                         │                      │ **\x18B00...
    │                         │                      │ 切换接收模式
    │                         │                      │ 解码帧 → 写文件
    │                         │                      │ ACK/NAK 应答
    │                         │                      │ ←── 应答帧 ──
    │    ←── 透传 ──          │                      │
    │ 收到 ACK，发下一块       │                      │
    │         ...              │                      │
    │ 传输完成 (EOT)           │                      │ 恢复正常终端模式
```

### 实现细节

**检测与切换：**

VTE 字节流经过 ZmodemDetector 过滤层：

```
VTE 输入流 → ZmodemDetector → 正常字节 → VTE 渲染
                  ↓ (检测到 ZMODEM 头)
             ZmodemReceiver / ZmodemSender
                  ↓
             文件写入/读取 + 进度回调
```

- 检测 ZMODEM 起始序列：`**\x18B00` (ZRINIT) 或 `**\x18B01` (ZRQINIT)
- 检测到后暂停 VTE 渲染，进入 ZMODEM 收发模式
- 传输完成或超时后恢复正常终端模式

**协议支持：**

| 特性 | 支持 |
|------|------|
| ZMODEM 接收 (sz → 下载) | 是 |
| ZMODEM 发送 (rz → 上传) | 是 |
| CRC32 校验 | 是 |
| 断点续传 (-r 参数) | 是 |
| 批量传输 (sz file1 file2) | 是 |
| 自动检测编码 (二进制安全) | 是 |

**UI 集成：**

ZMODEM 传输触发时：
- 终端区域顶部弹出传输进度条（不遮挡终端内容）
- 显示：文件名、大小、进度百分比、速率
- 提供取消按钮（发送 ZCANCEL 帧）
- 下载完成后 toast 通知，点击可打开文件所在目录

**配置：**

```toml
# settings.toml 追加
[zmodem]
enabled = true
auto_detect = true            # 自动检测 ZMODEM 头
download_dir = "~/Downloads"  # ZMODEM 下载保存目录（独立于 SFTP）
timeout_secs = 30             # 无数据超时，自动退出 ZMODEM 模式
```

### 用户操作流程

**堡垒机场景下载文件：**

```
1. GuiShell 连接堡垒机
2. 在堡垒机选择目标服务器（交互式菜单）
3. 登录目标服务器后，终端内执行：
   $ sz /var/log/syslog
4. GuiShell 自动检测 ZMODEM 头，弹出进度条
5. 文件保存到 ~/Downloads/syslog
6. 终端恢复正常
```

**堡垒机场景上传文件：**

```
1. 已登录目标服务器
2. 终端内执行：
   $ rz
3. GuiShell 检测到 ZMODEM 接收等待
4. 弹出本地文件选择对话框
5. 用户选择文件，GuiShell 通过 ZMODEM 发送
6. 文件到达目标服务器当前目录
```

### 架构变更

模块目录新增：

```
src/
├── core/
│   ├── zmodem/
│   │   ├── mod.rs         # ZMODEM 协议状态机
│   │   ├── detect.rs      # 魔术头检测 (字节流过滤层)
│   │   ├── frame.rs       # 帧编解码 (ZHDR/ZDATA/ZEOF/ZFIN)
│   │   ├── receive.rs     # 接收端 (sz 触发)
│   │   └── send.rs        # 发送端 (rz 触发)
```

## 配置文件总览

```
~/.config/guishell/
├── connections.toml    # 连接信息（密码在 keyring）
├── settings.toml       # 全局设置
├── monitor.toml        # 监控面板配置
└── keybindings.toml    # 自定义快捷键
```

### settings.toml

```toml
[terminal]
font = "Monospace 12"
scrollback_lines = 10000
color_scheme = "dark"         # "dark" | "light" | "solarized" | "custom"
cursor_blink = true

[appearance]
theme = "system"              # "system" | "dark" | "light"
sidebar_width = 220
file_browser_height = 250
show_sidebar = true
show_file_browser = false     # 默认折叠

[transfer]
default_download_dir = "~/Downloads"
resume_threshold_mb = 10
max_retries = 3
concurrent_transfers = 3

[ssh]
keepalive_interval_secs = 30
connect_timeout_secs = 10
default_charset = "utf-8"
```

## 传输方式总览

| 场景 | 传输方式 | 文件管理器 |
|------|----------|-----------|
| 直连主机 | SFTP | 双栏可用 |
| ProxyJump 跳转 | SFTP（端到端隧道） | 双栏可用 |
| 交互式堡垒机多跳 | ZMODEM (sz/rz) | 不可用，用终端 sz/rz |

GuiShell 在连接建立时自动判断：直连/ProxyJump 会话启用 SFTP 文件管理器；
检测到交互式多跳时提示用户使用 sz/rz 传输文件。
用户在任何会话中执行 sz/rz 都会自动触发 ZMODEM 模式。

## 非目标（明确排除）

- 不内置代码编辑器（用 VTE 打开 vim/nano 即可）
- 不做 Web 版本
- 不支持 Telnet/Serial 等非 SSH 协议（一期）
- 不做远程桌面/X11 转发
- 不收集/上传任何用户数据
