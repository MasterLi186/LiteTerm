# Native File Create and Delete Confirmation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add visible context-menu hover feedback, deletion confirmation, and current-directory file/directory creation to both Native file panes.

**Architecture:** `file_browser.rs` owns modal state, validates names, and emits typed actions. `main.rs` forwards confirmed actions to the existing per-tab SFTP worker. `sftp.rs` performs collision-safe local or remote creation and reports through the existing mutation event so only the affected pane refreshes.

**Tech Stack:** Rust, egui 0.31, ssh2 SFTP, tempfile tests

---

### Task 1: Make context-menu hover visible

**Files:**
- Modify: `native-prototype/src/file_browser.rs:574-643`
- Test: `native-prototype/src/file_browser.rs` `ui_tests`

- [ ] **Step 1: Write the failing hover-style test**

Add a pure style assertion:

```rust
#[test]
fn enabled_context_item_has_visible_hover_fill() {
    assert_eq!(context_item_fill(false, true), egui::Color32::TRANSPARENT);
    assert_ne!(context_item_fill(true, true), egui::Color32::TRANSPARENT);
    assert_eq!(context_item_fill(true, false), egui::Color32::TRANSPARENT);
}
```

- [ ] **Step 2: Verify RED**

Run in `native-prototype/`:

```bash
cargo test enabled_context_item_has_visible_hover_fill -- --nocapture
```

Expected: compile failure because `context_item_fill` does not exist.

- [ ] **Step 3: Implement the custom full-row menu item**

Add:

```rust
fn context_item_fill(hovered: bool, enabled: bool) -> egui::Color32 {
    if hovered && enabled {
        egui::Color32::from_rgb(0x30, 0x36, 0x3d)
    } else {
        egui::Color32::TRANSPARENT
    }
}
```

Replace the frameless `Button` in `render_context_menu` with an exact `152x22` click region. Paint `context_item_fill(response.hovered(), item.enabled)` across the full rectangle, then paint left-aligned text. Use `RED` for the delete command, `TEXT` for other enabled items, and `DIM` for disabled items. Only enabled responses may select a command.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo test enabled_context_item_has_visible_hover_fill -- --nocapture
cargo test file_browser::ui_tests
```

Expected: both commands exit 0.

### Task 2: Gate deletion behind a confirmation dialog

**Files:**
- Modify: `native-prototype/src/file_browser.rs:45-290`
- Modify: `native-prototype/src/file_browser.rs:574-760`
- Test: `native-prototype/src/file_browser.rs` `ui_tests`

- [ ] **Step 1: Write failing delete-state tests**

Change the existing delete-context test to require a dialog outcome and add a confirmation-action test:

```rust
#[test]
fn delete_context_action_opens_confirmation() {
    let menu = file_context_menu(
        FileSide::Remote,
        entry("old", true),
        "/srv",
        "/tmp",
        egui::Pos2::ZERO,
    );
    let ContextOutcome::Delete(dialog) =
        context_action(&menu, ContextCommand::Delete)
    else {
        panic!("expected delete confirmation");
    };
    assert_eq!(dialog.path, "/tmp/old");
    assert_eq!(dialog.name, "old");
    assert!(dialog.is_dir);
}

#[test]
fn confirmed_delete_targets_the_original_entry() {
    let dialog = DeleteDialogState {
        side: FileSide::Local,
        name: "cache".into(),
        path: "/tmp/cache".into(),
        is_dir: true,
        just_opened: false,
    };
    assert_eq!(
        delete_action(&dialog),
        FileBrowserAction::Delete {
            side: FileSide::Local,
            path: "/tmp/cache".into(),
            is_dir: true,
        }
    );
}
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test delete_context_action_opens_confirmation -- --nocapture
cargo test confirmed_delete_targets_the_original_entry -- --nocapture
```

Expected: compile failure because `ContextOutcome::Delete`, `DeleteDialogState`, and `delete_action` are absent.

- [ ] **Step 3: Add delete modal state and rendering**

Add:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
struct DeleteDialogState {
    side: FileSide,
    name: String,
    path: String,
    is_dir: bool,
    just_opened: bool,
}
```

Add `delete_dialog: Option<DeleteDialogState>` to `FileBrowserState`, add `Delete(DeleteDialogState)` to `ContextOutcome`, and make `ContextCommand::Delete` return this state instead of a `FileBrowserAction`.

Implement:

```rust
fn delete_action(dialog: &DeleteDialogState) -> FileBrowserAction {
    FileBrowserAction::Delete {
        side: dialog.side,
        path: dialog.path.clone(),
        is_dir: dialog.is_dir,
    }
}
```

