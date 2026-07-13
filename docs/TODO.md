# LiteTerm 待办事项

> 按优先级排序,每个 item 标注来源和预估工作量。

## 🔴 P0 — 必须做(影响基本使用)

- [x] ~~SSH stty 注入时序问题~~ (已加 3 秒超时兜底) — 首次 resize 时注入的 `stty cols/rows echo` 可能延迟到用户已进入子 shell（如 adb shell）后才触发，导致 stty 命令被当作普通输入执行。需要给注入加超时兜底：连接后 5 秒内没收到 resize 就直接注入 stty echo 恢复回显。
  - 来源:实际使用（BMC → adb shell 场景）
  - 工作量:0.5 天
  - 涉及:`src-tauri/src/commands/ssh.rs` 的 `need_stty_echo` 逻辑

- [ ] **Claude Code `/new` 后终端显示异常** — 在 LiteTerm 中运行 Claude Code 后执行 `/new` 清空上下文，终端出现大片空白区域，渲染不正确。可能是 xterm.js 未正确处理 Claude Code 发送的清屏/光标重定位序列。
  - 来源:实际使用
  - 工作量:1 天
  - 涉及:`src/components/Terminal/TerminalPane.tsx`、xterm.js 配置

- [x] ~~clippy 20 个 warning 清零~~ (已清零 + CI 去掉 || true) — 代码质量门应该严格,目前 `|| true` 绕过
  - 来源:CI 审查
  - 工作量:1 小时
  - 涉及:`src-tauri/src/` 多个文件

- [ ] **main 分支保护规则** — 当前任何人可直接 push/force push 到 main
  - 来源:CI/CD 审查
  - 操作:GitHub → Settings → Branches → Add rule
  - 需要:Require PR + status checks pass + no force push

- [x] ~~Cargo.toml version 同步~~ (已同步为 0.8.17)
  - 来源:CI 审查
  - 工作量:5 分钟

## 🟡 P1 — 应该做(提升用户体验)

- [ ] **非活跃标签 scrollback 自动缩减** — 后台标签从 10000 行缩到 1000,切回来恢复
  - 来源:内存优化分析
  - 工作量:0.5 天
  - 效果:5 标签场景省 ~141MB

- [ ] **多会话同步输入** — 同时往多个终端发命令(运维批量操作)
  - 来源:竞品分析(SecureCRT/MobaXterm 有)
  - 工作量:1-2 天

- [ ] **HTTP API: 增量读取 UTF-8/ANSI 边界切分** — `GET /tabs/:id/read` 的增量游标可能切在多字节 UTF-8 字符或 ANSI 转义序列中间,导致该次返回的纯文本出现乱码或残留转义字节。需要在缓冲区或读取层面保留跨调用的部分序列状态,或至少在 API 文档中声明该限制。
  - 来源:代码审查
  - 工作量:0.5 天
  - 涉及:`src-tauri/src/commands/api_server.rs` 的 `read_tab` handler

- [x] ~~HTTP API: do_ssh_connect 阻塞 tokio 线程~~ (已改用 spawn_blocking) — `do_ssh_connect` 内部用 `std::sync::mpsc::Receiver::recv()`(阻塞调用)等待 SSH 握手完成,通过 axum handler 触发时会占用共享 tokio 运行时工作线程,多个并发 SSH 连接请求可能耗尽线程池。需要改用 `spawn_blocking` 或有界并发限制。
  - 来源:代码审查(预先存在的模式,非本次引入)
  - 工作量:0.5 天
  - 涉及:`src-tauri/src/commands/ssh.rs` 的 `do_ssh_connect`

- [x] ~~HTTP API: 测试计划与实现不一致~~ (文档已更新) — `docs/testing/http-api-test-plan.md` 承诺的 ANSI-01~04、AUTH-01~04 单元测试(应在 `api_server.rs` 的 `#[cfg(test)]`)未实现,需要补齐或更新测试计划文档。
  - 来源:代码审查
  - 工作量:2 小时

- [ ] **SSH 配置导入** — 读取 `~/.ssh/config` 自动生成连接书签
  - 来源:竞品分析(Tabby/iTerm2 有)
  - 工作量:0.5 天

- [ ] **智能命令补全增强** — OSC 133 命令捕获 → SQLite + frecency 排序 → 上下文加权
  - 来源:smart-command-completion 调研(已完成分阶段计划)
  - 约束:只在 bash 下生效,zsh/fish 自带补全
  - 工作量:3-5 天(分阶段)
  - 阶段0:OSC 133 shell integration
  - 阶段1:SQLite 历史 + frecency + 行内灰字补全
  - 阶段2:cwd + 退出码加权
  - 阶段3:fig/autocomplete 规格库 + AI

