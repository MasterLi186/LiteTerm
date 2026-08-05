use super::*;

impl TerminalState {
    fn logical_cursor(&self) -> Option<LogicalPoint> {
        let term = self.term.as_ref()?;
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return None;
        }

        let grid = term.grid();
        let cursor = grid.cursor.point;
        Some(LogicalPoint {
            absolute_line: grid.history_size() as i64 + i64::from(cursor.line.0),
            column: if grid.cursor.input_needs_wrap {
                grid.columns()
            } else {
                cursor.column.0
            },
        })
    }

    pub fn current_bash_input(&self) -> Option<String> {
        let tracking = self.prompt_tracking.as_ref()?;
        if !tracking.active {
            return None;
        }
        let (prefix, anchor) = match tracking.snapshot_base.as_ref() {
            Some(snapshot) => (snapshot.prefix.as_str(), snapshot.anchor),
            None => ("", tracking.anchor?),
        };
        let remaining = MAX_SNAPSHOT_INPUT_BYTES.checked_sub(prefix.len())?;
        let delta = self.grid_delta_since(anchor, remaining)?;
        let mut input = String::with_capacity(prefix.len() + delta.len());
        input.push_str(prefix);
        input.push_str(&delta);
        Some(input)
    }

    pub fn has_authenticated_active_bash_prompt(&self) -> bool {
        self.prompt_tracking
            .as_ref()
            .is_some_and(|tracking| tracking.active)
    }

    pub fn completion_surface_safe(&self) -> bool {
        self.term.as_ref().is_some_and(|term| {
            !term
                .mode()
                .intersects(TermMode::ALT_SCREEN | TermMode::MOUSE_MODE)
        })
    }

    fn grid_delta_since(&self, anchor: LogicalPoint, max_bytes: usize) -> Option<String> {
        let cursor = self.logical_cursor()?;
        if cursor.absolute_line < anchor.absolute_line
            || (cursor.absolute_line == anchor.absolute_line && cursor.column < anchor.column)
        {
            return None;
        }

        let term = self.term.as_ref()?;
        let grid = term.grid();
        let history_size = grid.history_size() as i64;
        let screen_lines = grid.screen_lines() as i64;
        let max_absolute_line = history_size + screen_lines - 1;
        if anchor.absolute_line < 0
            || cursor.absolute_line > max_absolute_line
            || anchor.absolute_line > max_absolute_line
            || anchor.column > grid.columns()
            || cursor.column > grid.columns()
        {
            return None;
        }

        let mut input = String::new();
        for absolute_line in anchor.absolute_line..=cursor.absolute_line {
            let line = Line((absolute_line - history_size) as i32);
            let start_column = if absolute_line == anchor.absolute_line {
                anchor.column
            } else {
                0
            };
            let end_column = if absolute_line == cursor.absolute_line {
                cursor.column
            } else {
                grid.columns()
            };

            if absolute_line < cursor.absolute_line
                && !grid[line][Column(grid.columns() - 1)]
                    .flags
                    .contains(Flags::WRAPLINE)
            {
                return None;
            }

            for column in start_column..end_column {
                let cell = &grid[line][Column(column)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                if cell.c.is_control() {
                    return None;
                }
                if cell.c.len_utf8() > max_bytes.saturating_sub(input.len()) {
                    return None;
                }
                input.push(cell.c);
                if let Some(zerowidth) = cell.zerowidth() {
                    for character in zerowidth {
                        if character.len_utf8() > max_bytes.saturating_sub(input.len()) {
                            return None;
                        }
                        input.push(*character);
                    }
                }
            }
        }

        (!input.chars().any(char::is_control)).then_some(input)
    }

    pub fn current_bash_input_or_request_snapshot(&mut self, now: Instant) -> Option<String> {
        let input = self.current_bash_input();
        if input.is_none() {
            self.request_snapshot_if_due(now);
        }
        input
    }

    pub(super) fn request_snapshot_if_due(&mut self, now: Instant) -> bool {
        if self.zmodem_active() {
            return false;
        }
        let Some(tracking) = self.prompt_tracking.as_ref() else {
            return false;
        };
        if !tracking.active
            || tracking.snapshot_requested_at.is_some_and(|requested_at| {
                now.saturating_duration_since(requested_at) < SNAPSHOT_RETRY_TIMEOUT
            })
        {
            return false;
        }

        let sequence = self
            .local_bash_runtime
            .as_ref()
            .filter(|runtime| runtime.session() == &tracking.session)
            .map(|runtime| runtime.snapshot_sequence())
            .or_else(|| {
                self.remote_bash_runtime
                    .as_ref()
                    .filter(|runtime| runtime.session == tracking.session)
                    .map(|runtime| runtime.snapshot_sequence())
            });
        let Some(sequence) = sequence else {
            return false;
        };
        let bytes = sequence.as_bytes();
        if bytes.is_empty() || bytes.iter().any(|byte| matches!(byte, b'\r' | b'\n')) {
            return false;
        }
        let Some(writer) = self.writer.as_ref() else {
            return false;
        };
        if writer.try_send_normal(bytes.to_vec()).is_err() {
            return false;
        }
        if let Some(tracking) = &mut self.prompt_tracking {
            tracking.snapshot_requested_at = Some(now);
            tracking.outstanding_snapshot_responses =
                tracking.outstanding_snapshot_responses.saturating_add(1);
        }
        true
    }

    pub(super) fn invalidate_prompt_geometry(&mut self) {
        if let Some(tracking) = &mut self.prompt_tracking {
            tracking.anchor = None;
            tracking.snapshot_base = None;
        }
    }

    pub fn invalidate_readline_geometry(&mut self) {
        if let Some(tracking) = &mut self.prompt_tracking {
            tracking.stale_snapshot_responses = tracking.outstanding_snapshot_responses;
            tracking.snapshot_requested_at = None;
        }
        self.invalidate_prompt_geometry();
    }

    pub fn invalidate_prompt(&mut self) {
        if let Some(tracking) = &mut self.prompt_tracking {
            tracking.active = false;
            tracking.anchor = None;
            tracking.snapshot_base = None;
            tracking.snapshot_requested_at = None;
            tracking.outstanding_snapshot_responses = 0;
            tracking.stale_snapshot_responses = 0;
        }
    }

    pub fn take_bash_submission(&mut self) -> Option<String> {
        let submission = self.current_bash_input();
        self.invalidate_prompt();
        submission
    }

    pub fn finish_session(&mut self) {
        self.shutdown();
        self.prompt_tracking = None;
        self.local_bash_runtime = None;
        self.remote_bash_runtime = None;
    }

    pub fn shutdown(&mut self) {
        if let Some(shutdown_tx) = self.ssh_shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        if let Some(shutdown_tx) = self.serial_shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.writer = None;
        if let Some(sink) = &self.terminal_reply_sink {
            let mut sink = sink.lock().unwrap();
            sink.writer = None;
            sink.discard_deferred();
        }
        self.zmodem_protocol_writer = None;
        self.zmodem_input_gate.deactivate();
        self.pty_reader = None;
        self.pty_master = None;
        if let Some(child) = self.local_child.take() {
            terminate_local_child(child);
        }
        self.ssh_resize_tx = None;
        self.ssh_io_done_rx = None;
        self.serial_io_done_rx = None;
        if let Some(worker) = self.serial_join.take() {
            reap_serial_worker(worker);
        }
        self.local_bash_runtime = None;
        self.remote_bash_runtime = None;
        self.prompt_tracking = None;
    }

    fn invalidate_ambiguous_prompt(&mut self) {
        let alternate_screen = self
            .term
            .as_ref()
            .is_some_and(|term| term.mode().contains(TermMode::ALT_SCREEN));
        let has_geometry = self
            .prompt_tracking
            .as_ref()
            .is_some_and(|tracking| tracking.anchor.is_some() || tracking.snapshot_base.is_some());
        if alternate_screen {
            self.invalidate_prompt();
            return;
        }
        if !has_geometry || self.current_bash_input().is_some() {
            return;
        }

        let recoverable_backward_edit = self
            .prompt_tracking
            .as_ref()
            .and_then(|tracking| tracking.snapshot_base.as_ref())
            .zip(self.logical_cursor())
            .is_some_and(|(snapshot, cursor)| {
                cursor.absolute_line < snapshot.anchor.absolute_line
                    || (cursor.absolute_line == snapshot.anchor.absolute_line
                        && cursor.column < snapshot.anchor.column)
            });
        if recoverable_backward_edit {
            self.invalidate_prompt_geometry();
        } else {
            self.invalidate_prompt();
        }
    }

    pub(super) fn process_pty_output(
        &mut self,
        parser: &mut Processor,
        data: &[u8],
    ) -> Vec<IntegrationEvent> {
        if !data.is_empty() {
            self.mark_render_dirty();
            self.output_bytes_since_user_input = self
                .output_bytes_since_user_input
                .saturating_add(data.len());
        }
        self.refresh_terminal_reply_policy();
        self.begin_terminal_reply_batch();
        self.observe_terminal_reply_input(data);
        let boundaries = match &mut self.prompt_tracking {
            Some(tracking) => tracking.decoder.scan(data),
            None => {
                if let Some(term) = &mut self.term {
                    parser.advance(term, data);
                }
                self.prune_invalid_selection();
                self.finish_terminal_reply_batch();
                return Vec::new();
            }
        };

        let mut events = Vec::new();
        let mut start = 0;
        for boundary in boundaries {
            if let Some(term) = &mut self.term {
                parser.advance(term, &data[start..boundary.end_offset]);
            }
            start = boundary.end_offset;

            match boundary.kind {
                MarkerKind::Prompt => {
                    let anchor = self.logical_cursor();
                    if let Some(tracking) = &mut self.prompt_tracking {
                        tracking.active = true;
                        tracking.anchor = anchor;
                        tracking.snapshot_base = None;
                        tracking.snapshot_requested_at = None;
                        tracking.outstanding_snapshot_responses = 0;
                        tracking.stale_snapshot_responses = 0;
                    }
                }
                MarkerKind::HistoryPath(path) => {
                    if let Some(tracking) = &self.prompt_tracking {
                        events.push(IntegrationEvent::HistoryPath {
                            session: tracking.session.clone(),
                            path,
                        });
                    }
                }
                MarkerKind::InputSnapshot { line, point } => {
                    let anchor = self.logical_cursor();
                    if let Some(tracking) = self.prompt_tracking.as_mut() {
                        if !tracking.active || tracking.outstanding_snapshot_responses == 0 {
                            continue;
                        }
                        tracking.outstanding_snapshot_responses -= 1;
                        if tracking.stale_snapshot_responses > 0 {
                            tracking.stale_snapshot_responses -= 1;
                        } else if tracking.snapshot_requested_at.is_some() {
                            if let Some(anchor) = anchor {
                                tracking.anchor = None;
                                tracking.snapshot_base = Some(SnapshotBase {
                                    prefix: line[..point].to_owned(),
                                    anchor,
                                });
                                tracking.snapshot_requested_at = None;
                                tracking.stale_snapshot_responses =
                                    tracking.outstanding_snapshot_responses;
                            }
                        }
                    }
                }
            }
        }
        if let Some(term) = &mut self.term {
            parser.advance(term, &data[start..]);
        }
        self.prune_invalid_selection();
        self.invalidate_ambiguous_prompt();
        self.finish_terminal_reply_batch();
        events
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }
    pub fn rows(&self) -> u16 {
        self.rows
    }
}
