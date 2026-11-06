# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目定位与硬性约束

LiteTerm Native 是 LiteTerm 的唯一活跃实现：Rust + winit + wgpu + egui，无 WebView。包名 `liteterm-native`，版本以 `Cargo.toml` 为准。

- **Native-only**：只修改本目录（`native-prototype/`）内的产品代码。`../src/`、`../src-tauri/` 和根目录旧 GTK crate 仅为历史参考，禁止修改，除非用户明确点名并授权当次范围。详见 `../docs/NATIVE_ONLY.md` 与 `../AGENTS.md`（最高优先级）。
- **构建入口**：
  - 日常 debug 构建：本目录执行 `cargo build --locked --bin liteterm-native`。
  - 启动：仓库根目录执行 `./run-native.sh`（二进制缺失/过期时会自动执行同一条 cargo 构建；绝不生成安装包）。
  - Release 打包：只能在本目录执行无参数的 `./build.sh`。它会做 locked release 构建、调用固定版本 `cargo-packager 0.11.8` 并生成 `dist/installers/LiteTerm-Native_<版本>_<架构>.deb`；不要单独执行 `cargo build --release` 或 `cargo packager` 代替它。
  - 禁止使用 `npx tauri dev`，禁止跑 React/Tauri/GTK 构建路径（`npm run build`、仓库根目录的 `build.sh`、`run.sh`）。上述日常构建方式遵循 `../AGENTS.md` 的 Native 构建策略。
- **测试默认禁止**：除非用户明确要求“运行测试/自动化测试”，否则不要执行 `cargo test`、`cargo test --no-run`、测试脚本、`cargo clippy --all-targets` 等会编译或执行测试目标的命令；普通实现工作也不要顺手添加测试代码。Native UI 变更的默认验证方式是：构建 → `./run-native.sh` → 实际启动窗口并交互检查。
- **Tag 禁止**：未经用户当次明确授权，禁止创建/推送 `native-v*` tag。tag 会触发 CI 三/四平台构建与 Release；每次授权仅限一次。
- **提交**：中文 Conventional Commits（`feat:`/`fix:`/`docs:`/`refactor:`/`perf:` 等）；提交前对改动的 Rust 执行 `cargo fmt --all`（CI 用 `cargo fmt --all -- --check` 校验）。

Linux 构建依赖：`pkg-config libudev-dev libssl-dev`（CI 同样安装这些）。

## 常用命令

```bash
# 本目录内
cargo build --locked --bin liteterm-native   # 日常 debug 构建
cargo fmt --all                              # 格式化
cargo fmt --all -- --check                   # CI 格式检查
cargo clippy --locked --bin liteterm-native  # CI 使用的 lint（不含测试目标）
./build.sh                                   # Release + .deb 打包（仅此入口）

# 仓库根目录
./run-native.sh                              # 启动 Native 应用
```

仅在用户明确授权测试时：

```bash
cargo test                                   # lib + bin 全部单元测试
cargo test --bin liteterm-native             # 只跑二进制 crate 内测试
cargo test --bin liteterm-native <过滤名>     # 单个测试
cargo test --lib                             # 只跑 liteterm_native_api 库测试
./scripts/test_http_api.sh                   # HTTP API 冒烟（需应用已启动）
```

主题文件是生成的，不要手改 `src/themes_generated.rs`：

```bash
node scripts/generate-themes.mjs   # 从 ../src/themes.ts 生成 src/themes_generated.rs（191 套主题）
```

版本号由发布流程用 `scripts/bump_version.py patch|minor|major` 同步 `Cargo.toml` 与 `Cargo.lock`；本地不要自行改版本或打 tag。

## 架构总览

### 线程模型：主线程独占 UI，后台线程只发事件

