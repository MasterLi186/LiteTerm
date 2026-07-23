# Native 删除确认框自适应布局实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Native 文件管理器的删除确认框根据文件名动态调整宽度，并在超长内容换行时自动增加高度。

**Architecture:** 在 `file_browser.rs` 中增加一个基于 egui 当前字体测量结果的宽度策略函数，将宽度限制在 `260–520px`。删除窗口固定计算后的宽度但不固定高度，正文与路径使用换行标签，由 egui 内容布局决定最终高度。

**Tech Stack:** Rust、egui 0.31、现有 Native 单元测试与 `native-prototype/build.sh`

---

## 文件范围

- Modify/Test: `native-prototype/src/file_browser.rs`
- Verify: `native-prototype/build.sh`

`native-prototype/src/file_browser.rs` 当前是工作区已有的未跟踪文件，实施代码保持不暂存，避免把此前整份 Native 文件管理器内容意外提交。设计与计划文档可独立提交。

### Task 1: 文件名驱动的宽度策略

**Files:**
- Modify/Test: `native-prototype/src/file_browser.rs`

- [ ] **Step 1: 写入失败测试**

在 `ui_tests` 中导入 `delete_dialog_width`、`DELETE_DIALOG_MIN_WIDTH` 和 `DELETE_DIALOG_MAX_WIDTH`，增加：

```rust
#[test]
fn delete_dialog_width_grows_with_name_and_stops_at_maximum() {
    let ctx = egui::Context::default();
    let mut widths = [0.0; 3];
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        widths[0] = delete_dialog_width(ctx, "a");
        widths[1] = delete_dialog_width(ctx, &"medium-name-".repeat(4));
        widths[2] = delete_dialog_width(ctx, &"very-long-name-".repeat(40));
    });

    assert_eq!(widths[0], DELETE_DIALOG_MIN_WIDTH);
    assert!(widths[1] > widths[0]);
    assert_eq!(widths[2], DELETE_DIALOG_MAX_WIDTH);
}
```

- [ ] **Step 2: 运行测试确认 RED**

Run:

```bash
cd native-prototype
cargo test delete_dialog_width_grows_with_name_and_stops_at_maximum
```

Expected: 编译失败，提示 `delete_dialog_width` 或宽度常量尚未定义。

- [ ] **Step 3: 实现最小宽度计算**

在删除弹窗渲染函数附近增加：

```rust
const DELETE_DIALOG_MIN_WIDTH: f32 = 260.0;
const DELETE_DIALOG_MAX_WIDTH: f32 = 520.0;
const DELETE_DIALOG_HORIZONTAL_CHROME: f32 = 44.0;

fn delete_dialog_width(ctx: &egui::Context, name: &str) -> f32 {
    let prompt = format!("确定要删除“{name}”吗？");
    let text_width = ctx.fonts(|fonts| {
        fonts
            .layout_no_wrap(
                prompt,
                egui::FontId::proportional(12.0),
                TEXT,
            )
            .size()
            .x
    });
    (text_width + DELETE_DIALOG_HORIZONTAL_CHROME)
        .clamp(DELETE_DIALOG_MIN_WIDTH, DELETE_DIALOG_MAX_WIDTH)
}
```

- [ ] **Step 4: 运行测试确认 GREEN**

Run:

```bash
cd native-prototype
cargo test delete_dialog_width_grows_with_name_and_stops_at_maximum
```

Expected: 目标测试通过。

### Task 2: 内容包裹与动态高度

**Files:**
- Modify/Test: `native-prototype/src/file_browser.rs`

- [ ] **Step 1: 写入失败测试**

让 `render_delete_dialog` 返回 `Option<egui::Rect>`，在 `ui_tests` 的导入列表中加入该函数，并增加以下测试辅助函数与断言：

