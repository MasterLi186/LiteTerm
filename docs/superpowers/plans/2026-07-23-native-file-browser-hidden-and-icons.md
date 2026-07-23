# Native File Browser Hidden Files and Icons Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Native file manager hide dot-prefixed entries by default and render reliable vector folder/file icons.

**Architecture:** Keep directory results intact in `PaneState` and derive visible entries only in the UI layer. Replace font-dependent marker glyphs with a small `FileIconKind` classifier and egui `Painter` drawing helpers so rendering is deterministic across Linux fonts.

**Tech Stack:** Rust, egui 0.31, Cargo tests, existing `native-prototype/build.sh`

---

### Task 1: Filter hidden entries in the UI

**Files:**
- Modify: `native-prototype/src/file_browser.rs:90-330`
- Test: `native-prototype/src/file_browser.rs` module `ui_tests`

- [ ] **Step 1: Write the failing visibility tests**

Add tests that define the intended UI-only rule:

```rust
#[test]
fn dot_prefixed_entries_are_hidden_by_default() {
    assert!(!is_visible_entry(&entry(".git", true)));
    assert!(!is_visible_entry(&entry(".env", false)));
    assert!(is_visible_entry(&entry("src", true)));
    assert!(is_visible_entry(&entry("README.md", false)));
}

#[test]
fn visible_entry_count_excludes_dot_prefixed_entries() {
    let entries = vec![entry(".git", true), entry("src", true), entry("main.rs", false)];
    assert_eq!(visible_entries(&entries).count(), 2);
}
```

Create the test helper with real `FileEntry` values:

```rust
fn entry(name: &str, is_dir: bool) -> FileEntry {
    FileEntry {
        name: name.into(),
        path: format!("/tmp/{name}"),
        is_dir,
        size: 0,
        mtime: 0,
    }
}
```

- [ ] **Step 2: Run tests and verify the RED state**

Run:

```bash
cd native-prototype
cargo test file_browser::ui_tests::dot_prefixed_entries_are_hidden_by_default -- --nocapture
```

Expected: compilation fails because `is_visible_entry` does not exist.

- [ ] **Step 3: Add the minimal visibility helpers**

Add:

```rust
fn is_visible_entry(entry: &FileEntry) -> bool {
    !entry.name.starts_with('.')
}

fn visible_entries(entries: &[FileEntry]) -> impl Iterator<Item = &FileEntry> {
    entries.iter().filter(|entry| is_visible_entry(entry))
}
```

Use `visible_entries(&pane.entries).count()` in the pane title and iterate over `visible_entries(&pane.entries).enumerate()` when rendering rows. Do not mutate `PaneState.entries`.

- [ ] **Step 4: Run both visibility tests**

Run:

```bash
cargo test file_browser::ui_tests::dot_prefixed_entries_are_hidden_by_default -- --nocapture
cargo test file_browser::ui_tests::visible_entry_count_excludes_dot_prefixed_entries -- --nocapture
```

Expected: both tests pass.

### Task 2: Replace marker glyphs with vector icons

**Files:**
- Modify: `native-prototype/src/file_browser.rs:377-455`
- Test: `native-prototype/src/file_browser.rs` module `ui_tests`

- [ ] **Step 1: Write the failing icon classification test**

Add:

```rust
#[test]
fn directories_and_files_use_distinct_icon_kinds() {
    assert_eq!(file_icon_kind(&entry("src", true)), FileIconKind::Folder);
    assert_eq!(file_icon_kind(&entry("main.rs", false)), FileIconKind::File);
}
```

Update the existing render regression test to assert that painted text contains neither `"▸"` nor `"·"`.

- [ ] **Step 2: Run the icon test and verify the RED state**

Run:

```bash
cargo test file_browser::ui_tests::directories_and_files_use_distinct_icon_kinds -- --nocapture
```

Expected: compilation fails because `FileIconKind` and `file_icon_kind` do not exist.

- [ ] **Step 3: Add icon types and vector drawing**

Add:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileIconKind {
    Folder,
    File,
}

fn file_icon_kind(entry: &FileEntry) -> FileIconKind {
    if entry.is_dir { FileIconKind::Folder } else { FileIconKind::File }
}
```

Implement `paint_file_icon(painter, center, kind)` using only egui geometry:

- `Folder`: stroke a 12×8 body rectangle plus a 5×2 tab using `YELLOW`
- `File`: stroke an 8×11 page outline and two-line folded corner using `DIM`
- use a 1 px stroke and no Unicode text

Call the painter at `rect.left() + 13.0`, preserve the current name start at `rect.left() + 20.0`, and remove the marker `painter.text` call.

- [ ] **Step 4: Run icon and render regression tests**

Run:

```bash
cargo test file_browser::ui_tests::directories_and_files_use_distinct_icon_kinds -- --nocapture
cargo test file_browser::ui_tests::file_rows_use_main_time_format_and_vector_icons -- --nocapture
```

Expected: both tests pass; painted text contains no old marker glyphs.

### Task 3: Full verification and visual acceptance

**Files:**
- Verify: `native-prototype/src/file_browser.rs`
- Verify: root `build.sh` and `run.sh` remain unchanged

- [ ] **Step 1: Format and run the complete Native gate**

Run:

```bash
rustfmt --edition 2021 native-prototype/src/file_browser.rs
./native-prototype/build.sh
```

Expected: build, Clippy, and all Native tests exit successfully.

- [ ] **Step 2: Verify old GuiShell scripts and diff hygiene**

Run:

```bash
sha256sum build.sh run.sh
git diff --check
```

Expected script hashes:

```text
69e913332c3b14111a62f5843cf35dd6751f0f566dc941efc1218364350a2a5e  build.sh
f7d1b7b264548f8e1b968c781c0fa38529a92b24e2bd894b6dcea3b080b1ff6e  run.sh
```

- [ ] **Step 3: Restart Native and inspect the real SSH panes**

Run `./run-native.sh`, connect to the existing `192.168.110.81` profile, and open the file manager. Confirm:

- `.agents`, `.cache`, and other dot-prefixed entries are absent
- normal directories show yellow vector folders
- normal files show gray vector documents
- no replacement squares or duplicate-ID overlays appear
- size and `MM-DD HH:mm` columns remain aligned

- [ ] **Step 4: Preserve the shared dirty implementation safely**

Do not stage the whole untracked `native-prototype/src/file_browser.rs` or mixed dependency files as an isolated icon commit. Record the passing verification and keep the current `feat/memory-optimize` working tree intact, matching the user's earlier branch choice.
