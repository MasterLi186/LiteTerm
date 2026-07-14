use std::sync::{Arc, Mutex};
use std::time::Instant;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, Modifiers, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    keyboard::{Key, KeyCode, NamedKey, PhysicalKey},
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
    modifiers: Modifiers,
    // 鼠标选择
    mouse_pressed: bool,
    selection_start: Option<(usize, usize)>, // (col, row)
    selection_end: Option<(usize, usize)>,
    clipboard: Option<arboard::Clipboard>,
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
            modifiers: Modifiers::default(),
            mouse_pressed: false,
            selection_start: None,
            selection_end: None,
            clipboard: arboard::Clipboard::new().ok(),
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
            renderer.render(&term, self.cursor_visible, self.selection_start, self.selection_end);
        }
    }

    /// 像素坐标 → 终端 cell 坐标
    fn pixel_to_cell(&self, x: f64, y: f64) -> (usize, usize) {
        if let Some(renderer) = &self.renderer {
            let (cw, ch) = renderer.cell_size();
            let col = (x as f32 / cw).floor().max(0.0) as usize;
            let row = (y as f32 / ch).floor().max(0.0) as usize;
            (col, row)
        } else {
            (0, 0)
        }
    }

    /// 从终端 grid 提取选区文本
    fn get_selection_text(&self) -> String {
        let (start, end) = match (self.selection_start, self.selection_end) {
            (Some(s), Some(e)) => {
                if (s.1, s.0) <= (e.1, e.0) { (s, e) } else { (e, s) }
            }
            _ => return String::new(),
        };

        let term = self.terminal.lock().unwrap();
        let t = match term.term() {
            Some(t) => t,
            None => return String::new(),
        };

        let mut result = String::new();
        let grid = t.grid();
        use alacritty_terminal::grid::Dimensions;
        let cols = grid.columns();

        for row in start.1..=end.1 {
            let line = alacritty_terminal::index::Line(row as i32);
            let start_col = if row == start.1 { start.0 } else { 0 };
            let end_col = if row == end.1 { end.0 } else { cols.saturating_sub(1) };

            for col in start_col..=end_col.min(cols.saturating_sub(1)) {
                let cell = &grid[line][alacritty_terminal::index::Column(col)];
                if cell.c != '\0' {
                    result.push(cell.c);
                }
            }
            if row != end.1 {
                // 去掉行尾空格后换行
                let trimmed = result.trim_end().len();
                result.truncate(trimmed);
                result.push('\n');
            }
        }
        result.trim_end().to_string()
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // 不主动持续请求重绘——依赖 PTY 线程和光标线程通过 proxy 按需唤醒
        // 避免 CPU 忙循环
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
    }

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

        let terminal = self.terminal.clone();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            terminal::read_loop(terminal, move || {
                let _ = proxy.send_event(UserEvent::Redraw);
            });
        });

        let proxy2 = self.proxy.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(530));
                if proxy2.send_event(UserEvent::Redraw).is_err() { break; }
            }
        });

        window.request_redraw();
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

            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods;
            }

            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    if state == ElementState::Pressed {
                        self.mouse_pressed = true;
                        // 清除旧选区（新位置在 CursorMoved 中设置）
                    } else {
                        self.mouse_pressed = false;
                        // 选区完成，复制到剪贴板
                        if self.selection_start.is_some() && self.selection_end.is_some() {
                            let text = self.get_selection_text();
                            if !text.is_empty() {
                                if let Some(cb) = &mut self.clipboard {
                                    let _ = cb.set_text(&text);
                                }
                            }
                        }
                    }
                }
                // 中键粘贴（X11 primary selection）
                if button == MouseButton::Middle && state == ElementState::Pressed {
                    if let Some(cb) = &mut self.clipboard {
                        if let Ok(text) = cb.get_text() {
                            let mut term = self.terminal.lock().unwrap();
                            term.write_input(&text);
                        }
                    }
                    self.do_render();
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                if self.mouse_pressed {
                    let cell = self.pixel_to_cell(position.x, position.y);
                    if self.selection_start.is_none() {
                        self.selection_start = Some(cell);
                        self.selection_end = Some(cell);
                    } else {
                        self.selection_end = Some(cell);
                    }
                    self.do_render();
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
                self.do_render();
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                self.cursor_visible = true;
                self.cursor_timer = Instant::now();

                let ctrl = self.modifiers.state().control_key();
                let shift = self.modifiers.state().shift_key();

                // Ctrl+Shift+C = 复制选区
                if ctrl && shift {
                    if let PhysicalKey::Code(KeyCode::KeyC) = event.physical_key {
                        let text = self.get_selection_text();
                        if !text.is_empty() {
                            if let Some(cb) = &mut self.clipboard {
                                let _ = cb.set_text(&text);
                            }
                        }
                        return;
                    }
                    // Ctrl+Shift+V = 粘贴
                    if let PhysicalKey::Code(KeyCode::KeyV) = event.physical_key {
                        if let Some(cb) = &mut self.clipboard {
                            if let Ok(text) = cb.get_text() {
                                let mut term = self.terminal.lock().unwrap();
                                term.write_input(&text);
                            }
                        }
                        self.do_render();
                        return;
                    }
                }

                // 清除选区（任意按键）
                self.selection_start = None;
                self.selection_end = None;

                // Ctrl+字母
                if ctrl {
                    if let PhysicalKey::Code(code) = event.physical_key {
                        let ctrl_byte = match code {
                            KeyCode::KeyA => 0x01, KeyCode::KeyB => 0x02,
                            KeyCode::KeyC => 0x03, KeyCode::KeyD => 0x04,
                            KeyCode::KeyE => 0x05, KeyCode::KeyF => 0x06,
                            KeyCode::KeyG => 0x07, KeyCode::KeyH => 0x08,
                            KeyCode::KeyI => 0x09, KeyCode::KeyJ => 0x0a,
                            KeyCode::KeyK => 0x0b, KeyCode::KeyL => 0x0c,
                            KeyCode::KeyM => 0x0d, KeyCode::KeyN => 0x0e,
                            KeyCode::KeyO => 0x0f, KeyCode::KeyP => 0x10,
                            KeyCode::KeyQ => 0x11, KeyCode::KeyR => 0x12,
                            KeyCode::KeyS => 0x13, KeyCode::KeyT => 0x14,
                            KeyCode::KeyU => 0x15, KeyCode::KeyV => 0x16,
                            KeyCode::KeyW => 0x17, KeyCode::KeyX => 0x18,
                            KeyCode::KeyY => 0x19, KeyCode::KeyZ => 0x1a,
                            _ => 0u8,
                        };
                        if ctrl_byte > 0 {
                            let mut term = self.terminal.lock().unwrap();
                            term.write_input(&String::from(ctrl_byte as char));
                            drop(term);
                            self.do_render();
                            return;
                        }
                    }
                }

                // 特殊键
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
