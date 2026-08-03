use super::*;

pub(crate) fn api_tab_kind(tab_type: &TabType) -> &'static str {
    match tab_type {
        TabType::Local { .. } => "local",
        TabType::Ssh { .. } => "ssh",
        TabType::Serial { .. } => "serial",
        TabType::Process { .. } => "process",
        TabType::Network { .. } => "network",
        TabType::Recording { .. } => "recording",
        TabType::Settings => "settings",
    }
}

pub(crate) fn api_tab_dto(tab: &Tab) -> api::TabDto {
    let active_pane_id = tab.active_pane_id().to_string();
    api::TabDto {
        id: tab.id.clone(),
        label: tab.label.clone(),
        kind: api_tab_kind(&tab.tab_type).into(),
        panes: tab
            .panes()
            .map(|pane| api::PaneDto {
                id: pane.id().to_string(),
                active: pane.id() == active_pane_id,
            })
            .collect(),
        active_pane_id: Some(active_pane_id),
    }
}

pub(crate) fn validate_api_text(
    value: &str,
    field: &'static str,
    max_bytes: usize,
) -> Result<(), api::ApiError> {
    if value.trim().is_empty() {
        return Err(api::ApiError::bad_request(format!("{field} 不能为空")));
    }
    if value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(api::ApiError::bad_request(format!("{field} 无效")));
    }
    Ok(())
}

