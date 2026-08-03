// Native IME composition state machine (pure logic).
//
// Production API (Task 5 GREEN-A):
//   InputOwner, ImeAction, ImeState
//   preedit / commit / filter_keyboard_text / take_duplicate_guard
//   on_enabled / on_disabled / on_focus_lost / set_owner
//   is_enabled / has_active_preedit / owner
//
// No winit window, GPU, or timers — unit-testable pure state only.

/// Who currently owns text input focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputOwner {
    #[default]
    Terminal,
    Egui,
}

/// Result of an IME state transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeAction {
    None,
    Redraw,
    Commit(String),
}

/// Pure IME composition / duplicate-suppression state.
///
/// No timers, winit, GPU, IO, or unsafe — unit-testable only.
#[derive(Debug, Clone, Default)]
pub struct ImeState {
    /// Whether the platform IME is enabled (informational for wiring).
    enabled: bool,
    /// Current preedit (composition) text shown as overlay.
    preedit: String,
    /// Preedit cursor range as UTF-8 byte offsets into `preedit`, if any.
    cursor: Option<(usize, usize)>,
    /// One-shot PTY echo suppression target after a Terminal commit.
    duplicate_guard: Option<String>,
    /// Accumulated keyboard text matching a prefix of `duplicate_guard`.
    guard_seen: String,
    /// Current focus owner (Terminal vs Egui dialogs).
    owner: InputOwner,
}

impl ImeState {
    /// Update preedit overlay. Never commits to PTY and never arms the guard.
    pub fn preedit(&mut self, text: String, cursor: Option<(usize, usize)>) -> ImeAction {
        if text.is_empty() {
            self.preedit.clear();
            self.cursor = None;
        } else {
            self.preedit = text;
            self.cursor = cursor;
        }
        ImeAction::Redraw
    }

    /// Current preedit string (empty when idle).
    pub fn preedit_text(&self) -> &str {
        &self.preedit
    }

    /// Current preedit cursor range, if any.
    pub fn preedit_cursor(&self) -> Option<(usize, usize)> {
        self.cursor
    }

    /// Whether visible composition text is currently active.
    pub fn has_active_preedit(&self) -> bool {
        !self.preedit.is_empty()
    }

    /// Finish composition.
    ///
    /// - `Terminal`: return `Commit(text)`, clear preedit; arm one-shot duplicate
    ///   guard only when `text` is nonempty.
    /// - `Egui`: return `Redraw`, clear preedit; never arm PTY guard.
    pub fn commit(&mut self, text: String, owner: InputOwner) -> ImeAction {
        self.preedit.clear();
        self.cursor = None;
        self.duplicate_guard = None;
        self.guard_seen.clear();

        match owner {
            InputOwner::Terminal => {
                // Empty commit must not arm an empty guard.
                if !text.is_empty() {
                    self.duplicate_guard = Some(text.clone());
                }
                ImeAction::Commit(text)
            }
            InputOwner::Egui => {
                // Dialog path: no PTY write, no echo suppression.
                ImeAction::Redraw
            }
        }
    }

    /// Take and clear the armed duplicate guard (if any).
    pub fn take_duplicate_guard(&mut self) -> Option<String> {
        self.guard_seen.clear();
        self.duplicate_guard.take()
    }

    /// Filter keyboard text that may be a post-commit IME echo or a leak
    /// during active composition.
    ///
    /// Returns `None` when the chunk must be swallowed; `Some(text)` (or a
    /// remainder after overshoot) when it should reach the PTY.
    ///
    /// All prefix checks are UTF-8 safe (`str::starts_with` / `len` on
    /// complete prefixes — never slicing at invalid boundaries).
    pub fn filter_keyboard_text(&mut self, text: &str) -> Option<String> {
        // Active composition: suppress leaked keyboard text entirely.
        if !self.preedit.is_empty() {
            return None;
        }

        let Some(guard) = self.duplicate_guard.as_ref() else {
            return Some(text.to_string());
        };

        // candidate = already-seen prefix + this chunk (UTF-8 safe concat).
        let mut candidate = String::with_capacity(self.guard_seen.len() + text.len());
        candidate.push_str(&self.guard_seen);
        candidate.push_str(text);

        if candidate == *guard {
            // Exact full match — suppress once, clear guard.
            self.duplicate_guard = None;
            self.guard_seen.clear();
            return None;
        }

        if guard.starts_with(&candidate) {
            // Partial prefix of the committed string — keep accumulating.
            self.guard_seen = candidate;
            return None;
        }

        if candidate.starts_with(guard.as_str()) {
            // Overshoot: chunk begins with the remaining guard; emit remainder only.
            // `guard.len()` is a valid UTF-8 boundary because `starts_with` succeeded.
            let remainder = candidate[guard.len()..].to_string();
            self.duplicate_guard = None;
            self.guard_seen.clear();
            return Some(remainder);
        }

        // Mismatch: the current chunk is legitimate input. Clear the stale
        // guard and pass this chunk through without replaying prior prefixes.
        self.duplicate_guard = None;
        self.guard_seen.clear();
        Some(text.to_string())
    }

