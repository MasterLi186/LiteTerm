# Native P0 Features Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the five P0 capabilities in `native-prototype/TODO.md`: settings and shortcuts, the 191-theme catalog, terminal search, native IME input, and a Shell/SSH/disabled-Serial new-tab selector.

**Architecture:** Keep feature state and pure behavior in focused Rust modules; keep `main.rs` as the winit/egui coordinator. Generate and commit Rust theme data from `src/themes.ts`, apply palette/font changes through `Renderer`, and reuse existing local-shell and SSH session factories.

**Tech Stack:** Rust 2021, egui 0.31, winit 0.30, wgpu 24, cosmic-text 0.12, alacritty_terminal 0.25, serde/toml, Node.js 22 for the development-only theme generator.

---

## Working-Tree Safety

The branch already contains large user-owned changes. Before every Grok phase:

```bash
git status --short
mkdir -p .ccg/tasks/implement-p0-todos/baseline/<phase>
cp --preserve=mode,timestamps <owned-existing-file> .ccg/tasks/implement-p0-todos/baseline/<phase>/
```

After each phase, compare owned files against their snapshots and inspect every new file. Never run `git restore`, `git checkout --`, a repository-wide formatter, or a destructive cleanup. Grok must not commit; Codex commits only after verification and review.

### Task 1: Persisted Shortcut Model

**Files:**

- Create: `native-prototype/src/shortcuts.rs`
- Modify: `native-prototype/src/settings.rs`
- Modify: `native-prototype/src/main.rs`
- Test: unit tests inside `native-prototype/src/shortcuts.rs` and `native-prototype/src/settings.rs`

- [ ] **Step 1: Write failing shortcut parsing and conflict tests**

```rust
#[test]
fn parses_normalized_chord() {
    let chord = KeyChord::parse("Ctrl+Shift+F").unwrap();
    assert!(chord.ctrl);
    assert!(chord.shift);
    assert_eq!(chord.key, "F");
    assert_eq!(chord.to_string(), "Ctrl+Shift+F");
}

#[test]
fn rejects_duplicate_shortcuts() {
    let mut settings = ShortcutSettings::default();
    settings.close_tab = settings.new_tab.clone();
    assert_eq!(
        settings.validate().unwrap_err(),
        "快捷键冲突：新建标签页 与 关闭标签页"
    );
}
```

- [ ] **Step 2: Run the focused tests and confirm RED**

Run:

```bash
cd native-prototype
cargo test shortcuts::tests settings::tests
```

Expected: compilation fails because `KeyChord`, `ShortcutSettings`, or the new settings field does not exist.

- [ ] **Step 3: Implement the typed shortcut API**

Use these public contracts:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyChord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub super_key: bool,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ShortcutSettings {
    pub new_tab: String,
    pub close_tab: String,
    pub search: String,
    pub copy: String,
    pub paste: String,
    pub next_tab: String,
    pub previous_tab: String,
}

impl ShortcutSettings {
    pub fn validate(&self) -> Result<(), String>;
    pub fn chord(&self, action: ShortcutAction) -> Result<KeyChord, String>;
}
```

Defaults are `Ctrl+Shift+T`, `Ctrl+Shift+W`, `Ctrl+Shift+F`, `Ctrl+Shift+C`, `Ctrl+Shift+V`, `Ctrl+Tab`, and `Ctrl+Shift+Tab`. Add `pub shortcuts: ShortcutSettings` to `Settings` with `serde(default)`.

- [ ] **Step 4: Add settings compatibility tests**

```rust
#[test]
fn old_settings_without_shortcuts_receive_defaults() {
    let settings: Settings = toml::from_str("[terminal]\nfont = 'Ubuntu Mono'\n").unwrap();
    assert_eq!(settings.shortcuts.search, "Ctrl+Shift+F");
}

