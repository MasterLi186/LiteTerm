use std::collections::HashMap;
use std::sync::Arc;
use wgpu;
use wgpu::util::DeviceExt;
use winit::window::Window;
use cosmic_text::{
    Attrs, Buffer, FontSystem, Metrics, Shaping, SwashCache, SwashImage,
    CacheKey,
};

use crate::terminal::TerminalState;

/// 缓存的字形位图
struct GlyphBitmap {
    data: Vec<u8>,
    width: u32,
    height: u32,
    left: i32,
    top: i32,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    width: u32,
    height: u32,
    cell_width: f32,
    cell_height: f32,
    font_size: f32,
    font_system: FontSystem,
    swash_cache: SwashCache,
    // 字形位图缓存（避免每帧重新光栅化）
    glyph_cache: HashMap<CacheKey, Option<GlyphBitmap>>,
    // GPU 管线
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    vertex_buffer: wgpu::Buffer,
    // CPU 帧缓冲
    framebuffer: Vec<u8>,
    fb_width: u32,
    fb_height: u32,
}

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

@group(0) @binding(0) var t_diffuse: texture_2d<f32>;
@group(0) @binding(1) var s_diffuse: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(t_diffuse, s_diffuse, in.tex_coords);
}
"#;

// 默认 ANSI 16 色
const ANSI_COLORS: [[u8; 3]; 16] = [
    [0,0,0],[205,49,49],[13,188,121],[229,229,16],
    [36,114,200],[188,63,188],[17,168,205],[229,229,229],
    [102,102,102],[241,76,76],[35,209,139],[245,245,67],
    [59,142,234],[214,112,214],[41,184,219],[255,255,255],
];

