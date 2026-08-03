# Native Completion Reliability and New-Tab Selector Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore reliable history completion in local and SSH Bash sessions, including after resize, and replace the Native new-tab selector with the approved main-faithful compact design.

**Architecture:** Bash remains the authority for Readline state. A token- and generation-authenticated OSC snapshot frame restores the current line after grid anchors are lost; one rendered popup snapshot becomes the sole keyboard-interaction gate. The selector remains an egui overlay and reuses the existing shell/SSH factories and SSH editor.

**Tech Stack:** Rust, egui/winit, Bash Readline `bind -x`, OSC 777, Alacritty terminal grid, portable-pty, ssh2/SFTP.

---

## File Map and Execution Layers

Layer 1 has disjoint ownership and may run in parallel:

- Task 1 owns `bash_integration.rs` and `ssh.rs`.
- Task 2 owns `smart_completion.rs`.
- Task 3 owns `new_tab_selector.rs`.

Layer 2 starts after Tasks 1–3:

- Task 4 owns `terminal.rs` and `completion_integration_tests.rs`.
- Task 5 owns `completion_popup.rs` and `main.rs`.

Layer 3 is verification and review only. Agents must preserve unrelated dirty-tree changes, must not run repository-wide formatting, and must not commit without explicit user approval.

### Task 1: Authenticated Readline Snapshot Protocol

**Files:**
- Modify: `native-prototype/src/bash_integration.rs`
- Modify: `native-prototype/src/ssh.rs`

- [ ] **Step 1: Add RED decoder and RC-script tests**

Add focused tests for an `I;<point>;<base64url-line>` frame, malformed point/payload, stale token/generation, bounded frames, distinct fill/snapshot sequences, and generated RC bindings:

```rust
#[test]
fn input_snapshot_marker_decodes_line_and_cursor() {
    let body = format!(
        "777;LiteTerm;{TOKEN};{GENERATION};I;3;{}",
        URL_SAFE_NO_PAD.encode("git status")
    );
    assert_eq!(
        decode_body(&body),
        Some(MarkerKind::InputSnapshot {
            line: "git status".into(),
            point: 3,
        })
    );
}

#[test]
fn snapshot_point_must_be_a_utf8_boundary() {
    assert_eq!(decode_snapshot("你好", 1), None);
}

#[test]
fn rc_installs_fill_and_snapshot_widgets_in_all_readline_maps() {
    let rc = build_bash_rc_for_test();
    for map in ["emacs-standard", "vi-insert", "vi-command"] {
        assert!(rc.contains(&format!("bind -m {map}")));
    }
    assert!(rc.contains("READLINE_LINE"));
    assert!(rc.contains("READLINE_POINT"));
    assert!(rc.contains(";I;%s;%s"));
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml bash_integration::tests -- --nocapture
```

Expected: the new tests fail because `InputSnapshot` and the snapshot widget/sequence do not exist.

- [ ] **Step 3: Implement the bounded protocol and runtime fields**

Use distinct private sequences and redact both from `Debug`:

```rust
pub const MAX_SNAPSHOT_INPUT_BYTES: usize = 8 * 1024;
pub const MAX_OSC_FRAME: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    Prompt,
    HistoryPath(String),
    InputSnapshot { line: String, point: usize },
}

pub fn snapshot_sequence(session: &CompletionSessionKey) -> String {
    let numeric = sequence_number(session);
    format!("\x1b[778;{numeric}~")
}
```

Parse `I` only after canonical Base64URL decoding, enforce `line.len() <= MAX_SNAPSHOT_INPUT_BYTES`, `point <= line.len()`, `line.is_char_boundary(point)`, and reject control characters. Extend `LocalBashRuntime` and `RemoteBashRuntime` with the snapshot sequence. The Bash widget must encode `READLINE_LINE`, emit `READLINE_POINT`, never mutate the line, and never emit `CR/LF`.

Update `ssh.rs` only where it builds the temporary RC and constructs `RemoteBashRuntime`; do not alter authentication or connection behavior.

