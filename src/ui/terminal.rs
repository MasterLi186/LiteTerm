use gtk::prelude::*;
use gtk::glib;
use gtk::gio;
use vte::prelude::*;
use vte::Terminal as VteTerminal;

use std::io::{Read as IoRead, Write as IoWrite};
use std::sync::{Arc, Mutex};

use crate::config::settings::TerminalSettings;

/// Terminal pane wrapping a real VTE4 terminal emulator widget.
///
/// Uses `vte::Terminal` for full terminal emulation with ANSI escape
/// sequence support, proper cursor handling, and native scrollback.
pub struct TerminalPane {
    scrolled: gtk::ScrolledWindow,
    terminal: VteTerminal,
}

impl TerminalPane {
    pub fn new(settings: &TerminalSettings) -> Self {
        let terminal = VteTerminal::new();
        terminal.set_scrollback_lines(settings.scrollback_lines as i64);
        terminal.set_cursor_blink_mode(if settings.cursor_blink {
            vte::CursorBlinkMode::On
        } else {
            vte::CursorBlinkMode::Off
        });

        // Set font via Pango font description
        let font_desc = gtk::pango::FontDescription::from_string(&settings.font);
        terminal.set_font(Some(&font_desc));

        // Set terminal size and expand policy
        terminal.set_size(120, 36);
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);

        let scrolled = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&terminal)
            .build();

        Self { scrolled, terminal }
    }

    /// Returns the container widget suitable for packing into a notebook page.
    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.scrolled
    }

    /// Returns a reference to the underlying VTE terminal widget.
    pub fn vte(&self) -> &VteTerminal {
        &self.terminal
    }

    /// Spawn a local shell (e.g. /bin/bash) in this terminal pane.
    ///
    /// This makes the terminal immediately interactive without an SSH
    /// connection. Uses VTE's built-in PTY support.
    pub fn spawn_local_shell(&self) {
        // Determine the user's shell, falling back to /bin/bash
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

        self.terminal.spawn_async(
            vte::PtyFlags::DEFAULT,
            None,                        // working directory (None = home)
            &[&shell],                   // command
            &[],                         // environment (inherit)
            glib::SpawnFlags::DEFAULT,
            || {},                       // child setup (no-op)
            -1,                          // timeout (-1 = default)
            None::<&gio::Cancellable>,
            |_result| {
                // Spawn callback; errors are logged but not fatal
                if let Err(e) = _result {
                    log::error!("Failed to spawn local shell: {}", e);
                }
            },
        );
    }

    /// Feed raw bytes to the terminal display.
    ///
    /// VTE handles ANSI escape sequences, cursor movement, colors, etc.
    pub fn feed_data(&self, data: &[u8]) {
        self.terminal.feed(data);
    }

    /// Connect an SSH session to this terminal pane.
    ///
    /// Opens a PTY-backed shell channel, then bridges I/O:
    /// - Reader thread: SSH channel stdout -> VTE feed on GTK main thread
    /// - VTE commit signal: user keystrokes -> SSH channel stdin
    pub fn connect_ssh_channel(&self, session: Arc<ssh2::Session>) {
        // Open a channel and request a PTY + shell
        let channel = {
            let mut ch = session
                .channel_session()
                .expect("failed to open SSH channel");
            ch.request_pty("xterm-256color", None, None)
                .expect("failed to request PTY");
            ch.shell().expect("failed to start shell");
            Arc::new(Mutex::new(ch))
        };

        // --- Reader thread: SSH channel -> VTE terminal ---
        let channel_reader = Arc::clone(&channel);
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let rx = Arc::new(Mutex::new(rx));

        // Periodically drain the channel on the main thread and feed the VTE.
        let terminal_for_rx = self.terminal.clone();
        let rx_clone = Arc::clone(&rx);
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            let rx = rx_clone.lock().unwrap();
            while let Ok(data) = rx.try_recv() {
                terminal_for_rx.feed(&data);
            }
            glib::ControlFlow::Continue
        });

        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                let n = {
                    let mut ch = match channel_reader.lock() {
                        Ok(ch) => ch,
                        Err(_) => break,
                    };
                    match ch.read(&mut buf) {
                        Ok(0) => break, // EOF
                        Ok(n) => n,
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            continue;
                        }
                        Err(_) => break,
                    }
                };

                if tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });

        // --- Writer: VTE commit signal -> SSH channel ---
        // The commit signal fires when the user types in the VTE terminal.
        let channel_writer = Arc::clone(&channel);
        self.terminal.connect_commit(move |_terminal, text, _size| {
            if let Ok(mut ch) = channel_writer.lock() {
                let _ = ch.write_all(text.as_bytes());
                let _ = ch.flush();
            }
        });
    }
}
