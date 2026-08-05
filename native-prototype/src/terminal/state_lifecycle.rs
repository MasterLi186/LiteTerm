use super::*;

impl TerminalState {
    pub fn new() -> Self {
        Self {
            term: None,
            writer: None,
            zmodem_protocol_writer: None,
            zmodem_input_gate: Arc::new(crate::zmodem::runtime::ProtocolGate::new()),
            pty_reader: None,
            pty_master: None,
            local_child: None,
            ssh_resize_tx: None,
            ssh_shutdown_tx: None,
            ssh_io_done_rx: None,
            serial_shutdown_tx: None,
            serial_io_done_rx: None,
            serial_join: None,
            terminal_reply_sink: None,
            cols: 80,
            rows: 24,
            scroll_offset: 0,
            local_bash_runtime: None,
            remote_bash_runtime: None,
            prompt_tracking: None,
            replay_parser: None,
            render_revision: 1,
            output_bytes_since_user_input: 0,
        }
    }

    pub fn new_replay(cols: u16, rows: u16) -> Self {
        let mut terminal = Self::new();
        terminal.init_term(cols, rows);
        terminal
    }

    pub fn reset_replay(&mut self, cols: u16, rows: u16) {
        self.shutdown();
        self.replay_parser = None;
        self.init_term(cols, rows);
    }

    pub fn feed_replay_output(&mut self, bytes: &[u8]) {
        let mut parser = self.replay_parser.take().unwrap_or_else(Processor::new);
        let _ = self.process_pty_output(&mut parser, bytes);
        self.replay_parser = Some(parser);
    }

    #[cfg(test)]
    pub(crate) fn authenticated_prompt_with_input_for_test(
        session: CompletionSessionKey,
        input: &str,
    ) -> Self {
        let mut terminal = Self::new();
        terminal.init_term(80, 24);
        terminal.prompt_tracking = Some(PromptTracking {
            decoder: MarkerDecoder::new(session.clone()),
            session: session.clone(),
            active: false,
            anchor: None,
            snapshot_base: None,
            snapshot_requested_at: None,
            outstanding_snapshot_responses: 0,
            stale_snapshot_responses: 0,
        });
        let mut parser = Processor::new();
        let output = format!(
            "\x1b]777;LiteTerm;{};{};P\x07{}",
            session.token(),
            session.generation,
            input
        );
        terminal.process_pty_output(&mut parser, output.as_bytes());
        terminal
    }

    pub(super) fn init_term(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        let mut config = TermConfig::default();
        config.semantic_escape_chars = SEMANTIC_SELECTION_DELIMITERS.to_owned();
        let dims = TermDimensions {
            cols: cols as usize,
            rows: rows as usize,
        };
        let terminal_reply_sink = Arc::new(Mutex::new(TerminalReplySink::default()));
        self.term = Some(Term::new(
            config,
            &dims,
            Listener {
                terminal_reply_sink: Arc::clone(&terminal_reply_sink),
            },
        ));
        self.terminal_reply_sink = Some(terminal_reply_sink);
        self.replay_parser = None;
        self.output_bytes_since_user_input = 0;
    }

    pub fn spawn_shell(&mut self, cols: u16, rows: u16) {
        let shell = crate::terminal::default_shell_path();
        self.spawn_shell_with_path(&shell, cols, rows, CompletionSessionKey::new(1));
    }

    pub fn spawn_shell_with_path(
        &mut self,
        shell: &str,
        cols: u16,
        rows: u16,
        session: CompletionSessionKey,
    ) {
        self.spawn_shell_prevalidated(shell, cols, rows, session)
            .expect("启动可信本地 shell 失败");
    }

    pub fn try_spawn_shell_with_path(
        &mut self,
        shell: &str,
        cols: u16,
        rows: u16,
        session: CompletionSessionKey,
    ) -> Result<(), String> {
        validate_shell_path(shell)?;
        self.spawn_shell_prevalidated(shell, cols, rows, session)
    }

