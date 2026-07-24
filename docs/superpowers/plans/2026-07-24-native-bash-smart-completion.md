# Native Bash Smart Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add safe history-based completion popups to local and SSH Bash tabs, where the first Enter only fills Readline and a second human Enter executes.

**Architecture:** Keep history/matching and popup geometry in focused new modules. A session-only Bash RC emits authenticated private OSC markers and installs a private `bind -x` widget; `terminal.rs` advances the existing single Alacritty parser exactly to each marker boundary before snapshotting the prompt cursor. Local history uses a background file reader, while SSH history and candidate writes reuse the per-tab SFTP worker with tab/generation/token routing.

**Tech Stack:** Rust 2021, Alacritty Terminal 0.25, portable-pty, ssh2/SFTP, egui 0.31, winit 0.30, `base64`, `tempfile`

---

## Execution Constraints

- Stay on `feat/memory-optimize`; do not create or switch branches.
- Preserve the dirty worktree. Stage only files named by the current task.
- Do not modify or stop the old GuiShell process. Build Native with `./native-prototype/build.sh`; do not run root `./build.sh`, root `run.sh`, or `npx tauri dev`.
- Follow TDD. Every task gets a fresh implementation subagent, then a specification review and a code-quality review before moving on.
- Do not change the existing app-specific `native_cmd_history.json`; Bash completion reads Bash history only.

## File Map

- Create `native-prototype/src/smart_completion.rs`: session identity, history tail loading/parsing, mixed ranking, per-tab candidate state.
- Create `native-prototype/src/bash_integration.rs`: private OSC decoder, Bash RC generation, secure local runtime files, widget sequence and remote path planning.
- Create `native-prototype/src/completion_popup.rs`: popup geometry and egui rendering.
- Modify `native-prototype/src/terminal.rs`: synchronized marker boundaries, prompt anchor, soft-wrap input extraction, fill staging and invalidation.
- Modify `native-prototype/src/tab_manager.rs`: attach `CompletionState` to every tab and validate session identity.
- Modify `native-prototype/src/ssh.rs`: probe Bash, prepare remote integration files, exec integrated Bash, fall back to ordinary shell.
- Modify `native-prototype/src/sftp.rs`: bounded remote history reads and atomic candidate writes.
- Modify `native-prototype/src/main.rs`: async event routing, history requests, popup state and keyboard interception.
- Modify `native-prototype/src/renderer.rs`: expose terminal viewport and cursor rectangles using the renderer’s existing display-offset formula.
- Modify `native-prototype/Cargo.toml` and `native-prototype/Cargo.lock`: add runtime `base64` and `tempfile`.

### Task 1: History Parsing, Ranking, and Per-Tab State

**Files:**
- Create: `native-prototype/src/smart_completion.rs`
- Modify: `native-prototype/src/main.rs:12-31`

- [ ] **Step 1: Add the module declaration and failing history tests**

Add `mod smart_completion;` beside the other module declarations in `main.rs`. Create `smart_completion.rs` with the constants, test module, and these exact behavioral tests:

```rust
use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const MAX_CANDIDATES: usize = 8;
pub const MAX_HISTORY_ITEMS: usize = 5_000;
pub const MAX_HISTORY_BYTES: u64 = 2 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamped_multiline_entries_are_dropped() {
        let data = b"#100\nls -la\n#101\nprintf 'a\nb'\n#102\ngit status\n";
        assert_eq!(parse_bash_history(data), ["ls -la", "git status"]);
    }

    #[test]
    fn untimestamped_lines_are_independent_and_control_bytes_are_rejected() {
        let data = b"echo one\n\nprintf two\nbad\tcommand\n";
        assert_eq!(parse_bash_history(data), ["echo one", "printf two"]);
    }

    #[test]
    fn ranking_is_prefix_then_contains_recent_first_and_deduplicated() {
        let history = vec![
            "git status".into(),
            "echo git status".into(),
            "git log".into(),
            "git status".into(),
            "printf git".into(),
        ];
        assert_eq!(
            rank_candidates(&history, "git"),
            ["git status", "git log", "echo git status", "printf git"]
        );
    }

    #[test]
    fn exact_prefix_is_excluded_and_results_are_capped() {
        let history = (0..20).map(|index| format!("echo {index}")).collect::<Vec<_>>();
        let candidates = rank_candidates(&history, "echo");
        assert_eq!(candidates.len(), MAX_CANDIDATES);
        assert!(!rank_candidates(&["echo".into()], "echo").contains(&"echo".into()));
    }

    #[test]
    fn executed_command_moves_to_front_without_duplicates() {
        let mut state = CompletionState::new(CompletionSessionKey::new_for_test(7, "a"));
        state.replace_history(vec!["ls".into(), "pwd".into(), "ls".into()]);
        state.merge_executed("pwd");
        assert_eq!(state.history(), ["pwd", "ls"]);
    }

    #[test]
    fn escape_suppresses_until_the_next_user_edit() {
        let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "b"));
        state.replace_history(vec!["git status".into()]);
        state.refresh("git");
        assert!(state.is_popup_visible());
        state.dismiss();
        state.refresh("git");
        assert!(!state.is_popup_visible());
        state.on_user_edit();
        state.refresh("git");
        assert!(state.is_popup_visible());
    }
}
```

- [ ] **Step 2: Run the focused tests and verify they fail**

Run:

```bash
cd native-prototype
cargo test smart_completion::tests -- --nocapture
```

Expected: compilation fails because `parse_bash_history`, `rank_candidates`, `CompletionSessionKey`, and `CompletionState` are not defined.

- [ ] **Step 3: Implement the pure completion core**

Add these types and functions above the test module:

```rust
#[derive(Clone, PartialEq, Eq)]
pub struct CompletionSessionKey {
    pub generation: u64,
    token: String,
}

impl CompletionSessionKey {
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            token: uuid::Uuid::new_v4().simple().to_string(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(generation: u64, token: &str) -> Self {
        Self { generation, token: token.into() }
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

impl fmt::Debug for CompletionSessionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompletionSessionKey")
            .field("generation", &self.generation)
            .field("token", &"<redacted>")
            .finish()
    }
}

fn is_timestamp(line: &str) -> bool {
    line.strip_prefix('#')
        .is_some_and(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn safe_command(line: &str) -> Option<String> {
    let line = line.trim_end_matches('\r');
    (!line.is_empty() && !line.chars().any(char::is_control)).then(|| line.to_owned())
}

pub fn parse_bash_history(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let lines = text.lines().collect::<Vec<_>>();
    let timestamped = lines.iter().any(|line| is_timestamp(line));
    let mut oldest_first = Vec::new();

    if timestamped {
        let mut record = Vec::new();
        let flush = |record: &mut Vec<&str>, output: &mut Vec<String>| {
            if record.len() == 1 {
                if let Some(command) = safe_command(record[0]) {
                    output.push(command);
                }
            }
            record.clear();
        };
        for line in lines {
            if is_timestamp(line) {
                flush(&mut record, &mut oldest_first);
            } else {
                record.push(line);
            }
        }
        flush(&mut record, &mut oldest_first);
    } else {
        oldest_first.extend(lines.into_iter().filter_map(safe_command));
    }

    let mut seen = HashSet::new();
    oldest_first
        .into_iter()
        .rev()
        .filter(|command| seen.insert(command.clone()))
        .take(MAX_HISTORY_ITEMS)
        .collect()
}

pub fn rank_candidates(history: &[String], prefix: &str) -> Vec<String> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut prefix_matches = Vec::new();
    let mut contains_matches = Vec::new();
    for command in history {
        if command == prefix || !seen.insert(command.clone()) {
            continue;
        }
        if command.starts_with(prefix) {
            prefix_matches.push(command.clone());
        } else if command.contains(prefix) {
            contains_matches.push(command.clone());
        }
    }
    prefix_matches
        .into_iter()
        .chain(contains_matches)
        .take(MAX_CANDIDATES)
        .collect()
}

pub fn read_history_tail(path: &Path, max_bytes: u64) -> Result<Vec<u8>, String> {
    let mut file = File::open(path).map_err(|error| format!("无法读取历史文件: {error}"))?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    let start = length.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start)).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if start > 0 {
        if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=newline);
        } else {
            bytes.clear();
        }
    }
    Ok(bytes)
}

pub struct CompletionState {
    session: CompletionSessionKey,
    history: Vec<String>,
    candidates: Vec<String>,
    selected: usize,
    suppressed: bool,
    pending_fill: Option<u64>,
    history_path: Option<String>,
    sftp_ready: bool,
}

impl CompletionState {
    pub fn new(session: CompletionSessionKey) -> Self {
        Self {
            session,
            history: Vec::new(),
            candidates: Vec::new(),
            selected: 0,
            suppressed: false,
            pending_fill: None,
            history_path: None,
            sftp_ready: false,
        }
    }

    pub fn session(&self) -> &CompletionSessionKey { &self.session }
    pub fn history(&self) -> &[String] { &self.history }
    pub fn candidates(&self) -> &[String] { &self.candidates }
    pub fn selected(&self) -> usize { self.selected }
    pub fn selected_candidate(&self) -> Option<&str> {
        self.candidates.get(self.selected).map(String::as_str)
    }
    pub fn is_popup_visible(&self) -> bool {
        !self.suppressed && self.pending_fill.is_none() && !self.candidates.is_empty()
    }
    pub fn fill_pending(&self) -> bool { self.pending_fill.is_some() }
    pub fn replace_history(&mut self, history: Vec<String>) {
        let mut seen = HashSet::new();
        self.history = history
            .into_iter()
            .filter(|command| safe_command(command).is_some())
            .filter(|command| seen.insert(command.clone()))
            .take(MAX_HISTORY_ITEMS)
            .collect();
    }
    pub fn merge_executed(&mut self, command: &str) {
        if safe_command(command).is_none() { return; }
        self.history.retain(|entry| entry != command);
        self.history.insert(0, command.to_owned());
        self.history.truncate(MAX_HISTORY_ITEMS);
    }
    pub fn refresh(&mut self, prefix: &str) {
        self.candidates = rank_candidates(&self.history, prefix);
        self.selected = self.selected.min(self.candidates.len().saturating_sub(1));
    }
    pub fn move_selection(&mut self, delta: isize) {
        if self.candidates.is_empty() { return; }
        self.selected = (self.selected as isize + delta)
            .rem_euclid(self.candidates.len() as isize) as usize;
    }
    pub fn dismiss(&mut self) { self.suppressed = true; }
    pub fn on_user_edit(&mut self) {
        self.suppressed = false;
        self.pending_fill = None;
    }
    pub fn begin_fill(&mut self, request_id: u64) { self.pending_fill = Some(request_id); }
    pub fn finish_fill(&mut self, request_id: u64) -> bool {
        if self.pending_fill != Some(request_id) { return false; }
        self.pending_fill = None;
        self.suppressed = true;
        self.candidates.clear();
        true
    }
    pub fn fail_fill(&mut self, request_id: u64) {
        if self.pending_fill == Some(request_id) { self.pending_fill = None; }
    }
    pub fn cancel_pending_fill(&mut self) { self.pending_fill = None; }
    pub fn set_history_path(&mut self, path: String) { self.history_path = Some(path); }
    pub fn history_path(&self) -> Option<&str> { self.history_path.as_deref() }
    pub fn set_sftp_ready(&mut self, ready: bool) { self.sftp_ready = ready; }
    pub fn sftp_ready(&self) -> bool { self.sftp_ready }
}
```

- [ ] **Step 4: Add a bounded-tail test and run the focused suite**

Add:

```rust
#[test]
fn history_tail_drops_a_partial_first_line() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history");
    std::fs::write(&path, b"old-command\nnew-command\n").unwrap();
    assert_eq!(read_history_tail(&path, 15).unwrap(), b"new-command\n");
}
```

Run:

```bash
cd native-prototype
cargo fmt -- src/smart_completion.rs src/main.rs
cargo test smart_completion::tests -- --nocapture
```

Expected: all `smart_completion::tests` pass.

- [ ] **Step 5: Commit only the core module**

```bash
git add native-prototype/src/smart_completion.rs native-prototype/src/main.rs
git commit -m "feat: 添加 Bash 历史匹配核心"
```

### Task 2: Authenticated Private OSC Decoder

**Files:**
- Create: `native-prototype/src/bash_integration.rs`
- Modify: `native-prototype/src/main.rs:12-31`
- Modify: `native-prototype/Cargo.toml`
- Modify: `native-prototype/Cargo.lock`

- [ ] **Step 1: Add dependencies, module declaration, and failing decoder tests**

Add runtime dependencies:

```toml
base64 = "0.22"
tempfile = "3"
```

Add `mod bash_integration;` to `main.rs`. Create `bash_integration.rs` with:

```rust
use base64::Engine;
use crate::smart_completion::CompletionSessionKey;

pub const MAX_OSC_FRAME: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    Prompt,
    HistoryPath(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkerBoundary {
    pub end_offset: usize,
    pub kind: MarkerKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoder() -> MarkerDecoder {
        MarkerDecoder::new(CompletionSessionKey::new_for_test(9, "abc"))
    }

    #[test]
    fn marker_boundary_is_exclusive_and_supports_bel() {
        let mut decoder = decoder();
        let bytes = b"x\x1b]777;LiteTerm;abc;9;P\x07y";
        assert_eq!(
            decoder.scan(bytes),
            [MarkerBoundary { end_offset: bytes.len() - 1, kind: MarkerKind::Prompt }]
        );
    }

    #[test]
    fn split_st_terminated_history_marker_decodes_path() {
        let mut decoder = decoder();
        assert!(decoder.scan(b"\x1b]777;LiteTerm;abc;9;H;L2hvbWUvbWU").is_empty());
        assert_eq!(
            decoder.scan(b"\x1b\\tail"),
            [MarkerBoundary {
                end_offset: 2,
                kind: MarkerKind::HistoryPath("/home/me".into()),
            }]
        );
    }

    #[test]
    fn wrong_token_generation_and_oversized_frames_are_ignored_and_recover() {
        let mut decoder = decoder();
        assert!(decoder.scan(b"\x1b]777;LiteTerm;wrong;9;P\x07").is_empty());
        assert!(decoder.scan(b"\x1b]777;LiteTerm;abc;8;P\x07").is_empty());
        let oversized = format!("\x1b]777;{}\x07", "x".repeat(MAX_OSC_FRAME + 1));
        assert!(decoder.scan(oversized.as_bytes()).is_empty());
        assert_eq!(decoder.scan(b"\x1b]777;LiteTerm;abc;9;P\x07").len(), 1);
    }
}
```

- [ ] **Step 2: Run the decoder tests and verify failure**

Run:

```bash
cd native-prototype
cargo test bash_integration::tests -- --nocapture
```

Expected: compilation fails because `MarkerDecoder` is missing.

- [ ] **Step 3: Implement the bounded streaming decoder**

Add:

