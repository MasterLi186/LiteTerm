use super::*;

impl TerminalState {
    /// Snapshot all grid lines (history + screen) as search cells.
    /// Spacers are marked `is_spacer` and never contribute haystack text.
    pub fn search_lines(&self) -> Vec<crate::terminal_search::SearchLine> {
        use crate::terminal_search::{SearchCell, SearchLine};

        let Some(term) = self.term.as_ref() else {
            return Vec::new();
        };

        let grid = term.grid();
        let top = grid.topmost_line().0;
        let bottom = grid.bottommost_line().0;
        let columns = grid.columns();
        let mut lines = Vec::with_capacity(bottom.saturating_sub(top).saturating_add(1) as usize);

        for line_idx in top..=bottom {
            let line = Line(line_idx);
            let mut cells = Vec::with_capacity(columns);
            for col in 0..columns {
                let cell = &grid[line][Column(col)];
                let flags = cell.flags;
                let is_spacer =
                    flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
                let width = if is_spacer {
                    1
                } else if flags.contains(Flags::WIDE_CHAR) {
                    2
                } else {
                    1
                };
                let zerowidth = if is_spacer {
                    Vec::new()
                } else {
                    cell.zerowidth().map(|z| z.to_vec()).unwrap_or_default()
                };
                cells.push(SearchCell {
                    col,
                    ch: cell.c,
                    width,
                    is_spacer,
                    zerowidth,
                });
            }
            lines.push(SearchLine::new(line_idx, cells));
        }
        lines
    }

    pub fn visual_row_to_grid_line(&self, visual_row: usize) -> Option<Line> {
        let grid = self.term.as_ref()?.grid();
        let line = visual_row as i32 - grid.display_offset() as i32 - 1;
        (line >= grid.topmost_line().0 && line <= grid.bottommost_line().0).then_some(Line(line))
    }

    pub fn visual_point_to_grid_point(&self, point: (usize, usize)) -> Option<(usize, i32)> {
        let grid = self.term.as_ref()?.grid();
        let last_column = grid.columns().saturating_sub(1);
        let line = self.visual_row_to_grid_line(point.1)?;
        Some((point.0.min(last_column), line.0))
    }

    pub fn selection_text(&self, start: (usize, usize), end: (usize, usize)) -> String {
        let Some(start) = self.visual_point_to_grid_point(start) else {
            return String::new();
        };
        let Some(end) = self.visual_point_to_grid_point(end) else {
            return String::new();
        };
        self.selection_text_grid(start, end)
    }

    pub fn selection_text_grid(&self, start: (usize, i32), end: (usize, i32)) -> String {
        let (start, end) = if (start.1, start.0) <= (end.1, end.0) {
            (start, end)
        } else {
            (end, start)
        };
        let Some(term) = self.term.as_ref() else {
            return String::new();
        };
        let grid = term.grid();
        if start.1 < grid.topmost_line().0
            || start.1 > grid.bottommost_line().0
            || end.1 < grid.topmost_line().0
            || end.1 > grid.bottommost_line().0
        {
            return String::new();
        }
        let last_column = grid.columns().saturating_sub(1);
        let start = Point::new(Line(start.1), Column(start.0.min(last_column)));
        let end = Point::new(Line(end.1), Column(end.0.min(last_column)));

        term.bounds_to_string(start, end)
    }

