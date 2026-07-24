# Native Sidebar Full-Width and Disk Format Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every Native monitor card reach the sidebar divider and render disk rows as readable mount, usage, and compact capacity columns with TiB formatting.

**Architecture:** Keep the 220px sidebar and derive each card's content width from a finite target outer width minus its actual stroke. Format capacity once in `monitor.rs`, then render disk headers and rows through shared pure column geometry in `sidebar.rs` so narrow widths cannot overlap.

**Tech Stack:** Rust, egui 0.31, sysinfo, built-in Rust test framework

---

### Task 1: Make monitor card frames fill the sidebar

**Files:**
- Modify: `native-prototype/src/sidebar.rs:1-75`
- Test: `native-prototype/src/sidebar.rs:1880-1960`

- [ ] **Step 1: Change the geometry expectations first**

Update the existing tests before production code:

```rust
#[test]
fn monitor_card_width_fills_normal_sidebar_and_shrinks_to_available_space() {
    assert_eq!(sidebar_monitor_card_width(220.0, 220.0), 218.0);
    assert_eq!(sidebar_monitor_card_width(220.0, 120.0), 118.0);
    assert_eq!(sidebar_monitor_card_width(12.0, 1.0), 0.0);
}

#[test]
fn monitor_text_geometry_matches_the_full_width_card_interior() {
    assert_eq!(sidebar_uptime_column_width(218.0), 101.0);
    assert_eq!(sidebar_cpu_text_width(218.0), 202.0);
    assert_eq!(sidebar_uptime_column_width(118.0), 51.0);
    assert_eq!(sidebar_cpu_text_width(118.0), 102.0);
}
```

In `monitor_frame_geometry_never_exceeds_available_width`, change the normal assertions to:

```rust
let normal = sidebar_monitor_card_geometry(220.0, 220.0);
assert_eq!(normal.card_content_width, 218.0);
assert_eq!(normal.uptime_content_width, 202.0);
assert_eq!(normal.uptime_inner_margin, 8.0);
assert_eq!(normal.stroke_width, 1.0);
assert_eq!(
    normal.card_content_width + normal.stroke_width * 2.0,
    220.0
);
assert!(normal.can_render);
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml monitor_card_width_fills_normal_sidebar
```

Expected: FAIL because the current normal card content width is 196px rather than 218px.

- [ ] **Step 3: Derive content width from the target outer width**

Remove `SIDEBAR_CARD_GUTTER`. Replace `sidebar_monitor_card_width` with:

```rust
fn sidebar_monitor_card_width(panel_width: f32, available_width: f32) -> f32 {
    let panel_width = if panel_width.is_finite() {
        panel_width.max(0.0)
    } else {
        0.0
    };
    let available_width = if available_width.is_finite() {
        available_width.max(0.0)
    } else {
        0.0
    };
    let outer_width = panel_width.min(available_width);
    let stroke_width = SIDEBAR_CARD_STROKE_WIDTH.min(outer_width / 2.0);
    (outer_width - stroke_width * 2.0).max(0.0)
}
```

In `sidebar_monitor_card_geometry`, derive the same outer width and stroke:

```rust
let available_width = if available_width.is_finite() {
    available_width.max(0.0)
} else {
    0.0
};
let panel_width = if panel_width.is_finite() {
    panel_width.max(0.0)
} else {
    0.0
};
let outer_width = panel_width.min(available_width);
let stroke_width = SIDEBAR_CARD_STROKE_WIDTH.min(outer_width / 2.0);
let card_content_width = sidebar_monitor_card_width(panel_width, available_width);
```

Keep the existing uptime margin and `can_render` calculations. All five monitor frames already consume this geometry, so no per-card width constant is needed.

- [ ] **Step 4: Verify focused and full tests**

Run:

