// Terminal scrollback-aware literal search (pure logic).
//
// SearchCell, SearchLine, SearchMatch, TerminalSearchState, find_matches.

/// One grid cell for search haystack construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchCell {
    pub col: usize,
    pub ch: char,
    /// Display width in grid columns (1 for narrow, 2 for wide primary).
    pub width: usize,
    pub is_spacer: bool,
    pub zerowidth: Vec<char>,
}

impl SearchCell {
    pub fn primary(col: usize, ch: char) -> Self {
        Self {
            col,
            ch,
            width: 1,
            is_spacer: false,
            zerowidth: Vec::new(),
        }
    }

    pub fn wide(col: usize, ch: char) -> Self {
        Self {
            col,
            ch,
            width: 2,
            is_spacer: false,
            zerowidth: Vec::new(),
        }
    }

    pub fn spacer(col: usize) -> Self {
        Self {
            col,
            ch: '\0',
            width: 1,
            is_spacer: true,
            zerowidth: Vec::new(),
        }
    }

    pub fn with_zerowidth(col: usize, ch: char, zerowidth: Vec<char>) -> Self {
        Self {
            col,
            ch,
            width: 1,
            is_spacer: false,
            zerowidth,
        }
    }
}

/// One absolute terminal line and its cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchLine {
    pub line: i32,
    pub cells: Vec<SearchCell>,
}

impl SearchLine {
    pub fn new(line: i32, cells: Vec<SearchCell>) -> Self {
        Self { line, cells }
    }
}

/// Half-open match range `[start_col, end_col)` on an absolute line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    pub line: i32,
    pub start_col: usize,
    pub end_col: usize,
}

impl SearchMatch {
    pub fn contains_cell(&self, line: i32, col: usize) -> bool {
        self.line == line && col >= self.start_col && col < self.end_col
    }
}

/// Per-tab (or global) search UI + navigation state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSearchState {
    pub visible: bool,
    pub query: String,
    pub matches: Vec<SearchMatch>,
    pub current: Option<usize>,
    pub case_sensitive: bool,
    /// Recent queries for this tab, newest first. This is intentionally not
    /// persisted: closing a tab releases its search context and history.
    pub history: Vec<String>,
}

pub const MAX_SEARCH_HISTORY_ITEMS: usize = 20;

impl Default for TerminalSearchState {
    fn default() -> Self {
        Self {
            visible: false,
            query: String::new(),
            matches: Vec::new(),
            current: None,
            case_sensitive: false,
            history: Vec::new(),
        }
    }
}

impl TerminalSearchState {
    pub fn with_matches(query: impl Into<String>, matches: Vec<SearchMatch>) -> Self {
        let current = if matches.is_empty() { None } else { Some(0) };
        Self {
            visible: false,
            query: query.into(),
            matches,
            current,
            case_sensitive: false,
            history: Vec::new(),
        }
    }

    /// Remember a query after the user actually navigates search results.
    /// Blank input is ignored and duplicate entries are moved to the front.
    pub fn remember_query(&mut self) {
        if self.query.trim().is_empty() {
            return;
        }
        let query = self.query.clone();
        self.history.retain(|entry| entry != &query);
        self.history.insert(0, query);
        self.history.truncate(MAX_SEARCH_HISTORY_ITEMS);
    }

    pub fn next(&mut self) -> Option<&SearchMatch> {
        if self.matches.is_empty() {
            return None;
        }
        let next_idx = match self.current {
            Some(i) => (i + 1) % self.matches.len(),
            None => 0,
        };
        self.current = Some(next_idx);
        self.matches.get(next_idx)
    }

    pub fn previous(&mut self) -> Option<&SearchMatch> {
        if self.matches.is_empty() {
            return None;
        }
        let prev_idx = match self.current {
            Some(0) | None => self.matches.len() - 1,
            Some(i) => i - 1,
        };
        self.current = Some(prev_idx);
        self.matches.get(prev_idx)
    }

    pub fn replace_results(&mut self, query: impl Into<String>, matches: Vec<SearchMatch>) {
        self.query = query.into();
        self.current = if matches.is_empty() { None } else { Some(0) };
        self.matches = matches;
    }