    pub fn link_at_visual(
        &self,
        visual_row: usize,
        column: usize,
        allow_local_paths: bool,
    ) -> Option<crate::terminal_links::TerminalLink> {
        let term = self.term.as_ref()?;
        let grid = term.grid();
        let clicked_line = self.visual_row_to_grid_line(visual_row)?;
        if column >= grid.columns() {
            return None;
        }
        let mut first_line = clicked_line.0;
        while first_line > grid.topmost_line().0 {
            let previous = Line(first_line - 1);
            if !grid[previous][Column(grid.columns().saturating_sub(1))]
                .flags
                .contains(Flags::WRAPLINE)
            {
                break;
            }
            first_line -= 1;
        }
        let mut last_line = clicked_line.0;
        while last_line < grid.bottommost_line().0
            && grid[Line(last_line)][Column(grid.columns().saturating_sub(1))]
                .flags
                .contains(Flags::WRAPLINE)
        {
            last_line += 1;
        }

        let mut cells = Vec::new();
        for line_index in first_line..=last_line {
            let logical_offset = (line_index - first_line) as usize * grid.columns();
            for col in 0..grid.columns() {
                let cell = &grid[Line(line_index)][Column(col)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                cells.push(crate::terminal_links::LinkCell {
                    ch: cell.c,
                    start_col: logical_offset + col,
                    width: if cell.flags.contains(Flags::WIDE_CHAR) {
                        2
                    } else {
                        1
                    },
                });
            }
        }
        let clicked = &grid[clicked_line][Column(column)];
        let explicit_osc8 = clicked.hyperlink();
        crate::terminal_links::link_at(
            &cells,
            (clicked_line.0 - first_line) as usize * grid.columns() + column,
            explicit_osc8.as_ref().map(|link| link.uri()),
            allow_local_paths,
            false,
        )
    }

    /// Scroll the display so `line` (absolute Line.0) is visible.
    /// Above viewport → pin to top; below → pin to bottom; already visible → no-op.
    pub fn reveal_search_line(&mut self, line: i32) {
        use alacritty_terminal::grid::Scroll;

        let Some(term) = self.term.as_mut() else {
            return;
        };

        let grid = term.grid();
        let topmost = grid.topmost_line().0;
        let bottommost = grid.bottommost_line().0;
        let screen = grid.screen_lines() as i32;
        let offset = grid.display_offset() as i32;

        let line = line.clamp(topmost, bottommost);
        let view_top = offset.saturating_neg();
        let view_bottom = view_top.saturating_add(screen.saturating_sub(1));

        if line >= view_top && line <= view_bottom {
            return;
        }

        // view_top = -display_offset  ⇒  display_offset = -view_top
        let new_offset = if line < view_top {
            // Pin target to top of viewport.
            line.saturating_neg()
        } else {
            // Pin target to bottom: view_top = line - (screen - 1)
            let pin_top = line.saturating_sub(screen.saturating_sub(1));
            pin_top.saturating_neg()
        };

        let delta = new_offset.saturating_sub(offset);
        if delta != 0 {
            term.scroll_display(Scroll::Delta(delta));
            self.mark_render_dirty();
        }
    }

    pub fn term(&self) -> Option<&Term<Listener>> {
        self.term.as_ref()
    }

    pub fn term_mut(&mut self) -> Option<&mut Term<Listener>> {
        self.term.as_mut()
    }

    pub fn take_reader(&mut self) -> Option<Box<dyn Read + Send>> {
        self.pty_reader.take()
    }

    pub(crate) fn zmodem_protocol_writer(&self) -> Option<crate::zmodem::runtime::ProtocolWriter> {
        self.zmodem_protocol_writer.clone()
    }

    pub(crate) fn zmodem_input_gate(&self) -> Arc<crate::zmodem::runtime::ProtocolGate> {
        Arc::clone(&self.zmodem_input_gate)
    }

    pub fn zmodem_active(&self) -> bool {
        self.zmodem_input_gate.is_active()
    }

    pub(super) fn take_pty_write_events(&mut self) -> Vec<String> {
        let mut writes = Vec::new();
        if let Some(pty_write_rx) = &self.pty_write_rx {
            while let Ok(text) = pty_write_rx.try_recv() {
                writes.push(text);
            }
        }
        writes
    }

    pub(super) fn flush_pty_write_events(&mut self) {
        let writes = self.take_pty_write_events();
        for text in writes {
            self.enqueue_writer_bytes(text.into_bytes());
        }
    }
}
