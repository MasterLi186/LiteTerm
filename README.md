# LiteTerm Native

轻量级跨平台终端客户端：Rust + winit + wgpu + egui，无 WebView。

功能包括本地终端、SSH、SFTP 文件管理、串口、系统监控、进程/网络详情、分屏、ZMODEM、录屏回放和 HTTP API。

## 构建与运行

Linux 构建依赖：`pkg-config libudev-dev libssl-dev`。

```bash
# debug 构建
cargo build --locked --bin liteterm-native

# 启动（二进制缺失/过期时会自动构建）
./run-native.sh

# Release + .deb 打包
./build.sh
```

Release 产物：`dist/installers/LiteTerm-Native_<版本>_<架构>.deb`。

## 配置

配置目录：`~/.config/guishell/`

- `native-settings.toml`：终端字体、主题、侧边栏、传输、ZMODEM、快捷键
- `connections.toml`：SSH 连接书签
- `credentials.enc`：AES-256-GCM 加密的密码文件
- `native-session.toml`：标签页会话
- `native-api-token` / `native-api-port`：HTTP API 发现文件

## 文档

- `AGENTS.md` — 开发策略与验证规则
- `docs/NATIVE_ONLY.md` — 单实现约束与构建入口
- `docs/api/http-api.md` — HTTP API v1
- `TODO.md` — 待办与技术债

## 历史版本

旧的 React/Tauri/GTK 实现保存在 `legacy-tauri` 分支和旧 tag 中，不再参与当前构建。
