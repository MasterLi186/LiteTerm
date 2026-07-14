use std::sync::Arc;
use wgpu;
use wgpu::util::DeviceExt;
use winit::window::Window;
use cosmic_text::{
    Attrs, Buffer, Color as CColor, FontSystem, Metrics, Shaping, SwashCache,
};

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
    font_system: FontSystem,
    swash_cache: SwashCache,
    // 全屏四边形管线
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    vertex_buffer: wgpu::Buffer,
    // CPU 帧缓冲
    framebuffer: Vec<u8>,
    fb_width: u32,
    fb_height: u32,
}

// 全屏四边形顶点
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    tex_coords: [f32; 2],
}

const VERTICES: &[Vertex] = &[
    Vertex { position: [-1.0, -1.0], tex_coords: [0.0, 1.0] },
    Vertex { position: [ 1.0, -1.0], tex_coords: [1.0, 1.0] },
    Vertex { position: [-1.0,  1.0], tex_coords: [0.0, 0.0] },
    Vertex { position: [ 1.0, -1.0], tex_coords: [1.0, 1.0] },
    Vertex { position: [ 1.0,  1.0], tex_coords: [1.0, 0.0] },
    Vertex { position: [-1.0,  1.0], tex_coords: [0.0, 0.0] },
];

const SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) tex_coords: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.tex_coords = tex_coords;
    return out;
}

