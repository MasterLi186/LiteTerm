# LiteTerm

轻量级跨平台 SSH / 串口终端客户端，使用 Rust + winit + wgpu + egui 原生实现，无 WebView。

LiteTerm 是本仓库唯一的活跃实现。历史上的 React/Tauri/GTK 版本已移除，仅保留在 `legacy-tauri` 分支和旧 tag 中。

## 功能

- **终端**：本地 Shell、SSH、串口；分屏、搜索、主题、字体与字号设置
- **SSH**：密码 / 密钥登录，连接书签与分组，自动重连
- **SFTP**：本地 + 远程双栏文件管理，上传、下载、拖拽
- **串口**：USB VID/PID、厂商、硬件 SN、USB 路径与速率展示，ADB 设备关联，按硬件身份重连
- **系统监控**：本地 / 远程 CPU、内存、交换、进程、网络、磁盘
- **进程与网络**：进程列表与详情、终止进程、端口与网络详情
- **效率**：命令补全、历史、收藏、快捷命令、批量命令、本地端口转发
- **传输**：ZMODEM 上传 / 下载（本地与直连 SSH）
- **其他**：终端录屏与回放、191 套配色主题、中英文界面、HTTP API

## 下载

从 [GitHub Releases](https://github.com/MasterLi186/LiteTerm/releases/latest) 下载最新版本：

| 平台 | 安装包 |
|---|---|
| Linux x86_64 | `.deb` |
| Windows x86_64 | NSIS `.exe` |
| macOS Intel | `.dmg` |
| macOS Apple Silicon | `.dmg` |

## 构建

要求 Rust stable。Linux 还需要系统依赖：

```bash
sudo apt install pkg-config libudev-dev libssl-dev
```

```bash
git clone https://github.com/MasterLi186/LiteTerm.git
cd LiteTerm

# debug 构建
cargo build --locked --bin liteterm-native

# 启动（二进制缺失或过期时自动构建）
./run-native.sh
```

### Release 打包

```bash
./build.sh
```

产物：`dist/installers/LiteTerm_<版本>_<架构>.deb`。

## 配置

配置目录：`~/.config/guishell/`

| 文件 | 内容 |
|---|---|
| `native-settings.toml` | 字体、主题、侧边栏、传输、SSH keepalive、ZMODEM、快捷键 |
| `connections.toml` | SSH 连接书签与分组 |
| `credentials.enc` | AES-256-GCM 加密密码文件 |
| `native-session.toml` | 标签页会话（本地 / SSH / 串口） |
| `native-api-token` / `native-api-port` | HTTP API 发现文件 |

## 架构

- 主线程独占 winit 事件循环和全部 UI 状态；后台线程通过 `EventLoopProxy<UserEvent>` 回传事件，并使用 generation / session 标识丢弃陈旧事件。
- 终端基于 `alacritty_terminal` + `vte`，由自研 wgpu 渲染器绘制：`cosmic-text` 做字形布局，字形图集 + instanced quad 渲染终端单元。
- egui 负责标签栏、侧边栏、对话框、文件管理器和设置界面。
- SSH 每个子系统使用独立连接线程（`ssh2::Session` 非 `Send`）；远程监控按 `user@host:port` 共享连接。
- HTTP API 通过 axum 仅监听 IPv4 loopback，请求转发到主线程执行。

## 文档

- [HTTP API v1](docs/api/http-api.md)
- [开发约束](docs/NATIVE_ONLY.md)
- [开发策略](AGENTS.md)
- [待办与技术债](TODO.md)

## 发布

推送 `v*` tag 会触发 GitHub Actions 在 Linux、Windows、macOS Intel 和 macOS Apple Silicon 上构建并生成 Release。版本号由发布流程同步到 `Cargo.toml` 与 `Cargo.lock`。旧的 `native-v*` 标签仅作为历史版本保留。
