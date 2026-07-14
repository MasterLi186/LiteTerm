use std::sync::Arc;
use wgpu;
use winit::window::Window;

use crate::terminal::TerminalState;

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    width: u32,
    height: u32,
    cell_width: f32,
    cell_height: f32,
}

impl Renderer {
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance.create_surface(window).unwrap();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("无法获取 GPU adapter");

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .expect("无法获取 GPU device");

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // 暂用固定字符单元格大小（后续由 cosmic-text 测量）
        let cell_width = 8.0;
        let cell_height = 16.0;

        Self {
            surface,
            device,
            queue,
            config,
            width: size.width,
            height: size.height,
            cell_width,
            cell_height,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.width = width;
        self.height = height;
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn calculate_grid_size(&self) -> (u16, u16) {
        let cols = (self.width as f32 / self.cell_width).floor() as u16;
        let rows = (self.height as f32 / self.cell_height).floor() as u16;
        (cols.max(1), rows.max(1))
    }

    pub fn render(&mut self, terminal: &TerminalState) {
        let output = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Lost) => {
                self.resize(self.width, self.height);
                return;
            }
            Err(wgpu::SurfaceError::OutOfMemory) => {
                log::error!("GPU 显存不足");
                return;
            }
            Err(e) => {
                log::warn!("Surface error: {:?}", e);
                return;
            }
        };

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        {
            // 暂时只清屏为深色背景，证明 wgpu 管线工作
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.06,  // #0f1419 暗色背景
                            g: 0.08,
                            b: 0.10,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            // TODO: 渲染终端字符网格
            // 1. 遍历 terminal.term() 的 grid
            // 2. 用 cosmic-text 光栅化字符到纹理图集
            // 3. 用 instanced rendering 画字符四边形
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }
}
