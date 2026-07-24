# Native Terminal Protocol Event Routing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Native client's manual CSI query scanner with Alacritty's protocol-aware event routing so Fish Tab completion never receives a false terminal-size response.

**Architecture:** Each `TerminalState` owns an Alacritty event receiver, while its cloneable `Listener` only sends events into that channel. The persistent Alacritty `Processor` receives raw PTY chunks, then `TerminalState` drains `Event::PtyWrite` values into the existing local or SSH writer after parsing.

**Tech Stack:** Rust, `alacritty_terminal 0.25.1`, `std::sync::mpsc`, `portable-pty`, Cargo tests and Clippy.

---

## File Structure

- Modify: `native-prototype/src/terminal.rs`
  - Replace the empty listener with a channel-backed listener.
  - Own and drain Alacritty events in `TerminalState`.
  - Route raw PTY bytes through Alacritty and protocol replies through `WriterKind`.
  - Hold focused unit/integration tests in the existing file-local test module.
- Do not modify Fish configuration, `TERM`, keyboard routing, `main.rs`, `ssh.rs`, or build scripts.
- Do not stage or commit `terminal.rs`; it already contains user-owned uncommitted work. Only this plan document receives a commit.

### Task 1: Channel-backed Alacritty listener and protocol parsing

**Files:**
- Modify: `native-prototype/src/terminal.rs:1-70`
- Test: `native-prototype/src/terminal.rs` file-local `#[cfg(test)]` module

- [ ] **Step 1: Write failing parser-event tests**

Add a test helper that creates a real `Term<Listener>` through `TerminalState::init_term`, advances the production Alacritty processor, and collects queued `PtyWrite` strings:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    type TestProcessor = alacritty_terminal::vte::ansi::Processor<
        alacritty_terminal::vte::ansi::StdSyncHandler,
    >;

    fn advance_and_take(
        terminal: &mut TerminalState,
        parser: &mut TestProcessor,
        bytes: &[u8],
    ) -> Vec<String> {
        parser.advance(terminal.term.as_mut().unwrap(), bytes);
        terminal.take_pty_write_events()
    }

    #[test]
    fn fish_cursor_forward_is_not_a_text_area_query() {
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        let mut parser = TestProcessor::new();

        assert!(advance_and_take(&mut terminal, &mut parser, b"\x1b[18C").is_empty());
        assert!(advance_and_take(&mut terminal, &mut parser, b"\x1b[18~").is_empty());
    }

    #[test]
    fn text_area_query_uses_alacritty_reply_and_survives_chunk_split() {
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        let mut parser = TestProcessor::new();

        assert!(advance_and_take(&mut terminal, &mut parser, b"\x1b[18").is_empty());
        assert_eq!(
            advance_and_take(&mut terminal, &mut parser, b"t"),
            vec!["\x1b[8;48;180t"]
        );
    }

    #[test]
    fn cursor_position_query_still_uses_alacritty_reply() {
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        let mut parser = TestProcessor::new();

        assert!(advance_and_take(&mut terminal, &mut parser, b"\x1b[3;5H").is_empty());
        assert_eq!(
            advance_and_take(&mut terminal, &mut parser, b"\x1b[6n"),
            vec!["\x1b[3;5R"]
        );
    }

    #[test]
    fn listener_ignores_a_disconnected_event_receiver() {
        let (event_tx, event_rx) = mpsc::channel();
        drop(event_rx);

        Listener { event_tx }.send_event(Event::PtyWrite("ignored".to_string()));
    }
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml terminal::tests -- --nocapture
```

Expected: compilation fails because `Listener` has no event sender and `take_pty_write_events` does not exist.

- [ ] **Step 3: Implement the minimal channel-backed listener**

Replace the empty listener and initialize its receiver with each terminal:

```rust
#[derive(Clone)]
pub struct Listener {
    event_tx: mpsc::Sender<Event>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.event_tx.send(event);
    }
}

pub struct TerminalState {
    term: Option<Term<Listener>>,
    writer: Option<WriterKind>,
    pty_reader: Option<Box<dyn Read + Send>>,
    pty_master: Option<Box<dyn MasterPty + Send>>,
    ssh_resize_tx: ResizeSender,
    cols: u16,
    rows: u16,
    pub scroll_offset: i32,
    event_rx: Option<mpsc::Receiver<Event>>,
}

