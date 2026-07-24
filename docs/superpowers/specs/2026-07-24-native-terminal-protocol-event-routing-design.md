# Native 终端协议事件回写设计

## 背景与根因

LiteTerm Native 使用 `alacritty_terminal 0.25.1` 解析终端输出，但当前 `Listener` 丢弃全部终端事件。为了支持 `CSI 6n` 和 `CSI 18t`，`terminal.rs` 又按单次 PTY 读取内容手工扫描控制序列。

真实 Fish 字节流证明，按 Tab 补全时 Fish 输出光标右移 `ESC[18C`。现有代码只检查前缀 `ESC[18`，因此误判为字符区域尺寸查询 `ESC[18t`，错误回写 `ESC[8;48;180t`。Fish 随后把该响应作为普通输入插入命令行。

## 方案

删除手写 CSI 查询识别，接入 Alacritty 自带的 `EventListener` 和 `Event::PtyWrite`：

1. `TerminalState::init_term` 为每个终端创建事件通道。
2. `Listener::send_event` 只负责将 Alacritty 事件放入该通道，不直接获取终端锁或写 PTY。
3. `read_loop` 将每次读取的原始字节完整交给同一个 Alacritty `Processor`。解析器自身负责完整 CSI 识别和跨读取分片状态。
4. `parser.advance` 返回后，`read_loop` 排空事件通道；对 `PtyWrite` 使用当前终端的 `WriterKind` 写回本地 PTY或 SSH。
5. 删除 `remove_csi_sequence`、`has_dsr`、`has_18t` 及其人工响应。

这种异步事件通道避免 `send_event` 在解析过程中重入 `TerminalState` 锁。现有 `TERM=xterm-256color`、Tab 键转发、PTY resize 和 SSH resize 保持不变。

## 事件边界

本次实现处理 Alacritty 已格式化好的 `PtyWrite`，覆盖 `CSI 6n`、`CSI 18t` 以及同类标准协议回复。与当前功能无关的标题、剪贴板、颜色和像素尺寸请求仍按现状忽略，避免扩大范围；后续可通过同一事件通道独立接入。

通道断开或 PTY 写入失败不得导致渲染线程崩溃。当前 `write_input` 的容错语义保持不变。

## 测试与验收

- `ESC[18C` 和 `ESC[18~` 原样交给解析器，不产生 PTY 回复。
- 完整 `ESC[18t` 只产生一次正确的 `ESC[8;<rows>;<cols>t`。
- 将 `ESC[18t` 拆成多次 `parser.advance`，仍只回复一次。
- `ESC[6n` 继续根据当前光标位置回复。
- 本地 `Direct` writer 和 SSH `Channel` writer 均能收到相同的 `PtyWrite` 内容。
- 全量 Native 测试、Clippy、独立构建通过；实机 Fish 中连续 Tab 补全不再出现尺寸响应文本。

## 非目标

本次不更换终端类型、不降级到 VT100、不修改 Fish 配置，也不实现 Alacritty 的剪贴板、标题或颜色查询事件。
