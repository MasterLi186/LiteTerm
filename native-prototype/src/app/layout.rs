use super::*;

impl App {
    pub(super) fn grid_size(&self) -> (u16, u16) {
        self.tab_manager
            .active()
            .and_then(|tab| self.pane_layout.pane(tab.active_pane_id()))
            .map(|pane| self.grid_size_for_rect(pane.rect))
            .unwrap_or_else(|| {
                self.renderer
                    .as_ref()
                    .map(Renderer::calculate_grid_size)
                    .unwrap_or((80, 24))
            })
    }

    pub(super) fn pixels_per_point(&self) -> f32 {
        sanitize_pixels_per_point(self.egui_ctx.pixels_per_point())
    }

    pub(super) fn grid_size_for_tab_pane(&self, tab_id: &str, pane_id: &str) -> (u16, u16) {
        self.terminal_layout_rect()
            .and_then(|bounds| {
                self.tab_manager.find_by_id(tab_id).and_then(|index| {
                    self.tab_manager.tabs[index]
                        .layout
                        .layout(bounds)
                        .pane(pane_id)
                        .map(|pane| pane.rect)
                })
            })
            .map(|rect| self.grid_size_for_rect(rect))
            .unwrap_or_else(|| self.grid_size())
    }

    pub(super) fn terminal_layout_rect(&self) -> Option<egui::Rect> {
        let gpu = self.gpu.as_ref()?;
        let playback_height = self
            .tab_manager
            .active()
            .is_some_and(|tab| matches!(tab.tab_type, TabType::Recording { .. }))
            .then_some(recording::PLAYBACK_CONTROLS_HEIGHT)
            .unwrap_or(0.0);
        Some(logical_terminal_layout_rect(
            gpu.width,
            gpu.height,
            self.pixels_per_point(),
            self.sidebar_width,
            self.tab_bar_height,
            self.command_bar_height,
            self.active_file_browser_height() + playback_height,
        ))
    }

    pub(super) fn current_pane_layout(&self) -> LayoutSnapshot {
        let Some(bounds) = self.terminal_layout_rect() else {
            return LayoutSnapshot::default();
        };
        self.tab_manager
            .active()
            .filter(|tab| tab.tab_type.is_terminal())
            .map(|tab| tab.layout.layout(bounds))
            .unwrap_or_default()
    }

    pub(super) fn refresh_pane_layout(&mut self) {
        self.pane_layout = self.current_pane_layout();
    }

    pub(super) fn grid_size_for_rect(&self, rect: egui::Rect) -> (u16, u16) {
        let Some(renderer) = self.renderer.as_ref() else {
            return (80, 24);
        };
        renderer.calculate_grid_size_for_rect(logical_to_physical_pane_rect(
            rect,
            self.pixels_per_point(),
        ))
    }

    pub(super) fn pane_id_at(&self, x: f64, y: f64) -> Option<PaneId> {
        let logical = physical_to_egui_position((x, y), self.pixels_per_point());
        self.pane_layout
            .pane_at(logical)
            .map(|pane| pane.pane_id.clone())
    }

    pub(super) fn pane_rect(&self, pane_id: &str) -> Option<egui::Rect> {
        self.pane_layout.pane(pane_id).map(|pane| pane.rect)
    }

    /// 把焦点移动到指定方向上最近的 pane；没有邻居时返回 `false`。
    pub(super) fn focus_pane_in_direction(&mut self, direction: PaneFocusDirection) -> bool {
        let Some(pane_id) = self
            .tab_manager
            .active()
            .filter(|tab| tab.tab_type.is_terminal())
            .map(|tab| tab.active_pane_id().to_string())
        else {
            return false;
        };
        let target = self
            .pane_layout
            .neighbor(&pane_id, direction)
            .map(|pane| pane.pane_id.clone());
        let Some(target) = target else {
            return false;
        };
        self.focus_pane(&target)
    }

    /// 当前标签是否支持分屏（本地/SSH 终端）。
    pub(super) fn active_tab_supports_split(&self) -> bool {
        self.tab_manager.active().is_some_and(Tab::supports_split)
    }

    /// 切换当前活动 pane 的临时最大化（与 Terminator 的 Ctrl+Shift+X 一致）。
    ///
    /// 只有多 pane 的终端标签才生效；最大化状态保存在该标签的布局中，
    /// 切换标签后各自保留。
    pub(super) fn toggle_maximized_active_pane(&mut self) -> bool {
        let index = self.tab_manager.active_idx;
        let Some(tab) = self.tab_manager.tabs.get_mut(index) else {
            return false;
        };
        if !tab.supports_split() || tab.panes().len() <= 1 {
            return false;
        }
        tab.layout.toggle_maximized_pane();
        self.refresh_pane_layout();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        true
    }

    pub(super) fn terminal_for_pane(&self, pane_id: &str) -> Option<Arc<Mutex<TerminalState>>> {
        self.tab_manager
            .active()
            .and_then(|tab| tab.pane(pane_id))
            .map(|pane| pane.terminal.clone())
    }

    pub(super) fn pixel_to_cell_for_pane(
        &self,
        pane_id: &str,
        x: f64,
        y: f64,
    ) -> Option<(usize, usize)> {
        let renderer = self.renderer.as_ref()?;
        let rect = self.pane_rect(pane_id)?;
        let rect = logical_to_physical_pane_rect(rect, self.pixels_per_point());
        let (cell_width, cell_height) = renderer.cell_size();
        let (cols, rows) = renderer.calculate_grid_size_for_rect(rect);
        if cols == 0 || rows == 0 {
            return None;
        }
        let col = ((x as f32 - rect.x).max(0.0) / cell_width).floor() as usize;
        let row = ((y as f32 - rect.y).max(0.0) / cell_height).floor() as usize;
        Some((
            col.min(usize::from(cols.saturating_sub(1))),
            // 指针落到底部之外时要夹到最后一个可见行；rows 本身是越界索引。
            row.min(usize::from(rows.saturating_sub(1))),
        ))
    }

    pub(super) fn focus_pane(&mut self, pane_id: &str) -> bool {
        let Some(tab_id) = self.tab_manager.active().map(|tab| tab.id.clone()) else {
            return false;
        };
        if self
            .tab_manager
            .active()
            .is_some_and(|tab| tab.active_pane_id() == pane_id)
        {
            return true;
        }
        self.invalidate_completion_popup_snapshot();
        self.clear_terminal_ime_composition();
        self.clear_selection();
        self.reset_click_sequence();
        // 焦点切到别的 pane 时退出最大化，避免输入落进隐藏 pane。
        let mut cleared_maximize = false;
        if let Some(index) = self.tab_manager.find_by_id(&tab_id) {
            if let Some(tab) = self.tab_manager.tabs.get_mut(index) {
                cleared_maximize = tab.layout.clear_maximized_pane();
            }
        }
        let focused = self.tab_manager.set_active_pane(&tab_id, pane_id);
        if cleared_maximize {
            self.refresh_pane_layout();
        }
        focused
    }
}