fn init_term(&mut self, cols: u16, rows: u16) {
    self.cols = cols;
    self.rows = rows;
    let config = TermConfig::default();
    let dims = TermDimensions {
        cols: cols as usize,
        rows: rows as usize,
    };
    let (event_tx, event_rx) = mpsc::channel();
    self.term = Some(Term::new(config, &dims, Listener { event_tx }));
    self.event_rx = Some(event_rx);
}

fn take_pty_write_events(&mut self) -> Vec<String> {
    let mut writes = Vec::new();
    if let Some(event_rx) = &self.event_rx {
        while let Ok(event) = event_rx.try_recv() {
            if let Event::PtyWrite(text) = event {
                writes.push(text);
            }
        }
    }
    writes
}
```

Import `alacritty_terminal::event::Event` alongside the existing listener traits and initialize `event_rx` to `None` in `TerminalState::new`.

The complete new initializer entry is:

```rust
Self {
    term: None,
    writer: None,
    pty_reader: None,
    pty_master: None,
    ssh_resize_tx: None,
    cols: 80,
    rows: 24,
    scroll_offset: 0,
    event_rx: None,
}
```

- [ ] **Step 4: Run focused and full tests and verify GREEN**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml terminal::tests -- --nocapture
cargo test --manifest-path native-prototype/Cargo.toml
```

Expected: the four new tests pass; the full suite increases from 112 to 116 passing tests.

- [ ] **Step 5: Self-review without staging**

Run:

```bash
cargo fmt --manifest-path native-prototype/Cargo.toml --check
git diff --check -- native-prototype/src/terminal.rs
```

Expected: both commands exit 0. Leave `terminal.rs` unstaged and uncommitted.

### Task 2: Route parser replies through local and SSH writers

**Files:**
- Modify: `native-prototype/src/terminal.rs:30-280`
- Test: `native-prototype/src/terminal.rs` file-local `#[cfg(test)]` module

- [ ] **Step 1: Write failing writer-routing tests**

Add a test writer and exercise the same production `process_pty_output` path used by `read_loop`:

```rust
#[derive(Clone)]
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl Write for SharedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "closed test writer",
        ))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "closed test writer",
        ))
    }
}

#[test]
fn local_writer_does_not_receive_reply_for_fish_cursor_forward() {
    let output = Arc::new(Mutex::new(Vec::new()));
    let mut terminal = TerminalState::new();
    terminal.init_term(180, 48);
    terminal.writer = Some(WriterKind::Direct(Box::new(SharedWriter(output.clone()))));
    let mut parser = TestProcessor::new();

    terminal.process_pty_output(&mut parser, b"\x1b[18C");

    assert!(output.lock().unwrap().is_empty());
}

#[test]
fn local_and_ssh_writers_receive_the_same_alacritty_reply() {
    let expected = b"\x1b[8;48;180t".to_vec();

    let local_output = Arc::new(Mutex::new(Vec::new()));
    let mut local = TerminalState::new();
    local.init_term(180, 48);
    local.writer = Some(WriterKind::Direct(Box::new(SharedWriter(local_output.clone()))));
    local.process_pty_output(&mut TestProcessor::new(), b"\x1b[18t");
    assert_eq!(*local_output.lock().unwrap(), expected);

    let (ssh_tx, ssh_rx) = mpsc::channel();
    let mut ssh = TerminalState::new();
    ssh.init_term(180, 48);
    ssh.writer = Some(WriterKind::Channel(ssh_tx));
    ssh.process_pty_output(&mut TestProcessor::new(), b"\x1b[18t");
    assert_eq!(ssh_rx.try_recv().unwrap(), expected);
}

#[test]
fn protocol_reply_write_failure_does_not_panic() {
    let mut terminal = TerminalState::new();
    terminal.init_term(180, 48);
    terminal.writer = Some(WriterKind::Direct(Box::new(FailingWriter)));

    terminal.process_pty_output(&mut TestProcessor::new(), b"\x1b[18t");
}
```

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml terminal::tests -- --nocapture
```

Expected: compilation fails because `process_pty_output` does not exist.

- [ ] **Step 3: Implement production event draining and remove manual CSI scanning**

Add one production processing path:

```rust
type Processor = alacritty_terminal::vte::ansi::Processor<
    alacritty_terminal::vte::ansi::StdSyncHandler,