- [ ] **Step 4: Run GREEN tests and format only owned files**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml bash_integration::tests -- --nocapture
cargo fmt --manifest-path native-prototype/Cargo.toml -- native-prototype/src/bash_integration.rs native-prototype/src/ssh.rs
cargo fmt --manifest-path native-prototype/Cargo.toml --check
```

Expected: protocol tests pass and rustfmt reports no changes required.

### Task 2: Explicit Fill and History State

**Files:**
- Modify: `native-prototype/src/smart_completion.rs`

- [ ] **Step 1: Add RED state-machine tests**

Add tests that prove editing cancels a request, stale callbacks cannot finish it, history failures are observable without exposing data, and empty successful history is ready:

```rust
#[test]
fn user_edit_cancels_the_exact_pending_fill() {
    let mut state = state();
    state.begin_fill(41);
    assert_eq!(state.on_user_edit(), Some(41));
    assert!(!state.finish_fill(41));
}

#[test]
fn empty_history_is_ready_not_error() {
    let mut state = state();
    state.mark_history_loading();
    state.apply_history_result(Ok(Vec::new()));
    assert_eq!(state.history_status(), &HistoryStatus::Ready { items: 0 });
}

#[test]
fn history_error_keeps_only_a_safe_chinese_reason() {
    let mut state = state();
    state.apply_history_result(Err("permission denied: secret".into()));
    assert!(matches!(state.history_status(), HistoryStatus::Error { .. }));
    assert!(!format!("{:?}", state.history_status()).contains("secret"));
}
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml smart_completion::tests -- --nocapture
```

Expected: compilation fails on the new state types and methods.

- [ ] **Step 3: Implement typed state**

Replace `Option<u64>` with a typed pending request and add a safe history status:

```rust
#[derive(Clone, PartialEq, Eq)]
struct PendingFill {
    request_id: u64,
    session: CompletionSessionKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryStatus {
    Disabled { reason: &'static str },
    Loading,
    Ready { items: usize },
    Error { reason: &'static str },
}
```

`begin_fill` captures the current session. `on_user_edit` clears and returns the cancelled request ID. `finish_fill`, `fail_fill`, and `pending_fill_matches` require the same ID and current session. History APIs map raw I/O failures to fixed Chinese categories such as `历史文件不可读`; raw paths, candidate text and OS error strings must not enter `Debug`.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml smart_completion::tests -- --nocapture
cargo fmt --manifest-path native-prototype/Cargo.toml -- native-prototype/src/smart_completion.rs
```

Expected: all smart-completion unit tests pass.

### Task 3: Main-Faithful New-Tab Selector

**Files:**
- Modify: `native-prototype/src/new_tab_selector.rs`

- [ ] **Step 1: Add RED presentation and action tests**

Test basename-only shell labels, stable SSH grouping, responsive width, content-capped height, and the new SSH-editor action:

```rust
#[test]
fn shell_pills_show_only_the_basename() {
    assert_eq!(shell_label(Path::new("/usr/bin/bash")), "bash");
}

#[test]
fn ssh_rows_keep_first_seen_group_order() {
    let groups = group_connections(&[connection("a", "work"), connection("b", "default")]);
    assert_eq!(groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["work", "default"]);
}

#[test]
fn new_ssh_action_is_not_logged_with_connection_secrets() {
    assert_eq!(format!("{:?}", NewTabAction::NewSsh), "NewSsh");
}
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml new_tab_selector::tests -- --nocapture
```

Expected: the new helpers and `NewSsh` action are missing.

- [ ] **Step 3: Implement the approved egui presentation**

Add `NewTabAction::NewSsh` and presentation-only grouping types. Render:

```text
520px centered card, max 80vh, 16px viewport margin
header: “新建标签页” + borderless gray ×
$ Shell 环境: wrapped basename pills
@ SSH 连接: group color dot + group name + transparent host rows
+ 新建 SSH 连接
~ 串口设备: weak “将在 P1 实现” state
```

Use explicit colors `#161b22`, `#30363d`, `#0d1117`, `#21262d`, cyan `#00d4ff`, green `#00ff9f`, yellow `#f1fa8c`; use 60% black backdrop, 8px radius and shadow. Do not use default full-width egui buttons, section separators, full shell paths, usernames, search fields or selector tabs. Keep Escape, backdrop close, modal click capture, stale SSH key validation, and deferred action behavior.

- [ ] **Step 4: Verify GREEN and rendering invariants**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml new_tab_selector::tests -- --nocapture
cargo fmt --manifest-path native-prototype/Cargo.toml -- native-prototype/src/new_tab_selector.rs
git diff --check -- native-prototype/src/new_tab_selector.rs
```

Expected: selector tests pass; no whitespace errors.

### Task 4: Terminal Snapshot Lifecycle

**Files:**
- Modify: `native-prototype/src/terminal.rs`
- Modify: `native-prototype/src/completion_integration_tests.rs`

- [ ] **Step 1: Add RED terminal lifecycle tests**

Cover resize recovery, snapshot authentication, prompt invalidation and non-executing request sequences:

```rust
#[test]
fn resize_requests_snapshot_and_restores_the_prefix() {
    let mut harness = authenticated_harness("git");
    harness.resize(81, 24);
    assert_eq!(harness.input(), None);
    assert_eq!(harness.take_writer_bytes(), snapshot_sequence().as_bytes());
    harness.feed(&input_snapshot_marker("git", 3));
    assert_eq!(harness.input().as_deref(), Some("git"));
}

#[test]
fn stale_snapshot_does_not_restore_input() {
    let mut harness = authenticated_harness("ls");
    harness.resize(81, 24);
    harness.feed(&snapshot_marker_for_generation(999, "ls", 2));
    assert_eq!(harness.input(), None);
}

#[test]
fn snapshot_request_contains_no_execute_byte() {
    assert!(!snapshot_request().iter().any(|b| matches!(b, b'\r' | b'\n')));
}
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml terminal::tests -- --nocapture
cargo test --manifest-path native-prototype/Cargo.toml completion_integration_tests -- --nocapture
```

Expected: resize still permanently clears the anchor and no input-snapshot event is handled.

- [ ] **Step 3: Implement prompt-active recovery**

Extend tracking without replacing grid extraction:

```rust
struct SnapshotBase {
    prefix: String,
    anchor: LogicalPoint,
}

struct PromptTracking {
    session: CompletionSessionKey,
    decoder: MarkerDecoder,
    anchor: Option<LogicalPoint>,
    snapshot_base: Option<SnapshotBase>,
    prompt_active: bool,
    snapshot_requested_at: Option<Instant>,
}
```

`MarkerKind::Prompt` sets an empty grid anchor and clears old snapshot state. `InputSnapshot` stores `line[..point]` as the base and the current logical cursor as its anchor. `current_bash_input` returns `snapshot_base.prefix + grid_delta` when a base exists. Submission, shell switch, alternate screen, shutdown and unknown control input set `prompt_active = false`.

On a real resize, preserve whether an authenticated prompt was active, invalidate only geometry, resize the PTY, and enqueue the runtime snapshot sequence once. A request can retry only after its short timeout; a valid response clears the pending timestamp. Never send a request without an active authenticated Bash prompt.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml terminal::tests -- --nocapture
cargo test --manifest-path native-prototype/Cargo.toml completion_integration_tests -- --nocapture
cargo fmt --manifest-path native-prototype/Cargo.toml -- native-prototype/src/terminal.rs native-prototype/src/completion_integration_tests.rs
```

Expected: resize recovery, stale-frame and submission tests pass.

### Task 5: Rendered Popup as the Only Interaction Gate

**Files:**
- Modify: `native-prototype/src/completion_popup.rs`
- Modify: `native-prototype/src/main.rs`

- [ ] **Step 1: Add RED popup/key/event tests**

Add a value object that rejects blocked, offscreen, empty and stale candidates:

```rust
#[test]
fn offscreen_cursor_cannot_create_an_interactive_snapshot() {
    assert!(CompletionPopupSnapshot::new(
        "tab-a".into(),
        session(),
        bounds(),
        Some(offscreen_cursor()),
        vec!["ls -al".into()],
        0,
        false,
    ).is_none());
}

#[test]
fn space_and_tab_never_accept_a_visible_candidate() {
    assert_eq!(route(Key::Character(" ".into()), true), CompletionKeyAction::PassThrough);
    assert_eq!(route(Key::Named(NamedKey::Tab), true), CompletionKeyAction::PassThrough);
}

#[test]
fn invisible_candidate_enter_passes_to_bash() {
    assert_eq!(route(Key::Named(NamedKey::Enter), false), CompletionKeyAction::PassThrough);
}
```

Add tests that a user edit cancels pending fill before a write-complete event, while a matching completion callback commits only the widget sequence.

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml completion_popup::tests -- --nocapture
cargo test --manifest-path native-prototype/Cargo.toml --bin liteterm-native completion_ -- --nocapture
```

Expected: key routing still uses logical candidate visibility and the snapshot type does not exist.

- [ ] **Step 3: Implement and persist the rendered snapshot**

Create:

```rust
#[derive(Clone)]
pub struct CompletionPopupSnapshot {
    pub tab_id: String,
    pub session: CompletionSessionKey,
    pub bounds: egui::Rect,
    pub cursor: egui::Rect,
    pub candidates: Vec<String>,
    pub selected: usize,
}
```

Its constructor returns `None` for blocked UI, empty candidates, missing/non-finite cursor, or a cursor outside the terminal viewport. `render` accepts `&CompletionPopupSnapshot`.

Add `completion_popup_snapshot: Option<CompletionPopupSnapshot>` to the app. `do_render` computes it once from the active tab, current completion session, renderer and terminal cursor; both drawing and later keyboard routing consume that same snapshot. Before handling arrows/Enter, verify active tab ID and session still match. Remove logical `CompletionState::is_popup_visible()` from key routing.

Map history events to Task 2’s typed status and log only the safe status category. Preserve empty-success history. Add `NewTabAction::NewSsh` handling:

```rust
NewTabAction::NewSsh => {
    self.new_tab_selector.close();
    self.sidebar.show_new_connection = true;
    self.sidebar.new_conn = sidebar::NewConnForm::default();
    true
}
```

Do not change sidebar dialog implementation, shell/SSH factories, Tab behavior, Fish/Zsh behavior, or Tauri sources.

- [ ] **Step 4: Run integration GREEN tests**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml completion_popup::tests -- --nocapture
cargo test --manifest-path native-prototype/Cargo.toml --bin liteterm-native completion_ -- --nocapture
cargo test --manifest-path native-prototype/Cargo.toml completion_integration_tests -- --nocapture
cargo fmt --manifest-path native-prototype/Cargo.toml -- native-prototype/src/completion_popup.rs native-prototype/src/main.rs
```

Expected: invisible popup keys pass through, Space/Tab never fill, matching Enter fills without execution, and `NewSsh` opens the existing editor.

### Task 6: Full Verification and Review

**Files:**
- Modify only if an earlier test exposes an in-scope defect.
- Record review: `.ccg/tasks/fix-completion-redesign-new-tab/review.md`

- [ ] **Step 1: Run the Native test suite**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml
cargo fmt --manifest-path native-prototype/Cargo.toml --check
cargo clippy --manifest-path native-prototype/Cargo.toml --all-targets -- -D warnings
git diff --check
```

Expected: all commands exit 0.

- [ ] **Step 2: Build through the isolated Native script**

Run:

```bash
native-prototype/build.sh
```

Expected: the Native binary builds successfully without replacing the legacy guishell binary or root scripts.

- [ ] **Step 3: Perform staged reviews**

First review spec compliance against:

```text
docs/superpowers/specs/2026-07-27-native-completion-and-new-tab-design.md
```

Then review code quality, security of OSC decoding, async request cancellation, SSH fallback, egui layout, and dirty-tree scope. Any Critical or Warning finding is fixed and re-reviewed. Record the combined result and unavailable external-model diagnostics in `review.md`.

- [ ] **Step 4: Prepare manual acceptance checklist**

Verify on the desktop:

1. Local Bash and SSH Bash show candidates for a non-empty history prefix.
2. Resize while editing; candidates recover without pressing Enter.
3. Arrow keys select only a visible popup.
4. First Enter fills, Space never accepts, second Enter executes.
5. Fish/Zsh retain native key behavior.
6. The plus button shows the approved 520px compact selector.
7. Shell pills, grouped SSH rows, `+ 新建 SSH 连接`, disabled serial, backdrop, close and narrow-window behavior match the design.

Do not archive the CCG task or commit until the user reports that desktop acceptance passed.

### Task 7: Main-Style Per-Command Input Lifecycle

**Files:**
- Modify: `native-prototype/src/smart_completion.rs`
- Modify: `native-prototype/src/terminal.rs`
- Modify: `native-prototype/src/main.rs`
- Modify: `native-prototype/src/completion_integration_tests.rs`

This regression task adapts the proven `main:src/components/Terminal/TerminalPane.tsx`
`currentLineRef` lifecycle without replacing the authenticated Readline snapshot protocol.
The direct input tracker is the immediate source for ordinary typing; authenticated prompt
state remains the gate, and terminal grid/snapshot extraction remains the fallback after
cursor-moving or otherwise ambiguous Readline edits.

- [ ] **Step 1: Add RED input-cycle tests**

Add focused tests proving that two consecutive Bash commands can independently produce
candidates, even when terminal-grid reconstruction is unavailable for the first submission:

```rust
#[test]
fn consecutive_submissions_reset_the_input_cycle_and_popup_suppression() {
    let mut state = ready_state(&["ls -al", "free -h"]);
    state.observe_user_input("ls");
    assert_eq!(state.tracked_input(), Some("ls"));
    state.dismiss();
    assert_eq!(state.take_tracked_submission(), Some("ls".into()));
    state.finish_submission();
    state.observe_user_input("fr");
    state.refresh(state.tracked_input().unwrap());
    assert!(state.is_popup_visible());
}

#[test]
fn ambiguous_readline_edit_falls_back_to_authenticated_terminal_input() {
    let mut state = ready_state(&["git status"]);
    state.observe_user_input("gi");
    state.observe_user_input("\x1b[D");
    assert_eq!(state.tracked_input(), None);
    assert_eq!(completion_prefix(true, state.tracked_input(), Some("git")), Some("git"));
}
```

Also extend the terminal prompt lifecycle test with `prompt → submit → command output →
next prompt → input`, and assert the second prompt is active.

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml smart_completion::tests -- --nocapture
cargo test --manifest-path native-prototype/Cargo.toml completion_ -- --nocapture
```

Expected: compilation fails because the per-command tracker and prompt-gated prefix
selection do not exist.

- [ ] **Step 3: Implement the minimal main-style adapter**

Store `tracked_input: Option<String>` in `CompletionState`. `Some` is an exact line for
ordinary printable text, UTF-8 backspace and Ctrl+U; `None` means a cursor-moving,
history-navigation, Tab, paste-with-newline, or unknown control sequence made the mirror
ambiguous. Submission atomically takes the tracked line, clears candidates, pending fill
and suppression, and starts the next cycle at `Some("")`.

Expose only a boolean authenticated-prompt query from `TerminalState`. In `do_render`,
choose the completion prefix with this priority:

```text
authenticated prompt is inactive -> no prefix
exact tracked input exists         -> tracked input
tracker is ambiguous               -> terminal grid / Readline snapshot input
```

`write_active_user_input` updates the tracker before writing to the PTY.
`submit_active_bash_line` records the tracked submission first and falls back to
`TerminalState::take_bash_submission`; it must reset popup suppression even when grid
reconstruction returned `None`. A successfully filled candidate becomes the tracked line
without sending Enter.

- [ ] **Step 4: Verify GREEN and regressions**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml smart_completion::tests -- --nocapture
cargo test --manifest-path native-prototype/Cargo.toml completion_ -- --nocapture
cargo test --manifest-path native-prototype/Cargo.toml terminal::tests -- --nocapture
cargo test --manifest-path native-prototype/Cargo.toml completion_integration_tests -- --nocapture
cargo fmt --manifest-path native-prototype/Cargo.toml --check
git diff --check
```

Expected: consecutive-command tests pass; existing resize, stale-snapshot, popup-gating,
Space/Tab and first-Enter-fill tests remain green.

### Task 8: Strict Prefix Matching with Five Visible Candidates

**Files:**
- Modify: `native-prototype/src/smart_completion.rs`

- [x] **Step 1: Replace the old ranking test with a RED regression**

Use a history containing both `fish ...` commands and `strace ... liteterm-fish-*`
commands. Assert that ranking excludes the exact input and all middle-substring matches,
preserves first-seen history order, removes duplicates, and returns exactly the first five
strict-prefix candidates:

```rust
#[test]
fn ranking_is_strict_prefix_recent_first_deduplicated_and_capped_at_five() {
    let history = vec![
        "strace -o /tmp/liteterm-fish-child.strace fish".into(),
        "fish --help".into(),
        "fish -c one".into(),
        "fish -c two".into(),
        "fish -c three".into(),
        "fish -c four".into(),
        "fish -c five".into(),
        "fish -c six".into(),
        "fish --help".into(),
        "fish".into(),
    ];
    assert_eq!(
        rank_candidates(&history, "fish"),
        ["fish --help", "fish -c one", "fish -c two", "fish -c three", "fish -c four"]
    );
}
```

- [x] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml \
  smart_completion::tests::ranking_is_strict_prefix_recent_first_deduplicated_and_capped_at_five \
  -- --exact --nocapture
```

Expected: FAIL because the existing implementation returns eight candidates and appends
the `strace` substring match.

- [x] **Step 3: Implement the minimal ranking change**

Set `MAX_CANDIDATES` to `5`. In `rank_candidates`, keep only non-identical commands for
which `command.starts_with(prefix)`, deduplicate in input history order, and stop after
five. Do not change popup geometry or input lifecycle code.

- [x] **Step 4: Verify focused and full Native checks**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml smart_completion::tests -- --nocapture
./native-prototype/build.sh
git diff --check
```

Expected: all smart-completion tests and the full Native build pass. `run-native.sh`
continues to launch `native-prototype/target/debug/liteterm-native`, so the next launch
uses the rebuilt strict-prefix implementation.

### Task 9: Tab Accepts Completion and Enter Submits Input

**Files:**
- Modify: `native-prototype/src/main.rs`

- [x] **Step 1: Add RED keyboard-routing tests**

Update the completion keyboard tests to require:

```rust
assert_eq!(
    completion_key_action(
        &Key::Named(NamedKey::Tab),
        ModifiersState::empty(),
        true,
        false,
        false,
        false,
        false,
    ),
    CompletionKeyAction::Accept,
);
assert_eq!(
    completion_key_action(
        &Key::Named(NamedKey::Enter),
        ModifiersState::empty(),
        true,
        false,
        false,
        false,
        false,
    ),
    CompletionKeyAction::PassThrough,
);
```

Also assert that Tab without a visible popup and modified Tab pass through, while Enter
during an already pending Tab fill remains `WaitForFill`.

- [x] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml \
  layout_tests::tab_accepts_visible_completion_while_enter_submits_current_input \
  -- --exact --nocapture
```

Expected: FAIL because the old mapping accepts Enter and passes Tab through.

- [x] **Step 3: Implement the minimal key mapping**

In `completion_key_action`, map an unmodified `NamedKey::Tab` to `Accept` only when the
rendered popup snapshot is visible. Leave `NamedKey::Enter` as `PassThrough`, so the
existing terminal path calls `submit_active_bash_line`. Preserve `WaitForFill` for Enter
after Tab has already started an asynchronous fill. Do not change fill transport,
candidate ranking, popup rendering, or Bash key encoding.

- [x] **Step 4: Verify focused and full Native checks**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml completion_ -- --nocapture
./native-prototype/build.sh
git diff --check
```

Expected: completion tests and the full Native build pass; the rebuilt debug binary keeps
native Bash Tab behavior when no popup exists and uses Tab to fill a visible candidate.
