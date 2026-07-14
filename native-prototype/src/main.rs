use std::sync::{Arc, Mutex};
use std::time::Instant;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

mod terminal;
mod renderer;
mod atlas;

use terminal::TerminalState;
use renderer::Renderer;

#[derive(Debug)]
enum UserEvent {
    Redraw,
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    terminal: Arc<Mutex<TerminalState>>,
    cursor_visible: bool,
    cursor_timer: Instant,
    proxy: EventLoopProxy<UserEvent>,
}

impl App {
    fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            window: None,
            renderer: None,
            terminal: Arc::new(Mutex::new(TerminalState::new())),
            cursor_visible: true,
            cursor_timer: Instant::now(),
            proxy,
        }
    }

    fn do_render(&mut self) {
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
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("LiteTerm Native Prototype")
            .with_inner_size(winit::dpi::LogicalSize::new(1024.0, 768.0));
        let window = Arc::new(event_loop.create_window(attrs).unwrap());

        let renderer = pollster::block_on(Renderer::new(window.clone()));
        let (cols, rows) = renderer.calculate_grid_size();
        {
            let mut term = self.terminal.lock().unwrap();
            term.spawn_shell(cols, rows);
        }
        self.renderer = Some(renderer);

        // PTY 读取线程 → 通过 proxy 唤醒事件循环
        let terminal = self.terminal.clone();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            terminal::read_loop(terminal, move || {
                let _ = proxy.send_event(UserEvent::Redraw);
            });
        });

        // 光标闪烁线程
        let proxy2 = self.proxy.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(530));
                if proxy2.send_event(UserEvent::Redraw).is_err() {
                    break;
                }
            }
        });

        self.window = Some(window);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: UserEvent) {
        self.do_render();
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
                self.do_render();
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
                self.do_render();
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                self.cursor_visible = true;
                self.cursor_timer = Instant::now();

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
                    if let Some(text) = &event.text {
                        let bytes = text.as_str().as_bytes();
                        if bytes.len() == 1 && bytes[0] <= 0x1a {
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
                drop(term);
                self.do_render();
            }

            WindowEvent::RedrawRequested => {
                self.do_render();
            }

            _ => {}
        }
    }
}

fn main() {
    env_logger::init();
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().unwrap();
    let proxy = event_loop.create_proxy();
    let mut app = App::new(proxy);
    event_loop.run_app(&mut app).unwrap();
}
