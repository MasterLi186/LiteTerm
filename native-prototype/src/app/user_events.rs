use super::*;

impl App {
    pub(super) fn handle_user_event(&mut self, event: UserEvent) {
        match event {
            UserEvent::Api(call) => {
                if !dispatch_current_api_user_event(call, |call| self.handle_api_call(call)) {
                    return;
                }
            }
            UserEvent::Redraw => {
                let now = Instant::now();
                if prepare_completion_redraw(
                    &mut self.completion_popup_snapshot,
                    &mut self.completion_popup_epoch,
                    self.last_render_time,
                    now,
                ) == CompletionRedrawSchedule::RequestRedraw
                {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                    return;
                }
            }
            UserEvent::Monitor(event) => {
                let _ = apply_monitor_event_and_update_sidebar(
                    &mut self.monitor_slots,
                    &mut self.sidebar,
                    event,
                    &self.remote_monitor_generations,
                );
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            UserEvent::ProcessDetail {
                key,
                generation,
                requester,
                request_id,
                result,
            } => {
                let applied = apply_process_detail_event(
                    &mut self.process_managers,
                    &self.remote_monitor_generations,
                    &key,
                    generation,
                    &requester,
                    request_id,
                    result,
                );
                if applied {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
            UserEvent::NetworkDetail {
                key,
                generation,
                requester,
                request_id,
                result,
            } => {
                if monitor_event_is_current(&key, generation, &self.remote_monitor_generations) {
                    if let Some(state) = self.network_details.get_mut(&requester) {
                        if state.target() == &key {
                            let _ = state.apply_snapshot(request_id, result.map(|value| *value));
                        }
                    }
                }
            }
            UserEvent::RecordingLoaded { path, result } => match result {
                Ok(cast) => {
                    self.prepare_for_active_tab_change();
                    let playback = recording::PlaybackState::new(cast);
                    let (cols, rows) = playback.dimensions();
                    let (tab_id, _) = self.tab_manager.new_recording(path, cols, rows);
                    self.recording_playbacks.insert(tab_id, playback);
                    self.refresh_pane_layout();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                Err(error) => {
                    self.terminal_notice = Some(error);
                }
            },
            UserEvent::SerialPorts { generation, result } => {
                let _ = self.new_tab_selector.apply_serial_scan(generation, result);
            }
            UserEvent::SerialReady {
                tab_id,
                pane_id,
                generation,
                result,
            } => match result {
                Ok(handle) => {
                    let (cols, rows) = self.grid_size();
                    if let Some(terminal) = self
                        .tab_manager
                        .apply_serial(&tab_id, &pane_id, generation, handle, cols, rows)
                    {
                        if let Some(session) = self
                            .tab_manager
                            .find_by_id(&tab_id)
                            .and_then(|index| self.tab_manager.tabs[index].pane(&pane_id))
                            .map(|pane| pane.completion.session().clone())
                        {
                            self.start_read_loop(tab_id, pane_id, session, terminal);
                        }
                    }
                }
                Err(error) => {
                    log::warn!("串口打开失败: {error}");
                    if self
                        .tab_manager
                        .serial_failed(&tab_id, &pane_id, generation, &error)
                    {
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
                }
            },
            UserEvent::CompletionHistory {
                tab_id,
                pane_id,
                session,
                request,
                path,
                result,
            } => {
                if apply_completion_history_event(
                    &mut self.tab_manager,
                    &tab_id,
                    &pane_id,
                    &session,
                    &request,
                    &path,
                    result,
                ) {
                    log_completion_history_status(&self.tab_manager, &tab_id, &pane_id);
                }
            }
            UserEvent::AdbCompletionHistory {
                tab_id,
                pane_id,
                session,
                request,
                result,
            } => {
                if apply_adb_completion_history_event(
                    &mut self.tab_manager,
                    &tab_id,
                    &pane_id,
                    &session,
                    &request,
                    result,
                ) {
                    log_completion_history_status(&self.tab_manager, &tab_id, &pane_id);
                }
            }
            UserEvent::TerminalIntegration {
                tab_id,
                pane_id,
                session: event_session,
                event,
            } => match event {
                terminal::IntegrationEvent::HistoryPath { session, path } => {
                    if session != event_session {
                        return;
                    }
                    let history_request = apply_history_path_event(
                        &mut self.tab_manager,
                        &tab_id,
                        &pane_id,
                        &session,
                        path,
                    );
                    if let Some((session, path)) = history_request {
                        let is_ssh = self.tab_manager.find_by_id(&tab_id).is_some_and(|index| {
                            matches!(self.tab_manager.tabs[index].tab_type, TabType::Ssh { .. })
                        });
                        if is_ssh {
                            self.maybe_request_remote_history(&tab_id, &pane_id);
                        } else {
                            self.request_local_history(tab_id, pane_id, session, path);
                        }
                    }
                }
            },
            UserEvent::CompletionCandidateWritten {
                tab_id,
                pane_id,
                session,
                request_id,
                result,
            } => {
                self.handle_completion_candidate_written(
                    &tab_id, &pane_id, &session, request_id, result,
                );
            }
            UserEvent::Zmodem {
                tab_id,
                pane_id,
                session,
                event,
            } => {
                self.handle_zmodem_event(tab_id, pane_id, session, event);
            }
            UserEvent::SshReady {
                tab_id,
                pane_id,
                session,
                result,
            } => {
                if !self
                    .tab_manager
                    .ssh_attempt_is_current(&tab_id, &pane_id, &session)
                {
                    reject_stale_ssh_result(result);
                    return;
                }
                let keyring_allowed = api_tab_allows_keyring(&self.api_ephemeral_tabs, &tab_id);
                match result {
                    Ok(handle) => {
                        log::debug!("[SSH] 连接成功: {}", tab_id);
                        // 连接成功后保存密码到 keyring
                        if keyring_allowed {
                            if let Some(tab) = self.tab_manager.tabs.iter().find(|t| t.id == tab_id)
                            {
                                if let TabType::Ssh { params, .. } = &tab.tab_type {
                                    if !params.password.is_empty() {
                                        let entry = crate::keyring::KeyringEntry::new(
                                            &params.user,
                                            &params.host,
                                            params.port,
                                        );
                                        if let Err(e) = entry.store_password(&params.password) {
                                            log::warn!("[KEYRING] 保存密码失败: {}", e);
                                        }
                                    }
                                }
                            }
                        }
                        let sftp_params = self.tab_manager.tabs.iter().find_map(|tab| {
                            if tab.id != tab_id {
                                return None;
                            }
                            match &tab.tab_type {
                                TabType::Ssh { params, .. } => Some(params.clone()),
                                TabType::Local { .. }
                                | TabType::Process { .. }
                                | TabType::Network { .. }
                                | TabType::Serial { .. }
                                | TabType::Recording { .. }
                                | TabType::Settings => None,
                            }
                        });
                        let (cols, rows) = self.grid_size_for_tab_pane(&tab_id, &pane_id);
                        let applied = if let Some(terminal) = self
                            .tab_manager
                            .apply_ssh_pane(&tab_id, &pane_id, &session, handle, cols, rows)
                        {
                            self.start_read_loop(
                                tab_id.clone(),
                                pane_id.clone(),
                                session.clone(),
                                terminal,
                            );
                            true
                        } else {
                            false
                        };
                        if applied {
                            self.reconcile_remote_monitors();
                        }
                        if let (true, Some(params)) = (applied, sftp_params) {
                            let local_path = settings::file_browser_local_directory(
                                &self.settings.transfer.default_download_dir,
                            )
                            .to_string_lossy()
                            .into_owned();
                            let browser = self
                                .file_browsers
                                .entry(tab_id.clone())
                                .or_insert_with(|| file_browser::FileBrowserState::new(local_path));
                            // A successful SSH connection establishes a new session,
                            // including reconnect, restore, duplicate and API-open flows.
                            browser.open = false;
                            let worker = sftp::start_worker_for_pane(
                                tab_id.clone(),
                                pane_id.clone(),
                                session.clone(),
                                params,
                                self.proxy.clone(),
                            );
                            if let Some(previous) =
                                self.sftp_workers.insert(pane_id.clone(), worker)
                            {
                                let _ = previous.send(sftp::SftpCommand::Shutdown);
                            }
                            self.request_listing_for_pane(
                                &tab_id,
                                &pane_id,
                                sftp::FileSide::Local,
                                None,
                            );
                        }
                    }
                    Err(e) => {
                        log::warn!("[SSH] 连接失败: {}: {}", tab_id, e);
                        if !self.tab_manager.ssh_failed(&tab_id, &pane_id, &session, &e) {
                            return;
                        }
                        if keyring_allowed && e.contains("认证失败") {
                            if let Some(tab) = self.tab_manager.tabs.iter().find(|t| t.id == tab_id)
                            {
                                if let TabType::Ssh { label, params } = &tab.tab_type {
                                    self.sidebar.password_prompt = Some(sidebar::SshConnection {
                                        label: label.clone(),
                                        host: params.host.clone(),
                                        port: params.port,
                                        user: params.user.clone(),
                                        auth: "password".to_string(),
                                        key_path: String::new(),
                                        password: String::new(),
                                        group: String::new(),
                                        group_color: [0, 0, 0],
                                    });
                                    self.sidebar.password_error = e.clone();
                                    self.sidebar.password_input.clear();
                                    // 清空残留的连接请求，防止重复触发
                                    self.sidebar.on_connect = None;
                                }
                            }
                        }
                    }
                }
            }
            UserEvent::Sftp(worker_event) => {
                // A stale worker event can be rejected below after a tab closes
                // or reconnects. Retire the globally unique drag transfer ID
                // before that identity gate so interrupted uploads cannot leak
                // bookkeeping entries for the rest of the application lifetime.
                let tracked_drag_transfer = retire_drag_upload_transfer(
                    &mut self.drag_upload_transfer_ids,
                    &worker_event.event,
                );
                let Some((tab_id, pane_id, event)) = take_current_sftp_worker_event(
                    &self.tab_manager,
                    &self.sftp_workers,
                    worker_event,
                ) else {
                    return;
                };

                match event {
                    sftp::SftpEvent::CompletionHistoryRead {
                        session,
                        request,
                        path,
                        result,
                        ..
                    } => {
                        if apply_completion_history_event(
                            &mut self.tab_manager,
                            &tab_id,
                            &pane_id,
                            &session,
                            &request,
                            std::path::Path::new(&path),
                            result,
                        ) {
                            log_completion_history_status(&self.tab_manager, &tab_id, &pane_id);
                        }
                    }
                    sftp::SftpEvent::CompletionCandidateWritten {
                        session,
                        request_id,
                        result,
                        ..
                    } => {
                        self.handle_completion_candidate_written(
                            &tab_id, &pane_id, &session, request_id, result,
                        );
                    }
                    event => {
                        let drag_upload_failure = match &event {
                            sftp::SftpEvent::TransferFinished { result, .. }
                                if tracked_drag_transfer =>
                            {
                                result
                                    .as_ref()
                                    .err()
                                    .map(|error| bounded_notice("拖拽上传失败：", error))
                            }
                            _ => None,
                        };
                        match &event {
                            sftp::SftpEvent::Ready { home, .. } => {
                                if let Some(index) = self.tab_manager.find_by_id(&tab_id) {
                                    if let Some(pane) =
                                        self.tab_manager.tabs[index].pane_mut(&pane_id)
                                    {
                                        mark_sftp_ready(&mut pane.completion, home);
                                    }
                                }
                                self.maybe_request_remote_history(&tab_id, &pane_id);
                            }
                            sftp::SftpEvent::Failed { .. } => {
                                if let Some(index) = self.tab_manager.find_by_id(&tab_id) {
                                    if let Some(pane) =
                                        self.tab_manager.tabs[index].pane_mut(&pane_id)
                                    {
                                        pane.completion.set_sftp_ready(false);
                                    }
                                }
                            }
                            _ => {}
                        }
                        if self.file_browsers.contains_key(&tab_id) {
                            let refresh_side = refresh_side_for_event(&event);
                            if let Some(state) = self.file_browsers.get_mut(&tab_id) {
                                state.apply_event(&event);
                            }
                            if let Some(side) = refresh_side {
                                self.request_listing_for_pane(&tab_id, &pane_id, side, None);
                            }
                        }
                        if let Some(message) = drag_upload_failure {
                            self.terminal_notice = Some(message);
                        }
                    }
                }
            }
        }
        self.do_render();
        self.check_ssh_connect();
    }
}