- 主线程运行 winit `EventLoop`，`App`（`src/app.rs`）持有全部 UI 状态。**任何后台线程都不能直接修改 `App`**。
- 后台 → 主线程：`EventLoopProxy<UserEvent>`；`UserEvent` 定义在 `src/main.rs`（SSH 就绪、串口、补全历史、ZMODEM、SFTP、监控、进程详情、HTTP API、Redraw 等）。
- 主线程 → 后台：`std::sync::mpsc` 命令通道 / `AtomicBool` 停止标志 / 关闭信号。
- 典型后台线程：每个终端 pane 的 reader 线程（`terminal::read_loop_with_zmodem`，内部还有可取消的 reader pump）、SSH 连接/IO 线程、每个 SSH pane 的 SFTP worker、按 `MonitorKey` 共享的远程监控线程、本地监控采集线程、隧道监听线程、HTTP API 的 tokio 线程、ADB 历史写入线程。
- **陈旧事件防护**是贯穿全局的模式：后台事件都带 generation/session 标识（`CompletionSessionKey`、ZMODEM `TransferIdentity`、monitor generation、request_id），主线程应用事件前必须校验其仍对应当前 tab/pane，否则丢弃。

### `App` 与模块划分

`src/app.rs` 中的 `App` 是中心状态，按职责拆分到 `src/app/`：

- `runtime.rs`：创建终端/启动 read loop、ZMODEM 控制槽、补全填充派发。
- `events.rs` / `user_events.rs` / `window_events.rs`：winit 生命周期、后台事件与窗口事件分发。
- `render.rs` / `render_actions.rs` / `render_overlays.rs`：每帧 egui + 终端渲染与交互结果。
- `layout.rs`：窗口区域 → pane 矩形 → 终端 cols/rows。
- `tabs.rs` / `session.rs`：标签创建/关闭、会话恢复与重连。
- `pointer_motion.rs` / `terminal_interaction.rs`：鼠标选择、拖拽、终端交互。
- `api_ops.rs`：HTTP API 主线程操作映射。
- `ui_state.rs`：模态框/IME 归属/焦点仲裁。

领域模块（`src/*.rs`）：

- 终端：`terminal.rs`（`TerminalState`，封装 `alacritty_terminal::Term` + `vte` parser + pty/SSH 读写 + 选择/搜索/IME/ZMODEM）、`terminal/read_loop.rs`、`terminal_search.rs`、`terminal_links.rs`、`terminal_log.rs`。
- 连接：`ssh.rs`、`sftp.rs`、`serial.rs`、`tunnel.rs`、`connections.rs`、`keyring.rs`。
- 监控：`monitor.rs`（本地 sysinfo + 通用数据结构）、`remote_monitor.rs`（远程 `/proc` 快照）、`process_manager.rs`、`network_detail.rs`。
- 补全：`bash_integration.rs`、`smart_completion.rs`、`completion_popup.rs`、`adb_history.rs`、`app_completion.rs`。
- UI：`sidebar/`、`tab_bar.rs`、`command_bar/`、`settings_panel/`、`file_browser/`、`new_tab_selector.rs`、`tunnel_manager.rs`、`batch_command.rs`、`recording.rs`。
- 渲染：`renderer.rs` + `renderer/core.rs`、`atlas.rs`、`font_support.rs`。
- 其他：`themes.rs`（含生成的 `themes_generated.rs`）、`i18n.rs`、`ime.rs`、`x11_ime_reattach.rs`、`workspace_session.rs`、`settings.rs`、`shortcuts.rs`、`split.rs`、`tab_manager/`、`zmodem/`、`api.rs`（同时是 `liteterm_native_api` 库目标）。

`src/api.rs` 被声明为 `[lib] liteterm_native_api`，刻意不依赖 winit：HTTP 层通过 `api::Bridge` 把请求转成 `UserEvent::Api`，由主线程 `App::handle_api_call` 执行。这样 API 层可独立编译/测试。

### 渲染：wgpu 终端层 + egui 界面层

