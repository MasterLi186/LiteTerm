# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

GuiShell is a FinalShell replacement — a lightweight Linux SSH client built with Tauri 2 (Rust backend) + React/TypeScript (frontend) + xterm.js (terminal emulation). All UI text is Chinese.

There is also a legacy GTK4 version in the root `src/` Rust files and root `Cargo.toml` — this is superseded by the Tauri version but kept for its test suite.

## Build & Run

```bash
./run.sh              # Auto-builds frontend+backend if stale, then launches
./build.sh dev        # Quick build (frontend + backend, no bundling)
./build.sh release    # Production bundle with installer
./build.sh debug      # Debug build with devtools
```

Manual build steps:
```bash
npm run build                      # Frontend → dist/
cd src-tauri && cargo build        # Backend → src-tauri/target/debug/guishell-tauri
```

Do NOT use `npx tauri dev` — the system hits file descriptor limits with the file watcher. Always build then run the binary directly.

## Tests

Tests are for the Rust core layer only (the legacy root Cargo.toml):
```bash
cargo test                                    # All tests (runs from project root against root Cargo.toml)
cargo test --test zmodem_frame_test           # Single test file
cargo test --test monitor_parse_test -v       # Verbose single test
```

Test files are in `tests/` and test modules from `src/` (the GTK4 era code). The same core modules are copied into `src-tauri/src/` and share identical logic.

## Architecture

### Two Codebases Coexist

| Path | Purpose | Status |
|------|---------|--------|
| `src/*.rs` + root `Cargo.toml` | Legacy GTK4 version | Tests only |
| `src-tauri/` | Tauri 2 Rust backend | Active |
| `src/*.tsx` + `index.html` | React frontend | Active |

### Tauri Backend (`src-tauri/src/`)

```
lib.rs          — Tauri app setup, command registration, AppState initialization
state.rs        — AppState (Mutex-wrapped HashMaps for sessions, terminals, connections, settings, sftp)
commands/       — Tauri IPC command handlers (one file per domain)
  terminal.rs   — Local shell via portable-pty (open, write, resize, close)
  ssh.rs        — SSH connect (auth, shell channel, non-blocking read loop)
  connection.rs — CRUD for saved connections (TOML persistence)
  keyring.rs    — GNOME Keyring store/retrieve via secret-service
  monitor.rs    — System metrics collection over SSH (separate connection per session)
  sftp.rs       — SFTP file operations (separate connection per session)
config/         — Data models with serde (Settings, ConnectionStore, KeyringEntry)
core/           — Business logic, no UI dependency
  ssh.rs        — SshConnection wrapper (connect, authenticate, open_shell_channel)
  session.rs    — SessionState machine (Connecting → Connected → Disconnected)
  monitor.rs    — /proc parsers (CPU, memory, disk, network, load), MetricBuffer ring buffer
  sftp.rs       — SftpOps wrapper for ssh2::Sftp
  transfer.rs   — TransferQueue with concurrency control
  zmodem/       — ZMODEM protocol (frame codec, byte stream detector, send/receive state machines)
plugin/         — MetricPlugin trait and PluginRegistry (extensible monitoring)
```

### Key Threading Pattern

`ssh2::Session` is `!Send` — each SSH subsystem (terminal, monitor, SFTP) creates its own TCP+SSH connection on a dedicated thread. Communication with the frontend uses:
- `std::sync::mpsc` channels for input (frontend → backend)
- `app_handle.emit("event-name", payload)` for output (backend → frontend)

Reader threads have a 500ms startup delay to let the frontend register event listeners before data flows.

### React Frontend (`src/`)

```
App.tsx                          — Main layout, tab management, connection flow
components/Terminal/TerminalPane — xterm.js wrapper with two-div resize architecture
components/Sidebar/SystemInfoPanel — FinalShell-style system monitor (CPU/mem/disk/net/processes)
components/FileManager/FileBrowser — Tree + file list dual-pane browser
components/ConnectionDialog      — SSH connection add/edit form
types/index.ts                   — TypeScript interfaces matching Rust structs
styles/globals.css               — Tailwind + dark theme + xterm overrides
```

### xterm.js Resize Architecture

The TerminalPane uses a two-div pattern to prevent xterm's canvas from blowing out the flex layout:
- **wrapperRef** (outer): `position: absolute; inset: 0` — sized by parent flex, observed by ResizeObserver
- **containerRef** (inner): receives explicit pixel dimensions from wrapper's `getBoundingClientRect()` before `fitAddon.fit()`

Resize fires at multiple delays (50/150/400ms) to catch window animation. Always call `term.refresh()` after `fit()`.

## Config Files

User config stored at `~/.config/guishell/`:
- `connections.toml` — saved SSH connections (passwords in GNOME Keyring, never on disk)
- `settings.toml` — terminal font, appearance, transfer settings, SSH keepalive
- `monitor.toml` — monitoring panel configuration

## System Dependencies

VTE 0.70.6 compiled from source at `~/.local/` (Ubuntu 22.04 ships 0.68 without GTK4 support). GTK4 dev headers installed via `.dev-deps/` overlay. See `.cargo/config.toml` for PKG_CONFIG_PATH setup.

Required system packages: `libgtk-4-dev libadwaita-1-dev libsecret-1-dev libssh2-1-dev libcairo2-dev`

# CLAUDE.md

Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.
