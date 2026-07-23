# Native File Icons and Context Menu Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add extension-aware vector icons and a screen-safe, fully functional file-row context menu to the Native file manager.

**Architecture:** Keep icon classification and popup positioning as pure functions in `file_browser.rs`, while UI state records the selected entry, menu anchor, and rename input per terminal tab. Send rename/delete actions through the existing SFTP worker, emit a mutation result event, and let `main.rs` refresh the affected pane on success or preserve the list and show an error on failure.

**Tech Stack:** Rust, egui 0.31, ssh2 SFTP, standard filesystem APIs, Cargo tests, `native-prototype/build.sh`

---

### Task 1: Extension-aware icon classification

**Files:**
- Modify: `native-prototype/src/file_browser.rs:119-130`
- Test: `native-prototype/src/file_browser.rs` module `ui_tests`

- [ ] **Step 1: Write the failing classification tests**

Extend `FileIconKind` expectations:

```rust
#[test]
fn file_extensions_select_specialized_icon_kinds() {
    assert_eq!(file_icon_kind(&entry("main.RS", false)), FileIconKind::Code);
    assert_eq!(file_icon_kind(&entry("config.toml", false)), FileIconKind::Text);
    assert_eq!(file_icon_kind(&entry("photo.webp", false)), FileIconKind::Image);
    assert_eq!(file_icon_kind(&entry("backup.tar.gz", false)), FileIconKind::Archive);
    assert_eq!(file_icon_kind(&entry("kernel.elf", false)), FileIconKind::Binary);
    assert_eq!(file_icon_kind(&entry("LICENSE", false)), FileIconKind::File);
}
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
cd native-prototype
cargo test file_browser::ui_tests::file_extensions_select_specialized_icon_kinds -- --nocapture
```

Expected: compilation fails because the specialized enum variants do not exist.

- [ ] **Step 3: Implement the classifier**

Use the final suffix, normalized with ASCII lowercase:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileIconKind {
    Folder,
    Code,
    Text,
    Image,
    Archive,
    Binary,
    File,
}

fn file_icon_kind(entry: &FileEntry) -> FileIconKind {
    if entry.is_dir {
        return FileIconKind::Folder;
    }
    let extension = std::path::Path::new(&entry.name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "rs" | "c" | "cc" | "cpp" | "h" | "hpp" | "py" | "js" | "jsx" | "ts"
        | "tsx" | "go" | "java" | "kt" | "lua" | "php" | "rb" | "sh" | "bash"
        | "zsh" | "fish" => FileIconKind::Code,
        "txt" | "md" | "log" | "json" | "toml" | "yaml" | "yml" | "xml" | "ini"
        | "conf" | "cfg" | "env" | "properties" | "csv" => FileIconKind::Text,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" => {
            FileIconKind::Image
        }
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "zst" => {
            FileIconKind::Archive
        }
        "bin" | "elf" | "run" | "appimage" | "exe" | "so" | "dll" | "dylib" => {
            FileIconKind::Binary
        }
        _ => FileIconKind::File,
    }
}
```

- [ ] **Step 4: Draw all icon kinds and run the UI tests**

Keep `Folder` and `File` geometry. Add deterministic 12×12 line drawings:

- `Code`: cyan `< >`
- `Text`: muted-blue page with two horizontal lines
- `Image`: green frame with mountain line and point
- `Archive`: orange box with central zipper
- `Binary`: red-gray hexagon with a center point

Run:

```bash
rustfmt --edition 2021 src/file_browser.rs
cargo test file_browser::ui_tests:: -- --nocapture
```

Expected: all UI tests pass and painted text still contains no marker glyphs.

### Task 2: Context menu model and screen-safe positioning

**Files:**
- Modify: `native-prototype/src/file_browser.rs:30-145`
- Test: `native-prototype/src/file_browser.rs` module `ui_tests`

- [ ] **Step 1: Write failing popup-position and menu-model tests**

Add:

```rust
#[test]
fn context_menu_opens_above_when_bottom_space_is_insufficient() {
    let screen = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1280.0, 800.0));
    assert_eq!(
        popup_position(egui::pos2(900.0, 760.0), egui::vec2(160.0, 90.0), screen),
        egui::pos2(900.0, 670.0)
    );
}

