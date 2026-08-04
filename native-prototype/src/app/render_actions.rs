use super::*;

impl App {
    pub(super) fn handle_recording_dialog_action(
        &mut self,
        action: recording::RecordingDialogAction,
    ) {
        let recording::RecordingDialogAction::Confirm { mode, path } = action else {
            return;
        };
        match mode {
            recording::RecordingDialogMode::Start => {
                let Some(tab) = self.tab_manager.active() else {
                    self.terminal_notice = Some("当前没有可录制的终端".into());
                    return;
                };
                if matches!(tab.tab_type, TabType::Recording { .. }) {
                    self.terminal_notice = Some("录屏回放标签不能再次录制".into());
                    return;
                }
                let pane_id = tab.active_pane_id().to_string();
                let (cols, rows) = self.grid_size_for_tab_pane(&tab.id, &pane_id);
                match self.recordings.start(&pane_id, &path, cols, rows) {
                    Ok(()) => {
                        self.terminal_notice =
                            Some(format!("已开始录屏，停止后保存到：{}", path.display()));
                    }
                    Err(error) => self.terminal_notice = Some(error),
                }
            }
            recording::RecordingDialogMode::Playback => {
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let result = recording::load(&path);
                    let event = UserEvent::RecordingLoaded { path, result };
                    if let Err(winit::event_loop::EventLoopClosed(event)) = proxy.send_event(event)
                    {
                        cleanup_undelivered_user_event(event);
                    }
                });
            }
        }
    }

    pub(super) fn handle_playback_action(
        &mut self,
        tab_id: &str,
        action: recording::PlaybackAction,
    ) {
        if action == recording::PlaybackAction::None {
            return;
        }
        let terminal = self.tab_manager.find_by_id(tab_id).and_then(|index| {
            let tab = &self.tab_manager.tabs[index];
            matches!(tab.tab_type, TabType::Recording { .. }).then(|| tab.terminal.clone())
        });
        let (Some(terminal), Some(state)) = (terminal, self.recording_playbacks.get_mut(tab_id))
        else {
            return;
        };
        let mut terminal = terminal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match action {
            recording::PlaybackAction::None => {}
            recording::PlaybackAction::Toggle => state.toggle(&mut terminal),
            recording::PlaybackAction::Restart => state.restart(&mut terminal),
            recording::PlaybackAction::SetSpeed(speed) => state.set_speed(speed),
            recording::PlaybackAction::Seek(ratio) => state.seek_ratio(ratio, &mut terminal),
        }
    }

    pub(super) fn apply_settings_panel_action(
        &mut self,
        settings_action: settings_panel::SettingsPanelAction,
        window: &Window,
    ) {
        // 设置 Apply/Cancel：在后续长期 gpu 不可变借用之前处理
        match settings_action {
            settings_panel::SettingsPanelAction::None => {}
            settings_panel::SettingsPanelAction::Cancel => {
                // SettingsPanel::show 已 close
            }
            settings_panel::SettingsPanelAction::Apply(new_settings) => {
                let plan = plan_settings_apply(&self.settings, &new_settings);
                let persisted = persist_and_publish_zmodem_settings(
                    &self.zmodem_settings_source,
                    &new_settings,
                    plan.zmodem_changed,
                    settings::Settings::save,
                );
                if let Err(error) = persisted {
                    self.settings_panel.set_error(error);
                } else {
                    if plan.theme_changed {
                        if let Some(theme) =
                            themes::theme_by_name(&new_settings.terminal.color_scheme)
                        {
                            if let Some(renderer) = self.renderer.as_mut() {
                                renderer.set_theme(theme);
                            }
                        }
                    }
                    let font_changed = plan.font_family_changed || plan.font_size_changed;
                    if font_changed {
                        if let (Some(renderer), Some(gpu)) =
                            (self.renderer.as_mut(), self.gpu.as_ref())
                        {
                            renderer.set_font(
                                gpu,
                                &new_settings.terminal.font,
                                new_settings.terminal.font_size,
                            );
                        }
                    }
                    self.sidebar.width = new_settings.appearance.sidebar_width as f32;
                    self.sidebar.visible = new_settings.appearance.show_sidebar;
                    self.settings = new_settings;
                    self.settings_panel.mark_saved(&self.settings);
                    if font_changed {
                        self.sync_terminal_size();
                    }
                    window.request_redraw();
                }
            }
        }
    }

    pub(super) fn handle_terminal_context_action(&mut self, term_menu_action: Option<&str>) {
        // 处理终端右键菜单动作
        if let Some(action) = term_menu_action {
            match action {
                "copy" => {
                    let text = self.get_selection_text();
                    if !text.is_empty() {
                        if let Some(cb) = &mut self.clipboard {
                            let _ = cb.set_text(&text);
                        }
                    }
                }
                "paste" => {
                    let text = self
                        .clipboard
                        .as_mut()
                        .and_then(|clipboard| clipboard.get_text().ok());
                    if let Some(text) = text {
                        self.write_active_user_input(&text);
                    }
                }
                "select_all" => {
                    // 选中整个可见区域
                    if let Some(r) = &self.renderer {
                        let (cols, rows) = r.calculate_grid_size();
                        self.cancel_left_mouse_gesture();
                        let pane_id = self
                            .tab_manager
                            .active()
                            .map(|tab| tab.active_pane_id().to_string());
                        if let Some(pane_id) = pane_id {
                            self.selection_start =
                                self.visual_cell_to_selection_point_for_pane(&pane_id, (0, 0));
                            self.selection_end = self.visual_cell_to_selection_point_for_pane(
                                &pane_id,
                                (cols as usize - 1, rows as usize - 1),
                            );
                        }
                    }
                }
                "clear" => {
                    if let Some(terminal) = self.active_terminal() {
                        terminal.lock().unwrap().clear_display(false);
                    }
                    self.clear_selection();
                }
                "clear_scrollback" => {
                    if let Some(terminal) = self.active_terminal() {
                        terminal.lock().unwrap().clear_display(true);
                    }
                    self.clear_selection();
                }
                "search" => {
                    self.open_active_tab_search();
                }
                "theme" => {
                    self.cancel_left_mouse_gesture();
                    self.open_settings_tab();
                }
                "serial_reconnect" => {
                    let index = self.tab_manager.active_idx;
                    self.reconnect_serial_tab(index);
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                "serial_disconnect" => {
                    let index = self.tab_manager.active_idx;
                    self.disconnect_serial_tab(index);
                }
                "start_log" => {
                    let pane_id = self
                        .tab_manager
                        .active()
                        .filter(|tab| tab.tab_type.is_terminal())
                        .map(|tab| tab.active_pane_id().to_string());
                    if let Some(pane_id) = pane_id {
                        let default_name = format!(
                            "terminal_{}.txt",
                            chrono::Local::now().format("%Y%m%d_%H%M%S")
                        );
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title("选择日志保存位置")
                            .set_file_name(default_name)
                            .add_filter("文本日志", &["txt", "log"])
                            .save_file()
                        {
                            self.terminal_notice =
                                Some(match self.terminal_logs.start(&pane_id, &path) {
                                    Ok(()) => format!("已开始录制日志：{}", path.display()),
                                    Err(error) => error,
                                });
                        }
                    }
                }
                "stop_log" => {
                    let pane_id = self
                        .tab_manager
                        .active()
                        .map(|tab| tab.active_pane_id().to_string());
                    if let Some(pane_id) = pane_id {
                        self.terminal_notice = Some(match self.terminal_logs.stop(&pane_id) {
                            Ok(path) => format!("日志已保存到：{}", path.display()),
                            Err(error) => error,
                        });
                    }
                }
                "start_recording" => {
                    self.recording_dialog.open_start();
                }
                "stop_recording" => {
                    let pane_id = self
                        .tab_manager
                        .active()
                        .map(|tab| tab.active_pane_id().to_string());
                    if let Some(pane_id) = pane_id {
                        self.terminal_notice = Some(match self.recordings.stop(&pane_id) {
                            Ok(path) => format!("录屏已保存到：{}", path.display()),
                            Err(error) => error,
                        });
                    }
                }
                "play_recording" => {
                    self.recording_dialog.open_playback();
                }
                "split_h" => {
                    self.split_active_terminal(SplitDirection::Horizontal);
                }
                "split_v" => {
                    self.split_active_terminal(SplitDirection::Vertical);
                }
                "close_pane" => {
                    self.close_active_terminal_pane();
                }
                _ => {}
            }
        }
    }

    pub(super) fn handle_deferred_tab_action(&mut self, deferred_action: tab_bar::TabBarAction) {
        // Deferred tab actions (after gpu borrow ends)
        match deferred_action {
            tab_bar::TabBarAction::SwitchTo(idx) => {
                self.switch_to_tab(idx);
            }
            tab_bar::TabBarAction::Close(idx) => {
                self.close_tab(idx);
            }
            tab_bar::TabBarAction::CloseOthers(idx) => {
                self.close_other_tabs(idx);
            }
            tab_bar::TabBarAction::NewTab => {
                self.invalidate_completion_popup_snapshot();
                let generation = self.new_tab_selector.open();
                self.serial_scan(generation);
            }
            tab_bar::TabBarAction::OpenBatch => {
                self.cancel_left_mouse_gesture();
                self.batch_dialog.open(&self.batch_targets());
            }
            tab_bar::TabBarAction::OpenTunnels => {
                self.cancel_left_mouse_gesture();
                self.tunnel_manager.open();
            }
            tab_bar::TabBarAction::OpenSettings => {
                self.cancel_left_mouse_gesture();
                self.open_settings_tab();
            }
            tab_bar::TabBarAction::Reorder {
                dragged_id,
                target_id,
                placement,
            } => {
                self.tab_manager
                    .reorder_by_id(&dragged_id, &target_id, placement);
            }
            tab_bar::TabBarAction::Duplicate(idx) => {
                // 复制标签：新建同类型标签
                if let Some(tab) = self.tab_manager.tabs.get(idx) {
                    match &tab.tab_type {
                        TabType::Local { shell_path } => {
                            let shell = shell_path.clone();
                            self.new_local_tab_with_shell(&shell);
                        }
                        TabType::Ssh { label, params } => {
                            let source_tab_id = tab.id.clone();
                            let conn = sidebar::SshConnection {
                                label: label.clone(),
                                host: params.host.clone(),
                                port: params.port,
                                user: params.user.clone(),
                                auth: params.auth.clone(),
                                key_path: params.key_path.clone(),
                                password: params.password.clone(),
                                group: String::new(),
                                group_color: [0, 0, 0],
                            };
                            let new_tab_id = self.new_ssh_tab(&conn);
                            propagate_api_tab_credential_scope(
                                &mut self.api_ephemeral_tabs,
                                &source_tab_id,
                                &new_tab_id,
                            );
                        }
                        TabType::Process { .. }
                        | TabType::Network { .. }
                        | TabType::Serial { .. }
                        | TabType::Recording { .. }
                        | TabType::Settings => {}
                    }
                }
            }
            tab_bar::TabBarAction::Reconnect(idx) => {
                match self.tab_manager.tabs.get(idx).map(|tab| &tab.tab_type) {
                    Some(TabType::Ssh { .. }) => self.reconnect_ssh_tab(idx),
                    Some(TabType::Serial { .. }) => self.reconnect_serial_tab(idx),
                    _ => {}
                }
            }
            tab_bar::TabBarAction::Rename(idx) => {
                if let Some(tab) = self.tab_manager.tabs.get(idx) {
                    self.tab_rename_dialog
                        .open(tab.id.clone(), tab.label.clone());
                }
            }
            tab_bar::TabBarAction::ToggleMaximize => {
                if let Some(window) = &self.window {
                    window.set_maximized(!window.is_maximized());
                }
            }
            tab_bar::TabBarAction::MinimizeWindow => {
                if let Some(window) = &self.window {
                    window.set_minimized(true);
                }
            }
            tab_bar::TabBarAction::CloseWindow => {
                self.exit_requested = true;
                if let Some(window) = &self.window {
                    window.set_visible(false);
                }
            }
            _ => {}
        }
    }
}
