use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const MAX_CANDIDATES: usize = 8;
pub const MAX_HISTORY_ITEMS: usize = 5_000;
pub const MAX_HISTORY_BYTES: u64 = 2 * 1024 * 1024;

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
        Self {
            generation,
            token: token.into(),
        }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn successor(&self) -> Self {
        Self::new(self.generation.wrapping_add(1).max(1))
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
    (!line.is_empty() && !line.chars().any(char::is_control)).then(|| line.to_owned())
}

pub fn parse_bash_history(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let lines = text.lines().collect::<Vec<_>>();
    let timestamped = lines.iter().any(|line| is_timestamp(line));
    let mut oldest_first = Vec::new();

    if timestamped {
        let mut record = Vec::new();
        let mut seen_timestamp = false;
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
                if seen_timestamp {
                    flush(&mut record, &mut oldest_first);
                } else {
                    seen_timestamp = true;
                }
            } else if seen_timestamp {
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
    let starts_at_line_boundary = if start == 0 {
        true
    } else {
        let mut previous_byte = [0];
        file.seek(SeekFrom::Start(start - 1))
            .map_err(|error| error.to_string())?;
        file.read_exact(&mut previous_byte)
            .map_err(|error| error.to_string())?;
        previous_byte[0] == b'\n'
    };
    file.seek(SeekFrom::Start(start))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(max_bytes)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if start > 0 && !starts_at_line_boundary {
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

    pub fn session(&self) -> &CompletionSessionKey {
        &self.session
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    pub fn candidates(&self) -> &[String] {
        &self.candidates
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_candidate(&self) -> Option<&str> {
        self.candidates.get(self.selected).map(String::as_str)
    }

    pub fn is_popup_visible(&self) -> bool {
        !self.suppressed && self.pending_fill.is_none() && !self.candidates.is_empty()
    }

    pub fn fill_pending(&self) -> bool {
        self.pending_fill.is_some()
    }

    pub fn replace_history(&mut self, history: Vec<String>) {
        let mut seen = HashSet::new();
        self.history = history
            .into_iter()
            .filter(|command| safe_command(command.as_str()).is_some())
            .filter(|command| seen.insert(command.clone()))
            .take(MAX_HISTORY_ITEMS)
            .collect();
    }

    pub fn merge_executed(&mut self, command: &str) {
        if safe_command(command).is_none() {
            return;
        }
        self.history.retain(|entry| entry != command);
        self.history.insert(0, command.to_owned());
        self.history.truncate(MAX_HISTORY_ITEMS);
    }

    pub fn refresh(&mut self, prefix: &str) {
        self.candidates = rank_candidates(&self.history, prefix);
        self.selected = self.selected.min(self.candidates.len().saturating_sub(1));
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.candidates.is_empty() {
            return;
        }
        let candidate_count = self.candidates.len();
        let offset = delta.rem_euclid(candidate_count as isize) as usize;
        self.selected = (self.selected + offset) % candidate_count;
    }

    pub fn dismiss(&mut self) {
        self.suppressed = true;
    }

    pub fn on_user_edit(&mut self) {
        self.suppressed = false;
        self.pending_fill = None;
    }

    pub fn begin_fill(&mut self, request_id: u64) {
        self.pending_fill = Some(request_id);
    }

    pub fn pending_fill_matches(&self, request_id: u64) -> bool {
        self.pending_fill == Some(request_id)
    }

    pub fn finish_fill(&mut self, request_id: u64) -> bool {
        if self.pending_fill != Some(request_id) {
            return false;
        }
        self.pending_fill = None;
        self.suppressed = true;
        self.candidates.clear();
        true
    }

    pub fn fail_fill(&mut self, request_id: u64) {
        if self.pending_fill == Some(request_id) {
            self.pending_fill = None;
        }
    }

    pub fn cancel_pending_fill(&mut self) {
        self.pending_fill = None;
    }

    pub fn reset_session(&mut self, session: CompletionSessionKey) {
        *self = Self::new(session);
    }

    pub fn set_history_path(&mut self, path: String) {
        self.history_path = Some(path);
    }

    pub fn history_path(&self) -> Option<&str> {
        self.history_path.as_deref()
    }

    pub fn set_sftp_ready(&mut self, ready: bool) {
        self.sftp_ready = ready;
    }

    pub fn sftp_ready(&self) -> bool {
        self.sftp_ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TemporaryHistoryFile {
        path: std::path::PathBuf,
    }

    impl TemporaryHistoryFile {
        fn new(contents: &[u8]) -> Self {
            let path = std::env::temp_dir().join(format!(
                "liteterm-smart-completion-{}.history",
                uuid::Uuid::new_v4().simple()
            ));
            let file = Self { path };
            std::fs::write(&file.path, contents).unwrap();
            file
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TemporaryHistoryFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[test]
    fn timestamped_multiline_entries_are_dropped() {
        let data = b"#100\nls -la\n#101\nprintf 'a\nb'\n#102\ngit status\n";
        assert_eq!(parse_bash_history(data), ["git status", "ls -la"]);
    }

    #[test]
    fn timestamped_history_ignores_lines_before_the_first_timestamp() {
        let data = b"garbage\n#100\nls\n";
        assert_eq!(parse_bash_history(data), ["ls"]);
    }

    #[test]
    fn untimestamped_lines_are_independent_and_control_bytes_are_rejected() {
        let data = b"echo one\n\nprintf two\nbad\tcommand\n";
        assert_eq!(parse_bash_history(data), ["printf two", "echo one"]);
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
        let history = (0..20)
            .map(|index| format!("echo {index}"))
            .collect::<Vec<_>>();
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
    fn state_rejects_commands_with_trailing_carriage_returns() {
        let mut replaced = CompletionState::new(CompletionSessionKey::new_for_test(1, "replace"));
        replaced.replace_history(vec!["safe".into(), "bad\r".into()]);

        let mut merged = CompletionState::new(CompletionSessionKey::new_for_test(1, "merge"));
        merged.replace_history(vec!["safe".into()]);
        merged.merge_executed("bad\r");

        assert_eq!(
            (replaced.history(), merged.history()),
            (&["safe".to_owned()][..], &["safe".to_owned()][..])
        );
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

    #[test]
    fn selection_wraps_for_large_delta_without_overflow() {
        let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "c"));
        state.replace_history(vec!["git a".into(), "git b".into(), "git c".into()]);
        state.refresh("git");
        state.move_selection(1);
        state.move_selection(isize::MAX);
        assert_eq!(state.selected(), 2);
    }

    #[test]
    fn selection_wraps_for_minimum_delta_without_overflow() {
        let mut state = CompletionState::new(CompletionSessionKey::new_for_test(1, "d"));
        state.replace_history(vec!["git a".into(), "git b".into(), "git c".into()]);
        state.refresh("git");
        state.move_selection(isize::MIN);
        assert_eq!(state.selected(), 1);
    }

    #[test]
    fn successor_rotates_token_and_increments_generation() {
        let current = CompletionSessionKey::new_for_test(7, "old");
        let next = current.successor();
        assert_eq!(next.generation, 8);
        assert_ne!(next.token(), current.token());
    }

    #[test]
    fn successor_wraps_max_generation_to_one_and_rotates_token() {
        let current = CompletionSessionKey::new_for_test(u64::MAX, "old");
        let next = current.successor();
        assert_eq!(next.generation, 1);
        assert_ne!(next.token(), current.token());
    }

    #[test]
    fn history_tail_drops_a_partial_first_line() {
        let file = TemporaryHistoryFile::new(b"old-command\nnew-command\n");
        assert_eq!(
            read_history_tail(file.path(), 15).unwrap(),
            b"new-command\n"
        );
    }

    #[test]
    fn history_tail_keeps_a_complete_first_line_at_an_exact_boundary() {
        let file = TemporaryHistoryFile::new(b"old-command\nnew-command\n");
        assert_eq!(
            read_history_tail(file.path(), 12).unwrap(),
            b"new-command\n"
        );
    }

    #[test]
    fn history_tail_of_an_empty_file_is_empty() {
        let file = TemporaryHistoryFile::new(b"");
        assert!(read_history_tail(file.path(), 12).unwrap().is_empty());
    }

    #[test]
    fn history_tail_with_zero_byte_limit_is_empty() {
        let file = TemporaryHistoryFile::new(b"command\n");
        assert!(read_history_tail(file.path(), 0).unwrap().is_empty());
    }
}
