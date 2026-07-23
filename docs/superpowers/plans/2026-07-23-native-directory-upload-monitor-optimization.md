# Native Directory Upload and Monitor Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable recursive local-directory uploads in LiteTerm Native, then remove unnecessary task enumeration and `/proc/stat` descriptor caching from the system monitor.

**Architecture:** The file browser continues to emit the existing `Upload` action. The SFTP worker detects whether its local source is a file or directory; directories are converted into a deterministic, testable upload plan and streamed through the existing progress event. The monitor keeps `sysinfo`, but requests only process CPU/memory and disables Linux stat-file caching.

**Tech Stack:** Rust, egui 0.31, ssh2 SFTP, sysinfo 0.34, tempfile tests

---

### Task 1: Enable the local-directory upload action

**Files:**
- Modify: `native-prototype/src/file_browser.rs:189-220`
- Test: `native-prototype/src/file_browser.rs` `ui_tests`

- [ ] **Step 1: Write the failing menu-rule test**

Replace the old directory-disabled assertion with explicit local/remote rules:

```rust
#[test]
fn local_directories_can_upload_but_remote_directories_cannot_download() {
    let local = context_menu_items(
        FileSide::Local,
        &entry("src", true),
        "/home/local",
        "/srv/remote",
        true,
    );
    let remote = context_menu_items(
        FileSide::Remote,
        &entry("logs", true),
        "/home/local",
        "/srv/remote",
        true,
    );
    assert!(local[0].enabled);
    assert!(!remote[0].enabled);
}
```

- [ ] **Step 2: Verify RED**

Run `cargo test local_directories_can_upload_but_remote_directories_cannot_download -- --nocapture` in `native-prototype/`.

Expected: FAIL because the local directory item remains disabled.

- [ ] **Step 3: Implement the side-specific rule**

```rust
let transfer_enabled = match side {
    FileSide::Local => remote_ready,
    FileSide::Remote => remote_ready && !entry.is_dir,
};
```

Use `transfer_enabled` for the transfer menu item.

- [ ] **Step 4: Verify GREEN**

Run the same focused test, then `cargo test file_browser::ui_tests`.

### Task 2: Build a deterministic local upload plan

**Files:**
- Modify: `native-prototype/src/sftp.rs`
- Test: `native-prototype/src/sftp.rs` `tests`

- [ ] **Step 1: Write failing traversal tests**

Create a temporary tree containing `src/main.rs`, `empty/`, and a root file. Assert that `build_local_upload_plan(root)` returns directories before their children, relative paths exclude the source root, and `total_bytes` equals ordinary-file sizes. On Unix, add a symlink and assert the error contains `不支持上传符号链接`.

- [ ] **Step 2: Verify RED**

Run `cargo test local_upload_plan -- --nocapture`.

Expected: compile failure because `build_local_upload_plan` does not exist.

- [ ] **Step 3: Implement the plan types and traversal**

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalUploadEntry {
    source: PathBuf,
    relative: PathBuf,
    is_dir: bool,
    size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalUploadPlan {
    entries: Vec<LocalUploadEntry>,
    total_bytes: u64,
}
```

Use `symlink_metadata`, sort each `read_dir` level by file name, reject symlinks, push directories before recursion, and sum regular-file lengths.

- [ ] **Step 4: Verify GREEN**

Run `cargo test local_upload_plan -- --nocapture`.

### Task 3: Stream the directory plan through SFTP

**Files:**
- Modify: `native-prototype/src/sftp.rs:253-405`
- Test: `native-prototype/src/sftp.rs` `tests`

- [ ] **Step 1: Write failing remote-path tests**

Test that `remote_plan_path("/srv/release", Path::new("assets/icon.png"))` returns `/srv/release/assets/icon.png`, including root and trailing-slash cases.

- [ ] **Step 2: Verify RED**

Run `cargo test remote_plan_path -- --nocapture`.

- [ ] **Step 3: Implement recursive transfer**

Add:

```rust
fn upload_local_path(
    sftp: &ssh2::Sftp,
    proxy: &EventLoopProxy<crate::UserEvent>,
    tab_id: &str,
    transfer_id: &str,
    local_path: &str,
    remote_path: &str,
) -> Result<(), String>
```

For a file, call the current single-file transfer. For a directory:

1. Build the local plan.
2. Ensure `remote_path` is a directory, creating it with mode `0o755` if absent.
3. Create planned subdirectories in preorder.
4. Upload each planned file.
5. Emit progress using `completed_before + current_file_bytes` against the plan-wide total.

`ensure_remote_dir` must accept an existing directory but reject an existing non-directory. Refactor the copy loop to accept progress base and total without changing single-file semantics.

- [ ] **Step 4: Connect the worker**

Change the `SftpCommand::Upload` arm to call `upload_local_path`; keep the same `TransferFinished` event so successful directory uploads refresh the remote pane.

- [ ] **Step 5: Verify GREEN**

Run `cargo test sftp:: -- --nocapture`, then `cargo test`.

### Task 4: Remove monitor task and FD overhead

**Files:**
- Modify: `native-prototype/src/monitor.rs:1-105`
- Test: `native-prototype/src/monitor.rs`

- [ ] **Step 1: Write the failing refresh-policy test**

Extract `process_refresh_kind()` and assert:

```rust
let kind = process_refresh_kind();
assert!(kind.cpu());
assert!(kind.memory());
assert!(!kind.tasks());
```

- [ ] **Step 2: Verify RED**

Run `cargo test monitor_process_refresh_avoids_tasks -- --nocapture`.

- [ ] **Step 3: Implement the monitor policy**

Import `ProcessRefreshKind` and `ProcessesToUpdate`. Before the first process refresh call:

```rust
let _ = sysinfo::set_open_files_limit(0);
```

Replace `refresh_processes` with:

```rust
self.sys.refresh_processes_specifics(
    ProcessesToUpdate::All,
    true,
    process_refresh_kind(),
);
```

where `process_refresh_kind()` is:

```rust
ProcessRefreshKind::nothing()
    .with_cpu()
    .with_memory()
    .without_tasks()
```

The explicit `without_tasks()` is required because `sysinfo` 0.34 enables tasks even in `ProcessRefreshKind::nothing()`.

- [ ] **Step 4: Verify GREEN**

Run the focused monitor test, then all Native tests.

### Task 5: Build and live verification

**Files:**
- Verify: `native-prototype/build.sh`
- Verify: `run-native.sh`

- [ ] **Step 1: Run formatting and the independent build**

Run:

```bash
cd native-prototype
cargo fmt --check
./build.sh
```

Expected: build and Clippy exit 0; all tests pass.

- [ ] **Step 2: Verify directory upload**

Create a disposable local tree with a nested file and empty directory, upload it to a disposable remote path through the UI, and verify the remote tree and file bytes. Remove only the disposable remote tree after verification.

- [ ] **Step 3: Measure monitor resources**

Restart Native, wait for two monitor updates, then record `VmRSS`, `RssAnon`, `smaps_rollup`, total FD count, `/proc/*/stat` FD count, and `/proc/*/task/*/stat` FD count. Expected task-stat and process-stat cached FDs: zero.

- [ ] **Step 4: Preserve old launchers**

Verify root `build.sh` and `run.sh` hashes remain:

```text
69e913332c3b14111a62f5843cf35dd6751f0f566dc941efc1218364350a2a5e  build.sh
f7d1b7b264548f8e1b968c781c0fa38529a92b24e2bd894b6dcea3b080b1ff6e  run.sh
```

Implementation files already contain pre-existing user work, so do not stage or commit them as a bundle. Commit only this plan document; leave implementation changes visible in the working tree.