    pub fn status_text(&self) -> String {
        let total = self.matches.len();
        let current = self.current.map(|i| i + 1).unwrap_or(0);
        format!("{current}/{total}")
    }
}

/// One UTF-8 unit in the haystack with its grid column span.
struct TextUnit {
    /// Inclusive start byte in haystack.
    byte_start: usize,
    /// Exclusive end byte in haystack.
    byte_end: usize,
    /// Grid column this unit maps to (start).
    start_col: usize,
    /// Exclusive grid end for this unit's cell (covers wide width).
    end_col: usize,
}

/// Build haystack text and byte-boundary → grid column mapping for one line.
/// Spacers are skipped in text; zerowidth chars are appended and map to the primary col.
fn line_haystack(cells: &[SearchCell]) -> (String, Vec<TextUnit>) {
    let mut text = String::new();
    let mut units = Vec::new();

    for cell in cells {
        if cell.is_spacer {
            continue;
        }
        let cell_end = cell.col.saturating_add(cell.width.max(1));

        let byte_start = text.len();
        text.push(cell.ch);
        let byte_end = text.len();
        units.push(TextUnit {
            byte_start,
            byte_end,
            start_col: cell.col,
            end_col: cell_end,
        });

        for z in &cell.zerowidth {
            let z_start = text.len();
            text.push(*z);
            let z_end = text.len();
            units.push(TextUnit {
                byte_start: z_start,
                byte_end: z_end,
                start_col: cell.col,
                end_col: cell_end,
            });
        }
    }

    (text, units)
}

/// ASCII-only case fold: preserves UTF-8 byte lengths (non-ASCII chars unchanged).
fn ascii_case_fold(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii() {
                c.to_ascii_lowercase()
            } else {
                c
            }
        })
        .collect()
}

/// Map a haystack byte range `[start_byte, end_byte)` to half-open grid columns.
fn map_bytes_to_cols(
    units: &[TextUnit],
    start_byte: usize,
    end_byte: usize,
) -> Option<(usize, usize)> {
    if end_byte <= start_byte {
        return None;
    }
    let mut start_col = None;
    let mut end_col = 0usize;
    for u in units {
        if u.byte_start >= end_byte {
            break;
        }
        if u.byte_end <= start_byte {
            continue;
        }
        if start_col.is_none() {
            start_col = Some(u.start_col);
        }
        end_col = u.end_col;
    }
    start_col.map(|sc| (sc, end_col))
}

/// Literal, per-line search. Empty query yields no matches.
/// `case_sensitive == false` uses ASCII case folding (Unicode must not panic).
pub fn find_matches(lines: &[SearchLine], query: &str, case_sensitive: bool) -> Vec<SearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }

    let query_search = if case_sensitive {
        query.to_string()
    } else {
        ascii_case_fold(query)
    };

    let mut out = Vec::new();

    for line in lines {
        let (haystack, units) = line_haystack(&line.cells);
        if haystack.is_empty() {
            continue;
        }

        let haystack_search = if case_sensitive {
            haystack
        } else {
            ascii_case_fold(&haystack)
        };

        // Non-overlapping literal finds via byte offsets on the (possibly folded) string.
        // ASCII fold preserves byte layout relative to the original UTF-8 string.
        let mut search_from = 0usize;
        while search_from <= haystack_search.len() {
            let rest = &haystack_search[search_from..];
            let Some(rel) = rest.find(query_search.as_str()) else {
                break;
            };
            let start_byte = search_from + rel;
            let end_byte = start_byte + query_search.len();

            if let Some((start_col, end_col)) = map_bytes_to_cols(&units, start_byte, end_byte) {
                out.push(SearchMatch {
                    line: line.line,
                    start_col,
                    end_col,
                });
            }

            // Advance past this match (non-overlapping). Guard empty-query already handled.
            search_from = end_byte;
            if query_search.is_empty() {
                break;
            }
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Search bar coordinator API (pure — effects never encode PTY bytes)
// ---------------------------------------------------------------------------

/// Logical keys the search bar handles (mapped from egui/winit by the coordinator).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchBarKey {
    Enter,
    Escape,
}

/// Discrete actions derived from a key + modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchBarKeyAction {
    Next,
    Previous,
    Close,
}

