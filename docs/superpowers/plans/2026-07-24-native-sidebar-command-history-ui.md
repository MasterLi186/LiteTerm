# Native Sidebar and Command History UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve Native sidebar readability and width usage, replace the network area chart with line-only history, and anchor a GuiShell-style command-history popup to its trigger icon.

**Architecture:** Keep the 220px sidebar and introduce local typography/layout constants in `sidebar.rs` instead of changing global egui styles. Extract pure geometry helpers for rows, chart points, and popup placement so the visual behavior can be developed with TDD and then verified in a live maximized window.

**Tech Stack:** Rust, egui 0.31, winit 0.30, built-in Rust test framework

---

### Task 1: Sidebar typography and full-width cards

**Files:**
- Modify: `native-prototype/src/sidebar.rs:1-20`
- Modify: `native-prototype/src/sidebar.rs:250-625`
- Test: `native-prototype/src/sidebar.rs` new `#[cfg(test)]` module

- [ ] **Step 1: Add failing typography and width tests**

Add a test module with:

```rust
#[cfg(test)]
mod ui_tests {
    use super::{
        sidebar_card_inner_width, SIDEBAR_BODY_SIZE, SIDEBAR_META_SIZE,
        SIDEBAR_SECTION_SIZE, SIDEBAR_VALUE_SIZE,
    };

    #[test]
    fn sidebar_typography_is_readable_at_maximized_size() {
        assert_eq!(SIDEBAR_META_SIZE, 11.0);
        assert_eq!(SIDEBAR_BODY_SIZE, 12.0);
        assert_eq!(SIDEBAR_SECTION_SIZE, 12.0);
        assert_eq!(SIDEBAR_VALUE_SIZE, 13.0);
    }

    #[test]
    fn monitor_cards_fill_the_sidebar_content_width() {
        assert_eq!(sidebar_card_inner_width(196.0, 8.0), 180.0);
        assert_eq!(sidebar_card_inner_width(12.0, 8.0), 0.0);
    }
}
```

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
cd native-prototype
cargo test sidebar_typography_is_readable_at_maximized_size
```

Expected: compilation fails because the constants and `sidebar_card_inner_width` do not exist.

- [ ] **Step 3: Add local typography and geometry constants**

At module scope add:

```rust
const SIDEBAR_META_SIZE: f32 = 11.0;
const SIDEBAR_BODY_SIZE: f32 = 12.0;
const SIDEBAR_SECTION_SIZE: f32 = 12.0;
const SIDEBAR_VALUE_SIZE: f32 = 13.0;
const CONNECTION_ROW_HEIGHT: f32 = 22.0;
const PROCESS_ROW_HEIGHT: f32 = 24.0;

fn sidebar_card_inner_width(card_width: f32, horizontal_margin: f32) -> f32 {
    (card_width - horizontal_margin * 2.0).max(0.0)
}
```

- [ ] **Step 4: Apply the typography to the entire sidebar**

Replace sidebar-local 9–11px text with the matching constants:

- connection header and section titles use `SIDEBAR_SECTION_SIZE`;
- group labels and table headers use `SIDEBAR_META_SIZE`;
- connection names, process names, disk rows, network rates, memory and swap labels use `SIDEBAR_BODY_SIZE`;
- uptime/load and resource percentages use `SIDEBAR_VALUE_SIZE`.

Increase connection allocation from 18px to `CONNECTION_ROW_HEIGHT`. Do not change dialog typography below `render_dialogs`.

- [ ] **Step 5: Make uptime and resource cards full width**

For the uptime frame, compute its inner width and split it equally:

```rust
let card_width = panel_width - 24.0;
let inner_width = sidebar_card_inner_width(card_width, 8.0);
egui::Frame::new()
    .fill(card_bg)
    .corner_radius(6.0)
    .stroke(egui::Stroke::new(1.0, card_border))
    .inner_margin(egui::Margin::same(8))
    .show(ui, |ui| {
        ui.set_min_width(inner_width);
        let column_width = ((inner_width - 16.0) / 2.0).max(0.0);
        ui.columns(2, |columns| {
            columns[0].set_min_width(column_width);
            columns[1].set_min_width(column_width);
            columns[0].label(
                egui::RichText::new("运行时间")
                    .size(SIDEBAR_META_SIZE)
                    .color(label_color),
            );
            columns[0].label(
                egui::RichText::new(&mon.uptime)
                    .size(SIDEBAR_VALUE_SIZE)
                    .color(value_color),
            );
            columns[1].label(
                egui::RichText::new("系统负载")
                    .size(SIDEBAR_META_SIZE)
                    .color(label_color),
            );
            columns[1].label(
                egui::RichText::new(format!(
                    "{:.2}, {:.2}, {:.2}",
                    mon.load_avg[0], mon.load_avg[1], mon.load_avg[2]
                ))
                .size(SIDEBAR_VALUE_SIZE)
                .color(value_color),
            );
        });
    });
