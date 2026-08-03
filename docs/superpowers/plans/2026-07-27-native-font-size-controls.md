# Native Font Size Controls Implementation Plan

**Goal:** Set the Native terminal default font size to 22px and add linked decrement and
increment controls around the existing font-size slider.

**Architecture:** Keep `SettingsDraft::font_size` as the only UI state. The default
constant remains the single source for settings deserialization, sanitization fallback,
and renderer bootstrap; button stepping is a small bounded pure helper used by the egui
controls.

**Tech Stack:** Rust, serde, egui, cargo test.

---

### Task 1: Default Font Size

**Files:**
- Modify: `native-prototype/src/settings.rs`
- Modify: `native-prototype/TODO.md`

- [x] Add a failing test requiring `DEFAULT_TERMINAL_FONT_SIZE == 22.0` and missing
  `font_size` deserialization to use 22px.
- [x] Run `cargo test --manifest-path native-prototype/Cargo.toml settings::tests -- --nocapture`
  and verify the old 15px default fails.
- [x] Change the shared default to 22px and update stale test names, messages, comments,
  and the TODO description.
- [x] Re-run the settings tests and verify they pass.

### Task 2: Linked Minus, Slider, Plus, and Numeric Controls

**Files:**
- Modify: `native-prototype/src/settings_panel.rs`

- [x] Add failing pure tests for one-pixel stepping and clamping at 8px and 48px.
- [x] Run `cargo test --manifest-path native-prototype/Cargo.toml settings_panel::tests -- --nocapture`
  and verify the missing helper fails.
- [x] Add a bounded font-size step helper and render `−`, the existing slider, `+`, and
  the numeric drag control in that order, all bound to `draft.font_size`.
- [x] Re-run panel tests, `native-prototype/build.sh`, and `git diff --check`.

### Task 3: Review and Manual Acceptance

- [x] Review default compatibility, button boundary states, single-source UI state, and
  renderer bootstrap behavior.
- [ ] Restart with `run-native.sh`; verify the fresh/default 22px terminal and linked
  settings controls without altering an explicitly saved size.