#[test]
fn settings_roundtrip_preserves_shortcuts() {
    let mut settings = Settings::default();
    settings.shortcuts.search = "Ctrl+F".into();
    let encoded = toml::to_string_pretty(&settings).unwrap();
    let decoded: Settings = toml::from_str(&encoded).unwrap();
    assert_eq!(decoded.shortcuts.search, "Ctrl+F");
}
```

- [ ] **Step 5: Verify GREEN**

Run:

```bash
cd native-prototype
cargo test shortcuts::tests settings::tests
```

Expected: all focused tests pass.

### Task 2: Generated Theme Catalog and Runtime Palette

**Files:**

- Create: `native-prototype/scripts/generate-themes.mjs`
- Create: `native-prototype/src/themes.rs`
- Create: `native-prototype/src/themes_generated.rs`
- Modify: `native-prototype/src/renderer.rs`
- Modify: `native-prototype/src/main.rs`
- Test: unit tests inside `native-prototype/src/themes.rs` and `native-prototype/src/renderer.rs`

- [ ] **Step 1: Write failing catalog tests**

```rust
#[test]
fn generated_catalog_matches_tabby_source_count() {
    assert_eq!(all_themes().len(), 191);
}

#[test]
fn adventure_time_matches_existing_native_palette() {
    let theme = theme_by_name("AdventureTime").unwrap();
    assert_eq!(theme.background, [0x1f, 0x1d, 0x45]);
    assert_eq!(theme.foreground, [0xf8, 0xdc, 0xc0]);
    assert_eq!(theme.ansi[1], [0xbd, 0x00, 0x13]);
}

#[test]
fn theme_names_are_unique() {
    let names = all_themes().iter().map(|theme| theme.name).collect::<HashSet<_>>();
    assert_eq!(names.len(), all_themes().len());
}
```

- [ ] **Step 2: Confirm RED**

Run:

```bash
cd native-prototype
cargo test themes::tests
```

Expected: compilation fails because the theme module and catalog do not exist.

- [ ] **Step 3: Implement the theme data contract**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalTheme {
    pub name: &'static str,
    pub background: [u8; 3],
    pub foreground: [u8; 3],
    pub cursor: [u8; 3],
    pub selection: [u8; 3],
    pub ansi: [[u8; 3]; 16],
}

pub fn all_themes() -> &'static [TerminalTheme] {
    generated::TERMINAL_THEMES
}

pub fn theme_by_name(name: &str) -> Option<&'static TerminalTheme> {
    all_themes().iter().find(|theme| theme.name == name)
}
```

The generator reads `../src/themes.ts`, extracts each quoted theme and the 20 required colors, and writes deterministic Rust. It exits non-zero for a missing field, duplicate name, invalid hex color, or a count other than 191.

- [ ] **Step 4: Generate the checked-in catalog**

Run:

```bash
node native-prototype/scripts/generate-themes.mjs
```

Expected: `native-prototype/src/themes_generated.rs` contains exactly 191 `TerminalTheme` values.

- [ ] **Step 5: Replace hard-coded renderer colors with `TerminalPalette`**

Use this contract:

```rust
#[derive(Debug, Clone, Copy)]
pub struct TerminalPalette {
    pub background: [u8; 4],
    pub foreground: [u8; 4],
    pub cursor: [f32; 4],
    pub selection: [f32; 4],
    pub ansi: [[u8; 4]; 16],
}

impl TerminalPalette {
    pub fn from_theme(theme: &TerminalTheme) -> Self;
}

impl Renderer {
    pub fn set_theme(&mut self, theme: &TerminalTheme);
    pub fn palette(&self) -> &TerminalPalette;
}
```

`color_to_f32`, the render-pass clear color, cursor, selection, and ANSI fallbacks must use `self.palette`. Existing OSC/true-color values retain priority.

- [ ] **Step 6: Verify catalog and renderer tests**

Run:

```bash
cd native-prototype
cargo test themes::tests renderer::tests
```

Expected: all focused tests pass and regeneration produces no diff:

```bash
node scripts/generate-themes.mjs
git diff --exit-code -- src/themes_generated.rs
```

### Task 3: Settings Panel and Live Font/Theme Application

**Files:**

- Create: `native-prototype/src/settings_panel.rs`
- Modify: `native-prototype/src/settings.rs`
- Modify: `native-prototype/src/atlas.rs`
- Modify: `native-prototype/src/renderer.rs`
- Modify: `native-prototype/src/tab_manager.rs`
- Modify: `native-prototype/src/main.rs`
- Test: unit tests inside the changed modules

- [ ] **Step 1: Write failing draft validation and atlas reset tests**

