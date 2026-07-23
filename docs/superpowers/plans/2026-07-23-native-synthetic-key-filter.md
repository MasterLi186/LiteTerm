# Native 合成按键过滤实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Native 键盘事件入口丢弃 X11/winit 合成按键，消除启动时提示符前后出现相同随机字符串的问题。

**Architecture:** 提取一个纯键盘事件决策函数，以 `ElementState` 和 `is_synthetic` 判断事件是否可进入 Native 按键处理。synthetic 事件在传给 egui 和应用快捷键前直接返回；真实释放事件仍可传给 egui，但不会写入终端。删除原有 500ms 时间门限，保留真实按键 repeat 和所有现有快捷键映射。

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

在 `layout_tests` 中导入 `should_handle_keyboard_input` 和 `ElementState`，增加：

```rust
#[test]
fn synthetic_focus_keypress_is_never_handled() {
    assert!(!should_handle_keyboard_input(
        ElementState::Pressed,
        true,
        false,
    ));
}

#[test]
fn physical_pressed_is_handled_but_released_is_not() {
    assert!(should_handle_keyboard_input(
        ElementState::Pressed,
        false,
        false,
    ));
    assert!(!should_handle_keyboard_input(
        ElementState::Released,
        false,
        false,
    ));
}

#[test]
fn physical_repeat_press_remains_handled() {
    assert!(should_handle_keyboard_input(
        ElementState::Pressed,
        false,
        true,
    ));
}
```

- [ ] **Step 2: 运行测试确认 RED**

Run:

```bash
cd native-prototype
cargo test synthetic_focus_keypress_is_never_handled
```

Expected: 编译失败，提示 `should_handle_keyboard_input` 尚未定义。

- [ ] **Step 3: 实现纯决策函数**

在 `point_in_terminal_bounds` 附近增加：

```rust
fn should_handle_keyboard_input(
    state: ElementState,
    is_synthetic: bool,
    _repeat: bool,
) -> bool {
    state == ElementState::Pressed && !is_synthetic
}
```

- [ ] **Step 4: 在 egui 前丢弃 synthetic 事件**

在 `window_event` 开头增加：

```rust
if matches!(
    &event,
    WindowEvent::KeyboardInput {
        is_synthetic: true,
        ..
    }
) {
    return;
}
```

随后计算 `is_tab_key` 时显式读取 `is_synthetic` 并使用决策函数：

```rust
let is_tab_key = if let WindowEvent::KeyboardInput {
    event,
    is_synthetic,
} = &event
{
    should_handle_keyboard_input(event.state, *is_synthetic, event.repeat)
        && matches!(event.logical_key, Key::Named(NamedKey::Tab))
} else {
    false
};
```

- [ ] **Step 5: 替换终端处理门限**

将实际键盘处理分支改为：

```rust
WindowEvent::KeyboardInput {
    event,
    is_synthetic,
} => {
    if !should_handle_keyboard_input(event.state, is_synthetic, event.repeat) {
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
cargo test synthetic_focus_keypress_is_never_handled
cargo test physical_pressed_is_handled_but_released_is_not
cargo test physical_repeat_press_remains_handled
cargo test
cargo fmt --check
cargo clippy --all-targets
```

Expected: 三个目标测试和全部 Native 测试通过；本次代码不产生新的 Clippy 警告。

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