/// Side effects for the coordinator. Never carries PTY write payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchBarEffect {
    None,
    FocusQuery,
    Reveal(SearchMatch),
    Closed,
}

/// Map Enter / Shift+Enter / Escape to a search-bar action.
pub fn search_bar_key_action(key: SearchBarKey, shift: bool) -> SearchBarKeyAction {
    match key {
        SearchBarKey::Enter => {
            if shift {
                SearchBarKeyAction::Previous
            } else {
                SearchBarKeyAction::Next
            }
        }
        SearchBarKey::Escape => SearchBarKeyAction::Close,
    }
}

/// Show the search bar, recompute if the query is non-empty, and request query focus.
/// Empty query keeps `0/0` and returns `FocusQuery` (no reveal).
pub fn open_search(state: &mut TerminalSearchState, lines: &[SearchLine]) -> SearchBarEffect {
    state.visible = true;
    if state.query.is_empty() {
        state.matches.clear();
        state.current = None;
        return SearchBarEffect::FocusQuery;
    }
    let _ = recompute_search(state, lines);
    // Prefer FocusQuery so the coordinator always focuses the field; current match
    // is available on state for reveal_search_line.
    SearchBarEffect::FocusQuery
}

/// Apply Next / Previous / Close. Navigation yields `Reveal`; empty matches yield `None`.
pub fn apply_search_bar_action(
    state: &mut TerminalSearchState,
    action: SearchBarKeyAction,
) -> SearchBarEffect {
    match action {
        SearchBarKeyAction::Next => {
            state.remember_query();
            match state.next().copied() {
                Some(m) => SearchBarEffect::Reveal(m),
                None => SearchBarEffect::None,
            }
        }
        SearchBarKeyAction::Previous => {
            state.remember_query();
            match state.previous().copied() {
                Some(m) => SearchBarEffect::Reveal(m),
                None => SearchBarEffect::None,
            }
        }
        SearchBarKeyAction::Close => {
            state.visible = false;
            SearchBarEffect::Closed
        }
    }
}