```rust
#[test]
fn settings_draft_rejects_out_of_range_font_size() {
    let mut draft = SettingsDraft::from(&Settings::default());
    draft.font_size = 49.0;
    assert_eq!(draft.validate().unwrap_err(), "字号必须在 8 到 48 之间");
}

#[test]
fn atlas_reset_updates_metrics_and_drops_cached_glyphs() {
    let mut atlas = test_atlas(26.0);
    atlas.reset("Noto Sans Mono", 18.0);
    assert_eq!(atlas.font_family(), "Noto Sans Mono");
    assert_eq!(atlas.font_size, 18.0);
    assert!(atlas.is_empty());
}
```

- [ ] **Step 2: Confirm RED**

Run:

```bash
cd native-prototype
cargo test settings_panel::tests atlas::tests
```

Expected: compilation fails for the missing draft and reset APIs.

- [ ] **Step 3: Extend terminal settings**

Use explicit fields and compatible defaults:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalSettings {
    pub font: String,
    pub font_size: f32,
    pub scrollback_lines: u32,
    pub color_scheme: String,
    pub cursor_blink: bool,
}
```

Defaults are `Ubuntu Mono`, `26.0`, and `AdventureTime`. Loading an old value such as `Monospace 12` must not panic; normalize the family and retain the explicit default size when no `font_size` key exists.

- [ ] **Step 4: Implement settings draft and UI actions**

```rust
pub enum SettingsPanelAction {
    None,
    Apply(Settings),
    Cancel,
}

pub struct SettingsPanel {
    pub visible: bool,
    draft: SettingsDraft,
    error: Option<String>,
    theme_filter: String,
    capturing_shortcut: Option<ShortcutAction>,
}

impl SettingsPanel {
    pub fn open(&mut self, current: &Settings);
    pub fn show(&mut self, ctx: &egui::Context) -> SettingsPanelAction;
}
```

The Chinese UI includes font family, size 8–48, searchable theme list with previews, and the seven shortcuts. Save validates shortcuts and writes settings; Cancel discards the draft.

- [ ] **Step 5: Implement live renderer application**

Add:

```rust
impl Renderer {
    pub fn set_font(&mut self, gpu: &GpuState, family: &str, size: f32);
}

impl TabManager {
    pub fn resize_all(&mut self, cols: u16, rows: u16);
}
```

Applying settings updates the theme, rebuilds font GPU resources only when font fields changed, recalculates the grid, and resizes every local and SSH terminal.

- [ ] **Step 6: Verify focused tests**

Run:

```bash
cd native-prototype
cargo test settings::tests shortcuts::tests settings_panel::tests atlas::tests renderer::tests tab_manager::tests
```

Expected: all focused tests pass.

### Task 4: Scrollback-Aware Terminal Search

**Files:**

- Create: `native-prototype/src/terminal_search.rs`
- Modify: `native-prototype/src/terminal.rs`
- Modify: `native-prototype/src/renderer.rs`
- Modify: `native-prototype/src/tab_manager.rs`
- Modify: `native-prototype/src/main.rs`
- Test: unit tests inside `native-prototype/src/terminal_search.rs`

- [ ] **Step 1: Write failing pure search tests**

```rust
#[test]
fn finds_ascii_and_cjk_by_grid_columns() {
    let lines = vec![
        SearchLine::new(-1, vec![cell('a', 1), cell('中', 2), spacer(), cell('b', 1)]),
    ];
    let matches = find_matches(&lines, "中b", false);
    assert_eq!(matches, vec![SearchMatch { line: -1, start_col: 1, end_col: 3 }]);
}

#[test]
fn navigation_wraps_in_both_directions() {
    let mut state = TerminalSearchState::with_matches("x", three_matches());
    assert_eq!(state.next().unwrap().line, 1);
    assert_eq!(state.previous().unwrap().line, 0);
}

