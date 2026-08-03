# Bash and SSH Lifecycle Review Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make local Bash child reaping reliable across worker-start and queue failures, and make SSH completion notification independent of how the I/O loop exits.

**Architecture:** `terminal.rs` will own one process-wide reaper queue whose worker never captures a specific child during startup; a small injectable enqueue helper will synchronously fall back to `wait` after kill if the queue is unavailable or disconnected. `ssh.rs` will use separate shutdown-request and I/O-done channels, with the done signal sent after all SSH resources are dropped on every loop exit.

**Tech Stack:** Rust 2021, `std::sync::OnceLock`, `std::sync::mpsc`, `portable_pty`, existing native-prototype unit/integration tests.

---

### Task 1: Reliable local child reaper

**Files:**
- Modify: `native-prototype/src/terminal.rs`
- Test: `native-prototype/src/terminal.rs`

- [x] **Step 1: Write failing fallback tests**

Add fakes that record `kill` and `wait`, then test these three behaviors through an injectable helper:

```rust
assert!(enqueue_or_reap(child, None).is_err());
assert!(killed.load(Ordering::SeqCst));
assert!(waited.load(Ordering::SeqCst));
```

Cover unavailable worker, disconnected sender returning the child, and kill failure still reaching `wait`.

- [x] **Step 2: Verify RED**

Run:

```bash
cd native-prototype
cargo test local_reaper_
```

Expected: compile failure because the injectable enqueue helper does not exist.

- [x] **Step 3: Implement the singleton queue**

Use:

```rust
static LOCAL_CHILD_REAPER: OnceLock<
    Option<mpsc::Sender<Box<dyn portable_pty::Child + Send + Sync>>>,
> = OnceLock::new();
```

Start a long-lived worker without capturing a concrete child. Kill synchronously, enqueue the child, recover it from `SendError`, and synchronously `wait` only for unavailable/disconnected-worker fallbacks. The worker must retry kill as needed and call final `wait` rather than dropping after `try_wait` errors or a fixed deadline.

- [x] **Step 4: Verify GREEN**

Run:

```bash
cd native-prototype
cargo test local_reaper_
cargo test shutdown_hands_slow_child_reaping_off_without_blocking_ui_mutex
```

Expected: all selected tests pass and the normal queue path remains below 200 ms.

### Task 2: Exit-independent SSH completion

**Files:**
- Modify: `native-prototype/src/ssh.rs`
- Modify: `native-prototype/src/terminal.rs`
- Modify test constructors in `native-prototype/src/main.rs` and `native-prototype/src/tab_manager.rs`

- [x] **Step 1: Write the natural-exit race test**

Construct an `SshHandle` whose done channel is already signalled and shutdown receiver has no worker, then assert:

```rust
handle
    .shutdown_and_wait(Duration::from_secs(1))
    .expect("natural I/O exit must remain observable");
```

- [x] **Step 2: Verify RED**

Run:

```bash
cd native-prototype
cargo test ssh_handle_shutdown_and_wait_succeeds_after_natural_io_exit
```

Expected: compile or assertion failure because completion is coupled to the shutdown request.

- [x] **Step 3: Split request and completion channels**

Return `shutdown_tx: Sender<()>` and a separate done receiver in `SshHandle`. Make `shutdown_and_wait` send once, ignore send failure, then wait on done. The I/O thread must send done after dropping pipe, channel, and session on shutdown, EOF, or error. Production terminal shutdown remains fire-and-forget.

- [x] **Step 4: Verify GREEN**

Run:

```bash
cd native-prototype
cargo test ssh::tests::
cargo test shutdown
```

Expected: SSH unit tests and all shutdown-focused tests pass; the real SSH fixture remains ignored.

### Task 3: PTY environment assertion cleanup and final verification

**Files:**
- Modify: `native-prototype/src/bash_integration.rs`

- [x] **Step 1: Remove the overwritten external-HISTFILE setup**

Delete the external temp directory, sentinel file, overwritten `command.env("HISTFILE", ...)`, and its assertion. Retain the child-reported `HOME`, `HISTFILE`, `INPUTRC`, and `BASH_ENV` assertions plus the isolated history content assertion.

- [x] **Step 2: Run focused and full verification**

Run:

```bash
cd native-prototype
cargo test real_local_bash_pty_emits_authenticated_prompt_without_modifying_bashrc
cargo fmt --check
cargo test
cargo clippy --all-targets
```

Expected: focused PTY test passes, formatting passes, all non-ignored tests pass, and Clippy exits successfully with only the repository warning baseline.

- [x] **Step 3: Audit shared-worktree safety**

Run:

```bash
git diff --check
git diff --cached --name-only
git diff --exit-code -- build.sh run.sh native-prototype/build.sh native-prototype/run-native.sh
```

Expected: no whitespace errors, empty staged set, and no build/run script differences. Do not commit because the task explicitly forbids it.
