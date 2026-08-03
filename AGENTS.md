# Repository Guidelines

## Project Structure & Module Organization

LiteTerm combines a React/TypeScript frontend with a Tauri 2 Rust backend. Frontend entry points and shared types live in `src/`; reusable UI belongs in `src/components/`, with feature folders such as `Terminal/`, `FileManager/`, and `ProcessManager/`. Active backend code is under `src-tauri/src/`: keep UI-facing handlers in `commands/`, persistent models in `config/`, and UI-independent logic in `core/`. The root Rust crate retains the earlier GTK implementation used by integration tests in `tests/`. `native-prototype/` is a separate experimental native client. Design, API, and testing notes live in `docs/`; packaged icons are in `src-tauri/icons/`.

## Build, Test, and Development Commands

- `npm ci` installs the locked frontend dependencies.
- `npm run dev` starts the Vite frontend server for browser-side work.
- `./build.sh` is the required full build path. It cleans caches, type-checks and bundles the frontend, builds and lints Tauri Rust, and runs both Rust test suites. Do not substitute standalone `npm run build` or `cargo build`.
- `./run.sh` launches the already-built Tauri debug binary; run `./build.sh` first.
- `cargo test` runs the root integration suite; use `cargo test --test zmodem_frame_test` for one file.
- `(cd src-tauri && cargo test)` runs backend tests. `tests/test_http_api.sh` requires a running app and its generated API token.

Do not use `npx tauri dev`; file-watcher resource use is known to crash on the target environment.

## Coding Style & Naming Conventions

TypeScript is strict. Follow existing two-space indentation, single quotes in UI files, `PascalCase` component/file names, and `camelCase` functions and hooks. Keep user-visible text in Chinese. Rust follows `rustfmt`: four-space indentation, `snake_case` modules/functions, and `PascalCase` types. Run `cargo fmt --check` on changed Rust; `./build.sh` includes Clippy checks.

## Testing Guidelines

Add Rust integration tests as `tests/<feature>_test.rs` with focused `test_<behavior>` functions. Place pure protocol/parsing logic in `core/` so it can be tested without UI state. For visual or SSH flows, document manual checks and attach screenshots; `test_full_flow.sh` demonstrates the expected workflow.

## Commit & Pull Request Guidelines

History uses concise Chinese Conventional Commit subjects such as `feat: 多标签系统` and `fix: 滚轮方向修正`; use `feat:`, `fix:`, `docs:`, `refactor:`, or `perf:` as appropriate. PRs should summarize scope, list verification commands, link relevant issues/design docs, and include before/after screenshots for UI changes. Never create or push release tags without explicit, one-time user approval.
