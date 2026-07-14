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
mod sidebar;
mod connections;
mod ssh;

use atlas::is_word_char;
use terminal::TerminalState;
use renderer::{GpuState, Renderer};
use sidebar::Sidebar;

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
    gpu: Option<GpuState>,
    renderer: Option<Renderer>,
    terminal: Arc<Mutex<TerminalState>>,
    cursor_visible: bool,
    cursor_timer: Instant,
    startup_time: Instant,
    proxy: EventLoopProxy<UserEvent>,
    modifiers: Modifiers,
    // egui
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,
    sidebar: Sidebar,
    sidebar_width: f32,
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
            gpu: None,
            renderer: None,
            terminal: Arc::new(Mutex::new(TerminalState::new())),
            cursor_visible: true,
            cursor_timer: Instant::now(),
            startup_time: Instant::now(),
            proxy,
            modifiers: Modifiers::default(),
            egui_ctx: {
                let ctx = egui::Context::default();
                // 加载中文字体
                let mut fonts = egui::FontDefinitions::default();
                if let Ok(font_data) = std::fs::read("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc") {
                    fonts.font_data.insert(
                        "noto_cjk".to_owned(),
                        egui::FontData::from_owned(font_data).into(),
                    );
                    fonts.families
                        .entry(egui::FontFamily::Proportional)
                        .or_default()
                        .push("noto_cjk".to_owned());
                    fonts.families
                        .entry(egui::FontFamily::Monospace)
                        .or_default()
                        .push("noto_cjk".to_owned());
                }
                ctx.set_fonts(fonts);
                ctx
            },
            egui_state: None,
            egui_renderer: None,
            sidebar: Sidebar::new(),
            sidebar_width: 220.0,
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

        let gpu = match &self.gpu {
            Some(g) => g,
            None => return,
        };
        let window = match &self.window {
            Some(w) => w.clone(),
            None => return,
        };

        // 1. Run egui (sidebar)
        let egui_input = self.egui_state.as_mut().unwrap().take_egui_input(&window);
        let egui_output = self.egui_ctx.run(egui_input, |ctx| {
            self.sidebar_width = self.sidebar.ui(ctx);
        });

        // Handle egui platform output (cursor changes etc)
        self.egui_state.as_mut().unwrap().handle_platform_output(&window, egui_output.platform_output);

        let paint_jobs = self.egui_ctx.tessellate(egui_output.shapes, egui_output.pixels_per_point);
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [gpu.width, gpu.height],
            pixels_per_point: window.scale_factor() as f32,
        };

        // 2. Get surface texture
        let output = match gpu.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Lost) => {
                if let Some(g) = &mut self.gpu {
                    g.resize(g.width, g.height);
                }
                return;
            }
            Err(e) => { log::warn!("Surface error: {:?}", e); return; }
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Main Encoder"),
        });

        // 3. Clear the entire surface with terminal background
        {
            let _clear_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: renderer::BG_DEFAULT[0] as f64 / 255.0,
                            g: renderer::BG_DEFAULT[1] as f64 / 255.0,
                            b: renderer::BG_DEFAULT[2] as f64 / 255.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        // 4. Render terminal cells (right of sidebar)
        if let Some(renderer) = &mut self.renderer {
            let term_width = (gpu.width as f32 - self.sidebar_width).max(1.0);
            renderer.set_viewport(self.sidebar_width, term_width, gpu.height as f32, gpu);

            let term = self.terminal.lock().unwrap();
            renderer.render_to_pass(gpu, &view, &mut encoder, &term, self.cursor_visible, self.selection_start, self.selection_end);
        }

        // 5. Render egui (sidebar overlay, on top)
        let egui_renderer = self.egui_renderer.as_mut().unwrap();
        for (id, delta) in &egui_output.textures_delta.set {
            egui_renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
        let _cmd_bufs = egui_renderer.update_buffers(&gpu.device, &gpu.queue, &mut encoder, &paint_jobs, &screen_descriptor);

        {
            let mut egui_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Egui Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            // egui render needs 'static lifetime on the render pass
            let egui_pass_static: wgpu::RenderPass<'static> = egui_pass.forget_lifetime();
            let mut egui_pass_static = egui_pass_static;
            egui_renderer.render(&mut egui_pass_static, &paint_jobs, &screen_descriptor);
        }

        for id in &egui_output.textures_delta.free {
            egui_renderer.free_texture(id);
        }

        // 6. Submit and present
        gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }

    /// Convert pixel position to terminal cell, accounting for sidebar offset
    fn pixel_to_cell(&self, x: f64, y: f64) -> (usize, usize) {
        if let Some(renderer) = &self.renderer {
            let (cw, ch) = renderer.cell_size();
            let term_x = (x as f32 - self.sidebar_width).max(0.0);
            let col = (term_x / cw).floor() as usize;
            let row = (y as f32 / ch).floor().max(0.0) as usize;
            (col, row)
        } else {
            (0, 0)
        }
    }

    /// Check if mouse is in terminal area (right of sidebar)
    fn is_in_terminal(&self, x: f64) -> bool {
        x as f32 >= self.sidebar_width
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

    fn select_word(&mut self, col: usize, row: usize) {
        let term = self.terminal.lock().unwrap();
        let t = match term.term() { Some(t) => t, None => return };
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

    fn select_line(&mut self, row: usize) {
        let term = self.terminal.lock().unwrap();
        let t = match term.term() { Some(t) => t, None => return };
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

    fn is_mouse_mode(&self) -> bool {
        let term = self.terminal.lock().unwrap();
        Renderer::is_mouse_mode(&term)
    }

    fn send_mouse_event(&mut self, btn: u32, col: usize, row: usize, pressed: bool) {
        let c = if pressed { 'M' } else { 'm' };
        let seq = format!("\x1b[<{};{};{}{}", btn, col + 1, row + 1, c);
        let mut term = self.terminal.lock().unwrap();
        term.write_input(&seq);
    }

    fn check_ssh_connect(&mut self) {
        if let Some(idx) = self.sidebar.on_connect.take() {
            if let Some(conn) = self.sidebar.connections.get(idx).cloned() {
                log::info!("SSH connect: {} ({}:{})", conn.label, conn.host, conn.port);

                // Get terminal size
                let (cols, rows) = if let Some(r) = &self.renderer {
                    r.calculate_grid_size()
                } else {
                    (80, 24)
                };

                // Replace current terminal with SSH session
                let mut term = self.terminal.lock().unwrap();
                let result = term.spawn_ssh(
                    &conn.host, conn.port, &conn.user, &conn.auth,
                    &conn.key_path, cols, rows,
                );
                drop(term);

                match result {
                    Ok(()) => {
                        log::info!("SSH connected to {}", conn.label);
                        // Start read loop for SSH
                        let terminal = self.terminal.clone();
                        let proxy = self.proxy.clone();
                        std::thread::spawn(move || {
                            terminal::read_loop(terminal, move || {
                                let _ = proxy.send_event(UserEvent::Redraw);
                            });
                        });
                    }
                    Err(e) => {
                        log::error!("SSH connect failed: {}", e);
                        // TODO: show error in UI
                    }
                }
            }
        }
    }

    fn sync_terminal_size(&mut self) {
        if let (Some(renderer), Some(gpu)) = (&mut self.renderer, &self.gpu) {
            let term_width = (gpu.width as f32 - self.sidebar_width).max(1.0);
            renderer.set_viewport(self.sidebar_width, term_width, gpu.height as f32, gpu);
            let (cols, rows) = renderer.calculate_grid_size();
            let mut term = self.terminal.lock().unwrap();
            term.resize(cols, rows);
        }
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
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(event_loop.create_window(attrs).unwrap());

        // Init GPU
        let gpu = pollster::block_on(GpuState::new(window.clone()));

        // Init egui
        let egui_state = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(&gpu.device, gpu.format(), None, 1, false);

        // Init terminal renderer
        let renderer = Renderer::new(&gpu);
        let term_width = (gpu.width as f32 - self.sidebar_width).max(1.0);
        let renderer_ref = &renderer;
        let _ = term_width; // will be set on first render

        // Spawn shell with initial size
        {
            let (cols, rows) = renderer.calculate_grid_size();
            let mut term = self.terminal.lock().unwrap();
            term.spawn_shell(cols, rows);
        }

        self.egui_state = Some(egui_state);
        self.egui_renderer = Some(egui_renderer);
        self.renderer = Some(renderer);
        self.gpu = Some(gpu);

        // PTY read thread
        let terminal = self.terminal.clone();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            terminal::read_loop(terminal, move || {
                let _ = proxy.send_event(UserEvent::Redraw);
            });
        });

        // Cursor blink thread
        let proxy2 = self.proxy.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(530));
                if proxy2.send_event(UserEvent::Redraw).is_err() { break; }
            }
        });

        self.window = Some(window);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: UserEvent) {
        self.do_render();
        self.check_ssh_connect();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Pass events to egui first
        if let Some(egui_state) = &mut self.egui_state {
            if let Some(window) = &self.window {
                let response = egui_state.on_window_event(window, &event);
                if response.consumed {
                    self.do_render();
                    self.check_ssh_connect();
                    return;
                }
            }
        }

        match event {
            WindowEvent::CloseRequested => { event_loop.exit(); }

            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
                self.sync_terminal_size();
                self.do_render();
            }

            WindowEvent::ModifiersChanged(mods) => { self.modifiers = mods; }

            WindowEvent::MouseInput { state, button, .. } => {
                // Only handle terminal clicks (right of sidebar)
                if !self.is_in_terminal(self.mouse_position.0) {
                    return;
                }

                let shift = self.modifiers.state().shift_key();
                let mouse_mode = self.is_mouse_mode();

                if button == MouseButton::Left {
                    let cell = self.pixel_to_cell(self.mouse_position.0, self.mouse_position.1);

                    if state == ElementState::Pressed {
                        if mouse_mode && !shift {
                            self.send_mouse_event(0, cell.0, cell.1, true);
                            self.mouse_pressed = true;
                            return;
                        }

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
                        if mouse_mode && !shift {
                            self.send_mouse_event(0, cell.0, cell.1, false);
                        }
                        self.mouse_pressed = false;
                        self.copy_selection();
                    }
                }

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

                if self.mouse_pressed && self.is_in_terminal(position.x) {
                    let cell = self.pixel_to_cell(position.x, position.y);
                    if mouse_mode && !shift {
                        self.send_mouse_event(32, cell.0, cell.1, true);
                    } else if self.click_state == ClickState::Single {
                        self.selection_end = Some(cell);
                        self.do_render();
                    }
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                if !self.is_in_terminal(self.mouse_position.0) { return; }

                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as i32,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y / 18.0) as i32,
                };
                if lines == 0 { return; }

                let mouse_mode = self.is_mouse_mode();
                if mouse_mode {
                    let cell = self.pixel_to_cell(self.mouse_position.0, self.mouse_position.1);
                    let btn = if lines > 0 { 64 } else { 65 };
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
                // 启动后 500ms 内忽略键盘（防止窗口创建时的虚假按键事件）
                if self.startup_time.elapsed().as_millis() < 500 { return; }
                self.cursor_visible = true;
                self.cursor_timer = Instant::now();

                let ctrl = self.modifiers.state().control_key();
                let shift = self.modifiers.state().shift_key();

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

                self.selection_start = None;
                self.selection_end = None;

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
                self.check_ssh_connect();
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
