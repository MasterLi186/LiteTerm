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
                    self.selection_drag_anchor = None;
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
                (Some(LeftMouseGesture::LocalSelection), _, Some(cell))
                    if self.click_state == ClickState::Single =>
                {
                    if let Some((start, end)) =
                        drag_selection_range(self.selection_drag_anchor, cell)
                    {
                        self.selection_start = Some(start);
                        self.selection_end = Some(end);
                    } else {
                        self.selection_start = None;
                        self.selection_end = None;
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
}