```

Set an explicit minimum content width in the resource, process, network, and disk card bodies so their frame response spans the same `card_width`.

- [ ] **Step 6: Put the CPU model on its own wrapped row**

Render:

```rust
ui.horizontal(|ui| {
    ui.add_space(8.0);
    ui.label(egui::RichText::new("CPU").size(SIDEBAR_BODY_SIZE).color(section_color));
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(format!("{:.1}%", mon.cpu_percent))
                .size(SIDEBAR_VALUE_SIZE)
                .color(value_color),
        );
    });
});
ui.horizontal(|ui| {
    ui.add_space(8.0);
    ui.add_sized(
        [w, 0.0],
        egui::Label::new(
            egui::RichText::new(&mon.cpu_name)
                .size(SIDEBAR_META_SIZE)
                .color(section_color),
        )
        .wrap(),
    );
});
```

- [ ] **Step 7: Run focused and full tests**

Run:

```bash
cargo fmt --manifest-path native-prototype/Cargo.toml --check
cargo test --manifest-path native-prototype/Cargo.toml sidebar_
cargo test --manifest-path native-prototype/Cargo.toml
```

Expected: both new sidebar tests and all existing tests pass.

Do not stage or commit `native-prototype/src/sidebar.rs`; it contains unrelated in-progress Native work.

### Task 2: Full-width process rows and line-only network chart

**Files:**
- Modify: `native-prototype/src/sidebar.rs:626-870`
- Test: `native-prototype/src/sidebar.rs` `ui_tests`

- [ ] **Step 1: Add failing process-row and chart-point tests**

Extend `ui_tests`:

```rust
#[test]
fn process_row_uses_the_full_available_width() {
    let size = process_row_size(188.0);
    assert_eq!(size.x, 188.0);
    assert_eq!(size.y, PROCESS_ROW_HEIGHT);
}

#[test]
fn network_history_maps_to_an_open_polyline() {
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 44.0));
    let points = network_line_points(&[0.0, 5.0, 10.0], rect, 10.0);
    assert_eq!(points.len(), 3);
    assert_eq!(points.first().unwrap().x, rect.left());
    assert_eq!(points.last().unwrap().x, rect.right());
    assert!(points.last().unwrap().y < points.first().unwrap().y);
}
```

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
cd native-prototype
cargo test process_row_uses_the_full_available_width
```

Expected: compilation fails because `process_row_size` and `network_line_points` do not exist.

- [ ] **Step 3: Implement the pure layout helpers**

Add:

```rust
fn process_row_size(width: f32) -> egui::Vec2 {
    egui::vec2(width.max(0.0), PROCESS_ROW_HEIGHT)
}

fn network_line_points(data: &[f64], rect: egui::Rect, max_value: f64) -> Vec<egui::Pos2> {
    if data.len() < 2 {
        return Vec::new();
    }
    let max_value = max_value.max(1.0);
    data.iter()
        .enumerate()
        .map(|(index, value)| {
            let x = rect.left() + index as f32 / (data.len() - 1) as f32 * rect.width();
            let y = rect.bottom() - (*value / max_value) as f32 * (rect.height() - 4.0);
            egui::pos2(x, y)
        })
        .collect()
}
```

- [ ] **Step 4: Paint each process row across the full card**

For each process:

```rust
let (row_rect, row_response) =
    ui.allocate_exact_size(process_row_size(ui.available_width()), egui::Sense::hover());
let fill = if row_response.hovered() {
    egui::Color32::from_rgba_unmultiplied(0x00, 0xd4, 0xff, 0x0f)
} else if i % 2 == 1 {
    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 4)
} else {
    egui::Color32::TRANSPARENT
};
ui.painter().rect_filled(row_rect, 0.0, fill);
let content_rect = row_rect.shrink2(egui::vec2(8.0, 0.0));
ui.scope_builder(
    egui::UiBuilder::new()
        .max_rect(content_rect)
        .layout(egui::Layout::left_to_right(egui::Align::Center)),
    |ui| {
        ui.add_sized(
            [38.0, PROCESS_ROW_HEIGHT],
            egui::Label::new(memory_text).truncate(),
        );
        ui.add_sized([42.0, PROCESS_ROW_HEIGHT], cpu_badge);
        ui.add_sized(
            [ui.available_width(), PROCESS_ROW_HEIGHT],
            egui::Label::new(command_text).truncate(),
        );
    },
);
```

