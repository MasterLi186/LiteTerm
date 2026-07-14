use std::sync::Arc;
use wgpu;
use wgpu::util::DeviceExt;
use winit::window::Window;
use cosmic_text::{FontSystem, SwashCache};

use crate::atlas::GlyphAtlas;
use crate::terminal::TerminalState;

/// Per-cell instance data sent to GPU.
/// `flags`: bit 0 = underline, bit 1 = strikethrough
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CellInstance {
    pos: [f32; 2],
    size: [f32; 2],
    uv_pos: [f32; 2],
    uv_size: [f32; 2],
    glyph_offset: [f32; 2],
    glyph_size: [f32; 2],
    fg: [f32; 4],
    bg: [f32; 4],
    flags: u32,
    _pad: [u32; 3],
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
    @location(8) flags: u32,
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
    @location(9) flags: u32,
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
    out.flags = cell.flags;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.bg;

    // Glyph rendering
    let glyph_local = in.local_pos - in.glyph_offset;
    if glyph_local.x >= 0.0 && glyph_local.y >= 0.0 &&
       glyph_local.x < in.glyph_size.x && glyph_local.y < in.glyph_size.y &&
       in.glyph_size.x > 0.0 {
        let glyph_uv = in.uv_origin + (glyph_local / in.glyph_size) * in.uv_extent;
        let alpha = textureSample(t_atlas, s_atlas, glyph_uv).r;
        color = mix(color, in.fg, vec4(alpha, alpha, alpha, alpha));
    }

    // Underline (bit 0): 1px line at bottom - 2px
    if (in.flags & 1u) != 0u {
        let underline_y = in.cell_size.y - 2.0;
        if in.local_pos.y >= underline_y && in.local_pos.y < underline_y + 1.0 {
            color = in.fg;
        }
    }

    // Strikethrough (bit 1): 1px line at middle
    if (in.flags & 2u) != 0u {
        let strike_y = in.cell_size.y * 0.5;
        if in.local_pos.y >= strike_y && in.local_pos.y < strike_y + 1.0 {
            color = in.fg;
        }
    }

    return color;
}
"#;

