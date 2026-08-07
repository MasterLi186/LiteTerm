use super::*;

pub(super) fn completion_event_is_current(
    current: &CompletionSessionKey,
    session: &CompletionSessionKey,
) -> bool {
    current == session
}

pub(super) fn completion_fill_may_commit(
    state: &smart_completion::CompletionState,
    current_session: &CompletionSessionKey,
    event_session: &CompletionSessionKey,
    request_id: u64,
    result: &Result<(), String>,
) -> bool {
    result.is_ok() && current_session == event_session && state.pending_fill_matches(request_id)
}

pub(super) fn mark_sftp_ready(completion: &mut smart_completion::CompletionState, home: &str) {
    completion.set_sftp_ready(true);
    if completion.history_path().is_some()
        || !std::path::Path::new(home).is_absolute()
        || home.chars().any(char::is_control)
    {
        return;
    }
    completion.set_history_path(
        std::path::Path::new(home)
            .join(".bash_history")
            .to_string_lossy()
            .into_owned(),
    );
}

pub(super) fn remote_history_request(
    is_ssh: bool,
    completion: &smart_completion::CompletionState,
    worker_exists: bool,
) -> Option<(CompletionSessionKey, String)> {
    (is_ssh && worker_exists && completion.sftp_ready())
        .then(|| {
            completion
                .history_path()
                .map(|path| (completion.session().clone(), path.to_string()))
        })
        .flatten()
}

pub(super) fn take_current_sftp_worker_event(
    tab_manager: &TabManager,
    workers: &HashMap<String, sftp::SftpHandle>,
    worker_event: sftp::SftpWorkerEvent,
) -> Option<(String, String, sftp::SftpEvent)> {
    let tab_id = worker_event.event.tab_id().to_string();
    if worker_event.tab_id != tab_id {
        return None;
    }
    let pane_id = worker_event.pane_id.clone();
    let tab_index = tab_manager.find_by_id(&tab_id)?;
    let pane = tab_manager.tabs[tab_index].pane(&pane_id)?;
    let worker = workers.get(&pane_id)?;
    if worker.id() != worker_event.worker_id
        || worker.tab_id() != tab_id
        || worker.pane_id() != pane_id
        || worker.session() != &worker_event.session
        || pane.completion.session() != &worker_event.session
        || worker_event
            .event
            .completion_session()
            .is_some_and(|session| session != &worker_event.session)
    {
        return None;
    }
    Some((tab_id, pane_id, worker_event.event))
}

pub(super) fn cancel_pending_fill_for_tab(tab_manager: &mut TabManager, index: usize) {
    if let Some(tab) = tab_manager.tabs.get_mut(index) {
        tab.completion.cancel_pending_fill();
    }
}

pub(super) fn prepare_sftp_reconnect_state(completion: &mut smart_completion::CompletionState) {
    completion.set_sftp_ready(false);
    completion.cancel_pending_fill();
    completion.cancel_history_load();
}

pub(super) fn advance_completion_request_id(counter: &mut u64) -> u64 {
    *counter = counter.wrapping_add(1).max(1);
    *counter
}

pub(super) fn reject_stale_ssh_result(result: Result<ssh::SshHandle, String>) {
    if let Ok(handle) = result {
        handle.shutdown();
    }
}

pub(super) fn cleanup_undelivered_user_event(event: UserEvent) {
    match event {
        UserEvent::Api(call) => {
            let _ = call.respond(Err(api::ApiError::unavailable("主线程事件队列已关闭")));
        }
        UserEvent::SshReady { result, .. } => reject_stale_ssh_result(result),
        UserEvent::SerialReady {
            result: Ok(handle), ..
        } => terminal::shutdown_serial_handle(handle),
        _ => {}
    }
}

