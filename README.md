# LiteTerm

轻量级原生 Linux SSH 客户端 — FinalShell 开源替代品。

基于 **Tauri 2 + React + TypeScript + xterm.js** 构建，内存占用仅 ~30MB，安装包仅 6MB。

## 功能

- **SSH 连接管理** — 分组保存、密码存入系统密钥环、一键连接
- **本地终端** — 内置本地 Shell，支持多标签页
- **终端分屏** — 水平/垂直分屏，可嵌套，可拖拽调整比例
- **系统监控** — 实时 CPU/内存/交换/磁盘/网络/进程监控（本地+远程）
- **进程管理器** — 全进程列表，可排序，点击查看详情（命令行、环境变量、工作目录）
- **SFTP 文件管理器** — FileZilla 风格双栏布局，支持下载/上传/删除/重命名
- **ZMODEM 传输** — 支持通过堡垒机/跳板机多跳 SSH 传输文件（sz/rz）
- **会话持久化** — 关闭后自动记忆并恢复 SSH 连接
- **命令收藏** — 保存常用命令，一键发送到终端执行
- **全中文界面** — 所有菜单、对话框、提示信息均为中文

## 安装

### 从源码构建

```bash
# 依赖
sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-0 libssl-dev

# 构建
git clone git@github.com:MasterLi186/LiteTerm.git
cd LiteTerm
npm install
./build.sh

# 运行
./run.sh
```

### 安装包

```bash
# 构建 deb/rpm/AppImage
npx tauri build

# 安装 deb (Ubuntu/Debian)
sudo dpkg -i src-tauri/target/release/bundle/deb/GuiShell_0.1.0_amd64.deb

# 或直接运行 AppImage (无需安装)
chmod +x src-tauri/target/release/bundle/appimage/GuiShell_0.1.0_amd64.AppImage
./GuiShell_0.1.0_amd64.AppImage
```

## 技术栈

| 层 | 技术 |
|---|------|
| 前端 | React 18 + TypeScript + Tailwind CSS |
| 终端 | xterm.js + FitAddon + zmodem.js |
| 后端 | Rust + Tauri 2 |
| SSH/SFTP | ssh2 (libssh2) |
| 本地终端 | portable-pty |
| 密钥存储 | secret-service (GNOME Keyring) |

## 截图

（待添加）

## 许可证

MIT
