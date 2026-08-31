# LiteTerm

> **开发状态：Native Only。** 当前唯一开发和运行目标是 `native-prototype/`（Rust + winit + wgpu + egui），通过 `./run-native.sh` 启动。React/Tauri 与旧 GTK 代码仅作历史参考，除非明确要求，否则不再修改。详见 [Native-only 开发约束](docs/NATIVE_ONLY.md)。

轻量级跨平台 SSH 客户端 — FinalShell / Tabby / XShell 开源替代品。

当前版本基于 **Rust + winit + wgpu + egui** 构建。

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
- **连接管理** — 分组保存，密码存入本地加密凭据文件
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

前往 [Releases](https://github.com/MasterLi186/LiteTerm/releases) 下载项目发布资产。下表是项目整体支持的格式，不代表本地脚本会生成全部格式：

| 平台 | 格式 |
|------|------|
| Linux | `.deb` `.rpm` `.AppImage` |
| macOS | `.dmg` |
| Windows | `.msi` `.exe` |

### 从源码构建

```bash
# 系统依赖 (Ubuntu/Debian)
sudo apt install -y build-essential pkg-config libudev-dev

# 获取源码
git clone https://github.com/MasterLi186/LiteTerm.git
cd LiteTerm

# 快速开发：从 native-prototype 构建 debug Native 版本（不运行自动测试）
cd native-prototype
cargo build --locked --bin liteterm-native
cd ..

# 运行（根目录入口；不会生成安装包）
./run-native.sh
```

也可以在仓库根目录用等价命令构建：`cargo build --manifest-path native-prototype/Cargo.toml --locked --bin liteterm-native`。如果 debug 二进制缺失或过期，`./run-native.sh` 会直接执行该 Native cargo 构建，不会生成 `.deb`。

### Debian/Ubuntu 本地 `.deb` 打包

在仓库根目录执行：

```bash
./native-prototype/build.sh
```

这是无参数的本地发布打包入口：只构建 `release` 锁定版本并生成 Debian `.deb`，不运行自动测试，也不会自动使用 `sudo` 安装。首次运行若缺少指定版本，会自动安装固定的 `cargo-packager 0.11.8`。产物目录为 `native-prototype/dist/installers/`，文件名按版本和 Debian 架构动态生成：

```text
native-prototype/dist/installers/LiteTerm-Native_<版本>_<架构>.deb
```

安装时将占位符替换为实际文件名，例如：

```bash
sudo apt install ./native-prototype/dist/installers/LiteTerm-Native_<版本>_<架构>.deb
```

脚本最后会打印实际产物路径和可直接复制的安装命令。

## 技术栈

| 层 | 技术 |
|---|------|
| 窗口/UI | winit + egui |
| 渲染 | wgpu + cosmic-text |
| 核心 | Rust |
| SSH/SFTP | ssh2 (libssh2) |
| 本地终端 | portable-pty |
| 串口 | serialport |
| 密钥存储 | OpenSSL AES-256-GCM 本地加密文件 |

## 截图

（待添加）

## 许可证

MIT