    fn spawn_shell_prevalidated(
        &mut self,
        shell: &str,
        cols: u16,
        rows: u16,
        session: CompletionSessionKey,
    ) -> Result<(), String> {
        let local_bash_runtime = if is_bash_path(shell) {
            match LocalBashRuntime::create(session) {
                Ok(runtime) => Some(runtime),
                Err(error) => {
                    log::warn!("创建 Bash 智能补全运行环境失败，将使用普通 shell: {error}");
                    None
                }
            }
        } else {
            None
        };
        let prompt_tracking = local_bash_runtime.as_ref().map(|runtime| {
            let session = runtime.session().clone();
            PromptTracking {
                decoder: MarkerDecoder::new(session.clone()),
                session,
                active: false,
                anchor: None,
                snapshot_base: None,
                snapshot_requested_at: None,
                outstanding_snapshot_responses: 0,
                stale_snapshot_responses: 0,
            }
        });

        let pty_system = native_pty_system();
        let pty_pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("打开 PTY 失败: {error}"))?;
        pty_pair
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("设置 PTY 尺寸失败: {error}"))?;

        let mut cmd = CommandBuilder::new(shell);
        cmd.env("TERM", "xterm-256color");
        if let Some(runtime) = &local_bash_runtime {
            cmd.arg("--rcfile");
            cmd.arg(crate::bash_integration::bash_path_for_shell(
                runtime.rc_path(),
            ));
            cmd.arg("-i");
            #[cfg(test)]
            configure_isolated_test_bash_environment(&mut cmd, runtime);
        }
        let reader = pty_pair
            .master
            .try_clone_reader()
            .map_err(|error| format!("克隆 PTY reader 失败: {error}"))?;
        let writer = pty_pair
            .master
            .take_writer()
            .map_err(|error| format!("获取 PTY writer 失败: {error}"))?;
        let child = pty_pair
            .slave
            .spawn_command(cmd)
            .map_err(|error| format!("启动 shell 失败: {error}"))?;

        self.shutdown();
        self.init_term(cols, rows);
        self.local_bash_runtime = None;
        self.remote_bash_runtime = None;
        self.prompt_tracking = prompt_tracking;
        self.pty_reader = Some(reader);
        let (writer, protocol_writer) =
            spawn_writer_worker_with_protocol(writer, Arc::clone(&self.zmodem_input_gate));
        self.install_transport_writer(writer);
        self.zmodem_protocol_writer = Some(protocol_writer);
        self.pty_master = Some(pty_pair.master);
        self.local_child = Some(child);
        self.local_bash_runtime = local_bash_runtime;
        Ok(())
    }

    /// 设置 SSH 连接结果（由异步回调调用）
    pub fn apply_ssh_handle(&mut self, handle: crate::ssh::SshHandle, cols: u16, rows: u16) {
        self.shutdown();
        self.init_term(cols, rows);
        let crate::ssh::SshHandle {
            reader,
            write_tx,
            resize_tx,
            shutdown_tx,
            io_done_rx,
            bash_runtime,
        } = handle;
        self.zmodem_input_gate = write_tx.protocol_active_gate();
        let protocol_writer =
            crate::zmodem::runtime::ProtocolWriter::from_transport_writer(write_tx.clone());
        self.pty_reader = Some(reader);
        self.install_transport_writer(write_tx);
        self.zmodem_protocol_writer = Some(protocol_writer);
        self.ssh_resize_tx = Some(resize_tx);
        self.ssh_shutdown_tx = Some(shutdown_tx);
        self.ssh_io_done_rx = Some(io_done_rx);
        self.pty_master = None;
        self.local_child = None;
        self.local_bash_runtime = None;
        self.prompt_tracking = bash_runtime.as_ref().map(|runtime| PromptTracking {
            decoder: MarkerDecoder::new(runtime.session.clone()),
            session: runtime.session.clone(),
            active: false,
            anchor: None,
            snapshot_base: None,
            snapshot_requested_at: None,
            outstanding_snapshot_responses: 0,
            stale_snapshot_responses: 0,
        });
        self.remote_bash_runtime = bash_runtime;
    }

    pub fn apply_serial_handle(
        &mut self,
        handle: crate::serial::SerialHandle,
        cols: u16,
        rows: u16,
    ) {
        self.shutdown();
        self.init_term(cols, rows);
        let mut parts = handle.into_parts();
        self.pty_reader = Some(parts.reader);
        self.zmodem_input_gate = parts.write_tx.protocol_active_gate();
        self.install_transport_writer(parts.write_tx);
        self.zmodem_protocol_writer = None;
        self.serial_shutdown_tx = Some(parts.shutdown_tx);
        self.serial_io_done_rx = Some(parts.io_done_rx);
        self.serial_join = parts.join.take();
        self.pty_master = None;
        self.local_child = None;
        self.local_bash_runtime = None;
        self.remote_bash_runtime = None;
        self.prompt_tracking = None;
    }

    pub fn write_input(&mut self, text: &str) {
        if self.zmodem_active() {
            return;
        }
        self.output_bytes_since_user_input = 0;
        self.enqueue_writer_bytes(text.as_bytes().to_vec());
    }

    pub fn try_write_input(&mut self, text: &str) -> Result<(), String> {
        if self.zmodem_active() {
            return Err("ZMODEM 传输期间已拒绝终端输入".into());
        }
        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| "终端写入通道尚未就绪".to_string())?;
        let result = writer
            .try_send_normal(text.as_bytes().to_vec())
            .map_err(|error| error.to_string());
        if result.is_ok() {
            self.output_bytes_since_user_input = 0;
        }
        result
    }

    pub fn stage_completion_fill(
        &mut self,
        candidate: &str,
        direct_prefix: Option<&str>,
    ) -> Result<CandidateWriteRequest, String> {
        crate::bash_integration::validate_candidate_text(candidate)?;
        let session = self
            .local_bash_runtime
            .as_ref()
            .map(|runtime| runtime.session().clone())
            .or_else(|| {
                self.remote_bash_runtime
                    .as_ref()
                    .map(|runtime| runtime.session.clone())
            })
            .ok_or_else(|| "当前终端未启用 Bash 智能补全".to_string())?;
        if let Some(prefix) = direct_prefix {
            if !self.completion_surface_safe() {
                return Err("当前终端界面不允许智能补全".into());
            }
            if prefix.is_empty() || prefix.chars().any(char::is_control) {
                return Err("补全前缀无效".into());
            }
            let suffix = candidate
                .strip_prefix(prefix)
                .filter(|suffix| !suffix.is_empty())
                .ok_or_else(|| "候选项与当前输入不匹配".to_string())?;
            if suffix.chars().any(char::is_control) {
                return Err("候选项包含控制字符".into());
            }
            return Ok(CandidateWriteRequest {
                session,
                target: CandidateWriteTarget::Direct,
                bytes: suffix.as_bytes().to_vec(),
            });
        }
        if let Some(runtime) = &self.local_bash_runtime {
            return Ok(CandidateWriteRequest {
                session,
                target: CandidateWriteTarget::Local(runtime.candidate_path().to_path_buf()),
                bytes: candidate.as_bytes().to_vec(),
            });
        }
        if let Some(runtime) = &self.remote_bash_runtime {
            return Ok(CandidateWriteRequest {
                session,
                target: CandidateWriteTarget::Remote(runtime.candidate_path.clone()),
                bytes: candidate.as_bytes().to_vec(),
            });
        }
        unreachable!("completion session requires a Bash runtime")
    }

    pub fn commit_completion_fill(&mut self) -> bool {
        if self.zmodem_active() || !self.completion_surface_safe() {
            return false;
        }
        let sequence = self
            .local_bash_runtime
            .as_ref()
            .map(|runtime| runtime.widget_sequence())
            .or_else(|| {
                self.remote_bash_runtime
                    .as_ref()
                    .map(|runtime| runtime.widget_sequence.as_str())
            });
        let Some(sequence) = sequence else {
            return false;
        };
        let bytes = sequence.as_bytes();
        if bytes.is_empty() || bytes.iter().any(|byte| matches!(byte, b'\r' | b'\n')) {
            return false;
        }
        self.writer
            .as_ref()
            .is_some_and(|writer| writer.try_send_normal(bytes.to_vec()).is_ok())
    }

    pub fn commit_direct_completion_fill(&mut self, suffix: &[u8]) -> bool {
        let Ok(suffix_text) = std::str::from_utf8(suffix) else {
            return false;
        };
        if self.zmodem_active()
            || !self.completion_surface_safe()
            || suffix_text.is_empty()
            || suffix_text.chars().any(char::is_control)
        {
            return false;
        }
        self.writer
            .as_ref()
            .is_some_and(|writer| writer.try_send_normal(suffix.to_vec()).is_ok())
    }

    pub(super) fn enqueue_writer_bytes(&self, bytes: Vec<u8>) {
        if self.zmodem_active() {
            return;
        }
        if let Some(write_tx) = &self.writer {
            if let Err(error) = write_tx.try_send_normal(bytes) {
                // Fire-and-forget UI paths stay non-blocking, but queue
                // saturation/disconnect is no longer completely invisible.
                log::warn!("终端输入未能进入写队列: {error}");
            }
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.invalidate_prompt_geometry();
        self.mark_render_dirty();
        self.cols = cols;
        self.rows = rows;
        if let Some(t) = &mut self.term {
            let dims = TermDimensions {
                cols: cols as usize,
                rows: rows as usize,
            };
            t.resize(dims);
        }
        if let Some(master) = &self.pty_master {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        if let Some(tx) = &self.ssh_resize_tx {
            let _ = tx.try_send((cols, rows));
        }
        self.request_snapshot_if_due(Instant::now());
    }

    pub fn scroll(&mut self, delta: i32) {
        if let Some(t) = &self.term {
            let max = t.grid().history_size() as i32;
            self.scroll_offset = (self.scroll_offset + delta).clamp(0, max);
            self.mark_render_dirty();
        }
    }

    pub fn render_revision(&self) -> u64 {
        self.render_revision
    }

    pub fn mark_render_dirty(&mut self) {
        self.render_revision = self.render_revision.wrapping_add(1).max(1);
    }

    /// Clear terminal contents locally without sending escape bytes to the shell.
    ///
    /// These sequences are terminal *output*. Sending them through `write()` would
    /// make readline/fish interpret ESC as keyboard input and echo fragments like
    /// `[2J` instead of clearing the emulator.
    pub fn clear_display(&mut self, include_scrollback: bool) {
        let mut parser = Processor::new();
        let sequence = if include_scrollback {
            // Clear the viewport first so any lines moved into history are also
            // removed by the final "erase saved lines" operation.
            b"\x1b[2J\x1b[H\x1b[3J".as_slice()
        } else {
            b"\x1b[2J\x1b[H".as_slice()
        };
        self.process_pty_output(&mut parser, sequence);
        self.scroll_offset = 0;
    }
}