// AdventureTime 配色方案（来自 Tabby）
const ANSI_COLORS: [[u8; 3]; 16] = [
    // normal: black,    red,       green,     yellow
    [0x05,0x04,0x04], [0xbd,0x00,0x13], [0x4a,0xb1,0x18], [0xe7,0x74,0x1e],
    // normal: blue,     magenta,   cyan,      white
    [0x0f,0x4a,0xc6], [0x66,0x59,0x93], [0x70,0xa5,0x98], [0xf8,0xdc,0xc0],
    // bright: black,    red,       green,     yellow
    [0x4e,0x7c,0xbf], [0xfc,0x5f,0x5a], [0x9e,0xff,0x6e], [0xef,0xc1,0x1a],
    // bright: blue,     magenta,   cyan,      white
    [0x19,0x97,0xc6], [0x9b,0x59,0x53], [0xc8,0xfa,0xf4], [0xf6,0xf5,0xfb],
];
const BG_DEFAULT: [u8; 4] = [0x1f, 0x1d, 0x45, 255]; // #1f1d45
const FG_DEFAULT: [u8; 4] = [0xf8, 0xdc, 0xc0, 255]; // #f8dcc0
const CURSOR_COLOR: [f32; 4] = [0xef as f32/255.0, 0xbf as f32/255.0, 0x38 as f32/255.0, 1.0]; // #efbf38
const SELECTION_BG: [f32; 4] = [0x26 as f32/255.0, 0x4f as f32/255.0, 0x78 as f32/255.0, 1.0]; // #264f78

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
            .find(|f| !f.is_srgb()).copied()
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
                wgpu::VertexAttribute { offset: 0,  shader_location: 0, format: wgpu::VertexFormat::Float32x2 },  // pos
                wgpu::VertexAttribute { offset: 8,  shader_location: 1, format: wgpu::VertexFormat::Float32x2 },  // size
                wgpu::VertexAttribute { offset: 16, shader_location: 2, format: wgpu::VertexFormat::Float32x2 },  // uv_pos
                wgpu::VertexAttribute { offset: 24, shader_location: 3, format: wgpu::VertexFormat::Float32x2 },  // uv_size
                wgpu::VertexAttribute { offset: 32, shader_location: 4, format: wgpu::VertexFormat::Float32x2 },  // glyph_offset
                wgpu::VertexAttribute { offset: 40, shader_location: 5, format: wgpu::VertexFormat::Float32x2 },  // glyph_size
                wgpu::VertexAttribute { offset: 48, shader_location: 6, format: wgpu::VertexFormat::Float32x4 },  // fg
                wgpu::VertexAttribute { offset: 64, shader_location: 7, format: wgpu::VertexFormat::Float32x4 },  // bg
                wgpu::VertexAttribute { offset: 80, shader_location: 8, format: wgpu::VertexFormat::Uint32 },     // flags
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

    pub fn cell_size(&self) -> (f32, f32) {
        (self.atlas.cell_width, self.atlas.cell_height)
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

    pub fn is_selected(col: usize, row: usize, sel_start: Option<(usize, usize)>, sel_end: Option<(usize, usize)>) -> bool {
        let (start, end) = match (sel_start, sel_end) {
            (Some(s), Some(e)) => {
                if (s.1, s.0) <= (e.1, e.0) { (s, e) } else { (e, s) }
            }
            _ => return false,
        };
        if row < start.1 || row > end.1 { return false; }
        if row == start.1 && row == end.1 { return col >= start.0 && col <= end.0; }
        if row == start.1 { return col >= start.0; }
        if row == end.1 { return col <= end.0; }
        true
    }

    /// Check if terminal has mouse reporting mode enabled
    pub fn is_mouse_mode(terminal: &TerminalState) -> bool {
        if let Some(t) = terminal.term() {
            let mode = *t.mode();
            use alacritty_terminal::term::TermMode;
            mode.intersects(TermMode::MOUSE_MODE)
        } else {
            false
        }
    }

    pub fn render(
        &mut self,
        terminal: &TerminalState,
        cursor_visible: bool,
        sel_start: Option<(usize, usize)>,
        sel_end: Option<(usize, usize)>,
    ) {
        let term = match terminal.term() {
            Some(t) => t,
            None => return,
        };

        let content = term.renderable_content();
        let cw = self.atlas.cell_width;
        let ch = self.atlas.cell_height;
        let bg_default_f = [BG_DEFAULT[0] as f32 / 255.0, BG_DEFAULT[1] as f32 / 255.0, BG_DEFAULT[2] as f32 / 255.0, 1.0];

        let mut instances: Vec<CellInstance> = Vec::with_capacity(8192);
        let cursor = content.cursor;

        use alacritty_terminal::term::cell::Flags as CellFlags;

        for indexed in content.display_iter {
            let cell = &indexed.cell;
            let col_idx = indexed.point.column.0;
            let row_idx = indexed.point.line.0 as usize;
            let px = col_idx as f32 * cw;
            let py = row_idx as f32 * ch;

            let flags = cell.flags;

            // HIDDEN 或宽字符续位：跳过（宽字符在首位已按 2 列宽渲染）
            if flags.contains(CellFlags::HIDDEN)
                || flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                continue;
            }

            let mut fg = Self::color_to_f32(content.colors, cell.fg, FG_DEFAULT);
            let mut bg = Self::color_to_f32(content.colors, cell.bg, BG_DEFAULT);

            // INVERSE: swap fg/bg
            if flags.contains(CellFlags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }

            // DIM: reduce fg alpha
            if flags.contains(CellFlags::DIM) {
                fg[0] *= 0.5;
                fg[1] *= 0.5;
                fg[2] *= 0.5;
            }

            // Selection highlight
            if Self::is_selected(col_idx, row_idx, sel_start, sel_end) {
                std::mem::swap(&mut fg, &mut bg);
                if bg == bg_default_f { bg = SELECTION_BG; }
            }

            // Build GPU flags
            let mut gpu_flags: u32 = 0;
            if flags.intersects(CellFlags::ALL_UNDERLINES) { gpu_flags |= 1; }
            if flags.contains(CellFlags::STRIKEOUT) { gpu_flags |= 2; }

            let bold = flags.contains(CellFlags::BOLD);
            let italic = flags.contains(CellFlags::ITALIC);

            let ch_char = cell.c;
            if ch_char == ' ' || ch_char == '\0' {
                if bg != bg_default_f || gpu_flags != 0 {
                    instances.push(CellInstance {
                        pos: [px, py], size: [cw, ch],
                        uv_pos: [0.0, 0.0], uv_size: [0.0, 0.0],
                        glyph_offset: [0.0, 0.0], glyph_size: [0.0, 0.0],
                        fg, bg, flags: gpu_flags, _pad: [0; 3],
                    });
                }
                continue;
            }

            let glyph = self.atlas.ensure_glyph(&mut self.font_system, &mut self.swash_cache, ch_char, bold, italic);

            let cell_w = if flags.contains(CellFlags::WIDE_CHAR) { cw * 2.0 } else { cw };
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
                pos: [px, py], size: [cell_w, ch],
                uv_pos, uv_size,
                glyph_offset: g_offset, glyph_size: g_size,
                fg, bg, flags: gpu_flags, _pad: [0; 3],
            });
        }

        // Cursor
        if cursor_visible {
            let cx = cursor.point.column.0 as f32 * cw;
            let cy = cursor.point.line.0 as f32 * ch;
            instances.push(CellInstance {
                pos: [cx, cy], size: [2.0, ch],
                uv_pos: [0.0, 0.0], uv_size: [0.0, 0.0],
                glyph_offset: [0.0, 0.0], glyph_size: [0.0, 0.0],
                fg: CURSOR_COLOR,
                bg: CURSOR_COLOR,
                flags: 0, _pad: [0; 3],
            });
        }

        if instances.is_empty() { return; }

        if self.atlas.dirty {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.atlas_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &self.atlas.data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(self.atlas.atlas_width), rows_per_image: Some(self.atlas.atlas_height) },
                wgpu::Extent3d { width: self.atlas.atlas_width, height: self.atlas.atlas_height, depth_or_array_layers: 1 },
            );
            self.atlas.dirty = false;
        }

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
