# Native 启动合成按键过滤设计

## 背景与根因

Native 客户端在 X11 下偶发于启动首屏显示类似：

```text
n d lfl@host:~$ n d
```

窗口出现后用户并未输入，且每次字符不同。winit 在 X11 窗口获得焦点时，会为当时仍处于按下状态的键生成 `is_synthetic = true` 的 `KeyboardInput`。现有代码忽略该标志，仅过滤应用创建后 500ms 内的按键。GPU、字体和 atlas 初始化较慢时，合成事件会在时间窗结束后到达并写入 PTY。

PTY 先在提示符前回显字符，Bash/readline 随后输出提示符并重绘待编辑内容，因此同一字符串出现在提示符前后。

## 修复方案

在 `window_event` 的键盘事件入口显式读取 `is_synthetic`，并将事件分为三类：

- synthetic `Pressed` 在传给 egui 前直接丢弃，永不进入快捷键、Tab 或终端写入逻辑。
- synthetic `Released` 仅传给 egui，用于清理窗口失焦时的按键状态，不进入应用快捷键或终端。
- 真实 `Pressed` 进入现有应用和终端处理，真实 `Released` 仅传给 egui。
- 删除启动后 500ms 忽略所有键盘事件的时间窗。
- 保留真实按键的 repeat 行为、终端特殊键、控制字符和应用快捷键。
- `startup_time` 继续供现有鼠标诊断时间戳使用，不删除该字段。

不在 PTY 输出、终端网格、parser 或 renderer 中去重字符串，避免破坏合法终端内容。

## 测试与验证

提取返回 `Drop`、`EguiOnly` 或 `App` 的纯路由决策函数，并覆盖：

- synthetic `Pressed` 路由到 `Drop`。
- synthetic `Released` 路由到 `EguiOnly`。
- 真实 `Pressed` 路由到 `App`。
- 真实 `Released` 路由到 `EguiOnly`。
- 真实 repeat 沿用 `Pressed` 路由，仍进入 `App`。

运行独立 Native 构建与全部测试。现场验证使用 X11 自动化：启动前保持一个可打印键按下，聚焦 Native 后释放。修复后首个提示符前后均不得出现该字符，随后真实键盘输入仍应正常工作。
