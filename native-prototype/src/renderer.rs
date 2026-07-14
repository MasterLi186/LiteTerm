use std::sync::Arc;
use wgpu;
use wgpu::util::DeviceExt;
use winit::window::Window;
use cosmic_text::{FontSystem, SwashCache};

use crate::atlas::GlyphAtlas;
use crate::terminal::TerminalState;

/// 每个 cell 的实例数据（传给 GPU）
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CellInstance {
    pos: [f32; 2],       // cell 左上角像素坐标
    size: [f32; 2],      // cell 宽高
    uv_pos: [f32; 2],    // 字形在 atlas 中的 UV 左上角
    uv_size: [f32; 2],   // 字形 UV 宽高
    glyph_offset: [f32; 2], // 字形相对于 cell 的偏移
    glyph_size: [f32; 2],   // 字形像素大小
    fg: [f32; 4],
    bg: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    screen_size: [f32; 2],
    atlas_size: [f32; 2],
}

const SHADER: &str = r#"
struct Uniforms {
    screen_size: vec2<f32>,
    atlas_size: vec2<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var t_atlas: texture_2d<f32>;
@group(0) @binding(2) var s_atlas: sampler;

struct CellInstance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv_pos: vec2<f32>,
    @location(3) uv_size: vec2<f32>,
    @location(4) glyph_offset: vec2<f32>,
    @location(5) glyph_size: vec2<f32>,
    @location(6) fg: vec4<f32>,
    @location(7) bg: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) fg: vec4<f32>,
    @location(2) bg: vec4<f32>,
    @location(3) local_pos: vec2<f32>,
    @location(4) glyph_offset: vec2<f32>,
    @location(5) glyph_size: vec2<f32>,
    @location(6) cell_size: vec2<f32>,
    @location(7) uv_origin: vec2<f32>,
    @location(8) uv_extent: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, cell: CellInstance) -> VertexOutput {
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
    );
    let corner = corners[vi];
    let pixel = cell.pos + corner * cell.size;
    let ndc = vec2(
        pixel.x / u.screen_size.x * 2.0 - 1.0,
        1.0 - pixel.y / u.screen_size.y * 2.0,
    );

    var out: VertexOutput;
    out.position = vec4(ndc, 0.0, 1.0);
    out.uv = (cell.uv_pos + corner * cell.uv_size) / u.atlas_size;
    out.fg = cell.fg;
    out.bg = cell.bg;
    out.local_pos = corner * cell.size;
    out.glyph_offset = cell.glyph_offset;
    out.glyph_size = cell.glyph_size;
    out.cell_size = cell.size;
    out.uv_origin = cell.uv_pos / u.atlas_size;
    out.uv_extent = cell.uv_size / u.atlas_size;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.bg;

    let glyph_local = in.local_pos - in.glyph_offset;
    if glyph_local.x >= 0.0 && glyph_local.y >= 0.0 &&
       glyph_local.x < in.glyph_size.x && glyph_local.y < in.glyph_size.y &&
       in.glyph_size.x > 0.0 {
        let glyph_uv = in.uv_origin + (glyph_local / in.glyph_size) * in.uv_extent;
        let alpha = textureSample(t_atlas, s_atlas, glyph_uv).r;
        color = mix(color, in.fg, vec4(alpha, alpha, alpha, alpha));
    }

    return color;
}
"#;

