# Native Bash 补全可靠性与新标签选择器设计

## 目标与范围

本次修改只作用于 `feat/memory-optimize` 的 `native-prototype/`。目标是让本地 Bash 与 SSH Bash 的历史补全具有一致、可验证的输入状态，并将新标签选择器改为忠于 `main` 分支的紧凑布局。旧 Tauri/guishell、Fish、Zsh 和串口后端不在范围内。

## 问题根因

当前补全按逻辑候选是否非空接管方向键和回车，但弹窗渲染还依赖终端锁、渲染器、模态框和光标可见性。这会产生“弹窗不可见但候选仍可接受”的隐形交互。窗口真实缩放还会清除终端提示符锚点，直到 Bash 再次输出提示符标记前都无法提取当前输入。

Space 没有绑定到接受候选。用户看到的“按空格后补全”来自异步填充竞态：先前的回车已启动候选文件写入，回调稍后替换 `READLINE_LINE`，随后 Space 才被 Bash 接收。

## 统一的弹窗交互状态

每帧渲染前生成 `CompletionPopupSnapshot`，其中包含标签页 generation、候选、选择项、锚点矩形和 `interactive`。只有候选非空、没有阻塞模态框、终端可渲染且光标锚点位于可视区域时，`interactive` 才为真。

按键路由只读取这份快照：

- `↑` / `↓`：仅在 `interactive` 时切换候选，否则原样交给 Bash；
- `Tab`：仅在 `interactive` 且有选择项时开始填充；没有可见弹窗时原样交给 Bash；
- `Enter`：不接受候选，直接提交当前已输入命令；
- `Escape`：仅关闭可见候选；
- Space、普通文本、粘贴和其他未定义按键永远不能接受候选。

Tab 触发的填充进入显式 `PendingFill` 状态。状态记录 tab ID、generation、候选和请求 ID；完成前不再接受第二个候选，且短暂拦截 Enter，避免异步写入完成前误执行旧前缀。任何普通编辑输入都会取消该请求，迟到回调因请求 ID 或 generation 不匹配而丢弃。写入成功后仅触发私有 Readline widget 替换当前行，不发送 `CR/LF`，因此仍需用户按回车执行。

## Bash Readline 输入快照协议

沿用已有随机 token、generation 和私有 OSC 777 协议，增加输入快照请求/响应，不在应用侧猜测 Bash 编辑状态。

会话级 Bash 集成新增随机专用 Readline widget。Native 在真实 resize、提示符锚点丢失或当前输入不可证明时发送快照请求序列；widget 读取 `READLINE_LINE` 与 `READLINE_POINT`，使用 Base64URL 编码后发回带 token 和 generation 的私有 OSC 帧。帧设置严格长度上限，拒绝控制字符、非法编码、旧 generation 和伪造 token。

解析成功后，终端状态保存独立的 Readline 输入快照并重新计算候选。之后的普通 Bash 回显继续更新现有网格锚点；新提示符、执行回车、切换 shell、重连或 alternate screen 都会使旧快照失效。请求设置短超时和单请求门控，失败只禁用当前候选，不阻塞终端。

本地与 SSH 使用同一状态机。SSH 通过现有会话临时 RC 和安全随机绑定安装 widget；未启用 Bash 集成、部署失败或远端不支持编码工具时保持普通终端行为。

## 历史加载与可观测性

历史读取仍使用当前的大小、条数和 generation 限制，但不再静默吞掉状态。每个标签记录 `disabled`、`loading`、`ready` 或 `error`，以及安全的中文原因，例如“当前 Shell 不是 Bash”“远端历史文件不可读”。诊断不得显示密码、私钥、token 或完整候选内容。

历史为空不是错误；只有集成或读取失败才记录警告。该状态用于调试日志和测试，不在正常终端界面常驻提示。

## 候选匹配与数量

候选行为忠于 `main`：只接受从当前非空输入开头匹配的历史命令，即 `command.starts_with(input)`。与输入完全相同的命令不作为候选；历史命令中间出现输入文本时不得匹配。例如输入 `fish` 不能匹配 `strace ... liteterm-fish-*`。

候选保持历史记录的新近优先顺序，去重后最多返回并显示 5 项。排序不增加模糊匹配、子串匹配或隐藏的第二候选页，方向键只在这 5 项中循环。

## 新标签选择器

选择器复刻 `main:src/components/NewTabSelector.tsx` 的视觉层级，并以 egui 原生控件实现：

- 居中宽度 520px，最大高度为视口 80%，窄窗口保留 16px 边距；
- 背景 `#161b22`，边框 `#30363d`，8px 圆角和深色阴影，遮罩为 60% 黑色；
- 标题栏使用 14px 半粗标题与灰色关闭按钮，内容区 16px 内边距、分区间距 20px；
- Shell 使用可换行紧凑胶囊按钮，只显示路径 basename；
- SSH 按保存的 group 分组，显示颜色圆点、连接名称及 `host:port`，悬停整行高亮；
- 底部提供青色 `+ 新建 SSH 连接` 文本操作，复用现有连接编辑弹窗；
- 串口保持弱化禁用状态并标注 P1，不实现串口 I/O。

加号和新标签快捷键打开同一个选择器。Escape、关闭按钮或遮罩点击只关闭弹窗；创建动作被现有会话工厂接受后才关闭。

## 测试与验收

先补测试，再改实现。自动化覆盖：

- 弹窗不可渲染或光标离屏时不捕获方向键、Tab 与回车；
- 仅普通 Tab 在可见弹窗下创建填充请求；无弹窗 Tab 原样交给 Bash；
- Enter、Space、文本和粘贴永不创建填充请求；
- 编辑发生在异步回调前时取消请求，迟到回调不能改行；
- resize 后快照请求恢复本地与 SSH Bash 候选；
- 非法、超长、旧 generation 或错误 token 的快照被丢弃；
- Tab 只填入候选且不执行，随后回车才执行；直接回车执行当前前缀而不接受候选；
- `fish` 不匹配历史中间包含 `fish` 的调试命令，严格前缀候选最多 5 项；
- Shell basename、SSH 分组、创建 SSH 动作和禁用串口映射正确。

手工验收使用 `native-prototype/build.sh` 构建并运行独立 Native 二进制：分别验证本地 Bash、SSH Bash、缩放窗口、长命令换行、Readline 光标编辑、快速回车后按 Space，以及加号选择器在普通和窄窗口下的布局。不得修改或覆盖旧 `run.sh`、旧 guishell 二进制和 `main` 分支。