```rust
pub struct MarkerDecoder {
    session: CompletionSessionKey,
    ground_escape: bool,
    in_osc: bool,
    osc_escape: bool,
    overflow: bool,
    frame: Vec<u8>,
}

impl MarkerDecoder {
    pub fn new(session: CompletionSessionKey) -> Self {
        Self {
            session,
            ground_escape: false,
            in_osc: false,
            osc_escape: false,
            overflow: false,
            frame: Vec::with_capacity(96),
        }
    }

    fn reset_osc(&mut self) {
        self.in_osc = false;
        self.osc_escape = false;
        self.overflow = false;
        self.frame.clear();
    }

    fn push_frame_byte(&mut self, byte: u8) {
        if self.frame.len() == MAX_OSC_FRAME {
            self.overflow = true;
            self.frame.clear();
        } else if !self.overflow {
            self.frame.push(byte);
        }
    }

    fn finish(&mut self, end_offset: usize) -> Option<MarkerBoundary> {
        let parsed = (!self.overflow)
            .then(|| parse_marker(&self.frame, &self.session))
            .flatten()
            .map(|kind| MarkerBoundary { end_offset, kind });
        self.reset_osc();
        parsed
    }

    pub fn scan(&mut self, bytes: &[u8]) -> Vec<MarkerBoundary> {
        let mut boundaries = Vec::new();
        for (index, byte) in bytes.iter().copied().enumerate() {
            if !self.in_osc {
                if self.ground_escape {
                    self.ground_escape = false;
                    if byte == b']' {
                        self.in_osc = true;
                        self.frame.clear();
                    } else if byte == 0x1b {
                        self.ground_escape = true;
                    }
                } else if byte == 0x1b {
                    self.ground_escape = true;
                }
                continue;
            }

            if self.osc_escape {
                self.osc_escape = false;
                if byte == b'\\' {
                    if let Some(boundary) = self.finish(index + 1) {
                        boundaries.push(boundary);
                    }
                    continue;
                }
                self.push_frame_byte(0x1b);
            }

            match byte {
                0x07 => {
                    if let Some(boundary) = self.finish(index + 1) {
                        boundaries.push(boundary);
                    }
                }
                0x1b => self.osc_escape = true,
                value => self.push_frame_byte(value),
            }
        }
        boundaries
    }
}

fn parse_marker(frame: &[u8], session: &CompletionSessionKey) -> Option<MarkerKind> {
    let text = std::str::from_utf8(frame).ok()?;
    if text.chars().any(char::is_control) {
        return None;
    }
    let mut fields = text.split(';');
    if fields.next()? != "777"
        || fields.next()? != "LiteTerm"
        || fields.next()? != session.token()
        || fields.next()?.parse::<u64>().ok()? != session.generation
    {
        return None;
    }
    match fields.next()? {
        "P" if fields.next().is_none() => Some(MarkerKind::Prompt),
        "H" => {
            let payload = fields.next()?;
            if fields.next().is_some() { return None; }
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .ok()?;
            let path = String::from_utf8(bytes).ok()?;
            (!path.is_empty()
                && path.starts_with('/')
                && !path.chars().any(char::is_control))
                .then_some(MarkerKind::HistoryPath(path))
        }
        _ => None,
    }
}
```

- [ ] **Step 4: Run format and focused tests**

```bash
cd native-prototype
cargo fmt -- src/bash_integration.rs src/main.rs
cargo test bash_integration::tests -- --nocapture
```

Expected: all decoder tests pass, including split ST and recovery after oversized input.

- [ ] **Step 5: Commit**

```bash
git add native-prototype/Cargo.toml native-prototype/Cargo.lock native-prototype/src/bash_integration.rs native-prototype/src/main.rs
git commit -m "feat: 添加 Bash 会话标记解析"
```

### Task 3: Session-Only Bash RC and Secure Runtime Files

**Files:**
- Modify: `native-prototype/src/bash_integration.rs`
- Modify: `native-prototype/src/terminal.rs:56-143`
- Modify: `native-prototype/src/tab_manager.rs:16-70`

- [ ] **Step 1: Write failing RC and runtime tests**

Add to `bash_integration.rs` tests:

```rust
#[test]
fn rc_sources_user_config_preserves_prompt_command_and_installs_all_keymaps() {
    let identity = CompletionSessionKey::new_for_test(3, "abcd");
    let script = build_bash_rc(&identity, "/tmp/candidate", "\x1b[777;42~");
    assert!(script.contains("source \"$HOME/.bashrc\""));
    assert!(script.contains("PROMPT_COMMAND"));
    assert!(script.contains("emacs-standard"));
    assert!(script.contains("vi-insertion"));
    assert!(script.contains("vi-command"));
    assert!(script.contains("READLINE_LINE=$(<\"$__liteterm_candidate\")"));
    assert!(!script.contains("eval "));
    assert!(!script.contains("accept-line"));
}

#[test]
fn local_runtime_files_are_private_and_candidate_write_is_atomic() {
    use std::os::unix::fs::PermissionsExt;
    let session = CompletionSessionKey::new_for_test(4, "efgh");
    let runtime = LocalBashRuntime::create(session).unwrap();
    runtime.write_candidate("git status").unwrap();
    assert_eq!(std::fs::read(runtime.candidate_path()).unwrap(), b"git status");
    assert_eq!(
        std::fs::metadata(runtime.candidate_path()).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
```

- [ ] **Step 2: Verify the new tests fail**

```bash
cd native-prototype
cargo test bash_integration::tests -- --nocapture
```

Expected: missing `build_bash_rc` and `LocalBashRuntime`.

- [ ] **Step 3: Implement widget sequence, RC generation, and local runtime**

Add the following public API to `bash_integration.rs`. Use `std::os::unix::fs::OpenOptionsExt` for mode `0600`; `write_candidate` must write a sibling temporary file and `rename` it over the fixed candidate path.

```rust
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub fn is_bash_path(path: &str) -> bool {
    Path::new(path).file_name().is_some_and(|name| name == "bash")
}

pub fn widget_sequence(session: &CompletionSessionKey) -> String {
    let numeric = session
        .token()
        .get(..8)
        .and_then(|value| u32::from_str_radix(value, 16).ok())
        .unwrap_or(777);
    format!("\x1b[777;{numeric}~")
}

fn readline_literal(sequence: &str) -> String {
    sequence
        .bytes()
        .map(|byte| if byte == 0x1b { "\\e".into() } else { (byte as char).to_string() })
        .collect()
}

pub fn build_bash_rc(
    session: &CompletionSessionKey,
    candidate_path: &str,
    sequence: &str,
) -> String {
    let binding = readline_literal(sequence);
    format!(
        r#"
[[ -r "$HOME/.bashrc" ]] && source "$HOME/.bashrc"
readonly __liteterm_candidate='{candidate_path}'
__liteterm_widget() {{
    READLINE_LINE=$(<"$__liteterm_candidate")
    READLINE_POINT=${{#READLINE_LINE}}
}}
__liteterm_install() {{
    builtin bind -m emacs-standard -x '"{binding}":__liteterm_widget'
    builtin bind -m vi-insertion -x '"{binding}":__liteterm_widget'
    builtin bind -m vi-command -x '"{binding}":__liteterm_widget'
    local marker='\[\e]777;LiteTerm;{token};{generation};P\a\]'
    [[ "$PS1" == *"777;LiteTerm;{token};{generation};P"* ]] || PS1="${{PS1}}${{marker}}"
    if [[ -z "${{__liteterm_history_sent:-}}" ]]; then
        __liteterm_history_sent=1
        if command -v base64 >/dev/null && command -v tr >/dev/null; then
            local history_path="${{HISTFILE:-$HOME/.bash_history}}"
            local encoded
            encoded=$(printf %s "$history_path" | base64 | tr -d '\n=' | tr '+/' '-_')
            printf '\e]777;LiteTerm;{token};{generation};H;%s\a' "$encoded"
        fi
    fi
}}
if declare -p PROMPT_COMMAND 2>/dev/null | grep -q 'declare -a'; then
    PROMPT_COMMAND+=(__liteterm_install)
elif [[ -n "${{PROMPT_COMMAND:-}}" ]]; then
    PROMPT_COMMAND="${{PROMPT_COMMAND}};__liteterm_install"
else
    PROMPT_COMMAND=__liteterm_install
fi
"#,
        candidate_path = candidate_path,
        binding = binding,
        token = session.token(),
        generation = session.generation,
    )
}

pub struct LocalBashRuntime {
    _directory: tempfile::TempDir,
    session: CompletionSessionKey,
    rc_path: PathBuf,
    candidate_path: PathBuf,
    widget_sequence: String,
}

impl LocalBashRuntime {
    pub fn create(session: CompletionSessionKey) -> Result<Self, String> {
        let directory = tempfile::Builder::new()
            .prefix("liteterm-bash-")
            .tempdir()
            .map_err(|error| error.to_string())?;
        let rc_path = directory.path().join("session.bash");
        let candidate_path = directory.path().join("candidate");
        let sequence = widget_sequence(&session);
        let script = build_bash_rc(
            &session,
            candidate_path.to_string_lossy().as_ref(),
            &sequence,
        );
        for (path, bytes) in [(&rc_path, script.as_bytes()), (&candidate_path, b"".as_slice())] {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .map_err(|error| error.to_string())?;
            file.write_all(bytes).map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
        }
        Ok(Self {
            _directory: directory,
            session,
            rc_path,
            candidate_path,
            widget_sequence: sequence,
        })
    }

    pub fn session(&self) -> &CompletionSessionKey { &self.session }
    pub fn rc_path(&self) -> &Path { &self.rc_path }
    pub fn candidate_path(&self) -> &Path { &self.candidate_path }
    pub fn widget_sequence(&self) -> &str { &self.widget_sequence }

    pub fn write_candidate(&self, candidate: &str) -> Result<(), String> {
        if candidate.is_empty() || candidate.chars().any(char::is_control) {
            return Err("候选命令包含控制字符".into());
        }
        let temporary = self._directory.path().join("candidate.next");
        let _ = std::fs::remove_file(&temporary);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(candidate.as_bytes()).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        std::fs::rename(temporary, &self.candidate_path).map_err(|error| error.to_string())
    }
}
```

