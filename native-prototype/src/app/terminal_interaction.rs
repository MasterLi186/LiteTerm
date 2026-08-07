use super::*;

impl App {
    pub(super) fn pixel_to_cell(&self, x: f64, y: f64) -> (usize, usize) {
        self.tab_manager
            .active()
            .and_then(|tab| self.pixel_to_cell_for_pane(tab.active_pane_id(), x, y))
            .unwrap_or((0, 0))
    }

    pub(super) fn is_in_terminal(&self, x: f64, y: f64) -> bool {
        self.pane_id_at(x, y).is_some()
    }

    pub(super) fn get_selection_text(&self) -> String {
        let terminal = match self.active_terminal() {
            Some(t) => t,
            None => return String::new(),
        };
        let terminal = terminal.lock().unwrap();
        terminal.current_selection_text()
    }

    pub(super) fn visual_cell_to_selection_point_for_pane(
        &self,
        pane_id: &str,
        cell: (usize, usize),
    ) -> Option<SelectionPoint> {
        let terminal = self.terminal_for_pane(pane_id)?;
        let point = terminal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .visual_point_to_grid_point(cell);
        point
    }

    pub(super) fn begin_selection_for_pane(
        &mut self,
        pane_id: &str,
        cell: (usize, usize),
        kind: terminal::TerminalSelectionKind,
    ) -> bool {
        let Some(point) = self.visual_cell_to_selection_point_for_pane(pane_id, cell) else {
            return false;
        };
        let Some(terminal) = self.terminal_for_pane(pane_id) else {
            return false;
        };
        let updated = terminal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .begin_selection(point, kind);
        updated
    }

    pub(super) fn update_selection_for_pane(
        &mut self,
        pane_id: &str,
        cell: (usize, usize),
    ) -> bool {
        let Some(point) = self.visual_cell_to_selection_point_for_pane(pane_id, cell) else {
            return false;
        };
        let Some(terminal) = self.terminal_for_pane(pane_id) else {
            return false;
        };
        let updated = terminal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .update_selection(point);
        updated
    }

    pub(super) fn copy_selection(&mut self) {
        let text = self.get_selection_text();
        if !text.is_empty() {
            if let Some(cb) = &mut self.clipboard {
                let _ = cb.set_text(&text);
            }
        }
    }

    pub(super) fn is_mouse_mode(&self) -> bool {
        let Some(pane_id) = self
            .tab_manager
            .active()
            .map(|tab| tab.active_pane_id().to_string())
        else {
            return false;
        };
        self.is_mouse_mode_for_pane(&pane_id)
    }

    pub(super) fn is_mouse_mode_for_pane(&self, pane_id: &str) -> bool {
        let terminal = match self.terminal_for_pane(pane_id) {
            Some(t) => t,
            None => return false,
        };
        let term = terminal.lock().unwrap();
        Renderer::is_mouse_mode(&term)
    }

    pub(super) fn send_mouse_event(&mut self, btn: u32, col: usize, row: usize, pressed: bool) {
        let Some(pane_id) = self
            .tab_manager
            .active()
            .map(|tab| tab.active_pane_id().to_string())
        else {
            return;
        };
        self.send_mouse_event_to_pane(&pane_id, btn, col, row, pressed);
    }

    pub(super) fn send_mouse_event_to_pane(
        &mut self,
        pane_id: &str,
        btn: u32,
        col: usize,
        row: usize,
        pressed: bool,
    ) {
        self.invalidate_completion_popup_snapshot();
        let c = if pressed { 'M' } else { 'm' };
        let seq = format!("\x1b[<{};{};{}{}", btn, col + 1, row + 1, c);
        if let Some(tab) = self.tab_manager.tabs.get_mut(self.tab_manager.active_idx) {
            if !tab.tab_type.is_terminal() {
                return;
            }
            let Some(pane) = tab.pane_mut(pane_id) else {
                return;
            };
            let terminal = pane.terminal.clone();
            let mut terminal = terminal.lock().unwrap();
            write_completion_invalidating_control_sequence(
                &mut pane.completion,
                &mut terminal,
                &seq,
            );
        }
    }

    pub(super) fn check_ssh_connect(&mut self) {
        if let Some(conn) = self.sidebar.take_connect() {
            log::debug!(
                "[MAIN] check_ssh_connect: 新连接 {} ({}:{})",
                conn.label,
                conn.host,
                conn.port
            );
            self.new_ssh_tab(&conn);
        }
        if let Some(conn) = self.sidebar.password_connect.take() {
            log::debug!(
                "[MAIN] check_ssh_connect: 密码重试 {} ({}:{})",
                conn.label,
                conn.host,
                conn.port
            );
            self.new_ssh_tab(&conn);
        }
    }

    pub(super) fn sync_terminal_size(&mut self) {
        let Some(bounds) = self.terminal_layout_rect() else {
            return;
        };
        let Some((cell_width, cell_height)) = self.renderer.as_ref().map(Renderer::cell_size)
        else {
            return;
        };
        let pixels_per_point = self.pixels_per_point();

        for tab in self
            .tab_manager
            .tabs
            .iter()
            .filter(|tab| tab.tab_type.is_terminal())
        {
            let layout = tab.layout.layout(bounds);
            for viewport in &layout.panes {
                let Some(pane) = tab.pane(&viewport.pane_id) else {
                    continue;
                };
                let physical = logical_to_physical_pane_rect(viewport.rect, pixels_per_point);
                let cols = (physical.width.max(0.0) / cell_width).floor() as u16;
                let rows =
                    ((physical.height.max(0.0) / cell_height).floor() as u16).saturating_sub(1);
                pane.terminal
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .resize(cols.max(1), rows.max(1));
            }
        }

        self.refresh_pane_layout();
        let active_rect = self
            .tab_manager
            .active()
            .and_then(|tab| self.pane_layout.pane(tab.active_pane_id()))
            .map(|pane| pane.rect);
        if let (Some(rect), Some(renderer), Some(gpu)) =
            (active_rect, &mut self.renderer, &self.gpu)
        {
            let rect = logical_to_physical_pane_rect(rect, pixels_per_point);
            renderer.set_viewport(rect.x, rect.y, rect.width, rect.height, gpu);
        }
    }
}