- 终端文本由自研 wgpu 渲染器绘制：`cosmic-text` 做字形 shaping/布局，`GlyphAtlas` 生成字形纹理图集，instanced quad 绘制 cell 背景与字形。
- 活动终端标签的**所有 pane** 都会按各自矩形渲染；每个 pane 渲染前 `term.resize(cols, rows)` 同步尺寸。
- egui（`egui_winit` + `egui_wgpu`）渲染标签栏、侧边栏、对话框、菜单、补全弹窗等，最后叠加在终端之上。
- 布局：`app/layout.rs` 用窗口尺寸减去侧边栏、标签栏、底部命令栏、文件管理器高度，得到终端区域；`split.rs` 的 `PaneNode`/`PaneTree` 维护分屏树，`tab_manager/model.rs` 的 `TerminalLayout` 维护每个标签的 pane 集合与活动 pane。
- 渲染由事件驱动，`UserEvent::Redraw` 节流，另有 530ms 光标闪烁线程；pane 级实例缓存会复用内容未变化的 GPU buffer（cell 级 dirty tracking 仍是 TODO）。

### 终端、补全与 IME

- `TerminalState` 统一本地 PTY、SSH channel、串口三类后端；读线程把字节交给 `vte::ansi::Processor` 解析进 `alacritty_terminal::Term`。
- 补全不是单纯的 UI 功能：`bash_integration.rs` 会给 bash 注入 rc/OSC 标记，用于识别提示符、历史文件路径和提交命令；`smart_completion.rs` 维护每个 pane 的命令行状态与候选，历史从本地文件或远程 SFTP worker 读取；ADB 场景按设备序列号保存独立历史。
- `ime.rs` 是纯状态机，负责在“终端 / egui”之间仲裁输入归属并去重预编辑/提交；`x11_ime_reattach.rs` 处理 X11 下 winit 输入法在按键时重连的兼容问题。

### SSH / SFTP / 监控 / 隧道

- `ssh2::Session` 是 `!Send`，因此**每个子系统在自己的线程上创建独立 TCP + SSH 连接**：终端每个 pane 一条、远端监控按 `MonitorKey`（`user@host:port`）共享一条、SFTP 每个 SSH pane 一条。
- `connections.toml` 与 API 请求里保留了 `proxy_jump` 字段，但当前 Native 终端连接没有实际实现跳板机（API 明确返回“暂不受支持”）；不要假设它可用。
- `remote_monitor.rs` 通过一条组合 shell 命令批量读取远端 `/proc`、`df`、`ps` 等（见 `REMOTE_SNAPSHOT_COMMAND`），并对输出做严格的字节上限与解析。
- `tunnel.rs` 本地端口转发只监听 `127.0.0.1`，每个隧道独立线程 + 停止信号。
- `serial.rs` 使用 `serialport` crate，独立 reader/writer 线程。

### ZMODEM

`zmodem/` 包含帧编解码、检测、发送/接收状态机、runtime 和 UI 状态。runtime 嵌在终端 reader pump 中，使阻塞读取期间仍能轮询取消/超时/关闭；支持本地终端与直连 SSH，串口禁用。下载目录来自 `settings.zmodem.download_dir`，所有传输事件带 `TransferIdentity` 以防陈旧 UI 操作。

### HTTP API

- `src/api/server.rs` 用 axum 在后台 tokio 线程启动，仅监听 IPv4 loopback，默认端口 19526。
- 启动时在配置目录写入 `native-api-token`（32 字节随机数的 64 位 hex）和 `native-api-port`（JSON：port/pid/instance）；客户端必须读发现文件，不要混用 legacy token。
- 请求经 `api::Bridge` → `UserEvent::Api` 进入主线程执行；终端输出通过 `OutputRegistry` 以游标增量读取。
- 限额：请求体 64 KiB、请求体读取 5 秒、主线程调用 5 秒、最多 32 并发。接口文档在 `../docs/api/http-api.md`。

## 配置与持久化（`~/.config/guishell/`）

