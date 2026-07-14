use std::sync::{Arc, Mutex};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
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
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            renderer: None,
            terminal: Arc::new(Mutex::new(TerminalState::new())),
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

        // 初始化 GPU 渲染器
        let renderer = pollster::block_on(Renderer::new(window.clone()));
        self.renderer = Some(renderer);

        // 启动本地 shell
        {
            let mut term = self.terminal.lock().unwrap();
            term.spawn_shell(80, 24);
        }

        // 启动 PTY 读取线程
        let terminal = self.terminal.clone();
        let window_ref = window.clone();
        std::thread::spawn(move || {
            terminal::read_loop(terminal, move || {
                window_ref.request_redraw();
            });
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
                    // 同步终端尺寸
                    let (cols, rows) = renderer.calculate_grid_size();
                    let mut term = self.terminal.lock().unwrap();
                    term.resize(cols, rows);
                }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == winit::event::ElementState::Pressed {
                    if let Some(text) = &event.text {
                        let mut term = self.terminal.lock().unwrap();
                        term.write_input(text.as_str());
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                if let Some(renderer) = &mut self.renderer {
                    let term = self.terminal.lock().unwrap();
                    renderer.render(&term);
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