#[test]
fn context_menu_is_clamped_to_screen_edges() {
    let screen = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1280.0, 800.0));
    assert_eq!(
        popup_position(egui::pos2(1270.0, 20.0), egui::vec2(160.0, 90.0), screen),
        egui::pos2(1120.0, 20.0)
    );
}

#[test]
fn directory_transfer_item_is_disabled_but_mutations_remain_enabled() {
    let items = context_menu_items(
        FileSide::Local,
        &entry("src", true),
        "/home/local",
        "/srv/remote",
        true,
    );
    assert_eq!(items[0].label, "上传到远程 (remote)");
    assert!(!items[0].enabled);
    assert!(items.iter().any(|item| item.label == "重命名" && item.enabled));
    assert!(items.iter().any(|item| item.label == "删除" && item.enabled));
}
```

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
cargo test file_browser::ui_tests::context_menu_opens_above_when_bottom_space_is_insufficient -- --nocapture
```

Expected: compilation fails because `popup_position` does not exist.

- [ ] **Step 3: Add pure positioning and menu-spec helpers**

Implement:

```rust
fn popup_position(pointer: egui::Pos2, size: egui::Vec2, screen: egui::Rect) -> egui::Pos2 {
    let max_x = (screen.right() - size.x).max(screen.left());
    let x = pointer.x.clamp(screen.left(), max_x);
    let desired_y = if pointer.y + size.y > screen.bottom() {
        pointer.y - size.y
    } else {
        pointer.y
    };
    let max_y = (screen.bottom() - size.y).max(screen.top());
    egui::pos2(x, desired_y.clamp(screen.top(), max_y))
}
```

Define `ContextCommand::{Transfer, Rename, Delete}` and `ContextItemSpec { label, command, enabled, separator_before }`. `context_menu_items` must return the exact main labels and disable transfer for directories or an unavailable remote connection.

- [ ] **Step 4: Run positioning and menu tests**

Run:

```bash
cargo test file_browser::ui_tests::context_menu_ -- --nocapture
cargo test file_browser::ui_tests::directory_transfer_item_is_disabled_but_mutations_remain_enabled -- --nocapture
```

Expected: all focused tests pass.

### Task 3: Context menu UI, rename dialog, and browser actions

**Files:**
- Modify: `native-prototype/src/file_browser.rs:25-530`
- Test: `native-prototype/src/file_browser.rs` module `ui_tests`

- [ ] **Step 1: Write failing action-construction tests**

Add:

```rust
#[test]
fn blank_rename_is_rejected() {
    assert!(rename_action(FileSide::Local, "/tmp/a", "/tmp", "   ").is_none());
}

#[test]
fn rename_action_keeps_the_entry_in_its_parent_directory() {
    assert_eq!(
        rename_action(FileSide::Remote, "/srv/old.txt", "/srv", "new.txt"),
        Some(FileBrowserAction::Rename {
            side: FileSide::Remote,
            old_path: "/srv/old.txt".into(),
            new_path: "/srv/new.txt".into(),
        })
    );
}
```

- [ ] **Step 2: Run the rename test and verify RED**

Run:

```bash
cargo test file_browser::ui_tests::blank_rename_is_rejected -- --nocapture
```

Expected: compilation fails because `rename_action` and the `Rename` action variant do not exist.

- [ ] **Step 3: Add state and action types**

Add:

```rust
#[derive(Clone, Debug)]
struct FileContextMenu {
    side: FileSide,
    entry: FileEntry,
    parent_path: String,
    destination_path: String,
    pointer: egui::Pos2,
}

#[derive(Clone, Debug)]
struct RenameDialogState {
    side: FileSide,
    old_path: String,
    parent_path: String,
    value: String,
    request_focus: bool,
}
```

Add `context_menu: Option<FileContextMenu>` and `rename_dialog: Option<RenameDialogState>` to `FileBrowserState`. Add actions:

