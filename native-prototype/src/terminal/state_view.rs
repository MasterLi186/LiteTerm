use super::*;

impl TerminalState {
    pub fn begin_selection(&mut self, point: (usize, i32), kind: TerminalSelectionKind) -> bool {
        let Some(term) = self.term.as_mut() else {
            return false;
        };
        let point = Point::new(Line(point.1), Column(point.0));
        term.selection = Some(Selection::new(kind.into(), point, Side::Left));
        self.mark_render_dirty();
        true
    }

    pub fn update_selection(&mut self, point: (usize, i32)) -> bool {
        let Some(term) = self.term.as_mut() else {
            return false;
        };
        let Some(selection) = term.selection.as_ref() else {
            return false;
        };
        let was_empty = selection.is_empty();
        let mut updated = selection.clone();
        updated.update(Point::new(Line(point.1), Column(point.0)), Side::Right);
        updated.include_all();
        let same_cell_jitter = was_empty
            && updated
                .to_range(term)
                .is_some_and(|range| range.start == range.end);
        if same_cell_jitter {
            return false;
        }
        term.selection = Some(updated);
        self.mark_render_dirty();
        true
    }

    /// Extend a visible selection, or start a new selection at the live terminal cursor.
    ///
    /// A plain click leaves an empty selection behind so a subsequent drag can start without
    /// losing its press position. That empty mouse anchor must not become the anchor of a later
    /// Shift+click: without visible selected text, users expect Shift+click to select between the
    /// live terminal cursor and the clicked scrollback cell.
    pub fn shift_extend_selection(&mut self, point: (usize, i32)) -> bool {
        let Some(term) = self.term.as_mut() else {
            return false;
        };
        let has_visible_selection = term
            .selection
            .as_ref()
            .and_then(|selection| selection.to_range(term))
            .is_some();
        if !has_visible_selection {
            let cursor = term.grid().cursor.point;
            term.selection = Some(Selection::new(SelectionType::Simple, cursor, Side::Left));
            self.mark_render_dirty();
        }

        // Reaching this point means the Shift+click was handled even when the click is exactly on
        // the cursor and therefore produces no visible range yet.
        let _ = self.update_selection(point);
        true
    }

    pub fn clear_selection(&mut self) {
        let Some(term) = self.term.as_mut() else {
            return;
        };
        if term.selection.take().is_some() {
            self.mark_render_dirty();
        }
    }

    pub fn has_selection_anchor(&self) -> bool {
        self.term
            .as_ref()
            .is_some_and(|term| term.selection.is_some())
    }

    pub fn selection_range(&self) -> Option<SelectionRange> {
        let term = self.term.as_ref()?;
        term.selection
            .as_ref()
            .and_then(|selection| selection.to_range(term))
    }

    pub fn current_selection_text(&self) -> String {
        self.term
            .as_ref()
            .and_then(Term::selection_to_string)
            .unwrap_or_default()
    }

    pub(super) fn prune_invalid_selection(&mut self) {
        let Some(term) = self.term.as_mut() else {
            return;
        };
        let invalid = term.selection.as_ref().is_some_and(|selection| {
            let mut probe = selection.clone();
            probe.include_all();
            probe.to_range(term).is_none()
        });
        if invalid {
            term.selection = None;
            self.mark_render_dirty();
        }
    }

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

    pub(super) fn install_transport_writer(
        &mut self,
        writer: crate::zmodem::runtime::TransportWriter,
    ) {
        if let Some(sink) = &self.terminal_reply_sink {
            sink.lock().unwrap().writer = Some(
                crate::zmodem::runtime::TerminalReplyWriter::from_transport_writer(writer.clone()),
            );
        }
        self.writer = Some(writer);
    }

    pub(super) fn refresh_terminal_reply_policy(&self) {
        // A canonical PTY consumer (for example `cat`) cannot be waiting for a
        // terminal report. Writing DA/DSR replies there only leaves control
        // bytes for the next shell prompt. Interactive programs switch the PTY
        // to raw mode; shell integration additionally closes the short race in
        // which Bash has regained the foreground but has not emitted PS1 yet.
        // Nested adb/SSH shells keep the outer PTY raw, so primary-screen bulk
        // output is bounded as a second signal; alternate-screen TUIs remain
        // unrestricted.
        let alternate_screen = self
            .term
            .as_ref()
            .is_some_and(|term| term.mode().contains(TermMode::ALT_SCREEN));
        let bounded_primary_output =
            self.output_bytes_since_user_input <= MAX_COMPAT_PRIMARY_REPLY_OUTPUT_BYTES;
        #[cfg(unix)]
        let interactive_local = self
            .pty_master
            .as_ref()
            .and_then(|master| {
                let fd = master.as_raw_fd()?;
                let mut attributes = std::mem::MaybeUninit::<libc::termios>::uninit();
                if unsafe { libc::tcgetattr(fd, attributes.as_mut_ptr()) } != 0 {
                    return None;
                }
                let attributes = unsafe { attributes.assume_init() };
                let raw_mode = attributes.c_lflag & libc::ICANON == 0;
                let Some(shell_pid) = self
                    .local_child
                    .as_ref()
                    .and_then(|child| child.process_id())
                    .map(|pid| pid as libc::pid_t)
                else {
                    return Some(raw_mode);
                };
                let Some(prompt) = self.prompt_tracking.as_ref() else {
                    return Some(raw_mode);
                };
                let foreground_is_shell = master.process_group_leader() == Some(shell_pid);
                Some(raw_mode && (prompt.active || !foreground_is_shell))
            })
            .unwrap_or(true);
        #[cfg(not(unix))]
        let interactive_local = true;
        let allowed = alternate_screen || (bounded_primary_output && interactive_local);

        if let Some(sink) = &self.terminal_reply_sink {
            sink.lock().unwrap().allowed = allowed;
        }
    }

    pub(super) fn take_pty_write_events(&mut self) -> Vec<String> {
        let Some(sink) = &self.terminal_reply_sink else {
            return Vec::new();
        };
        std::mem::take(&mut sink.lock().unwrap().pending)
    }
}