pub(crate) fn validate_api_shell_path(shell: &str) -> Result<(), api::ApiError> {
    validate_api_text(shell, "shell_path", 4095)?;
    let path = std::path::Path::new(shell);
    if !path.is_absolute() {
        return Err(api::ApiError::bad_request("shell_path 必须是绝对路径"));
    }
    let metadata = std::fs::metadata(path)
        .map_err(|_| api::ApiError::bad_request("shell_path 不存在或不可访问"))?;
    if !metadata.is_file() {
        return Err(api::ApiError::bad_request("shell_path 不是文件"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(api::ApiError::bad_request("shell_path 不可执行"));
        }
    }
    Ok(())
}

pub(crate) fn api_internal(message: &'static str) -> api::ApiError {
    api::ApiError::new(
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        message,
    )
}

pub(crate) fn api_conflict(code: &'static str, message: &'static str) -> api::ApiError {
    api::ApiError::new(axum::http::StatusCode::CONFLICT, code, message)
}

pub(crate) fn api_too_many(message: &'static str) -> api::ApiError {
    api::ApiError::new(
        axum::http::StatusCode::TOO_MANY_REQUESTS,
        "write_queue_full",
        message,
    )
}

pub(crate) fn current_api_parts(
    call: api::MainThreadCall,
) -> Option<(api::ApiOperation, api::ApiResponseSender)> {
    if call.is_expired() {
        let _ = call.respond(Err(api::ApiError::timeout()));
        return None;
    }
    call.into_current_parts().ok()
}

pub(crate) fn dispatch_current_api_user_event(
    call: api::MainThreadCall,
    handle: impl FnOnce(api::MainThreadCall) -> bool,
) -> bool {
    if call.is_expired() {
        let _ = call.respond(Err(api::ApiError::timeout()));
        return false;
    }
    handle(call)
}

pub(crate) fn resolve_api_pane_in(
    tab_manager: &TabManager,
    tab_id: &str,
    pane_id: Option<&str>,
) -> Result<String, api::ApiError> {
    let index = tab_manager
        .find_by_id(tab_id)
        .ok_or_else(|| api::ApiError::not_found("标签不存在"))?;
    let tab = &tab_manager.tabs[index];
    match pane_id {
        Some(pane_id) if tab.pane(pane_id).is_some() => Ok(pane_id.to_string()),
        Some(_) => Err(api::ApiError::not_found("终端面板不存在")),
        None => Ok(tab.active_pane_id().to_string()),
    }
}

pub(crate) fn map_api_write_error(error: &str) -> api::ApiError {
    if error.contains("队列已满") {
        api_too_many("终端写入队列已满")
    } else if error.contains("ZMODEM") {
        api_conflict("zmodem_active", "ZMODEM 传输期间不能写入终端")
    } else {
        api::ApiError::not_found("终端写入通道不可用")
    }
}

pub(crate) fn api_tab_allows_keyring(ephemeral_tabs: &HashSet<String>, tab_id: &str) -> bool {
    !ephemeral_tabs.contains(tab_id)
}

pub(crate) fn propagate_api_tab_credential_scope(
    ephemeral_tabs: &mut HashSet<String>,
    source_tab_id: &str,
    new_tab_id: &str,
) {
    if ephemeral_tabs.contains(source_tab_id) {
        ephemeral_tabs.insert(new_tab_id.to_string());
    }
}

pub(crate) fn clear_api_tab_credential_scope(ephemeral_tabs: &mut HashSet<String>, tab_id: &str) {
    ephemeral_tabs.remove(tab_id);
}

impl App {
    pub(super) fn end_api_pane_stream(&mut self, pane_id: &str) {
        if let Some((tab_id, stream_id)) = self.api_streams.remove(pane_id) {
            let _ = self.api_outputs.end_stream(&tab_id, pane_id, stream_id);
        }
    }

    pub(super) fn end_api_tab_streams(&mut self, tab_id: &str) {
        let pane_ids = self
            .api_streams
            .iter()
            .filter(|(_, (owner, _))| owner == tab_id)
            .map(|(pane_id, _)| pane_id.clone())
            .collect::<Vec<_>>();
        for pane_id in pane_ids {
            self.end_api_pane_stream(&pane_id);
        }
        clear_api_tab_credential_scope(&mut self.api_ephemeral_tabs, tab_id);
    }

    pub(super) fn resolve_api_pane(
        &self,
        tab_id: &str,
        pane_id: Option<&str>,
    ) -> Result<String, api::ApiError> {
        resolve_api_pane_in(&self.tab_manager, tab_id, pane_id)
    }

    pub(super) fn handle_api_call(&mut self, call: api::MainThreadCall) -> bool {
        // This must remain the first operation: current_api_parts drops an
        // expired operation before any lookup or mutation can occur.
        let Some((operation, response)) = current_api_parts(call) else {
            return false;
        };
        let reply = match operation {
            api::ApiOperation::ListTabs => Ok(api::ApiReply::Tabs(
                self.tab_manager.tabs.iter().map(api_tab_dto).collect(),
            )),
            api::ApiOperation::OpenLocal(request) => {
                let shell = request
                    .shell_path
                    .unwrap_or_else(crate::terminal::default_shell_path);
                validate_api_shell_path(&shell).and_then(|()| {
                    if response.is_expired() {
                        return Err(api::ApiError::timeout());
                    }
                    self.prepare_for_active_tab_change();
                    let (cols, rows) = self.grid_size();
                    let (tab_id, terminal) = self
                        .tab_manager
                        .try_new_local(&shell, cols, rows)
                        .map_err(|_| api_internal("无法启动本地终端"))?;
                    let (session, history_path) = self
                        .tab_manager
                        .find_by_id(&tab_id)
                        .and_then(|index| self.tab_manager.tabs.get(index))
                        .map(|tab| {
                            (
                                tab.completion.session().clone(),
                                tab.completion.history_path().map(std::path::PathBuf::from),
                            )
                        })
                        .ok_or_else(|| api_internal("本地终端初始化失败"))?;
                    self.start_read_loop(tab_id.clone(), tab_id.clone(), session.clone(), terminal);
                    if let Some(path) = history_path {
                        self.request_local_history(tab_id.clone(), tab_id.clone(), session, path);
                    }
                    let dto = self
                        .tab_manager
                        .find_by_id(&tab_id)
                        .map(|index| api_tab_dto(&self.tab_manager.tabs[index]))
                        .ok_or_else(|| api_internal("本地终端初始化失败"))?;
                    Ok(api::ApiReply::Opened(dto))
                })
            }
            api::ApiOperation::OpenSsh(request) => {
                let auth = request.auth_method.as_deref().unwrap_or_else(|| {
                    if request.password.is_some() {
                        "password"
                    } else if request.key_path.is_some() {
                        "key"
                    } else {
                        "agent"
                    }
                });
                let validation = (|| {
                    validate_api_text(&request.host, "host", 255)?;
                    validate_api_text(&request.user, "user", 255)?;
                    if request.port == 0 {
                        return Err(api::ApiError::bad_request("port 无效"));
                    }
                    if !matches!(auth, "password" | "key" | "agent") {
                        return Err(api::ApiError::bad_request("auth_method 不受支持"));
                    }
                    if request
                        .proxy_jump
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty())
                    {
                        return Err(api::ApiError::bad_request("proxy_jump 暂不受支持"));
                    }
                    if let Some(password) = request.password.as_deref() {
                        if password.len() > 4096 || password.chars().any(char::is_control) {
                            return Err(api::ApiError::bad_request("password 无效"));
                        }
                    }
                    if let Some(key_path) = request.key_path.as_deref() {
                        validate_api_text(key_path, "key_path", 4095)?;
                    }
                    if auth == "password"
                        && request.password.as_deref().unwrap_or_default().is_empty()
                    {
                        return Err(api::ApiError::bad_request("password 不能为空"));
                    }
                    Ok(())
                })();
                validation.and_then(|()| {
                    if response.is_expired() {
                        return Err(api::ApiError::timeout());
                    }
                    let label = format!("{}@{}", request.user, request.host);
                    let conn = sidebar::SshConnection {
                        label,
                        host: request.host,
                        port: request.port,
                        user: request.user,
                        auth: auth.to_string(),
                        key_path: request.key_path.unwrap_or_default(),
                        password: request.password.unwrap_or_default(),
                        group: String::new(),
                        group_color: [0, 0, 0],
                    };
                    self.prepare_for_active_tab_change();
                    let tab_id = self.tab_manager.new_ssh_placeholder(&conn);
                    let params = ssh::ConnectionParams::from(&conn);
                    let session = self
                        .tab_manager
                        .find_by_id(&tab_id)
                        .map(|index| self.tab_manager.tabs[index].completion.session().clone())
                        .ok_or_else(|| api_internal("SSH 标签初始化失败"))?;
                    self.api_ephemeral_tabs.insert(tab_id.clone());
                    self.spawn_ssh_connect(tab_id.clone(), tab_id.clone(), params, session);
                    let dto = self
                        .tab_manager
                        .find_by_id(&tab_id)
                        .map(|index| api_tab_dto(&self.tab_manager.tabs[index]))
                        .ok_or_else(|| api_internal("SSH 标签初始化失败"))?;
                    Ok(api::ApiReply::Opened(dto))
                })
            }
            api::ApiOperation::Focus { tab_id, pane_id } => self
                .resolve_api_pane(&tab_id, pane_id.as_deref())
                .and_then(|pane_id| {
                    if response.is_expired() {
                        return Err(api::ApiError::timeout());
                    }
                    let index = self
                        .tab_manager
                        .find_by_id(&tab_id)
                        .ok_or_else(|| api::ApiError::not_found("标签不存在"))?;
                    self.switch_to_tab(index);
                    if !self.focus_pane(&pane_id) {
                        return Err(api::ApiError::not_found("终端面板不存在"));
                    }
                    self.refresh_pane_layout();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                    Ok(api::ApiReply::Focused { tab_id, pane_id })
                }),
            api::ApiOperation::Write {
                tab_id,
                pane_id,
                data,
            } => self
                .resolve_api_pane(&tab_id, pane_id.as_deref())
                .and_then(|pane_id| {
                    if response.is_expired() {
                        return Err(api::ApiError::timeout());
                    }
                    let index = self
                        .tab_manager
                        .find_by_id(&tab_id)
                        .ok_or_else(|| api::ApiError::not_found("标签不存在"))?;
                    if !self.tab_manager.tabs[index].tab_type.is_terminal() {
                        return Err(api::ApiError::not_found("标签没有终端面板"));
                    }
                    let pane = self.tab_manager.tabs[index]
                        .pane(&pane_id)
                        .ok_or_else(|| api::ApiError::not_found("终端面板不存在"))?;
                    match &pane.status {
                        tab_manager::PaneStatus::Connecting
                        | tab_manager::PaneStatus::Idle
                        | tab_manager::PaneStatus::Failed(_) => {
                            return Err(api::ApiError::not_found("终端写入通道尚未就绪"));
                        }
                        tab_manager::PaneStatus::Connected => {}
                    }
                    let text = std::str::from_utf8(&data)
                        .map_err(|_| api::ApiError::bad_request("data 必须是 UTF-8 文本"))?;
                    let mut terminal = pane
                        .terminal
                        .lock()
                        .map_err(|_| api_internal("终端状态不可用"))?;
                    if terminal.zmodem_active() {
                        return Err(api_conflict("zmodem_active", "ZMODEM 传输期间不能写入终端"));
                    }
                    terminal
                        .try_write_input(text)
                        .map_err(|error| map_api_write_error(&error))?;
                    Ok(api::ApiReply::Written { bytes: data.len() })
                }),
            api::ApiOperation::Close { tab_id, pane_id } => {
                let validation = self.resolve_api_pane(&tab_id, pane_id.as_deref());
                validation.and_then(|_| {
                    if response.is_expired() {
                        return Err(api::ApiError::timeout());
                    }
                    let index = self
                        .tab_manager
                        .find_by_id(&tab_id)
                        .ok_or_else(|| api::ApiError::not_found("标签不存在"))?;
                    self.close_tab(index);
                    Ok(api::ApiReply::Closed)
                })
            }
            api::ApiOperation::ResolvePane { tab_id } => {
                self.resolve_api_pane(&tab_id, None).and_then(|pane_id| {
                    let index = self
                        .tab_manager
                        .find_by_id(&tab_id)
                        .ok_or_else(|| api::ApiError::not_found("标签不存在"))?;
                    if !self.tab_manager.tabs[index].tab_type.is_terminal() {
                        return Err(api::ApiError::not_found("标签没有终端面板"));
                    }
                    Ok(api::ApiReply::PaneResolved { pane_id })
                })
            }
        };
        let _ = response.respond(reply);
        true
    }
}