const BG_DEFAULT: [u8; 4] = [15, 20, 25, 255];
const FG_DEFAULT: [u8; 4] = [229, 229, 229, 255];

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
        let surface_format = surface_caps.formats.iter()
            .find(|f| f.is_srgb()).copied()
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

        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();

        let font_size = 15.0;
        let line_height = font_size * 1.2;
        let cell_width = font_size * 0.6;
        let cell_height = line_height;

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

        Self {
            surface, device, queue, config,
            width: w, height: h,
            cell_width, cell_height, font_size,
            font_system, swash_cache,
            glyph_cache: HashMap::new(),
            pipeline, bind_group_layout, vertex_buffer,
            framebuffer: vec![0u8; (w * h * 4) as usize],
            fb_width: w, fb_height: h,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 { return; }
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

    fn color_to_rgba(colors: &alacritty_terminal::term::color::Colors, c: alacritty_terminal::vte::ansi::Color, default: [u8; 4]) -> [u8; 4] {
        use alacritty_terminal::vte::ansi::Color as AC;
        match c {
            AC::Spec(rgb) => [rgb.r, rgb.g, rgb.b, 255],
            AC::Named(named) => {
                if let Some(rgb) = colors[named] {
                    [rgb.r, rgb.g, rgb.b, 255]
                } else {
                    let idx = named as usize;
                    if idx < 16 { [ANSI_COLORS[idx][0], ANSI_COLORS[idx][1], ANSI_COLORS[idx][2], 255] }
                    else { default }
                }
            }
            AC::Indexed(idx) => {
                if let Some(rgb) = colors[idx as usize] {
                    [rgb.r, rgb.g, rgb.b, 255]
                } else if (idx as usize) < 16 {
                    [ANSI_COLORS[idx as usize][0], ANSI_COLORS[idx as usize][1], ANSI_COLORS[idx as usize][2], 255]
                } else if idx < 232 {
                    // 216 色立方体
                    let i = idx - 16;
                    let r = (i / 36) * 51;
                    let g = ((i % 36) / 6) * 51;
                    let b = (i % 6) * 51;
                    [r, g, b, 255]
                } else {
                    // 灰度
                    let v = 8 + (idx - 232) * 10;
                    [v, v, v, 255]
                }
            }
        }
    }

    /// 获取字形位图（带缓存）
    fn get_glyph(&mut self, ch: char) -> Option<&GlyphBitmap> {
        let metrics = Metrics::new(self.font_size, self.cell_height);
        let mut buf = Buffer::new(&mut self.font_system, metrics);
        buf.set_size(&mut self.font_system, Some(self.cell_width * 2.0), Some(self.cell_height * 2.0));
        let attrs = Attrs::new().family(cosmic_text::Family::Monospace);
        let mut s = [0u8; 4];
        let ch_str = ch.encode_utf8(&mut s);
        buf.set_text(&mut self.font_system, ch_str, attrs, Shaping::Advanced);
        buf.shape_until_scroll(&mut self.font_system, false);

        let mut cache_key = None;
        let mut gx_offset = 0i32;
        let mut gy_offset = 0i32;

        for run in buf.layout_runs() {
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                cache_key = Some(physical.cache_key);
                gx_offset = physical.x;
                gy_offset = physical.y;
                break;
            }
            break;
        }

        let key = cache_key?;

        if !self.glyph_cache.contains_key(&key) {
            let bitmap = if let Some(image) = self.swash_cache.get_image(&mut self.font_system, key) {
                Some(GlyphBitmap {
                    data: image.data.clone(),
                    width: image.placement.width as u32,
                    height: image.placement.height as u32,
                    left: gx_offset + image.placement.left,
                    top: gy_offset - image.placement.top + self.font_size as i32,
                })
            } else {
                None
            };
            self.glyph_cache.insert(key, bitmap);
        }

        self.glyph_cache.get(&key)?.as_ref()
    }

    /// 在帧缓冲上画一个填充矩形
    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: [u8; 4]) {
        for iy in 0..h as i32 {
            let py = y + iy;
            if py < 0 || py >= self.fb_height as i32 { continue; }
            for ix in 0..w as i32 {
                let px = x + ix;
                if px < 0 || px >= self.fb_width as i32 { continue; }
                let idx = ((py as u32 * self.fb_width + px as u32) * 4) as usize;
                if idx + 3 < self.framebuffer.len() {
                    self.framebuffer[idx..idx+4].copy_from_slice(&color);
                }
            }
        }
    }

    /// 在帧缓冲上画一个字形
    fn draw_glyph(&mut self, glyph: &GlyphBitmap, x: i32, y: i32, fg: [u8; 4]) {
        let gx = x + glyph.left;
        let gy = y + glyph.top;
        for iy in 0..glyph.height as i32 {
            let py = gy + iy;
            if py < 0 || py >= self.fb_height as i32 { continue; }
            for ix in 0..glyph.width as i32 {
                let px = gx + ix;
                if px < 0 || px >= self.fb_width as i32 { continue; }
                let src_idx = (iy as u32 * glyph.width + ix as u32) as usize;
                if src_idx >= glyph.data.len() { continue; }
                let alpha = glyph.data[src_idx];
                if alpha == 0 { continue; }
                let dst_idx = ((py as u32 * self.fb_width + px as u32) * 4) as usize;
                if dst_idx + 3 >= self.framebuffer.len() { continue; }
                let a = alpha as f32 / 255.0;
                let inv = 1.0 - a;
                self.framebuffer[dst_idx]     = (fg[0] as f32 * a + self.framebuffer[dst_idx] as f32 * inv) as u8;
                self.framebuffer[dst_idx + 1] = (fg[1] as f32 * a + self.framebuffer[dst_idx + 1] as f32 * inv) as u8;
                self.framebuffer[dst_idx + 2] = (fg[2] as f32 * a + self.framebuffer[dst_idx + 2] as f32 * inv) as u8;
                self.framebuffer[dst_idx + 3] = 255;
            }
        }
    }

    fn render_to_framebuffer(&mut self, terminal: &TerminalState, cursor_visible: bool) {
        // 清屏
        for pixel in self.framebuffer.chunks_exact_mut(4) {
            pixel.copy_from_slice(&BG_DEFAULT);
        }

        let term = match terminal.term() {
            Some(t) => t,
            None => return,
        };

        let content = term.renderable_content();
        let cw = self.cell_width;
        let ch = self.cell_height;
        let cw_u = cw as u32;
        let ch_u = ch as u32;

        // 收集所有 cell 数据（避免 borrow 冲突）
        struct CellInfo {
            col: usize,
            row: usize,
            ch: char,
            fg: [u8; 4],
            bg: [u8; 4],
        }
        let mut cells = Vec::new();
        let cursor = content.cursor;

        for indexed in content.display_iter {
            let cell = &indexed.cell;
            let col = indexed.point.column.0;
            let row = indexed.point.line.0 as usize;
            let fg = Self::color_to_rgba(content.colors, cell.fg, FG_DEFAULT);
            let bg = Self::color_to_rgba(content.colors, cell.bg, BG_DEFAULT);
            cells.push(CellInfo { col, row, ch: cell.c, fg, bg });
        }

        // Pass 1: 画背景色
        for ci in &cells {
            if ci.bg != BG_DEFAULT {
                let x = (ci.col as f32 * cw) as i32;
                let y = (ci.row as f32 * ch) as i32;
                self.fill_rect(x, y, cw_u, ch_u, ci.bg);
            }
        }

        // Pass 2: 画字符
        // 先收集需要渲染的字形
        let mut glyph_draws: Vec<(char, i32, i32, [u8; 4])> = Vec::new();
        for ci in &cells {
            if ci.ch == ' ' || ci.ch == '\0' { continue; }
            let x = (ci.col as f32 * cw) as i32;
            let y = (ci.row as f32 * ch) as i32;
            glyph_draws.push((ci.ch, x, y, ci.fg));
        }

        for (ch, x, y, fg) in glyph_draws {
            if let Some(glyph) = self.get_glyph(ch) {
                let g = GlyphBitmap {
                    data: glyph.data.clone(),
                    width: glyph.width,
                    height: glyph.height,
                    left: glyph.left,
                    top: glyph.top,
                };
                self.draw_glyph(&g, x, y, fg);
            }
        }

        // Pass 3: 画光标
        if cursor_visible {
            let cx = (cursor.point.column.0 as f32 * cw) as i32;
            let cy = (cursor.point.line.0 as f32 * ch) as i32;
            // 竖线光标
            self.fill_rect(cx, cy, 2, ch_u, [0, 212, 255, 255]);
        }
    }

    pub fn render(&mut self, terminal: &TerminalState, cursor_visible: bool) {
        self.render_to_framebuffer(terminal, cursor_visible);

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Terminal Texture"),
            size: wgpu::Extent3d { width: self.fb_width, height: self.fb_height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &self.framebuffer,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * self.fb_width), rows_per_image: Some(self.fb_height) },
            wgpu::Extent3d { width: self.fb_width, height: self.fb_height, depth_or_array_layers: 1 },
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
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Render Encoder") });

        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &bind_group, &[]);
            rp.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            rp.draw(0..6, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }
}