- [ ] **本地进程管理器** — 当前只有 SSH 远端进程管理器,本地终端点击进程无反应。已有 sysinfo crate 可获取本地进程,需要加前端入口。
  - 来源:Windows 测试发现
  - 工作量:1 天

- [ ] **进程管理器:杀进程** — 右键 → 发送信号(SIGTERM/SIGKILL)
  - 来源:对标 FinalShell
  - 工作量:2 小时

- [x] ~~系统设置面板~~ (已完成基础版:弹窗式字体/字号/配色选择)
- [x] ~~设置标签页(对标 Tabby)~~ (已完成：7 分类 + 配色预览列表 + Tauri 命令读写 settings.toml)

- [ ] **串口 ttyUSB 设备绑定** — 当前按 /dev/ttyUSB0 等路径打开,设备上下电后映射关系会变。需要按 USB VID:PID 或序列号绑定,而非路径。调研 GitHub 上完善的串口框架(如 serial-monitor-rust、espmonitor 等)。
  - 来源:实际使用问题
  - 工作量:1-2 天
  - 备注:Linux 可用 udev rules 固定映射,但应用层也应支持 VID:PID 选择

## 🟢 P2 — 可以做(锦上添花)

- [ ] **AI 命令生成 + 终端输出解释** — 对接 Ollama 或本地推理后端
  - 来源:158BitNet 集成设计(已有 spec + plan,代码在 feat/158bitnet-ai 分支)
  - 约束:158BitNet ARM only(x86 不可用),需换 Ollama 或等 x86 支持
  - 工作量:3-5 天(基础设施已写好,只需对接后端)

- [ ] **终端标签右键菜单** — 复制标签/关闭其他/关闭右侧(已部分实现)
  - 来源:对标 Chrome/VS Code
  - 工作量:2 小时

- [ ] **SFTP 批量操作** — 多选文件 → 批量下载/删除/移动
  - 来源:对标 FinalShell/FileZilla
  - 工作量:1-2 天

- [ ] **网络流量图表** — 侧边栏网络区域加实时折线图(当前只有数字)
  - 来源:竞品分析(WindTerm 有)
  - 工作量:0.5 天(recharts 已引入)

- [ ] **暗色/亮色整体主题切换** — 当前只有终端主题,整体 UI 无主题切换
  - 来源:竞品分析(Tabby/Warp 有)
  - 工作量:1 天

- [ ] **终端绿色背景色遮盖文字** — `ls` 输出的目录名用绿色背景(ANSI bg color)时,文字颜色被背景色盖住看不清。需要调整暗色默认主题的 green/brightGreen 配色,或者在绿色背景时自动用深色前景。
  - 来源:实际使用截图
  - 工作量:30 分钟
  - 涉及:`TerminalPane.tsx` 的 `TERMINAL_THEMES` 配色定义

## 🔧 技术债

- [ ] **重连遮罩代码去重** — App.tsx 有两份完全相同的遮罩 JSX(splitTrees/非 splitTrees 分支)
  - 工作量:15 分钟

- [ ] **parse_ps_aux 格式检测优化** — 当前用 TTY 字段长度启发式区分两种 ps 格式,脆弱
  - 来源:代码审查
  - 工作量:30 分钟

- [ ] **process.rs 旧代码清理** — `open_ssh_and_exec` + `get_process_list` + `get_process_detail` 已不被前端调用,可删除
  - 工作量:15 分钟

- [ ] **monitor.rs 旧 exec_local_command 的非 unix stub** — 提示"不支持此平台"但本地监控已用 sysinfo 替代,死代码
  - 工作量:10 分钟

- [ ] **前端 reconnecting 遮罩 ref focus 优化** — callback ref 每次渲染都抢焦点,应改为首次 mount 时 focus 一次
  - 来源:代码审查
  - 工作量:15 分钟

## ⚠️ 已知限制

- **Ctrl+滚轮缩放后已输出文本不折行** — xterm.js 5.x 的 reflow 只处理 soft-wrapped 行(长命令自动换行的),`ls` 等按列排版的输出不带 wrap 标记,resize 后不会重排。这是 xterm.js 的架构限制(对比 VTE/Terminator 有 pixel-level reflow)。新输入的命令会按新尺寸正确输出。

## ❌ 明确不做

| 功能 | 原因 |
|------|------|
| 移动端 | 无竞品做到,市场需求不明确 |
| RDP/VNC 远程桌面 | 工作量巨大,与 SSH 终端定位不同 |
| X11 转发 | 使用场景小,MobaXterm 专长 |
| 插件系统 | 架构改造大,投入产出比低 |
| Telnet/TFTP | 过时协议 |
| 内置文本编辑器 | 用户有 vim/nano |
| Windows WSL 集成 | Windows Terminal 更合适 |
| 自动重连(无用户确认) | 设计决策:对标 FinalShell,手动重连 |
