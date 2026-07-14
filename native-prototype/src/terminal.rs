use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::Term;
use alacritty_terminal::term::Config as TermConfig;
use portable_pty::{CommandBuilder, PtySize, native_pty_system, MasterPty};

struct TermDimensions {
    cols: usize,
    rows: usize,
}

impl Dimensions for TermDimensions {
    fn total_lines(&self) -> usize { self.rows + 10000 }
    fn screen_lines(&self) -> usize { self.rows }
    fn columns(&self) -> usize { self.cols }
}

#[derive(Clone)]
pub struct Listener;

impl EventListener for Listener {
    fn send_event(&self, _event: Event) {}
}

pub struct TerminalState {
    term: Option<Term<Listener>>,
    pty_writer: Option<Box<dyn Write + Send>>,
    pty_reader: Option<Box<dyn Read + Send>>,
    pty_master: Option<Box<dyn MasterPty + Send>>,
    cols: u16,
    rows: u16,
    pub scroll_offset: i32,
}

impl TerminalState {
    pub fn new() -> Self {
        Self {
            term: None,
            pty_writer: None,
            pty_reader: None,
            pty_master: None,
            cols: 80,
            rows: 24,
            scroll_offset: 0,
        }
    }

    pub fn spawn_shell(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;

        let config = TermConfig::default();
        let dims = TermDimensions { cols: cols as usize, rows: rows as usize };
        let term = Term::new(config, &dims, Listener);
        self.term = Some(term);

        let pty_system = native_pty_system();
        let pty_pair = pty_system
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .expect("打开 PTY 失败");

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let mut cmd = CommandBuilder::new(&shell);
        cmd.env("TERM", "xterm-256color");
        pty_pair.slave.spawn_command(cmd).expect("启动 shell 失败");

        let reader = pty_pair.master.try_clone_reader().expect("克隆 PTY reader 失败");
        let writer = pty_pair.master.take_writer().expect("获取 PTY writer 失败");

        self.pty_reader = Some(reader);
        self.pty_writer = Some(writer);
        self.pty_master = Some(pty_pair.master);
    }

    pub fn write_input(&mut self, text: &str) {
        if let Some(writer) = &mut self.pty_writer {
            let _ = writer.write_all(text.as_bytes());
            let _ = writer.flush();
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        if cols == self.cols && rows == self.rows { return; }
        self.cols = cols;
        self.rows = rows;
        if let Some(t) = &mut self.term {
            let dims = TermDimensions { cols: cols as usize, rows: rows as usize };
            t.resize(dims);
        }
        if let Some(master) = &self.pty_master {
            let _ = master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        }
    }

    pub fn scroll(&mut self, delta: i32) {
        if let Some(t) = &self.term {
            let max = t.grid().history_size() as i32;
            self.scroll_offset = (self.scroll_offset + delta).clamp(0, max);
            // alacritty_terminal 的 display_offset 需要通过 scroll_display 设置
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

    pub fn cols(&self) -> u16 { self.cols }
    pub fn rows(&self) -> u16 { self.rows }
}

pub fn read_loop<F>(terminal: Arc<Mutex<TerminalState>>, request_redraw: F)
where
    F: Fn() + Send + 'static,
{
    use std::io::ErrorKind;
    use std::time::{Duration, Instant};

    let mut reader = {
        let mut term = terminal.lock().unwrap();
        match term.take_reader() {
            Some(r) => r,
            None => return,
        }
    };

    let mut buf = [0u8; 8192];
    let mut parser = alacritty_terminal::vte::ansi::Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new();

    // 合并重绘：攒数据最多 8ms 或 64KB 后统一刷新
    const BATCH_TIMEOUT: Duration = Duration::from_millis(8);
    const BATCH_MAX_BYTES: usize = 65536;

    loop {
        // 阻塞等待第一块数据
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::TimedOut => continue,
            Err(e) => { log::error!("PTY 读取错误: {}", e); break; }
        };

        let mut term_state = terminal.lock().unwrap();
        if let Some(t) = &mut term_state.term {
            parser.advance(t, &buf[..n]);
        }

        // 在超时内继续尝试读（非阻塞），尽可能攒批
        let batch_start = Instant::now();
        let mut total = n;

        loop {
            if total >= BATCH_MAX_BYTES || batch_start.elapsed() >= BATCH_TIMEOUT {
                break;
            }
            // portable-pty 的 reader 是阻塞的，用短 sleep 模拟非阻塞
            // 如果 8ms 内没有更多数据，就停止攒批
            std::thread::sleep(Duration::from_micros(500));
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n2) => {
                    if let Some(t) = &mut term_state.term {
                        parser.advance(t, &buf[..n2]);
                    }
                    total += n2;
                }
                Err(e) if e.kind() == ErrorKind::TimedOut || e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }

        drop(term_state);
        request_redraw();
    }
}