#[test]
fn empty_query_has_no_matches() {
    assert!(find_matches(&fixture_lines(), "", false).is_empty());
}
```

- [ ] **Step 2: Confirm RED**

Run:

```bash
cd native-prototype
cargo test terminal_search::tests
```

Expected: compilation fails because the search module does not exist.

- [ ] **Step 3: Implement search state and terminal snapshot**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    pub line: i32,
    pub start_col: usize,
    pub end_col: usize,
}

pub struct TerminalSearchState {
    pub visible: bool,
    pub query: String,
    pub matches: Vec<SearchMatch>,
    pub current: Option<usize>,
    pub case_sensitive: bool,
}

impl TerminalState {
    pub fn search_lines(&self) -> Vec<SearchLine>;
    pub fn reveal_search_line(&mut self, line: i32);
}
```

Skip `WIDE_CHAR_SPACER` as text while retaining its grid width. Search is a literal substring for P0 and covers history plus visible screen.

- [ ] **Step 4: Add search bar and input routing**

`Ctrl+Shift+F` and the existing right-click item open the search bar and focus its text field. Query changes recompute active-tab matches. Enter/Shift+Enter select next/previous, Escape closes, and terminal PTY input is suppressed while the search field owns focus. The status reads `current/total`, including `0/0`.

- [ ] **Step 5: Add renderer highlights**

Extend `render_to_pass` with:

```rust
pub struct SearchHighlights<'a> {
    pub matches: &'a [SearchMatch],
    pub current: Option<usize>,
}
```

Compare each displayed cell's absolute grid line and column. Use subdued background for all matches and a distinct background for the current match without replacing selection semantics.

- [ ] **Step 6: Verify focused tests**

Run:

```bash
cd native-prototype
cargo test terminal_search::tests terminal::tests renderer::tests
```

Expected: all focused tests pass.

### Task 5: Native IME Composition

**Files:**

- Create: `native-prototype/src/ime.rs`
- Modify: `native-prototype/src/main.rs`
- Test: unit tests inside `native-prototype/src/ime.rs`

- [ ] **Step 1: Write failing IME state tests**

```rust
#[test]
fn preedit_never_writes_to_terminal() {
    let mut ime = ImeState::default();
    assert_eq!(ime.preedit("zhong".into(), Some((0, 5))), ImeAction::Redraw);
    assert_eq!(ime.preedit_text(), "zhong");
}

#[test]
fn commit_is_forwarded_once_when_terminal_owns_focus() {
    let mut ime = ImeState::default();
    ime.preedit("zhong".into(), None);
    assert_eq!(ime.commit("中".into(), InputOwner::Terminal), ImeAction::Commit("中".into()));
    assert_eq!(ime.take_duplicate_guard(), Some("中"));
    assert_eq!(ime.preedit_text(), "");
}

#[test]
fn dialog_commit_does_not_reach_pty() {
    let mut ime = ImeState::default();
    assert_eq!(ime.commit("中".into(), InputOwner::Egui), ImeAction::Redraw);
}
```

- [ ] **Step 2: Confirm RED**

Run:

```bash
cd native-prototype
cargo test ime::tests
```

Expected: compilation fails because `ImeState` does not exist.

