# Repository Guidelines

## Native-only Development Policy (Highest Priority)

LiteTerm Native is the only implementation in this repository: Rust + winit + wgpu + egui, with no WebView. All product code lives at the repository root (`src/`, `Cargo.toml`, `assets/`, `scripts/`, `packaging/`). The historical React/Tauri/GTK trees were removed from this branch; they remain only in the `legacy-tauri` branch and old tags.

Do not recreate a `native-prototype/` subdirectory and do not reintroduce React/Tauri/GTK build paths.

## Project Structure & Module Organization

Active product code lives in `src/` at the repository root, using Rust, winit, wgpu, and egui. `assets/` holds icons and the theme-generation source, `scripts/` holds maintenance scripts, and `packaging/` holds desktop packaging metadata. Design, API, and testing notes live in `docs/`.

## Build, Test, and Development Commands

- The default build command is `cargo build --locked --bin liteterm-native` from the repository root; it compiles the debug product without opting into automated tests.
- `./run-native.sh` launches the debug product. If its binary is missing or stale, it runs the same native cargo build; it never creates a `.deb` installer.
- `./build.sh` is the no-argument local Debian packaging entrypoint. It builds the locked release binary and creates `dist/installers/LiteTerm-Native_<版本>_<架构>.deb`; it does not run automated tests.
- `node scripts/generate-themes.mjs` regenerates `src/themes_generated.rs` from `assets/themes.ts`. Never hand-edit the generated file.

## Coding Style & Naming Conventions

Rust follows `rustfmt`: four-space indentation, `snake_case` modules/functions, and `PascalCase` types. Keep user-visible text and comments in Chinese; identifiers use English. Run `cargo fmt --all -- --check` on changed Rust; test-target Clippy remains subject to the explicit authorization rule below.

## Testing Guidelines

Automated testing is disabled by default for this repository. Existing tests remain in the tree, but normal implementation, review, build, and verification requests do not authorize running or compiling test targets.

- Do not run `cargo test`, `cargo test --no-run`, test scripts, `cargo clippy --all-targets`, or any command that transitively executes or compiles automated tests unless the user explicitly requests automated testing. The packaging `build.sh` is not a test command and does not run tests.
- Do not add or update test-only code as part of ordinary implementation work unless the user explicitly asks for tests. A request to fix, review, build, verify, or launch the application is not implicit test authorization.
- For native UI changes, the default verification path is: from the repository root run `cargo build --locked --bin liteterm-native`, then `./run-native.sh` (or the built binary directly), launch the real native window, and inspect the affected behavior through actual interaction. Automated geometry or headless egui assertions must never be presented as a substitute for clicking and visually checking the running interface.
- Report verification precisely. If only compilation and launch were performed, say so; never claim that behavior, UI appearance, or automated tests were verified without performing the corresponding checks.

## Commit & Pull Request Guidelines

History uses concise Chinese Conventional Commit subjects such as `feat: 多标签系统` and `fix: 滚轮方向修正`; use `feat:`, `fix:`, `docs:`, `refactor:`, or `perf:` as appropriate. PRs should summarize scope, list verification commands, link relevant issues/design docs, and include before/after screenshots for UI changes. Never create or push release tags without explicit, one-time user approval.
