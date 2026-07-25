use std::io::{Read, Write};
use std::sync::{mpsc, Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::StdSyncHandler;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use crate::bash_integration::{is_bash_path, LocalBashRuntime, MarkerDecoder, MarkerKind};
use crate::smart_completion::CompletionSessionKey;

type Processor = alacritty_terminal::vte::ansi::Processor<StdSyncHandler>;

struct TermDimensions {
    cols: usize,
    rows: usize,
}

impl Dimensions for TermDimensions {
    fn total_lines(&self) -> usize {
        self.rows + 10000
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

#[derive(Clone)]
pub struct Listener {
    pty_write_tx: mpsc::Sender<String>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event {
            let _ = self.pty_write_tx.send(text);
        }
    }
}

fn spawn_writer_worker(mut writer: Box<dyn Write + Send>) -> mpsc::Sender<Vec<u8>> {
    let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        while let Ok(bytes) = write_rx.recv() {
            if writer.write_all(&bytes).is_err() || writer.flush().is_err() {
                break;
            }
        }
    });
    write_tx
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntegrationEvent {
    HistoryPath {
        session: CompletionSessionKey,
        path: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogicalPoint {
    absolute_line: i64,
    column: usize,
}

pub struct PromptTracking {
    session: CompletionSessionKey,
    decoder: MarkerDecoder,
    anchor: Option<LogicalPoint>,
}

pub struct TerminalState {
    term: Option<Term<Listener>>,
    writer: Option<mpsc::Sender<Vec<u8>>>,
    pty_reader: Option<Box<dyn Read + Send>>,
    pty_master: Option<Box<dyn MasterPty + Send>>,
    pty_write_rx: Option<mpsc::Receiver<String>>,
    cols: u16,
    rows: u16,
    pub scroll_offset: i32,
    pub local_bash_runtime: Option<LocalBashRuntime>,
    prompt_tracking: Option<PromptTracking>,
}

impl TerminalState {
    pub fn new() -> Self {
        Self {
            term: None,
            writer: None,
            pty_reader: None,
            pty_master: None,
            pty_write_rx: None,
            cols: 80,
            rows: 24,
            scroll_offset: 0,
            local_bash_runtime: None,
            prompt_tracking: None,
        }
    }

    fn init_term(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        let config = TermConfig::default();
        let dims = TermDimensions {
            cols: cols as usize,
            rows: rows as usize,
        };
        let (pty_write_tx, pty_write_rx) = mpsc::channel();
        self.term = Some(Term::new(config, &dims, Listener { pty_write_tx }));
        self.pty_write_rx = Some(pty_write_rx);
    }

    pub fn spawn_shell(&mut self, cols: u16, rows: u16) {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        self.spawn_shell_with_path(&shell, cols, rows, CompletionSessionKey::new(1));
    }

    pub fn spawn_shell_with_path(
        &mut self,
        shell: &str,
        cols: u16,
        rows: u16,
        session: CompletionSessionKey,
    ) {
        self.init_term(cols, rows);
        self.local_bash_runtime = None;

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
        self.prompt_tracking = local_bash_runtime.as_ref().map(|runtime| {
            let session = runtime.session().clone();
            PromptTracking {
                decoder: MarkerDecoder::new(session.clone()),
                session,
                anchor: None,
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
            .expect("打开 PTY 失败");

        let mut cmd = CommandBuilder::new(shell);
        cmd.env("TERM", "xterm-256color");
        if let Some(runtime) = &local_bash_runtime {
            cmd.arg("--rcfile");
            cmd.arg(runtime.rc_path());
            cmd.arg("-i");
        }
        pty_pair.slave.spawn_command(cmd).expect("启动 shell 失败");

        let reader = pty_pair
            .master
            .try_clone_reader()
            .expect("克隆 PTY reader 失败");
        let writer = pty_pair.master.take_writer().expect("获取 PTY writer 失败");

        self.pty_reader = Some(reader);
        self.writer = Some(spawn_writer_worker(writer));
        self.pty_master = Some(pty_pair.master);
        self.local_bash_runtime = local_bash_runtime;
    }

    /// 设置 SSH 连接结果（由异步回调调用）
    pub fn apply_ssh_handle(&mut self, handle: crate::ssh::SshHandle, cols: u16, rows: u16) {
        self.init_term(cols, rows);
        self.pty_reader = Some(handle.reader);
        self.writer = Some(handle.write_tx);
        self.pty_master = None;
        self.local_bash_runtime = None;
        self.prompt_tracking = None;
    }

    pub fn write_input(&mut self, text: &str) {
        self.enqueue_writer_bytes(text.as_bytes().to_vec());
    }

    fn enqueue_writer_bytes(&self, bytes: Vec<u8>) {
        if let Some(write_tx) = &self.writer {
            let _ = write_tx.send(bytes);
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.invalidate_prompt();
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
        // TODO: SSH resize (would need to send window-change request through channel)
    }

    pub fn scroll(&mut self, delta: i32) {
        if let Some(t) = &self.term {
            let max = t.grid().history_size() as i32;
            self.scroll_offset = (self.scroll_offset + delta).clamp(0, max);
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

    fn take_pty_write_events(&mut self) -> Vec<String> {
        let mut writes = Vec::new();
        if let Some(pty_write_rx) = &self.pty_write_rx {
            while let Ok(text) = pty_write_rx.try_recv() {
                writes.push(text);
            }
        }
        writes
    }

    fn flush_pty_write_events(&mut self) {
        let writes = self.take_pty_write_events();
        for text in writes {
            self.enqueue_writer_bytes(text.into_bytes());
        }
    }

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
        let anchor = self.prompt_tracking.as_ref()?.anchor?;
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
                input.push(cell.c);
                if let Some(zerowidth) = cell.zerowidth() {
                    input.extend(zerowidth);
                }
            }
        }

        (!input.chars().any(char::is_control)).then_some(input)
    }

    pub fn invalidate_prompt(&mut self) {
        if let Some(tracking) = &mut self.prompt_tracking {
            tracking.anchor = None;
        }
    }

    pub fn take_bash_submission(&mut self) -> Option<String> {
        let submission = self.current_bash_input();
        self.invalidate_prompt();
        submission
    }

    pub fn finish_session(&mut self) {
        self.prompt_tracking = None;
        self.local_bash_runtime = None;
    }

    fn invalidate_ambiguous_prompt(&mut self) {
        let has_anchor = self
            .prompt_tracking
            .as_ref()
            .is_some_and(|tracking| tracking.anchor.is_some());
        if has_anchor && self.current_bash_input().is_none() {
            self.invalidate_prompt();
        }
    }

    fn process_pty_output(&mut self, parser: &mut Processor, data: &[u8]) -> Vec<IntegrationEvent> {
        let boundaries = match &mut self.prompt_tracking {
            Some(tracking) => tracking.decoder.scan(data),
            None => {
                if let Some(term) = &mut self.term {
                    parser.advance(term, data);
                }
                self.flush_pty_write_events();
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
                        tracking.anchor = anchor;
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
            }
        }
        if let Some(term) = &mut self.term {
            parser.advance(term, &data[start..]);
        }
        self.flush_pty_write_events();
        self.invalidate_ambiguous_prompt();
        events
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }
    pub fn rows(&self) -> u16 {
        self.rows
    }
}

pub fn read_loop<F, G>(terminal: Arc<Mutex<TerminalState>>, request_redraw: F, integration_event: G)
where
    F: Fn() + Send + 'static,
    G: Fn(IntegrationEvent) + Send + 'static,
{
    let mut reader = {
        let mut term = terminal.lock().unwrap();
        match term.take_reader() {
            Some(r) => r,
            None => {
                term.finish_session();
                return;
            }
        }
    };

    let mut buf = [0u8; 8192];
    let mut parser = Processor::new();

    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                log::error!("读取错误: {}", e);
                break;
            }
        };

        let events = {
            let mut term_state = terminal.lock().unwrap();
            term_state.process_pty_output(&mut parser, &buf[..n])
        };

        for event in events {
            integration_event(event);
        }
        request_redraw();
    }

    terminal.lock().unwrap().finish_session();
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::vte::ansi::StdSyncHandler;
    use std::io;
    use std::time::Duration;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";
    const GENERATION: u64 = 42;

    type TestProcessor = alacritty_terminal::vte::ansi::Processor<StdSyncHandler>;

    fn completion_session() -> CompletionSessionKey {
        CompletionSessionKey::new_for_test(GENERATION, TOKEN)
    }

    fn prompt_marker() -> Vec<u8> {
        format!("\x1b]777;LiteTerm;{TOKEN};{GENERATION};P\x07").into_bytes()
    }

    fn history_marker(path_payload: &str) -> Vec<u8> {
        format!("\x1b]777;LiteTerm;{TOKEN};{GENERATION};H;{path_payload}\x07").into_bytes()
    }

    fn tracked_terminal(cols: u16, rows: u16) -> TerminalState {
        let mut terminal = TerminalState::new();
        terminal.init_term(cols, rows);
        terminal.prompt_tracking = Some(PromptTracking {
            session: completion_session(),
            decoder: MarkerDecoder::new(completion_session()),
            anchor: None,
        });
        terminal
    }

    struct SharedWriter {
        captured: Arc<Mutex<Vec<u8>>>,
        completed_tx: mpsc::Sender<()>,
    }

    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.captured.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            let _ = self.completed_tx.send(());
            Ok(())
        }
    }

    struct FailingWriter {
        attempt_tx: mpsc::Sender<()>,
        dropped_tx: mpsc::Sender<()>,
    }

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            let _ = self.attempt_tx.send(());
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "writer disconnected",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "writer disconnected",
            ))
        }
    }

    impl Drop for FailingWriter {
        fn drop(&mut self) {
            let _ = self.dropped_tx.send(());
        }
    }

    struct BlockingWriter {
        entered_tx: mpsc::Sender<()>,
        release_rx: mpsc::Receiver<()>,
    }

    impl Write for BlockingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let _ = self.entered_tx.send(());
            let _ = self.release_rx.recv();
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct BrokenReader;

    impl Read for BrokenReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "reader failed"))
        }
    }

    fn advance_and_take(
        terminal: &mut TerminalState,
        parser: &mut TestProcessor,
        bytes: &[u8],
    ) -> Vec<String> {
        parser.advance(terminal.term.as_mut().unwrap(), bytes);
        terminal.take_pty_write_events()
    }

    #[test]
    fn new_terminal_has_no_local_bash_runtime() {
        assert!(TerminalState::new().local_bash_runtime.is_none());
    }

    #[test]
    fn prompt_marker_boundary_makes_same_chunk_suffix_current_input() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice(b"git");

        let events = terminal.process_pty_output(&mut parser, &output);

        assert!(events.is_empty());
        assert_eq!(terminal.current_bash_input().as_deref(), Some("git"));
    }

    #[test]
    fn exact_width_input_includes_the_pending_wrap_cell() {
        let mut terminal = tracked_terminal(3, 4);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice(b"git");

        terminal.process_pty_output(&mut parser, &output);

        assert_eq!(terminal.current_bash_input().as_deref(), Some("git"));
    }

    #[test]
    fn exact_width_prompt_does_not_become_part_of_wrapped_input() {
        let mut terminal = tracked_terminal(3, 4);
        let mut parser = TestProcessor::new();
        let mut output = b"abc".to_vec();
        output.extend_from_slice(&prompt_marker());
        output.extend_from_slice(b"git");

        terminal.process_pty_output(&mut parser, &output);

        assert_eq!(terminal.current_bash_input().as_deref(), Some("git"));
    }

    #[test]
    fn prompt_marker_split_across_chunks_still_anchors_before_suffix() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();
        let marker = prompt_marker();
        let split = marker.len() - 2;

        assert!(terminal
            .process_pty_output(&mut parser, &marker[..split])
            .is_empty());
        assert_eq!(terminal.current_bash_input(), None);

        let mut tail = marker[split..].to_vec();
        tail.extend_from_slice(b"git");
        assert!(terminal.process_pty_output(&mut parser, &tail).is_empty());
        assert_eq!(terminal.current_bash_input().as_deref(), Some("git"));
    }

    #[test]
    fn segmented_marker_processing_matches_unmodified_parser_input() {
        let mut tracked = tracked_terminal(80, 24);
        let mut baseline = TerminalState::new();
        baseline.init_term(80, 24);
        let mut tracked_parser = TestProcessor::new();
        let mut baseline_parser = TestProcessor::new();
        let mut output = b"before".to_vec();
        output.extend_from_slice(&prompt_marker());
        output.extend_from_slice(b"git");
        output.extend_from_slice(&history_marker("L3RtcC9oaXN0b3J5"));
        output.extend_from_slice(&prompt_marker());
        output.extend_from_slice("你好abc".as_bytes());
        output.extend_from_slice(b"\x1b[3");

        tracked.process_pty_output(&mut tracked_parser, &output);
        baseline_parser.advance(baseline.term.as_mut().unwrap(), &output);

        let tracked_term = tracked.term().unwrap();
        let baseline_term = baseline.term().unwrap();
        assert_eq!(tracked_term.grid(), baseline_term.grid());
        assert_eq!(
            tracked_term.grid().cursor.point,
            baseline_term.grid().cursor.point
        );
        assert_eq!(
            tracked_term.grid().cursor.input_needs_wrap,
            baseline_term.grid().cursor.input_needs_wrap
        );
        assert_eq!(tracked_term.mode(), baseline_term.mode());

        tracked.process_pty_output(&mut tracked_parser, b"1mX");
        baseline_parser.advance(baseline.term.as_mut().unwrap(), b"1mX");
        let tracked_term = tracked.term().unwrap();
        let baseline_term = baseline.term().unwrap();
        assert_eq!(tracked_term.grid(), baseline_term.grid());
        assert_eq!(
            tracked_term.grid().cursor.point,
            baseline_term.grid().cursor.point
        );
    }

    #[test]
    fn soft_wrap_with_wide_spacers_extracts_cjk_and_ascii() {
        let mut terminal = tracked_terminal(5, 4);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice("你好abc".as_bytes());

        terminal.process_pty_output(&mut parser, &output);

        assert_eq!(terminal.current_bash_input().as_deref(), Some("你好abc"));
    }

    #[test]
    fn wide_cell_and_zerowidth_combining_character_are_preserved() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice("好e\u{301}".as_bytes());

        terminal.process_pty_output(&mut parser, &output);

        assert_eq!(terminal.current_bash_input().as_deref(), Some("好e\u{301}"));
    }

    #[test]
    fn hard_newline_invalidates_prompt_anchor() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice(b"git\r\n");

        terminal.process_pty_output(&mut parser, &output);

        assert_eq!(terminal.current_bash_input(), None);
        assert!(terminal.prompt_tracking.as_ref().unwrap().anchor.is_none());
    }

    #[test]
    fn alternate_screen_invalidates_prompt_anchor_permanently() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();

        terminal.process_pty_output(&mut parser, &prompt_marker());
        assert_eq!(terminal.current_bash_input().as_deref(), Some(""));

        terminal.process_pty_output(&mut parser, b"\x1b[?1049h");
        assert!(terminal.prompt_tracking.as_ref().unwrap().anchor.is_none());
        terminal.process_pty_output(&mut parser, b"\x1b[?1049l");
        assert_eq!(terminal.current_bash_input(), None);
    }

    #[test]
    fn only_real_resize_invalidates_prompt_anchor() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();

        terminal.process_pty_output(&mut parser, &prompt_marker());
        terminal.resize(80, 24);
        assert_eq!(terminal.current_bash_input().as_deref(), Some(""));

        terminal.resize(81, 24);
        assert_eq!(terminal.current_bash_input(), None);
        assert!(terminal.prompt_tracking.as_ref().unwrap().anchor.is_none());
    }

    #[test]
    fn history_path_event_carries_tracking_session() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();

        let events = terminal.process_pty_output(&mut parser, &history_marker("L3RtcC9oaXN0b3J5"));

        assert_eq!(
            events,
            vec![IntegrationEvent::HistoryPath {
                session: completion_session(),
                path: "/tmp/history".to_owned(),
            }]
        );
    }

    #[test]
    fn taking_submission_returns_input_and_clears_anchor() {
        let mut terminal = tracked_terminal(80, 24);
        let mut parser = TestProcessor::new();
        let mut output = prompt_marker();
        output.extend_from_slice(b"git");
        terminal.process_pty_output(&mut parser, &output);

        assert_eq!(terminal.take_bash_submission().as_deref(), Some("git"));
        assert_eq!(terminal.current_bash_input(), None);
    }

    #[test]
    fn finish_session_drops_runtime_temp_directory_and_tracking() {
        let runtime = LocalBashRuntime::create(completion_session()).unwrap();
        let temp_dir = runtime.temp_dir().to_owned();
        let mut terminal = tracked_terminal(80, 24);
        terminal.local_bash_runtime = Some(runtime);

        terminal.finish_session();

        assert!(!temp_dir.exists());
        assert!(terminal.local_bash_runtime.is_none());
        assert!(terminal.prompt_tracking.is_none());
    }

    #[test]
    fn read_loop_callbacks_run_unlocked_and_eof_finishes_session() {
        let runtime = LocalBashRuntime::create(completion_session()).unwrap();
        let temp_dir = runtime.temp_dir().to_owned();
        let mut state = tracked_terminal(80, 24);
        state.local_bash_runtime = Some(runtime);
        let mut output = prompt_marker();
        output.extend_from_slice(b"git");
        output.extend_from_slice(&history_marker("L3RtcC9oaXN0b3J5"));
        state.pty_reader = Some(Box::new(std::io::Cursor::new(output)));
        let terminal = Arc::new(Mutex::new(state));
        let redraw_terminal = Arc::clone(&terminal);
        let event_terminal = Arc::clone(&terminal);
        let redraw_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let redraw_count_callback = Arc::clone(&redraw_count);
        let event_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let event_count_callback = Arc::clone(&event_count);

        read_loop(
            Arc::clone(&terminal),
            move || {
                assert!(redraw_terminal.try_lock().is_ok());
                redraw_count_callback.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
            move |_event| {
                assert!(event_terminal.try_lock().is_ok());
                event_count_callback.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
        );

        assert_eq!(redraw_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(event_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!temp_dir.exists());
        let state = terminal.lock().unwrap();
        assert!(state.local_bash_runtime.is_none());
        assert!(state.prompt_tracking.is_none());
    }

    #[test]
    fn read_loop_without_reader_still_finishes_session() {
        let runtime = LocalBashRuntime::create(completion_session()).unwrap();
        let temp_dir = runtime.temp_dir().to_owned();
        let mut state = tracked_terminal(80, 24);
        state.local_bash_runtime = Some(runtime);
        let terminal = Arc::new(Mutex::new(state));

        read_loop(Arc::clone(&terminal), || {}, |_event| {});

        assert!(!temp_dir.exists());
        assert!(terminal.lock().unwrap().prompt_tracking.is_none());
    }

    #[test]
    fn read_loop_unrecoverable_error_finishes_session_without_callbacks() {
        let runtime = LocalBashRuntime::create(completion_session()).unwrap();
        let temp_dir = runtime.temp_dir().to_owned();
        let mut state = tracked_terminal(80, 24);
        state.local_bash_runtime = Some(runtime);
        state.pty_reader = Some(Box::new(BrokenReader));
        let terminal = Arc::new(Mutex::new(state));
        let callback_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let redraw_count = Arc::clone(&callback_count);
        let event_count = Arc::clone(&callback_count);

        read_loop(
            Arc::clone(&terminal),
            move || {
                redraw_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
            move |_event| {
                event_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
        );

        assert_eq!(callback_count.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(!temp_dir.exists());
        let state = terminal.lock().unwrap();
        assert!(state.local_bash_runtime.is_none());
        assert!(state.prompt_tracking.is_none());
    }

    #[test]
    fn fish_cursor_forward_is_not_a_text_area_query() {
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        let mut parser = TestProcessor::new();

        assert!(advance_and_take(&mut terminal, &mut parser, b"\x1b[18C").is_empty());
        assert!(advance_and_take(&mut terminal, &mut parser, b"\x1b[18~").is_empty());
    }

    #[test]
    fn text_area_query_uses_alacritty_reply_and_survives_chunk_split() {
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        let mut parser = TestProcessor::new();

        assert!(advance_and_take(&mut terminal, &mut parser, b"\x1b[18").is_empty());
        assert_eq!(
            advance_and_take(&mut terminal, &mut parser, b"t"),
            vec!["\x1b[8;48;180t"]
        );
        assert!(terminal.take_pty_write_events().is_empty());
    }

    #[test]
    fn cursor_position_query_still_uses_alacritty_reply() {
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        let mut parser = TestProcessor::new();

        assert!(advance_and_take(&mut terminal, &mut parser, b"\x1b[3;5H").is_empty());
        assert_eq!(
            advance_and_take(&mut terminal, &mut parser, b"\x1b[6n"),
            vec!["\x1b[3;5R"]
        );
    }

    #[test]
    fn listener_ignores_a_disconnected_event_receiver() {
        let (pty_write_tx, pty_write_rx) = mpsc::channel();
        drop(pty_write_rx);
        let listener = Listener { pty_write_tx };

        listener.send_event(Event::PtyWrite("reply".to_owned()));
    }

    #[test]
    fn listener_discards_non_pty_write_events_at_the_boundary() {
        let (pty_write_tx, pty_write_rx) = mpsc::channel();
        let listener = Listener { pty_write_tx };

        listener.send_event(Event::Bell);

        assert!(matches!(
            pty_write_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn local_writer_does_not_receive_reply_for_fish_cursor_forward() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (completed_tx, completed_rx) = mpsc::channel();
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        terminal.writer = Some(spawn_writer_worker(Box::new(SharedWriter {
            captured: Arc::clone(&captured),
            completed_tx,
        })));
        let mut parser = TestProcessor::new();

        terminal.process_pty_output(&mut parser, b"\x1b[18C");

        assert!(matches!(
            completed_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(captured.lock().unwrap().is_empty());
    }

    #[test]
    fn local_and_ssh_writers_receive_the_same_alacritty_reply() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (completed_tx, completed_rx) = mpsc::channel();
        let mut local = TerminalState::new();
        local.init_term(180, 48);
        local.writer = Some(spawn_writer_worker(Box::new(SharedWriter {
            captured: Arc::clone(&captured),
            completed_tx,
        })));
        let mut local_parser = TestProcessor::new();

        let (write_tx, write_rx) = mpsc::channel();
        let mut ssh = TerminalState::new();
        ssh.init_term(180, 48);
        ssh.writer = Some(write_tx);
        let mut ssh_parser = TestProcessor::new();

        local.process_pty_output(&mut local_parser, b"\x1b[18t");
        ssh.process_pty_output(&mut ssh_parser, b"\x1b[18t");

        completed_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(&*captured.lock().unwrap(), b"\x1b[8;48;180t");
        assert_eq!(write_rx.try_recv().unwrap(), b"\x1b[8;48;180t");
    }

    #[test]
    fn protocol_reply_write_failure_does_not_panic() {
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (dropped_tx, dropped_rx) = mpsc::channel();
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        terminal.writer = Some(spawn_writer_worker(Box::new(FailingWriter {
            attempt_tx,
            dropped_tx,
        })));
        let mut parser = TestProcessor::new();

        terminal.process_pty_output(&mut parser, b"\x1b[18t");

        attempt_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        dropped_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        terminal.write_input("after failure");
    }

    #[test]
    fn local_writer_worker_keeps_protocol_processing_nonblocking() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut terminal = TerminalState::new();
        terminal.init_term(180, 48);
        terminal.writer = Some(spawn_writer_worker(Box::new(BlockingWriter {
            entered_tx,
            release_rx,
        })));
        let mut parser = TestProcessor::new();

        terminal.process_pty_output(&mut parser, b"\x1b[18t");

        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        release_tx.send(()).unwrap();
    }
}
