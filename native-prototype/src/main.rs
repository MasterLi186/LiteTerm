use std::sync::{Arc, Mutex};
use std::time::Instant;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

mod terminal;
mod renderer;

use terminal::TerminalState;
use renderer::Renderer;

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    terminal: Arc<Mutex<TerminalState>>,
    cursor_visible: bool,
    cursor_timer: Instant,
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            renderer: None,
            terminal: Arc::new(Mutex::new(TerminalState::new())),
            cursor_visible: true,
            cursor_timer: Instant::now(),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("LiteTerm Native Prototype")
            .with_inner_size(winit::dpi::LogicalSize::new(1024.0, 768.0));
        let window = Arc::new(event_loop.create_window(attrs).unwrap());

        let renderer = pollster::block_on(Renderer::new(window.clone()));

        // 根据实际窗口大小计算初始 grid
        let (cols, rows) = renderer.calculate_grid_size();
        {
            let mut term = self.terminal.lock().unwrap();
            term.spawn_shell(cols, rows);
        }

        self.renderer = Some(renderer);

        let terminal = self.terminal.clone();
        let window_ref = window.clone();
        std::thread::spawn(move || {
            terminal::read_loop(terminal, move || {
                window_ref.request_redraw();
            });
        });

        // 光标闪烁定时器
        let window_blink = window.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(530));
                window_blink.request_redraw();
            }
        });

        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                    let (cols, rows) = renderer.calculate_grid_size();
                    let mut term = self.terminal.lock().unwrap();
                    term.resize(cols, rows);
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as i32,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y / 18.0) as i32,
                };
                if lines != 0 {
                    let mut term = self.terminal.lock().unwrap();
                    term.scroll(-lines);
                    if let Some(t) = term.term_mut() {
                        use alacritty_terminal::grid::Scroll;
                        if lines < 0 {
                            t.scroll_display(Scroll::Delta((-lines) as i32));
                        } else {
                            t.scroll_display(Scroll::Delta(-(lines) as i32));
                        }
                    }
                }
                if let Some(w) = &self.window { w.request_redraw(); }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                self.cursor_visible = true;
                self.cursor_timer = Instant::now();

                let modifiers = event.physical_key;

                // 特殊键转义序列
                let esc = match event.logical_key {
                    Key::Named(NamedKey::Enter)     => Some("\r"),
                    Key::Named(NamedKey::Backspace)  => Some("\x7f"),
                    Key::Named(NamedKey::Tab)        => Some("\t"),
                    Key::Named(NamedKey::Escape)     => Some("\x1b"),
                    Key::Named(NamedKey::ArrowUp)    => Some("\x1b[A"),
                    Key::Named(NamedKey::ArrowDown)  => Some("\x1b[B"),
                    Key::Named(NamedKey::ArrowRight) => Some("\x1b[C"),
                    Key::Named(NamedKey::ArrowLeft)  => Some("\x1b[D"),
                    Key::Named(NamedKey::Home)       => Some("\x1b[H"),
                    Key::Named(NamedKey::End)        => Some("\x1b[F"),
                    Key::Named(NamedKey::PageUp)     => Some("\x1b[5~"),
                    Key::Named(NamedKey::PageDown)   => Some("\x1b[6~"),
                    Key::Named(NamedKey::Delete)     => Some("\x1b[3~"),
                    Key::Named(NamedKey::Insert)     => Some("\x1b[2~"),
                    _ => None,
                };

                let mut term = self.terminal.lock().unwrap();
                if let Some(seq) = esc {
                    term.write_input(seq);
                } else if let Key::Character(ref ch) = event.logical_key {
                    // Ctrl+字母：winit 的 logical_key 仍是字母，需要手动转控制字符
                    // 检查 text 是否为控制字符（winit 在 Ctrl 按下时 text 为 \x01-\x1a）
                    if let Some(text) = &event.text {
                        let bytes = text.as_str().as_bytes();
                        if bytes.len() == 1 && bytes[0] <= 0x1a {
                            // Ctrl+A=0x01 ... Ctrl+Z=0x1a
                            term.write_input(text.as_str());
                            return;
                        }
                    }
                    term.write_input(ch.as_str());
                } else if let Some(text) = &event.text {
                    if !text.as_str().is_empty() {
                        term.write_input(text.as_str());
                    }
                }
                let _ = modifiers; // suppress warning
            }

            WindowEvent::RedrawRequested => {
                // 光标闪烁
                let elapsed = self.cursor_timer.elapsed().as_millis();
                if elapsed >= 530 {
                    self.cursor_visible = !self.cursor_visible;
                    self.cursor_timer = Instant::now();
                }

                if let Some(renderer) = &mut self.renderer {
                    let term = self.terminal.lock().unwrap();
                    renderer.render(&term, self.cursor_visible);
                }
            }

            _ => {}
        }
    }
}

fn main() {
    env_logger::init();
    let event_loop = EventLoop::new().unwrap();
    let mut app = App::new();
    event_loop.run_app(&mut app).unwrap();
}
