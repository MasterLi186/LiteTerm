use super::*;

impl App {
    pub(super) fn open_settings_tab(&mut self) {
        if let Some(renderer) = &self.renderer {
            self.settings_panel
                .set_font_families(renderer.monospace_font_families());
        }
        if self
            .tab_manager
            .active()
            .is_some_and(|tab| matches!(tab.tab_type, TabType::Settings))
        {
            return;
        }
        self.prepare_for_active_tab_change();
        let (_, created) = self.tab_manager.open_settings();
        if created {
            self.settings_panel.open(&self.settings);
            if let Some(renderer) = &self.renderer {
                self.settings_panel
                    .set_font_families(renderer.monospace_font_families());
            }
        } else {
            self.settings_panel.visible = true;
        }
    }

    pub(super) fn new_local_tab_with_shell(&mut self, shell: &str) {
        self.prepare_for_active_tab_change();
        let (cols, rows) = self.grid_size();
        let (tab_id, terminal) = self.tab_manager.new_local(shell, cols, rows);
        let pane_session = self
            .tab_manager
            .active()
            .filter(|tab| tab.id == tab_id)
            .map(|tab| tab.completion.session().clone())
            .expect("新建本地标签缺少补全会话");
        let history_request = self
            .tab_manager
            .active()
            .filter(|tab| tab.id == tab_id)
            .and_then(|tab| {
                tab.completion.history_path().map(|path| {
                    (
                        tab.completion.session().clone(),
                        std::path::PathBuf::from(path),
                    )
                })
            });
        self.start_read_loop(tab_id.clone(), tab_id.clone(), pane_session, terminal);
        if let Some((session, path)) = history_request {
            self.request_local_history(tab_id.clone(), tab_id, session, path);
        }
    }

    /// Create a new local terminal tab with default shell
    pub(super) fn new_local_tab(&mut self) {
        let shell = crate::terminal::default_shell_path();
        self.new_local_tab_with_shell(&shell);
    }

    /// Create a new SSH tab (connects in background)
    pub(super) fn new_ssh_tab(&mut self, conn: &sidebar::SshConnection) -> String {
        self.prepare_for_active_tab_change();
        let tab_id = self.tab_manager.new_ssh_placeholder(conn);
        let params = crate::ssh::ConnectionParams::from(conn);
        let session = self
            .tab_manager
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .map(|tab| tab.completion.session().clone())
            .expect("新建 SSH 标签缺少补全会话");
        self.spawn_ssh_connect(tab_id.clone(), tab_id.clone(), params, session);
        tab_id
    }

    pub(super) fn handle_new_tab_selector_action(
        &mut self,
        action: new_tab_selector::NewTabAction,
        connections: &[sidebar::SshConnection],
    ) -> bool {
        match action {
            new_tab_selector::NewTabAction::None => false,
            new_tab_selector::NewTabAction::Close => {
                self.new_tab_selector.close();
                true
            }
            new_tab_selector::NewTabAction::OpenShell(path) => {
                if let Some(shell) = path.to_str() {
                    self.new_local_tab_with_shell(shell);
                    self.new_tab_selector.close();
                }
                true
            }
            new_tab_selector::NewTabAction::OpenSsh(key) => {
                if let Some(connection) =
                    new_tab_selector::resolve_ssh_connection(connections, &key).cloned()
                {
                    self.new_ssh_tab(&connection);
                    self.new_tab_selector.close();
                }
                true
            }
            new_tab_selector::NewTabAction::OpenSerial(spec) => {
                self.new_serial_tab(spec);
                self.new_tab_selector.close();
                true
            }
            new_tab_selector::NewTabAction::RefreshSerial(_) => {
                let generation = self.new_tab_selector.begin_serial_scan();
                self.serial_scan(generation);
                true
            }
            new_tab_selector::NewTabAction::NewSsh => {
                self.invalidate_completion_popup_snapshot();
                self.new_tab_selector.close();
                self.sidebar.new_conn = sidebar::NewConnForm::default();
                self.sidebar.show_new_connection = true;
                true
            }
        }
    }

