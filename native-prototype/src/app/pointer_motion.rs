use super::*;

impl App {
    pub(super) fn handle_cursor_moved(
        &mut self,
        position: winit::dpi::PhysicalPosition<f64>,
        settings_panel_visible: bool,
        has_blocking_dialog: bool,
    ) {
        self.mouse_position = (position.x, position.y);
        let process_terminal_motion =
            match terminal_pointer_motion_action(settings_panel_visible, has_blocking_dialog) {
                TerminalPointerMotionAction::Process => true,
                TerminalPointerMotionAction::BlockAndCancelGesture => {
                    self.left_mouse_gesture = None;
                    self.left_mouse_pane_id = None;
                    self.dragged_split = None;
                    self.selection_auto_scroll_lines = 0;
                    false
                }
            };
        if process_terminal_motion {
            let active_tab_id = self.tab_manager.active().map(|tab| tab.id.as_str());
            if let Some(split_id) = active_dragged_split(self.dragged_split.as_ref(), active_tab_id)
            {
                if let Some(divider) = self
                    .pane_layout
                    .dividers
                    .iter()
                    .copied()
                    .find(|divider| divider.split_id == split_id)
                {
                    let ratio = split::PaneTree::ratio_for_pointer(
                        divider,
                        physical_to_egui_position(
                            (position.x, position.y),
                            self.pixels_per_point(),
                        ),
                    );
                    if let Some(tab) = self.tab_manager.tabs.get_mut(self.tab_manager.active_idx) {
                        tab.layout.set_split_ratio(split_id, ratio);
                    }
                    self.refresh_pane_layout();
                    if let Some(window) = &self.window {
                        window.set_cursor(match divider.direction {
                            SplitDirection::Horizontal => CursorIcon::RowResize,
                            SplitDirection::Vertical => CursorIcon::ColResize,
                        });
                    }
                }
            }

            let hovered_pane_id = self.pane_id_at(position.x, position.y);
            let target_pane_id = self
                .left_mouse_pane_id
                .as_ref()
                .filter(|_| self.left_mouse_gesture.is_some())
                .cloned()
                .or_else(|| hovered_pane_id.clone());
            let cell = target_pane_id
                .as_deref()
                .and_then(|pane_id| self.pixel_to_cell_for_pane(pane_id, position.x, position.y));
            if terminal_link_modifier_active(self.modifiers.state()) {
                let cursor =
                    if hovered_pane_id
                        .as_deref()
                        .zip(cell)
                        .is_some_and(|(pane_id, cell)| {
                            self.tab_manager
                                .active()
                                .is_some_and(|tab| tab.active_pane_id() == pane_id)
                                && self.terminal_link_at_cell(cell).is_some()
                        })
                    {
                        CursorIcon::Pointer
                    } else {
                        CursorIcon::Default
                    };
                if let Some(window) = &self.window {
                    window.set_cursor(cursor);
                }
            } else if self.dragged_split.is_none() {
                if let Some(window) = &self.window {
                    let divider_cursor = self
                        .pane_layout
                        .divider_at(physical_to_egui_position(
                            (position.x, position.y),
                            self.pixels_per_point(),
                        ))
                        .map(|divider| match divider.direction {
                            SplitDirection::Horizontal => CursorIcon::RowResize,
                            SplitDirection::Vertical => CursorIcon::ColResize,
                        })
                        .unwrap_or(CursorIcon::Default);
                    window.set_cursor(divider_cursor);
                }
            }
            match (self.left_mouse_gesture, target_pane_id.as_deref(), cell) {
                (Some(LeftMouseGesture::TerminalReport { .. }), Some(pane_id), Some(cell)) => {
                    self.left_mouse_gesture =
                        Some(LeftMouseGesture::TerminalReport { last_cell: cell });
                    self.send_mouse_event_to_pane(pane_id, 32, cell.0, cell.1, true);
                }
                (Some(LeftMouseGesture::LocalSelection), Some(pane_id), Some(cell)) => {
                    self.update_selection_for_pane(pane_id, cell);
                    if let (Some(renderer), Some(rect)) =
                        (self.renderer.as_ref(), self.pane_rect(pane_id))
                    {
                        let (_, cell_height) = renderer.cell_size();
                        let rect = logical_to_physical_pane_rect(rect, self.pixels_per_point());
                        let auto_scroll_lines = selection_auto_scroll_lines(
                            position.y as f32,
                            rect.y,
                            rect.y + rect.height,
                            cell_height,
                        );
                        if auto_scroll_lines != self.selection_auto_scroll_lines {
                            self.selection_auto_scroll_at = Instant::now()
                                .checked_sub(Duration::from_millis(32))
                                .unwrap_or_else(Instant::now);
                        }
                        self.selection_auto_scroll_lines = auto_scroll_lines;
                    } else {
                        self.selection_auto_scroll_lines = 0;
                    }
                }
                _ => {}
            }
        }
        // egui hover 需要重绘，但节流避免每个像素都全量渲染
        static LAST_RENDER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let now_ms = self.cursor_timer.elapsed().as_millis() as u64
            + self.startup_time.elapsed().as_millis() as u64;
        let last = LAST_RENDER.load(std::sync::atomic::Ordering::Relaxed);
        if now_ms.saturating_sub(last) > 16 {
            // ~60fps 上限
            LAST_RENDER.store(now_ms, std::sync::atomic::Ordering::Relaxed);
            self.do_render();
        }
    }

    pub(super) fn tick_selection_auto_scroll(&mut self, now: Instant) -> bool {
        if self.selection_auto_scroll_lines == 0
            || !matches!(
                self.left_mouse_gesture,
                Some(LeftMouseGesture::LocalSelection)
            )
            || now.duration_since(self.selection_auto_scroll_at) < Duration::from_millis(32)
        {
            return false;
        }
        let Some(pane_id) = self.left_mouse_pane_id.clone() else {
            self.selection_auto_scroll_lines = 0;
            return false;
        };
        let Some(cell) =
            self.pixel_to_cell_for_pane(&pane_id, self.mouse_position.0, self.mouse_position.1)
        else {
            self.selection_auto_scroll_lines = 0;
            return false;
        };
        let Some(terminal) = self.terminal_for_pane(&pane_id) else {
            self.selection_auto_scroll_lines = 0;
            return false;
        };

        let mut terminal = terminal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut changed = false;
        if let Some(term) = terminal.term_mut() {
            use alacritty_terminal::grid::Scroll;
            let before = term.grid().display_offset();
            term.scroll_display(Scroll::Delta(self.selection_auto_scroll_lines));
            changed = before != term.grid().display_offset();
        }
        if changed {
            terminal.mark_render_dirty();
            if let Some(point) = terminal.visual_point_to_grid_point(cell) {
                terminal.update_selection(point);
            }
        }
        self.selection_auto_scroll_at = now;
        changed
    }
}