>;

impl TerminalState {
    fn flush_pty_write_events(&mut self) {
        for text in self.take_pty_write_events() {
            self.write_input(&text);
        }
    }

    fn process_pty_output(&mut self, parser: &mut Processor, bytes: &[u8]) {
        if let Some(term) = &mut self.term {
            parser.advance(term, bytes);
        }
        self.flush_pty_write_events();
    }
}
```

Change `read_loop` to call only:

```rust
let mut term_state = terminal.lock().unwrap();
term_state.process_pty_output(&mut parser, &buf[..n]);
```

Delete `remove_csi_sequence`, `has_esc`, `has_dsr`, `has_18t`, manual cursor calculation, and manual size reply. Do not add a second CSI parser or any Fish-specific condition.

- [ ] **Step 4: Run focused and full tests and verify GREEN**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml terminal::tests -- --nocapture
cargo test --manifest-path native-prototype/Cargo.toml
cargo clippy --manifest-path native-prototype/Cargo.toml --all-targets
```

Expected: seven terminal protocol tests pass; the full suite reaches 119 passing tests; Clippy exits 0 with only the repository's existing warnings.

- [ ] **Step 5: Verify the final source diff without staging**

Run:

```bash
cargo fmt --manifest-path native-prototype/Cargo.toml --check
git diff --check -- native-prototype/src/terminal.rs
rg -n 'remove_csi_sequence|has_18t|has_dsr' native-prototype/src/terminal.rs
```

Expected: formatting and diff checks pass; `rg` returns no matches. Leave source unstaged and uncommitted.

### Task 3: Full build and live Fish regression

**Files:**
- Verify: `native-prototype/build.sh`
- Verify: `run-native.sh`
- Do not modify either script.

- [ ] **Step 1: Run the required Native build**

Run:

```bash
./native-prototype/build.sh
```

Expected: build, Clippy, and all 119 tests pass.

- [ ] **Step 2: Restart only the Native verification service**

Record both PIDs, stop only `codex-liteterm-native-verify.service`, and relaunch it through `run-native.sh`. Confirm the old `guishell-tauri` PID is unchanged.

```bash
ps -eo pid,lstart,comm,args | rg 'guishell-tauri|liteterm-native'
systemctl --user stop codex-liteterm-native-verify.service
systemd-run --user \
  --unit=codex-liteterm-native-verify.service \
  --working-directory=/home/lfl/ssd/code/guishell \
  ./run-native.sh
systemctl --user status codex-liteterm-native-verify.service --no-pager --lines=8
ps -eo pid,lstart,comm,args | rg 'guishell-tauri|liteterm-native'
```

- [ ] **Step 3: Reproduce the original Fish workflow**

In a temporary Native tab:

```text
fish
cd hdd/code/cix<Tab>
```

Press Tab repeatedly on valid and partial paths. Expected: completion/redraw works and no `[8;<rows>;<cols>t` text appears.

- [ ] **Step 4: Capture protocol evidence**

Launch a temporary Fish under `strace` as done during diagnosis and verify:

```text
read(0, "\x09", 1)
```

is followed by normal completion output; there must be no Fish input reads forming `\x1b[8;<rows>;<cols>t` after `ESC[18C`.

Use a fresh trace prefix, launch `strace ... fish` inside the temporary tab, reproduce the Tab completion, exit that Fish, and inspect:

```bash
rg -n 'read\(0, "\\x09"|read\(0, "\\x1b"|\\x5b\\x38\\x3b' \
  /tmp/liteterm-fish-fixed.strace.*
```

- [ ] **Step 5: Final safety checks**

Run:

```bash
git diff --check
git diff --cached --quiet
sha256sum build.sh run.sh
ps -eo pid,ppid,rss,comm,args | rg 'guishell-tauri|liteterm-native|WebKitWebProcess'
```

Expected: no whitespace errors, staged diff empty, root scripts retain their original hashes, and both old GuiShell and the rebuilt Native remain running.