- [ ] **Step 4: Launch local Bash through the temporary RC**

Extend `TerminalState` with:

```rust
local_bash_runtime: Option<crate::bash_integration::LocalBashRuntime>,
```

Initialize it to `None`. Change `spawn_shell_with_path` to accept a `CompletionSessionKey`, create a runtime only for Bash, and add `--rcfile <path> -i`:

```rust
pub fn spawn_shell_with_path(
    &mut self,
    shell: &str,
    cols: u16,
    rows: u16,
    session: crate::smart_completion::CompletionSessionKey,
) {
    self.init_term(cols, rows);
    let runtime = crate::bash_integration::is_bash_path(shell)
        .then(|| crate::bash_integration::LocalBashRuntime::create(session))
        .transpose()
        .unwrap_or_else(|error| {
            log::warn!("Bash 集成初始化失败，使用普通 shell: {error}");
            None
        });
    let mut cmd = CommandBuilder::new(shell);
    cmd.env("TERM", "xterm-256color");
    if let Some(runtime) = &runtime {
        cmd.arg("--rcfile");
        cmd.arg(runtime.rc_path());
        cmd.arg("-i");
    }
    let pty_system = native_pty_system();
    let pty_pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("打开 PTY 失败");
    pty_pair.slave.spawn_command(cmd).expect("启动 shell 失败");
    let reader = pty_pair
        .master
        .try_clone_reader()
        .expect("克隆 PTY reader 失败");
    let writer = pty_pair.master.take_writer().expect("获取 PTY writer 失败");
    self.pty_reader = Some(reader);
    self.writer = Some(spawn_writer_worker(writer));
    self.pty_master = Some(pty_pair.master);
    self.local_bash_runtime = runtime;
}
```

Update `TabManager::new_local` to construct one `CompletionSessionKey`, initialize `CompletionState`, pass the cloned session into `spawn_shell_with_path`, and store the state on `Tab`:

```rust
let session = crate::smart_completion::CompletionSessionKey::new(1);
let completion = crate::smart_completion::CompletionState::new(session.clone());
term.spawn_shell_with_path(shell, cols, rows, session);
```

Add `pub completion: CompletionState` to `Tab`; initialize SSH placeholders with a fresh generation-1 state too.
Update the convenience `spawn_shell` method to create a fresh generation-1 session and pass it to the new signature, so no caller retains the old three-argument API.

- [ ] **Step 5: Run focused and existing terminal tests**

```bash
cd native-prototype
cargo fmt -- src/bash_integration.rs src/terminal.rs src/tab_manager.rs
cargo test bash_integration::tests -- --nocapture
cargo test terminal::tests -- --nocapture
```

Expected: Bash integration tests pass and the existing 9 terminal protocol tests remain green.

- [ ] **Step 6: Commit**

```bash
git add native-prototype/src/bash_integration.rs native-prototype/src/terminal.rs native-prototype/src/tab_manager.rs
git commit -m "feat: 注入临时 Bash 会话集成"
```

### Task 4: Alacritty-Synchronized Prompt Tracking and Input Extraction

**Files:**
- Modify: `native-prototype/src/terminal.rs`
- Modify: `native-prototype/src/bash_integration.rs`

- [ ] **Step 1: Write failing synchronized-boundary and grid tests**

Add terminal tests that feed one persistent `TestProcessor`:

```rust
#[test]
fn prompt_anchor_is_snapshotted_before_suffix_bytes() {
    let session = CompletionSessionKey::new_for_test(1, "abc");
    let mut state = terminal_with_completion(20, 4, session);
    let mut parser = TestProcessor::new();
    state.process_pty_output(
        &mut parser,
        b"$ \x1b]777;LiteTerm;abc;1;P\x07git",
    );
    assert_eq!(state.current_bash_input().as_deref(), Some("git"));
}

#[test]
fn marker_segmentation_preserves_every_original_byte() {
    let session = CompletionSessionKey::new_for_test(1, "abc");
    let mut decoder = MarkerDecoder::new(session);
    let bytes = b"before\x1b]777;LiteTerm;abc;1;P\x07after";
    let boundaries = decoder.scan(bytes);
    let mut rebuilt = Vec::new();
    let mut start = 0;
    for boundary in boundaries {
        rebuilt.extend_from_slice(&bytes[start..boundary.end_offset]);
        start = boundary.end_offset;
    }
    rebuilt.extend_from_slice(&bytes[start..]);
    assert_eq!(rebuilt, bytes);
}

#[test]
fn current_input_crosses_soft_wrap_and_skips_wide_spacer() {
    let session = CompletionSessionKey::new_for_test(1, "abc");
    let mut state = terminal_with_completion(6, 4, session);
    let mut parser = TestProcessor::new();
    state.process_pty_output(
        &mut parser,
        "$ \x1b]777;LiteTerm;abc;1;P\u{7}你好abc".as_bytes(),
    );
    assert_eq!(state.current_bash_input().as_deref(), Some("你好abc"));
}

#[test]
fn resize_and_hard_newline_invalidate_prompt() {
    let session = CompletionSessionKey::new_for_test(1, "abc");
    let mut state = terminal_with_completion(20, 4, session);
    let mut parser = TestProcessor::new();
    state.process_pty_output(&mut parser, b"$ \x1b]777;LiteTerm;abc;1;P\x07git\n");
    assert_eq!(state.current_bash_input(), None);
    state.process_pty_output(&mut parser, b"$ \x1b]777;LiteTerm;abc;1;P\x07git");
    state.resize(21, 4);
    assert_eq!(state.current_bash_input(), None);
}

#[test]
fn history_path_event_preserves_session_identity() {
    let session = CompletionSessionKey::new_for_test(2, "abc");
    let mut state = terminal_with_completion(20, 4, session.clone());
    let mut parser = TestProcessor::new();
    let events = state.process_pty_output(
        &mut parser,
        b"\x1b]777;LiteTerm;abc;2;H;L2hvbWUvbWU\x07",
    );
    assert_eq!(
        events,
        [IntegrationEvent::HistoryPath {
            session,
            path: "/home/me".into()
        }]
    );
}
```

- [ ] **Step 2: Run tests and verify failure**

```bash
cd native-prototype
cargo test terminal::tests -- --nocapture
```

Expected: missing prompt tracker APIs and event return type.

- [ ] **Step 3: Implement segmented parser advancement**

Add:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntegrationEvent {
    HistoryPath {
        session: crate::smart_completion::CompletionSessionKey,
        path: String,
    },
}

#[derive(Clone, Copy)]
struct LogicalPoint {
    absolute_line: i64,
    column: usize,
}

struct PromptTracking {
    session: crate::smart_completion::CompletionSessionKey,
    decoder: crate::bash_integration::MarkerDecoder,
    anchor: Option<LogicalPoint>,
}
```

Add `prompt_tracking: Option<PromptTracking>` to `TerminalState`. Configure it from the local runtime session and later from SSH runtime metadata. Replace `process_pty_output` with:

```rust
fn process_pty_output(
    &mut self,
    parser: &mut Processor,
    data: &[u8],
) -> Vec<IntegrationEvent> {
    let boundaries = self
        .prompt_tracking
        .as_mut()
        .map(|tracking| tracking.decoder.scan(data))
        .unwrap_or_default();
    let mut events = Vec::new();
    let mut start = 0;

    for boundary in boundaries {
        if let Some(term) = &mut self.term {
            parser.advance(term, &data[start..boundary.end_offset]);
        }
        start = boundary.end_offset;
        match boundary.kind {
            crate::bash_integration::MarkerKind::Prompt => self.snapshot_prompt_anchor(),
            crate::bash_integration::MarkerKind::HistoryPath(path) => {
                if let Some(tracking) = &self.prompt_tracking {
                    events.push(IntegrationEvent::HistoryPath {
                        session: tracking.session.clone(),
                        path,
                    });
                }
            }
        }
    }
    if let Some(term) = &mut self.term {
        parser.advance(term, &data[start..]);
    }
    self.invalidate_ambiguous_prompt();
    self.flush_pty_write_events();
    events
}
```

Implement `snapshot_prompt_anchor`, `current_bash_input`, `invalidate_prompt`, and `invalidate_ambiguous_prompt` using `history_size + grid.cursor.point.line` as the logical line. While extracting:

- return `None` in `TermMode::ALT_SCREEN`;
- require every intermediate line’s last cell to contain `Flags::WRAPLINE`;
- skip `WIDE_CHAR_SPACER` and `LEADING_WIDE_CHAR_SPACER`;
- append every character returned by `cell.zerowidth()` after the cell’s base character;
- stop before the current cursor column;
- reject embedded control characters and any out-of-range logical line.

Use this exact public surface:

```rust
pub fn current_bash_input(&self) -> Option<String>;
pub fn invalidate_prompt(&mut self);
pub fn take_bash_submission(&mut self) -> Option<String>;
```

`take_bash_submission` reads the current input first, then clears the anchor.

- [ ] **Step 4: Keep protocol replies separate from user-input classification**

Extract the existing channel send into:

```rust
fn enqueue_writer_bytes(&self, bytes: Vec<u8>) {
    if let Some(write_tx) = &self.writer {
        let _ = write_tx.send(bytes);
    }
}