pub(super) fn apply_completion_history_event(
    tab_manager: &mut TabManager,
    tab_id: &str,
    pane_id: &str,
    session: &CompletionSessionKey,
    request: &HistoryLoadRequest,
    requested_path: &std::path::Path,
    result: Result<Vec<u8>, String>,
) -> bool {
    let Some(index) = tab_manager.find_by_id(tab_id) else {
        return false;
    };
    let Some(pane) = tab_manager.tabs[index].pane_mut(pane_id) else {
        return false;
    };
    let completion = &mut pane.completion;
    if !completion_event_is_current(completion.session(), session)
        || completion.history_path().map(std::path::Path::new) != Some(requested_path)
    {
        return false;
    }

    completion.apply_history_result(
        request,
        result.map(|bytes| smart_completion::parse_bash_history(&bytes)),
    )
}

pub(super) fn apply_adb_completion_history_event(
    tab_manager: &mut TabManager,
    tab_id: &str,
    pane_id: &str,
    session: &CompletionSessionKey,
    request: &AdbHistoryLoadRequest,
    result: Result<Vec<String>, String>,
) -> bool {
    let Some(index) = tab_manager.find_by_id(tab_id) else {
        return false;
    };
    let Some(pane) = tab_manager.tabs[index].pane_mut(pane_id) else {
        return false;
    };
    let completion = &mut pane.completion;
    if !completion_event_is_current(completion.session(), session) {
        return false;
    }
    completion.apply_adb_history_result(request, result)
}

pub(super) fn adb_history_scope(tab_type: &TabType) -> Option<adb_history::HostScope> {
    match tab_type {
        TabType::Local { .. } => Some(adb_history::HostScope::Local),
        TabType::Ssh { params, .. } => Some(adb_history::HostScope::Ssh {
            user: params.user.clone(),
            host: params.host.clone(),
            port: params.port,
        }),
        TabType::Process { .. }
        | TabType::Network { .. }
        | TabType::Serial { .. }
        | TabType::Recording { .. }
        | TabType::Settings => None,
    }
}

pub(super) fn completion_history_status_diagnostic(
    status: smart_completion::HistoryStatus,
) -> String {
    match status {
        smart_completion::HistoryStatus::Disabled { .. } => "补全历史状态：已禁用".into(),
        smart_completion::HistoryStatus::Loading => "补全历史状态：加载中".into(),
        smart_completion::HistoryStatus::Ready { items } => {
            format!("补全历史状态：就绪（{items} 条）")
        }
        smart_completion::HistoryStatus::Error { .. } => "补全历史状态：加载失败".into(),
    }
}

pub(super) fn log_completion_history_status(tab_manager: &TabManager, tab_id: &str, pane_id: &str) {
    if let Some(index) = tab_manager.find_by_id(tab_id) {
        if let Some(pane) = tab_manager.tabs[index].pane(pane_id) {
            log::debug!(
                "{}",
                completion_history_status_diagnostic(pane.completion.history_status())
            );
        }
    }
}

pub(super) fn apply_history_path_event(
    tab_manager: &mut TabManager,
    tab_id: &str,
    pane_id: &str,
    session: &CompletionSessionKey,
    path: String,
) -> Option<(CompletionSessionKey, std::path::PathBuf)> {
    let index = tab_manager.find_by_id(tab_id)?;
    let tab = &mut tab_manager.tabs[index];
    if path.chars().any(char::is_control) {
        return None;
    }
    let is_local = matches!(&tab.tab_type, TabType::Local { .. });
    let path_buf = if is_local {
        crate::bash_integration::local_history_path(&path)?
    } else {
        let path_buf = std::path::PathBuf::from(&path);
        path_buf.is_absolute().then_some(path_buf)?
    };
    let path_string = if is_local {
        path_buf.to_string_lossy().into_owned()
    } else {
        path
    };
    let completion = &mut tab.pane_mut(pane_id)?.completion;
    if !completion_event_is_current(completion.session(), session)
        || completion.history_path() == Some(path_string.as_str())
    {
        return None;
    }

    completion.replace_history(Vec::new());
    completion.set_history_path(path_string);
    Some((completion.session().clone(), path_buf))
}

