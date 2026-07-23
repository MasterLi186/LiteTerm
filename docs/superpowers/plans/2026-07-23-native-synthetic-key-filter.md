# Native 合成按键过滤实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Native 键盘事件入口丢弃 X11/winit 合成按键，消除启动时提示符前后出现相同随机字符串的问题。

**Architecture:** 提取纯键盘路由函数，以 `ElementState` 和 `is_synthetic` 返回 `Drop`、`EguiOnly` 或 `App`。synthetic `Pressed` 在 egui 前丢弃；synthetic 与真实 `Released` 只更新 egui 状态；真实 `Pressed` 才进入 Native 快捷键和终端处理。删除原有 500ms 时间门限，保留真实按键 repeat 和所有现有快捷键映射。

**Tech Stack:** Rust、winit 0.30、X11、现有 Native 单元测试与 `native-prototype/build.sh`

---

## 文件范围

- Modify/Test: `native-prototype/src/main.rs`
- Verify: `native-prototype/build.sh`
- Verify: `run-native.sh`

`native-prototype/src/main.rs` 已包含工作区既有修改，实施代码保持不暂存，避免把其他 Native 开发内容意外提交。

### Task 1: 键盘事件源头过滤

**Files:**
- Modify/Test: `native-prototype/src/main.rs`

- [ ] **Step 1: 写入失败测试**

在 `layout_tests` 中导入 `keyboard_input_route`、`KeyboardInputRoute` 和 `ElementState`，增加：

```rust
#[test]
fn keyboard_input_route_separates_synthetic_and_release_events() {
    assert_eq!(
        keyboard_input_route(ElementState::Pressed, true),
        KeyboardInputRoute::Drop,
    );
    assert_eq!(
        keyboard_input_route(ElementState::Released, true),
        KeyboardInputRoute::EguiOnly,
    );
    assert_eq!(
        keyboard_input_route(ElementState::Pressed, false),
        KeyboardInputRoute::App,
    );
    assert_eq!(
        keyboard_input_route(ElementState::Released, false),
        KeyboardInputRoute::EguiOnly,
    );
}
```

- [ ] **Step 2: 运行测试确认 RED**

Run:

```bash
cd native-prototype
cargo test keyboard_input_route_separates_synthetic_and_release_events
```

Expected: 编译失败，提示 `keyboard_input_route` 或 `KeyboardInputRoute` 尚未定义。

- [ ] **Step 3: 实现纯决策函数**

在 `point_in_terminal_bounds` 附近增加：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyboardInputRoute {
    Drop,
    EguiOnly,
    App,
}

fn keyboard_input_route(state: ElementState, is_synthetic: bool) -> KeyboardInputRoute {
    match (state, is_synthetic) {
        (ElementState::Pressed, true) => KeyboardInputRoute::Drop,
        (ElementState::Pressed, false) => KeyboardInputRoute::App,
        (ElementState::Released, _) => KeyboardInputRoute::EguiOnly,
    }
}
```

- [ ] **Step 4: 在 egui 前仅丢弃 synthetic Pressed**

在 `window_event` 开头增加：

```rust
let keyboard_route = match &event {
    WindowEvent::KeyboardInput {
        event,
        is_synthetic,
    } => Some(keyboard_input_route(event.state, *is_synthetic)),
    _ => None,
};
if keyboard_route == Some(KeyboardInputRoute::Drop) {
    return;
}
```

随后计算 `is_tab_key` 时复用路由结果：

```rust
let is_tab_key = keyboard_route == Some(KeyboardInputRoute::App)
    && matches!(
        &event,
        WindowEvent::KeyboardInput { event, .. }
            if matches!(event.logical_key, Key::Named(NamedKey::Tab))
    );
```

- [ ] **Step 5: 替换终端处理门限**

将实际键盘处理分支改为：

```rust
WindowEvent::KeyboardInput { event, .. } => {
    if keyboard_route != Some(KeyboardInputRoute::App) {
        return;
    }
    self.cursor_visible = true;
    self.cursor_timer = Instant::now();
}
```

只替换该分支的模式和两个旧 guard；从“按键时回到最底部”开始的快捷键、控制字符、特殊键与 `event.text` 代码保持原样。

删除：

```rust
if self.startup_time.elapsed().as_millis() < 500 {
    return;
}
```

不要删除 `startup_time` 字段；现有鼠标诊断日志仍使用它。

- [ ] **Step 6: 运行目标与全量测试确认 GREEN**

Run:

```bash
cd native-prototype
cargo test keyboard_input_route_separates_synthetic_and_release_events
cargo test
cargo fmt --check
cargo clippy --all-targets
```

Expected: 路由目标测试和全部 Native 测试通过；本次代码不产生新的 Clippy 警告。

### Task 2: X11 聚焦复现与最终构建

**Files:**
- Verify: `native-prototype/build.sh`
- Verify: `run-native.sh`

- [ ] **Step 1: 执行独立 Native 构建**

Run:

```bash
cd native-prototype
cargo fmt
./build.sh
```

Expected: 编译、Clippy 和全部测试通过。

- [ ] **Step 2: 启动最新 Native**

只终止 `comm` 精确为 `liteterm-native` 的测试进程或对应临时 user-systemd 单元，然后从仓库根目录运行 `./run-native.sh`。不得停止 `guishell-tauri`。

- [ ] **Step 3: 验证 synthetic 聚焦事件被丢弃**

在另一个无关窗口获得焦点时，使用 X11 自动化保持可打印键按下：

```bash
native_window_id="$(xdotool search --name 'LiteTerm Native' | tail -n 1)"
xdotool keydown n
xdotool windowactivate --sync "$native_window_id"
xdotool keyup n
```

重复使用另一组键，至少执行三轮。每轮捕获 Native 首个本地提示符截图。Expected：提示符前后均不出现被保持的字符。

若 `keydown` 已写入原先获得焦点的窗口，不将其视为 Native 失败；只检查 Native 聚焦后的终端内容。

- [ ] **Step 4: 验证真实输入仍正常**

Native 窗口聚焦后使用 `xdotool type --clearmodifiers 'k'`。Expected：提示符后只出现一个 `k`。随后发送 `BackSpace` 清除测试字符，不执行命令。

- [ ] **Step 5: 核对边界与运行进程**

Run:

```bash
sha256sum build.sh run.sh
git branch --show-current
git status --short
ps -eo pid=,comm=,args= | awk '$2 == "liteterm-native" {print}'
```

Expected：分支仍为 `feat/memory-optimize`；根目录旧脚本哈希不变；最新 Native 保持运行；实现文件保持未暂存。
