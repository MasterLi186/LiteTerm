# LiteTerm

轻量级跨平台 SSH 客户端 — FinalShell / Tabby / XShell 开源替代品。

基于 **Tauri 2 + React + TypeScript + xterm.js** 构建，内存占用 ~30MB，安装包仅 6MB（deb）。

## 功能特性

### 终端
- **多标签页** — 本地终端 / SSH / 串口，标签可拖拽排序、右键重命名
- **分屏** — 水平/垂直分屏，可嵌套，拖拽调整比例
- **多 Shell 支持** — bash / zsh / fish / dash 等系统已安装的 shell
- **串口终端** — 自动检测 USB/ACM 设备，可选波特率
- **终端搜索** — Ctrl+Shift+F 搜索终端内容
- **6 套主题** — 暗色默认 / Monokai / Solarized Dark / Dracula / One Dark / 浅色
- **选中即复制** — 选中自动复制，中键粘贴
- **录屏回放** — 录制终端操作为 asciicast v2 格式，支持回放（变速/暂停/进度条）
- **日志录制** — 右键开始录制，选择保存路径，自动去除 ANSI 转义码
- **快捷键自定义** — 9 组快捷键可自由配置

### SSH
- **连接管理** — 分组保存，密码存入系统密钥环（GNOME Keyring）
- **ProxyJump** — 跳板机支持，通过系统 ssh -J 实现
- **端口转发** — 本地隧道，支持多条同时运行
- **自动重连** — 断线后指数退避重连（最多 5 次）
- **会话持久化** — 关闭后自动记忆，下次启动恢复连接
- **批量命令** — 选中多个 SSH 标签，同时发送同一命令
- **ZMODEM** — 通过堡垒机/跳板机多跳传输文件（sz/rz）

### 系统监控（左侧面板）
- **CPU** — 型号、核心数、实时占用率（渐变进度条 + 发光效果）
- **内存/交换** — 实际用量 + 总量 + 百分比
- **进程列表** — 按内存/CPU/命令排序，实际 RSS 大小，CPU 底色进度条
- **进程管理器** — 全进程表，可排序，点击查看详情（命令行、环境变量、工作目录）
- **网络流量** — 多网卡选择，上下行速率，SVG 面积图
- **磁盘用量** — 所有挂载点，可用/总量 + 百分比
- **本地+远程** — 未连接 SSH 时显示本机监控

### 文件管理器（底部面板）
- **FileZilla 风格** — 左边本地文件，右边远程文件（SFTP）
- **下载/上传** — 右键菜单，传输进度条
- **目录导航** — 路径栏 + 排序 + 隐藏文件切换
- **命令收藏** — 保存常用命令，点击直接发送到终端

### 配置管理
- **导入/导出** — connections.toml 一键备份恢复
- **SSH 密钥管理** — 查看/生成密钥（Ed25519/RSA/ECDSA），复制公钥

## 安装

### 下载安装包

前往 [Releases](https://github.com/MasterLi186/LiteTerm/releases) 下载：

| 平台 | 格式 |
|------|------|
| Linux | `.deb` `.rpm` `.AppImage` |
| macOS | `.dmg` |
| Windows | `.msi` `.exe` |

### 从源码构建

```bash
# 系统依赖 (Ubuntu/Debian)
sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev libudev-dev

# 构建
git clone https://github.com/MasterLi186/LiteTerm.git
cd LiteTerm
npm install
./build.sh

# 运行
./run.sh
```

## 技术栈

| 层 | 技术 |
|---|------|
| 前端 | React 18 + TypeScript + Tailwind CSS |
| 终端 | xterm.js + FitAddon + SearchAddon + zmodem.js |
| 后端 | Rust + Tauri 2 |
| SSH/SFTP | ssh2 (libssh2) |
| 本地终端 | portable-pty |
| 串口 | serialport |
| 密钥存储 | secret-service (GNOME Keyring) |

## 截图

（待添加）

## 许可证

MIT