```rust
Rename { side: FileSide, old_path: String, new_path: String },
Delete { side: FileSide, path: String, is_dir: bool },
```

`rename_action` trims the input, rejects empty values, and uses `crate::sftp::join_path(parent_path, name)`.

- [ ] **Step 4: Open and render the custom menu**

On `response.secondary_clicked()`:

```rust
pane.selected = Some(entry.name.clone());
pending_menu = Some(FileContextMenu {
    side,
    entry: entry.clone(),
    parent_path: pane.path.clone(),
    destination_path: destination_path.to_string(),
    pointer: response.interact_pointer_pos().unwrap_or(response.rect.left_top()),
});
```

Render an `egui::Area` at `popup_position(pointer, vec2(160.0, 90.0), screen_rect)`. Use `Frame::popup`, 160 px minimum width, full-width compact buttons, a separator, disabled transfer for directories, and dark project colors. Close it after a command, on Esc, or on a primary click outside the returned area rectangle.

- [ ] **Step 5: Render rename dialog and verify UI tests**

Render a centered non-resizable `egui::Window` titled `重命名`; request focus/select-all once, submit on Enter or `确定`, and close on Esc or `取消`. A successful submit pushes `rename_action`.

Run:

```bash
rustfmt --edition 2021 src/file_browser.rs
cargo test file_browser::ui_tests:: -- --nocapture
```

Expected: all UI tests pass, including duplicate egui ID detection.

### Task 4: Worker rename/delete operations and mutation events

**Files:**
- Modify: `native-prototype/src/sftp.rs:15-350`
- Test: `native-prototype/src/sftp.rs` modules `tests` and `worker_tests`

- [ ] **Step 1: Write failing local-operation tests**

Add:

```rust
#[test]
fn local_rename_and_recursive_delete_match_main_behavior() {
    let temp = tempfile::tempdir().unwrap();
    let old = temp.path().join("old");
    let new = temp.path().join("new");
    std::fs::create_dir(&old).unwrap();
    std::fs::write(old.join("nested.txt"), b"x").unwrap();

    rename_local(&old, &new).unwrap();
    assert!(new.join("nested.txt").exists());
    delete_local(&new, true).unwrap();
    assert!(!new.exists());
}
```

- [ ] **Step 2: Run the local-operation test and verify RED**

Run:

```bash
cargo test sftp::tests::local_rename_and_recursive_delete_match_main_behavior -- --nocapture
```

Expected: compilation fails because `rename_local` and `delete_local` do not exist.

- [ ] **Step 3: Add mutation types and filesystem helpers**

Add:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileOperation { Rename, Delete }

pub fn rename_local(old_path: &Path, new_path: &Path) -> Result<(), String> {
    std::fs::rename(old_path, new_path).map_err(|e| format!("重命名失败: {e}"))
}

