# Native P0 Features Design

## Goal and Scope

Complete the five P0 items in `native-prototype/TODO.md` without reintroducing WebView dependencies or disturbing the existing Tauri application:

1. Settings UI for terminal font, font size, theme, and shortcuts.
2. Switching among the Tabby-compatible themes defined by `src/themes.ts`.
3. Scrollback-aware terminal search opened with `Ctrl+Shift+F`.
4. Linux IME composition through winit IME events.
5. A new-tab selector for local shells and saved SSH connections.

The selector shows Serial as disabled with a Chinese P1 notice. Serial device discovery and I/O remain out of scope.

## Architecture

The implementation uses focused Native modules instead of adding feature logic directly to `main.rs`:

- `settings_panel.rs` renders and validates the settings dialog.
- `themes.rs` defines the runtime theme model and lookup API.
- `themes_generated.rs` contains the checked-in theme catalog generated from `src/themes.ts`.
- `shortcuts.rs` parses, validates, and matches configurable shortcuts.
- `terminal_search.rs` owns query, match positions, navigation, and search-bar state.
- `ime.rs` owns preedit/commit state and duplicate-input suppression.
- `new_tab_selector.rs` renders Shell, SSH, and disabled Serial choices.

`main.rs` remains the coordinator: it routes winit events, renders egui overlays, applies actions to `TabManager`, and requests redraws. Existing local-shell and SSH construction paths remain the only session factories.

## Settings and Theme Flow

`settings.rs` gains `serde(default)` compatibility and typed shortcut defaults. Startup loads the persisted settings before renderer creation. Invalid or absent configuration falls back to defaults and reports a non-fatal Chinese warning.

Settings are edited in a draft copy. Save performs shortcut-conflict validation, persists atomically, and then applies changes. Theme changes update default foreground/background, cursor, selection, and ANSI 16-color palette. Font changes rebuild font-dependent renderer resources and resize every open terminal so PTY rows and columns stay consistent.

The theme catalog is generated during development and committed as Rust data. Native builds do not require Node.js and never parse TypeScript at runtime. A generator command and consistency test keep the catalog synchronized with `src/themes.ts`.

## Search and Rendering

`terminal_search.rs` searches both the visible grid and scrollback, retaining terminal grid coordinates rather than UTF-8 byte offsets. This keeps CJK and wide-character highlighting aligned. Query changes reset navigation; Enter selects the next match, Shift+Enter selects the previous match, and Escape closes the bar and returns focus to the terminal.

The renderer receives immutable search-highlight data for the active tab. All matches use a subdued highlight, while the current match uses a distinct highlight. Empty queries produce no matches; missing matches display `0/0` without changing scroll position.

## IME Event Flow

winit `Ime::Enabled`, `Preedit`, `Commit`, and `Disabled` events are routed through `ime.rs`. Preedit text is displayed near the active cursor but is never sent to the PTY. Commit text is sent exactly once and clears preedit state. While composition is active, overlapping keyboard text events are suppressed, while control-key shortcuts continue through the normal shortcut router.

IME input is routed to whichever egui control currently owns keyboard focus. Text entered in settings, connection dialogs, command history, or the new-tab selector must not leak into the terminal.

## New-Tab Selector

The tab-bar `+` button and configurable new-tab shortcut open one modal selector rather than immediately creating the default shell. The Shell section lists valid executable entries from `/etc/shells`. The SSH section reuses the current grouped connection configuration and existing connection flow. Serial is visible but disabled with the text `串口终端将在 P1 实现`.

Selecting a valid entry closes the modal only after the existing creation path accepts the request. Escape and backdrop clicks close it without creating a tab.

## Error Handling

- Configuration parse and save failures remain non-fatal and appear in Chinese.
- Missing fonts retain the current working font and explain the fallback.
- Theme lookup failure falls back to AdventureTime.
- Search generation IDs discard stale results if the query or active tab changes.
- Missing shell executables are filtered out.
- SSH authentication and connection errors continue through the existing dialogs.

## Testing and Acceptance

Pure modules receive unit tests before implementation:

- Settings defaulting, round-trip persistence, atomic save, and shortcut conflicts.
- Theme count, unique names, AdventureTime values, color parsing, and generated-source consistency.
- Search next/previous, scrollback, no-match, CJK, and wide-cell coordinates.
- IME preedit, commit-once, cancel, focus routing, and shortcut preservation.
- Shell filtering and selector action mapping.

Integration checks cover settings application, renderer palette changes, terminal resizing, and event routing. The required verification path is `native-prototype/build.sh`, followed by manual checks for fcitx5/ibus input, terminal highlighting, theme previews, Shell/SSH creation, and disabled Serial behavior. The existing root `build.sh`, root `run.sh`, and Tauri binary are not replaced or modified.

## Delivery Sequence

1. Settings compatibility, shortcuts, and generated theme catalog.
2. Renderer theme/font application and settings UI.
3. Search state, grid matching, navigation, and highlights.
4. IME state machine and winit/egui focus routing.
5. New-tab selector and existing session-factory integration.
6. Update the five P0 checkboxes only after their acceptance checks pass.

Grok performs implementation through CCG's `codeagent-wrapper`; Codex owns orchestration, diff inspection, test evidence, and review. Because Claude is unavailable in this environment, cross-checking uses Grok plus Codex.