- [ ] **Step 3: Implement the state machine**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOwner {
    Terminal,
    Egui,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeAction {
    None,
    Redraw,
    Commit(String),
}

#[derive(Default)]
pub struct ImeState {
    enabled: bool,
    preedit: String,
    cursor: Option<(usize, usize)>,
    duplicate_guard: Option<String>,
}
```

Enabled/Disabled update state, Preedit only updates overlay data, and Commit returns PTY text only for `InputOwner::Terminal`.

- [ ] **Step 4: Route winit IME events**

Enable IME after window creation with `window.set_ime_allowed(true)`. Handle `WindowEvent::Ime(Ime::Enabled/Disabled/Preedit/Commit)` before generic keyboard text. Update `set_ime_cursor_area` from `Renderer::cursor_screen_rect`; suppress only the matching duplicate keyboard text after a commit.

- [ ] **Step 5: Render preedit overlay**

When the terminal owns focus and preedit is non-empty, render a compact egui overlay at the terminal cursor with underline styling. Dialog, command-bar, completion-popup, search, and settings focus take precedence over terminal routing.

- [ ] **Step 6: Verify focused tests**

Run:

```bash
cd native-prototype
cargo test ime::tests
```

Expected: all focused tests pass.

### Task 6: Shell/SSH New-Tab Selector

**Files:**

- Create: `native-prototype/src/new_tab_selector.rs`
- Modify: `native-prototype/src/main.rs`
- Modify: `native-prototype/src/tab_bar.rs`
- Test: unit tests inside `native-prototype/src/new_tab_selector.rs`

- [ ] **Step 1: Write failing shell filtering and action tests**

```rust
#[test]
fn filters_comments_duplicates_and_missing_shells() {
    let input = "# shells\n/bin/bash\n/bin/bash\n/not/executable\n/bin/sh\n";
    let shells = parse_shells(input, |path| path == Path::new("/bin/bash") || path == Path::new("/bin/sh"));
    assert_eq!(shells, vec![PathBuf::from("/bin/bash"), PathBuf::from("/bin/sh")]);
}

#[test]
fn serial_choice_is_disabled() {
    assert!(!NewTabKind::Serial.enabled());
    assert_eq!(NewTabKind::Serial.subtitle(), "串口终端将在 P1 实现");
}
```

- [ ] **Step 2: Confirm RED**

Run:

```bash
cd native-prototype
cargo test new_tab_selector::tests
```

Expected: compilation fails because the selector module does not exist.

- [ ] **Step 3: Implement selector state and actions**

```rust
pub enum NewTabAction {
    None,
    Close,
    OpenShell(PathBuf),
    OpenSsh(String),
}

pub struct NewTabSelector {
    pub visible: bool,
    shells: Vec<PathBuf>,
    error: Option<String>,
}

impl NewTabSelector {
    pub fn open(&mut self);
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        connections: &[SshConnection],
    ) -> NewTabAction;
}
```

Read `/etc/shells` on open, filter non-executable paths, preserve file order, and deduplicate. Show grouped saved SSH connections through the existing connection data. Serial is visible, disabled, and adds no dependency.

- [ ] **Step 4: Replace new-tab entry behavior**

The tab-bar `+` action and configured `Ctrl+Shift+T` chord open the selector. `OpenShell` calls `new_local_tab_with_shell`; `OpenSsh` resolves the saved connection and calls the existing `new_ssh_tab`. Escape and backdrop clicks close without creating a tab.

- [ ] **Step 5: Verify focused tests**

Run:

```bash
cd native-prototype
cargo test new_tab_selector::tests
```

Expected: all focused tests pass.

### Task 7: Full Integration, Documentation, and Review

**Files:**

- Modify: `native-prototype/TODO.md`
- Modify: `.ccg/tasks/implement-p0-todos/task.json`
- Create: `.ccg/tasks/implement-p0-todos/review.md`
- Test: Native full build and manual acceptance

- [ ] **Step 1: Format only changed Rust files**

Run `rustfmt` with explicit file paths. Do not run a repository-wide formatter.

- [ ] **Step 2: Run the required Native verification**

Run:

```bash
./native-prototype/build.sh
```

Expected: `cargo build`, `cargo clippy --all-targets`, and `cargo test` all exit 0, and `native-prototype/target/debug/liteterm-native` exists.

- [ ] **Step 3: Run script contract tests**

Run:

```bash
cargo test --test native_build_script_test --test native_color_api_test --test run_native_script_test
```

Expected: all root integration tests pass.

- [ ] **Step 4: Perform manual Linux acceptance**

Use:

```bash
./run-native.sh
```

Verify:

1. Settings survive restart; font/size/theme apply immediately.
2. Theme filter exposes 191 unique entries and AdventureTime remains correct.
3. Search finds visible and scrollback Chinese/ASCII text and navigates both directions.
4. fcitx5 or ibus preedit is visible; committed Chinese appears exactly once.
5. `+` and `Ctrl+Shift+T` open the selector; Shell and SSH work; Serial is disabled.

- [ ] **Step 5: Review only P0-owned deltas**

Compare each owned file to its phase snapshot, inspect new files, and confirm no unrelated Native behavior changed. Run a CCG Grok review and a Codex lead review, merge findings into `review.md`, fix every Critical issue, and rerun the full verification.

- [ ] **Step 6: Update P0 checkboxes**

Mark the five P0 items complete only when their automated and manual acceptance evidence exists.

- [ ] **Step 7: Commit after review**

Stage only verified P0 files and use concise Chinese Conventional Commit subjects. Do not include unrelated dirty files, create tags, or push.