Keep the existing memory text, CPU badge, and command colors when constructing
`memory_text`, `cpu_badge`, and `command_text`.

- [ ] **Step 5: Remove network area fills**

Replace the inline point and polygon construction with:

```rust
let chart_max = self
    .net_tx_history
    .iter()
    .chain(self.net_rx_history.iter())
    .copied()
    .fold(1.0_f64, f64::max);
for (data, color) in [
    (&self.net_tx_history, upload_color),
    (&self.net_rx_history, download_color),
] {
    let points = network_line_points(data, rect, chart_max);
    for segment in points.windows(2) {
        ui.painter()
            .line_segment([segment[0], segment[1]], egui::Stroke::new(1.5, color));
    }
}
```

Delete the `convex_polygon` fill path and its alpha color. Keep the dark chart background and both upload/download histories.

- [ ] **Step 6: Run focused and full tests**

Run:

```bash
cargo fmt --manifest-path native-prototype/Cargo.toml --check
cargo test --manifest-path native-prototype/Cargo.toml process_row_
cargo test --manifest-path native-prototype/Cargo.toml network_history_
cargo test --manifest-path native-prototype/Cargo.toml
```

Expected: all new geometry tests and all existing tests pass.

### Task 3: Button-anchored GuiShell-style history popup

**Files:**
- Modify: `native-prototype/src/command_bar.rs:1-30`
- Modify: `native-prototype/src/command_bar.rs:144-410`
- Test: `native-prototype/src/command_bar.rs` new `#[cfg(test)]` module

- [ ] **Step 1: Add failing popup geometry tests**

Add:

```rust
#[cfg(test)]
mod ui_tests {
    use super::{history_popup_position, history_popup_size};

    #[test]
    fn history_popup_aligns_its_right_edge_to_the_button() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let button = egui::Rect::from_min_size(egui::pos2(1180.0, 760.0), egui::vec2(20.0, 18.0));
        let size = history_popup_size(20, screen);
        let pos = history_popup_position(button, size, screen);
        assert_eq!(pos.x + size.x, button.right());
        assert!(pos.y + size.y <= button.top());
    }

    #[test]
    fn history_popup_is_clamped_inside_a_narrow_screen() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 240.0));
        let button = egui::Rect::from_min_size(egui::pos2(8.0, 210.0), egui::vec2(20.0, 18.0));
        let size = history_popup_size(50, screen);
        let pos = history_popup_position(button, size, screen);
        assert!(pos.x >= screen.left() + 4.0);
        assert!(pos.y >= screen.top() + 4.0);
        assert!(pos.x + size.x <= screen.right() - 4.0);
        assert!(pos.y + size.y <= screen.bottom() - 4.0);
    }
}
```

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
cd native-prototype
cargo test history_popup_aligns_its_right_edge_to_the_button
```

Expected: compilation fails because the popup geometry helpers do not exist.

- [ ] **Step 3: Implement popup size and placement**

Add:

```rust
const HISTORY_POPUP_WIDTH: f32 = 384.0;
const HISTORY_POPUP_MAX_HEIGHT: f32 = 288.0;
const HISTORY_POPUP_MARGIN: f32 = 4.0;

fn history_popup_size(history_len: usize, screen: egui::Rect) -> egui::Vec2 {
    let width = HISTORY_POPUP_WIDTH.min((screen.width() - 8.0).max(1.0));
    let wanted_height = (64.0 + history_len as f32 * 28.0).clamp(96.0, HISTORY_POPUP_MAX_HEIGHT);
    egui::vec2(width, wanted_height.min((screen.height() - 8.0).max(1.0)))
}