```rust
fn rendered_delete_dialog_size(name: &str, path: &str) -> egui::Vec2 {
    let ctx = egui::Context::default();
    let mut state = FileBrowserState::new("/tmp".into());
    state.delete_dialog = Some(DeleteDialogState {
        side: crate::sftp::FileSide::Local,
        name: name.into(),
        path: path.into(),
        is_dir: false,
        just_opened: false,
    });
    let mut size = egui::Vec2::ZERO;
    for _ in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            let mut actions = Vec::new();
            size = render_delete_dialog(ctx, &mut state, &mut actions)
                .expect("delete dialog should render")
                .size();
        });
    }
    size
}

#[test]
fn long_delete_name_wraps_and_makes_dialog_taller() {
    let short = rendered_delete_dialog_size("a", "/tmp/a");
    let long_name = "超长文件名".repeat(80);
    let long_path = format!("/tmp/{long_name}");
    let long = rendered_delete_dialog_size(&long_name, &long_path);

    assert!(short.y < 138.0, "short dialog should shrink: {short:?}");
    assert!(long.y > short.y, "long dialog should grow: short={short:?}, long={long:?}");
}
```

- [ ] **Step 2: 运行测试确认 RED**

Run:

```bash
cd native-prototype
cargo test long_delete_name_wraps_and_makes_dialog_taller
```

Expected: 测试编译失败或断言失败，因为当前函数不返回窗口矩形且窗口仍固定为 `380×138`。

- [ ] **Step 3: 改为固定动态宽度、自动内容高度**

把函数返回类型改为 `Option<egui::Rect>`，无弹窗时返回 `None`。计算 `dialog_width`，移除 `.fixed_size(egui::vec2(380.0, 138.0))`，将窗口主体改为：

```rust
let dialog_width = delete_dialog_width(ctx, &dialog.name);
let window = egui::Window::new("确认删除")
    .id(egui::Id::new("file_delete_dialog"))
    .order(egui::Order::Foreground)
    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
    .default_width(dialog_width)
    .min_width(dialog_width)
    .max_width(dialog_width)
    .collapsible(false)
    .resizable(false)
    .title_bar(true)
    .show(ctx, |ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("确定要删除“{}”吗？", dialog.name))
                    .size(12.0)
                    .color(TEXT),
            )
            .wrap(),
        );
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(&dialog.path).size(10.0).color(MUTED),
            )
            .wrap(),
        );
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("删除").color(RED)).fill(
                        egui::Color32::from_rgba_unmultiplied(
                            RED.r(),
                            RED.g(),
                            RED.b(),
                            32,
                        ),
                    ),
                )
                .clicked()
            {
                submit = true;
            }
            if ui.button("取消").clicked() {
                cancel = true;
            }
        });
    });
```

在取消判定前保存窗口矩形：

```rust
let window_rect = window.as_ref().map(|window| window.response.rect);
```

处理取消或确认后以 `window_rect` 作为函数返回值。调用方保持现状并忽略返回值。

- [ ] **Step 4: 运行目标测试与全部 Native 测试**

Run:

```bash
cd native-prototype
cargo test long_delete_name_wraps_and_makes_dialog_taller
cargo test
```

Expected: 动态高度测试通过，全部 Native 测试零失败。

### Task 3: 构建与现场 UI 验证

**Files:**
- Verify: `native-prototype/build.sh`

- [ ] **Step 1: 格式化并执行独立 Native 构建**

Run:

```bash
cd native-prototype
cargo fmt
./build.sh
```

Expected: Rust 编译、Clippy 和全部测试通过。

- [ ] **Step 2: 重启新 Native 二进制**

仅终止精确匹配的 `liteterm-native` 测试进程，然后运行：

```bash
./run-native.sh
```

Expected: `native-prototype/target/debug/liteterm-native` 启动；根目录旧 GuiShell 不受影响。

- [ ] **Step 3: 验证短文件名**

在本地或远端文件列表中对短文件名打开删除确认框。确认窗口宽度接近最小值，高度小于旧的 `138px`，按钮紧跟路径下方；取消后文件仍存在。

- [ ] **Step 4: 验证超长文件名**

创建一个一次性超长文件名，对其打开删除确认框。确认窗口宽度不超过约 `520px`，文件名与路径完整换行，窗口高度增加且按钮仍可见。取消后清理该一次性文件。

- [ ] **Step 5: 核对工作区边界**

Run:

```bash
sha256sum build.sh run.sh
git branch --show-current
git status --short
```

Expected: 分支仍为 `feat/memory-optimize`，根目录旧脚本哈希不变，Native 实现文件保持未暂存。
