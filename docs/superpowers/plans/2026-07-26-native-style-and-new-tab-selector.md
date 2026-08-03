# Native Style Recovery and New-Tab Selector Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development` and keep file ownership non-overlapping.

**Goal:** Restore the Native terminal's original 15 px AdventureTime appearance without changing legacy guishell settings, and make both the tab-bar `+` and the configured new-tab shortcut open a Shell/SSH selector.

**Scope:** Only `feat/memory-optimize` and `native-prototype/`. Do not edit the legacy `~/.config/guishell/settings.toml`, the Tauri implementation, or `main`.

## Task 1: Isolate Native Settings and Restore Defaults

**Files:**

- Modify: `native-prototype/src/settings.rs`
- Modify: `native-prototype/src/renderer.rs`

- [x] Add tests proving a fresh Native configuration defaults to font size `15.0` and `AdventureTime`.
- [x] Add a path test proving Native persists to `~/.config/guishell/native-settings.toml`.
- [x] Change every Native fallback/default font size from `26.0` to `15.0`.
- [x] Change the renderer bootstrap size to `15.0` so the first frame does not flash at 26 px.
- [x] Run focused settings and renderer tests.

## Task 2: Build the Selector Module

**Files:**

- Create: `native-prototype/src/new_tab_selector.rs`

- [x] Add failing tests for `/etc/shells` parsing: ignore comments/blanks, require executable files, deduplicate while preserving order.
- [x] Define explicit actions for no-op, close, local shell, and saved SSH selection.
- [x] Render a compact Chinese modal with “本地终端”, “SSH 连接”, and a disabled “串口终端将在 P1 实现”.
- [x] Use a stable SSH identity rather than a display label alone.
- [x] Close on Escape or backdrop click and constrain the panel to the viewport.
- [x] Run focused selector tests.

## Task 3: Integrate Selector Routing

**Files:**

- Modify: `native-prototype/src/main.rs`
- Modify only if required: `native-prototype/src/tab_bar.rs`

- [x] Register and store selector state in `App`.
- [x] Route `TabBarAction::NewTab` and the configured new-tab shortcut to `selector.open()`.
- [x] Route shell selection through `new_local_tab_with_shell` and SSH selection through `new_ssh_tab`.
- [x] Include the modal in pointer, keyboard, terminal-input, and IME ownership gates.
- [x] Preserve startup-tab and duplicate-tab behavior.
- [x] Add routing tests where practical.

## Task 4: Review and Verification

- [x] Run file-scoped `rustfmt --check` without formatting unrelated files.
- [x] Run the Native full test/build pipeline.
- [x] Inspect the scoped diff and perform Grok + Codex review.
- [x] Record results in `.ccg/tasks/implement-p0-todos/review.md`.
- [x] Build with `native-prototype/build.sh`; do not use the legacy `build.sh`.