    /// IME enabled by the platform — informational only, never commits or
    /// clears in-progress composition state.
    pub fn on_enabled(&mut self) -> ImeAction {
        self.enabled = true;
        ImeAction::None
    }

    /// Whether the platform IME is currently enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// IME disabled by the platform — clear transient state, never commit.
    pub fn on_disabled(&mut self) -> ImeAction {
        self.enabled = false;
        self.clear_transient()
    }

    /// Window / widget focus lost — clear transient state, never commit.
    pub fn on_focus_lost(&mut self) -> ImeAction {
        self.clear_transient()
    }

    /// Current text input owner.
    pub fn owner(&self) -> InputOwner {
        self.owner
    }

    /// Change input owner. A real owner transition clears preedit + guard;
    /// reasserting the current owner preserves in-progress state.
    pub fn set_owner(&mut self, owner: InputOwner) -> ImeAction {
        if self.owner == owner {
            return ImeAction::None;
        }
        self.owner = owner;
        self.clear_transient()
    }

    /// Drop preedit, cursor, and duplicate-suppression state.
    /// Returns `Redraw` if anything visible/transient was present, else `None`.
    fn clear_transient(&mut self) -> ImeAction {
        let had_state = !self.preedit.is_empty()
            || self.cursor.is_some()
            || self.duplicate_guard.is_some()
            || !self.guard_seen.is_empty();
        self.preedit.clear();
        self.cursor = None;
        self.duplicate_guard = None;
        self.guard_seen.clear();
        if had_state {
            ImeAction::Redraw
        } else {
            ImeAction::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ImeAction, ImeState, InputOwner};

    // -------------------------------------------------------------------------
    // Preedit
    // -------------------------------------------------------------------------

    #[test]
    fn preedit_returns_redraw_and_stores_text_and_cursor() {
        let mut ime = ImeState::default();
        assert_eq!(ime.preedit("zhong".into(), Some((0, 5))), ImeAction::Redraw);
        assert_eq!(ime.preedit_text(), "zhong");
        assert_eq!(ime.preedit_cursor(), Some((0, 5)));
    }

    #[test]
    fn empty_preedit_clears_text_and_cursor() {
        let mut ime = ImeState::default();
        ime.preedit("zhong".into(), Some((1, 3)));
        assert_eq!(ime.preedit(String::new(), None), ImeAction::Redraw);
        assert_eq!(ime.preedit_text(), "");
        assert_eq!(ime.preedit_cursor(), None);
    }

    #[test]
    fn preedit_never_returns_commit() {
        let mut ime = ImeState::default();
        assert_eq!(ime.preedit("ni".into(), Some((0, 2))), ImeAction::Redraw);
        assert_eq!(ime.preedit("nihao".into(), Some((0, 5))), ImeAction::Redraw);
        assert_eq!(ime.preedit(String::new(), None), ImeAction::Redraw);
        // Guard must stay unarmed — preedit alone never commits to PTY.
        assert_eq!(ime.take_duplicate_guard(), None);
    }

    // -------------------------------------------------------------------------
    // Terminal commit
    // -------------------------------------------------------------------------

    #[test]
    fn terminal_commit_returns_commit_once_clears_preedit_and_arms_guard() {
        let mut ime = ImeState::default();
        ime.preedit("zhong".into(), Some((0, 5)));
        assert_eq!(
            ime.commit("中".into(), InputOwner::Terminal),
            ImeAction::Commit("中".into())
        );
        assert_eq!(ime.preedit_text(), "");
        assert_eq!(ime.preedit_cursor(), None);
        // Guard is armed for exactly the committed string (inspected via take).
        assert_eq!(ime.take_duplicate_guard(), Some("中".into()));
        // take consumes; second take is empty.
        assert_eq!(ime.take_duplicate_guard(), None);
    }

    #[test]
    fn empty_terminal_commit_clears_stale_guard() {
        let mut ime = ImeState::default();
        let _ = ime.commit("中文".into(), InputOwner::Terminal);
        assert_eq!(ime.filter_keyboard_text("中"), None);

        assert_eq!(
            ime.commit(String::new(), InputOwner::Terminal),
            ImeAction::Commit(String::new())
        );
        assert_eq!(ime.filter_keyboard_text("文"), Some("文".into()));
    }

    // -------------------------------------------------------------------------
    // Egui owner commit
    // -------------------------------------------------------------------------

    #[test]
    fn egui_commit_never_returns_pty_commit_and_does_not_arm_guard() {
        let mut ime = ImeState::default();
        let _ = ime.commit("中".into(), InputOwner::Terminal);
        ime.preedit("zhong".into(), Some((0, 5)));
        assert_eq!(
            ime.commit("dialog".into(), InputOwner::Egui),
            ImeAction::Redraw
        );
        // Preedit still cleared (composition finished for the UI path).
        assert_eq!(ime.preedit_text(), "");
        assert_eq!(ime.preedit_cursor(), None);
        // The Egui commit clears the stale Terminal guard, so matching keyboard
        // text must not be swallowed later.
        assert_eq!(
            ime.filter_keyboard_text("中"),
            Some("中".into()),
            "without a PTY guard, keyboard text must pass through"
        );
    }

    // -------------------------------------------------------------------------
    // Duplicate keyboard suppression (post-commit)
    // -------------------------------------------------------------------------

    #[test]
    fn exact_duplicate_keyboard_text_is_suppressed_once() {
        let mut ime = ImeState::default();
        assert_eq!(
            ime.commit("中".into(), InputOwner::Terminal),
            ImeAction::Commit("中".into())
        );
        // First exact echo is fully suppressed.
        assert_eq!(ime.filter_keyboard_text("中"), None);
        // Guard is one-shot; a later identical keystroke is real input.
        assert_eq!(ime.filter_keyboard_text("中"), Some("中".into()));
    }

    #[test]
    fn partial_duplicate_chunks_accumulate_then_suppress() {
        let mut ime = ImeState::default();
        assert_eq!(
            ime.commit("中文".into(), InputOwner::Terminal),
            ImeAction::Commit("中文".into())
        );
        // Partial prefixes of the committed text accumulate and stay suppressed.
        assert_eq!(ime.filter_keyboard_text("中"), None);
        assert_eq!(ime.filter_keyboard_text("文"), None);
        // After full match, guard is cleared.
        assert_eq!(ime.filter_keyboard_text("x"), Some("x".into()));
    }

    #[test]
    fn overshoot_duplicate_returns_only_remainder() {
        let mut ime = ImeState::default();
        assert_eq!(
            ime.commit("中".into(), InputOwner::Terminal),
            ImeAction::Commit("中".into())
        );
        // Chunk starts with committed text plus extra — only remainder goes to PTY.
        assert_eq!(ime.filter_keyboard_text("中x"), Some("x".into()));
        // Guard cleared after overshoot handling.
        assert_eq!(ime.filter_keyboard_text("中"), Some("中".into()));
    }

    #[test]
    fn mismatch_clears_guard_and_preserves_unrelated_chunk() {
        let mut ime = ImeState::default();
        assert_eq!(
            ime.commit("中".into(), InputOwner::Terminal),
            ImeAction::Commit("中".into())
        );
        // Unrelated keyboard text under an armed guard is legitimate input.
        assert_eq!(ime.filter_keyboard_text("a"), Some("a".into()));
        // Subsequent text is no longer suppressed.
        assert_eq!(ime.filter_keyboard_text("b"), Some("b".into()));
    }

    #[test]
    fn partial_prefix_mismatch_preserves_current_chunk_and_clears_guard() {
        let mut ime = ImeState::default();
        assert_eq!(
            ime.commit("中文".into(), InputOwner::Terminal),
            ImeAction::Commit("中文".into())
        );
        assert_eq!(ime.filter_keyboard_text("中"), None);
        assert_eq!(ime.filter_keyboard_text("x"), Some("x".into()));
        assert_eq!(ime.filter_keyboard_text("文"), Some("文".into()));
    }

    // -------------------------------------------------------------------------
    // Active composition keyboard leak
    // -------------------------------------------------------------------------

    #[test]
    fn keyboard_text_during_active_composition_is_suppressed() {
        let mut ime = ImeState::default();
        ime.preedit("zhong".into(), Some((0, 5)));
        assert_eq!(ime.filter_keyboard_text("z"), None);
        assert_eq!(ime.filter_keyboard_text("zhong"), None);
        // Composition state preserved; no accidental commit/guard.
        assert_eq!(ime.preedit_text(), "zhong");
        assert_eq!(ime.preedit_cursor(), Some((0, 5)));
        assert_eq!(ime.take_duplicate_guard(), None);
    }

    // -------------------------------------------------------------------------
    // Disabled / focus loss / owner change — clear without commit
    // -------------------------------------------------------------------------

    #[test]
    fn enabled_is_informational_and_does_not_commit_or_clear_transient_state() {
        let mut ime = ImeState::default();
        assert!(!ime.is_enabled());
        let _ = ime.commit("中".into(), InputOwner::Terminal);
        ime.preedit("zhong".into(), Some((0, 5)));

        assert_eq!(ime.on_enabled(), ImeAction::None);
        assert!(ime.is_enabled());
        assert_eq!(ime.preedit_text(), "zhong");
        assert_eq!(ime.preedit_cursor(), Some((0, 5)));
        assert_eq!(ime.take_duplicate_guard(), Some("中".into()));
    }

    #[test]
    fn disabled_clears_preedit_and_guard_without_commit() {
        let mut ime = ImeState::default();
        let _ = ime.on_enabled();
        assert!(ime.is_enabled());
        ime.preedit("zhong".into(), Some((0, 5)));
        let _ = ime.commit("中".into(), InputOwner::Terminal);
        // Re-arm a composition + guard scenario: commit already armed guard;
        // also put preedit back as if a new composition started.
        ime.preedit("hao".into(), Some((0, 3)));
        // Ensure guard is still present from terminal commit (not taken).
        // (commit arms guard; preedit must not clear it — only explicit cancel paths do)

        let action = ime.on_disabled();
        assert_ne!(action, ImeAction::Commit("中".into()));
        assert_ne!(action, ImeAction::Commit("hao".into()));
        // Prefer Redraw when there was visible/transient state to drop.
        assert_eq!(action, ImeAction::Redraw);
        assert_eq!(ime.preedit_text(), "");
        assert_eq!(ime.preedit_cursor(), None);
        assert_eq!(ime.take_duplicate_guard(), None);
        assert!(!ime.is_enabled());
    }

    #[test]
    fn focus_lost_clears_preedit_and_guard_without_commit() {
        let mut ime = ImeState::default();
        ime.preedit("zhong".into(), Some((0, 5)));
        let _ = ime.commit("中".into(), InputOwner::Terminal);
        ime.preedit("ni".into(), Some((0, 2)));

        let action = ime.on_focus_lost();
        assert_ne!(
            matches!(action, ImeAction::Commit(_)),
            true,
            "focus loss must never emit PTY Commit"
        );
        assert_eq!(action, ImeAction::Redraw);
        assert_eq!(ime.preedit_text(), "");
        assert_eq!(ime.preedit_cursor(), None);
        assert_eq!(ime.take_duplicate_guard(), None);
    }

    #[test]
    fn terminal_to_egui_owner_change_clears_without_commit() {
        let mut ime = ImeState::default();
        assert_eq!(ime.owner(), InputOwner::Terminal);
        // Default owner is Terminal-facing composition.
        ime.preedit("zhong".into(), Some((0, 5)));
        let _ = ime.commit("中".into(), InputOwner::Terminal);
        ime.preedit("partial".into(), Some((0, 7)));

        let action = ime.set_owner(InputOwner::Egui);
        assert!(
            !matches!(action, ImeAction::Commit(_)),
            "owner change must never emit PTY Commit"
        );
        assert_eq!(action, ImeAction::Redraw);
        assert_eq!(ime.preedit_text(), "");
        assert_eq!(ime.preedit_cursor(), None);
        assert_eq!(ime.take_duplicate_guard(), None);
        assert_eq!(ime.owner(), InputOwner::Egui);
        // Keyboard text must not be swallowed after owner handoff.
        assert_eq!(ime.filter_keyboard_text("中"), Some("中".into()));
    }

    #[test]
    fn unchanged_owner_preserves_preedit_and_duplicate_guard() {
        let mut ime = ImeState::default();
        let _ = ime.commit("中".into(), InputOwner::Terminal);
        ime.preedit("zhong".into(), Some((0, 5)));

        assert_eq!(ime.set_owner(InputOwner::Terminal), ImeAction::None);
        assert_eq!(ime.owner(), InputOwner::Terminal);
        assert_eq!(ime.preedit_text(), "zhong");
        assert_eq!(ime.preedit_cursor(), Some((0, 5)));
        assert_eq!(ime.take_duplicate_guard(), Some("中".into()));
    }

    #[test]
    fn active_preedit_tracks_visible_composition() {
        let mut ime = ImeState::default();
        assert!(!ime.has_active_preedit());
        ime.preedit("zhong".into(), Some((0, 5)));
        assert!(ime.has_active_preedit());
        ime.preedit(String::new(), None);
        assert!(!ime.has_active_preedit());
    }

    #[test]
    fn idle_clear_paths_are_idempotent() {
        let mut ime = ImeState::default();
        // No composition / guard: cancel paths may return None or Redraw, but never Commit.
        for action in [
            ime.on_disabled(),
            ime.on_focus_lost(),
            ime.set_owner(InputOwner::Egui),
        ] {
            assert!(
                !matches!(action, ImeAction::Commit(_)),
                "idle clear must not commit, got {action:?}"
            );
        }
        assert_eq!(ime.preedit_text(), "");
        assert_eq!(ime.take_duplicate_guard(), None);
    }
}