pub fn write_input(&mut self, text: &str) {
    self.enqueue_writer_bytes(text.as_bytes().to_vec());
}
```

Make `flush_pty_write_events` call `enqueue_writer_bytes(text.into_bytes())` directly. Later user-input state changes happen in `main.rs`; Alacritty-generated DSR/CSI replies must never close or edit completion state.

- [ ] **Step 5: Return integration events from the read loop**

Change:

```rust
pub fn read_loop<R, I>(
    terminal: Arc<Mutex<TerminalState>>,
    request_redraw: R,
    integration_event: I,
)
where
    R: Fn() + Send + 'static,
    I: Fn(IntegrationEvent) + Send + 'static,
```

For each chunk, capture the returned events while holding the terminal lock, release the lock, send every event through `integration_event`, then request redraw. Update current call sites temporarily with a no-op integration callback until Task 5 wires `UserEvent`.

- [ ] **Step 6: Invalidate on real resize and verify all terminal tests**

Inside `resize`, after confirming dimensions changed, call `invalidate_prompt()` before resizing the grid.

Run:

```bash
cd native-prototype
cargo fmt -- src/terminal.rs src/bash_integration.rs src/main.rs
cargo test terminal::tests -- --nocapture
```

Expected: new prompt tests and all prior terminal protocol tests pass.

- [ ] **Step 7: Commit**

```bash
git add native-prototype/src/terminal.rs native-prototype/src/bash_integration.rs native-prototype/src/main.rs
git commit -m "feat: 跟踪 Bash 提示符输入区"
```

### Task 5: Local History Loading and Session-Safe Main-Thread Routing

**Files:**
- Modify: `native-prototype/src/main.rs`
- Modify: `native-prototype/src/tab_manager.rs`
- Modify: `native-prototype/src/smart_completion.rs`

- [ ] **Step 1: Write stale-event and history-merge tests**

Add pure helpers in `main.rs` tests:

```rust
#[test]
fn completion_event_matches_tab_generation_and_token() {
    let tab = test_tab_with_session(4, "current");
    assert!(completion_event_is_current(&tab, &CompletionSessionKey::new_for_test(4, "current")));
    assert!(!completion_event_is_current(&tab, &CompletionSessionKey::new_for_test(3, "current")));
    assert!(!completion_event_is_current(&tab, &CompletionSessionKey::new_for_test(4, "old")));
}

#[test]
fn first_fill_does_not_merge_history_but_submission_does() {
    let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "x"));
    state.replace_history(vec!["git status".into()]);
    state.begin_fill(10);
    assert_eq!(state.history(), ["git status"]);
    state.finish_fill(10);
    state.merge_executed("git log");
    assert_eq!(state.history(), ["git log", "git status"]);
}
```

- [ ] **Step 2: Add typed completion events**

Extend `UserEvent`:

```rust
CompletionHistory {
    tab_id: String,
    session: smart_completion::CompletionSessionKey,
    result: Result<Vec<u8>, String>,
},
TerminalIntegration {
    tab_id: String,
    event: terminal::IntegrationEvent,
},
```

Update its `Debug` implementation without printing tokens or history contents.

- [ ] **Step 3: Wire tab-aware terminal integration events**

Change `start_read_loop` to accept `tab_id`:

```rust
fn start_read_loop(&self, tab_id: String, terminal: Arc<Mutex<TerminalState>>) {
    let redraw_proxy = self.proxy.clone();
    let event_proxy = self.proxy.clone();
    let event_tab = tab_id.clone();
    std::thread::spawn(move || {
        terminal::read_loop(
            terminal,
            move || { let _ = redraw_proxy.send_event(UserEvent::Redraw); },
            move |event| {
                let _ = event_proxy.send_event(UserEvent::TerminalIntegration {
                    tab_id: event_tab.clone(),
                    event,
                });
            },
        );
    });
}
```

Pass the actual tab ID from local creation and `SshReady`.

- [ ] **Step 4: Load local history asynchronously**

Add:

```rust
fn request_local_history(
    &self,
    tab_id: String,
    session: CompletionSessionKey,
    path: std::path::PathBuf,
) {
    let proxy = self.proxy.clone();
    std::thread::spawn(move || {
        let result = smart_completion::read_history_tail(
            &path,
            smart_completion::MAX_HISTORY_BYTES,
        );
        let _ = proxy.send_event(UserEvent::CompletionHistory {
            tab_id,
            session,
            result,
        });
    });
}
```

After creating a local Bash tab, save the absolute default `~/.bash_history` in `CompletionState` and request it. On a valid `IntegrationEvent::HistoryPath`, validate the event session, update `CompletionState::history_path`, and request that path if it differs. On `CompletionHistory`, require tab/session equality before parsing and replacing history. If the history file is absent or unreadable, keep an empty history without changing terminal availability.

Add:

```rust
fn completion_event_is_current(
    tab: &tab_manager::Tab,
    session: &CompletionSessionKey,
) -> bool {
    tab.completion.session() == session
}
```

- [ ] **Step 5: Refresh candidates from the actual terminal grid**

Before starting the egui frame in `do_render`, read `TerminalState::current_bash_input()` for the active tab. Call `tab.completion.refresh(&prefix)` when `Some(prefix)` and clear candidates on `None` or empty input. Add `CompletionState::clear_candidates()` as:

```rust
pub fn clear_candidates(&mut self) {
    self.candidates.clear();
    self.selected = 0;
}
```

- [ ] **Step 6: Verify focused and full Native unit tests**

```bash
cd native-prototype
cargo fmt -- src/main.rs src/tab_manager.rs src/smart_completion.rs
cargo test smart_completion::tests -- --nocapture
cargo test terminal::tests -- --nocapture
cargo test
```

Expected: all Native tests pass; no token appears in debug output.

- [ ] **Step 7: Commit**

```bash
git add native-prototype/src/main.rs native-prototype/src/tab_manager.rs native-prototype/src/smart_completion.rs
git commit -m "feat: 异步加载本地 Bash 历史"
```

### Task 6: SSH Bash Detection, Integrated Launch, and Plain-Shell Fallback

**Files:**
- Modify: `native-prototype/src/ssh.rs`
- Modify: `native-prototype/src/bash_integration.rs`
- Modify: `native-prototype/src/terminal.rs`
- Modify: `native-prototype/src/tab_manager.rs`
- Modify: `native-prototype/src/main.rs`

- [ ] **Step 1: Write pure SSH integration tests**

Add to `ssh.rs` tests:

```rust
#[test]
fn only_a_bash_basename_enables_integration() {
    assert!(is_bash_path("/bin/bash"));
    assert!(is_bash_path("/usr/local/bin/bash"));
    assert!(!is_bash_path("/usr/bin/fish"));
    assert!(!is_bash_path("bash -l"));
}