    pub(super) fn spawn_ssh_connect(
        &self,
        tab_id: String,
        pane_id: String,
        params: ssh::ConnectionParams,
        session: CompletionSessionKey,
    ) {
        let (cols, rows) = self.grid_size_for_tab_pane(&tab_id, &pane_id);
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let result = crate::ssh::connect(&params, cols, rows, Some(session.clone()));
            let event = UserEvent::SshReady {
                tab_id,
                pane_id,
                session,
                result,
            };
            if let Err(winit::event_loop::EventLoopClosed(event)) = proxy.send_event(event) {
                cleanup_undelivered_user_event(event);
            }
        });
    }

    pub(super) fn reconnect_ssh_tab(&mut self, index: usize) {
        let Some(pane_id) = self
            .tab_manager
            .tabs
            .get(index)
            .filter(|tab| matches!(tab.tab_type, TabType::Ssh { .. }))
            .map(|tab| tab.active_pane_id().to_string())
        else {
            return;
        };
        self.end_api_pane_stream(&pane_id);
        self.remove_zmodem_pane(&pane_id);
        let Some(plan) = self.tab_manager.reset_ssh_for_reconnect(index) else {
            return;
        };
        plan.old_terminal.lock().unwrap().shutdown();
        self.remove_sftp_pane(&plan.pane_id);
        self.spawn_ssh_connect(plan.tab_id, plan.pane_id, plan.params, plan.session);
    }

    pub(super) fn reconnect_serial_tab(&mut self, index: usize) {
        let Some(pane_id) = self
            .tab_manager
            .tabs
            .get(index)
            .filter(|tab| matches!(tab.tab_type, TabType::Serial { .. }))
            .map(|tab| tab.active_pane_id().to_string())
        else {
            return;
        };
        self.end_api_pane_stream(&pane_id);
        self.remove_zmodem_pane(&pane_id);
        let Some(plan) = self.tab_manager.reset_serial_for_reconnect(index) else {
            return;
        };
        plan.old_terminal.lock().unwrap().shutdown();
        self.spawn_serial_open(plan.open);
    }

    pub(super) fn disconnect_serial_tab(&mut self, index: usize) {
        let Some(pane_id) = self
            .tab_manager
            .tabs
            .get(index)
            .filter(|tab| matches!(tab.tab_type, TabType::Serial { .. }))
            .map(|tab| tab.active_pane_id().to_string())
        else {
            return;
        };
        self.end_api_pane_stream(&pane_id);
        self.remove_zmodem_pane(&pane_id);
        let Some(plan) = self.tab_manager.disconnect_serial(index) else {
            return;
        };
        plan.old_terminal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .shutdown();
        debug_assert_eq!(plan.pane_id, pane_id);
        self.clear_selection();
        self.invalidate_completion_popup_snapshot();
        self.terminal_notice = Some("串口已断开".into());
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    pub(super) fn request_listing(
        &mut self,
        tab_id: &str,
        side: sftp::FileSide,
        path: Option<String>,
    ) {
        let Some(pane_id) = self.tab_manager.find_by_id(tab_id).and_then(|index| {
            let tab = &self.tab_manager.tabs[index];
            self.sftp_workers
                .contains_key(tab.active_pane_id())
                .then(|| tab.active_pane_id().to_string())
        }) else {
            return;
        };
        self.request_listing_for_pane(tab_id, &pane_id, side, path);
    }

    pub(super) fn request_listing_for_pane(
        &mut self,
        tab_id: &str,
        pane_id: &str,
        side: sftp::FileSide,
        path: Option<String>,
    ) {
        let Some(state) = self.file_browsers.get_mut(tab_id) else {
            return;
        };
        let path = path.unwrap_or_else(|| match side {
            sftp::FileSide::Local => state.local.path.clone(),
            sftp::FileSide::Remote => state.remote.path.clone(),
        });
        let request_id = state.next_request(side, path.clone());
        let command = match side {
            sftp::FileSide::Local => sftp::SftpCommand::ListLocal { request_id, path },
            sftp::FileSide::Remote => sftp::SftpCommand::ListRemote { request_id, path },
        };
        let result = self
            .sftp_workers
            .get(pane_id)
            .ok_or_else(|| "SFTP worker 不存在".to_string())
            .and_then(|worker| worker.send(command));
        if let Err(error) = result {
            let pane = match side {
                sftp::FileSide::Local => &mut state.local,
                sftp::FileSide::Remote => &mut state.remote,
            };
            pane.loading = false;
            pane.error = Some(error);
        }
    }

    pub(super) fn handle_file_browser_action(
        &mut self,
        tab_id: &str,
        pane_id: &str,
        action: file_browser::FileBrowserAction,
    ) {
        match action {
            file_browser::FileBrowserAction::Toggle => {}
            file_browser::FileBrowserAction::List { side, path } => {
                self.request_listing_for_pane(tab_id, pane_id, side, Some(path));
            }
            file_browser::FileBrowserAction::Reconnect => {
                if let Some(index) = self.tab_manager.find_by_id(tab_id) {
                    if let Some(pane) = self.tab_manager.tabs[index].pane_mut(pane_id) {
                        prepare_sftp_reconnect_state(&mut pane.completion);
                    }
                }
                if let Some(worker) = self.sftp_workers.get(pane_id) {
                    let _ = worker.send(sftp::SftpCommand::Reconnect);
                }
            }
            file_browser::FileBrowserAction::Upload {
                local_path,
                remote_path,
            } => {
                let id = uuid::Uuid::new_v4().to_string();
                let filename = std::path::Path::new(&local_path).file_name().map_or_else(
                    || local_path.clone(),
                    |name| name.to_string_lossy().into_owned(),
                );
                if let Some(state) = self.file_browsers.get_mut(tab_id) {
                    state.start_transfer(id.clone(), filename, sftp::TransferDirection::Upload);
                }
                let result = self
                    .sftp_workers
                    .get(pane_id)
                    .ok_or_else(|| "SFTP worker 不存在".to_string())
                    .and_then(|worker| {
                        worker.send(sftp::SftpCommand::Upload {
                            transfer_id: id.clone(),
                            local_path,
                            remote_path,
                        })
                    });
                if let Err(error) = result {
                    if let Some(state) = self.file_browsers.get_mut(tab_id) {
                        state.apply_event(&sftp::SftpEvent::TransferFinished {
                            tab_id: tab_id.to_string(),
                            transfer_id: id,
                            direction: sftp::TransferDirection::Upload,
                            result: Err(error),
                        });
                    }
                }
            }
            file_browser::FileBrowserAction::Download {
                remote_path,
                local_path,
            } => {
                let id = uuid::Uuid::new_v4().to_string();
                let filename = std::path::Path::new(&remote_path).file_name().map_or_else(
                    || remote_path.clone(),
                    |name| name.to_string_lossy().into_owned(),
                );
                if let Some(state) = self.file_browsers.get_mut(tab_id) {
                    state.start_transfer(id.clone(), filename, sftp::TransferDirection::Download);
                }
                let result = self
                    .sftp_workers
                    .get(pane_id)
                    .ok_or_else(|| "SFTP worker 不存在".to_string())
                    .and_then(|worker| {
                        worker.send(sftp::SftpCommand::Download {
                            transfer_id: id.clone(),
                            remote_path,
                            local_path,
                        })
                    });
                if let Err(error) = result {
                    if let Some(state) = self.file_browsers.get_mut(tab_id) {
                        state.apply_event(&sftp::SftpEvent::TransferFinished {
                            tab_id: tab_id.to_string(),
                            transfer_id: id,
                            direction: sftp::TransferDirection::Download,
                            result: Err(error),
                        });
                    }
                }
            }
            file_browser::FileBrowserAction::Rename {
                side,
                old_path,
                new_path,
            } => {
                let result = self
                    .sftp_workers
                    .get(pane_id)
                    .ok_or_else(|| "SFTP worker 不存在".to_string())
                    .and_then(|worker| {
                        worker.send(sftp::SftpCommand::Rename {
                            side,
                            old_path,
                            new_path,
                        })
                    });
                if let Err(error) = result {
                    if let Some(state) = self.file_browsers.get_mut(tab_id) {
                        state.apply_event(&sftp::SftpEvent::MutationFinished {
                            tab_id: tab_id.to_string(),
                            side,
                            operation: sftp::FileOperation::Rename,
                            result: Err(error),
                        });
                    }
                }
            }
            file_browser::FileBrowserAction::Create { side, path, kind } => {
                let result = self
                    .sftp_workers
                    .get(pane_id)
                    .ok_or_else(|| "SFTP worker 不存在".to_string())
                    .and_then(|worker| worker.send(sftp::SftpCommand::Create { side, path, kind }));
                if let Err(error) = result {
                    if let Some(state) = self.file_browsers.get_mut(tab_id) {
                        state.apply_event(&sftp::SftpEvent::MutationFinished {
                            tab_id: tab_id.to_string(),
                            side,
                            operation: sftp::FileOperation::Create,
                            result: Err(error),
                        });
                    }
                }
            }
            file_browser::FileBrowserAction::Delete { side, path, is_dir } => {
                let result = self
                    .sftp_workers
                    .get(pane_id)
                    .ok_or_else(|| "SFTP worker 不存在".to_string())
                    .and_then(|worker| {
                        worker.send(sftp::SftpCommand::Delete { side, path, is_dir })
                    });
                if let Err(error) = result {
                    if let Some(state) = self.file_browsers.get_mut(tab_id) {
                        state.apply_event(&sftp::SftpEvent::MutationFinished {
                            tab_id: tab_id.to_string(),
                            side,
                            operation: sftp::FileOperation::Delete,
                            result: Err(error),
                        });
                    }
                }
            }
        }
    }

    pub(super) fn remove_sftp_tab(&mut self, tab_id: &str) {
        shutdown_and_remove_tab_scoped_resources(
            &mut self.file_browsers,
            &mut self.sftp_workers,
            tab_id,
        );
    }

    pub(super) fn remove_sftp_pane(&mut self, pane_id: &str) {
        shutdown_and_remove_pane_worker(&mut self.sftp_workers, pane_id);
    }

    pub(super) fn split_active_terminal(&mut self, direction: SplitDirection) {
        let (cols, rows) = self.grid_size();
        let split = self.tab_manager.split_active_pane(direction, cols, rows);
        if split.is_ok() {
            self.clear_terminal_ime_composition();
        }
        match split {
            Ok(tab_manager::SplitPanePlan::Local {
                tab_id,
                pane_id,
                terminal,
                session,
                history_path,
            }) => {
                self.start_read_loop(tab_id.clone(), pane_id.clone(), session.clone(), terminal);
                if let Some(path) = history_path {
                    self.request_local_history(tab_id, pane_id, session, path);
                }
                self.refresh_pane_layout();
            }
            Ok(tab_manager::SplitPanePlan::Ssh {
                tab_id,
                pane_id,
                params,
                session,
            }) => {
                self.spawn_ssh_connect(tab_id, pane_id, params, session);
                self.refresh_pane_layout();
            }
            Err(error) => {
                self.terminal_notice = Some(error);
            }
        }
    }

    pub(super) fn close_active_terminal_pane(&mut self) {
        self.clear_terminal_ime_composition();
        if let Some(pane_id) = self
            .tab_manager
            .active()
            .filter(|tab| tab.tab_type.is_terminal())
            .map(|tab| tab.active_pane_id().to_string())
        {
            let _ = self.recordings.stop(&pane_id);
            let _ = self.terminal_logs.stop(&pane_id);
            self.remove_zmodem_pane(&pane_id);
        }
        match self.tab_manager.close_active_pane() {
            tab_manager::CloseActivePaneResult::NotTerminal => {
                self.terminal_notice = Some("当前标签不是可关闭的终端面板".into());
            }
            tab_manager::CloseActivePaneResult::CloseTab => {
                self.close_tab(self.tab_manager.active_idx);
            }
            tab_manager::CloseActivePaneResult::Closed { pane_id, .. } => {
                self.end_api_pane_stream(&pane_id);
                self.remove_sftp_pane(&pane_id);
                self.left_mouse_pane_id = None;
                self.clear_selection();
                self.refresh_pane_layout();
            }
        }
    }

    pub(super) fn close_tab(&mut self, index: usize) {
        cancel_pending_fill_for_tab(&mut self.tab_manager, index);
        if self.tab_manager.tabs.get(index).is_none() {
            return;
        }
        let closing_settings = matches!(self.tab_manager.tabs[index].tab_type, TabType::Settings);
        let closed_tab_id = self.tab_manager.tabs[index].id.clone();
        let closed_pane_ids = self.tab_manager.tabs[index]
            .panes()
            .map(|pane| pane.id().to_string())
            .collect::<Vec<_>>();
        for pane_id in closed_pane_ids {
            let _ = self.recordings.stop(&pane_id);
            let _ = self.terminal_logs.stop(&pane_id);
        }
        self.recording_playbacks.remove(&closed_tab_id);
        self.end_api_tab_streams(&closed_tab_id);
        self.remove_zmodem_tab(&closed_tab_id);
        if index == self.tab_manager.active_idx {
            self.prepare_for_active_tab_change();
        }
        self.process_managers.remove(&closed_tab_id);
        self.network_details.remove(&closed_tab_id);
        let actions = close_tab_scoped_resources_and_plan(
            &mut self.tab_manager,
            &mut self.file_browsers,
            &mut self.sftp_workers,
            &self.remote_monitor_params,
            index,
        )
        .expect("标签索引已在同一同步调用中验证");
        if self.tab_manager.is_empty() {
            self.new_local_tab();
        }
        if closing_settings {
            self.settings_panel.close();
        }
        self.apply_remote_monitor_actions(actions);
    }

    pub(super) fn close_other_tabs(&mut self, keep_index: usize) {
        let Some(keep_id) = self
            .tab_manager
            .tabs
            .get(keep_index)
            .map(|tab| tab.id.clone())
        else {
            return;
        };
        let keeping_settings = self
            .tab_manager
            .tabs
            .get(keep_index)
            .is_some_and(|tab| matches!(tab.tab_type, TabType::Settings));
        assert_tab_scoped_resource_invariant(&self.file_browsers, &self.sftp_workers);
        let removed_ids: Vec<String> = self
            .tab_manager
            .tabs
            .iter()
            .filter(|tab| tab.id != keep_id)
            .map(|tab| tab.id.clone())
            .collect();
        for tab_id in &removed_ids {
            self.end_api_tab_streams(tab_id);
            self.remove_zmodem_tab(tab_id);
            self.recording_playbacks.remove(tab_id);
        }
        for tab in self.tab_manager.tabs.iter().filter(|tab| tab.id != keep_id) {
            for pane in tab.panes() {
                let _ = self.recordings.stop(pane.id());
                let _ = self.terminal_logs.stop(pane.id());
            }
        }
        for tab in self
            .tab_manager
            .tabs
            .iter_mut()
            .filter(|tab| tab.id != keep_id)
        {
            tab.completion.cancel_pending_fill();
        }
        let removed_terminals = self
            .tab_manager
            .tabs
            .iter()
            .filter(|tab| tab.id != keep_id && tab.tab_type.is_terminal())
            .map(|tab| tab.terminal.clone())
            .collect::<Vec<_>>();
        if keep_index != self.tab_manager.active_idx {
            self.prepare_for_active_tab_change();
        }
        for terminal in removed_terminals {
            terminal.lock().unwrap().shutdown();
        }
        self.tab_manager.close_others(keep_index);
        if !keeping_settings {
            self.settings_panel.close();
        }
        self.process_managers.retain(|tab_id, _| tab_id == &keep_id);
        self.network_details.retain(|tab_id, _| tab_id == &keep_id);
        for tab_id in removed_ids {
            self.remove_sftp_tab(&tab_id);
        }
        assert_tab_scoped_resource_invariant(&self.file_browsers, &self.sftp_workers);
        self.reconcile_remote_monitors();
    }
}