```bash
cargo fmt --manifest-path native-prototype/Cargo.toml --check
cargo test --manifest-path native-prototype/Cargo.toml sidebar::ui_tests::monitor_
cargo test --manifest-path native-prototype/Cargo.toml
```

Expected: normal frame outer width is exactly 220px, narrow outer width is exactly 120px, and all tests pass.

Do not stage or commit `native-prototype/src/sidebar.rs`; it contains pre-existing uncommitted Native work.

### Task 2: Format TiB capacities at the monitor data source

**Files:**
- Modify: `native-prototype/src/monitor.rs:45-59`
- Test: `native-prototype/src/monitor.rs:247-260`

- [ ] **Step 1: Add failing binary-unit boundary tests**

Extend `monitor.rs` tests:

```rust
use super::{format_bytes, process_refresh_kind};

#[test]
fn disk_capacity_uses_tib_at_the_binary_threshold() {
    const TIB: u64 = 1_099_511_627_776;
    assert_eq!(format_bytes(TIB - 1), "1024.0G");
    assert_eq!(format_bytes(TIB), "1.0T");
    assert_eq!(format_bytes(TIB + TIB / 10), "1.1T");
}

#[test]
fn disk_capacity_keeps_existing_smaller_units() {
    assert_eq!(format_bytes(1024), "1K");
    assert_eq!(format_bytes(1024 * 1024), "1M");
    assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0G");
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml disk_capacity_uses_tib
```

Expected: FAIL because 1 TiB is currently formatted as `1024.0G`.

- [ ] **Step 3: Add the TiB branch**

Replace `format_bytes` with:

```rust
fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    const TIB: f64 = GIB * 1024.0;

    let bytes = bytes as f64;
    if bytes >= TIB {
        format!("{:.1}T", bytes / TIB)
    } else if bytes >= GIB {
        format!("{:.1}G", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.0}M", bytes / MIB)
    } else {
        format!("{}K", (bytes / KIB) as u64)
    }
}
```

This preserves the existing `DiskItem` shape and automatically applies the new unit to available and total capacity.

- [ ] **Step 4: Verify focused and full tests**

Run:

```bash
cargo fmt --manifest-path native-prototype/Cargo.toml --check
cargo test --manifest-path native-prototype/Cargo.toml disk_capacity_
cargo test --manifest-path native-prototype/Cargo.toml
```

Expected: the boundary tests and all existing tests pass.

Do not stage or commit `native-prototype/src/monitor.rs`; it is an untracked in-progress Native file.

### Task 3: Render disk data in three bounded columns

**Files:**
- Modify: `native-prototype/src/sidebar.rs:1-220`
- Modify: `native-prototype/src/sidebar.rs:1120-1210`
- Test: `native-prototype/src/sidebar.rs` `ui_tests`

- [ ] **Step 1: Add failing disk-column tests**

Add constants and helper expectations to `ui_tests`:

```rust
#[test]
fn disk_columns_fit_normal_and_narrow_rows() {
    for width in [202.0, 102.0, 64.25, 20.0, 1.0, 0.0, -1.0] {
        let columns = disk_row_columns(width);
        let total = columns.mount_width
            + columns.percent_width
            + columns.capacity_width
            + columns.gap_width * 2.0;
        assert!(columns.mount_width >= 0.0);
        assert!(columns.percent_width >= 0.0);
        assert!(columns.capacity_width >= 0.0);
        assert!(columns.gap_width >= 0.0);
        assert!(total <= width.max(0.0));
    }

    let normal = disk_row_columns(202.0);
    assert_eq!(normal.percent_width, 32.0);
    assert_eq!(normal.capacity_width, 80.0);
    assert_eq!(normal.gap_width, 4.0);
    assert_eq!(normal.mount_width, 82.0);
}
```

Add a headless rendering test:

```rust
#[test]
fn disk_row_content_does_not_advance_or_expand_the_parent() {
    egui::__run_test_ui(|ui| {
        let (row_rect, _) =
            ui.allocate_exact_size(egui::vec2(202.0, DISK_ROW_HEIGHT), egui::Sense::hover());
        let cursor_after_row = ui.cursor();
        render_disk_row_content(
            ui,
            row_rect,
            "/home/用户/very-long-mount-point",
            "96%",
            "933.4G/1.8T",
            egui::Color32::RED,
            egui::Color32::GRAY,
        );
        assert_eq!(ui.cursor(), cursor_after_row);
    });
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml disk_columns_fit_normal
```

Expected: compilation fails because `disk_row_columns`, `DISK_ROW_HEIGHT`, and the disk column structure do not exist.

- [ ] **Step 3: Add pure disk-column geometry**

At module scope add:

```rust
const DISK_ROW_HEIGHT: f32 = 22.0;
const DISK_PERCENT_COLUMN_WIDTH: f32 = 32.0;
const DISK_CAPACITY_COLUMN_WIDTH: f32 = 80.0;
const DISK_COLUMN_GAP: f32 = 4.0;
const DISK_MIN_MOUNT_COLUMN_WIDTH: f32 = 32.0;

#[derive(Clone, Copy, Debug)]
struct DiskRowColumns {
    mount_width: f32,
    percent_width: f32,
    capacity_width: f32,
    gap_width: f32,
}

fn disk_row_columns(content_width: f32) -> DiskRowColumns {
    let content_width = if content_width.is_finite() {
        content_width.max(0.0)
    } else {
        0.0
    };
    let mount_reserve = content_width.min(DISK_MIN_MOUNT_COLUMN_WIDTH);
    let fixed_width =
        DISK_PERCENT_COLUMN_WIDTH + DISK_CAPACITY_COLUMN_WIDTH + DISK_COLUMN_GAP * 2.0;
    let scale = ((content_width - mount_reserve) / fixed_width).clamp(0.0, 1.0);
    let percent_width = DISK_PERCENT_COLUMN_WIDTH * scale;
    let capacity_width = DISK_CAPACITY_COLUMN_WIDTH * scale;
    let gap_width = DISK_COLUMN_GAP * scale;
    let used = percent_width + capacity_width + gap_width * 2.0;
    let mut mount_width = (content_width - used).max(0.0);
    while mount_width + used > content_width && mount_width > 0.0 {
        mount_width = mount_width.next_down().max(0.0);
    }

    DiskRowColumns {
        mount_width,
        percent_width,
        capacity_width,
        gap_width,
    }
}
```

- [ ] **Step 4: Add shared bounded row rendering**

Add `render_disk_row_content` beside `render_process_row_content`. It must:

1. inset the row by at most 8px on each side;
2. derive three non-overlapping rects from `disk_row_columns`;
3. create a child UI clipped to `row_rect`;
4. render mount and capacity using `.truncate()`;
5. right-align the capacity label with `.halign(egui::Align::RIGHT)`;
6. never advance the parent cursor.

Use this signature:

```rust
fn render_disk_row_content(
    ui: &mut egui::Ui,
    row_rect: egui::Rect,
    mount: &str,
    percent_text: &str,
    capacity: &str,
    percent_color: egui::Color32,
    text_color: egui::Color32,
)
```

Use the following widgets inside the computed rectangles:

```rust
row_ui.put(
    mount_rect,
    egui::Label::new(
        egui::RichText::new(mount)
            .size(SIDEBAR_BODY_SIZE)
            .color(text_color),
    )
    .truncate(),
);
row_ui.put(
    percent_rect,
    egui::Label::new(
        egui::RichText::new(percent_text)
            .size(SIDEBAR_BODY_SIZE)
            .color(percent_color),
    )
    .halign(egui::Align::RIGHT)
    .truncate(),
);
row_ui.put(
    capacity_rect,
    egui::Label::new(
        egui::RichText::new(capacity)
            .size(SIDEBAR_BODY_SIZE)
            .color(text_color),
    )
    .halign(egui::Align::RIGHT)
    .truncate(),
);
```