#[test]
fn remote_paths_are_token_scoped_and_shell_safe() {
    let session = CompletionSessionKey::new_for_test(7, "abcdef");
    let paths = RemoteBashPaths::new(&session);
    assert_eq!(paths.rc, "/tmp/liteterm-native-abcdef-7.rc");
    assert_eq!(paths.candidate, "/tmp/liteterm-native-abcdef-7.candidate");
    assert!(paths.launch_command("/bin/bash").contains("--rcfile"));
    assert!(!paths.launch_command("/bin/bash").contains('\n'));
}

#[test]
fn failed_integration_decision_falls_back_to_plain_shell() {
    assert_eq!(
        choose_shell_mode(Ok("/bin/bash".into()), Err("sftp disabled".into())),
        ShellChoice::Plain
    );
    assert_eq!(
        choose_shell_mode(Ok("/bin/fish".into()), Ok(())),
        ShellChoice::Plain
    );
}

#[test]
fn ssh_shutdown_signal_is_independent_from_terminal_input() {
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
    shutdown_tx.send(()).unwrap();
    assert!(shutdown_rx.try_recv().is_ok());
}
```

- [ ] **Step 2: Define remote runtime metadata**

In `bash_integration.rs`, add:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteBashRuntime {
    pub session: CompletionSessionKey,
    pub candidate_path: String,
    pub widget_sequence: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteBashPaths {
    pub rc: String,
    pub candidate: String,
}

impl RemoteBashPaths {
    pub fn new(session: &CompletionSessionKey) -> Self {
        let stem = format!("liteterm-native-{}-{}", session.token(), session.generation);
        Self {
            rc: format!("/tmp/{stem}.rc"),
            candidate: format!("/tmp/{stem}.candidate"),
        }
    }

    pub fn launch_command(&self, bash: &str) -> String {
        format!(
            "umask 077; trap 'rm -f -- \"{}\" \"{}\"' EXIT HUP INT TERM; '{}' --rcfile '{}' -i",
            self.rc, self.candidate, bash, self.rc
        )
    }
}
```

Only accept probed shell paths that are absolute, contain no control/quote characters, and have basename `bash`.

- [ ] **Step 3: Probe the login shell and prepare files through the authenticated session**

Add the pure decision helper used by the tests:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellChoice {
    Plain,
    IntegratedBash,
}

fn choose_shell_mode(
    probe: Result<String, String>,
    preparation: Result<(), String>,
) -> ShellChoice {
    match (probe, preparation) {
        (Ok(shell), Ok(())) if crate::bash_integration::is_bash_path(&shell) => {
            ShellChoice::IntegratedBash
        }
        _ => ShellChoice::Plain,
    }
}
```

Change `ssh::connect` to:

```rust
pub fn connect(
    params: &ConnectionParams,
    cols: u16,
    rows: u16,
    integration: Option<CompletionSessionKey>,
) -> Result<SshHandle, String>
```

Before opening the final PTY channel:

1. Call `session.set_timeout(3_000)`, open a short-lived exec channel for `printf '%s' "$SHELL"`, read at most 4096 bytes, close the probe channel, then restore `session.set_timeout(0)`.
2. Reject empty, relative, non-Bash, quoted or control-containing shell paths.
3. If it is Bash, use `session.sftp()` to create the RC and empty candidate paths with `WRITE | CREATE | EXCLUSIVE`, mode `0600`.
4. Write `build_bash_rc` to the RC path and sync/close both handles.
5. Request PTY on a new channel and call `channel.exec(paths.launch_command(shell))`.
6. On any failure, remove created paths best-effort, open another channel, request the same PTY, and call `channel.shell()`.

Add to `SshHandle`:

```rust
pub bash_runtime: Option<crate::bash_integration::RemoteBashRuntime>,
pub shutdown_tx: mpsc::Sender<()>,
```

Create a matching `shutdown_rx` before spawning the I/O thread. Check it at the top of every loop; when signaled, switch the session to blocking mode, call `channel.close()`, and break. Keep the existing `write_tx`/`resize_tx` behavior.

- [ ] **Step 4: Apply remote protocol metadata to the exact tab**

In `TabManager::apply_ssh`, reject only a `Some(bash_runtime)` whose session does not equal the tab’s current completion session; a `None` runtime is the valid plain-shell fallback and must still apply. Add

```rust
remote_bash_runtime: Option<crate::bash_integration::RemoteBashRuntime>,
```

to `TerminalState`. In `TerminalState::apply_ssh_handle`, initialize `MarkerDecoder` from `handle.bash_runtime`, move that metadata into `remote_bash_runtime`, and retain the remote candidate path/widget sequence for fill requests. Local terminals continue using `local_bash_runtime`; the two options are mutually exclusive.

Pass the placeholder tab’s cloned session into the background `ssh::connect` call. Extend `UserEvent::SshReady` with that session and reject stale results before saving credentials or starting SFTP.

- [ ] **Step 5: Close SSH channels before removing tabs**

Add `ssh_shutdown_tx: Option<mpsc::Sender<()>>` to `TerminalState`, initialize it from `SshHandle`, and expose:

```rust
pub fn shutdown(&mut self) {
    if let Some(shutdown_tx) = self.ssh_shutdown_tx.take() {
        let _ = shutdown_tx.send(());
    }
    self.writer = None;
    self.ssh_resize_tx = None;
}
```

In `App::close_tab`, lock the target terminal and call `shutdown()` before `TabManager::close`. Apply the same operation to every removed tab in `close_other_tabs`. On `WindowEvent::CloseRequested`, call `shutdown()` for all tabs, send `SftpCommand::Shutdown` to every worker, and only then call `event_loop.exit()`. This guarantees the remote wrapper gets EOF/close and can remove its temporary RC and candidate files.

- [ ] **Step 6: Run tests**

```bash
cd native-prototype
cargo fmt -- src/ssh.rs src/bash_integration.rs src/terminal.rs src/tab_manager.rs src/main.rs
cargo test ssh::tests -- --nocapture
cargo test bash_integration::tests -- --nocapture
cargo test terminal::tests -- --nocapture
```

Expected: pure SSH decisions pass; no real network is needed.

- [ ] **Step 7: Commit**

```bash
git add native-prototype/src/ssh.rs native-prototype/src/bash_integration.rs native-prototype/src/terminal.rs native-prototype/src/tab_manager.rs native-prototype/src/main.rs
git commit -m "feat: 接入远端 Bash 会话集成"
```

### Task 7: Remote History and Atomic Candidate SFTP Operations

**Files:**
- Modify: `native-prototype/src/sftp.rs`
- Modify: `native-prototype/src/main.rs`
- Modify: `native-prototype/src/terminal.rs`
- Modify: `native-prototype/src/smart_completion.rs`

- [ ] **Step 1: Write failing tail-range, event identity, and atomic-path tests**

Add:

```rust
#[test]
fn remote_tail_start_is_bounded() {
    assert_eq!(history_tail_start(100, 20), 80);
    assert_eq!(history_tail_start(10, 20), 0);
}

#[test]
fn candidate_temporary_path_stays_beside_target() {
    assert_eq!(
        candidate_temporary_path("/tmp/liteterm.candidate", 42).unwrap(),
        "/tmp/.liteterm.candidate.42.tmp"
    );
    assert!(candidate_temporary_path("/", 1).is_err());
}

#[test]
fn completion_event_keeps_session_and_request_identity() {
    let session = CompletionSessionKey::new_for_test(3, "token");
    let event = SftpEvent::CompletionCandidateWritten {
        tab_id: "tab".into(),
        session: session.clone(),
        request_id: 9,
        result: Ok(()),
    };
    assert_eq!(event.completion_session(), Some(&session));
}
```

- [ ] **Step 2: Add SFTP commands and events**

Add:

```rust
ReadCompletionHistory {
    session: CompletionSessionKey,
    path: String,
    max_bytes: u64,
},
WriteCompletionCandidate {
    session: CompletionSessionKey,
    request_id: u64,
    path: String,
    bytes: Vec<u8>,
},
```

and:

```rust
CompletionHistoryRead {
    tab_id: String,
    session: CompletionSessionKey,
    result: Result<Vec<u8>, String>,
},
CompletionCandidateWritten {
    tab_id: String,
    session: CompletionSessionKey,
    request_id: u64,
    result: Result<(), String>,
},
```

Update every exhaustive `SftpEvent` match in `main.rs` and tests.
Add:

```rust
impl SftpEvent {
    pub fn completion_session(&self) -> Option<&CompletionSessionKey> {
        match self {
            Self::CompletionHistoryRead { session, .. }
            | Self::CompletionCandidateWritten { session, .. } => Some(session),
            _ => None,
        }
    }
}
```

- [ ] **Step 3: Implement bounded remote reading**

Use `Sftp::stat`, `ssh2::File::seek`, and `read_to_end`:

```rust
fn history_tail_start(length: u64, max_bytes: u64) -> u64 {
    length.saturating_sub(max_bytes)
}

