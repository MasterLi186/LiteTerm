# Native Prototype: 多标签系统

## 目标

在 Rust 原生原型中实现完整的多标签系统，对标 guishell 的标签功能：多终端会话、标签栏 UI、新建/关闭/切换标签。

## 架构

```
┌──────────────────────────────────────────────────────────┐
│ 标签栏 (egui)                                            │
│ [终端 1 ×] [155bmc ×] [156bmc ×] [+]                     │
├──────────┬───────────────────────────────────────────────┤
│ 侧边栏   │ 活跃标签的终端 (wgpu)                          │
│ (egui)   │ TerminalState[activeTabId]                    │
│          │                                               │
│ 连接列表  │ alacritty_terminal grid → GPU 渲染             │
│          │                                               │
└──────────┴───────────────────────────────────────────────┘
```

## 数据模型

### Tab 结构

```rust
enum TabType {
    Local { shell_path: String },
    Ssh { host: String, port: u16, user: String, auth: String, key_path: String, label: String },
    Serial { device: String, baud_rate: u32 },
}

struct Tab {
    id: String,                          // uuid
    label: String,                       // 标签名
    tab_type: TabType,
    terminal: Arc<Mutex<TerminalState>>, // 每个标签独立的终端状态
    read_thread_running: bool,
}
```

### TabManager

```rust
struct TabManager {
    tabs: Vec<Tab>,
    active_idx: usize,
}

impl TabManager {
    fn new_local(&mut self, shell: &str, cols: u16, rows: u16);
    fn new_ssh(&mut self, conn: &SshConnection, cols: u16, rows: u16, proxy: EventLoopProxy);
    fn new_serial(&mut self, device: &str, baud: u32, cols: u16, rows: u16);
    fn close(&mut self, idx: usize);
    fn active(&self) -> Option<&Tab>;
    fn active_terminal(&self) -> Option<Arc<Mutex<TerminalState>>>;
}
```

## UI 设计

### 标签栏（egui TopBottomPanel::top）

- 每个标签：图标(类型指示) + 标签名 + × 关闭按钮
- 类型图标：● 本地(绿) / ● SSH(青) / ● 串口(黄)
- 活跃标签高亮背景
- 右端 [+] 按钮 → 弹出新标签选择器
- 鼠标中键点标签 = 关闭
- 标签可拖拽排序（后续实现）

### 新标签选择器（egui Window 弹窗）

- Shell 环境：列出系统可用 shell（bash / fish / zsh）
- SSH 连接：列出 connections.toml 的连接（跟侧边栏同源）
- 串口设备：列出 serialport::available_ports()
- 对标 guishell 的 NewTabSelector 组件

### 侧边栏交互

- 双击 SSH 连接 → 新建 SSH 标签（不替换当前）
- "本机" 点击 → 切换到本地终端标签（如果没有就新建）

## 键盘快捷键

| 快捷键 | 功能 |
|--------|------|
| Ctrl+Shift+T | 新建本地终端标签 |
| Ctrl+Shift+W | 关闭当前标签 |
| Ctrl+Tab | 下一个标签 |
| Ctrl+Shift+Tab | 上一个标签 |
| Ctrl+1~9 | 切换到第 N 个标签 |

## SSH 连接流程

1. 侧边栏双击连接 或 新标签选择器点击
2. 创建新 Tab（TabType::Ssh），标签名 = 连接 label
3. 后台线程 ssh::connect() → UserEvent::SshReady(tab_id, result)
4. 成功：Tab.terminal.apply_ssh_handle()，启动 read_loop
5. 失败：标签显示错误信息，用户可关闭或重试
6. 认证失败时弹密码输入框（egui Window）

## 渲染切换

- `App.terminal` 改为 `App.tab_manager`
- `do_render()` 只渲染 `tab_manager.active_terminal()` 的内容
- 非活跃标签的终端状态保持在内存（read_loop 继续运行，数据继续写入 grid）
- 切换标签时 renderer 读新的 TerminalState → 立即显示该终端的当前内容

## 模块变更

| 文件 | 改动 |
|------|------|
| `tab_manager.rs`（新）| TabManager、Tab 结构体 |
| `tab_bar.rs`（新）| egui 标签栏渲染 |
| `new_tab_dialog.rs`（新）| 新标签选择弹窗 |
| `main.rs` | App.terminal → App.tab_manager，事件分发 |
| `sidebar.rs` | 双击连接 → 新建标签而非替换 |
| `renderer.rs` | render() 接受动态 TerminalState 引用 |
| `terminal.rs` | spawn_shell/apply_ssh_handle 不变 |

## 不做

- 标签拖拽排序（后续）
- 分屏（后续）
- 标签右键菜单（后续）
- 标签持久化/恢复（后续）
