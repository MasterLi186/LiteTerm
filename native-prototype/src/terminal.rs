use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::Term;
use alacritty_terminal::term::Config as TermConfig;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

/// alacritty_terminal 的 Dimensions trait 实现
struct TermDimensions {
    cols: usize,
    rows: usize,
}

impl Dimensions for TermDimensions {
    fn total_lines(&self) -> usize { self.rows + 10000 }
    fn screen_lines(&self) -> usize { self.rows }
    fn columns(&self) -> usize { self.cols }
}

/// 事件监听器（alacritty_terminal 需要）
#[derive(Clone)]
struct Listener;

impl EventListener for Listener {
    fn send_event(&self, _event: Event) {}
}

pub struct TerminalState {
    term: Option<Term<Listener>>,
    pty_writer: Option<Box<dyn Write + Send>>,
    pty_reader: Option<Box<dyn Read + Send>>,
    cols: u16,
    rows: u16,
}

impl TerminalState {
    pub fn new() -> Self {
        Self {
            term: None,
            pty_writer: None,
            pty_reader: None,
            cols: 80,
            rows: 24,
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
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("打开 PTY 失败");

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let mut cmd = CommandBuilder::new(&shell);
        cmd.env("TERM", "xterm-256color");
        pty_pair.slave.spawn_command(cmd).expect("启动 shell 失败");

        let reader = pty_pair.master.try_clone_reader().expect("克隆 PTY reader 失败");
        let writer = pty_pair.master.take_writer().expect("获取 PTY writer 失败");

        self.pty_reader = Some(reader);
        self.pty_writer = Some(writer);
    }

    pub fn write_input(&mut self, text: &str) {
        if let Some(writer) = &mut self.pty_writer {
            let _ = writer.write_all(text.as_bytes());
            let _ = writer.flush();
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
    }

    #[allow(dead_code)]
    pub fn term(&self) -> Option<&Term<Listener>> {
        self.term.as_ref()
    }

    pub fn take_reader(&mut self) -> Option<Box<dyn Read + Send>> {
        self.pty_reader.take()
    }

    #[allow(dead_code)]
    pub fn cols(&self) -> u16 { self.cols }
    #[allow(dead_code)]
    pub fn rows(&self) -> u16 { self.rows }
}

/// PTY 读取循环：从 PTY 读数据，触发重绘
pub fn read_loop<F>(terminal: Arc<Mutex<TerminalState>>, request_redraw: F)
where
    F: Fn() + Send + 'static,
{
    let mut reader = {
        let mut term = terminal.lock().unwrap();
        match term.take_reader() {
            Some(r) => r,
            None => return,
        }
    };

    let mut buf = [0u8; 4096];
    let mut parser = alacritty_terminal::vte::ansi::Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new();

    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let mut term_state = terminal.lock().unwrap();
                if let Some(t) = &mut term_state.term {
                    parser.advance(t, &buf[..n]);
                }
                drop(term_state);
                request_redraw();
            }
            Err(e) => {
                log::error!("PTY 读取错误: {}", e);
                break;
            }
        }
    }
}