Render a centered `egui::Window` titled `确认删除`, with the target name and full path, `取消`, and a red `删除` button. Escape, backdrop click, and Cancel close without an action. Only the red button pushes `delete_action`.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo test delete_context_action_opens_confirmation -- --nocapture
cargo test confirmed_delete_targets_the_original_entry -- --nocapture
cargo test file_browser::ui_tests
```

Expected: all commands exit 0 and the old immediate-delete expectation is gone.

### Task 3: Add pane creation controls and validated actions

**Files:**
- Modify: `native-prototype/src/file_browser.rs:1-110`
- Modify: `native-prototype/src/file_browser.rs:430-530`
- Modify: `native-prototype/src/file_browser.rs:640-760`
- Test: `native-prototype/src/file_browser.rs` `ui_tests`

- [ ] **Step 1: Write failing creation tests**

Add tests for validation, path construction, and visible controls:

```rust
#[test]
fn create_action_rejects_invalid_names_and_joins_current_path() {
    for name in ["", " ", ".", "..", "a/b", r"a\b"] {
        assert!(create_action(FileSide::Local, "/tmp", CreateKind::File, name).is_none());
    }
    assert_eq!(
        create_action(FileSide::Remote, "/srv", CreateKind::Directory, "release"),
        Some(FileBrowserAction::Create {
            side: FileSide::Remote,
            path: "/srv/release".into(),
            kind: CreateKind::Directory,
        })
    );
}

#[test]
fn dual_panes_render_create_buttons() {
    let painted_text = render_text(FileBrowserState::new("/tmp".into()));
    assert_eq!(painted_text.iter().filter(|text| *text == "＋文件").count(), 2);
    assert_eq!(painted_text.iter().filter(|text| *text == "＋目录").count(), 2);
}
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test create_action_rejects_invalid_names_and_joins_current_path -- --nocapture
cargo test dual_panes_render_create_buttons -- --nocapture
```

Expected: compile failure because `CreateKind`, `create_action`, and the controls are absent.

- [ ] **Step 3: Add creation state, action, and buttons**

Define `CreateKind` in `sftp.rs` and import it into `file_browser.rs`. Add:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
struct CreateDialogState {
    side: FileSide,
    parent_path: String,
    kind: CreateKind,
    value: String,
    request_focus: bool,
}
```

Extend `FileBrowserAction`:

```rust
Create {
    side: FileSide,
    path: String,
    kind: CreateKind,
},
```

Implement:

```rust
fn create_action(
    side: FileSide,
    parent_path: &str,
    kind: CreateKind,
    value: &str,
) -> Option<FileBrowserAction> {
    let name = value.trim();
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.contains('/')
        || name.contains('\\')
    {
        return None;
    }
    Some(FileBrowserAction::Create {
        side,
        path: crate::sftp::join_path(parent_path, name),
        kind,
    })
}
```

Add compact `＋文件` and `＋目录` buttons in each pane title bar. Pass an explicit `create_enabled` flag: true locally and `state.ready` remotely. A click opens `CreateDialogState` for that pane’s current `pane.path`.

Render the centered creation dialog with automatic focus, Enter submission, Escape/backdrop cancellation, and disabled confirmation for invalid names.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo test create_action_rejects_invalid_names_and_joins_current_path -- --nocapture
cargo test dual_panes_render_create_buttons -- --nocapture
cargo test file_browser::ui_tests
```

Expected: all commands exit 0.

### Task 4: Implement collision-safe creation in the worker

**Files:**
- Modify: `native-prototype/src/sftp.rs:20-105`
- Modify: `native-prototype/src/sftp.rs:280-505`
- Modify: `native-prototype/src/sftp.rs:735-830`
- Test: `native-prototype/src/sftp.rs` `tests`

- [ ] **Step 1: Write failing local creation tests**

Add:

```rust
#[test]
fn local_create_makes_empty_file_and_directory_without_overwriting() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("new.txt");
    let directory = temp.path().join("new-dir");

    create_local(&file, CreateKind::File).unwrap();
    create_local(&directory, CreateKind::Directory).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"");
    assert!(directory.is_dir());
    assert!(create_local(&file, CreateKind::File).is_err());
    assert!(create_local(&directory, CreateKind::Directory).is_err());
}

#[test]
fn remote_file_creation_flags_are_exclusive() {
    let flags = remote_file_create_flags();
    assert!(flags.contains(ssh2::OpenFlags::WRITE));
    assert!(flags.contains(ssh2::OpenFlags::CREATE));
    assert!(flags.contains(ssh2::OpenFlags::EXCLUSIVE));
    assert!(!flags.contains(ssh2::OpenFlags::TRUNCATE));
}
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test local_create_makes_empty_file_and_directory_without_overwriting -- --nocapture
cargo test remote_file_creation_flags_are_exclusive -- --nocapture
```

Expected: compile failure because the local and remote creation helpers do not exist.

- [ ] **Step 3: Implement local and remote creation**

Add:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateKind {
    File,
    Directory,
}

fn create_local(path: &Path, kind: CreateKind) -> Result<(), String> {
    match kind {
        CreateKind::File => std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map(|_| ())
            .map_err(|error| format!("无法创建本地文件 {}: {error}", path.display())),
        CreateKind::Directory => std::fs::create_dir(path)
            .map_err(|error| format!("无法创建本地目录 {}: {error}", path.display())),
    }
}

fn remote_file_create_flags() -> ssh2::OpenFlags {
    ssh2::OpenFlags::WRITE | ssh2::OpenFlags::CREATE | ssh2::OpenFlags::EXCLUSIVE
}
```

