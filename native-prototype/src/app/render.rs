use super::*;
use crate::tab_manager::PaneStatus;

impl App {
    pub(super) fn do_render(&mut self) {
        self.invalidate_completion_popup_snapshot();
        let completion_popup_frame_epoch = self.completion_popup_epoch;
        let elapsed = self.cursor_timer.elapsed().as_millis();
        if elapsed >= 530 {
            self.cursor_visible = !self.cursor_visible;
            self.cursor_timer = Instant::now();
        }

        if self.gpu.is_none() {
            return;
        }
        let window = match &self.window {
            Some(w) => w.clone(),
            None => return,
        };

        let active_playback = self.tab_manager.active().and_then(|tab| {
            matches!(tab.tab_type, TabType::Recording { .. })
                .then(|| (tab.id.clone(), tab.terminal.clone()))
        });
        if let Some((tab_id, terminal)) = active_playback.as_ref() {
            if let Some(playback) = self.recording_playbacks.get_mut(tab_id) {
                let mut terminal = terminal
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                playback.tick(&mut terminal);
            }
        }

        let active_input = if self.active_zmodem_has_overlay() {
            None
        } else {
            self.tab_manager
                .tabs
                .get_mut(self.tab_manager.active_idx)
                .filter(|tab| tab.tab_type.is_terminal())
                .and_then(|tab| {
                    let terminal = tab.terminal.clone();
                    terminal.lock().ok().and_then(|mut terminal| {
                        completion_input_for_render(
                            &mut tab.completion,
                            &mut terminal,
                            Instant::now(),
                        )
                    })
                })
        };
        refresh_active_completion(&mut self.tab_manager, active_input.as_deref());

        self.refresh_pane_layout();
        let active_pane_rect = self
            .tab_manager
            .active()
            .and_then(|tab| self.pane_layout.pane(tab.active_pane_id()))
            .map(|pane| pane.rect);
        if let (Some(gpu), Some(renderer)) = (&self.gpu, &mut self.renderer) {
            if let Some(rect) = active_pane_rect {
                let rect = logical_to_physical_pane_rect(rect, self.egui_ctx.pixels_per_point());
                renderer.set_viewport(rect.x, rect.y, rect.width, rect.height, gpu);
            }
        }
        let completion_popup_blocked = self.is_settings_tab_active()
            || self.has_blocking_dialog()
            || completion_blocking_egui_overlay_visible(&self.egui_ctx);
        let completion_popup_view = self
            .tab_manager
            .active()
            .filter(|tab| tab.tab_type.is_terminal())
            .and_then(|tab| {
                let renderer = self.renderer.as_ref()?;
                let pixels_per_point = self.pixels_per_point();
                let bounds = physical_to_logical_rect(renderer.viewport_rect(), pixels_per_point);
                let cursor = tab
                    .terminal
                    .lock()
                    .ok()
                    .and_then(|terminal| renderer.cursor_screen_rect(&terminal))
                    .map(|rect| physical_to_logical_rect(rect, pixels_per_point));
                completion_popup::CompletionPopupSnapshot::new_for_pane(
                    tab.id.clone(),
                    tab.active_pane_id().to_string(),
                    tab.completion.session().clone(),
                    completion_popup_blocked || !tab.completion.is_popup_visible(),
                    bounds,
                    cursor,
                    tab.completion.candidates().to_vec(),
                    tab.completion.selected(),
                )
            });
        let mut rendered_completion_popup_snapshot = None;

        // 1. Run egui (tab bar + sidebar + dialogs)
        let mut egui_input = self.egui_state.as_mut().unwrap().take_egui_input(&window);
        normalize_ui_wheel_events(&mut egui_input.events);
        let mut tab_action = tab_bar::TabBarAction::None;
        let mut tab_rename_request = None;
        let mut term_menu_action: Option<&str> = None;
        let mut cmd_bar_action: Option<String> = None;
        let active_sftp_identity = self.tab_manager.active().and_then(|tab| {
            matches!(&tab.tab_type, TabType::Ssh { .. })
                .then(|| (tab.id.clone(), tab.active_pane_id().to_string()))
        });
        let active_sftp_tab = active_sftp_identity
            .as_ref()
            .map(|(tab_id, _)| tab_id.clone());
        let active_monitor_key = self.tab_manager.active_monitor_key();
        let active_monitor_data =
            active_monitor_snapshot(&self.monitor_slots, &active_monitor_key).cloned();
        let active_monitor_error = active_monitor_slot(&self.monitor_slots, &active_monitor_key)
            .and_then(|slot| slot.error.clone());
        let active_process_tab = self.tab_manager.active().and_then(|tab| {
            matches!(&tab.tab_type, TabType::Process { .. })
                .then(|| (tab.id.clone(), tab.monitor_key()))
        });
        let active_network_tab = self.tab_manager.active().and_then(|tab| {
            matches!(&tab.tab_type, TabType::Network { .. })
                .then(|| (tab.id.clone(), tab.monitor_key()))
        });
        let active_settings_tab = self.is_settings_tab_active();
        if let Some((tab_id, _)) = active_network_tab.as_ref() {
            if let (Some(state), Some(data)) = (
                self.network_details.get_mut(tab_id),
                active_monitor_data.as_ref(),
            ) {
                state.update_rates(&data.net_interfaces);
            }
        }
        let mut file_actions = Vec::new();
        let mut process_actions = Vec::new();
        let mut network_actions = Vec::new();
        let mut batch_action = batch_command::BatchCommandAction::None;
        let mut tunnel_action = tunnel_manager::TunnelManagerAction::None;
        let batch_targets = self.batch_targets();
        let tunnel_connections = self.sidebar.connections.clone();
        let tunnel_infos = self.tunnel_registry.infos();
        let mut settings_action = settings_panel::SettingsPanelAction::None;
        let mut new_tab_action = new_tab_selector::NewTabAction::None;
        let mut recording_dialog_action = recording::RecordingDialogAction::None;
        let mut playback_action = recording::PlaybackAction::None;
        let mut zmodem_actions = Vec::new();
        let mut zmodem_overlay_visible = self.active_zmodem_has_overlay();
        let mut selector_connections = Vec::new();
        // Snapshot IME preedit overlay inputs before the egui run closure so we
        // do not call &self methods (or hold terminal locks) inside it.
        let preedit_settings_visible = active_settings_tab;
        let preedit_has_blocking_dialog = self.has_blocking_dialog();
        let preedit_search_owns = self.search_field_owns_focus;
        let preedit_text = self.ime.preedit_text().to_owned();
        let preedit_cursor_viewport = if !preedit_text.is_empty() {
            self.renderer.as_ref().and_then(|renderer| {
                let pixels_per_point = self.pixels_per_point();
                let terminal = self.active_terminal()?;
                let term = terminal.lock().ok()?;
                let cursor =
                    physical_to_logical_rect(renderer.cursor_screen_rect(&term)?, pixels_per_point);
                let viewport = physical_to_logical_rect(renderer.viewport_rect(), pixels_per_point);
                Some((cursor, viewport))
            })
        } else {
            None
        };
        let pane_layout_overlay = self.pane_layout.clone();
        let drag_upload_state = self.drag_upload.clone();
        let drag_upload_overlay = self.active_drag_upload_target().and_then(|target| {
            pane_layout_overlay
                .pane(&target.pane_id)
                .map(|pane| (target, pane.rect))
        });
        let active_zmodem_overlay = self
            .tab_manager
            .active()
            .filter(|tab| {
                tab.tab_type.is_terminal() && !matches!(tab.tab_type, TabType::Recording { .. })
            })
            .and_then(|tab| {
                let pane_id = tab.active_pane_id().to_string();
                let pane = tab.pane(&pane_id)?;
                let pane_rect = pane_layout_overlay.pane(&pane_id)?.rect;
                Some((
                    tab.id.clone(),
                    pane_id,
                    pane.completion.session().clone(),
                    pane_rect,
                ))
            });
        let split_actions_enabled = self
            .tab_manager
            .active()
            .is_some_and(|tab| matches!(tab.tab_type, TabType::Local { .. } | TabType::Ssh { .. }));
        let (horizontal_split_label, vertical_split_label) =
            terminal_split_menu_labels(split_actions_enabled);
        let active_recording = self
            .tab_manager
            .active()
            .map(|tab| self.recordings.is_recording(tab.active_pane_id()))
            .unwrap_or(false);
        let active_logging = self
            .tab_manager
            .active()
            .map(|tab| self.terminal_logs.is_logging(tab.active_pane_id()))
            .unwrap_or(false);
        let active_serial_menu_state = self.tab_manager.active().and_then(|tab| {
            matches!(tab.tab_type, TabType::Serial { .. }).then(|| match &tab.status {
                PaneStatus::Connecting => SerialTerminalMenuState::Connecting,
                PaneStatus::Connected => SerialTerminalMenuState::Connected,
                PaneStatus::Idle | PaneStatus::Failed(_) => SerialTerminalMenuState::Disconnected,
            })
        });
        let active_serial_failure = self.tab_manager.active().and_then(|tab| {
            let PaneStatus::Failed(error) = &tab.status else {
                return None;
            };
            matches!(tab.tab_type, TabType::Serial { .. }).then(|| {
                let pane_rect = pane_layout_overlay.pane(tab.active_pane_id())?.rect;
                Some((self.tab_manager.active_idx, pane_rect, error.clone()))
            })?
        });
        let mut retry_failed_serial = None;
        let active_playback_snapshot = active_playback.as_ref().and_then(|(tab_id, _)| {
            self.recording_playbacks
                .get(tab_id)
                .map(recording::PlaybackSnapshot::from)
        });
        let terminal_notice = self.terminal_notice.clone();
        let mut dismiss_terminal_notice = false;
        let egui_ctx = self.egui_ctx.clone();
        let egui_output = egui_ctx.run(egui_input, |ctx| {
            let pane_painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new("terminal_pane_chrome"),
            ));
            for divider in &pane_layout_overlay.dividers {
                pane_painter.rect_filled(
                    divider.rect,
                    0.0,
                    egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                );
            }
            if let Some((target, pane_rect)) = drag_upload_overlay.as_ref() {
                drag_upload::render_active_pane_overlay(
                    ctx,
                    &drag_upload_state,
                    target,
                    *pane_rect,
                );
            }
            if let Some(message) = terminal_notice.as_deref() {
                egui::Window::new("终端面板")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .show(ctx, |ui| {
                        ui.label(message);
                        if ui.button("知道了").clicked() {
                            dismiss_terminal_notice = true;
                        }
                    });
            }
            // Sidebar first (claims left space)
            self.sidebar_width = self.sidebar.ui_with_monitor(
                ctx,
                &active_monitor_key,
                active_monitor_data.as_ref(),
                active_monitor_error.as_deref(),
            );
            if self.new_tab_selector.is_open() {
                selector_connections = self.sidebar.connections.clone();
            }
            // 设置页占用完整内容高度；终端快捷命令栏仅对工作标签显示。
            if active_settings_tab {
                self.command_bar_height = 0.0;
            } else {
                self.command_bar_height = command_bar::COMMAND_BAR_HEIGHT;
                cmd_bar_action = self.command_bar.ui(ctx, self.sidebar_width);
            }
            if let Some(tab_id) = active_sftp_tab.as_ref() {
                if let Some(state) = self.file_browsers.get_mut(tab_id) {
                    file_actions = file_browser::render(ctx, state);
                }
            }
            tab_action = tab_bar::render_tab_bar(ctx, &self.tab_manager, &mut self.tab_drag_state);
            tab_rename_request = self.tab_rename_dialog.render(ctx);
            if active_settings_tab {
                settings_action = self.settings_panel.show_page(ctx);
            }
            if let Some((tab_id, key)) = active_process_tab.as_ref() {
                let state = self
                    .process_managers
                    .entry(tab_id.clone())
                    .or_insert_with(|| process_manager::ProcessManagerState::new(key.clone()));
                process_actions = process_manager::render(
                    ctx,
                    state,
                    active_monitor_data
                        .as_ref()
                        .map(|data| data.processes.as_slice()),
                    active_monitor_data
                        .as_ref()
                        .map(|data| data.zombie_processes.as_slice()),
                    active_monitor_data.as_ref().map(|data| &data.process_stats),
                    active_monitor_error.as_deref(),
                );
            }
            if let Some((tab_id, _)) = active_network_tab.as_ref() {
                if let Some(state) = self.network_details.get_mut(tab_id) {
                    network_actions = network_detail::render(ctx, state);
                }
            }
            if let Some((_, pane_id, _, pane_rect)) = active_zmodem_overlay.as_ref() {
                let view = self.zmodem_views.entry(pane_id.clone()).or_default();
                zmodem_actions = zmodem::ui::render(ctx, pane_id, *pane_rect, view);
                zmodem_overlay_visible = view.has_overlay();
            }
            batch_action = self.batch_dialog.show(ctx, &batch_targets);
            tunnel_action = self
                .tunnel_manager
                .show(ctx, &tunnel_connections, &tunnel_infos);

            let sidebar_modal_visible = blocking_dialog_visible(
                self.sidebar.password_prompt.is_some()
                    || self.sidebar.show_new_connection
                    || self.sidebar.show_key_manager,
                self.new_tab_selector.is_open(),
            );
            if !zmodem_overlay_visible
                && completion_popup_may_render(
                    ctx,
                    completion_popup_view.is_some(),
                    sidebar_modal_visible,
                )
            {
                if let Some(snapshot) = completion_popup_view.as_ref() {
                    completion_popup::render(ctx, snapshot);
                    rendered_completion_popup_snapshot = Some(snapshot.clone());
                }
            }

            self.render_connection_context_menu(ctx);
            self.render_ssh_password_prompt(ctx);
            if let Some((index, pane_rect, error)) = active_serial_failure.as_ref() {
                if self.render_serial_failure_card(ctx, *pane_rect, error) {
                    retry_failed_serial = Some(*index);
                }
            }
            term_menu_action = self.render_terminal_context_menu(
                ctx,
                horizontal_split_label,
                vertical_split_label,
                split_actions_enabled,
                active_logging,
                active_recording,
                active_serial_menu_state,
            );

            // 终端搜索条：标签栏下方、视口右上角（对齐 main TerminalPane 样式）
            {
                let search_visible = self
                    .tab_manager
                    .active()
                    .map(|t| t.search.visible)
                    .unwrap_or(false);
                if search_visible {
                    let mut query_changed = false;
                    let mut case_changed = false;
                    let mut nav_action: Option<terminal_search::SearchBarKeyAction> = None;
                    let mut close_clicked = false;
                    let mut field_focused = false;
                    let mut selected_history: Option<String> = None;
                    let search_y = self.tab_bar_height + 8.0;
                    egui::Area::new(egui::Id::new("terminal_search_bar"))
                        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, search_y))
                        .order(egui::Order::Foreground)
                        .show(ctx, |ui| {
                            egui::Frame::new()
                                .fill(egui::Color32::from_rgba_unmultiplied(
                                    0x16, 0x1b, 0x22, 0xf2,
                                ))
                                .stroke(egui::Stroke::new(
                                    1.0,
                                    egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                                ))
                                .corner_radius(6.0)
                                .inner_margin(egui::Margin::symmetric(8, 4))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 4.0;

                                        let status = self
                                            .tab_manager
                                            .active()
                                            .map(|t| t.search.status_text())
                                            .unwrap_or_else(|| "0/0".into());

                                        // Query field — mutable through active tab
                                        let query_id = self
                                            .tab_manager
                                            .active()
                                            .map(|tab| terminal_search_query_id(&tab.id))
                                            .unwrap_or_else(|| {
                                                terminal_search_query_id("no-active-tab")
                                            });
                                        let mut query_buf = self
                                            .tab_manager
                                            .active()
                                            .map(|t| t.search.query.clone())
                                            .unwrap_or_default();
                                        let te = egui::TextEdit::singleline(&mut query_buf)
                                            .id(query_id)
                                            .desired_width(180.0)
                                            .hint_text("搜索...")
                                            .font(egui::TextStyle::Body)
                                            .text_color(egui::Color32::from_rgb(0xe6, 0xed, 0xf3))
                                            .frame(true);
                                        let resp = ui.add(te);
                                        if self.search_request_focus {
                                            resp.request_focus();
                                            self.search_request_focus = false;
                                        }
                                        field_focused = resp.has_focus();
                                        if resp.changed() {
                                            let idx = self.tab_manager.active_idx;
                                            if let Some(tab) = self.tab_manager.tabs.get_mut(idx) {
                                                tab.search.query = query_buf;
                                            }
                                            query_changed = true;
                                        }

                                        let history = self
                                            .tab_manager
                                            .active()
                                            .map(|tab| tab.search.history.clone())
                                            .unwrap_or_default();
                                        ui.menu_button(
                                            egui::RichText::new("⌄")
                                                .size(14.0)
                                                .color(egui::Color32::from_rgb(0x8b, 0x94, 0x9e)),
                                            |ui| {
                                                ui.set_min_width(240.0);
                                                ui.label(
                                                    egui::RichText::new("搜索历史")
                                                        .size(12.0)
                                                        .color(egui::Color32::from_rgb(
                                                            0x8b, 0x94, 0x9e,
                                                        )),
                                                );
                                                ui.separator();
                                                if history.is_empty() {
                                                    ui.add_enabled(
                                                        false,
                                                        egui::Label::new("暂无历史记录"),
                                                    );
                                                } else {
                                                    egui::ScrollArea::vertical()
                                                        .max_height(220.0)
                                                        .animated(false)
                                                        .show(ui, |ui| {
                                                            for entry in &history {
                                                                if ui
                                                                    .add_sized(
                                                                        egui::vec2(232.0, 26.0),
                                                                        egui::Button::new(entry)
                                                                            .frame(false),
                                                                    )
                                                                    .on_hover_text(entry)
                                                                    .clicked()
                                                                {
                                                                    selected_history =
                                                                        Some(entry.clone());
                                                                    ui.close_menu();
                                                                }
                                                            }
                                                        });
                                                }
                                            },
                                        )
                                        .response
                                        .on_hover_text("选择历史搜索项");

                                        // Status current/total
                                        ui.label(
                                            egui::RichText::new(status)
                                                .size(12.0)
                                                .color(egui::Color32::from_rgb(0x8b, 0x94, 0x9e)),
                                        );

                                        // Case toggle
                                        let mut case_sensitive = self
                                            .tab_manager
                                            .active()
                                            .map(|t| t.search.case_sensitive)
                                            .unwrap_or(false);
                                        let case_btn = ui
                                            .add(
                                                egui::Button::new(
                                                    egui::RichText::new("Aa").size(11.0).color(
                                                        if case_sensitive {
                                                            egui::Color32::from_rgb(
                                                                0x00, 0xd4, 0xff,
                                                            )
                                                        } else {
                                                            egui::Color32::from_rgb(
                                                                0x8b, 0x94, 0x9e,
                                                            )
                                                        },
                                                    ),
                                                )
                                                .fill(egui::Color32::TRANSPARENT)
                                                .stroke(egui::Stroke::new(
                                                    1.0,
                                                    egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                                                )),
                                            )
                                            .on_hover_text("区分大小写");
                                        if case_btn.clicked() {
                                            case_sensitive = !case_sensitive;
                                            let idx = self.tab_manager.active_idx;
                                            if let Some(tab) = self.tab_manager.tabs.get_mut(idx) {
                                                tab.search.case_sensitive = case_sensitive;
                                            }
                                            case_changed = true;
                                        }

                                        let btn_style =
                                            |label: &str| {
                                                egui::Button::new(
                                                    egui::RichText::new(label).size(11.0).color(
                                                        egui::Color32::from_rgb(0x8b, 0x94, 0x9e),
                                                    ),
                                                )
                                                .fill(egui::Color32::TRANSPARENT)
                                                .stroke(egui::Stroke::new(
                                                    1.0,
                                                    egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                                                ))
                                            };

                                        if ui
                                            .add(btn_style("上一个"))
                                            .on_hover_text("上一个 (Shift+Enter)")
                                            .clicked()
                                        {
                                            nav_action =
                                                Some(terminal_search::SearchBarKeyAction::Previous);
                                        }
                                        if ui
                                            .add(btn_style("下一个"))
                                            .on_hover_text("下一个 (Enter)")
                                            .clicked()
                                        {
                                            nav_action =
                                                Some(terminal_search::SearchBarKeyAction::Next);
                                        }
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    egui::RichText::new("×").size(14.0).color(
                                                        egui::Color32::from_rgb(0x8b, 0x94, 0x9e),
                                                    ),
                                                )
                                                .fill(egui::Color32::TRANSPARENT)
                                                .frame(false),
                                            )
                                            .on_hover_text("关闭 (Esc)")
                                            .clicked()
                                        {
                                            close_clicked = true;
                                        }

                                        // Enter / Shift+Enter / Escape while search owns focus
                                        if field_focused || resp.has_focus() {
                                            let (enter, escape, shift) = ui.input(|i| {
                                                (
                                                    i.key_pressed(egui::Key::Enter),
                                                    i.key_pressed(egui::Key::Escape),
                                                    i.modifiers.shift,
                                                )
                                            });
                                            if enter {
                                                nav_action =
                                                    Some(terminal_search::search_bar_key_action(
                                                        terminal_search::SearchBarKey::Enter,
                                                        shift,
                                                    ));
                                            }
                                            if escape {
                                                nav_action =
                                                    Some(terminal_search::search_bar_key_action(
                                                        terminal_search::SearchBarKey::Escape,
                                                        false,
                                                    ));
                                            }
                                        }
                                    });
                                });
                        });

                    // The visible search bar owns terminal keyboard routing even if a
                    // button click briefly moves egui focus away from the TextEdit.
                    self.search_field_owns_focus = true;

                    if let Some(query) = selected_history {
                        let idx = self.tab_manager.active_idx;
                        if let Some(tab) = self.tab_manager.tabs.get_mut(idx) {
                            tab.search.query = query;
                            tab.search.remember_query();
                        }
                        query_changed = true;
                        self.search_request_focus = true;
                    }

                    if query_changed || case_changed {
                        // Recompute using terminal snapshot. Avoid methods that borrow
                        // all of App (closure already holds other fields).
                        let lines = self
                            .tab_manager
                            .active_terminal()
                            .map(|t| t.lock().unwrap().search_lines())
                            .unwrap_or_default();
                        let idx = self.tab_manager.active_idx;
                        let effect = self
                            .tab_manager
                            .tabs
                            .get_mut(idx)
                            .map(|tab| terminal_search::recompute_search(&mut tab.search, &lines))
                            .unwrap_or(terminal_search::SearchBarEffect::None);
                        if let terminal_search::SearchBarEffect::Reveal(m) = effect {
                            if let Some(term) = self.tab_manager.active_terminal() {
                                term.lock().unwrap().reveal_search_line(m.line);
                            }
                        }
                    }

                    if close_clicked {
                        nav_action = Some(terminal_search::SearchBarKeyAction::Close);
                    }
                    if let Some(action) = nav_action {
                        let idx = self.tab_manager.active_idx;
                        let effect = self
                            .tab_manager
                            .tabs
                            .get_mut(idx)
                            .map(|tab| {
                                terminal_search::apply_search_bar_action(&mut tab.search, action)
                            })
                            .unwrap_or(terminal_search::SearchBarEffect::None);
                        match effect {
                            terminal_search::SearchBarEffect::Reveal(m) => {
                                if let Some(term) = self.tab_manager.active_terminal() {
                                    term.lock().unwrap().reveal_search_line(m.line);
                                }
                            }
                            terminal_search::SearchBarEffect::Closed => {
                                self.search_request_focus = false;
                                self.search_field_owns_focus = false;
                            }
                            _ => {}
                        }
                    }
                } else if !self.search_request_focus {
                    // Search bar hidden and no pending open — clear focus gate
                    self.search_field_owns_focus = false;
                }
            }

            // 设置加载警告 banner（非 modal）：在设置面板之前渲染
            if let Some(warning) = self.settings_load_warning.as_ref() {
                let mut dismiss = false;
                egui::Window::new("设置提示")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_TOP, [0.0, 48.0])
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        ui.set_max_width(480.0);
                        ui.label(
                            egui::RichText::new(warning.as_str())
                                .color(egui::Color32::from_rgb(0xe6, 0xa2, 0x3c)),
                        );
                        ui.add_space(8.0);
                        if ui.button("知道了").clicked() {
                            dismiss = true;
                        }
                    });
                if dismiss {
                    self.settings_load_warning.take();
                }
            }

            // 新标签选择器使用与动作解析相同的当前连接快照。
            new_tab_action = self.new_tab_selector.show(ctx, &selector_connections);

            recording_dialog_action = self.recording_dialog.render(ctx);

            if let Some(snapshot) = active_playback_snapshot {
                playback_action = recording::render_playback_controls(
                    ctx,
                    self.sidebar_width,
                    self.command_bar_height,
                    snapshot,
                );
            }

            // Terminal IME preedit overlay — visual only; never touches grid/PTY.
            // Owner flags (except ctx-backed ones) and cursor geometry are captured
            // before this closure to keep borrows short and avoid &mut self conflicts.
            let ime_owner = resolve_ime_input_owner(
                preedit_settings_visible,
                preedit_has_blocking_dialog,
                completion_blocking_egui_overlay_visible(ctx),
                preedit_search_owns,
                ctx.wants_keyboard_input(),
            );
            if ime_owner == ime::InputOwner::Terminal && !preedit_text.is_empty() {
                if let Some((cursor, viewport)) = preedit_cursor_viewport {
                    // Clamp origin inside the terminal viewport (and thus the window).
                    let max_x = (viewport.max.x - 4.0).max(viewport.min.x);
                    let max_y = (viewport.max.y - 4.0).max(viewport.min.y);
                    let pos = egui::pos2(
                        cursor.min.x.clamp(viewport.min.x, max_x),
                        cursor.min.y.clamp(viewport.min.y, max_y),
                    );
                    egui::Area::new(egui::Id::new("terminal_ime_preedit"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(pos)
                        .interactable(false)
                        .show(ctx, |ui| {
                            egui::Frame::new()
                                .fill(egui::Color32::from_rgba_unmultiplied(0, 0, 0, 180))
                                .inner_margin(egui::Margin::symmetric(4, 2))
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new(preedit_text.as_str())
                                            .monospace()
                                            .underline()
                                            .color(egui::Color32::from_rgb(0xf0, 0xf0, 0xf0)),
                                    );
                                });
                        });
                }
            }
        });
        let tab_renamed = tab_rename_request
            .map(|request| {
                self.tab_manager
                    .rename(&request.tab_id, request.label.as_str())
            })
            .unwrap_or(false);
        if dismiss_terminal_notice {
            self.terminal_notice = None;
        }
        if active_settings_tab || self.has_blocking_dialog() {
            rendered_completion_popup_snapshot = None;
        }

        self.egui_state
            .as_mut()
            .unwrap()
            .handle_platform_output(&window, egui_output.platform_output);

        // Context actions may open a synchronous native file dialog. Handle
        // them before the terminal IME reassertion so focus/IME state is
        // restored after the dialog returns, not immediately before it opens.
        self.handle_terminal_context_action(term_menu_action);
        if let Some(index) = retry_failed_serial {
            self.reconnect_serial_tab(index);
        }

        // After egui may have toggled IME for TextEdits, reassert terminal IME
        // when the terminal owns input and the window is focused.
        let ime_owner = self.resolve_current_ime_owner();
        self.sync_ime_owner(ime_owner);
        if should_reassert_terminal_ime(ime_owner, self.window_focused) {
            window.set_ime_allowed(true);
            let cursor_area = {
                let ppp = self.egui_ctx.pixels_per_point();
                self.renderer.as_ref().and_then(|renderer| {
                    let terminal = self.active_terminal()?;
                    let term = terminal.lock().ok()?;
                    let rect = physical_to_logical_rect(renderer.cursor_screen_rect(&term)?, ppp);
                    Some(logical_to_physical_ime_cursor_area(
                        rect.min.x,
                        rect.min.y,
                        rect.width(),
                        rect.height(),
                        ppp,
                    ))
                })
            };
            if let Some(area) = cursor_area {
                window.set_ime_cursor_area(
                    winit::dpi::PhysicalPosition::new(area.x, area.y),
                    winit::dpi::PhysicalSize::new(area.width, area.height),
                );
            }
        }

        self.apply_settings_panel_action(settings_action, &window);
        self.handle_recording_dialog_action(recording_dialog_action);
        if let Some((tab_id, _)) = active_playback.as_ref() {
            self.handle_playback_action(tab_id, playback_action);
        }

        if let Some((tab_id, pane_id, session, _)) = active_zmodem_overlay.as_ref() {
            self.handle_zmodem_ui_actions(tab_id, pane_id, session, zmodem_actions);
        }

        // 处理命令栏点击
        if let Some(cmd) = cmd_bar_action.filter(|_| !self.active_zmodem_has_overlay()) {
            self.write_active_user_input(&cmd);
        }

        let frame_pixels_per_point = sanitize_pixels_per_point(egui_output.pixels_per_point);
        let paint_jobs = self
            .egui_ctx
            .tessellate(egui_output.shapes, frame_pixels_per_point);

        // 2. Keep the tab/popup/terminal snapshot consistent for the whole presented frame.
        let frame_action_timing = tab_action_frame_timing(&tab_action);
        let deferred_action = tab_action;

        let gpu = match &self.gpu {
            Some(g) => g,
            None => return,
        };
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [gpu.width, gpu.height],
            pixels_per_point: frame_pixels_per_point,
        };

        // 3. Get surface texture
        let output = match gpu.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Lost) => {
                if let Some(g) = &mut self.gpu {
                    g.resize(g.width, g.height);
                }
                return;
            }
            Err(e) => {
                log::warn!("Surface error: {:?}", e);
                return;
            }
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Main Encoder"),
            });

        // 4. Clear surface
        {
            let bg = self
                .renderer
                .as_ref()
                .map(|r| r.palette().background)
                .unwrap_or([0, 0, 0, 255]);
            let _clear_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: bg[0] as f64 / 255.0,
                            g: bg[1] as f64 / 255.0,
                            b: bg[2] as f64 / 255.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        // 5. Render every pane in the active terminal tab.
        let pane_render_items = self
            .tab_manager
            .active()
            .filter(|tab| tab.tab_type.is_terminal())
            .map(|tab| {
                self.pane_layout
                    .panes
                    .iter()
                    .filter_map(|viewport| {
                        let pane = tab.pane(&viewport.pane_id)?;
                        let search = (pane.search.visible && !pane.search.query.is_empty())
                            .then(|| (pane.search.matches.clone(), pane.search.current));
                        Some((
                            viewport.pane_id.clone(),
                            viewport.rect,
                            pane.terminal.clone(),
                            search,
                            viewport.pane_id == tab.active_pane_id(),
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some(renderer) = &mut self.renderer {
            renderer.begin_pane_frame();
            for (_pane_id, rect, terminal, search_hl_owned, is_active) in pane_render_items {
                let pane_rect = logical_to_physical_pane_rect(rect, frame_pixels_per_point);
                let (cols, rows) = renderer.calculate_grid_size_for_rect(pane_rect);
                let search_hl =
                    search_hl_owned
                        .as_ref()
                        .map(|(matches, current)| renderer::SearchHighlights {
                            matches: matches.as_slice(),
                            current: *current,
                        });
                {
                    let mut term = terminal
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    term.resize(cols, rows);
                    renderer.prepare_pane_draw(
                        gpu,
                        &_pane_id,
                        pane_rect,
                        &term,
                        self.cursor_visible && is_active,
                        is_active.then_some(self.selection_start).flatten(),
                        is_active.then_some(self.selection_end).flatten(),
                        search_hl,
                    );
                } // Release the terminal mutex before encoding the shared GPU pass.
            }
            renderer.render_prepared_panes(gpu, &view, &mut encoder);
        }

        // 6. Render egui overlay
        let egui_renderer = self.egui_renderer.as_mut().unwrap();
        for (id, delta) in &egui_output.textures_delta.set {
            egui_renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
        let _cmd_bufs = egui_renderer.update_buffers(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &paint_jobs,
            &screen_descriptor,
        );

        {
            let egui_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Egui Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let egui_pass_static: wgpu::RenderPass<'static> = egui_pass.forget_lifetime();
            let mut egui_pass_static = egui_pass_static;
            egui_renderer.render(&mut egui_pass_static, &paint_jobs, &screen_descriptor);
        }

        for id in &egui_output.textures_delta.free {
            egui_renderer.free_texture(id);
        }

        gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        publish_completion_popup_snapshot(
            &mut self.completion_popup_snapshot,
            rendered_completion_popup_snapshot,
            true,
            completion_popup_frame_epoch,
            self.completion_popup_epoch,
        );
        self.last_render_time = Instant::now();

        self.handle_deferred_tab_action(deferred_action);
        if self.handle_new_tab_selector_action(new_tab_action, &selector_connections) {
            window.request_redraw();
        }
        if tab_renamed {
            window.request_redraw();
        }
        if matches!(
            frame_action_timing,
            FrameActionTiming::AfterPresent {
                request_redraw: true
            }
        ) {
            window.request_redraw();
        }
        if let Some((tab_id, pane_id)) = active_sftp_identity {
            for action in file_actions {
                self.handle_file_browser_action(&tab_id, &pane_id, action);
            }
        }
        if let Some((tab_id, key)) = active_process_tab {
            self.handle_process_manager_actions(&tab_id, &key, process_actions);
        }
        if let Some((tab_id, key)) = active_network_tab {
            self.handle_network_detail_actions(&tab_id, &key, network_actions);
        }
        match batch_action {
            batch_command::BatchCommandAction::Execute { command, tab_ids } => {
                self.execute_batch(command, tab_ids);
            }
            batch_command::BatchCommandAction::None | batch_command::BatchCommandAction::Close => {}
        }
        match tunnel_action {
            tunnel_manager::TunnelManagerAction::Create(spec) => {
                if let Err(error) = self.tunnel_registry.start(spec) {
                    self.tunnel_manager.set_error(error);
                }
            }
            tunnel_manager::TunnelManagerAction::Close(id) => {
                if !self.tunnel_registry.close(id) {
                    let _ = self.tunnel_registry.remove_finished(id);
                }
            }
            tunnel_manager::TunnelManagerAction::None
            | tunnel_manager::TunnelManagerAction::Dismiss => {}
        }
        if let Some(action) = self.sidebar.take_open_process_manager() {
            self.open_process_manager(action.key);
            window.request_redraw();
        }
        if let Some(action) = self.sidebar.take_open_network_detail() {
            self.open_network_detail(action.key, action.initial_iface);
            window.request_redraw();
        }
        let _ = self.tunnel_registry.poll();
    }
}
