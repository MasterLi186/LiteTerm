# LiteTerm Native Prototype — TODO

## ✅ 已实现

### 终端核心
- [x] 本地 shell 终端 (alacritty_terminal + portable-pty)
- [x] SSH 终端连接 (libssh2，密钥/密码/agent 认证)
- [x] 多标签系统 (独立 TerminalState)
- [x] GPU instanced rendering (wgpu + 字形纹理图集)
- [x] ANSI 颜色 (16+256+TrueColor)
- [x] 粗体/斜体/下划线/删除线/DIM/INVERSE/HIDDEN
- [x] CJK 宽字符 (WIDE_CHAR_SPACER)
- [x] 鼠标选择/双击选词/三击选行
- [x] 复制粘贴 (Ctrl+Shift+C/V + 中键 + 右键菜单)
- [x] 鼠标上报 (SGR 模式，vim/htop/less)
- [x] 光标闪烁 (530ms)
- [x] Scrollback 滚动 + 按键回底部
- [x] resize 同步 (本地 PTY + SSH window-change)
- [x] CSI 6n/18t 查询回复 (resize 命令正常工作)
- [x] Tab 补全 (不被 egui 拦截)
- [x] Ctrl+A~Z + F1-F12 键映射
- [x] AdventureTime 配色方案
- [x] Native 默认字体字号 (Ubuntu Mono 22px，独立设置可覆盖)
- [x] 终端文本日志（系统另存为对话框，默认 `.txt`，过滤 ANSI 控制序列）

### 侧边栏
- [x] 连接管理 (connections.toml 读写)
- [x] 分组折叠/展开
- [x] 单击连接 SSH (keyring 自动读密码)
- [x] keyring 密码存取 (AES-256-GCM 加密)
- [x] 密码输入弹窗 (认证失败时)
- [x] 新建连接对话框
- [x] SSH 密钥管理
- [x] 导入/导出配置（系统原生文件对话框）
- [x] 连接右键菜单 (连接/新建会话/编辑属性/删除)
- [x] 系统监控面板 (sysinfo: CPU/内存/交换/进程/网络/磁盘)
- [x] 侧边栏可滚动
- [x] 网卡下拉选择 (单个网卡 + 速率)
- [x] 网络折线图 (mini chart，2s 采样，切换网卡清空)
- [x] 监控面板 UI 对齐 guishell (卡片/进度条/进程 tab 排序)

### 标签栏
- [x] 标签切换/关闭/新建 (+按钮)
- [x] 右键菜单 (重命名/复制标签页/重新连接/关闭/关闭其他)
- [x] Ctrl+Shift+T 新建 / Ctrl+Shift+W 关闭
- [x] Ctrl+Tab / Ctrl+Shift+Tab 切换
- [x] Ctrl+1~9 快速切换

### 底部命令栏
- [x] 快捷命令按钮 (df -h / free -h / top / ss / ls 等)
- [x] 命令栏输入框 + 回车执行
- [x] 命令栏历史记录（持久化 `~/.config/guishell/native_cmd_history.json`）
- [x] 命令栏收藏（持久化 `native_cmd_favorites.json`，右键可加）
- [x] 添加/编辑/删除快捷命令（`native_quick_commands.json`）
- [x] 命令栏起始位置对齐侧边栏右侧（SidePanel 先于 BottomPanel）

### 基础设施
- [x] crash handler (panic + SIGSEGV/SIGBUS/SIGABRT → crash.log)
- [x] 渲染节流 60fps
- [x] 无 WebView 内存泄漏
- [x] DNS 域名解析 (SSH 连接)
- [x] SFTP 文件管理器 (FileZilla 风格双栏，按标签隔离)
- [x] SSH 远端系统监控 (按 user@host:port 共享采集与独立展示)

## ❌ 未实现 — P0 (日常使用必需)

- [x] **设置面板** — 字体/字号/配色/快捷键 (guishell: Settings.tsx + SettingsTab.tsx)
- [x] **配色方案切换** — 190+ Tabby 主题 (guishell: themes.ts)
- [x] **终端搜索** — Ctrl+Shift+F (guishell: SearchAddon)
- [x] **IME 中文输入法** — winit IME 预编辑/提交状态机与输入归属隔离
- [x] **新标签选择器** — Shell/SSH 选择弹窗（串口入口禁用，P1 实现）

## ✅ 已实现 — P1 (重要功能)

- [x] **进程管理器标签页** — 远端进程列表+详情 (guishell: ProcessTable.tsx)
- [x] **网络详情标签页** — 网卡连接列表 (guishell: NetworkDetail.tsx)
- [x] **SSH 端口转发/隧道** — 本地隧道管理 (guishell: TunnelManager.tsx + tunnel.rs)
- [x] **批量命令** — 同时发命令到多个终端 (guishell: BatchCommand.tsx)
- [x] **串口终端** — serialport 设备连接 (guishell: serial.rs)
- [x] **链接点击打开** — URL/文件路径 (guishell: WebLinksAddon + 自定义 linkProvider)
- [x] **标签拖拽排序**

## ❌ 未实现 — P2 (锦上添花)

- [x] **终端录屏回放** — asciicast v2 格式（播放/暂停、重播、倍速、进度跳转）
- [x] **ZMODEM 文件传输** — sz/rz (guishell: zmodem.rs)
- [x] **分屏** — 水平/垂直分割 (guishell: SplitContainer.tsx)
- [x] **标签重命名对话框**
- [x] **HTTP API 自动化接口** — 127.0.0.1 (guishell: api_server.rs)
- [x] **自定义快捷键** — 绑定配置 (guishell: ShortcutSettings.tsx)
- [ ] **RDP 远程桌面** — IronRDP (未来方向)

## 技术债

- [x] 大型 Rust 模块按职责拆分（生成主题文件除外，手写源码均小于 1000 行）
- [x] pane 级 dirty cache — 内容未变化时复用 GPU instance buffer
- [ ] cell 级 dirty tracking — 只上传 alacritty 标记为变化的 cell 区间
- [x] 清理 debug `eprintln!` 日志（仅保留 crash 直出与测试输出）
- [ ] 清理 cargo warnings
- [ ] 提交 git (用户要求验证通过后再提交)
