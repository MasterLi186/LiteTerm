use super::*;

impl App {
    pub(super) fn is_settings_tab_active(&self) -> bool {
        self.tab_manager
            .active()
            .is_some_and(|tab| matches!(tab.tab_type, TabType::Settings))
    }

    pub(super) fn invalidate_completion_popup_snapshot(&mut self) {
        self.completion_popup_snapshot = None;
        self.completion_popup_epoch = self.completion_popup_epoch.wrapping_add(1);
    }

    /// Sidebar modal dialogs that steal text input from the terminal.
    pub(super) fn has_sidebar_dialog(&self) -> bool {
        self.sidebar.password_prompt.is_some()
            || self.sidebar.show_new_connection
            || self.sidebar.show_key_manager
    }

    pub(super) fn active_zmodem_has_overlay(&self) -> bool {
        self.tab_manager
            .active()
            .filter(|tab| tab.tab_type.is_terminal())
            .and_then(|tab| self.zmodem_views.get(tab.active_pane_id()))
            .is_some_and(zmodem::ui::PaneZmodemView::has_overlay)
    }

    pub(super) fn has_blocking_dialog(&self) -> bool {
        blocking_dialog_visible(self.has_sidebar_dialog(), self.new_tab_selector.is_open())
            || self.command_bar.has_blocking_dialog()
            || self.tab_rename_dialog.is_open()
            || self.batch_dialog.is_open()
            || self.tunnel_manager.is_open()
            || self.recording_dialog.is_open()
            || self.active_zmodem_has_overlay()
    }

    /// Resolve current IME input owner from UI focus / modal state.
    /// Completion popup alone does not force Egui ownership.
    pub(super) fn resolve_current_ime_owner(&self) -> ime::InputOwner {
        if self.active_terminal().is_none() {
            return ime::InputOwner::Egui;
        }
        resolve_ime_input_owner(
            self.is_settings_tab_active(),
            self.has_blocking_dialog(),
            completion_blocking_egui_overlay_visible(&self.egui_ctx),
            self.search_field_owns_focus,
            self.egui_ctx.wants_keyboard_input(),
        )
    }

    pub(super) fn current_terminal_ime_identity(&self) -> Option<TerminalImeIdentity> {
        self.tab_manager
            .active()
            .filter(|tab| tab.tab_type.is_terminal())
            .map(|tab| TerminalImeIdentity {
                tab_id: tab.id.clone(),
                pane_id: tab.active_pane_id().to_string(),
            })
    }

    pub(super) fn clear_terminal_ime_composition(&mut self) {
        let _ = self.ime.on_focus_lost();
        self.ime_terminal_owner = None;
    }

    /// Synchronize terminal/Egui ownership. Same-owner updates are idempotent.
    pub(super) fn sync_ime_owner(&mut self, owner: ime::InputOwner) {
        if self.ime.owner() != owner {
            self.ime_terminal_owner = None;
        }
        let _ = self.ime.set_owner(owner);
    }