@group(0) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(0) @binding(1)
var s_diffuse: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(t_diffuse, s_diffuse, in.tex_coords);
}
"#;

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

        // 字体系统
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();

        // 测量单元格大小
        let font_size = 15.0;
        let metrics = Metrics::new(font_size, font_size * 1.2);
        let mut measure_buf = Buffer::new(&mut font_system, metrics);
        measure_buf.set_size(&mut font_system, Some(200.0), Some(50.0));
        measure_buf.set_text(&mut font_system, "W", Attrs::new(), Shaping::Advanced);
        measure_buf.shape_until_scroll(&mut font_system, false);
        let cell_width = font_size * 0.6; // 等宽字体近似
        let cell_height = metrics.line_height;

        // 创建渲染管线
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let w = size.width.max(1);
        let h = size.height.max(1);
        let framebuffer = vec![0u8; (w * h * 4) as usize];

        Self {
            surface,
            device,
            queue,
            config,
            width: w,
            height: h,
            cell_width,
            cell_height,
            font_system,
            swash_cache,
            pipeline,
            bind_group_layout,
            vertex_buffer,
            framebuffer,
            fb_width: w,
            fb_height: h,
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
        self.fb_width = width;
        self.fb_height = height;
        self.framebuffer = vec![0u8; (width * height * 4) as usize];
    }

    pub fn calculate_grid_size(&self) -> (u16, u16) {
        let cols = (self.width as f32 / self.cell_width).floor() as u16;
        let rows = (self.height as f32 / self.cell_height).floor() as u16;
        (cols.max(1), rows.max(1))
    }

    /// 从 alacritty Color 转为 RGBA
    fn color_to_rgba(color: &alacritty_terminal::term::color::Colors, c: alacritty_terminal::vte::ansi::Color) -> [u8; 4] {
        use alacritty_terminal::vte::ansi::Color as AC;
        match c {
            AC::Spec(rgb) => [rgb.r, rgb.g, rgb.b, 255],
            AC::Named(named) => {
                if let Some(rgb) = color[named] {
                    [rgb.r, rgb.g, rgb.b, 255]
                } else {
                    // 默认 ANSI 颜色
                    let idx = named as usize;
                    let defaults: [[u8; 3]; 16] = [
                        [0,0,0],[205,49,49],[13,188,121],[229,229,16],
                        [36,114,200],[188,63,188],[17,168,205],[229,229,229],
                        [102,102,102],[241,76,76],[35,209,139],[245,245,67],
                        [59,142,234],[214,112,214],[41,184,219],[229,229,229],
                    ];
                    if idx < 16 { [defaults[idx][0], defaults[idx][1], defaults[idx][2], 255] }
                    else { [229, 229, 229, 255] }
                }
            }
            AC::Indexed(idx) => {
                if let Some(rgb) = color[idx as usize] {
                    [rgb.r, rgb.g, rgb.b, 255]
                } else {
                    [229, 229, 229, 255]
                }
            }
        }
    }

    /// CPU 渲染终端内容到帧缓冲
    fn render_to_framebuffer(&mut self, terminal: &TerminalState) {
        let bg_color: [u8; 4] = [15, 20, 25, 255]; // 暗色背景
        // 清屏
        for pixel in self.framebuffer.chunks_exact_mut(4) {
            pixel.copy_from_slice(&bg_color);
        }

        let term = match terminal.term() {
            Some(t) => t,
            None => return,
        };

        let content = term.renderable_content();
        let cw = self.cell_width;
        let ch = self.cell_height;
        let font_size = 15.0;
        let metrics = Metrics::new(font_size, ch);

        for indexed in content.display_iter {
            let cell = &indexed.cell;
            let col = indexed.point.column.0;
            let row = indexed.point.line.0 as usize;

            let x = (col as f32 * cw) as u32;
            let y = (row as f32 * ch) as u32;

            if cell.c == ' ' || cell.c == '\0' {
                continue;
            }

            // 用 cosmic-text 光栅化单个字符
            let mut buf = Buffer::new(&mut self.font_system, metrics);
            buf.set_size(&mut self.font_system, Some(cw * 2.0), Some(ch * 2.0));
            let attrs = Attrs::new().family(cosmic_text::Family::Monospace);
            let mut s = [0u8; 4];
            let ch_str = cell.c.encode_utf8(&mut s);
            buf.set_text(&mut self.font_system, ch_str, attrs, Shaping::Advanced);
            buf.shape_until_scroll(&mut self.font_system, false);

            let fg = Self::color_to_rgba(content.colors, cell.fg);

            // 绘制字形
            for run in buf.layout_runs() {
                for glyph in run.glyphs.iter() {
                    let physical = glyph.physical((0.0, 0.0), 1.0);
                    if let Some(image) = self.swash_cache.get_image(&mut self.font_system, physical.cache_key) {
                        let gx = x as i32 + physical.x + image.placement.left;
                        let gy = y as i32 + physical.y - image.placement.top + font_size as i32;

                        for iy in 0..image.placement.height as i32 {
                            for ix in 0..image.placement.width as i32 {
                                let px = gx + ix;
                                let py = gy + iy;
                                if px < 0 || py < 0 || px >= self.fb_width as i32 || py >= self.fb_height as i32 {
                                    continue;
                                }
                                let src_idx = (iy * image.placement.width as i32 + ix) as usize;
                                if src_idx >= image.data.len() { continue; }
                                let alpha = image.data[src_idx];
                                if alpha == 0 { continue; }

                                let dst_idx = ((py as u32 * self.fb_width + px as u32) * 4) as usize;
                                if dst_idx + 3 >= self.framebuffer.len() { continue; }

                                // Alpha blend
                                let a = alpha as f32 / 255.0;
                                let inv_a = 1.0 - a;
                                self.framebuffer[dst_idx]     = (fg[0] as f32 * a + self.framebuffer[dst_idx] as f32 * inv_a) as u8;
                                self.framebuffer[dst_idx + 1] = (fg[1] as f32 * a + self.framebuffer[dst_idx + 1] as f32 * inv_a) as u8;
                                self.framebuffer[dst_idx + 2] = (fg[2] as f32 * a + self.framebuffer[dst_idx + 2] as f32 * inv_a) as u8;
                                self.framebuffer[dst_idx + 3] = 255;
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn render(&mut self, terminal: &TerminalState) {
        // CPU 渲染到帧缓冲
        self.render_to_framebuffer(terminal);

        // 上传帧缓冲到 GPU 纹理
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Terminal Texture"),
            size: wgpu::Extent3d {
                width: self.fb_width,
                height: self.fb_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.framebuffer,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * self.fb_width),
                rows_per_image: Some(self.fb_height),
            },
            wgpu::Extent3d {
                width: self.fb_width,
                height: self.fb_height,
                depth_or_array_layers: 1,
            },
        );

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("texture_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&texture_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        let output = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Lost) => { self.resize(self.width, self.height); return; }
            Err(e) => { log::warn!("Surface error: {:?}", e); return; }
        };

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.draw(0..6, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }
}