pub(super) fn refresh_active_completion(tab_manager: &mut TabManager, input: Option<&str>) {
    let Some(tab) = tab_manager.tabs.get_mut(tab_manager.active_idx) else {
        return;
    };
    match input {
        Some(prefix) if !prefix.is_empty() => {
            tab.completion.refresh(prefix);
        }
        _ => tab.completion.clear_candidates(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CompletionKeyAction {
    Previous,
    Next,
    Accept,
    Dismiss,
    PassThrough,
}

pub(super) fn completion_key_action(
    key: &Key,
    modifiers: winit::keyboard::ModifiersState,
    popup_visible: bool,
    _fill_pending: bool,
    egui_wants_keyboard: bool,
    has_dialog: bool,
    search_field_owns_focus: bool,
) -> CompletionKeyAction {
    if modifiers.control_key()
        || modifiers.alt_key()
        || modifiers.super_key()
        || egui_wants_keyboard
        || has_dialog
        || search_field_owns_focus
    {
        return CompletionKeyAction::PassThrough;
    }
    if !popup_visible {
        return CompletionKeyAction::PassThrough;
    }
    match key {
        Key::Named(NamedKey::ArrowUp) => CompletionKeyAction::Previous,
        Key::Named(NamedKey::ArrowDown) => CompletionKeyAction::Next,
        Key::Named(NamedKey::Tab) if !modifiers.shift_key() => CompletionKeyAction::Accept,
        Key::Named(NamedKey::Escape) => CompletionKeyAction::Dismiss,
        _ => CompletionKeyAction::PassThrough,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CompletionUserInputEffect {
    ExactTrackedEdit,
    RecoverableReadlineEdit,
    InvalidatePrompt,
}

pub(super) fn completion_user_input_effect(input: &str) -> CompletionUserInputEffect {
    const RECOVERABLE_READLINE_SEQUENCES: &[&str] = &[
        "\t", "\x01", "\x02", "\x05", "\x06", "\x0b", "\x0e", "\x10", "\x14", "\x17", "\x19",
        "\x1b\x7f", "\x1b[A", "\x1b[B", "\x1b[C", "\x1b[D", "\x1b[H", "\x1b[F", "\x1b[3~",
    ];
    if !input.is_empty() && !input.chars().any(char::is_control) {
        return CompletionUserInputEffect::ExactTrackedEdit;
    }
    if matches!(input, "\x7f" | "\x08" | "\x15") {
        CompletionUserInputEffect::ExactTrackedEdit
    } else if RECOVERABLE_READLINE_SEQUENCES.contains(&input) {
        CompletionUserInputEffect::RecoverableReadlineEdit
    } else {
        CompletionUserInputEffect::InvalidatePrompt
    }
}

pub(super) fn apply_completion_user_input_state(
    completion: &mut smart_completion::CompletionState,
    popup_snapshot: &mut Option<completion_popup::CompletionPopupSnapshot>,
    input: &str,
) -> CompletionUserInputEffect {
    *popup_snapshot = None;
    let effect = completion_user_input_effect(input);
    completion.on_user_edit();
    completion.track_user_input(input);
    match effect {
        CompletionUserInputEffect::ExactTrackedEdit => {}
        CompletionUserInputEffect::RecoverableReadlineEdit
        | CompletionUserInputEffect::InvalidatePrompt => {
            completion.clear_candidates();
        }
    }
    effect
}

pub(super) fn apply_completion_prompt_effect(
    terminal: &mut TerminalState,
    effect: CompletionUserInputEffect,
) {
    match effect {
        CompletionUserInputEffect::ExactTrackedEdit => {}
        CompletionUserInputEffect::RecoverableReadlineEdit => {
            terminal.invalidate_readline_geometry();
        }
        CompletionUserInputEffect::InvalidatePrompt => terminal.invalidate_prompt(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CtrlTerminalInputAction {
    Submit,
    Write(char),
}

pub(super) fn ctrl_terminal_input_action(ctrl_byte: u8) -> CtrlTerminalInputAction {
    if matches!(ctrl_byte, b'\n' | b'\r') {
        CtrlTerminalInputAction::Submit
    } else {
        CtrlTerminalInputAction::Write(char::from(ctrl_byte))
    }
}

pub(super) fn completion_input_for_render(
    completion: &mut smart_completion::CompletionState,
    terminal: &mut TerminalState,
    now: Instant,
) -> Option<String> {
    if !terminal.completion_surface_safe() {
        return None;
    }
    completion.resume_surface_tracking();
    if terminal.has_authenticated_active_bash_prompt() {
        completion.observe_authenticated_prompt();
        return completion
            .tracked_input()
            .map(str::to_owned)
            .or_else(|| terminal.current_bash_input_or_request_snapshot(now));
    }

    // Git Bash on Windows can emit a prompt redraw without the OSC marker
    // being visible to ConPTY.  The completion state still receives every
    // real keyboard edit, so use that input as a narrowly scoped fallback for
    // local Bash only.  PowerShell/CMD never have a Bash runtime and remain
    // unsupported by the history completion feature.
    #[cfg(windows)]
    if terminal.has_local_bash_runtime() {
        return completion.tracked_input().map(str::to_owned);
    }

    if completion.completion_suspended_without_prompt() {
        None
    } else {
        completion.tracked_input().map(str::to_owned)
    }
}

#[cfg(test)]
pub(super) fn open_new_ssh_editor(
    new_tab_selector: &mut new_tab_selector::NewTabSelector,
    sidebar: &mut Sidebar,
) {
    new_tab_selector.close();
    sidebar.new_conn = sidebar::NewConnForm::default();
    sidebar.show_new_connection = true;
}

pub(super) fn current_completion_popup_snapshot<'a>(
    tab_manager: &TabManager,
    snapshot: &'a Option<completion_popup::CompletionPopupSnapshot>,
) -> Option<&'a completion_popup::CompletionPopupSnapshot> {
    let active = tab_manager.active()?;
    snapshot.as_ref().filter(|snapshot| {
        snapshot.tab_id == active.id
            && snapshot.pane_id == active.active_pane_id()
            && &snapshot.session == active.completion.session()
    })
}

pub(super) fn completion_snapshot_selection(
    snapshot: &completion_popup::CompletionPopupSnapshot,
) -> Option<(String, String, String)> {
    snapshot
        .candidates
        .get(snapshot.selected)
        .cloned()
        .map(|candidate| (snapshot.tab_id.clone(), snapshot.pane_id.clone(), candidate))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CompletionRedrawSchedule {
    RenderNow,
    RequestRedraw,
}

pub(super) fn prepare_completion_redraw(
    popup_snapshot: &mut Option<completion_popup::CompletionPopupSnapshot>,
    popup_epoch: &mut u64,
    last_render_time: Instant,
    now: Instant,
) -> CompletionRedrawSchedule {
    *popup_snapshot = None;
    *popup_epoch = popup_epoch.wrapping_add(1);
    if now.saturating_duration_since(last_render_time) >= std::time::Duration::from_millis(16) {
        CompletionRedrawSchedule::RenderNow
    } else {
        CompletionRedrawSchedule::RequestRedraw
    }
}

pub(super) fn publish_completion_popup_snapshot(
    stored: &mut Option<completion_popup::CompletionPopupSnapshot>,
    rendered: Option<completion_popup::CompletionPopupSnapshot>,
    frame_presented: bool,
    frame_epoch: u64,
    current_epoch: u64,
) {
    *stored = (frame_presented && frame_epoch == current_epoch)
        .then_some(rendered)
        .flatten();
}

pub(super) fn write_completion_invalidating_control_sequence(
    completion: &mut smart_completion::CompletionState,
    terminal: &mut TerminalState,
    sequence: &str,
) {
    completion.clear_candidates();
    terminal.invalidate_prompt();
    terminal.write_input(sequence);
}