    /// Apply a winit IME event to the state machine and optionally write Commit once.
    pub(super) fn handle_ime_event(&mut self, ime_event: Ime) {
        let owner = self.resolve_current_ime_owner();
        self.sync_ime_owner(owner);
        let current_terminal_owner = self.current_terminal_ime_identity();

        let routed_event = match ime_event {
            Ime::Enabled => RoutedImeEvent::Enabled,
            Ime::Disabled => {
                self.ime_terminal_owner = None;
                RoutedImeEvent::Disabled
            }
            Ime::Preedit(text, cursor) => {
                if owner == ime::InputOwner::Terminal {
                    if text.is_empty() {
                        self.ime_terminal_owner = None;
                    } else if let Some(identity) = current_terminal_owner.clone() {
                        if self
                            .ime_terminal_owner
                            .as_ref()
                            .is_some_and(|existing| existing != &identity)
                        {
                            let _ = self.ime.on_focus_lost();
                        }
                        self.ime_terminal_owner = Some(identity);
                    }
                }
                RoutedImeEvent::Preedit(text, cursor)
            }
            Ime::Commit(text) => {
                if owner == ime::InputOwner::Terminal
                    && !terminal_ime_commit_matches(
                        self.ime_terminal_owner.as_ref(),
                        current_terminal_owner.as_ref(),
                    )
                {
                    self.clear_terminal_ime_composition();
                    self.do_render();
                    return;
                }
                self.ime_terminal_owner = None;
                RoutedImeEvent::Commit(text)
            }
        };
        let action = apply_routed_ime_event(&mut self.ime, owner, routed_event);

        match action {
            ime::ImeAction::Commit(text) => {
                // Egui-owner commit never returns Commit; Terminal only, once.
                if !text.is_empty() {
                    self.scroll_active_terminal_to_bottom();
                    self.clear_selection();
                    self.write_active_user_input(&text);
                }
                self.do_render();
            }
            ime::ImeAction::Redraw => {
                self.do_render();
            }
            ime::ImeAction::None => {}
        }
    }

    /// Get the active terminal, if any
    pub(super) fn active_terminal(&self) -> Option<Arc<Mutex<TerminalState>>> {
        self.tab_manager.active_terminal()
    }

    /// Scroll active terminal display to the bottom (only for terminal-directed keys).
    pub(super) fn scroll_active_terminal_to_bottom(&mut self) {
        if let Some(terminal) = self.active_terminal() {
            let mut term = terminal.lock().unwrap();
            let mut changed = false;
            if let Some(t) = term.term_mut() {
                if t.grid().display_offset() != 0 {
                    use alacritty_terminal::grid::Scroll;
                    t.scroll_display(Scroll::Bottom);
                    changed = true;
                }
            }
            if changed {
                term.mark_render_dirty();
            }
        }
    }