// ANSI 16 色
const ANSI_COLORS: [[u8; 3]; 16] = [
    [0,0,0],[205,49,49],[13,188,121],[229,229,16],
    [36,114,200],[188,63,188],[17,168,205],[229,229,229],
    [102,102,102],[241,76,76],[35,209,139],[245,245,67],
    [59,142,234],[214,112,214],[41,184,219],[255,255,255],
];
const BG_DEFAULT: [u8; 4] = [15, 20, 25, 255];
const FG_DEFAULT: [u8; 4] = [229, 229, 229, 255];

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    width: u32,
    height: u32,
    font_system: FontSystem,
    swash_cache: SwashCache,
    atlas: GlyphAtlas,
    atlas_texture: wgpu::Texture,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
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

        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        let font_size = 15.0;
        let atlas = GlyphAtlas::new(&mut font_system, &mut swash_cache, font_size);

        // 创建 atlas 纹理
        let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Glyph Atlas"),
            size: wgpu::Extent3d { width: atlas.atlas_width, height: atlas.atlas_height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &atlas_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &atlas.data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(atlas.atlas_width), rows_per_image: Some(atlas.atlas_height) },
            wgpu::Extent3d { width: atlas.atlas_width, height: atlas.atlas_height, depth_or_array_layers: 1 },
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0, visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { multisampled: false, view_dimension: wgpu::TextureViewDimension::D2, sample_type: wgpu::TextureSampleType::Float { filterable: true } },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniforms"),
            contents: bytemuck::cast_slice(&[Uniforms {
                screen_size: [size.width as f32, size.height as f32],
                atlas_size: [atlas.atlas_width as f32, atlas.atlas_height as f32],
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<CellInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute { offset: 0,  shader_location: 0, format: wgpu::VertexFormat::Float32x2 }, // pos
                wgpu::VertexAttribute { offset: 8,  shader_location: 1, format: wgpu::VertexFormat::Float32x2 }, // size
                wgpu::VertexAttribute { offset: 16, shader_location: 2, format: wgpu::VertexFormat::Float32x2 }, // uv_pos
                wgpu::VertexAttribute { offset: 24, shader_location: 3, format: wgpu::VertexFormat::Float32x2 }, // uv_size
                wgpu::VertexAttribute { offset: 32, shader_location: 4, format: wgpu::VertexFormat::Float32x2 }, // glyph_offset
                wgpu::VertexAttribute { offset: 40, shader_location: 5, format: wgpu::VertexFormat::Float32x2 }, // glyph_size
                wgpu::VertexAttribute { offset: 48, shader_location: 6, format: wgpu::VertexFormat::Float32x4 }, // fg
                wgpu::VertexAttribute { offset: 64, shader_location: 7, format: wgpu::VertexFormat::Float32x4 }, // bg
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[instance_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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

        Self {
            surface, device, queue, config,
            width: size.width.max(1), height: size.height.max(1),
            font_system, swash_cache, atlas, atlas_texture,
            pipeline, bind_group_layout, uniform_buffer, sampler,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 { return; }
        self.width = width;
        self.height = height;
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[Uniforms {
            screen_size: [width as f32, height as f32],
            atlas_size: [self.atlas.atlas_width as f32, self.atlas.atlas_height as f32],
        }]));
    }

    pub fn calculate_grid_size(&self) -> (u16, u16) {
        let cols = (self.width as f32 / self.atlas.cell_width).floor() as u16;
        let rows = (self.height as f32 / self.atlas.cell_height).floor() as u16;
        (cols.max(1), rows.max(1))
    }

    fn color_to_f32(colors: &alacritty_terminal::term::color::Colors, c: alacritty_terminal::vte::ansi::Color, default: [u8; 4]) -> [f32; 4] {
        use alacritty_terminal::vte::ansi::Color as AC;
        let rgba = match c {
            AC::Spec(rgb) => [rgb.r, rgb.g, rgb.b, 255],
            AC::Named(named) => {
                if let Some(rgb) = colors[named] { [rgb.r, rgb.g, rgb.b, 255] }
                else {
                    let idx = named as usize;
                    if idx < 16 { [ANSI_COLORS[idx][0], ANSI_COLORS[idx][1], ANSI_COLORS[idx][2], 255] }
                    else { default }
                }
            }
            AC::Indexed(idx) => {
                if let Some(rgb) = colors[idx as usize] { [rgb.r, rgb.g, rgb.b, 255] }
                else if (idx as usize) < 16 { [ANSI_COLORS[idx as usize][0], ANSI_COLORS[idx as usize][1], ANSI_COLORS[idx as usize][2], 255] }
                else if idx < 232 {
                    let i = idx - 16;
                    [(i / 36) * 51, ((i % 36) / 6) * 51, (i % 6) * 51, 255]
                } else {
                    let v = 8 + (idx - 232) * 10;
                    [v, v, v, 255]
                }
            }
        };
        [rgba[0] as f32 / 255.0, rgba[1] as f32 / 255.0, rgba[2] as f32 / 255.0, rgba[3] as f32 / 255.0]
    }

    pub fn render(&mut self, terminal: &TerminalState, cursor_visible: bool) {
        let term = match terminal.term() {
            Some(t) => t,
            None => return,
        };

        let content = term.renderable_content();
        let cw = self.atlas.cell_width;
        let ch = self.atlas.cell_height;
        let bg_default_f = [BG_DEFAULT[0] as f32 / 255.0, BG_DEFAULT[1] as f32 / 255.0, BG_DEFAULT[2] as f32 / 255.0, 1.0];
        let fg_default_f = [FG_DEFAULT[0] as f32 / 255.0, FG_DEFAULT[1] as f32 / 255.0, FG_DEFAULT[2] as f32 / 255.0, 1.0];

        let mut instances: Vec<CellInstance> = Vec::with_capacity(8192);
        let cursor = content.cursor;

        for indexed in content.display_iter {
            let cell = &indexed.cell;
            let col = indexed.point.column.0 as f32;
            let row = indexed.point.line.0 as f32;
            let px = col * cw;
            let py = row * ch;

            let fg = Self::color_to_f32(content.colors, cell.fg, FG_DEFAULT);
            let bg = Self::color_to_f32(content.colors, cell.bg, BG_DEFAULT);

            let ch_char = cell.c;
            if ch_char == ' ' || ch_char == '\0' {
                // 只画背景（如果非默认）
                if bg != bg_default_f {
                    instances.push(CellInstance {
                        pos: [px, py], size: [cw, ch],
                        uv_pos: [0.0, 0.0], uv_size: [0.0, 0.0],
                        glyph_offset: [0.0, 0.0], glyph_size: [0.0, 0.0],
                        fg, bg,
                    });
                }
                continue;
            }

            // 确保字形在图集中
            let glyph = self.atlas.ensure_glyph(&mut self.font_system, &mut self.swash_cache, ch_char);

            let (uv_pos, uv_size, g_offset, g_size) = if let Some(g) = glyph {
                (
                    [g.x as f32, g.y as f32],
                    [g.width as f32, g.height as f32],
                    [g.bearing_x as f32, g.bearing_y as f32],
                    [g.width as f32, g.height as f32],
                )
            } else {
                ([0.0, 0.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0])
            };

            instances.push(CellInstance {
                pos: [px, py], size: [cw, ch],
                uv_pos, uv_size,
                glyph_offset: g_offset, glyph_size: g_size,
                fg, bg,
            });
        }

        // 光标
        if cursor_visible {
            let cx = cursor.point.column.0 as f32 * cw;
            let cy = cursor.point.line.0 as f32 * ch;
            instances.push(CellInstance {
                pos: [cx, cy], size: [2.0, ch],
                uv_pos: [0.0, 0.0], uv_size: [0.0, 0.0],
                glyph_offset: [0.0, 0.0], glyph_size: [0.0, 0.0],
                fg: [0.0, 0.83, 1.0, 1.0], // cyan
                bg: [0.0, 0.83, 1.0, 1.0],
            });
        }

        if instances.is_empty() { return; }

        // 如果 atlas 有更新，重新上传纹理
        if self.atlas.dirty {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.atlas_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &self.atlas.data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(self.atlas.atlas_width), rows_per_image: Some(self.atlas.atlas_height) },
                wgpu::Extent3d { width: self.atlas.atlas_width, height: self.atlas.atlas_height, depth_or_array_layers: 1 },
            );
            self.atlas.dirty = false;
        }

        // 创建实例缓冲区
        let instance_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Cell Instances"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let atlas_view = self.atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&atlas_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
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
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: BG_DEFAULT[0] as f64 / 255.0, g: BG_DEFAULT[1] as f64 / 255.0, b: BG_DEFAULT[2] as f64 / 255.0, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &bind_group, &[]);
            rp.set_vertex_buffer(0, instance_buffer.slice(..));
            rp.draw(0..6, 0..instances.len() as u32);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }
}
