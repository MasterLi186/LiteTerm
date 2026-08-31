# Repository Guidelines

## Native-only Development Policy (Highest Priority)

`native-prototype/` is the only active LiteTerm implementation. Unless the user explicitly names another implementation, all product code changes must stay inside `native-prototype/`; `run-native.sh` and native-only documentation/build scripts may also be changed when required. Do not modify the React/Tauri implementation (`src/`, `src-tauri/`), the legacy root Rust/GTK implementation, or their tests and build paths without explicit user authorization.

Use `cargo build` from `native-prototype/` to compile the active product, then use `./run-native.sh` to launch it. The full `native-prototype/build.sh` path is test-enabled and opt-in only, as specified below. Historical React/Tauri/GTK files and documents remain reference material only; they are not implementation targets.

## Project Structure & Module Organization

Active product code lives in `native-prototype/`, using Rust, winit, wgpu, and egui. The React/Tauri trees under `src/` and `src-tauri/`, plus the root GTK-era crate, are retained only as historical reference unless the user explicitly reactivates them. Design, API, and testing notes live in `docs/`.

## Build, Test, and Development Commands

- The default build command is `cargo build` from `native-prototype/`; it compiles the active product without opting into automated tests.
- After a successful default build, use `./run-native.sh` to launch the native application. Be aware that this launcher invokes `native-prototype/build.sh` when the binary is stale, so compile first and confirm the binary is current.
- `native-prototype/build.sh` is an opt-in full-validation command: it runs `cargo clippy --all-targets` and `cargo test`. Do not invoke it unless the user explicitly authorizes automated testing.
- Do not run or modify React/Tauri/GTK build paths unless the user explicitly requests that implementation.

Do not use `npx tauri dev`; Tauri is not an active development target and its file watcher is known to exhaust resources on the target environment.

## Coding Style & Naming Conventions

TypeScript is strict. Follow existing two-space indentation, single quotes in UI files, `PascalCase` component/file names, and `camelCase` functions and hooks. Keep user-visible text in Chinese. Rust follows `rustfmt`: four-space indentation, `snake_case` modules/functions, and `PascalCase` types. Run `cargo fmt --check` on changed Rust; test-target Clippy remains subject to the explicit authorization rule below.

## Testing Guidelines

Automated testing is disabled by default for this repository. Existing tests remain in the tree, but normal implementation, review, build, and verification requests do not authorize running or compiling test targets.

- Do not run `cargo test`, `cargo test --no-run`, test scripts, `cargo clippy --all-targets`, or any command that transitively executes or compiles automated tests unless the user explicitly requests automated testing.
- Do not add or update test-only code as part of ordinary implementation work unless the user explicitly asks for tests. A request to fix, review, build, verify, or launch the application is not implicit test authorization.
- For native UI changes, the default verification path is: run `cargo build`, launch the real native window, and inspect the affected behavior through actual interaction. Automated geometry or headless egui assertions must never be presented as a substitute for clicking and visually checking the running interface.
- Report verification precisely. If only compilation and launch were performed, say so; never claim that behavior, UI appearance, or automated tests were verified without performing the corresponding checks.

## Commit & Pull Request Guidelines

History uses concise Chinese Conventional Commit subjects such as `feat: 多标签系统` and `fix: 滚轮方向修正`; use `feat:`, `fix:`, `docs:`, `refactor:`, or `perf:` as appropriate. PRs should summarize scope, list verification commands, link relevant issues/design docs, and include before/after screenshots for UI changes. Never create or push release tags without explicit, one-time user approval.