    /// Mutable access to the active tab (TabManager has no active_mut).
    pub(super) fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let idx = self.tab_manager.active_idx;
        self.tab_manager.tabs.get_mut(idx)
    }

    pub(super) fn remote_params_for_key(
        &self,
        key: &monitor::MonitorKey,
    ) -> Option<ssh::ConnectionParams> {
        self.tab_manager
            .active()
            .into_iter()
            .chain(self.tab_manager.tabs.iter())
            .filter_map(|tab| tab.tab_type.remote_params())
            .find(|params| monitor::MonitorKey::from_ssh(params) == *key)
            .cloned()
    }

    pub(super) fn open_process_manager(&mut self, key: monitor::MonitorKey) {
        let params = match &key {
            monitor::MonitorKey::Local => None,
            monitor::MonitorKey::Remote { .. } => self.remote_params_for_key(&key),
        };
        if matches!(key, monitor::MonitorKey::Remote { .. }) && params.is_none() {
            return;
        }
        let existing_index = self.tab_manager.tabs.iter().position(|tab| {
            matches!(
                &tab.tab_type,
                TabType::Process { key: existing, .. } if existing == &key
            )
        });
        if existing_index != Some(self.tab_manager.active_idx) {
            self.prepare_for_active_tab_change();
        }
        let label = format!("进程 - {}", key.status_text());
        let tab_id = self.tab_manager.open_process(label, key.clone(), params);
        self.process_managers
            .entry(tab_id)
            .or_insert_with(|| process_manager::ProcessManagerState::new(key));
        self.reconcile_remote_monitors();
    }

    pub(super) fn open_network_detail(
        &mut self,
        key: monitor::MonitorKey,
        initial_iface: Option<String>,
    ) {
        let params = match &key {
            monitor::MonitorKey::Local => None,
            monitor::MonitorKey::Remote { .. } => self.remote_params_for_key(&key),
        };
        if matches!(key, monitor::MonitorKey::Remote { .. }) && params.is_none() {
            return;
        }
        let existing = self.tab_manager.tabs.iter().position(
            |tab| matches!(&tab.tab_type, TabType::Network { key: existing, .. } if existing == &key),
        );
        if existing != Some(self.tab_manager.active_idx) {
            self.prepare_for_active_tab_change();
        }
        let tab_id = self
            .tab_manager
            .open_network(key.clone(), params, initial_iface.clone());
        let state = self
            .network_details
            .entry(tab_id)
            .or_insert_with(|| network_detail::NetworkDetailState::new(key, initial_iface.clone()));
        if let Some(interface) = initial_iface {
            state.select_interface(Some(interface));
        }
        self.reconcile_remote_monitors();
    }

    pub(super) fn handle_network_detail_actions(
        &mut self,
        tab_id: &str,
        key: &monitor::MonitorKey,
        actions: Vec<network_detail::NetworkDetailAction>,
    ) {
        for action in actions {
            let network_detail::NetworkDetailAction::Refresh { request_id } = action;
            match key {
                monitor::MonitorKey::Local => {
                    let proxy = self.proxy.clone();
                    let requester = tab_id.to_string();
                    let key = key.clone();
                    std::thread::spawn(move || {
                        let result = network_detail::collect_local().map(Box::new);
                        let _ = proxy.send_event(UserEvent::NetworkDetail {
                            key,
                            generation: 0,
                            requester,
                            request_id,
                            result,
                        });
                    });
                }
                monitor::MonitorKey::Remote { .. } => {
                    if let Some(worker) = self.remote_monitors.get(key) {
                        worker.fetch_network_detail(tab_id.to_string(), request_id);
                    } else if let Some(state) = self.network_details.get_mut(tab_id) {
                        let _ = state
                            .apply_snapshot(request_id, Err("远端监控尚未连接，请稍后重试".into()));
                    }
                }
            }
        }
    }

    pub(super) fn serial_scan(&self, generation: u64) {
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let result = serial::list_ports();
            let _ = proxy.send_event(UserEvent::SerialPorts { generation, result });
        });
    }

    pub(super) fn new_serial_tab(&mut self, spec: serial::SerialSpec) {
        self.prepare_for_active_tab_change();
        let plan = self.tab_manager.new_serial_placeholder(spec);
        self.spawn_serial_open(plan);
    }

    pub(super) fn spawn_serial_open(&self, plan: tab_manager::SerialOpenPlan) {
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let result = serial::open(plan.spec);
            let event = UserEvent::SerialReady {
                tab_id: plan.tab_id,
                pane_id: plan.pane_id,
                generation: plan.generation,
                result,
            };
            if let Err(winit::event_loop::EventLoopClosed(event)) = proxy.send_event(event) {
                cleanup_undelivered_user_event(event);
            }
        });
    }

    pub(super) fn batch_targets(&self) -> Vec<batch_command::BatchTarget> {
        self.tab_manager
            .tabs
            .iter()
            .filter_map(|tab| match &tab.tab_type {
                TabType::Ssh { label, params } => Some(batch_command::BatchTarget {
                    id: tab.id.clone(),
                    label: label.clone(),
                    identity: format!("{}@{}:{}", params.user, params.host, params.port),
                    connected: tab.ssh_connected,
                }),
                _ => None,
            })
            .collect()
    }

    pub(super) fn execute_batch(&mut self, command: String, tab_ids: Vec<String>) {
        let bytes = format!("{command}\r");
        let mut result = batch_command::BatchResult::default();
        for tab_id in tab_ids {
            let Some(index) = self.tab_manager.find_by_id(&tab_id) else {
                result.failed.push(tab_id);
                continue;
            };
            let label = self.tab_manager.tabs[index].label.clone();
            let terminal = self.tab_manager.tabs[index].terminal.clone();
            let write_result = terminal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .try_write_input(&bytes);
            match write_result {
                Ok(()) => result.sent.push(label),
                Err(_) => result.failed.push(label),
            }
        }
        self.batch_dialog.set_result(result);
    }

    pub(super) fn handle_process_manager_actions(
        &mut self,
        tab_id: &str,
        key: &monitor::MonitorKey,
        actions: Vec<process_manager::ProcessManagerAction>,
    ) {
        for action in actions {
            match action {
                process_manager::ProcessManagerAction::Refresh => match key {
                    monitor::MonitorKey::Local => {
                        if let Some(refresh) = &self.local_monitor_refresh {
                            let _ = refresh.try_send(());
                        }
                    }
                    monitor::MonitorKey::Remote { .. } => {
                        if let Some(worker) = self.remote_monitors.get(key) {
                            worker.refresh();
                        }
                    }
                },
                process_manager::ProcessManagerAction::Select { pid, request_id } => match key {
                    monitor::MonitorKey::Local => {
                        let proxy = self.proxy.clone();
                        let requester = tab_id.to_string();
                        std::thread::spawn(move || {
                            let result = monitor::collect_local_process_detail(pid).map(Box::new);
                            let _ = proxy.send_event(UserEvent::ProcessDetail {
                                key: monitor::MonitorKey::Local,
                                generation: 0,
                                requester,
                                request_id,
                                result,
                            });
                        });
                    }
                    monitor::MonitorKey::Remote { .. } => {
                        if let Some(worker) = self.remote_monitors.get(key) {
                            worker.fetch_process_detail(tab_id.to_string(), request_id, pid);
                        } else if let Some(state) = self.process_managers.get_mut(tab_id) {
                            let _ = state.apply_detail(
                                request_id,
                                Err("远端监控尚未连接，请稍后重试".into()),
                            );
                        }
                    }
                },
                process_manager::ProcessManagerAction::CopyText(text) => {
                    if let Some(clipboard) = &mut self.clipboard {
                        let _ = clipboard.set_text(text);
                    }
                }
                process_manager::ProcessManagerAction::CloseDetail => {}
            }
        }
    }

    /// Open search on the active tab, focus the query field, and reveal the current match.
    pub(super) fn open_active_tab_search(&mut self) {
        let Some(term_arc) = self.active_terminal() else {
            return;
        };
        let lines = term_arc.lock().unwrap().search_lines();
        let reveal_line = {
            let Some(tab) = self.active_tab_mut() else {
                return;
            };
            let _effect = terminal_search::open_search(&mut tab.search, &lines);
            tab.search
                .current
                .and_then(|i| tab.search.matches.get(i).map(|m| m.line))
        };
        self.search_request_focus = true;
        self.search_field_owns_focus = true;
        if let Some(line) = reveal_line {
            term_arc.lock().unwrap().reveal_search_line(line);
        }
    }

    pub(super) fn apply_search_bar_effect(&mut self, effect: terminal_search::SearchBarEffect) {
        match effect {
            terminal_search::SearchBarEffect::Reveal(m) => {
                if let Some(term) = self.active_terminal() {
                    term.lock().unwrap().reveal_search_line(m.line);
                }
            }
            terminal_search::SearchBarEffect::FocusQuery => {
                self.search_request_focus = true;
                self.search_field_owns_focus = true;
            }
            terminal_search::SearchBarEffect::Closed => {
                self.search_request_focus = false;
                self.search_field_owns_focus = false;
            }
            terminal_search::SearchBarEffect::None => {}
        }
    }

    pub(super) fn reconcile_remote_monitors(&mut self) {
        let required = self.tab_manager.remote_monitor_requirements();
        let actions = reconcile_actions(&required, &self.remote_monitor_params);
        self.apply_remote_monitor_actions(actions);
    }

    pub(super) fn apply_remote_monitor_actions(&mut self, actions: RemoteMonitorReconcileActions) {
        for key in actions.stops {
            if let Some(handle) = self.remote_monitors.remove(&key) {
                handle.shutdown();
            }
            for state in self
                .process_managers
                .values_mut()
                .filter(|state| state.target() == &key)
            {
                state.clear_detail();
            }
            for state in self
                .network_details
                .values_mut()
                .filter(|state| state.target() == &key)
            {
                state.cancel_pending_refresh();
            }
            self.remote_monitor_generations.remove(&key);
            self.remote_monitor_params.remove(&key);
            remove_monitor_slots(&mut self.monitor_slots, std::slice::from_ref(&key));
            self.sidebar.remove_monitor_view(&key);
        }

        for (key, params) in actions.starts {
            let generation =
                next_remote_monitor_generation(&mut self.next_remote_monitor_generation);
            let proxy = self.proxy.clone();
            let started_params = params.clone();
            match remote_monitor::start_ssh_worker_with_sink(
                key.clone(),
                generation,
                params,
                move |event| {
                    proxy
                        .send_event(user_event_from_remote(event))
                        .map_err(|_| ())
                },
            ) {
                Ok(handle) => {
                    debug_assert_eq!(handle.generation(), generation);
                    self.remote_monitor_generations
                        .insert(key.clone(), generation);
                    self.remote_monitor_params
                        .insert(key.clone(), started_params);
                    self.remote_monitors.insert(key, handle);
                }
                Err(_) => log::warn!("[MONITOR] 启动远端监控 worker 失败"),
            }
        }
        prune_sidebar_monitor_views(&mut self.sidebar, &self.tab_manager);
    }

    pub(super) fn shutdown_remote_monitors(&mut self) {
        for (_, handle) in self.remote_monitors.drain() {
            handle.shutdown();
        }
        self.remote_monitor_params.clear();
        self.remote_monitor_generations.clear();
        self.monitor_slots
            .retain(|key, _| matches!(key, monitor::MonitorKey::Local));
    }

    pub(super) fn cancel_left_mouse_gesture(&mut self) {
        let pane_id = self.left_mouse_pane_id.take();
        if let Some(cell) = take_left_mouse_gesture_state(
            &mut self.left_mouse_gesture,
            &mut self.selection_drag_anchor,
        ) {
            if let Some(pane_id) = pane_id {
                self.send_mouse_event_to_pane(&pane_id, 0, cell.0, cell.1, false);
            }
        }
    }

    pub(super) fn clear_selection(&mut self) {
        clear_selection_state(
            &mut self.selection_start,
            &mut self.selection_end,
            &mut self.selection_drag_anchor,
        );
    }

    pub(super) fn reset_click_sequence(&mut self) {
        reset_click_sequence_state(
            &mut self.click_state,
            &mut self.last_click_time,
            &mut self.last_click_pos,
            Instant::now(),
        );
    }

    pub(super) fn prepare_for_active_tab_change(&mut self) {
        self.invalidate_completion_popup_snapshot();
        self.show_terminal_menu = false;
        self.pending_terminal_link = None;
        // Drop in-progress composition without committing when the active tab changes.
        self.clear_terminal_ime_composition();
        self.dragged_split = None;
        let active_idx = self.tab_manager.active_idx;
        cancel_pending_fill_for_tab(&mut self.tab_manager, active_idx);
        let release_cell = prepare_for_active_tab_change_state(
            &mut self.left_mouse_gesture,
            &mut self.selection_start,
            &mut self.selection_end,
            &mut self.selection_drag_anchor,
            (
                &mut self.click_state,
                &mut self.last_click_time,
                &mut self.last_click_pos,
            ),
            Instant::now(),
        );
        let pane_id = self.left_mouse_pane_id.take();
        if let Some(cell) = release_cell {
            if let Some(pane_id) = pane_id {
                self.send_mouse_event_to_pane(&pane_id, 0, cell.0, cell.1, false);
            }
        }
    }

    pub(super) fn switch_to_tab(&mut self, index: usize) {
        if index >= self.tab_manager.len() || index == self.tab_manager.active_idx {
            return;
        }
        self.prepare_for_active_tab_change();
        self.tab_manager.switch_to(index);
    }

    pub(super) fn terminal_link_at_cell(
        &self,
        cell: (usize, usize),
    ) -> Option<(String, terminal_links::TerminalLink)> {
        let tab = self.tab_manager.active()?;
        let allow_local_paths = matches!(tab.tab_type, TabType::Local { .. });
        let link = tab
            .terminal
            .lock()
            .ok()?
            .link_at_visual(cell.1, cell.0, allow_local_paths)?;
        Some((tab.id.clone(), link))
    }

    pub(super) fn activate_terminal_link(
        &self,
        link: &terminal_links::TerminalLink,
    ) -> Result<(), String> {
        let target = match &link.target {
            terminal_links::LinkTarget::Url(url) => std::path::PathBuf::from(url),
            terminal_links::LinkTarget::LocalPath { path, .. } => {
                let expanded = if let Some(rest) = path.strip_prefix("~/") {
                    dirs::home_dir()
                        .ok_or_else(|| "无法定位用户目录".to_string())?
                        .join(rest)
                } else {
                    std::path::PathBuf::from(path)
                };
                if !expanded.is_absolute() || !expanded.exists() {
                    return Err("链接路径不存在或不是绝对路径".into());
                }
                expanded
            }
        };
        std::process::Command::new("xdg-open")
            .arg(target)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("打开链接失败：{error}"))
    }

    pub(super) fn next_tab(&mut self) {
        if self.tab_manager.len() > 1 {
            self.prepare_for_active_tab_change();
            self.tab_manager.next_tab();
        }
    }

    pub(super) fn prev_tab(&mut self) {
        if self.tab_manager.len() > 1 {
            self.prepare_for_active_tab_change();
            self.tab_manager.prev_tab();
        }
    }

    pub(super) fn active_file_browser_height(&self) -> f32 {
        self.tab_manager
            .active()
            .and_then(|tab| self.file_browsers.get(&tab.id))
            .map_or(0.0, |state| file_browser::reserved_height(state.open))
    }

    pub(super) fn active_drag_upload_target(&self) -> Option<drag_upload::DropTarget> {
        let tab = self.tab_manager.active()?;
        if !matches!(tab.tab_type, TabType::Ssh { .. }) {
            return None;
        }
        let pane = tab.pane(tab.active_pane_id())?;
        if !pane.ssh_connected {
            return None;
        }
        Some(drag_upload::DropTarget {
            tab_id: tab.id.clone(),
            pane_id: pane.id().to_string(),
            session_generation: pane.completion.session().generation,
            session_token: pane.completion.session().token().to_string(),
            remote_directory: self
                .file_browsers
                .get(&tab.id)
                .map(|state| state.remote.path.clone()),
        })
    }

    pub(super) fn current_drop_session(
        &self,
        target: &drag_upload::DropTarget,
    ) -> Option<CompletionSessionKey> {
        let index = self.tab_manager.find_by_id(&target.tab_id)?;
        let tab = &self.tab_manager.tabs[index];
        if !matches!(tab.tab_type, TabType::Ssh { .. }) {
            return None;
        }
        let pane = tab.pane(&target.pane_id)?;
        let session = pane.completion.session();
        (pane.ssh_connected
            && session.generation == target.session_generation
            && session.token() == target.session_token)
            .then(|| session.clone())
    }

    pub(super) fn start_zmodem_drop(
        &mut self,
        target: &drag_upload::DropTarget,
        session: &CompletionSessionKey,
        paths: Vec<std::path::PathBuf>,
    ) {
        if let Err(error) = validate_zmodem_drop_paths(&paths) {
            self.zmodem_views
                .entry(target.pane_id.clone())
                .or_default()
                .show_send_error(error);
            return;
        }
        self.handle_zmodem_ui_actions(
            &target.tab_id,
            &target.pane_id,
            session,
            vec![zmodem::ui::ZmodemUiAction::StartSend { paths }],
        );
    }

    pub(super) fn dispatch_drag_upload(&mut self, batch: drag_upload::DropBatch) {
        let Some(session) = self.current_drop_session(&batch.target) else {
            self.terminal_notice =
                Some("拖拽上传目标已关闭、切换或重新连接，请重新拖入文件".into());
            return;
        };
        let pane_id = batch.target.pane_id.clone();
        let worker_matches = self.sftp_workers.get(&pane_id).is_some_and(|worker| {
            worker.tab_id() == batch.target.tab_id
                && worker.pane_id() == pane_id
                && worker.session() == &session
                && batch.target.remote_directory.is_some()
        });
        let sftp_ready = self
            .tab_manager
            .find_by_id(&batch.target.tab_id)
            .and_then(|index| self.tab_manager.tabs[index].pane(&pane_id))
            .is_some_and(|pane| pane.completion.sftp_ready());
        let zmodem_ready = self.zmodem_controls.get(&pane_id).is_some_and(|slot| {
            slot.tab_id == batch.target.tab_id
                && slot.pane_id == pane_id
                && slot.session == session
                && zmodem_ui_capability(
                    slot.capability,
                    self.settings.zmodem.enabled,
                    true,
                    slot.unavailable_reason.as_deref(),
                )
                .disabled_reason()
                .is_none()
        });

        match drag_upload::choose_transport(true, worker_matches, sftp_ready, zmodem_ready) {
            drag_upload::TransportChoice::Sftp => {
                let remote_directory = batch
                    .target
                    .remote_directory
                    .as_deref()
                    .expect("SFTP routing requires a captured remote directory");
                let jobs = match drag_upload::plan_sftp_jobs(&batch.paths, remote_directory) {
                    Ok(jobs) => jobs,
                    Err(error) => {
                        self.terminal_notice = Some(format!("拖拽上传失败：{error}"));
                        return;
                    }
                };
                let tracked = jobs
                    .into_iter()
                    .map(|job| {
                        let transfer_id = uuid::Uuid::new_v4().to_string();
                        let filename = job.local_path.file_name().map_or_else(
                            || job.local_path.to_string_lossy().into_owned(),
                            |name| name.to_string_lossy().into_owned(),
                        );
                        let request = sftp::SftpUploadRequest {
                            transfer_id: transfer_id.clone(),
                            local_path: job.local_path.to_string_lossy().into_owned(),
                            remote_path: job.remote_path,
                        };
                        (transfer_id, filename, request)
                    })
                    .collect::<Vec<_>>();
                let uploads = tracked
                    .iter()
                    .map(|(_, _, request)| request.clone())
                    .collect();
                let result = self
                    .sftp_workers
                    .get(&pane_id)
                    .ok_or_else(|| "SFTP worker 不存在".to_string())
                    .and_then(|worker| worker.send(sftp::SftpCommand::UploadBatch { uploads }));
                match result {
                    Ok(()) => {
                        if let Some(state) = self.file_browsers.get_mut(&batch.target.tab_id) {
                            for (transfer_id, filename, _) in tracked {
                                state.start_transfer(
                                    transfer_id.clone(),
                                    filename,
                                    sftp::TransferDirection::Upload,
                                );
                                self.drag_upload_transfer_ids.insert(transfer_id);
                            }
                        }
                    }
                    Err(error) if zmodem_ready => {
                        log::warn!("拖拽 SFTP 批次未入队，改用 ZMODEM：{error}");
                        self.start_zmodem_drop(&batch.target, &session, batch.paths);
                    }
                    Err(error) => {
                        self.terminal_notice = Some(format!("拖拽上传失败：{error}"));
                    }
                }
            }
            drag_upload::TransportChoice::Zmodem => {
                self.start_zmodem_drop(&batch.target, &session, batch.paths);
            }
            drag_upload::TransportChoice::Unavailable(reason) => {
                let reason = match reason {
                    drag_upload::TransportUnavailable::SshNotConnected => "SSH 会话尚未连接",
                    drag_upload::TransportUnavailable::NoReadyTransport => {
                        "SFTP 与 ZMODEM 上传通道均不可用"
                    }
                };
                self.terminal_notice = Some(format!("拖拽上传失败：{reason}"));
            }
        }
    }
}
