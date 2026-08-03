use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::adb_history::AdbHistoryIdentity;

pub const MAX_CANDIDATES: usize = 5;
pub const MAX_HISTORY_ITEMS: usize = 5_000;
pub const MAX_HISTORY_BYTES: u64 = 2 * 1024 * 1024;
const HISTORY_DISABLED_REASON: &str = "当前会话未启用历史记录补全";
const HISTORY_ERROR_REASON: &str = "历史记录加载失败";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ForegroundCompletion {
    #[default]
    IntegratedBash,
    AdbShell,
    FishInAdb,
    AwaitingBashPrompt,
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryStatus {
    Disabled { reason: &'static str },
    Loading,
    Ready { items: usize },
    Error { reason: &'static str },
}

#[derive(Clone, PartialEq, Eq)]
pub struct HistoryLoadRequest {
    generation: u64,
    session: CompletionSessionKey,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AdbHistoryLoadRequest {
    generation: u64,
    adb_epoch: u64,
    session: CompletionSessionKey,
    identity: AdbHistoryIdentity,
}

impl AdbHistoryLoadRequest {
    pub fn identity(&self) -> &AdbHistoryIdentity {
        &self.identity
    }
}

impl fmt::Debug for AdbHistoryLoadRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdbHistoryLoadRequest")
            .field("generation", &self.generation)
            .field("adb_epoch", &self.adb_epoch)
            .field("session", &self.session)
            .field("identity", &self.identity)
            .finish()
    }
}

impl fmt::Debug for HistoryLoadRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HistoryLoadRequest")
            .field("generation", &self.generation)
            .field("session", &self.session)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
struct PendingFill {
    request_id: u64,
    session: CompletionSessionKey,
    candidate: String,
}

impl fmt::Debug for PendingFill {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingFill")
            .field("request_id", &self.request_id)
            .field("session", &self.session)
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
    history
        .into_iter()
        .filter(|command| command.as_str() != prefix)
        .filter(|command| command.starts_with(prefix))
        .filter(|command| seen.insert((*command).clone()))
        .take(MAX_CANDIDATES)
        .cloned()
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
    foreground: ForegroundCompletion,
    surface_tracking_paused: bool,
    tracked_input: Option<String>,
    history: Vec<String>,
    history_generation: u64,
    candidates: Vec<String>,
    candidate_cache_key: Option<CandidateCacheKey>,
    #[cfg(test)]
    candidate_cache_key_build_count: usize,
    selected: usize,
    suppressed: bool,
    pending_fill: Option<PendingFill>,
    host_history_status: HistoryStatus,
    history_load_generation: u64,
    pending_history_load: Option<HistoryLoadRequest>,
    host_loaded_history: Vec<String>,
    host_session_history: Vec<String>,
    next_adb_epoch: u64,
    active_adb: Option<AdbCompletionContext>,
    history_path: Option<String>,
    sftp_ready: bool,
}

struct AdbCompletionContext {
    identity: AdbHistoryIdentity,
    epoch: u64,
    loaded_history: Vec<String>,
    session_history: Vec<String>,
    history_status: HistoryStatus,
    load_generation: u64,
    pending_load: Option<AdbHistoryLoadRequest>,
}

#[derive(Clone, PartialEq, Eq)]
struct CandidateCacheKey {
    session: CompletionSessionKey,
    history_generation: u64,
    prefix: String,
}

impl CompletionState {
    pub fn new(session: CompletionSessionKey) -> Self {
        Self {
            session,
            foreground: ForegroundCompletion::IntegratedBash,
            surface_tracking_paused: false,
            tracked_input: Some(String::new()),
            history: Vec::new(),
            history_generation: 0,
            candidates: Vec::new(),
            candidate_cache_key: None,
            #[cfg(test)]
            candidate_cache_key_build_count: 0,
            selected: 0,
            suppressed: false,
            pending_fill: None,
            host_history_status: HistoryStatus::Disabled {
                reason: HISTORY_DISABLED_REASON,
            },
            history_load_generation: 0,
            pending_history_load: None,
            host_loaded_history: Vec::new(),
            host_session_history: Vec::new(),
            next_adb_epoch: 0,
            active_adb: None,
            history_path: None,
            sftp_ready: false,
        }
    }

    pub fn session(&self) -> &CompletionSessionKey {
        &self.session
    }

    pub fn tracked_input(&self) -> Option<&str> {
        self.tracked_input.as_deref()
    }

    pub fn direct_fill_prefix(&self) -> Option<&str> {
        (self.foreground == ForegroundCompletion::AdbShell)
            .then(|| self.tracked_input())
            .flatten()
            .filter(|prefix| !prefix.is_empty())
    }

    pub fn completion_suspended_without_prompt(&self) -> bool {
        self.foreground != ForegroundCompletion::AdbShell
    }

    pub fn observe_submission(
        &mut self,
        submission: Option<&str>,
        authenticated_prompt: bool,
    ) -> Option<String> {
        let command = submission.and_then(simple_command_words)?;
        let mut entered_adb = None;
        self.foreground = match self.foreground {
            ForegroundCompletion::IntegratedBash if authenticated_prompt => {
                if let Some(serial) = interactive_adb_serial(&command) {
                    entered_adb = Some(serial.to_owned());
                    ForegroundCompletion::AdbShell
                } else {
                    ForegroundCompletion::IntegratedBash
                }
            }
            ForegroundCompletion::AdbShell if is_interactive_fish(&command) => {
                ForegroundCompletion::FishInAdb
            }
            ForegroundCompletion::AdbShell if is_shell_exit(&command) => {
                self.leave_adb_history();
                ForegroundCompletion::AwaitingBashPrompt
            }
            ForegroundCompletion::FishInAdb if is_shell_exit(&command) => {
                ForegroundCompletion::AdbShell
            }
            foreground => foreground,
        };
        entered_adb
    }

    pub fn observe_empty_ctrl_d(&mut self) -> bool {
        if self.tracked_input.as_deref() != Some("") {
            return false;
        }
        self.foreground = match self.foreground {
            ForegroundCompletion::AdbShell => {
                self.leave_adb_history();
                ForegroundCompletion::AwaitingBashPrompt
            }
            ForegroundCompletion::FishInAdb => ForegroundCompletion::AdbShell,
            _ => return false,
        };
        self.clear_candidates();
        self.pending_fill = None;
        self.suppressed = false;
        self.tracked_input = Some(String::new());
        true
    }

    pub fn observe_authenticated_prompt(&mut self) {
        if self.foreground != ForegroundCompletion::IntegratedBash {
            self.leave_adb_history();
            self.foreground = ForegroundCompletion::IntegratedBash;
            self.clear_candidates();
            self.pending_fill = None;
            self.suppressed = false;
            self.tracked_input = Some(String::new());
        }
    }

    pub fn pause_surface_tracking(&mut self) {
        self.clear_candidates();
        self.pending_fill = None;
        self.suppressed = false;
        self.surface_tracking_paused = true;
        self.tracked_input = None;
    }

    pub fn resume_surface_tracking(&mut self) {
        if self.surface_tracking_paused {
            self.surface_tracking_paused = false;
            self.tracked_input = Some(String::new());
        }
    }

    pub fn current_input(&self, fallback: Option<&str>) -> Option<String> {
        self.tracked_input
            .clone()
            .or_else(|| fallback.map(str::to_owned))
    }

    pub fn track_user_input(&mut self, input: &str) {
        match input {
            "\x15" => self.tracked_input = Some(String::new()),
            "\x7f" | "\x08" => {
                if let Some(tracked_input) = &mut self.tracked_input {
                    tracked_input.pop();
                }
            }
            _ if !input.is_empty() && !input.chars().any(char::is_control) => {
                if let Some(tracked_input) = &mut self.tracked_input {
                    tracked_input.push_str(input);
                }
            }
            _ => self.tracked_input = None,
        }
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    pub fn history_status(&self) -> HistoryStatus {
        self.active_adb
            .as_ref()
            .map(|adb| adb.history_status)
            .unwrap_or(self.host_history_status)
    }

    pub fn mark_history_loading(&mut self) -> HistoryLoadRequest {
        self.history_load_generation = self.history_load_generation.wrapping_add(1).max(1);
        let request = HistoryLoadRequest {
            generation: self.history_load_generation,
            session: self.session.clone(),
        };
        self.pending_history_load = Some(request.clone());
        self.host_history_status = HistoryStatus::Loading;
        request
    }

    pub fn cancel_history_load(&mut self) {
        self.pending_history_load = None;
        self.host_history_status = HistoryStatus::Disabled {
            reason: HISTORY_DISABLED_REASON,
        };
    }

    pub fn apply_history_result<E>(
        &mut self,
        request: &HistoryLoadRequest,
        result: Result<Vec<String>, E>,
    ) -> bool {
        if self.pending_history_load.as_ref() != Some(request) || request.session != self.session {
            return false;
        }
        self.pending_history_load = None;
        match result {
            Ok(history) => {
                self.host_loaded_history = normalize_history(history);
                self.rebuild_history();
                self.host_history_status = HistoryStatus::Ready {
                    items: self.host_history_len(),
                };
            }
            Err(_) => {
                self.host_loaded_history.clear();
                self.rebuild_history();
                self.host_history_status = HistoryStatus::Error {
                    reason: HISTORY_ERROR_REASON,
                };
            }
        }
        true
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
        self.host_loaded_history = normalize_history(history);
        self.rebuild_history();
        self.update_ready_history_items();
    }

    pub fn merge_executed(&mut self, command: &str) {
        merge_recent(&mut self.host_session_history, command);
        self.rebuild_history();
        self.update_ready_history_items();
    }

    pub fn activate_adb_history(&mut self, identity: AdbHistoryIdentity) -> bool {
        if self.foreground != ForegroundCompletion::AdbShell {
            return false;
        }
        self.next_adb_epoch = self.next_adb_epoch.wrapping_add(1).max(1);
        self.active_adb = Some(AdbCompletionContext {
            identity,
            epoch: self.next_adb_epoch,
            loaded_history: Vec::new(),
            session_history: Vec::new(),
            history_status: HistoryStatus::Disabled {
                reason: HISTORY_DISABLED_REASON,
            },
            load_generation: 0,
            pending_load: None,
        });
        self.rebuild_history();
        true
    }

    pub fn active_adb_identity(&self) -> Option<&AdbHistoryIdentity> {
        self.active_adb.as_ref().map(|adb| &adb.identity)
    }

    pub fn adb_submission_identity(&self) -> Option<&AdbHistoryIdentity> {
        (self.foreground == ForegroundCompletion::AdbShell)
            .then(|| self.active_adb_identity())
            .flatten()
    }

    pub fn mark_adb_history_loading(&mut self) -> Option<AdbHistoryLoadRequest> {
        let adb = self.active_adb.as_mut()?;
        adb.load_generation = adb.load_generation.wrapping_add(1).max(1);
        let request = AdbHistoryLoadRequest {
            generation: adb.load_generation,
            adb_epoch: adb.epoch,
            session: self.session.clone(),
            identity: adb.identity.clone(),
        };
        adb.pending_load = Some(request.clone());
        adb.history_status = HistoryStatus::Loading;
        Some(request)
    }

    pub fn apply_adb_history_result<E>(
        &mut self,
        request: &AdbHistoryLoadRequest,
        result: Result<Vec<String>, E>,
    ) -> bool {
        if request.session != self.session {
            return false;
        }
        let Some(adb) = self.active_adb.as_mut() else {
            return false;
        };
        if adb.pending_load.as_ref() != Some(request)
            || adb.epoch != request.adb_epoch
            || adb.identity != request.identity
        {
            return false;
        }
        adb.pending_load = None;
        match result {
            Ok(history) => {
                adb.loaded_history = normalize_history(history);
                adb.history_status = HistoryStatus::Ready {
                    items: merged_history([
                        adb.session_history.as_slice(),
                        adb.loaded_history.as_slice(),
                    ])
                    .len(),
                };
            }
            Err(_) => {
                adb.loaded_history.clear();
                adb.history_status = HistoryStatus::Error {
                    reason: HISTORY_ERROR_REASON,
                };
            }
        }
        self.rebuild_history();
        true
    }

    pub fn refresh(&mut self, prefix: &str) -> bool {
        if self.candidate_cache_key.as_ref().is_some_and(|cache_key| {
            cache_key.session == self.session
                && cache_key.history_generation == self.history_generation
                && cache_key.prefix == prefix
        }) {
            return false;
        }
        let cache_key = CandidateCacheKey {
            session: self.session.clone(),
            history_generation: self.history_generation,
            prefix: prefix.to_owned(),
        };
        #[cfg(test)]
        {
            self.candidate_cache_key_build_count += 1;
        }
        self.candidates = rank_candidates(&self.history, prefix);
        self.candidate_cache_key = Some(cache_key);
        self.selected = self.selected.min(self.candidates.len().saturating_sub(1));
        true
    }

    pub fn clear_candidates(&mut self) {
        self.candidates.clear();
        self.candidate_cache_key = None;
        self.selected = 0;
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

    pub fn on_user_edit(&mut self) -> Option<u64> {
        self.suppressed = false;
        self.pending_fill
            .take()
            .map(|pending_fill| pending_fill.request_id)
    }

    pub fn begin_fill(&mut self, request_id: u64, candidate: &str) -> bool {
        if self.pending_fill.is_some() {
            return false;
        }
        self.pending_fill = Some(PendingFill {
            request_id,
            session: self.session.clone(),
            candidate: candidate.to_owned(),
        });
        true
    }

    pub fn pending_fill_matches(&self, request_id: u64) -> bool {
        self.pending_fill.as_ref().is_some_and(|pending_fill| {
            pending_fill.request_id == request_id && pending_fill.session == self.session
        })
    }

    pub fn finish_fill(&mut self, request_id: u64) -> bool {
        if !self.pending_fill_matches(request_id) {
            return false;
        }
        let pending_fill = self.pending_fill.take().unwrap();
        self.tracked_input = Some(pending_fill.candidate);
        self.suppressed = true;
        self.clear_candidates();
        true
    }

    pub fn fail_fill(&mut self, request_id: u64) {
        if self.pending_fill_matches(request_id) {
            self.pending_fill = None;
        }
    }

    pub fn cancel_pending_fill(&mut self) {
        self.pending_fill = None;
    }

    pub fn complete_submission(&mut self, fallback: Option<&str>) -> Option<String> {
        let submission = self.current_input(fallback);
        if let Some(command) = submission.as_deref().filter(|command| !command.is_empty()) {
            match self.foreground {
                ForegroundCompletion::IntegratedBash => self.merge_executed(command),
                ForegroundCompletion::AdbShell => {
                    if let Some(adb) = self.active_adb.as_mut() {
                        merge_recent(&mut adb.session_history, command);
                        if matches!(adb.history_status, HistoryStatus::Ready { .. }) {
                            adb.history_status = HistoryStatus::Ready {
                                items: merged_history([
                                    adb.session_history.as_slice(),
                                    adb.loaded_history.as_slice(),
                                ])
                                .len(),
                            };
                        }
                        self.rebuild_history();
                    }
                }
                ForegroundCompletion::FishInAdb | ForegroundCompletion::AwaitingBashPrompt => {}
            }
        }
        self.clear_candidates();
        self.pending_fill = None;
        self.suppressed = false;
        self.tracked_input = Some(String::new());
        submission
    }

    pub fn reset_session(&mut self, session: CompletionSessionKey) {
        *self = Self::new(session);
    }

    fn invalidate_history_snapshot(&mut self) {
        self.history_generation = self.history_generation.wrapping_add(1);
        self.candidate_cache_key = None;
    }

    fn update_ready_history_items(&mut self) {
        if matches!(self.host_history_status, HistoryStatus::Ready { .. }) {
            self.host_history_status = HistoryStatus::Ready {
                items: self.host_history_len(),
            };
        }
    }

    fn host_history_len(&self) -> usize {
        merged_history([
            self.host_session_history.as_slice(),
            self.host_loaded_history.as_slice(),
        ])
        .len()
    }

    fn leave_adb_history(&mut self) {
        if self.active_adb.take().is_some() {
            self.rebuild_history();
        }
    }

    fn rebuild_history(&mut self) {
        let history = if let Some(adb) = &self.active_adb {
            merged_history([
                adb.session_history.as_slice(),
                adb.loaded_history.as_slice(),
                self.host_session_history.as_slice(),
                self.host_loaded_history.as_slice(),
            ])
        } else {
            merged_history([
                self.host_session_history.as_slice(),
                self.host_loaded_history.as_slice(),
            ])
        };
        if self.history != history {
            self.history = history;
            self.invalidate_history_snapshot();
        }
    }

    #[cfg(test)]
    fn candidate_cache_key_build_count(&self) -> usize {
        self.candidate_cache_key_build_count
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

fn simple_command_words(command: &str) -> Option<Vec<&str>> {
    let command = command.trim();
    if command.is_empty()
        || command
            .chars()
            .any(|character| matches!(character, ';' | '|' | '&' | '<' | '>' | '`' | '\n' | '\r'))
        || command.contains("$(")
        || command.contains(['\'', '"'])
    {
        return None;
    }
    Some(command.split_whitespace().collect())
}

fn command_basename(command: &str) -> &str {
    command.rsplit('/').next().unwrap_or(command)
}

fn interactive_adb_serial<'a>(words: &'a [&str]) -> Option<&'a str> {
    if words.len() < 4 || command_basename(words[0]) != "adb" || words.last() != Some(&"shell") {
        return None;
    }
    let mut serial = None;
    let mut index = 1;
    while index + 1 < words.len() {
        if words[index] == "-s" {
            let candidate = words.get(index + 1)?;
            if serial.is_some()
                || candidate.is_empty()
                || candidate.starts_with('-')
                || candidate.chars().any(char::is_control)
            {
                return None;
            }
            serial = Some(*candidate);
            index += 2;
        } else {
            return None;
        }
    }
    serial
}

fn is_interactive_fish(words: &[&str]) -> bool {
    command_basename(words[0]) == "fish"
        && words[1..].iter().all(|argument| {
            matches!(
                *argument,
                "-i" | "--interactive" | "-l" | "--login" | "-N" | "--no-config"
            )
        })
}

fn is_shell_exit(words: &[&str]) -> bool {
    match words {
        ["exit" | "logout"] => true,
        ["exit", status] => status.parse::<u8>().is_ok(),
        _ => false,
    }
}

fn normalize_history(entries: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .filter(|command| safe_command(command).is_some())
        .filter(|command| seen.insert(command.clone()))
        .take(MAX_HISTORY_ITEMS)
        .collect()
}

fn merged_history<const N: usize>(layers: [&[String]; N]) -> Vec<String> {
    normalize_history(layers.into_iter().flat_map(|layer| layer.iter().cloned()))
}

fn merge_recent(entries: &mut Vec<String>, command: &str) {
    if safe_command(command).is_none() {
        return;
    }
    entries.retain(|entry| entry != command);
    entries.insert(0, command.to_owned());
    entries.truncate(MAX_HISTORY_ITEMS);
}

#[cfg(test)]
#[path = "smart_completion/tests.rs"]
mod tests;