/// Re-run `find_matches` from `state.query` / `case_sensitive`, reset current to first
/// (or None), and optionally reveal the first hit.
pub fn recompute_search(state: &mut TerminalSearchState, lines: &[SearchLine]) -> SearchBarEffect {
    let query = state.query.clone();
    let matches = find_matches(lines, &query, state.case_sensitive);
    state.replace_results(query, matches);
    match state.current.and_then(|i| state.matches.get(i).copied()) {
        Some(m) => SearchBarEffect::Reveal(m),
        None => SearchBarEffect::None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_search_bar_action, find_matches, open_search, recompute_search,
        search_bar_key_action, SearchBarEffect, SearchBarKey, SearchBarKeyAction, SearchCell,
        SearchLine, SearchMatch, TerminalSearchState, MAX_SEARCH_HISTORY_ITEMS,
    };

    // --- Fixture helpers (call production constructors once they exist) ---

    /// Primary cell at `col` with display width 1 (ASCII / narrow).
    fn cell_primary(col: usize, ch: char) -> SearchCell {
        SearchCell::primary(col, ch)
    }

    /// Wide primary (e.g. CJK) at `col`; next grid column is a spacer.
    fn cell_wide(col: usize, ch: char) -> SearchCell {
        SearchCell::wide(col, ch)
    }

    /// Wide-char spacer occupying one grid column (not part of haystack text).
    fn cell_spacer(col: usize) -> SearchCell {
        SearchCell::spacer(col)
    }

    /// Primary cell that also carries zero-width combining marks.
    fn cell_with_zerowidth(col: usize, ch: char, zerowidth: &[char]) -> SearchCell {
        SearchCell::with_zerowidth(col, ch, zerowidth.to_vec())
    }

    /// Canonical wide-char fixture used by analysis/plan:
    /// col0=`a`, col1=`中`(width 2), col2=spacer, col3=`b`.
    /// Grid: a@0 | 中@1 (w2) | spacer@2 | b@3
    fn cjk_wide_fixture_line(line: i32) -> SearchLine {
        SearchLine::new(
            line,
            vec![
                cell_primary(0, 'a'),
                cell_wide(1, '中'),
                cell_spacer(2),
                cell_primary(3, 'b'),
            ],
        )
    }

    fn three_matches() -> Vec<SearchMatch> {
        vec![
            SearchMatch {
                line: 0,
                start_col: 0,
                end_col: 1,
            },
            SearchMatch {
                line: 1,
                start_col: 2,
                end_col: 3,
            },
            SearchMatch {
                line: 2,
                start_col: 4,
                end_col: 5,
            },
        ]
    }

    // =========================================================================
    // find_matches — text extraction & column mapping
    // =========================================================================

    /// ASCII literal substring: columns are 1:1 with characters.
    #[test]
    fn finds_ascii_literal_by_grid_columns() {
        let lines = vec![SearchLine::new(
            0,
            vec![
                cell_primary(0, 'h'),
                cell_primary(1, 'e'),
                cell_primary(2, 'l'),
                cell_primary(3, 'l'),
                cell_primary(4, 'o'),
            ],
        )];
        let matches = find_matches(&lines, "ell", false);
        assert_eq!(
            matches,
            vec![SearchMatch {
                line: 0,
                start_col: 1,
                end_col: 4, // half-open: covers cols 1,2,3
            }]
        );
    }

    /// CJK wide primary + spacer: query "中b" on fixture a|中|spacer|b.
    /// Half-open range must cover primary 中@1, spacer@2, and b@3 → end_col=4.
    #[test]
    fn finds_cjk_and_wide_spacer_with_half_open_end_col() {
        let lines = vec![cjk_wide_fixture_line(-1)];
        let matches = find_matches(&lines, "中b", false);
        assert_eq!(
            matches,
            vec![SearchMatch {
                line: -1,
                start_col: 1,
                end_col: 4,
            }],
            "spacer is skipped in haystack but half-open end_col must cover b@3"
        );
    }

    /// Zero-width combining marks stay on the primary column mapping.
    /// Haystack text includes zerowidth chars; columns map back to the base cell.
    #[test]
    fn zerowidth_combining_chars_map_back_to_primary_column() {
        // cells: 'e'@0, 'a'+combining-acute@1, 't'@2  → haystack "eát" (a + U+0301)
        let acute = '\u{0301}';
        let lines = vec![SearchLine::new(
            5,
            vec![
                cell_primary(0, 'e'),
                cell_with_zerowidth(1, 'a', &[acute]),
                cell_primary(2, 't'),
            ],
        )];
        // Match the combining sequence "a" + acute as query, or full "eát"
        let query: String = ['a', acute].iter().collect();
        let matches = find_matches(&lines, &query, false);
        assert_eq!(
            matches,
            vec![SearchMatch {
                line: 5,
                start_col: 1,
                end_col: 2, // only the primary column of the combining cluster
            }],
            "zerowidth codepoints must not invent extra grid columns"
        );
    }

    // =========================================================================
    // find_matches — query rules
    // =========================================================================

    #[test]
    fn empty_query_has_no_matches() {
        let lines = vec![
            cjk_wide_fixture_line(-1),
            SearchLine::new(0, vec![cell_primary(0, 'x'), cell_primary(1, 'y')]),
        ];
        assert!(
            find_matches(&lines, "", false).is_empty(),
            "empty query must never produce matches"
        );
        assert!(find_matches(&lines, "", true).is_empty());
    }

    #[test]
    fn case_insensitive_match_when_case_sensitive_false() {
        let lines = vec![SearchLine::new(
            0,
            vec![
                cell_primary(0, 'A'),
                cell_primary(1, 'b'),
                cell_primary(2, 'C'),
            ],
        )];
        let matches = find_matches(&lines, "abc", false);
        assert_eq!(
            matches,
            vec![SearchMatch {
                line: 0,
                start_col: 0,
                end_col: 3,
            }]
        );
    }

    #[test]
    fn case_sensitive_rejects_case_mismatch() {
        let lines = vec![SearchLine::new(
            0,
            vec![
                cell_primary(0, 'A'),
                cell_primary(1, 'b'),
                cell_primary(2, 'C'),
            ],
        )];
        assert!(
            find_matches(&lines, "abc", true).is_empty(),
            "case_sensitive=true must not match differing case"
        );
        let matches = find_matches(&lines, "AbC", true);
        assert_eq!(
            matches,
            vec![SearchMatch {
                line: 0,
                start_col: 0,
                end_col: 3,
            }]
        );
    }

    #[test]
    fn multiple_non_overlapping_matches_on_same_line() {
        // "xx y xx" → two matches for "xx"
        let lines = vec![SearchLine::new(
            2,
            vec![
                cell_primary(0, 'x'),
                cell_primary(1, 'x'),
                cell_primary(2, ' '),
                cell_primary(3, 'y'),
                cell_primary(4, ' '),
                cell_primary(5, 'x'),
                cell_primary(6, 'x'),
            ],
        )];
        let matches = find_matches(&lines, "xx", false);
        assert_eq!(
            matches,
            vec![
                SearchMatch {
                    line: 2,
                    start_col: 0,
                    end_col: 2,
                },
                SearchMatch {
                    line: 2,
                    start_col: 5,
                    end_col: 7,
                },
            ]
        );
    }

    #[test]
    fn does_not_match_across_line_boundaries() {
        // Line 0 ends with "ab", line 1 starts with "cd" — query "bc" must not match.
        let lines = vec![
            SearchLine::new(0, vec![cell_primary(0, 'a'), cell_primary(1, 'b')]),
            SearchLine::new(1, vec![cell_primary(0, 'c'), cell_primary(1, 'd')]),
        ];
        assert!(
            find_matches(&lines, "bc", false).is_empty(),
            "P0 literal search is single-line only"
        );
        // Sanity: each line still matches its own content.
        assert_eq!(
            find_matches(&lines, "ab", false),
            vec![SearchMatch {
                line: 0,
                start_col: 0,
                end_col: 2,
            }]
        );
        assert_eq!(
            find_matches(&lines, "cd", false),
            vec![SearchMatch {
                line: 1,
                start_col: 0,
                end_col: 2,
            }]
        );
    }

    // =========================================================================
    // TerminalSearchState — default, navigation wrap, replace
    // =========================================================================

    #[test]
    fn default_state_is_hidden_empty_and_case_insensitive() {
        let state = TerminalSearchState::default();
        assert!(!state.visible);
        assert!(state.query.is_empty());
        assert!(state.matches.is_empty());
        assert_eq!(state.current, None);
        assert!(!state.case_sensitive);
    }

    #[test]
    fn with_matches_selects_first_when_nonempty() {
        let state = TerminalSearchState::with_matches("x", three_matches());
        assert_eq!(state.query, "x");
        assert_eq!(state.matches.len(), 3);
        assert_eq!(state.current, Some(0));
    }

    #[test]
    fn navigation_wraps_in_both_directions() {
        let mut state = TerminalSearchState::with_matches("x", three_matches());
        // Start at 0; next → 1
        assert_eq!(state.next().map(|m| m.line), Some(1));
        assert_eq!(state.current, Some(1));
        // next → 2
        assert_eq!(state.next().map(|m| m.line), Some(2));
        // next wraps → 0
        assert_eq!(state.next().map(|m| m.line), Some(0));
        assert_eq!(state.current, Some(0));
        // previous wraps → 2
        assert_eq!(state.previous().map(|m| m.line), Some(2));
        assert_eq!(state.current, Some(2));
        // previous → 1
        assert_eq!(state.previous().map(|m| m.line), Some(1));
        // previous → 0
        assert_eq!(state.previous().map(|m| m.line), Some(0));
    }

    #[test]
    fn empty_matches_next_previous_do_not_panic_and_current_stays_none() {
        let mut state = TerminalSearchState::with_matches("nope", vec![]);
        assert_eq!(state.current, None);
        assert!(state.next().is_none());
        assert_eq!(state.current, None);
        assert!(state.previous().is_none());
        assert_eq!(state.current, None);
    }

    #[test]
    fn replace_results_resets_current_to_first_or_none() {
        let mut state = TerminalSearchState::with_matches("x", three_matches());
        let _ = state.next();
        let _ = state.next();
        assert_eq!(state.current, Some(2));

        // Replace with two new matches → current must reset to Some(0)
        let new_matches = vec![
            SearchMatch {
                line: 10,
                start_col: 0,
                end_col: 1,
            },
            SearchMatch {
                line: 11,
                start_col: 0,
                end_col: 1,
            },
        ];
        state.replace_results("y", new_matches);
        assert_eq!(state.query, "y");
        assert_eq!(state.matches.len(), 2);
        assert_eq!(state.current, Some(0));
        assert_eq!(state.matches[0].line, 10);

        // Replace with empty → current becomes None (no illegal index)
        state.replace_results("z", vec![]);
        assert_eq!(state.query, "z");
        assert!(state.matches.is_empty());
        assert_eq!(state.current, None);
    }

    // =========================================================================
    // status_text — current/total including 0/0
    // =========================================================================

    #[test]
    fn status_text_is_current_over_total_including_zero() {
        let empty = TerminalSearchState::default();
        assert_eq!(empty.status_text(), "0/0");

        let mut state = TerminalSearchState::with_matches("x", three_matches());
        // 1-based display for humans: first match is 1/3
        assert_eq!(state.status_text(), "1/3");
        let _ = state.next();
        assert_eq!(state.status_text(), "2/3");
        let _ = state.next();
        assert_eq!(state.status_text(), "3/3");
        let _ = state.next(); // wrap to first
        assert_eq!(state.status_text(), "1/3");

        state.replace_results("none", vec![]);
        assert_eq!(state.status_text(), "0/0");
    }

    // =========================================================================
    // SearchMatch::contains_cell — half-open boundaries, no off-by-one
    // =========================================================================

    #[test]
    fn contains_cell_half_open_excludes_end_col() {
        // Match covering cols [1, 4): 中@1, spacer@2, b@3
        let m = SearchMatch {
            line: -1,
            start_col: 1,
            end_col: 4,
        };

        assert!(!m.contains_cell(-1, 0), "col before start must be excluded");
        assert!(m.contains_cell(-1, 1), "start_col inclusive");
        assert!(m.contains_cell(-1, 2), "wide spacer col inside range");
        assert!(m.contains_cell(-1, 3), "last character column inclusive");
        assert!(!m.contains_cell(-1, 4), "end_col exclusive — no off-by-one");
        assert!(!m.contains_cell(0, 1), "different line must not match");
    }

    #[test]
    fn contains_cell_single_column_match_boundary() {
        let m = SearchMatch {
            line: 0,
            start_col: 3,
            end_col: 4,
        };
        assert!(!m.contains_cell(0, 2));
        assert!(m.contains_cell(0, 3));
        assert!(!m.contains_cell(0, 4));
    }

    // =========================================================================
    // P0 Task 4 RED-D: search bar open / keys / recompute / reveal (pure)
    //
    // Locks the coordinator API GREEN implements in terminal_search.rs (and
    // wires from main.rs). Effects never encode PTY writes — only UI/state
    // and optional reveal targets for TerminalState::reveal_search_line.
    // =========================================================================

    /// Minimal ASCII fixture with three "x" hits on distinct lines.
    fn ascii_three_x_lines() -> Vec<SearchLine> {
        vec![
            SearchLine::new(0, vec![cell_primary(0, 'x'), cell_primary(1, 'a')]),
            SearchLine::new(1, vec![cell_primary(0, 'b'), cell_primary(1, 'x')]),
            SearchLine::new(2, vec![cell_primary(0, 'x'), cell_primary(1, 'c')]),
        ]
    }

    /// Enter → Next, Shift+Enter → Previous, Escape → Close (independent of state).
    #[test]
    fn search_bar_key_enter_next_shift_enter_previous_escape_close() {
        assert_eq!(
            search_bar_key_action(SearchBarKey::Enter, false),
            SearchBarKeyAction::Next
        );
        assert_eq!(
            search_bar_key_action(SearchBarKey::Enter, true),
            SearchBarKeyAction::Previous
        );
        assert_eq!(
            search_bar_key_action(SearchBarKey::Escape, false),
            SearchBarKeyAction::Close
        );
        assert_eq!(
            search_bar_key_action(SearchBarKey::Escape, true),
            SearchBarKeyAction::Close,
            "Escape closes even with Shift"
        );
    }

    /// open_search: visible + focus-query effect; empty query stays 0/0 with no reveal.
    #[test]
    fn open_search_makes_visible_focuses_query_and_status_zero_without_pty() {
        let mut state = TerminalSearchState::default();
        assert!(!state.visible);

        let effect = open_search(&mut state, &[]);
        assert!(state.visible, "open_search must show the search bar");
        assert_eq!(state.status_text(), "0/0");
        assert!(
            matches!(effect, SearchBarEffect::FocusQuery),
            "open must request query-field focus; got {effect:?}"
        );
        // Contract: effect is UI/focus only — never a PTY write payload.
        assert!(
            !matches!(effect, SearchBarEffect::Reveal(_)),
            "empty query must not request reveal"
        );
    }

    /// open_search with existing query recomputes and can yield a reveal target
    /// (line for reveal_search_line) without any PTY input path.
    #[test]
    fn open_search_with_query_reveals_first_match_without_pty_input() {
        let lines = ascii_three_x_lines();
        let mut state = TerminalSearchState::default();
        state.query = "x".into();

        let effect = open_search(&mut state, &lines);
        assert!(state.visible);
        assert_eq!(state.matches.len(), 3);
        assert_eq!(state.current, Some(0));
        assert_eq!(state.status_text(), "1/3");

        match effect {
            SearchBarEffect::Reveal(m) => {
                assert_eq!(m.line, 0);
                assert_eq!(m.start_col, 0);
                // Coordinator uses m.line → TerminalState::reveal_search_line(m.line)
                // — pure reveal target, no PTY bytes.
            }
            SearchBarEffect::FocusQuery => {
                // Allowed if GREEN returns FocusQuery and expose current via state;
                // still must have a navigation target on state.
                let target = state
                    .current
                    .and_then(|i| state.matches.get(i))
                    .expect("open with hits must expose current match for reveal");
                assert_eq!(target.line, 0);
            }
            other => panic!("expected Reveal or FocusQuery with current match, got {other:?}"),
        }
    }

    /// Enter/Shift+Enter navigate and return Reveal; Escape closes; 0/0 preserved.
    #[test]
    fn apply_search_bar_action_navigates_and_closes_with_stable_status() {
        let mut state = TerminalSearchState::with_matches("x", three_matches());
        state.visible = true;
        assert_eq!(state.status_text(), "1/3");

        let next = apply_search_bar_action(&mut state, SearchBarKeyAction::Next);
        assert_eq!(state.current, Some(1));
        assert_eq!(state.status_text(), "2/3");
        match next {
            SearchBarEffect::Reveal(m) => assert_eq!(m.line, 1),
            other => panic!("Next must Reveal current match, got {other:?}"),
        }

        let prev = apply_search_bar_action(&mut state, SearchBarKeyAction::Previous);
        assert_eq!(state.current, Some(0));
        assert_eq!(state.status_text(), "1/3");
        match prev {
            SearchBarEffect::Reveal(m) => assert_eq!(m.line, 0),
            other => panic!("Previous must Reveal current match, got {other:?}"),
        }

        let closed = apply_search_bar_action(&mut state, SearchBarKeyAction::Close);
        assert!(!state.visible, "Escape/Close must hide the search bar");
        assert!(matches!(closed, SearchBarEffect::Closed));

        // Empty results: status stays 0/0; Next/Previous do not invent a target.
        let mut empty = TerminalSearchState::default();
        empty.visible = true;
        empty.query = "nope".into();
        assert_eq!(empty.status_text(), "0/0");
        assert!(matches!(
            apply_search_bar_action(&mut empty, SearchBarKeyAction::Next),
            SearchBarEffect::None
        ));
        assert!(matches!(
            apply_search_bar_action(&mut empty, SearchBarKeyAction::Previous),
            SearchBarEffect::None
        ));
        assert_eq!(empty.status_text(), "0/0");
        assert_eq!(empty.current, None);
    }

    /// Query / case_sensitive changes recompute matches and reset current to first.
    #[test]
    fn recompute_search_on_query_or_case_change_resets_current_to_first() {
        let lines = ascii_three_x_lines();
        let mut state = TerminalSearchState::with_matches("x", three_matches());
        state.visible = true;
        let _ = state.next();
        let _ = state.next();
        assert_eq!(state.current, Some(2));

        // Query change → recompute + current = first hit
        state.query = "x".into();
        let effect = recompute_search(&mut state, &lines);
        assert_eq!(state.matches.len(), 3);
        assert_eq!(
            state.current,
            Some(0),
            "query change must reset current to first"
        );
        assert_eq!(state.status_text(), "1/3");
        match effect {
            SearchBarEffect::Reveal(m) => assert_eq!(m.line, 0),
            SearchBarEffect::None => {
                // None only acceptable if GREEN prefers caller to read state.current
                assert_eq!(state.matches[0].line, 0);
            }
            other => panic!("recompute with hits should Reveal or None+state, got {other:?}"),
        }

        // Navigate away, then case_sensitive flip recomputes and resets again
        let _ = state.next();
        assert_eq!(state.current, Some(1));
        state.case_sensitive = true;
        state.query = "X".into(); // no uppercase X in fixture
        let effect_cs = recompute_search(&mut state, &lines);
        assert!(
            state.matches.is_empty(),
            "case-sensitive 'X' must miss lowercase 'x' fixture"
        );
        assert_eq!(state.current, None);
        assert_eq!(state.status_text(), "0/0");
        assert!(
            matches!(effect_cs, SearchBarEffect::None),
            "no matches → no reveal"
        );

        // Case-insensitive again restores hits and first current
        state.case_sensitive = false;
        state.query = "x".into();
        let _ = recompute_search(&mut state, &lines);
        assert_eq!(state.matches.len(), 3);
        assert_eq!(state.current, Some(0));
        assert_eq!(state.status_text(), "1/3");
    }

    #[test]
    fn search_history_is_per_state_recent_first_deduplicated_and_bounded() {
        let mut state = TerminalSearchState::default();
        state.query = "alpha".into();
        state.remember_query();
        state.query = "beta".into();
        state.remember_query();
        state.query = "alpha".into();
        state.remember_query();

        assert_eq!(state.history, ["alpha", "beta"]);

        for index in 0..(MAX_SEARCH_HISTORY_ITEMS + 5) {
            state.query = format!("query-{index}");
            state.remember_query();
        }
        assert_eq!(state.history.len(), MAX_SEARCH_HISTORY_ITEMS);
        assert_eq!(state.history.first().map(String::as_str), Some("query-24"));
        assert_eq!(state.history.last().map(String::as_str), Some("query-5"));

        let other_tab = TerminalSearchState::default();
        assert!(other_tab.history.is_empty());
    }

    #[test]
    fn navigation_records_non_empty_query_but_close_and_blank_do_not() {
        let mut state = TerminalSearchState::with_matches("needle", three_matches());
        let _ = apply_search_bar_action(&mut state, SearchBarKeyAction::Next);
        assert_eq!(state.history, ["needle"]);

        state.query = "   ".into();
        let _ = apply_search_bar_action(&mut state, SearchBarKeyAction::Previous);
        let _ = apply_search_bar_action(&mut state, SearchBarKeyAction::Close);
        assert_eq!(state.history, ["needle"]);
    }
}
