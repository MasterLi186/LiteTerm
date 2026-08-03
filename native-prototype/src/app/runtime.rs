use super::*;

impl App {
    /// Start a read_loop for a terminal on a background thread
    pub(super) fn start_read_loop(
        &mut self,
        tab_id: String,
        pane_id: String,
        session: CompletionSessionKey,
        terminal: Arc<Mutex<TerminalState>>,
    ) {
        let Some(capability) = self.tab_manager.find_by_id(&tab_id).and_then(|index| {
            let tab = &self.tab_manager.tabs[index];
            tab.pane(&pane_id)
                .filter(|pane| pane.completion.session() == &session)
                .map(|_| tab.tab_type.zmodem_capability())
        }) else {
            return;
        };
        let receive_directory =
            settings::resolve_zmodem_download_dir(&self.settings.zmodem.download_dir);
        let mut unavailable_reason = receive_directory
            .as_ref()
            .err()
            .map(|error| format!("ZMODEM 下载目录无效：{error}"));
        let receive_directory = receive_directory.unwrap_or_else(|_| std::path::PathBuf::from("/"));
        let transfer_id = match allocate_zmodem_transfer_id(&mut self.next_zmodem_transfer_id) {
            Ok(transfer_id) => Some(transfer_id),
            Err(error) => {
                unavailable_reason.get_or_insert_with(|| error.into());
                None
            }
        };
        let identity = zmodem::runtime::TransferIdentity {
            transfer_id: transfer_id.unwrap_or(0),
            generation: session.generation,
        };
        let (commands, command_receiver) = zmodem::runtime::runtime_command_channel();
        let mut config = zmodem::runtime::RuntimeConfig::new(
            capability,
            receive_directory,
            identity,
            command_receiver,
        );
        config.enabled =
            self.settings.zmodem.enabled && unavailable_reason.is_none() && transfer_id.is_some();
        config.auto_detect = self.settings.zmodem.auto_detect;
        config.transfer_timeout = Some(Duration::from_secs(
            self.settings.zmodem.timeout_secs.into(),
        ));
        config.allow_settings_enable = unavailable_reason.is_none() && transfer_id.is_some();
        config.use_settings_source(self.zmodem_settings_source.clone());
        replace_zmodem_slot(
            &mut self.zmodem_controls,
            &mut self.zmodem_views,
            ZmodemControlSlot {
                tab_id: tab_id.clone(),
                pane_id: pane_id.clone(),
                session: session.clone(),
                commands,
                pending_send: None,
                capability,
                unavailable_reason,
            },
        );
        self.end_api_pane_stream(&pane_id);
        let output_sink = self
            .api_outputs
            .begin_stream(tab_id.clone(), pane_id.clone());
        self.api_streams
            .insert(pane_id.clone(), (tab_id.clone(), output_sink.stream_id()));

        let redraw_proxy = self.proxy.clone();
        let event_proxy = self.proxy.clone();
        let zmodem_proxy = self.proxy.clone();
        let zmodem_tab_id = tab_id.clone();
        let zmodem_pane_id = pane_id.clone();
        let zmodem_session = session.clone();
        let recordings = self.recordings.clone();
        let terminal_logs = self.terminal_logs.clone();
        let recording_pane_id = pane_id.clone();
        std::thread::spawn(move || {
            terminal::read_loop_with_zmodem(
                terminal,
                move || {
                    let _ = redraw_proxy.send_event(UserEvent::Redraw);
                },
                move |event| {
                    let _ = event_proxy.send_event(UserEvent::TerminalIntegration {
                        tab_id: tab_id.clone(),
                        pane_id: pane_id.clone(),
                        session: session.clone(),
                        event,
                    });
                },
                move |bytes| {
                    output_sink.append(bytes);
                    recordings.record_output(&recording_pane_id, bytes);
                    terminal_logs.record_output(&recording_pane_id, bytes);
                },
                config,
                move |event| {
                    let _ = zmodem_proxy.send_event(UserEvent::Zmodem {
                        tab_id: zmodem_tab_id.clone(),
                        pane_id: zmodem_pane_id.clone(),
                        session: zmodem_session.clone(),
                        event,
                    });
                },
            );
        });
    }