fn read_remote_history_tail(
    sftp: &ssh2::Sftp,
    path: &Path,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    let length = sftp.stat(path)
        .map_err(|error| format!("无法读取远端历史属性: {error}"))?
        .size
        .unwrap_or(0);
    let start = history_tail_start(length, max_bytes);
    let mut file = sftp.open(path).map_err(|error| format!("无法打开远端历史: {error}"))?;
    file.seek(SeekFrom::Start(start)).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if start > 0 {
        if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=newline);
        } else {
            bytes.clear();
        }
    }
    Ok(bytes)
}
```

- [ ] **Step 4: Implement atomic remote candidate writes**

Validate candidate bytes with the same nonempty/no-control rule. Create a sibling request-scoped path with `WRITE | CREATE | EXCLUSIVE`, mode `0600`, write and close it, then `sftp.rename(temp, target, Some(RenameFlags::OVERWRITE))`. Remove the temporary file on every error.

```rust
fn candidate_temporary_path(path: &str, request_id: u64) -> Result<String, String> {
    let path = Path::new(path);
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "候选路径缺少父目录".to_string())?;
    let name = path.file_name().and_then(|name| name.to_str())
        .ok_or_else(|| "候选文件名无效".to_string())?;
    Ok(parent.join(format!(".{name}.{request_id}.tmp")).to_string_lossy().into_owned())
}
```

Add both command branches to the existing serial SFTP worker and emit the typed completion events.

- [ ] **Step 5: Join SFTP Ready and HISTFILE arrival in either order**

In `main.rs`, add `maybe_request_remote_history(tab_id)`. It sends `ReadCompletionHistory` only when:

- the current tab is SSH;
- `CompletionState::sftp_ready()` is true;
- the current session has a validated history path;
- the worker exists.

Call it after both `SftpEvent::Ready` and `TerminalIntegration::HistoryPath`. Each `Sftp::Reconnect` Ready event triggers a reload.

On `SftpEvent::Ready`, mark SFTP ready and, if no OSC history path has arrived, set `<ready.home>/.bash_history` as the remote fallback. Before sending `SftpCommand::Reconnect`, set `sftp_ready` false. On `CompletionHistoryRead`, require exact tab/session equality, parse with `parse_bash_history`, and replace history. Ignore stale results.

- [ ] **Step 6: Stage and commit remote completion fills**

Define a unified request:

```rust
pub enum CandidateWriteTarget {
    Local(std::path::PathBuf),
    Remote(String),
}

pub struct CandidateWriteRequest {
    pub session: CompletionSessionKey,
    pub target: CandidateWriteTarget,
    pub bytes: Vec<u8>,
}

pub fn stage_completion_fill(&mut self, candidate: &str)
    -> Result<CandidateWriteRequest, String>;
pub fn commit_completion_fill(&mut self) -> bool;
```

`stage_completion_fill` only validates/clones metadata while the terminal is locked; it performs no file I/O and sends no terminal bytes. The main thread assigns a request ID, calls `CompletionState::begin_fill`, and dispatches:

- `Local(path)`: a background thread calls a static `write_local_candidate_atomic(path, bytes)` helper and emits `UserEvent::CompletionCandidateWritten { tab_id, session, request_id, result }`;
- `Remote(path)`: send `SftpCommand::WriteCompletionCandidate`.

Add `completion_request_id: u64` to `App`, initialize it to zero, and allocate IDs with:

```rust
fn next_completion_request_id(&mut self) -> u64 {
    self.completion_request_id = self.completion_request_id.wrapping_add(1).max(1);
    self.completion_request_id
}
```

Add the local completion event to `UserEvent` with exactly those four fields plus `result: Result<(), String>`, and redact its session token in `Debug`.

Expose the local writer from `bash_integration.rs` and make `LocalBashRuntime::write_candidate` delegate to it:

```rust
pub fn write_local_candidate_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.iter().any(|byte| byte.is_ascii_control()) {
        return Err("候选命令包含控制字符".into());
    }
    let parent = path.parent().ok_or_else(|| "候选路径缺少父目录".to_string())?;
    let temporary = parent.join("candidate.next");
    let _ = std::fs::remove_file(&temporary);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        error.to_string()
    })
}
```

On either matching successful event, lock the original tab terminal, call `commit_completion_fill()`, and only then call `finish_fill(request_id)`. `commit_completion_fill` sends only the private widget sequence. On failure, call `fail_fill` and leave the Bash line unchanged.

The completion event handler must match all four values: tab ID, generation/token session key, and request ID. Switching away from a tab or closing it cancels its pending request so a late success cannot inject bytes.

- [ ] **Step 7: Run tests**

```bash
cd native-prototype
cargo fmt -- src/sftp.rs src/main.rs src/terminal.rs src/smart_completion.rs
cargo test sftp::tests -- --nocapture
cargo test sftp::worker_tests -- --nocapture
cargo test smart_completion::tests -- --nocapture
cargo test
```

Expected: all tests pass; SFTP failures never send a widget sequence.

- [ ] **Step 8: Commit**

```bash
git add native-prototype/src/sftp.rs native-prototype/src/main.rs native-prototype/src/terminal.rs native-prototype/src/smart_completion.rs
git commit -m "feat: 读取远端 Bash 历史并安全填充"
```

### Task 8: Cursor-Anchored Popup and Keyboard Behavior

**Files:**
- Create: `native-prototype/src/completion_popup.rs`
- Modify: `native-prototype/src/main.rs`
- Modify: `native-prototype/src/renderer.rs`
- Modify: `native-prototype/src/smart_completion.rs`
- Modify: `native-prototype/src/terminal.rs`

- [ ] **Step 1: Write failing popup geometry tests**

Create `completion_popup.rs`:

```rust
pub const ROW_HEIGHT: f32 = 26.0;
pub const POPUP_WIDTH: f32 = 420.0;
pub const POPUP_MARGIN: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopupGeometry {
    pub position: egui::Pos2,
    pub size: egui::Vec2,
    pub opens_above: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_opens_below_cursor_when_space_allows() {
        let bounds = egui::Rect::from_min_size(egui::pos2(220.0, 34.0), egui::vec2(800.0, 600.0));
        let cursor = egui::Rect::from_min_size(egui::pos2(400.0, 100.0), egui::vec2(10.0, 20.0));
        let geometry = popup_geometry(bounds, cursor, 4);
        assert!(!geometry.opens_above);
        assert!(geometry.position.y >= cursor.bottom());
    }

    #[test]
    fn popup_flips_above_and_clamps_inside_terminal() {
        let bounds = egui::Rect::from_min_size(egui::pos2(220.0, 34.0), egui::vec2(300.0, 160.0));
        let cursor = egui::Rect::from_min_size(egui::pos2(500.0, 170.0), egui::vec2(10.0, 20.0));
        let geometry = popup_geometry(bounds, cursor, 8);
        assert!(geometry.opens_above);
        let rect = egui::Rect::from_min_size(geometry.position, geometry.size);
        assert!(bounds.contains_rect(rect));
    }
}
```

- [ ] **Step 2: Implement geometry and non-interactive rendering**

Implement `popup_geometry` with height `ROW_HEIGHT * candidate_count`, width clamped to terminal width minus margins, x clamped to bounds, and below-first/above-on-overflow placement.

Expose:

```rust
pub fn render(
    ctx: &egui::Context,
    tab_id: &str,
    bounds: egui::Rect,
    cursor: egui::Rect,
    candidates: &[String],
    selected: usize,
);
```

Use `egui::Area` with `Order::Foreground` and `.interactable(false)` so the popup cannot take focus or intercept terminal mouse input. Paint background `#1c2028`, border `#30363d`, 26 px rows, normal text `#c9d1d9`, and a full-row selected fill `#30363d`. Truncate visual text with painter clipping; never alter the candidate string. Give the area ID `("bash_completion", tab_id)`. Do not add a fullscreen backdrop.

