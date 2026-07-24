# Native Terminal Click Selection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the white single-cell highlight created by a plain terminal click while preserving drag selection, double-click word selection, triple-click line selection, and terminal mouse reporting.

**Architecture:** Keep the renderer unchanged and correct the event source in `App`. A plain press records a private drag anchor without creating a visible selection; cursor movement across a cell boundary converts that anchor into the existing `selection_start`/`selection_end` range.

**Tech Stack:** Rust, winit 0.30, alacritty-terminal, wgpu, built-in Rust test framework

---

### Task 1: Add a testable drag-range transition

**Files:**
- Modify: `native-prototype/src/main.rs:64-84`
- Test: `native-prototype/src/main.rs:2147-2200`

- [ ] **Step 1: Write failing transition tests**

Add `drag_selection_range` to the `layout_tests` imports and add:

```rust
#[test]
fn plain_click_anchor_is_not_a_visible_selection() {
    assert_eq!(drag_selection_range(Some((4, 2)), (4, 2)), None);
}

#[test]
fn dragging_across_cells_creates_a_visible_selection() {
    assert_eq!(
        drag_selection_range(Some((4, 2)), (7, 2)),
        Some(((4, 2), (7, 2)))
    );
}

#[test]
fn moving_without_a_pressed_anchor_does_not_create_selection() {
    assert_eq!(drag_selection_range(None, (7, 2)), None);
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
cd native-prototype
cargo test plain_click_anchor_is_not_a_visible_selection
```

Expected: compilation fails with unresolved import or missing function `drag_selection_range`.

- [ ] **Step 3: Add the minimal pure transition**

Near `point_in_terminal_bounds`, add:

```rust
fn drag_selection_range(
    anchor: Option<(usize, usize)>,
    current: (usize, usize),
) -> Option<((usize, usize), (usize, usize))> {
    anchor
        .filter(|start| *start != current)
        .map(|start| (start, current))
}
```

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run:

```bash
cd native-prototype
cargo test plain_click_anchor_is_not_a_visible_selection
cargo test dragging_across_cells_creates_a_visible_selection
cargo test moving_without_a_pressed_anchor_does_not_create_selection
```

Expected: each command runs one matching test and passes with zero failures.

### Task 2: Route terminal mouse gestures through the drag anchor

**Files:**
- Modify: `native-prototype/src/main.rs:125-205`
- Modify: `native-prototype/src/main.rs:1687-1810`
- Modify: `native-prototype/src/main.rs:940-960,1955-1970`

- [ ] **Step 1: Store the pending drag anchor**

Add this field beside the existing selection fields:

```rust
selection_drag_anchor: Option<(usize, usize)>,
```

Initialize it to `None` in `App::new`.

- [ ] **Step 2: Make a plain press create only a pending gesture**

In both branches that start a new `ClickState::Single`, replace the single-cell selection assignment with:

```rust
self.selection_start = None;
self.selection_end = None;
self.selection_drag_anchor = Some(cell);
```

Before `select_word` and `select_line`, clear `selection_drag_anchor`. Also clear it when forwarding mouse events to an application in terminal mouse mode.

- [ ] **Step 3: Create a visible selection only after crossing cells**

In `WindowEvent::CursorMoved`, replace the direct `selection_end` update with:

```rust
let range = drag_selection_range(self.selection_drag_anchor, cell);
match range {
    Some((start, end)) => {
        self.selection_start = Some(start);
        self.selection_end = Some(end);
    }
    None => {
        self.selection_start = None;
        self.selection_end = None;
    }
}
```

This also removes the selection if the pointer returns to its original cell.

- [ ] **Step 4: Finish the gesture without copying a plain click**

On left-button release:

```rust
self.mouse_pressed = false;
self.selection_drag_anchor = None;
if self.selection_start.is_some() && self.selection_end.is_some() {
    self.copy_selection();
}
```

Clear `selection_drag_anchor` anywhere the existing code clears both selection endpoints, including tab switches and terminal keyboard input.

- [ ] **Step 5: Run formatting and the full Native test suite**

Run:

```bash
cargo fmt --manifest-path native-prototype/Cargo.toml --check
./native-prototype/build.sh
```

Expected: formatting, build, Clippy command, and all Native tests exit successfully. Existing warnings may remain, but no new warning should reference the new helper or field.

Do not stage or commit `native-prototype/src/main.rs`; the file already contains unrelated in-progress Native work.

### Task 3: Review and X11 interaction verification

**Files:**
- Review: `native-prototype/src/main.rs`
- Verify: `native-prototype/target/debug/liteterm-native`

- [ ] **Step 1: Run independent specification review**

Check the actual diff against the design:

- plain click creates no visible selection and performs no copy;
- drag selection starts only after crossing a cell;
- double-click and triple-click retain their existing paths;
- terminal mouse mode and `Shift` override remain unchanged;
- renderer selection behavior is untouched.

- [ ] **Step 2: Run independent code-quality review**

Review state cleanup on release, tab switch, keyboard input, and terminal mouse mode. Reject duplicated selection rules or rendering-specific workarounds.

- [ ] **Step 3: Rebuild and launch only Native**

Run:

```bash
./native-prototype/build.sh
./run-native.sh
```

Keep the old `guishell-tauri` process running. Confirm root `build.sh` and `run.sh` hashes are unchanged.

- [ ] **Step 4: Verify interaction behavior**

In the local terminal:

1. Click several blank and text cells; no white block remains at any clicked position.
2. Press and move inside one character cell; no selection appears.
3. Drag across several cells; the range highlights and copies on release.
4. Drag away and return to the anchor before release; no selection remains and nothing is copied.
5. Double-click a word and a one-character token; both highlight.
6. Triple-click a line; the line highlights.
7. In an application with mouse reporting enabled, verify clicks are still forwarded; hold `Shift` to verify local selection.

Capture screenshots for plain click, drag selection, double-click, and triple-click.
