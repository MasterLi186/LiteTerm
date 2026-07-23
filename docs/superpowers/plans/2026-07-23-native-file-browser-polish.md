# Native File Browser Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the rough Native file-browser grid with a compact `main`-style dual-pane UI, eliminate egui duplicate-ID diagnostics, and format modification times as `MM-DD HH:mm`.

**Architecture:** Keep the existing `FileBrowserState`, actions, and SFTP event flow. Give each pane a stable egui ID scope, replace `Grid` rows with full-width painted rows driven by a pure column-layout helper, and format Unix timestamps through `chrono::Local`.

**Tech Stack:** Rust, egui 0.31, chrono 0.4, Cargo tests

---

### Task 1: Add readable modification-time formatting

**Files:**
- Modify: `native-prototype/Cargo.toml`
- Modify: `native-prototype/Cargo.lock`
- Modify: `native-prototype/src/file_browser.rs`
- Test: `native-prototype/src/file_browser.rs`

- [ ] **Step 1: Add failing formatter tests**

Extend `ui_tests`:

```rust
use super::{format_mtime, format_size, reserved_height};

#[test]
fn zero_mtime_is_blank() {
    assert_eq!(format_mtime(0), "");
}

#[test]
fn mtime_matches_main_display_shape() {
    let value = format_mtime(1_784_135_600);
    assert_eq!(value.len(), 11);
    assert_eq!(&value[2..3], "-");
    assert_eq!(&value[5..6], " ");
    assert_eq!(&value[8..9], ":");
    assert!(value
        .chars()
        .enumerate()
        .all(|(index, ch)| matches!(index, 2 | 5 | 8) || ch.is_ascii_digit()));
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
(cd native-prototype && cargo test file_browser::ui_tests::mtime -- --nocapture)
```

Expected: compilation fails because `format_mtime` does not exist.

- [ ] **Step 3: Add chrono and implement the formatter**

Add:

```toml
chrono = { version = "0.4", default-features = false, features = ["clock"] }
```

Implement:

```rust
pub fn format_mtime(epoch: u64) -> String {
    use chrono::{Local, TimeZone};

    if epoch == 0 {
        return String::new();
    }
    Local
        .timestamp_opt(epoch as i64, 0)
        .single()
        .map(|time| time.format("%m-%d %H:%M").to_string())
        .unwrap_or_default()
}
```

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run the command from Step 2. Expected: both mtime tests pass.

### Task 2: Reproduce and eliminate duplicate egui IDs

**Files:**
- Modify: `native-prototype/src/file_browser.rs`
- Test: `native-prototype/src/file_browser.rs`

- [ ] **Step 1: Add a failing dual-pane render test**

Add a recursive helper that collects painted text:

```rust
fn collect_text(shape: &egui::epaint::Shape, output: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Text(text) => output.push(text.galley.job.text.clone()),
        egui::epaint::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, output);
            }
        }
        _ => {}
    }
}
```

Render an open `FileBrowserState` in a `1280x800` `egui::RawInput`, collect all text from `FullOutput.shapes`, and assert that none contains `"use of"` or `"ScrollArea ID"`.

```rust
#[test]
fn dual_pane_render_has_unique_ids() {
    let ctx = egui::Context::default();
    let mut input = egui::RawInput::default();
    input.screen_rect = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(1280.0, 800.0),
    ));
    let mut state = FileBrowserState::new("/tmp".into());
    let output = ctx.run(input, |ctx| {
        let _ = render(ctx, &mut state);
    });
    let mut painted_text = Vec::new();
    for clipped in &output.shapes {
        collect_text(&clipped.shape, &mut painted_text);
    }
    assert!(
        painted_text
            .iter()
            .all(|text| !text.contains("use of") && !text.contains("ScrollArea ID")),
        "duplicate egui IDs: {painted_text:#?}"
    );
}
```

- [ ] **Step 2: Run the render test and verify RED**

Run:

```bash
(cd native-prototype && cargo test file_browser::ui_tests::dual_pane_render_has_unique_ids -- --nocapture)
```

Expected: FAIL with the duplicate-ID diagnostic text currently visible in the screenshot.

- [ ] **Step 3: Scope each pane with stable IDs**

Wrap each pane call:

```rust
columns[0].push_id("local_file_pane", |ui| {
    render_pane(
        ui,
        FileSide::Local,
        &mut state.local,
        &remote_destination,
        &mut actions,
    );
});
columns[1].push_id("remote_file_pane", |ui| {
    render_pane(
        ui,
        FileSide::Remote,
        &mut state.remote,
        &local_destination,
        &mut actions,
    );
});
```

Also set:

```rust
egui::ScrollArea::vertical()
    .id_salt(match side {
        FileSide::Local => "local_file_scroll",
        FileSide::Remote => "remote_file_scroll",
    })
```

- [ ] **Step 4: Run the render test and verify GREEN**

Run the command from Step 2. Expected: PASS with no ID diagnostic text.

### Task 3: Define deterministic compact column layout

**Files:**
- Modify: `native-prototype/src/file_browser.rs`
- Test: `native-prototype/src/file_browser.rs`

- [ ] **Step 1: Add failing column-layout tests**

