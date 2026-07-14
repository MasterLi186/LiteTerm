use std::io::{Read, Write};
use std::sync::{Arc, Mutex, mpsc};

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

/// Writer abstraction: either direct Box<dyn Write> or mpsc sender
enum WriterKind {
    Direct(Box<dyn Write + Send>),
    Channel(mpsc::Sender<Vec<u8>>),
}

pub struct TerminalState {
    term: Option<Term<Listener>>,
    writer: Option<WriterKind>,
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
            writer: None,
            pty_reader: None,
            pty_master: None,
            cols: 80,
            rows: 24,
            scroll_offset: 0,
        }
    }

    fn init_term(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        let config = TermConfig::default();
        let dims = TermDimensions { cols: cols as usize, rows: rows as usize };
        self.term = Some(Term::new(config, &dims, Listener));
    }

    pub fn spawn_shell(&mut self, cols: u16, rows: u16) {
        self.init_term(cols, rows);

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
        self.writer = Some(WriterKind::Direct(writer));
        self.pty_master = Some(pty_pair.master);
    }

    /// Connect via SSH. This spawns the SSH connection on a dedicated thread
    /// and sets up reader/writer for the terminal.
    pub fn spawn_ssh(
        &mut self,
        host: &str,
        port: u16,
        user: &str,
        auth: &str,
        key_path: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), String> {
        self.init_term(cols, rows);

        let host = host.to_string();
        let user = user.to_string();
        let auth = auth.to_string();
        let key_path = key_path.to_string();

        // SSH connect runs on a separate thread (ssh2::Session is !Send)
        // We use a oneshot channel to get the result back
        let (result_tx, result_rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let kp = if key_path.is_empty() { None } else { Some(key_path.as_str()) };
            let result = crate::ssh::connect(&host, port, &user, &auth, kp, cols, rows);
            let _ = result_tx.send(result);
        });

        // Wait for connection (blocking, but this is called from the UI thread
        // during setup — could be made async later)
        let ssh_handle = result_rx.recv()
            .map_err(|_| "SSH 连接线程异常退出".to_string())?
            .map_err(|e| format!("SSH 连接失败: {}", e))?;

        self.pty_reader = Some(ssh_handle.reader);
        self.writer = Some(WriterKind::Channel(ssh_handle.write_tx));
        self.pty_master = None; // No PTY master for SSH

        Ok(())
    }

    pub fn write_input(&mut self, text: &str) {
        match &mut self.writer {
            Some(WriterKind::Direct(w)) => {
                let _ = w.write_all(text.as_bytes());
                let _ = w.flush();
            }
            Some(WriterKind::Channel(tx)) => {
                let _ = tx.send(text.as_bytes().to_vec());
            }
            None => {}
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

    pub fn cols(&self) -> u16 { self.cols }
    pub fn rows(&self) -> u16 { self.rows }
}

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

    let mut buf = [0u8; 8192];
    let mut parser = alacritty_terminal::vte::ansi::Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new();

    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => { log::error!("读取错误: {}", e); break; }
        };

        {
            let mut term_state = terminal.lock().unwrap();
            if let Some(t) = &mut term_state.term {
                parser.advance(t, &buf[..n]);
            }
        }

        request_redraw();
    }
}