    pub(super) fn remove_zmodem_pane(&mut self, pane_id: &str) {
        remove_zmodem_pane_resources(&mut self.zmodem_controls, &mut self.zmodem_views, pane_id);
    }

    pub(super) fn remove_zmodem_tab(&mut self, tab_id: &str) {
        let pane_ids = self
            .tab_manager
            .find_by_id(tab_id)
            .map(|index| {
                self.tab_manager.tabs[index]
                    .panes()
                    .map(|pane| pane.id().to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        remove_zmodem_tab_resources(&mut self.zmodem_controls, &mut self.zmodem_views, tab_id);
        for pane_id in pane_ids {
            self.zmodem_views.remove(&pane_id);
        }
    }

    pub(super) fn shutdown_all_zmodem(&mut self) {
        let pane_ids = self.zmodem_controls.keys().cloned().collect::<Vec<_>>();
        for pane_id in pane_ids {
            self.remove_zmodem_pane(&pane_id);
        }
        self.zmodem_views.clear();
    }

    pub(super) fn handle_zmodem_event(
        &mut self,
        tab_id: String,
        pane_id: String,
        session: CompletionSessionKey,
        event: zmodem::runtime::RuntimeEvent,
    ) {
        if !zmodem_event_identity_is_current(
            &self.tab_manager,
            &self.zmodem_controls,
            &tab_id,
            &pane_id,
            &session,
            event.identity.generation,
        ) {
            return;
        }
        observe_zmodem_transfer_id(
            &mut self.next_zmodem_transfer_id,
            event.identity.transfer_id,
        );
        let pending_send = &mut self
            .zmodem_controls
            .get_mut(&pane_id)
            .expect("已验证的 ZMODEM 控制槽缺失")
            .pending_send;
        let view = self.zmodem_views.entry(pane_id).or_default();
        if apply_zmodem_runtime_event_arbitrated(view, pending_send, event) {
            self.invalidate_completion_popup_snapshot();
        }
    }

    pub(super) fn handle_zmodem_ui_actions(
        &mut self,
        tab_id: &str,
        pane_id: &str,
        session: &CompletionSessionKey,
        actions: Vec<zmodem::ui::ZmodemUiAction>,
    ) {
        let Some(slot) = self.zmodem_controls.get(pane_id).cloned() else {
            return;
        };
        if slot.tab_id != tab_id || slot.pane_id != pane_id || slot.session != *session {
            return;
        }
        for action in actions {
            match action {
                zmodem::ui::ZmodemUiAction::StartSend { paths } => {
                    let capability = zmodem_ui_capability(
                        slot.capability,
                        self.settings.zmodem.enabled,
                        true,
                        slot.unavailable_reason.as_deref(),
                    );
                    if let Some(reason) = capability.disabled_reason() {
                        self.zmodem_views
                            .entry(pane_id.to_string())
                            .or_default()
                            .show_send_error(reason);
                        continue;
                    }
                    let commands = slot.commands.clone();
                    let pending_send = &mut self
                        .zmodem_controls
                        .get_mut(pane_id)
                        .expect("已验证的 ZMODEM 控制槽缺失")
                        .pending_send;
                    let view = self.zmodem_views.entry(pane_id.to_string()).or_default();
                    request_zmodem_send(
                        &commands,
                        pending_send,
                        view,
                        &mut self.next_zmodem_transfer_id,
                        session.generation,
                        paths,
                    );
                    self.invalidate_completion_popup_snapshot();
                    self.clear_terminal_ime_composition();
                }
                zmodem::ui::ZmodemUiAction::Cancel { transfer_id } => {
                    let current = self
                        .zmodem_views
                        .get(pane_id)
                        .is_some_and(|view| view.active_transfer_id() == Some(transfer_id));
                    if !current {
                        continue;
                    }
                    let result = try_send_zmodem_command(
                        &slot.commands,
                        zmodem::runtime::RuntimeCommand::Cancel(
                            zmodem::runtime::TransferIdentity {
                                transfer_id,
                                generation: session.generation,
                            },
                        ),
                    );
                    let view = self
                        .zmodem_views
                        .get_mut(pane_id)
                        .expect("已验证的 ZMODEM 视图缺失");
                    match result {
                        Ok(()) => {
                            let _ = view
                                .set_status(transfer_id, zmodem::ui::TransferStatus::Cancelling);
                        }
                        Err(ZmodemControlSendError::Full) => {
                            view.show_transfer_error(ZmodemControlSendError::Full.message());
                        }
                        Err(ZmodemControlSendError::Disconnected) => {
                            let _ = view.set_status(
                                transfer_id,
                                zmodem::ui::TransferStatus::Failed(
                                    ZmodemControlSendError::Disconnected.message().into(),
                                ),
                            );
                        }
                    }
                }
                zmodem::ui::ZmodemUiAction::Dismiss { transfer_id } => {
                    if let Some(view) = self.zmodem_views.get_mut(pane_id) {
                        let _ = view.dismiss_transfer(transfer_id);
                    }
                }
            }
        }
    }

    pub(super) fn request_local_history(
        &mut self,
        tab_id: String,
        pane_id: String,
        session: CompletionSessionKey,
        path: std::path::PathBuf,
    ) {
        let Some(index) = self.tab_manager.find_by_id(&tab_id) else {
            return;
        };
        let Some(pane) = self.tab_manager.tabs[index].pane_mut(&pane_id) else {
            return;
        };
        let completion = &mut pane.completion;
        if completion.session() != &session
            || completion.history_path().map(std::path::Path::new) != Some(path.as_path())
        {
            return;
        }
        let request = completion.mark_history_loading();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let result =
                smart_completion::read_history_tail(&path, smart_completion::MAX_HISTORY_BYTES);
            let _ = proxy.send_event(UserEvent::CompletionHistory {
                tab_id,
                pane_id,
                session,
                request,
                path,
                result,
            });
        });
    }

    pub(super) fn maybe_request_remote_history(&mut self, tab_id: &str, pane_id: &str) {
        let Some(index) = self.tab_manager.find_by_id(tab_id) else {
            return;
        };
        let tab = &self.tab_manager.tabs[index];
        let Some(pane) = tab.pane(pane_id) else {
            return;
        };
        let request = remote_history_request(
            matches!(tab.tab_type, TabType::Ssh { .. }),
            &pane.completion,
            self.sftp_workers.contains_key(pane_id),
        );
        let Some((session, path)) = request else {
            return;
        };
        if let Some(worker) = self.sftp_workers.get(pane_id) {
            let request = self.tab_manager.tabs[index]
                .pane_mut(pane_id)
                .expect("validated pane must remain")
                .completion
                .mark_history_loading();
            let result = worker.send(sftp::SftpCommand::ReadCompletionHistory {
                session,
                request: request.clone(),
                path,
                max_bytes: smart_completion::MAX_HISTORY_BYTES,
            });
            if let Err(error) = result {
                self.tab_manager.tabs[index]
                    .pane_mut(pane_id)
                    .expect("validated pane must remain")
                    .completion
                    .apply_history_result(&request, Err::<Vec<String>, _>(error));
            }
        }
    }

    pub(super) fn next_completion_request_id(&mut self) -> u64 {
        advance_completion_request_id(&mut self.completion_request_id)
    }

    pub(super) fn stage_completion_fill(
        &mut self,
        tab_id: &str,
        pane_id: &str,
        candidate: &str,
    ) -> Result<(), String> {
        self.invalidate_completion_popup_snapshot();
        if !self
            .tab_manager
            .active()
            .is_some_and(|tab| tab.id == tab_id && tab.active_pane_id() == pane_id)
        {
            return Err("补全标签页已切换".into());
        }
        let index = self
            .tab_manager
            .find_by_id(tab_id)
            .ok_or_else(|| "补全标签页已关闭".to_string())?;
        let pane = self.tab_manager.tabs[index]
            .pane(pane_id)
            .ok_or_else(|| "补全面板已关闭".to_string())?;
        let direct_prefix = pane.completion.direct_fill_prefix().map(str::to_owned);
        let terminal = pane.terminal.clone();
        let request = terminal
            .lock()
            .unwrap()
            .stage_completion_fill(candidate, direct_prefix.as_deref())?;
        if self.tab_manager.tabs[index]
            .pane(pane_id)
            .is_none_or(|pane| pane.completion.session() != &request.session)
        {
            return Err("补全会话已失效".into());
        }

        let request_id = self.next_completion_request_id();
        if !self.tab_manager.tabs[index]
            .pane_mut(pane_id)
            .expect("validated pane must remain")
            .completion
            .begin_fill(request_id, candidate)
        {
            return Err("已有补全填充正在进行".into());
        }
        let dispatch = match request.target {
            terminal::CandidateWriteTarget::Local(path) => {
                let proxy = self.proxy.clone();
                let tab_id = tab_id.to_string();
                let pane_id = pane_id.to_string();
                let session = request.session;
                let bytes = request.bytes;
                std::thread::spawn(move || {
                    let result = bash_integration::write_local_candidate_atomic(&path, &bytes);
                    let _ = proxy.send_event(UserEvent::CompletionCandidateWritten {
                        tab_id,
                        pane_id,
                        session,
                        request_id,
                        result,
                    });
                });
                Ok(())
            }
            terminal::CandidateWriteTarget::Remote(path) => self
                .sftp_workers
                .get(pane_id)
                .ok_or_else(|| "SFTP worker 不存在".to_string())
                .and_then(|worker| {
                    worker.send(sftp::SftpCommand::WriteCompletionCandidate {
                        session: request.session,
                        request_id,
                        path,
                        bytes: request.bytes,
                    })
                }),
            terminal::CandidateWriteTarget::Direct => {
                let still_current = self.tab_manager.active().is_some_and(|tab| {
                    tab.id == tab_id
                        && tab.active_pane_id() == pane_id
                        && tab.completion.session() == &request.session
                        && tab.completion.direct_fill_prefix() == direct_prefix.as_deref()
                        && tab.completion.pending_fill_matches(request_id)
                });
                if !still_current {
                    Err("补全上下文已失效".into())
                } else if terminal
                    .lock()
                    .unwrap()
                    .commit_direct_completion_fill(&request.bytes)
                {
                    self.tab_manager.tabs[index]
                        .pane_mut(pane_id)
                        .expect("validated pane must remain")
                        .completion
                        .finish_fill(request_id);
                    Ok(())
                } else {
                    Err("无法写入补全内容".into())
                }
            }
        };
        if let Err(error) = dispatch {
            self.invalidate_completion_popup_snapshot();
            self.tab_manager.tabs[index]
                .pane_mut(pane_id)
                .expect("validated pane must remain")
                .completion
                .fail_fill(request_id);
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn handle_completion_candidate_written(
        &mut self,
        tab_id: &str,
        pane_id: &str,
        event_session: &CompletionSessionKey,
        request_id: u64,
        result: Result<(), String>,
    ) {
        self.invalidate_completion_popup_snapshot();
        let Some(index) = self.tab_manager.find_by_id(tab_id) else {
            return;
        };
        if !self
            .tab_manager
            .active()
            .is_some_and(|tab| tab.id == tab_id && tab.active_pane_id() == pane_id)
        {
            if let Some(pane) = self.tab_manager.tabs[index].pane_mut(pane_id) {
                pane.completion.fail_fill(request_id);
            }
            return;
        }
        let Some(pane) = self.tab_manager.tabs[index].pane_mut(pane_id) else {
            return;
        };
        let current_session = pane.completion.session().clone();
        let may_commit = completion_fill_may_commit(
            &pane.completion,
            &current_session,
            event_session,
            request_id,
            &result,
        );
        if !may_commit {
            if result.is_err()
                && current_session == *event_session
                && pane.completion.pending_fill_matches(request_id)
            {
                pane.completion.fail_fill(request_id);
            }
            return;
        }

        let terminal = pane.terminal.clone();
        let committed = terminal.lock().unwrap().commit_completion_fill();
        if committed {
            pane.completion.finish_fill(request_id);
        } else {
            pane.completion.fail_fill(request_id);
        }
    }

    pub(super) fn write_active_user_input(&mut self, input: &str) {
        self.invalidate_completion_popup_snapshot();
        let index = self.tab_manager.active_idx;
        let Some(tab) = self.tab_manager.tabs.get_mut(index) else {
            return;
        };
        if !tab.tab_type.is_terminal() || matches!(tab.tab_type, TabType::Recording { .. }) {
            return;
        }
        let terminal = tab.terminal.clone();
        let mut terminal = terminal.lock().unwrap();
        if !terminal.completion_surface_safe() {
            tab.completion.pause_surface_tracking();
            terminal.write_input(input);
            return;
        }
        tab.completion.resume_surface_tracking();
        if input == "\x04" && tab.completion.observe_empty_ctrl_d() {
            terminal.invalidate_prompt();
            terminal.write_input(input);
            return;
        }
        let effect = apply_completion_user_input_state(
            &mut tab.completion,
            &mut self.completion_popup_snapshot,
            input,
        );
        apply_completion_prompt_effect(&mut terminal, effect);
        terminal.write_input(input);
    }

    pub(super) fn submit_active_bash_line(&mut self) {
        self.invalidate_completion_popup_snapshot();
        let index = self.tab_manager.active_idx;
        let proxy = self.proxy.clone();
        let Some(tab) = self.tab_manager.tabs.get_mut(index) else {
            return;
        };
        if !tab.tab_type.is_terminal() || matches!(tab.tab_type, TabType::Recording { .. }) {
            return;
        }
        let tab_id = tab.id.clone();
        let pane_id = tab.active_pane_id().to_string();
        let session = tab.completion.session().clone();
        let host_scope = adb_history_scope(&tab.tab_type);
        let terminal = tab.terminal.clone();
        let mut terminal = terminal.lock().unwrap();
        let authenticated_prompt = terminal.has_authenticated_active_bash_prompt();
        let fallback = terminal.take_bash_submission();
        let persist_identity = tab.completion.adb_submission_identity().cloned();
        let submission = tab.completion.complete_submission(fallback.as_deref());
        let entered_serial = tab
            .completion
            .observe_submission(submission.as_deref(), authenticated_prompt);
        let load_request = entered_serial
            .zip(host_scope)
            .and_then(|(serial, scope)| adb_history::AdbHistoryIdentity::new(scope, serial))
            .and_then(|identity| {
                tab.completion
                    .activate_adb_history(identity)
                    .then(|| tab.completion.mark_adb_history_loading())
                    .flatten()
            });
        terminal.write_input("\r");
        drop(terminal);

        if let (Some(identity), Some(command)) = (
            persist_identity,
            submission
                .as_deref()
                .filter(|command| !command.is_empty())
                .map(str::to_owned),
        ) {
            if self.adb_history_writer.enqueue(identity, command).is_err() {
                log::warn!("ADB 补全历史写入队列不可用");
            }
        }

        if let Some(request) = load_request {
            match self
                .adb_history_writer
                .enqueue_load(request.identity().clone())
            {
                Ok(receiver) => std::thread::spawn(move || {
                    let result = receiver
                        .recv()
                        .unwrap_or_else(|_| Err("ADB 历史加载器不可用".into()));
                    let _ = proxy.send_event(UserEvent::AdbCompletionHistory {
                        tab_id,
                        pane_id,
                        session,
                        request,
                        result,
                    });
                }),
                Err(error) => {
                    let _ = proxy.send_event(UserEvent::AdbCompletionHistory {
                        tab_id,
                        pane_id,
                        session,
                        request,
                        result: Err(error),
                    });
                    return;
                }
            };
        }
    }
}