pub fn delete_local(path: &Path, is_dir: bool) -> Result<(), String> {
    if is_dir {
        std::fs::remove_dir_all(path).map_err(|e| format!("删除本地目录失败: {e}"))
    } else {
        std::fs::remove_file(path).map_err(|e| format!("删除本地文件失败: {e}"))
    }
}
```

Extend `SftpCommand` with `Rename` and `Delete`, both carrying `side`. Extend `SftpEvent` with:

```rust
MutationFinished {
    tab_id: String,
    side: FileSide,
    operation: FileOperation,
    result: Result<(), String>,
}
```

- [ ] **Step 4: Execute local and remote mutations in the worker**

For local paths, expand `~` and call the helpers. For remote rename call `sftp.rename(old, new, None)`. For remote delete call `sftp.rmdir(path)` for directories or `sftp.unlink(path)` for files. Map errors to Chinese messages and always send one `MutationFinished` event.

- [ ] **Step 5: Run SFTP tests**

Run:

```bash
rustfmt --edition 2021 src/sftp.rs
cargo test sftp::tests:: -- --nocapture
cargo test sftp::worker_tests:: -- --nocapture
```

Expected: all SFTP tests pass.

### Task 5: Dispatch actions, refresh success, and surface errors

**Files:**
- Modify: `native-prototype/src/main.rs:210-335`
- Modify: `native-prototype/src/main.rs:1450-1485`
- Modify: `native-prototype/src/file_browser.rs:580-675`
- Test: `native-prototype/src/file_browser.rs` module `tests`
- Test: `native-prototype/src/main.rs` module `layout_tests`

- [ ] **Step 1: Write failing mutation-result tests**

In `file_browser.rs`:

```rust
#[test]
fn failed_delete_sets_error_on_the_affected_pane() {
    let mut state = FileBrowserState::new("/tmp".into());
    state.apply_event(&SftpEvent::MutationFinished {
        tab_id: "tab".into(),
        side: FileSide::Remote,
        operation: FileOperation::Delete,
        result: Err("目录非空".into()),
    });
    assert_eq!(state.remote.error.as_deref(), Some("删除失败: 目录非空"));
    assert!(state.local.error.is_none());
}
```

In `main.rs`, test a pure helper:

```rust
#[test]
fn successful_mutation_refreshes_its_source_pane() {
    let event = SftpEvent::MutationFinished {
        tab_id: "tab".into(),
        side: FileSide::Remote,
        operation: FileOperation::Rename,
        result: Ok(()),
    };
    assert_eq!(refresh_side_for_event(&event), Some(FileSide::Remote));
}
```

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
cargo test failed_delete_sets_error_on_the_affected_pane -- --nocapture
cargo test successful_mutation_refreshes_its_source_pane -- --nocapture
```

Expected: compilation fails because mutation event handling and `refresh_side_for_event` do not exist.

- [ ] **Step 3: Dispatch Rename and Delete actions**

Map `FileBrowserAction::Rename` and `Delete` to the corresponding `SftpCommand`. If the worker send fails, immediately apply a matching `MutationFinished` error event to the browser state.

- [ ] **Step 4: Handle mutation events**

In `FileBrowserState::apply_event`, set the affected pane error to `重命名失败: …` or `删除失败: …` only on failure. Extract `refresh_side_for_event`: successful upload refreshes remote, successful download refreshes local, successful mutation refreshes its `side`.

Include `MutationFinished` in the `UserEvent::Sftp` tab-id match, apply it to state, then request the returned refresh side.

- [ ] **Step 5: Run focused and complete tests**

Run:

```bash
rustfmt --edition 2021 src/main.rs src/file_browser.rs
cargo test failed_delete_sets_error_on_the_affected_pane -- --nocapture
cargo test successful_mutation_refreshes_its_source_pane -- --nocapture
cargo test
```

Expected: every Native test passes.

### Task 6: Build and real UI acceptance

**Files:**
- Verify: `native-prototype/src/file_browser.rs`
- Verify: `native-prototype/src/sftp.rs`
- Verify: `native-prototype/src/main.rs`
- Preserve: root `build.sh` and `run.sh`

- [ ] **Step 1: Run the independent Native gate**

Run:

```bash
./native-prototype/build.sh
```

Expected: build, Clippy, and all tests exit successfully.

- [ ] **Step 2: Verify old script hashes and diff hygiene**

Run:

```bash
sha256sum build.sh run.sh
git diff --check
```

Expected:

```text
69e913332c3b14111a62f5843cf35dd6751f0f566dc941efc1218364350a2a5e  build.sh
f7d1b7b264548f8e1b968c781c0fa38529a92b24e2bd894b6dcea3b080b1ff6e  run.sh
```

- [ ] **Step 3: Verify menu placement and operations**

Run `./run-native.sh`, connect to `192.168.110.81`, and verify:

- top-row right-click opens downward
- bottom-row right-click opens upward and remains inside the window
- local/remote menus match main labels and disabled states
- upload/download starts the existing transfer flow
- rename updates the row after automatic refresh
- local delete removes a temporary nested directory
- remote non-empty directory deletion shows an error without clearing the list
- specialized vector icons appear without replacement squares

- [ ] **Step 4: Preserve the shared dirty implementation**

Do not stage whole untracked Native files or mixed tracked files as an isolated feature commit. Keep the existing `feat/memory-optimize` branch and working tree intact, consistent with the user's earlier instruction.