- [ ] **Step 3: Write keyboard routing tests**

Extract a pure route:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompletionKeyAction {
    Previous,
    Next,
    Accept,
    Dismiss,
    WaitForFill,
    PassThrough,
}

fn completion_key_action(
    key: &Key,
    modifiers: winit::keyboard::ModifiersState,
    popup_visible: bool,
    fill_pending: bool,
    egui_wants_keyboard: bool,
    has_dialog: bool,
) -> CompletionKeyAction;
```

Tests:

```rust
#[test]
fn arrows_and_enter_are_only_captured_for_a_visible_popup() {
    assert_eq!(
        completion_key_action(
            &Key::Named(NamedKey::ArrowDown),
            ModifiersState::empty(),
            true,
            false,
            false,
            false,
        ),
        CompletionKeyAction::Next
    );
    assert_eq!(
        completion_key_action(
            &Key::Named(NamedKey::Enter),
            ModifiersState::empty(),
            true,
            false,
            false,
            false,
        ),
        CompletionKeyAction::Accept
    );
    assert_eq!(
        completion_key_action(
            &Key::Named(NamedKey::ArrowDown),
            ModifiersState::empty(),
            false,
            false,
            false,
            false,
        ),
        CompletionKeyAction::PassThrough
    );
}
```

Add assertions that Tab, Ctrl+Tab, a focused command bar, and an open dialog all return `PassThrough`.
Also assert that Enter with `fill_pending = true` returns `WaitForFill`, so a fast second Enter cannot execute the old prefix while the candidate file is still being written.

- [ ] **Step 4: Intercept popup keys before egui**

Immediately after synthetic/release filtering and before sending the event to egui, handle completion only when there is no Ctrl/Alt/Super modifier, no dialog and no egui keyboard focus:

- Up/Down call `move_selection(-1/+1)`;
- Escape calls `dismiss`;
- Enter clones the selected candidate and calls `stage_completion_fill`;
- Enter during an already pending fill is consumed as `WaitForFill` and sends no terminal bytes;
- return after redraw for all captured actions.

Do not send `\r`, `\n`, End, Ctrl-U, macros, or candidate text through the terminal writer during this first Enter.

When no popup captures Enter, call `take_bash_submission()` before the existing `"\r"` write and merge the returned nonempty command into the current tab history. For Ctrl-J, Ctrl-M, pasted CR/LF and unknown control sequences, invalidate the prompt before forwarding. Known printable input and Readline editing keys call `CompletionState::on_user_edit()`.

- [ ] **Step 5: Expose renderer-owned cursor geometry**

Add to `renderer.rs`:

```rust
pub fn viewport_rect(&self) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(self.viewport_x, self.viewport_y),
        egui::vec2(self.viewport_width, self.viewport_height),
    )
}

pub fn cursor_screen_rect(&self, terminal: &TerminalState) -> Option<egui::Rect> {
    let term = terminal.term()?;
    let point = term.grid().cursor.point;
    let display_offset = term.grid().display_offset() as i32;
    let visual_row = point.line.0 + display_offset + 1;
    let (_, rows) = self.calculate_grid_size();
    if visual_row < 0 || visual_row >= i32::from(rows) {
        return None;
    }
    Some(egui::Rect::from_min_size(
        egui::pos2(
            self.viewport_x + point.column.0 as f32 * self.atlas.cell_width,
            self.viewport_y + visual_row as f32 * self.atlas.cell_height,
        ),
        egui::vec2(self.atlas.cell_width, self.atlas.cell_height),
    ))
}
```

Add renderer tests comparing this rect with the existing cursor formula for display offset 0 and confirming a non-visible scrollback cursor returns `None`.

- [ ] **Step 6: Render from a lock-free view snapshot**

Before `egui_ctx.run`, briefly lock the active terminal, clone the candidates/selected index and obtain `Renderer::cursor_screen_rect`; release the terminal lock before entering egui. Use `Renderer::viewport_rect` as terminal bounds. Render after sidebar/file browser/tab bar and before modal dialogs.

When switching tabs, call `CompletionState::cancel_pending_fill()` on the old active tab before changing `active_idx`; any late result then fails the request-ID check and cannot inject into the terminal.

- [ ] **Step 7: Verify UI and keyboard tests**

```bash
cd native-prototype
cargo fmt -- src/completion_popup.rs src/main.rs src/renderer.rs src/smart_completion.rs src/terminal.rs
cargo test completion_popup::tests -- --nocapture
cargo test layout_tests -- --nocapture
cargo test smart_completion::tests -- --nocapture
cargo test
```

Expected: popup geometry always stays inside terminal bounds; arrows pass through when hidden; first Enter contains no execute byte.

- [ ] **Step 8: Commit**

```bash
git add native-prototype/src/completion_popup.rs native-prototype/src/main.rs native-prototype/src/renderer.rs native-prototype/src/smart_completion.rs native-prototype/src/terminal.rs
git commit -m "feat: 添加 Bash 智能补全弹窗"
```

### Task 9: Integration Regression, Native Build, and Manual Acceptance

**Files:**
- Modify only if a failing test identifies a scoped defect in files from Tasks 1-8.
- Do not modify root `build.sh`, root `run.sh`, or old GuiShell sources.

- [ ] **Step 1: Run formatting and all unit tests**

```bash
cd native-prototype
cargo fmt --check
cargo test
cargo clippy --all-targets
```

Expected: all tests pass; Clippy exits 0. Record pre-existing warnings separately rather than broad cleanup.

- [ ] **Step 2: Run the isolated Native build**

From repository root:

```bash
./native-prototype/build.sh
```

Expected: Native build, Clippy, and full Native test suite succeed without rebuilding or replacing the old GuiShell binary.

- [ ] **Step 3: Launch only the new Native binary**

Use the existing isolated launcher:

```bash
./run-native.sh
```

Do not stop the old GuiShell process. Confirm the running executable name remains `liteterm-native`.

- [ ] **Step 4: Manually verify local Bash**

1. Open a local Bash tab with a populated `~/.bash_history`.
2. Type a nonempty prefix and confirm prefix matches precede contains matches.
3. Use Up/Down and press Enter once; verify the candidate appears but no command output begins.
4. Press Enter a second time; verify execution and immediate in-memory history reuse.
5. Verify Backspace, Left/Right, Home/End, paste, Unicode, and wrapped lines.
6. Run `fish` or `zsh`; confirm the popup stays disabled until a new integrated Bash prompt.
7. Test Emacs mode, `set -o vi`, and user bindings mapping End/Ctrl-U to `accept-line`; first Enter must still not execute.

- [ ] **Step 5: Manually verify SSH Bash and failure fallback**

1. Connect to a Bash SSH target; verify remote history suggestions.
2. Fill and execute a remote command using two Enter presses.
3. Trigger SFTP reconnect; verify history reloads and the terminal remains interactive.
4. Connect to Fish/Zsh or a target with SFTP disabled; verify the ordinary terminal opens without completion and without connection failure.
5. Close the tab during a history read and during a candidate write; verify no stale event writes to another tab.

- [ ] **Step 6: Inspect process and file cleanup**

Verify local runtime directories and remote `/tmp/liteterm-native-<token>-<generation>.*` files disappear when their Bash session exits normally. Confirm failure paths leave no reusable world-readable file and no token is logged.

- [ ] **Step 7: Run final diff safety checks**

```bash
git diff --check
git diff --cached --check
git status --short
git diff --name-only HEAD
```

Expected: no whitespace errors, no unrelated staged files, root launch/build scripts unchanged.

- [ ] **Step 8: Commit any scoped integration fix**

If Steps 1-7 required a fix, stage only its exact Native files and commit:

```bash
git commit -m "fix: 完善 Bash 智能补全回归"
```

If no fix was needed, do not create an empty commit.

## Final Review Gate

Before reporting completion:

1. A specification reviewer checks every requirement in `docs/superpowers/specs/2026-07-24-native-bash-smart-completion-design.md`.
2. A code-quality reviewer checks parser uniqueness, thread/lock boundaries, token redaction, SFTP atomicity, widget safety, and unrelated dirty-file preservation.
3. Run `verification-before-completion`; report exact test counts and `./native-prototype/build.sh` result.
4. Do not merge, tag, push, or stop either running application without explicit user approval.
