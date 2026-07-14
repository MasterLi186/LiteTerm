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

use atlas::is_word_char;
use terminal::TerminalState;
use renderer::Renderer;

#[derive(Debug)]
enum UserEvent {
    Redraw,
}

#[derive(Clone, Copy, PartialEq)]
enum ClickState {
    None,
    Single,
    Double,
    Triple,
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    terminal: Arc<Mutex<TerminalState>>,
    cursor_visible: bool,
    cursor_timer: Instant,
    proxy: EventLoopProxy<UserEvent>,
    modifiers: Modifiers,
    // Mouse state
    mouse_pressed: bool,
    mouse_position: (f64, f64),
    selection_start: Option<(usize, usize)>,
    selection_end: Option<(usize, usize)>,
    clipboard: Option<arboard::Clipboard>,
    // Click detection
    last_click_time: Instant,
    last_click_pos: (usize, usize),
    click_state: ClickState,
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
            mouse_position: (0.0, 0.0),
            selection_start: None,
            selection_end: None,
            clipboard: arboard::Clipboard::new().ok(),
            last_click_time: Instant::now() - std::time::Duration::from_secs(10),
            last_click_pos: (0, 0),
            click_state: ClickState::None,
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
                let trimmed = result.trim_end().len();
                result.truncate(trimmed);
                result.push('\n');
            }
        }
        result.trim_end().to_string()
    }

    /// Double-click: select word at (col, row)
    fn select_word(&mut self, col: usize, row: usize) {
        let term = self.terminal.lock().unwrap();
        let t = match term.term() {
            Some(t) => t,
            None => return,
        };
        let grid = t.grid();
        use alacritty_terminal::grid::Dimensions;
        let cols = grid.columns();
        let line = alacritty_terminal::index::Line(row as i32);

        let center_char = grid[line][alacritty_terminal::index::Column(col)].c;
        if !is_word_char(center_char) && center_char != ' ' {
            self.selection_start = Some((col, row));
            self.selection_end = Some((col, row));
            return;
        }

        let mut start = col;
        while start > 0 {
            let c = grid[line][alacritty_terminal::index::Column(start - 1)].c;
            if !is_word_char(c) { break; }
            start -= 1;
        }
        let mut end = col;
        while end + 1 < cols {
            let c = grid[line][alacritty_terminal::index::Column(end + 1)].c;
            if !is_word_char(c) { break; }
            end += 1;
        }
        drop(term);
        self.selection_start = Some((start, row));
        self.selection_end = Some((end, row));
    }

    /// Triple-click: select entire line
    fn select_line(&mut self, row: usize) {
        let term = self.terminal.lock().unwrap();
        let t = match term.term() {
            Some(t) => t,
            None => return,
        };
        use alacritty_terminal::grid::Dimensions;
        let cols = t.grid().columns();
        drop(term);
        self.selection_start = Some((0, row));
        self.selection_end = Some((cols.saturating_sub(1), row));
    }

    fn copy_selection(&mut self) {
        let text = self.get_selection_text();
        if !text.is_empty() {
            if let Some(cb) = &mut self.clipboard {
                let _ = cb.set_text(&text);
            }
        }
    }

    /// Check if app is in mouse-reporting mode (vim, htop, etc.)
    fn is_mouse_mode(&self) -> bool {
        let term = self.terminal.lock().unwrap();
        Renderer::is_mouse_mode(&term)
    }

    /// Send SGR mouse event to PTY: \x1b[<btn;col;row;M or m
    fn send_mouse_event(&mut self, btn: u32, col: usize, row: usize, pressed: bool) {
        let c = if pressed { 'M' } else { 'm' };
        let seq = format!("\x1b[<{};{};{}{}", btn, col + 1, row + 1, c);
        let mut term = self.terminal.lock().unwrap();
        term.write_input(&seq);
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }
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
            WindowEvent::CloseRequested => { event_loop.exit(); }

            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                    let (cols, rows) = renderer.calculate_grid_size();
                    let mut term = self.terminal.lock().unwrap();
                    term.resize(cols, rows);
                }
                self.do_render();
            }

            WindowEvent::ModifiersChanged(mods) => { self.modifiers = mods; }

            WindowEvent::MouseInput { state, button, .. } => {
                let shift = self.modifiers.state().shift_key();
                let mouse_mode = self.is_mouse_mode();

                if button == MouseButton::Left {
                    let cell = self.pixel_to_cell(self.mouse_position.0, self.mouse_position.1);

                    if state == ElementState::Pressed {
                        // Mouse reporting (unless Shift is held to force selection)
                        if mouse_mode && !shift {
                            self.send_mouse_event(0, cell.0, cell.1, true);
                            self.mouse_pressed = true;
                            return;
                        }

                        // Click detection: single / double / triple
                        let now = Instant::now();
                        let elapsed = now.duration_since(self.last_click_time).as_millis();
                        let same_pos = cell == self.last_click_pos;

                        if elapsed < 400 && same_pos {
                            match self.click_state {
                                ClickState::Single => {
                                    self.click_state = ClickState::Double;
                                    self.select_word(cell.0, cell.1);
                                }
                                ClickState::Double => {
                                    self.click_state = ClickState::Triple;
                                    self.select_line(cell.1);
                                }
                                _ => {
                                    self.click_state = ClickState::Single;
                                    self.selection_start = Some(cell);
                                    self.selection_end = Some(cell);
                                }
                            }
                        } else {
                            self.click_state = ClickState::Single;
                            self.selection_start = Some(cell);
                            self.selection_end = Some(cell);
                        }
                        self.last_click_time = now;
                        self.last_click_pos = cell;
                        self.mouse_pressed = true;
                        self.do_render();
                    } else {
                        // Release
                        if mouse_mode && !shift {
                            self.send_mouse_event(0, cell.0, cell.1, false);
                        }
                        self.mouse_pressed = false;
                        self.copy_selection();
                    }
                }

                // Middle-click paste
                if button == MouseButton::Middle && state == ElementState::Pressed {
                    if mouse_mode && !shift {
                        let cell = self.pixel_to_cell(self.mouse_position.0, self.mouse_position.1);
                        self.send_mouse_event(1, cell.0, cell.1, true);
                    } else {
                        if let Some(cb) = &mut self.clipboard {
                            if let Ok(text) = cb.get_text() {
                                let mut term = self.terminal.lock().unwrap();
                                term.write_input(&text);
                            }
                        }
                    }
                    self.do_render();
                }

                // Right-click
                if button == MouseButton::Right && state == ElementState::Pressed {
                    if mouse_mode && !shift {
                        let cell = self.pixel_to_cell(self.mouse_position.0, self.mouse_position.1);
                        self.send_mouse_event(2, cell.0, cell.1, true);
                    }
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_position = (position.x, position.y);
                let shift = self.modifiers.state().shift_key();
                let mouse_mode = self.is_mouse_mode();

                if self.mouse_pressed {
                    let cell = self.pixel_to_cell(position.x, position.y);

                    if mouse_mode && !shift {
                        // Mouse motion reporting (SGR drag: button 32+btn)
                        self.send_mouse_event(32, cell.0, cell.1, true);
                    } else if self.click_state == ClickState::Single {
                        // Drag selection
                        self.selection_end = Some(cell);
                        self.do_render();
                    }
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as i32,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y / 18.0) as i32,
                };
                if lines == 0 { return; }

                let mouse_mode = self.is_mouse_mode();
                if mouse_mode {
                    let cell = self.pixel_to_cell(self.mouse_position.0, self.mouse_position.1);
                    let btn = if lines > 0 { 64 } else { 65 }; // scroll up / down
                    for _ in 0..lines.unsigned_abs() {
                        self.send_mouse_event(btn, cell.0, cell.1, true);
                    }
                } else {
                    let mut term = self.terminal.lock().unwrap();
                    term.scroll(-lines);
                    if let Some(t) = term.term_mut() {
                        use alacritty_terminal::grid::Scroll;
                        t.scroll_display(Scroll::Delta(-lines));
                    }
                }
                self.do_render();
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed { return; }
                self.cursor_visible = true;
                self.cursor_timer = Instant::now();

                let ctrl = self.modifiers.state().control_key();
                let shift = self.modifiers.state().shift_key();

                // Ctrl+Shift+C = copy
                if ctrl && shift {
                    if let PhysicalKey::Code(KeyCode::KeyC) = event.physical_key {
                        self.copy_selection();
                        return;
                    }
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

                // Clear selection on any key
                self.selection_start = None;
                self.selection_end = None;

                // Ctrl+letter
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

                let esc = match event.logical_key {
                    Key::Named(NamedKey::Enter)      => Some("\r"),
                    Key::Named(NamedKey::Backspace)   => Some("\x7f"),
                    Key::Named(NamedKey::Tab)         => Some("\t"),
                    Key::Named(NamedKey::Escape)      => Some("\x1b"),
                    Key::Named(NamedKey::ArrowUp)     => Some("\x1b[A"),
                    Key::Named(NamedKey::ArrowDown)   => Some("\x1b[B"),
                    Key::Named(NamedKey::ArrowRight)  => Some("\x1b[C"),
                    Key::Named(NamedKey::ArrowLeft)   => Some("\x1b[D"),
                    Key::Named(NamedKey::Home)        => Some("\x1b[H"),
                    Key::Named(NamedKey::End)         => Some("\x1b[F"),
                    Key::Named(NamedKey::PageUp)      => Some("\x1b[5~"),
                    Key::Named(NamedKey::PageDown)    => Some("\x1b[6~"),
                    Key::Named(NamedKey::Delete)      => Some("\x1b[3~"),
                    Key::Named(NamedKey::Insert)      => Some("\x1b[2~"),
                    Key::Named(NamedKey::F1)          => Some("\x1bOP"),
                    Key::Named(NamedKey::F2)          => Some("\x1bOQ"),
                    Key::Named(NamedKey::F3)          => Some("\x1bOR"),
                    Key::Named(NamedKey::F4)          => Some("\x1bOS"),
                    Key::Named(NamedKey::F5)          => Some("\x1b[15~"),
                    Key::Named(NamedKey::F6)          => Some("\x1b[17~"),
                    Key::Named(NamedKey::F7)          => Some("\x1b[18~"),
                    Key::Named(NamedKey::F8)          => Some("\x1b[19~"),
                    Key::Named(NamedKey::F9)          => Some("\x1b[20~"),
                    Key::Named(NamedKey::F10)         => Some("\x1b[21~"),
                    Key::Named(NamedKey::F11)         => Some("\x1b[23~"),
                    Key::Named(NamedKey::F12)         => Some("\x1b[24~"),
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

            WindowEvent::RedrawRequested => { self.do_render(); }
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