Add `SftpCommand::Create { side, path, kind }` and `FileOperation::Create`. In the worker:

- Local: call `create_local(&expand_local_path(&path), kind)`.
- Remote file: call `sftp.open_mode(path, remote_file_create_flags(), 0o644, ssh2::OpenType::File)` and immediately drop the handle.
- Remote directory: call `sftp.mkdir(path, 0o755)`.
- Send `MutationFinished { operation: FileOperation::Create, ... }`.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo test local_create_makes_empty_file_and_directory_without_overwriting -- --nocapture
cargo test remote_file_creation_flags_are_exclusive -- --nocapture
cargo test sftp:: -- --nocapture
```

Expected: all commands exit 0.

### Task 5: Route creation and refresh the affected pane

**Files:**
- Modify: `native-prototype/src/main.rs:70-105`
- Modify: `native-prototype/src/main.rs:270-410`
- Modify: `native-prototype/src/file_browser.rs:1180-1210`
- Test: `native-prototype/src/main.rs` `layout_tests`
- Test: `native-prototype/src/file_browser.rs` `tests`

- [ ] **Step 1: Write failing operation-label test**

Extend the mutation error test:

```rust
state.apply_event(&SftpEvent::MutationFinished {
    tab_id: "tab".into(),
    side: FileSide::Local,
    operation: FileOperation::Create,
    result: Err("目标已存在".into()),
});
assert_eq!(state.local.error.as_deref(), Some("创建失败: 目标已存在"));
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test mutation_result_updates_the_matching_pane_error -- --nocapture
```

Expected: compile failure until `FileOperation::Create` and its label are handled.

- [ ] **Step 3: Route the create action**

Handle:

```rust
FileBrowserAction::Create { side, path, kind } => {
    let result = self
        .sftp_workers
        .get(tab_id)
        .ok_or_else(|| "SFTP worker 不存在".to_string())
        .and_then(|worker| worker.send(SftpCommand::Create { side, path, kind }));
    if let Err(error) = result {
        if let Some(state) = self.file_browsers.get_mut(tab_id) {
            state.apply_event(&SftpEvent::MutationFinished {
                tab_id: tab_id.to_string(),
                side,
                operation: FileOperation::Create,
                result: Err(error),
            });
        }
    }
}
```

Map `FileOperation::Create` to the Chinese error prefix `创建`. Keep `refresh_side_for_event` unchanged because all successful mutation events already refresh their own side.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo test mutation_result_updates_the_matching_pane_error -- --nocapture
cargo test successful_mutation_refreshes_its_own_side_only -- --nocapture
cargo test
```

Expected: all Native tests pass.

### Task 6: Format, build, and run UI verification

**Files:**
- Verify: `native-prototype/build.sh`
- Verify: `run-native.sh`

- [ ] **Step 1: Run formatting and independent build**

Run:

```bash
cd native-prototype
cargo fmt --check
./build.sh
```

Expected: formatting, build, Clippy, and all tests exit 0. Existing unrelated warnings may remain, but no new warning should originate from the changed code.

- [ ] **Step 2: Verify the live interactions**

Restart only `liteterm-native` through `./run-native.sh`. In both panes verify:

1. `＋文件` and `＋目录` are visible.
2. Menu rows visibly highlight under the mouse.
3. Delete opens confirmation and Cancel leaves the target intact.
4. Local and remote creation produce empty files and directories in the displayed current paths.
5. Repeating a name shows an error without changing the existing target.

- [ ] **Step 3: Clean disposable targets**

Use a unique name under the currently displayed local and remote directories. Remove only those exact test targets after verification and confirm they no longer exist.

- [ ] **Step 4: Preserve old launchers and branch**

Verify:

```bash
sha256sum build.sh run.sh
git branch --show-current
```

Expected:

```text
69e913332c3b14111a62f5843cf35dd6751f0f566dc941efc1218364350a2a5e  build.sh
f7d1b7b264548f8e1b968c781c0fa38529a92b24e2bd894b6dcea3b080b1ff6e  run.sh
feat/memory-optimize
```

The implementation files contain pre-existing user work and remain unstaged. Commit only this plan document.