| 文件 | 内容 |
|---|---|
| `native-settings.toml` | 终端字体/字号/配色、侧边栏、传输、SSH keepalive、ZMODEM、快捷键（刻意用 `native-` 前缀，避免与旧 Tauri `settings.toml` 混淆） |
| `connections.toml` | 分组 + SSH 连接书签（TOML） |
| `credentials.enc` | 密码 AES-256-GCM 加密文件，PBKDF2（hostname+username）派生密钥，绑定机器 |
| `native-session.toml` | 上次标签页会话（仅本地/SSH/串口，最多 64 个） |
| `native_cmd_history.json` / `native_cmd_favorites.json` / `native_quick_commands.json` | 底部命令栏历史/收藏/快捷命令 |
| `native-api-token` / `native-api-port` | HTTP API 发现文件 |
| `crash.log`、`crash.log.old` | panic/SIGSEGV/SIGBUS/SIGABRT 崩溃记录 |

## 国际化与主题

- 源码中的用户可见文案以中文为准；`i18n::tr()` 根据 `LC_ALL` / `LC_MESSAGES` / `LANG` 是否以 `zh` 开头返回中文或英文。新增文案必须同时加入 `i18n.rs` 的映射表，否则英文环境会回退为中文。
- 主题由 `scripts/generate-themes.mjs` 从历史文件 `../src/themes.ts` 解析生成 `src/themes_generated.rs`；`themes.rs` 只做查询封装。生成文件禁止手改，改主题应改生成脚本/来源并重新生成。

## 代码组织约定

- 手写 Rust 模块控制在 ~1000 行以内；大模块已按职责拆成子模块，测试放到 `<module>/tests.rs` 与 `part_NN.rs`。新增测试时沿用该模式。
- 中文注释、中文错误/日志文案与现有代码保持一致；标识符用英文 `snake_case`/`PascalCase`。
- 耗时或阻塞操作（SSH/SFTP/文件 IO/进程详情/历史读取）放到后台线程，主线程只做状态应用与绘制。

## 测试组织（仅在获授权时运行）

- 单元测试内联在 `#[cfg(test)]` 模块中；大块测试位于 `src/<module>/tests.rs` 及 `part_NN.rs`。
- 应用级/布局级测试在 `src/main_tests/`，由 `main.rs` 的 `#[cfg(test)] #[path = ...]` 挂载（`layout_tests`、`api_main_tests`、`zmodem_main_tests`）。
- 库目标测试在 `src/api.rs` 的 tests 模块。
- 仓库没有独立 `tests/` 集成测试目录；不要按旧 GTK 仓库的习惯去找。

## CI 与发布

- `.github/workflows/native-build.yml`：由 `native-v*` tag 或手动触发；quality job 运行 `cargo fmt --all -- --check` 和 `cargo clippy --locked --bin liteterm-native`；build job 在 Linux（deb）、Windows（nsis）、macOS Intel / Apple Silicon（dmg）上执行 `cargo build --release --locked` 并用 `cargo-packager 0.11.8` 打包。
- `.github/workflows/native-release.yml`：手动 dispatch，按 patch/minor/major 调 `scripts/bump_version.py`，提交版本、创建并推送 `native-vX.Y.Z` tag，从而触发构建。本地不要模拟该流程。
- `build.sh` 会校验 deb 的 Package/Version/Architecture/Depends，以及包内二进制、desktop 入口和 5 个尺寸图标；输出目录中只允许存在一个 `.deb`。

## 相关文档

- `../AGENTS.md` — Native-only 开发策略、测试授权规则（最高优先级）。
- `../docs/NATIVE_ONLY.md` — 修改边界与构建入口。
- `../docs/api/http-api.md` — HTTP API v1 接口与认证。
- `../docs/design/http-api-feature.md` — API 设计背景。
- `TODO.md` — 已实现/未实现功能与技术债清单。
- 注意：`../docs/design/architecture.md` 描述的是历史 React/Tauri 架构，**不适用于本目录**。