Define the desired API in tests:

```rust
#[test]
fn file_columns_fill_available_width() {
    let columns = file_columns(600.0);
    assert_eq!(columns.size, 64.0);
    assert_eq!(columns.mtime, 94.0);
    assert_eq!(columns.name + columns.size + columns.mtime, 600.0);
}

#[test]
fn file_name_column_keeps_a_usable_minimum() {
    assert_eq!(file_columns(180.0).name, 80.0);
}
```

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
(cd native-prototype && cargo test file_browser::ui_tests::file_columns -- --nocapture)
```

Expected: compilation fails because `file_columns` does not exist.

- [ ] **Step 3: Implement the layout helper**

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
struct FileColumns {
    name: f32,
    size: f32,
    mtime: f32,
}

fn file_columns(width: f32) -> FileColumns {
    const SIZE: f32 = 64.0;
    const MTIME: f32 = 94.0;
    FileColumns {
        name: (width - SIZE - MTIME).max(80.0),
        size: SIZE,
        mtime: MTIME,
    }
}
```

- [ ] **Step 4: Run tests and verify GREEN**

Run the command from Step 2. Expected: both layout tests pass.

### Task 4: Replace the grid with polished full-width rows

**Files:**
- Modify: `native-prototype/src/file_browser.rs`

- [ ] **Step 1: Add theme constants and panel frames**

Use:

```rust
const PANEL_BG: egui::Color32 = egui::Color32::from_rgb(0x0d, 0x11, 0x17);
const HEADER_BG: egui::Color32 = egui::Color32::from_rgb(0x16, 0x1b, 0x22);
const BORDER: egui::Color32 = egui::Color32::from_rgb(0x21, 0x26, 0x2d);
const MUTED: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x94, 0x9e);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xc9, 0xd1, 0xd9);
const CYAN: egui::Color32 = egui::Color32::from_rgb(0x00, 0xd4, 0xff);
const YELLOW: egui::Color32 = egui::Color32::from_rgb(0xd2, 0x99, 0x22);
```

Apply `PANEL_BG` to the main panel, `HEADER_BG` to pane and column headers, and `BORDER` to separators.

- [ ] **Step 2: Render the column header as a full-width row**

Allocate a 20 px row, compute `file_columns(rect.width())`, and paint “文件名”, “大小”, and “修改时间” at stable left/right anchors using a 10 px proportional font and muted color.

- [ ] **Step 3: Render each file as a full-width interactive row**

For each entry, allocate a 20 px clickable row. Paint:

```text
selected: rgba(0,212,255,0.10)
hovered:  rgba(0,212,255,0.06)
odd row:  rgba(255,255,255,0.015)
directory marker: ▸ in #d29922
file marker: · in #6e7681
```

Use `format_size(entry.size)` and `format_mtime(entry.mtime)`. Preserve the existing click and double-click action generation exactly.

- [ ] **Step 4: Style loading, errors, transfers, and toggle bar**

Keep the same data and actions while using the shared theme colors, compact margins, and Chinese status text. Do not add sorting, search, context menus, rename, or delete.

- [ ] **Step 5: Run all Native tests**

Run:

```bash
(cd native-prototype && cargo fmt --check && cargo test)
```

Expected: all tests pass.

### Task 5: Build and visually verify

**Files:**
- Verify: `native-prototype/target/debug/liteterm-native`
- Verify unchanged: `build.sh`
- Verify unchanged: `run.sh`

- [ ] **Step 1: Run the isolated Native build**

Run:

```bash
./native-prototype/build.sh
```

Expected: build, normal Clippy, and all Native tests succeed; existing warnings may remain.

- [ ] **Step 2: Verify old GuiShell scripts remain unchanged**

Run:

```bash
sha256sum build.sh run.sh
```

Expected:

```text
69e913332c3b14111a62f5843cf35dd6751f0f566dc941efc1218364350a2a5e  build.sh
f7d1b7b264548f8e1b968c781c0fa38529a92b24e2bd894b6dcea3b080b1ff6e  run.sh
```

- [ ] **Step 3: Launch Native for manual inspection**

Run:

```bash
./run-native.sh
```

Confirm the duplicate-ID overlay is absent, both panes fill their width, and modification times use `MM-DD HH:mm`.

### Task 6: Review and commit only in-scope changes

**Files:**
- Modify: `native-prototype/Cargo.toml`
- Modify: `native-prototype/Cargo.lock`
- Modify: `native-prototype/src/file_browser.rs`

- [ ] **Step 1: Review scoped changes and dirty-worktree overlap**

Run:

```bash
git diff -- native-prototype/Cargo.toml native-prototype/Cargo.lock
git diff --no-index /dev/null native-prototype/src/file_browser.rs
git status --short
```

Because these files contain earlier uncommitted Native work, do not stage unrelated changes. If the implementation cannot form a self-contained commit without those prerequisites, leave it verified in the working tree and report that explicitly.

- [ ] **Step 2: Commit only when the index is self-contained**

If and only if a clean checkout of the staged index compiles and tests:

```bash
git commit -m "fix: 美化 Native 文件管理器"
```

Otherwise, preserve the implementation unstaged rather than mixing prior work into this commit.
