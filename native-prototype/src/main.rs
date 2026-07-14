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
mod tab_manager;
mod tab_bar;

use atlas::is_word_char;
use terminal::TerminalState;
use renderer::{GpuState, Renderer};
use sidebar::Sidebar;
use tab_manager::TabManager;

enum UserEvent {
    Redraw,
    SshReady { tab_id: String, result: Result<crate::ssh::SshHandle, String> },
}

impl std::fmt::Debug for UserEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserEvent::Redraw => write!(f, "Redraw"),
            UserEvent::SshReady { tab_id, result } => {
                let status = if result.is_ok() { "Ok" } else { "Err" };
                write!(f, "SshReady({}, {})", tab_id, status)
            }
        }
    }
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
    tab_manager: TabManager,
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
    tab_bar_height: f32,
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
            tab_manager: TabManager::new(),
            cursor_visible: true,
            cursor_timer: Instant::now(),
            startup_time: Instant::now(),
            proxy,
            modifiers: Modifiers::default(),
            egui_ctx: {
                let ctx = egui::Context::default();
                let mut fonts = egui::FontDefinitions::default();
                if let Ok(font_data) = std::fs::read("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc") {
                    fonts.font_data.insert(
                        "noto_cjk".to_owned(),
                        egui::FontData::from_owned(font_data).into(),
                    );
                    fonts.families.entry(egui::FontFamily::Proportional).or_default().push("noto_cjk".to_owned());
                    fonts.families.entry(egui::FontFamily::Monospace).or_default().push("noto_cjk".to_owned());
                }
                ctx.set_fonts(fonts);
                ctx
            },
            egui_state: None,
            egui_renderer: None,
            sidebar: Sidebar::new(),
            sidebar_width: 220.0,
            tab_bar_height: tab_bar::TAB_BAR_HEIGHT,
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

    /// Get the active terminal, if any
    fn active_terminal(&self) -> Option<Arc<Mutex<TerminalState>>> {
        self.tab_manager.active_terminal()
    }

    /// Start a read_loop for a terminal on a background thread
    fn start_read_loop(&self, terminal: Arc<Mutex<TerminalState>>) {
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            terminal::read_loop(terminal, move || {
                let _ = proxy.send_event(UserEvent::Redraw);
            });
        });
    }

    /// Create a new local terminal tab with default shell
    fn new_local_tab(&mut self) {
        let (cols, rows) = self.grid_size();
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let (_id, terminal) = self.tab_manager.new_local(&shell, cols, rows);
        self.start_read_loop(terminal);
    }

    /// Create a new SSH tab (connects in background)
    fn new_ssh_tab(&mut self, conn: &sidebar::SshConnection) {
        let tab_id = self.tab_manager.new_ssh_placeholder(conn);
        let (cols, rows) = self.grid_size();

        let host = conn.host.clone();
        let port = conn.port;
        let user = conn.user.clone();
        let auth = conn.auth.clone();
        let key_path = conn.key_path.clone();
        let tid = tab_id.clone();
        let proxy = self.proxy.clone();

        std::thread::spawn(move || {
            let kp = if key_path.is_empty() { None } else { Some(key_path.as_str()) };
            let result = crate::ssh::connect(&host, port, &user, &auth, kp, cols, rows);
            let _ = proxy.send_event(UserEvent::SshReady { tab_id: tid, result });
        });
    }

    fn grid_size(&self) -> (u16, u16) {
        if let Some(r) = &self.renderer { r.calculate_grid_size() } else { (80, 24) }
    }

    fn do_render(&mut self) {
        let elapsed = self.cursor_timer.elapsed().as_millis();
        if elapsed >= 530 {
            self.cursor_visible = !self.cursor_visible;
            self.cursor_timer = Instant::now();
        }

        let gpu = match &self.gpu { Some(g) => g, None => return };
        let window = match &self.window { Some(w) => w.clone(), None => return };

        // 1. Run egui (tab bar + sidebar + dialogs)
        let egui_input = self.egui_state.as_mut().unwrap().take_egui_input(&window);
        let mut tab_action = tab_bar::TabBarAction { switch_to: None, close: None, new_tab: false };
        let egui_output = self.egui_ctx.run(egui_input, |ctx| {
            tab_action = tab_bar::render_tab_bar(ctx, &self.tab_manager);
            self.sidebar_width = self.sidebar.ui(ctx);
        });

        self.egui_state.as_mut().unwrap().handle_platform_output(&window, egui_output.platform_output);

        let paint_jobs = self.egui_ctx.tessellate(egui_output.shapes, egui_output.pixels_per_point);
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [gpu.width, gpu.height],
            pixels_per_point: window.scale_factor() as f32,
        };

        // 2. Handle tab bar actions (non-mutating ones only here)
        if let Some(idx) = tab_action.switch_to {
            self.tab_manager.switch_to(idx);
            self.selection_start = None;
            self.selection_end = None;
        }
        // defer close/new_tab to after render (needs &mut self without gpu borrow)
        let deferred_close = tab_action.close;
        let deferred_new = tab_action.new_tab;

        // 3. Get surface texture
        let output = match gpu.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Lost) => {
                if let Some(g) = &mut self.gpu { g.resize(g.width, g.height); }
                return;
            }
            Err(e) => { log::warn!("Surface error: {:?}", e); return; }
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Main Encoder"),
        });

        // 4. Clear surface
        {
            let _clear_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view, resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: renderer::BG_DEFAULT[0] as f64 / 255.0,
                            g: renderer::BG_DEFAULT[1] as f64 / 255.0,
                            b: renderer::BG_DEFAULT[2] as f64 / 255.0, a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None,
            });
        }

        // 5. Render active terminal
        if let Some(terminal) = self.active_terminal() {
            if let Some(renderer) = &mut self.renderer {
                let term_width = (gpu.width as f32 - self.sidebar_width).max(1.0);
                let term_height = (gpu.height as f32 - self.tab_bar_height).max(1.0);
                renderer.set_viewport(self.sidebar_width, self.tab_bar_height, term_width, term_height, gpu);

                let term = terminal.lock().unwrap();
                renderer.render_to_pass(gpu, &view, &mut encoder, &term, self.cursor_visible, self.selection_start, self.selection_end);
            }
        }

        // 6. Render egui overlay
        let egui_renderer = self.egui_renderer.as_mut().unwrap();
        for (id, delta) in &egui_output.textures_delta.set {
            egui_renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
        let _cmd_bufs = egui_renderer.update_buffers(&gpu.device, &gpu.queue, &mut encoder, &paint_jobs, &screen_descriptor);

        {
            let mut egui_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Egui Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None,
            });
            let egui_pass_static: wgpu::RenderPass<'static> = egui_pass.forget_lifetime();
            let mut egui_pass_static = egui_pass_static;
            egui_renderer.render(&mut egui_pass_static, &paint_jobs, &screen_descriptor);
        }

        for id in &egui_output.textures_delta.free {
            egui_renderer.free_texture(id);
        }

        gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        // Deferred tab actions (after gpu borrow ends)
        if let Some(idx) = deferred_close {
            eprintln!("[MAIN] deferred close tab {}", idx);
            self.tab_manager.close(idx);
            if self.tab_manager.is_empty() {
                self.new_local_tab();
            }
        }
        if deferred_new {
            eprintln!("[MAIN] deferred new tab");
            self.new_local_tab();
        }
    }

    fn pixel_to_cell(&self, x: f64, y: f64) -> (usize, usize) {
        if let Some(renderer) = &self.renderer {
            let (cw, ch) = renderer.cell_size();
            let term_x = (x as f32 - self.sidebar_width).max(0.0);
            let term_y = (y as f32 - self.tab_bar_height).max(0.0);
            let col = (term_x / cw).floor() as usize;
            let row = (term_y / ch).floor() as usize;
            (col, row)
        } else { (0, 0) }
    }

    fn is_in_terminal(&self, x: f64, y: f64) -> bool {
        x as f32 >= self.sidebar_width && y as f32 >= self.tab_bar_height
    }

    fn get_selection_text(&self) -> String {
        let (start, end) = match (self.selection_start, self.selection_end) {
            (Some(s), Some(e)) => { if (s.1, s.0) <= (e.1, e.0) { (s, e) } else { (e, s) } }
            _ => return String::new(),
        };
        let terminal = match self.active_terminal() { Some(t) => t, None => return String::new() };
        let term = terminal.lock().unwrap();
        let t = match term.term() { Some(t) => t, None => return String::new() };
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
                if cell.c != '\0' { result.push(cell.c); }
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
        let terminal = match self.active_terminal() { Some(t) => t, None => return };
        let term = terminal.lock().unwrap();
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
        drop(terminal);
        self.selection_start = Some((start, row));
        self.selection_end = Some((end, row));
    }

    fn select_line(&mut self, row: usize) {
        let terminal = match self.active_terminal() { Some(t) => t, None => return };
        let term = terminal.lock().unwrap();
        let t = match term.term() { Some(t) => t, None => return };
        use alacritty_terminal::grid::Dimensions;
        let cols = t.grid().columns();
        drop(term);
        drop(terminal);
        self.selection_start = Some((0, row));
        self.selection_end = Some((cols.saturating_sub(1), row));
    }

    fn copy_selection(&mut self) {
        let text = self.get_selection_text();
        if !text.is_empty() {
            if let Some(cb) = &mut self.clipboard { let _ = cb.set_text(&text); }
        }
    }

    fn is_mouse_mode(&self) -> bool {
        let terminal = match self.active_terminal() { Some(t) => t, None => return false };
        let term = terminal.lock().unwrap();
        Renderer::is_mouse_mode(&term)
    }

    fn send_mouse_event(&mut self, btn: u32, col: usize, row: usize, pressed: bool) {
        let c = if pressed { 'M' } else { 'm' };
        let seq = format!("\x1b[<{};{};{}{}", btn, col + 1, row + 1, c);
        if let Some(terminal) = self.active_terminal() {
            let mut term = terminal.lock().unwrap();
            term.write_input(&seq);
        }
    }

    fn check_ssh_connect(&mut self) {
        if let Some(conn) = self.sidebar.take_connect() {
            self.new_ssh_tab(&conn);
        }
    }

    fn sync_terminal_size(&mut self) {
        if let (Some(renderer), Some(gpu)) = (&mut self.renderer, &self.gpu) {
            let term_width = (gpu.width as f32 - self.sidebar_width).max(1.0);
            let term_height = (gpu.height as f32 - self.tab_bar_height).max(1.0);
            renderer.set_viewport(self.sidebar_width, self.tab_bar_height, term_width, term_height, gpu);
            let (cols, rows) = renderer.calculate_grid_size();
            // Resize ALL tabs' terminals
            for tab in &self.tab_manager.tabs {
                let mut term = tab.terminal.lock().unwrap();
                term.resize(cols, rows);
            }
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
            .with_title("LiteTerm Native")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(event_loop.create_window(attrs).unwrap());

        let gpu = pollster::block_on(GpuState::new(window.clone()));
        let egui_state = egui_winit::State::new(
            self.egui_ctx.clone(), egui::ViewportId::ROOT, &window,
            Some(window.scale_factor() as f32), None,
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(&gpu.device, gpu.format(), None, 1, false);
        let renderer = Renderer::new(&gpu);

        self.egui_state = Some(egui_state);
        self.egui_renderer = Some(egui_renderer);
        self.renderer = Some(renderer);
        self.gpu = Some(gpu);

        // Create initial local terminal tab
        self.new_local_tab();

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

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Redraw => {}
            UserEvent::SshReady { tab_id, result } => {
                match result {
                    Ok(handle) => {
                        eprintln!("[SSH] 连接成功: {}", tab_id);
                        let (cols, rows) = self.grid_size();
                        if let Some(terminal) = self.tab_manager.apply_ssh(& tab_id, handle, cols, rows) {
                            self.start_read_loop(terminal);
                        }
                    }
                    Err(e) => {
                        eprintln!("[SSH] 连接失败: {}: {}", tab_id, e);
                        self.tab_manager.ssh_failed(&tab_id, &e);
                    }
                }
            }
        }
        self.do_render();
        self.check_ssh_connect();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Pass events to egui first
        let mut egui_consumed = false;
        if let Some(egui_state) = &mut self.egui_state {
            if let Some(window) = &self.window {
                let response = egui_state.on_window_event(window, &event);
                if response.consumed {
                    egui_consumed = true;
                    self.do_render();
                    self.check_ssh_connect();
                    // 滚轮和键盘事件即使 egui 消费了也要继续传给终端
                    let pass_through = matches!(event,
                        WindowEvent::MouseWheel { .. } | WindowEvent::KeyboardInput { .. }
                    ) && self.is_in_terminal(self.mouse_position.0, self.mouse_position.1);
                    if !pass_through {
                        return;
                    }
                }
            }
        }

        match event {
            WindowEvent::CloseRequested => { event_loop.exit(); }

            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu { gpu.resize(size.width, size.height); }
                self.sync_terminal_size();
                self.do_render();
            }

            WindowEvent::ModifiersChanged(mods) => { self.modifiers = mods; }

            WindowEvent::MouseInput { state, button, .. } => {
                if !self.is_in_terminal(self.mouse_position.0, self.mouse_position.1) { return; }

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
                                ClickState::Single => { self.click_state = ClickState::Double; self.select_word(cell.0, cell.1); }
                                ClickState::Double => { self.click_state = ClickState::Triple; self.select_line(cell.1); }
                                _ => { self.click_state = ClickState::Single; self.selection_start = Some(cell); self.selection_end = Some(cell); }
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
                        if mouse_mode && !shift { self.send_mouse_event(0, cell.0, cell.1, false); }
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
                                if let Some(terminal) = self.active_terminal() {
                                    let mut term = terminal.lock().unwrap();
                                    term.write_input(&text);
                                }
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
                if self.mouse_pressed && self.is_in_terminal(position.x, position.y) {
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
                if !self.is_in_terminal(self.mouse_position.0, self.mouse_position.1) { return; }
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as i32,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y / 18.0) as i32,
                };
                if lines == 0 { return; }
                let mouse_mode = self.is_mouse_mode();
                if mouse_mode {
                    let cell = self.pixel_to_cell(self.mouse_position.0, self.mouse_position.1);
                    let btn = if lines > 0 { 64 } else { 65 };
                    for _ in 0..lines.unsigned_abs() { self.send_mouse_event(btn, cell.0, cell.1, true); }
                } else if let Some(terminal) = self.active_terminal() {
                    let mut term = terminal.lock().unwrap();
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
                if self.startup_time.elapsed().as_millis() < 500 { return; }
                self.cursor_visible = true;
                self.cursor_timer = Instant::now();

                let ctrl = self.modifiers.state().control_key();
                let shift = self.modifiers.state().shift_key();

                // Tab management shortcuts
                if ctrl && shift {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::KeyT) => { self.new_local_tab(); self.do_render(); return; }
                        PhysicalKey::Code(KeyCode::KeyW) => {
                            let idx = self.tab_manager.active_idx;
                            self.tab_manager.close(idx);
                            if self.tab_manager.is_empty() { self.new_local_tab(); }
                            self.do_render();
                            return;
                        }
                        PhysicalKey::Code(KeyCode::KeyC) => { self.copy_selection(); return; }
                        PhysicalKey::Code(KeyCode::KeyV) => {
                            if let Some(cb) = &mut self.clipboard {
                                if let Ok(text) = cb.get_text() {
                                    if let Some(terminal) = self.active_terminal() {
                                        let mut term = terminal.lock().unwrap();
                                        term.write_input(&text);
                                    }
                                }
                            }
                            self.do_render();
                            return;
                        }
                        PhysicalKey::Code(KeyCode::Tab) => { self.tab_manager.prev_tab(); self.do_render(); return; }
                        _ => {}
                    }
                }

                // Ctrl+Tab = next tab
                if ctrl && !shift {
                    if let Key::Named(NamedKey::Tab) = event.logical_key {
                        self.tab_manager.next_tab();
                        self.do_render();
                        return;
                    }
                    // Ctrl+1~9 switch to tab N
                    if let PhysicalKey::Code(code) = event.physical_key {
                        let tab_num = match code {
                            KeyCode::Digit1 => Some(0),
                            KeyCode::Digit2 => Some(1),
                            KeyCode::Digit3 => Some(2),
                            KeyCode::Digit4 => Some(3),
                            KeyCode::Digit5 => Some(4),
                            KeyCode::Digit6 => Some(5),
                            KeyCode::Digit7 => Some(6),
                            KeyCode::Digit8 => Some(7),
                            KeyCode::Digit9 => Some(8),
                            _ => None,
                        };
                        if let Some(n) = tab_num {
                            if n < self.tab_manager.len() {
                                self.tab_manager.switch_to(n);
                                self.do_render();
                                return;
                            }
                        }
                    }
                }

                self.selection_start = None;
                self.selection_end = None;

                // Ctrl+letter → control character
                if ctrl && !shift {
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
                            if let Some(terminal) = self.active_terminal() {
                                let mut term = terminal.lock().unwrap();
                                term.write_input(&String::from(ctrl_byte as char));
                            }
                            self.do_render();
                            return;
                        }
                    }
                }

                // Special keys
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

                if let Some(terminal) = self.active_terminal() {
                    let mut term = terminal.lock().unwrap();
                    if let Some(seq) = esc {
                        term.write_input(seq);
                    } else if let Some(text) = &event.text {
                        if !text.as_str().is_empty() {
                            term.write_input(text.as_str());
                        }
                    }
                }
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
