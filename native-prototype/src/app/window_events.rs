use super::*;

impl App {
    pub(super) fn handle_window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        match &event {
            WindowEvent::HoveredFile(_) => {
                let changed = if let Some(target) = self.active_drag_upload_target() {
                    self.drag_upload.hover(target)
                } else {
                    self.drag_upload.cancel_hover()
                };
                if changed {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                return;
            }
            WindowEvent::HoveredFileCancelled => {
                if self.drag_upload.cancel_hover() {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                return;
            }
            WindowEvent::DroppedFile(path) => {
                if let Some(target) = self.drag_upload.drop_target().cloned() {
                    if self.drag_upload.push_drop(target, path.clone())
                        == drag_upload::PushDropOutcome::RejectedDifferentTarget
                    {
                        self.terminal_notice =
                            Some("一次拖拽只能上传到同一个 SSH 终端，请重新拖入文件".into());
                    }
                } else {
                    self.drag_upload.cancel_hover();
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                return;
            }
            _ => {}
        }
        let keyboard_route = if let WindowEvent::KeyboardInput {
            event,
            is_synthetic,
            ..
        } = &event
        {
            Some(keyboard_input_route(event.state, *is_synthetic))
        } else {
            None
        };
        if keyboard_route == Some(KeyboardInputRoute::Drop) {
            return;
        }
        if let (
            Some(keyboard_route),
            WindowEvent::KeyboardInput {
                event: key_event, ..
            },
        ) = (keyboard_route, &event)
        {
            if selector_keyboard_input_scheduling(
                self.new_tab_selector.is_open(),
                keyboard_route,
                &key_event.logical_key,
            )
            .or_else(|| {
                rename_keyboard_input_scheduling(
                    self.tab_rename_dialog.is_open(),
                    keyboard_route,
                    &key_event.logical_key,
                )
            })
            .is_some()
            {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }

        let settings_panel_visible = self.is_settings_tab_active();
        let has_blocking_dialog = self.has_blocking_dialog();
        let has_dialog = terminal_input_blocked(settings_panel_visible, has_blocking_dialog)
            || completion_blocking_egui_overlay_visible(&self.egui_ctx);
        if has_dialog {
            self.invalidate_completion_popup_snapshot();
            self.pending_terminal_link = None;
        }
        let ime_owner = resolve_ime_input_owner(
            settings_panel_visible,
            has_blocking_dialog,
            completion_blocking_egui_overlay_visible(&self.egui_ctx),
            self.search_field_owns_focus,
            self.egui_ctx.wants_keyboard_input(),
        );
        self.sync_ime_owner(ime_owner);
        let ime_input_kind = match &event {
            WindowEvent::KeyboardInput { .. } => ImeOwnedInputKind::Keyboard,
            WindowEvent::ModifiersChanged(_) => ImeOwnedInputKind::ModifiersChanged,
            WindowEvent::Ime(_) => ImeOwnedInputKind::Ime,
            _ => ImeOwnedInputKind::Other,
        };
        if terminal_preedit_blocks_input(ime_owner, self.ime.has_active_preedit(), ime_input_kind) {
            return;
        }

        if keyboard_route == Some(KeyboardInputRoute::App)
            && !has_dialog
            && self
                .tab_manager
                .active()
                .is_some_and(|tab| tab.tab_type.is_terminal())
            && matches!(&event, WindowEvent::KeyboardInput { event, .. }
                if is_primary_find_shortcut(&event.logical_key, self.modifiers.state()))
        {
            self.open_active_tab_search();
            self.do_render();
            return;
        }

        match &event {
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let logical_position = physical_to_egui_position(
                    self.mouse_position,
                    self.egui_ctx.pixels_per_point(),
                );
                self.pending_window_drag_origin = (!has_dialog
                    && self.tab_drag_state.title_drag_contains(logical_position))
                .then_some(self.mouse_position);
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.pending_window_drag_origin = None;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let current = (position.x, position.y);
                if self
                    .pending_window_drag_origin
                    .is_some_and(|origin| window_drag_threshold_reached(origin, current, 3.0))
                {
                    self.pending_window_drag_origin = None;
                    self.mouse_position = current;
                    if let Some(window) = &self.window {
                        match window.drag_window() {
                            Ok(()) => return,
                            Err(error) => log::warn!("启动窗口拖拽失败: {error}"),
                        }
                    }
                }
            }
            WindowEvent::Focused(false) => {
                self.pending_window_drag_origin = None;
            }
            _ => {}
        }
        if keyboard_route == Some(KeyboardInputRoute::App) {
            if let WindowEvent::KeyboardInput { event, .. } = &event {
                let completion_surface_safe = self
                    .tab_manager
                    .active()
                    .filter(|tab| tab.tab_type.is_terminal())
                    .and_then(|tab| tab.terminal.lock().ok())
                    .is_some_and(|terminal| terminal.completion_surface_safe());
                if !completion_surface_safe {
                    self.invalidate_completion_popup_snapshot();
                }
                let popup_snapshot = completion_surface_safe
                    .then(|| {
                        current_completion_popup_snapshot(
                            &self.tab_manager,
                            &self.completion_popup_snapshot,
                        )
                        .cloned()
                    })
                    .flatten();
                let fill_pending = self
                    .tab_manager
                    .active()
                    .is_some_and(|tab| tab.completion.fill_pending());
                let action = completion_key_action(
                    &event.logical_key,
                    self.modifiers.state(),
                    popup_snapshot.is_some(),
                    fill_pending,
                    self.egui_ctx.wants_keyboard_input(),
                    has_dialog,
                    self.search_field_owns_focus,
                );
                match action {
                    CompletionKeyAction::Previous | CompletionKeyAction::Next => {
                        self.invalidate_completion_popup_snapshot();
                        if let Some(tab) =
                            self.tab_manager.tabs.get_mut(self.tab_manager.active_idx)
                        {
                            let delta = if action == CompletionKeyAction::Previous {
                                -1
                            } else {
                                1
                            };
                            tab.completion.move_selection(delta);
                        }
                        self.do_render();
                        return;
                    }
                    CompletionKeyAction::Dismiss => {
                        self.invalidate_completion_popup_snapshot();
                        if let Some(tab) =
                            self.tab_manager.tabs.get_mut(self.tab_manager.active_idx)
                        {
                            tab.completion.dismiss();
                        }
                        self.do_render();
                        return;
                    }
                    CompletionKeyAction::Accept => {
                        self.invalidate_completion_popup_snapshot();
                        let selection = popup_snapshot
                            .as_ref()
                            .and_then(completion_snapshot_selection);
                        if let Some((tab_id, pane_id, candidate)) = selection {
                            if let Err(error) =
                                self.stage_completion_fill(&tab_id, &pane_id, &candidate)
                            {
                                log::warn!("Bash 补全填充暂存失败: {error}");
                            }
                        }
                        self.do_render();
                        return;
                    }
                    CompletionKeyAction::PassThrough => {}
                }
            }
        }

        let active_left_release = matches!(
            &event,
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            }
        ) && self.left_mouse_gesture.is_some();

        // Tab 键不传给 egui（egui 用 Tab 切焦点会收起侧边栏）
        // 只拦截 Pressed 的 Tab，Released 不处理
        let is_tab_key = keyboard_route == Some(KeyboardInputRoute::App)
            && matches!(
                &event,
                WindowEvent::KeyboardInput { event, .. }
                    if matches!(event.logical_key, Key::Named(NamedKey::Tab))
            );

        let mut egui_consumed = false;

        // Tab 键在没有弹窗时跳过 egui，直接给终端
        if is_tab_key && !has_dialog {
            // 不传给 egui，直接走下面的 KeyboardInput 处理
        } else {
            // Pass events to egui first
            if let Some(egui_state) = &mut self.egui_state {
                if let Some(window) = &self.window {
                    let response = egui_state.on_window_event(window, &event);
                    if response.consumed {
                        egui_consumed = true;
                        match consumed_event_frame_scheduling(
                            matches!(
                                event,
                                WindowEvent::MouseInput {
                                    button: MouseButton::Right,
                                    ..
                                }
                            ),
                            matches!(event, WindowEvent::MouseWheel { .. }),
                            matches!(event, WindowEvent::CursorMoved { .. }),
                        ) {
                            InputFrameScheduling::RenderNow => self.do_render(),
                            InputFrameScheduling::RequestRedraw => window.request_redraw(),
                        }
                        self.check_ssh_connect();
                        // 滚轮/指针与键盘事件即使 egui 消费了也可能继续传给终端
                        // MouseWheel/MouseInput 经 should_pass_pointer_to_terminal 门控 modal
                        // 键盘经 should_pass_keyboard_to_terminal 判定
                        let in_terminal =
                            self.is_in_terminal(self.mouse_position.0, self.mouse_position.1);
                        // 检查 egui 是否有文本框获焦（命令输入栏/弹窗输入框）
                        let egui_wants_keyboard = self.egui_ctx.wants_keyboard_input();
                        let pass_through = if matches!(event, WindowEvent::MouseWheel { .. }) {
                            should_pass_pointer_to_terminal(
                                settings_panel_visible,
                                has_blocking_dialog,
                                in_terminal,
                            )
                        } else if matches!(event, WindowEvent::MouseInput { .. }) {
                            // helper 门控 modal（in_terminal=true 仅取 modal 语义）；
                            // consumed 普通点击不二次投递，仅 active left release 可穿透收尾
                            should_pass_pointer_to_terminal(
                                settings_panel_visible,
                                has_blocking_dialog,
                                true,
                            ) && should_pass_consumed_mouse_input(in_terminal, active_left_release)
                        } else if matches!(event, WindowEvent::KeyboardInput { .. }) {
                            should_pass_keyboard_to_terminal(
                                settings_panel_visible,
                                has_blocking_dialog,
                                egui_wants_keyboard,
                                self.search_field_owns_focus,
                            )
                        } else if matches!(event, WindowEvent::Ime(_)) {
                            // Always continue to our IME state machine after egui saw
                            // the event (TextEdits still receive IME; no double-feed).
                            true
                        } else if matches!(event, WindowEvent::ScaleFactorChanged { .. }) {
                            // egui updates its scale first; App still must resize all
                            // pane grids even when the platform emits no Resized event.
                            true
                        } else {
                            false
                        };
                        if !pass_through {
                            // modal 阻断 MouseInput 时取消手势，避免 stale left_mouse_gesture
                            if matches!(event, WindowEvent::MouseInput { .. })
                                && terminal_input_blocked(
                                    settings_panel_visible,
                                    has_blocking_dialog,
                                )
                            {
                                self.cancel_left_mouse_gesture();
                            }
                            return;
                        }
                    }
                }
            }
        } // end else (Tab key bypass)

        match event {
            WindowEvent::CloseRequested => {
                self.save_workspace_session();
                if let Some(mut server) = self.api_server.take() {
                    server.stop();
                }
                self.shutdown_all_zmodem();
                for tab in &self.tab_manager.tabs {
                    if tab.tab_type.is_terminal() {
                        for pane in tab.panes() {
                            pane.terminal
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .shutdown();
                        }
                    }
                }
                for worker in self.sftp_workers.values() {
                    let _ = worker.send(sftp::SftpCommand::Shutdown);
                }
                self.shutdown_remote_monitors();
                self.tunnel_registry.close_all();
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                debug_assert_eq!(
                    plan_window_geometry_update(WindowGeometryEventKind::Resized),
                    WindowGeometryUpdatePlan::ResizeSurfaceAndSyncAndRender,
                );
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
                self.sync_terminal_size();
                self.do_render();
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                debug_assert_eq!(
                    plan_window_geometry_update(WindowGeometryEventKind::ScaleFactorChanged),
                    WindowGeometryUpdatePlan::SyncAndRender,
                );
                self.egui_ctx
                    .set_pixels_per_point(sanitize_pixels_per_point(scale_factor as f32));
                if let (Some(window), Some(gpu)) = (&self.window, &mut self.gpu) {
                    let size = window.inner_size();
                    gpu.resize(size.width, size.height);
                }
                self.sync_terminal_size();
                self.do_render();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods;
            }

            WindowEvent::Focused(focused) => {
                self.window_focused = focused;
                if focused {
                    // 窗口获得焦点后重置修饰键状态（避免 Alt+Tab 残留）
                    self.modifiers = Modifiers::default();
                    // Native file dialogs temporarily disable platform IME.
                    // Restore it immediately when focus returns to a terminal;
                    // the requested redraw also refreshes the cursor area.
                    let terminal_owns_ime =
                        self.resolve_current_ime_owner() == ime::InputOwner::Terminal;
                    if let Some(window) = &self.window {
                        if terminal_owns_ime {
                            window.set_ime_allowed(true);
                        }
                        window.request_redraw();
                    }
                } else {
                    // Drop composition without committing on focus loss.
                    self.clear_terminal_ime_composition();
                    let clear_selection =
                        should_clear_selection_on_focus_loss(self.left_mouse_gesture);
                    self.cancel_left_mouse_gesture();
                    self.pending_terminal_link = None;
                    self.reset_click_sequence();
                    if clear_selection {
                        self.clear_selection();
                    }
                }
            }

            WindowEvent::Ime(ime_event) => {
                // egui already received this event above (when not Tab-bypassed).
                // Update our state machine and write Terminal commits exactly once.
                self.handle_ime_event(ime_event);
            }

            WindowEvent::MouseInput { state, button, .. } => {
                // modal 打开时阻断指针进终端，并取消已有 left_mouse_gesture
                if terminal_input_blocked(settings_panel_visible, has_blocking_dialog) {
                    self.cancel_left_mouse_gesture();
                    self.dragged_split = None;
                    return;
                }
                let pointer =
                    physical_to_egui_position(self.mouse_position, self.pixels_per_point());
                let hovered_pane_id = self.pane_id_at(self.mouse_position.0, self.mouse_position.1);
                let in_terminal = hovered_pane_id.is_some();
                let mouse_mode = hovered_pane_id
                    .as_deref()
                    .is_some_and(|pane_id| self.is_mouse_mode_for_pane(pane_id));
                if let Some(frame_action) = open_terminal_menu_mouse_press_gate(
                    self.show_terminal_menu,
                    state,
                    button,
                    mouse_mode,
                ) {
                    self.show_terminal_menu = false;
                    self.terminal_menu_ignore_pointer_press_once = false;
                    debug_assert_eq!(frame_action, InputFrameScheduling::RequestRedraw);
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                    return;
                }
                if button == MouseButton::Left && state == ElementState::Released {
                    if self.dragged_split.take().is_some() {
                        if let Some(window) = &self.window {
                            window.set_cursor(CursorIcon::Default);
                        }
                        return;
                    }
                }
                if button == MouseButton::Left && state == ElementState::Pressed {
                    if let Some(divider) = self.pane_layout.divider_at(pointer) {
                        self.cancel_left_mouse_gesture();
                        if let Some(tab_id) = self.tab_manager.active().map(|tab| tab.id.clone()) {
                            self.dragged_split = Some(DraggedSplit {
                                tab_id,
                                split_id: divider.split_id,
                            });
                        }
                        if let Some(window) = &self.window {
                            window.set_cursor(match divider.direction {
                                SplitDirection::Horizontal => CursorIcon::RowResize,
                                SplitDirection::Vertical => CursorIcon::ColResize,
                            });
                        }
                        return;
                    }
                }
                if state == ElementState::Pressed {
                    if let Some(pane_id) = hovered_pane_id.as_deref() {
                        self.focus_pane(pane_id);
                    }
                }
                if button == MouseButton::Left && state == ElementState::Released {
                    let gesture_pane_id = self.left_mouse_pane_id.take();
                    if let Some(pending) = self.pending_terminal_link.take() {
                        if !terminal_link_modifier_active(self.modifiers.state()) {
                            return;
                        }
                        let current = gesture_pane_id.as_deref().and_then(|pane_id| {
                            self.pixel_to_cell_for_pane(
                                pane_id,
                                self.mouse_position.0,
                                self.mouse_position.1,
                            )
                        });
                        if current
                            .and_then(|cell| self.terminal_link_at_cell(cell))
                            .as_ref()
                            == Some(&pending)
                        {
                            if let Err(error) = self.activate_terminal_link(&pending.1) {
                                log::warn!("{error}");
                            }
                        }
                        return;
                    }
                    if !should_process_left_release(in_terminal, self.left_mouse_gesture.is_some())
                    {
                        return;
                    }

                    let gesture = self.left_mouse_gesture.take();
                    let current_cell = gesture_pane_id.as_deref().and_then(|pane_id| {
                        self.pixel_to_cell_for_pane(
                            pane_id,
                            self.mouse_position.0,
                            self.mouse_position.1,
                        )
                    });
                    let terminal_release_cell = terminal_report_release_cell(gesture, current_cell);
                    let copy_selection = should_copy_left_selection(
                        gesture,
                        self.selection_start,
                        self.selection_end,
                    );
                    self.selection_drag_anchor = None;

                    if let Some(cell) = terminal_release_cell {
                        if let Some(pane_id) = gesture_pane_id.as_deref() {
                            self.send_mouse_event_to_pane(pane_id, 0, cell.0, cell.1, false);
                        }
                    }
                    if copy_selection {
                        self.copy_selection();
                    }
                    return;
                }

                if !in_terminal {
                    return;
                }

                let shift = self.modifiers.state().shift_key();
                let pane_id = hovered_pane_id.expect("terminal hit must identify a pane");
                let cell = self
                    .pixel_to_cell_for_pane(&pane_id, self.mouse_position.0, self.mouse_position.1)
                    .unwrap_or((0, 0));

                if button == MouseButton::Left {
                    if state == ElementState::Pressed {
                        if terminal_link_modifier_active(self.modifiers.state()) {
                            self.cancel_left_mouse_gesture();
                            self.left_mouse_pane_id = Some(pane_id.clone());
                            self.pending_terminal_link = self.terminal_link_at_cell(cell);
                            return;
                        }
                        self.cancel_left_mouse_gesture();
                        self.left_mouse_pane_id = Some(pane_id.clone());
                        let gesture = left_mouse_gesture(mouse_mode, shift, cell);
                        self.left_mouse_gesture = Some(gesture);
                        if matches!(gesture, LeftMouseGesture::TerminalReport { .. }) {
                            self.selection_drag_anchor = None;
                            self.send_mouse_event_to_pane(&pane_id, 0, cell.0, cell.1, true);
                            return;
                        }

                        self.selection_start = None;
                        self.selection_end = None;
                        let now = Instant::now();
                        let elapsed = now.duration_since(self.last_click_time).as_millis();
                        let same_pos = cell == self.last_click_pos;
                        self.click_state =
                            click_state_after_press(self.click_state, elapsed, same_pos);
                        match self.click_state {
                            ClickState::Double => {
                                self.selection_drag_anchor = None;
                                self.select_word(cell.0, cell.1);
                            }
                            ClickState::Triple => {
                                self.selection_drag_anchor = None;
                                self.select_line(cell.1);
                            }
                            _ => {
                                self.selection_start = None;
                                self.selection_end = None;
                                self.selection_drag_anchor =
                                    self.visual_cell_to_selection_point_for_pane(&pane_id, cell);
                            }
                        }
                        self.last_click_time = now;
                        self.last_click_pos = cell;
                        self.do_render();
                    }
                }

                if button == MouseButton::Middle && state == ElementState::Pressed {
                    if mouse_mode && !shift {
                        self.send_mouse_event_to_pane(&pane_id, 1, cell.0, cell.1, true);
                    } else {
                        let text = self
                            .clipboard
                            .as_mut()
                            .and_then(|clipboard| clipboard.get_text().ok());
                        if let Some(text) = text {
                            self.write_active_user_input(&text);
                        }
                    }
                    self.do_render();
                }

                if button == MouseButton::Right && state == ElementState::Pressed {
                    let transition = terminal_context_menu_press_transition(
                        self.mouse_position,
                        self.egui_ctx.pixels_per_point(),
                    );
                    let window = self.window.clone();
                    apply_terminal_context_menu_transition(
                        &mut self.show_terminal_menu,
                        &mut self.terminal_menu_pos,
                        &mut self.terminal_menu_ignore_pointer_press_once,
                        transition,
                        move || {
                            if let Some(window) = &window {
                                window.request_redraw();
                            }
                        },
                    );
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.handle_cursor_moved(position, settings_panel_visible, has_blocking_dialog)
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.pending_terminal_link = None;
                let hovered_pane_id = self.pane_id_at(self.mouse_position.0, self.mouse_position.1);
                let in_term = hovered_pane_id.is_some();
                if !should_pass_pointer_to_terminal(
                    settings_panel_visible,
                    has_blocking_dialog,
                    in_term,
                ) {
                    self.terminal_wheel_accumulator.reset();
                    self.terminal_wheel_pane_id = None;
                    return;
                }
                let pane_id = hovered_pane_id.expect("terminal hit must identify a pane");
                if self.terminal_wheel_pane_id.as_ref() != Some(&pane_id) {
                    self.terminal_wheel_accumulator.reset();
                    self.terminal_wheel_pane_id = Some(pane_id.clone());
                }
                let lines = self.terminal_wheel_accumulator.scroll_lines(&delta);
                log::trace!(
                    "[SCROLL] delta={:?} lines={} in_term={} mouse=({:.0},{:.0}) egui_consumed={}",
                    delta,
                    lines,
                    in_term,
                    self.mouse_position.0,
                    self.mouse_position.1,
                    egui_consumed
                );
                if lines == 0 {
                    return;
                }
                let local_selection_active = matches!(
                    self.left_mouse_gesture,
                    Some(LeftMouseGesture::LocalSelection)
                ) && self.left_mouse_pane_id.as_ref()
                    == Some(&pane_id);
                let mouse_mode = self.is_mouse_mode_for_pane(&pane_id);
                if mouse_mode && !local_selection_active {
                    let cell = self
                        .pixel_to_cell_for_pane(
                            &pane_id,
                            self.mouse_position.0,
                            self.mouse_position.1,
                        )
                        .unwrap_or((0, 0));
                    let btn = if lines > 0 { 64 } else { 65 };
                    for _ in 0..lines.unsigned_abs() {
                        self.send_mouse_event_to_pane(&pane_id, btn, cell.0, cell.1, true);
                    }
                } else if let Some(terminal) = self.terminal_for_pane(&pane_id) {
                    let mut term = terminal.lock().unwrap();
                    let mut changed = false;
                    let current_cell = self.pixel_to_cell_for_pane(
                        &pane_id,
                        self.mouse_position.0,
                        self.mouse_position.1,
                    );
                    let mut selection_point = None;
                    if let Some(t) = term.term_mut() {
                        use alacritty_terminal::grid::Scroll;
                        let before = t.grid().display_offset();
                        t.scroll_display(Scroll::Delta(lines));
                        let after = t.grid().display_offset();
                        changed = before != after;
                        use alacritty_terminal::grid::Dimensions;
                        let history = t
                            .grid()
                            .total_lines()
                            .saturating_sub(t.grid().screen_lines());
                        log::trace!(
                            "[SCROLL] display_offset: {} → {} (history={}, lines={})",
                            before,
                            after,
                            history,
                            lines
                        );
                    }
                    if changed {
                        term.mark_render_dirty();
                        if local_selection_active {
                            selection_point =
                                current_cell.and_then(|cell| term.visual_point_to_grid_point(cell));
                        }
                    }
                    drop(term);
                    if let Some(point) = selection_point {
                        if let Some((start, end)) =
                            drag_selection_range(self.selection_drag_anchor, point)
                        {
                            self.selection_start = Some(start);
                            self.selection_end = Some(end);
                        }
                    }
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if keyboard_route != Some(KeyboardInputRoute::App) {
                    return;
                }
                // NOTE: Do NOT scroll-to-bottom here. Search open/nav/typing and
                // shortcuts must not yank the viewport. Only terminal-directed
                // keyboard input (below the pass gate) scrolls to bottom.

                let ctrl = self.modifiers.state().control_key();
                let shift = self.modifiers.state().shift_key();
                let alt = self.modifiers.state().alt_key();
                let egui_wants_keyboard = self.egui_ctx.wants_keyboard_input();
                let search_owns = self.search_field_owns_focus;

                // Egui normally consumes these while TextEdit is focused. This fallback
                // covers the frame after clicking a search-bar button, without PTY input.
                if let Some(action) = search_keyboard_fallback_action(
                    &event.logical_key,
                    shift,
                    search_owns,
                    has_dialog,
                ) {
                    let effect = self
                        .active_tab_mut()
                        .map(|tab| {
                            terminal_search::apply_search_bar_action(&mut tab.search, action)
                        })
                        .unwrap_or(terminal_search::SearchBarEffect::None);
                    self.apply_search_bar_effect(effect);
                    self.do_render();
                    return;
                }

                // Search shortcut: allow even when the search field owns focus
                // (re-open / re-focus). Other shortcuts require no egui grab.
                if !has_dialog {
                    if let Some(action) = self
                        .settings
                        .shortcuts
                        .match_action(&event.logical_key, self.modifiers.state())
                    {
                        let allow = matches!(action, shortcuts::ShortcutAction::Search)
                            || !egui_wants_keyboard;
                        if allow {
                            match action {
                                shortcuts::ShortcutAction::NewTab => {
                                    self.invalidate_completion_popup_snapshot();
                                    let generation = self.new_tab_selector.open();
                                    self.serial_scan(generation);
                                    if let Some(window) = &self.window {
                                        window.request_redraw();
                                    }
                                    return;
                                }
                                shortcuts::ShortcutAction::CloseTab => {
                                    let idx = self.tab_manager.active_idx;
                                    self.close_tab(idx);
                                    self.do_render();
                                    return;
                                }
                                shortcuts::ShortcutAction::Copy => {
                                    self.copy_selection();
                                    return;
                                }
                                shortcuts::ShortcutAction::Paste => {
                                    let text = self
                                        .clipboard
                                        .as_mut()
                                        .and_then(|clipboard| clipboard.get_text().ok());
                                    if let Some(text) = text {
                                        self.scroll_active_terminal_to_bottom();
                                        self.write_active_user_input(&text);
                                    }
                                    self.do_render();
                                    return;
                                }
                                shortcuts::ShortcutAction::NextTab => {
                                    self.next_tab();
                                    self.do_render();
                                    return;
                                }
                                shortcuts::ShortcutAction::PreviousTab => {
                                    self.prev_tab();
                                    self.do_render();
                                    return;
                                }
                                shortcuts::ShortcutAction::Search => {
                                    self.open_active_tab_search();
                                    self.do_render();
                                    return;
                                }
                            }
                        }
                    }
                }

                // Ctrl+1~9 switch to tab N（保留直达标签行为）
                if ctrl && !shift && !has_dialog && !egui_wants_keyboard && !search_owns {
                    if let PhysicalKey::Code(code) = event.physical_key {
                        let tab_num = match code {
                            KeyCode::Digit1 => Some(0),
                            KeyCode::Digit2 => Some(1),
                            KeyCode::Digit3 => Some(2),
                            KeyCode::Digit4 => Some(3),
                            KeyCode::Digit5 => Some(4),
                            KeyCode::Digit6 => Some(5),
                            KeyCode::Digit7 => Some(6),
                            KeyCode::Digit8 => Some(7),
                            KeyCode::Digit9 => Some(8),
                            _ => None,
                        };
                        if let Some(n) = tab_num {
                            if n < self.tab_manager.len() {
                                self.switch_to_tab(n);
                                self.do_render();
                                return;
                            }
                        }
                    }
                }

                // 设置面板 / 侧栏弹窗 / egui 抢键 / 搜索框获焦时绝不落入 PTY
                if !should_pass_keyboard_to_terminal(
                    settings_panel_visible,
                    has_blocking_dialog,
                    egui_wants_keyboard,
                    search_owns,
                ) {
                    return;
                }

                // Modifier presses alter subsequent keys only. They must not cancel a mouse
                // selection or yank a scrollback viewport back to the live prompt.
                if is_modifier_only_key(&event.logical_key) {
                    return;
                }

                // Ctrl+letter → control character
                if ctrl && !shift {
                    if let PhysicalKey::Code(code) = event.physical_key {
                        let ctrl_byte = match code {
                            KeyCode::KeyA => 0x01,
                            KeyCode::KeyB => 0x02,
                            KeyCode::KeyC => 0x03,
                            KeyCode::KeyD => 0x04,
                            KeyCode::KeyE => 0x05,
                            KeyCode::KeyF => 0x06,
                            KeyCode::KeyG => 0x07,
                            KeyCode::KeyH => 0x08,
                            KeyCode::KeyI => 0x09,
                            KeyCode::KeyJ => 0x0a,
                            KeyCode::KeyK => 0x0b,
                            KeyCode::KeyL => 0x0c,
                            KeyCode::KeyM => 0x0d,
                            KeyCode::KeyN => 0x0e,
                            KeyCode::KeyO => 0x0f,
                            KeyCode::KeyP => 0x10,
                            KeyCode::KeyQ => 0x11,
                            KeyCode::KeyR => 0x12,
                            KeyCode::KeyS => 0x13,
                            KeyCode::KeyT => 0x14,
                            KeyCode::KeyU => 0x15,
                            KeyCode::KeyV => 0x16,
                            KeyCode::KeyW => 0x17,
                            KeyCode::KeyX => 0x18,
                            KeyCode::KeyY => 0x19,
                            KeyCode::KeyZ => 0x1a,
                            _ => 0u8,
                        };
                        if ctrl_byte > 0 {
                            self.prepare_for_terminal_user_input();
                            match ctrl_terminal_input_action(ctrl_byte) {
                                CtrlTerminalInputAction::Submit => {
                                    self.submit_active_bash_line();
                                }
                                CtrlTerminalInputAction::Write(character) => {
                                    self.write_active_user_input(&character.to_string());
                                }
                            }
                            self.do_render();
                            return;
                        }
                    }
                }

                if matches!(event.logical_key, Key::Named(NamedKey::Enter)) {
                    self.prepare_for_terminal_user_input();
                    self.submit_active_bash_line();
                    self.do_render();
                    return;
                }

                // Special keys
                let esc = match event.logical_key {
                    Key::Named(NamedKey::Backspace) => Some(terminal_backspace_sequence(alt)),
                    Key::Named(NamedKey::Tab) => Some("\t"),
                    Key::Named(NamedKey::Escape) => Some("\x1b"),
                    Key::Named(NamedKey::ArrowUp) => Some("\x1b[A"),
                    Key::Named(NamedKey::ArrowDown) => Some("\x1b[B"),
                    Key::Named(NamedKey::ArrowRight) => Some("\x1b[C"),
                    Key::Named(NamedKey::ArrowLeft) => Some("\x1b[D"),
                    Key::Named(NamedKey::Home) => Some("\x1b[H"),
                    Key::Named(NamedKey::End) => Some("\x1b[F"),
                    Key::Named(NamedKey::PageUp) => Some("\x1b[5~"),
                    Key::Named(NamedKey::PageDown) => Some("\x1b[6~"),
                    Key::Named(NamedKey::Delete) => Some("\x1b[3~"),
                    Key::Named(NamedKey::Insert) => Some("\x1b[2~"),
                    Key::Named(NamedKey::F1) => Some("\x1bOP"),
                    Key::Named(NamedKey::F2) => Some("\x1bOQ"),
                    Key::Named(NamedKey::F3) => Some("\x1bOR"),
                    Key::Named(NamedKey::F4) => Some("\x1bOS"),
                    Key::Named(NamedKey::F5) => Some("\x1b[15~"),
                    Key::Named(NamedKey::F6) => Some("\x1b[17~"),
                    Key::Named(NamedKey::F7) => Some("\x1b[18~"),
                    Key::Named(NamedKey::F8) => Some("\x1b[19~"),
                    Key::Named(NamedKey::F9) => Some("\x1b[20~"),
                    Key::Named(NamedKey::F10) => Some("\x1b[21~"),
                    Key::Named(NamedKey::F11) => Some("\x1b[23~"),
                    Key::Named(NamedKey::F12) => Some("\x1b[24~"),
                    _ => None,
                };

                if let Some(seq) = esc {
                    self.prepare_for_terminal_user_input();
                    self.write_active_user_input(seq);
                } else if let Some(text) = &event.text {
                    // Filter only raw text through IME echo/preedit suppression.
                    // Named control / shortcut sequences above are unchanged.
                    if !text.as_str().is_empty() {
                        if let Some(filtered) = self.ime.filter_keyboard_text(text.as_str()) {
                            if !filtered.is_empty() {
                                self.prepare_for_terminal_user_input();
                                self.write_active_user_input(&filtered);
                            }
                        }
                    }
                }
                self.do_render();
            }

            WindowEvent::RedrawRequested => {
                self.do_render();
                self.check_ssh_connect();
            }
            _ => {}
        }
    }
}