- [ ] **Step 5: Use the same geometry for the header and data rows**

Replace the adaptive disk `ui.horizontal` blocks with full-width allocated rows:

```rust
let (header_rect, _) = ui.allocate_exact_size(
    egui::vec2(ui.available_width(), DISK_ROW_HEIGHT),
    egui::Sense::hover(),
);
render_disk_row_content(
    ui,
    header_rect,
    "挂载点",
    "使用率",
    "可用/总量",
    label_color,
    label_color,
);

for (index, disk) in mon.disk_items.iter().enumerate() {
    let (row_rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), DISK_ROW_HEIGHT),
        egui::Sense::hover(),
    );
    if index % 2 == 1 {
        ui.painter().rect_filled(
            row_rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 4),
        );
    }
    let percent_color = if disk.percent > 90 {
        egui::Color32::from_rgb(0xf8, 0x51, 0x49)
    } else if disk.percent > 70 {
        egui::Color32::from_rgb(0xd2, 0x99, 0x22)
    } else {
        section_color
    };
    let percent_text = format!("{}%", disk.percent);
    let capacity_text = format!("{}/{}", disk.avail, disk.size);
    render_disk_row_content(
        ui,
        row_rect,
        &disk.mount,
        &percent_text,
        &capacity_text,
        percent_color,
        section_color,
    );
}
```

- [ ] **Step 6: Verify focused and full tests**

Run:

```bash
cargo fmt --manifest-path native-prototype/Cargo.toml --check
cargo test --manifest-path native-prototype/Cargo.toml disk_
cargo test --manifest-path native-prototype/Cargo.toml
cargo clippy --manifest-path native-prototype/Cargo.toml --all-targets
```

Expected: all disk geometry, rendering, capacity, and existing tests pass without a new warning attributable to this task.

Do not stage or commit `native-prototype/src/sidebar.rs`; it contains pre-existing uncommitted Native work.

### Task 4: Independent review and live Native verification

**Files:**
- Review: `native-prototype/src/sidebar.rs`
- Review: `native-prototype/src/monitor.rs`
- Verify: `native-prototype/target/debug/liteterm-native`

- [ ] **Step 1: Review specification compliance**

Confirm from production code that:

- each normal monitor frame has a 220px outer width;
- narrow widths never exceed `ui.available_width()`;
- all card types consume the same geometry;
- TiB formatting starts at exactly 1,099,511,627,776 bytes;
- disk header and data rows share bounded three-column geometry;
- Unicode mount points are truncated by egui rather than sliced by bytes.

- [ ] **Step 2: Review code quality**

Check finite-value handling, floating-point column sums, child UI clipping, parent cursor behavior, reuse of helpers, and absence of per-frame parsing or avoidable allocations.

- [ ] **Step 3: Build only Native and preserve the old GuiShell**

Run:

```bash
./native-prototype/build.sh
sha256sum build.sh run.sh
```

Expected: build, Clippy, and all Native tests pass; root script hashes remain:

```text
69e913332c3b14111a62f5843cf35dd6751f0f566dc941efc1218364350a2a5e  build.sh
f7d1b7b264548f8e1b968c781c0fa38529a92b24e2bd894b6dcea3b080b1ff6e  run.sh
```

- [ ] **Step 4: Restart only Native and capture the fixed layout**

Launch the new binary through `./run-native.sh`, without stopping `guishell-tauri`. In a maximized window verify:

1. uptime, resource, process, network, and disk cards reach the sidebar divider;
2. no 22px blank strip remains;
3. disk mount, usage, and capacity columns do not overlap;
4. capacities at or above 1 TiB use one-decimal `T`;
5. a 710px-wide window still clips long mount points without horizontal expansion.

Save a full-window screenshot and a cropped sidebar screenshot.