fn history_popup_position(
    button: egui::Rect,
    size: egui::Vec2,
    screen: egui::Rect,
) -> egui::Pos2 {
    let min_x = screen.left() + HISTORY_POPUP_MARGIN;
    let max_x = (screen.right() - HISTORY_POPUP_MARGIN - size.x).max(min_x);
    let x = (button.right() - size.x).clamp(min_x, max_x);
    let above = button.top() - HISTORY_POPUP_MARGIN - size.y;
    let below = button.bottom() + HISTORY_POPUP_MARGIN;
    let min_y = screen.top() + HISTORY_POPUP_MARGIN;
    let max_y = (screen.bottom() - HISTORY_POPUP_MARGIN - size.y).max(min_y);
    let y = if above >= min_y { above } else { below.clamp(min_y, max_y) };
    egui::pos2(x, y)
}
```

- [ ] **Step 4: Capture the history button rectangle**

Inside `CommandBar::ui`, remove `popup_left`, keep the public signature as:

```rust
pub fn ui(&mut self, ctx: &egui::Context, _sidebar_width: f32) -> Option<String>
```

Declare:

```rust
let mut history_button_rect = None;
let mut history_opened_this_frame = false;
```

After creating `hist_btn`, assign `history_button_rect = Some(hist_btn.rect)` and set `history_opened_this_frame` when the click opens the popup.

- [ ] **Step 5: Render a constrained popup and outside-click backdrop**

When `show_history` is true:

```rust
let screen = ctx.screen_rect();
let button = history_button_rect.unwrap_or_else(|| {
    egui::Rect::from_min_size(screen.right_bottom() - egui::vec2(24.0, 24.0), egui::vec2(20.0, 18.0))
});
let popup_size = history_popup_size(self.history.len(), screen);
let popup_pos = history_popup_position(button, popup_size, screen);
```

Render a full-screen `egui::Area` in `Order::Middle` as the outside-click target. Ignore its click during `history_opened_this_frame`. Render the popup as a foreground `egui::Area` with `fixed_pos(popup_pos)` and a framed `ui.set_min_size(popup_size)` body.

- [ ] **Step 6: Align popup content with GuiShell**

Build a custom header showing `命令历史（{count}）` and a right-aligned `清空` button. For each history command:

- clicking the elided monospace text assigns it to `self.input_text`;
- `▶` executes it through `clicked_cmd` and closes the popup;
- `⧉` calls `ctx.copy_text(command.clone())`;
- `☆`/`★` adds or removes the matching favorite and persists favorites.

Use 28px rows, a separator between rows, and a vertical `ScrollArea` limited to the popup body. Remove the old bottom “清空历史/关闭” footer.

- [ ] **Step 7: Run focused and full verification**

Run:

```bash
cargo fmt --manifest-path native-prototype/Cargo.toml --check
cargo test --manifest-path native-prototype/Cargo.toml history_popup_
./native-prototype/build.sh
```

Expected: popup tests pass; build, Clippy command, and all Native tests exit successfully.

Do not stage or commit `native-prototype/src/command_bar.rs`; it is an untracked in-progress Native file.

### Task 4: Independent review and live visual verification

**Files:**
- Review: `native-prototype/src/sidebar.rs`
- Review: `native-prototype/src/command_bar.rs`
- Verify: `native-prototype/target/debug/liteterm-native`

- [ ] **Step 1: Run specification review**

Verify all six user-visible requirements against actual code: full-sidebar typography, full-width process rows, line-only network history, wrapped CPU model, full-width uptime card, and icon-anchored GuiShell history layout.

- [ ] **Step 2: Run code-quality review**

Check that typography remains local to the sidebar, geometry helpers match rendered use, popup operations persist correctly, Unicode slicing is safe, and no input event leaks through the popup backdrop.

- [ ] **Step 3: Build and launch only Native**

Run:

```bash
./native-prototype/build.sh
./run-native.sh
```

Keep the old `guishell-tauri` process running. Confirm root `build.sh` and `run.sh` hashes remain unchanged.

- [ ] **Step 4: Verify maximized and narrow layouts**

At 1280×800 and a narrower window:

1. Capture the full sidebar and confirm all sidebar text is readable.
2. Confirm uptime/load and every monitor card fill the available width.
3. Confirm the CPU model occupies a separate wrapped row.
4. Hover multiple process rows and confirm the background spans the full row.
5. Observe several network samples and confirm only green/blue polylines are drawn.
6. Open history and confirm it appears immediately above the history icon, remains on-screen, and matches the GuiShell header/row actions.
7. Exercise fill, execute, copy, favorite, clear, and outside-click close actions.

Save screenshots for the maximized sidebar, process-row hover, line-only chart, history popup, and narrow-window clamping.
