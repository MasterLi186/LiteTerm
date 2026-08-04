use super::*;

pub(super) fn cached_pane_slot_index(draw_index: usize) -> Option<usize> {
    (draw_index < MAX_CACHED_PANE_DRAW_SLOTS).then_some(draw_index)
}

struct CachedPaneDrawSlot {
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instance_capacity: wgpu::BufferAddress,
    instance_count: u32,
    content_signature: Option<PaneContentSignature>,
}

enum PreparedPaneResources {
    Cached(usize),
    Transient {
        bind_group: wgpu::BindGroup,
        instance_buffer: wgpu::Buffer,
        _uniform_buffer: wgpu::Buffer,
    },
}

struct PreparedPaneDraw {
    rect: ClampedPaneRenderRect,
    instance_count: u32,
    resources: PreparedPaneResources,
}

pub struct Renderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    atlas: GlyphAtlas,
    atlas_texture: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    cached_pane_slots: Vec<CachedPaneDrawSlot>,
    prepared_pane_draws: Vec<PreparedPaneDraw>,
    next_pane_slot: usize,
    palette: TerminalPalette,
    style_revision: u64,
    pub viewport_x: f32,
    pub viewport_y: f32,
    pub viewport_width: f32,
    pub viewport_height: f32,
}

impl Renderer {
    pub fn new(gpu: &GpuState) -> Self {
        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        let atlas = GlyphAtlas::new(&mut font_system, &mut swash_cache, BOOTSTRAP_FONT_SIZE);

        let atlas_texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Glyph Atlas"),
            size: wgpu::Extent3d {
                width: atlas.atlas_width,
                height: atlas.atlas_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(atlas.atlas_width),
                rows_per_image: Some(atlas.atlas_height),
            },
            wgpu::Extent3d {
                width: atlas.atlas_width,
                height: atlas.atlas_height,
                depth_or_array_layers: 1,
            },
        );
        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("bind_group_layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                multisampled: false,
                                view_dimension: wgpu::TextureViewDimension::D2,
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Shader"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });

        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<CellInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 8,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 24,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 40,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 48,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 64,
                    shader_location: 7,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 80,
                    shader_location: 8,
                    format: wgpu::VertexFormat::Uint32,
                },
            ],
        };

        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                        format: gpu.config.format,
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

        let palette = TerminalPalette::from_theme(
            crate::themes::theme_by_name(crate::settings::DEFAULT_TERMINAL_COLOR_SCHEME)
                .expect("default terminal theme must exist"),
        );

        Self {
            font_system,
            swash_cache,
            atlas,
            atlas_texture,
            atlas_view,
            pipeline,
            bind_group_layout,
            sampler,
            cached_pane_slots: Vec::new(),
            prepared_pane_draws: Vec::new(),
            next_pane_slot: 0,
            palette,
            style_revision: 1,
            viewport_x: 0.0,
            viewport_y: 0.0,
            viewport_width: gpu.width as f32,
            viewport_height: gpu.height as f32,
        }
    }

    pub fn set_theme(&mut self, theme: &crate::themes::TerminalTheme) {
        self.palette = TerminalPalette::from_theme(theme);
        self.style_revision = self.style_revision.wrapping_add(1).max(1);
    }

    /// Return the installed fixed-width font families reported by the same
    /// database used for terminal shaping. Keeping discovery here avoids a
    /// second platform-specific font scan just for the settings page.
    pub fn monospace_font_families(&self) -> Vec<String> {
        let mut families = std::collections::BTreeSet::new();
        for face in self.font_system.db().faces().filter(|face| face.monospaced) {
            if let Some((family, _language)) = face.families.first() {
                let family = family.trim();
                if !family.is_empty() {
                    families.insert(family.to_string());
                }
            }
        }
        families.into_iter().collect()
    }

    /// 实时切换字体：重建 CPU 字体状态并清空 atlas（同尺寸纹理原地覆盖），不重建 pipeline/主题。
    pub fn set_font(&mut self, gpu: &GpuState, family: &str, size: f32) {
        self.font_system = FontSystem::new();
        self.swash_cache = SwashCache::new();
        self.atlas.reset(family, size);
        // 立即上传已清零的 atlas，避免切换后一帧残留旧字形
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.atlas.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.atlas.atlas_width),
                rows_per_image: Some(self.atlas.atlas_height),
            },
            wgpu::Extent3d {
                width: self.atlas.atlas_width,
                height: self.atlas.atlas_height,
                depth_or_array_layers: 1,
            },
        );
        self.atlas.dirty = false;
        self.style_revision = self.style_revision.wrapping_add(1).max(1);
    }

    pub fn palette(&self) -> &TerminalPalette {
        &self.palette
    }

    pub fn set_viewport(&mut self, x: f32, y: f32, width: f32, height: f32, _gpu: &GpuState) {
        self.viewport_x = x;
        self.viewport_y = y;
        self.viewport_width = width;
        self.viewport_height = height;
    }

    pub fn calculate_grid_size(&self) -> (u16, u16) {
        self.calculate_grid_size_for_rect(PaneRenderRect::new(
            self.viewport_x,
            self.viewport_y,
            self.viewport_width,
            self.viewport_height,
        ))
    }

    /// Calculate the terminal grid for a pane without mutating renderer state.
    pub fn calculate_grid_size_for_rect(&self, rect: PaneRenderRect) -> (u16, u16) {
        let cols = (rect.width.max(0.0) / self.atlas.cell_width).floor() as u16;
        // 减 1 行安全余量：字形 bearing 可能超出 cell 底部边界
        let rows =
            ((rect.height.max(0.0) / self.atlas.cell_height).floor() as u16).saturating_sub(1);
        (cols.max(1), rows.max(1))
    }

    pub fn cell_size(&self) -> (f32, f32) {
        (self.atlas.cell_width, self.atlas.cell_height)
    }

    pub fn viewport_rect(&self) -> egui::Rect {
        egui::Rect::from_min_size(
            egui::pos2(self.viewport_x, self.viewport_y),
            egui::vec2(self.viewport_width, self.viewport_height),
        )
    }

    pub fn cursor_screen_rect(&self, terminal: &TerminalState) -> Option<egui::Rect> {
        let term = terminal.term()?;
        let point = term.grid().cursor.point;
        let display_offset = term.grid().display_offset() as i32;
        cursor_screen_rect_for_viewport(
            self.viewport_x,
            self.viewport_y,
            self.atlas.cell_width,
            self.atlas.cell_height,
            self.viewport_height,
            point.line.0,
            point.column.0,
            display_offset,
        )
    }

    fn color_to_f32(
        &self,
        colors: &alacritty_terminal::term::color::Colors,
        c: alacritty_terminal::vte::ansi::Color,
        default: [u8; 4],
    ) -> [f32; 4] {
        use alacritty_terminal::vte::ansi::Color as AC;
        let rgba = match c {
            AC::Spec(rgb) => [rgb.r, rgb.g, rgb.b, 255],
            AC::Named(named) => {
                if let Some(rgb) = colors[named] {
                    [rgb.r, rgb.g, rgb.b, 255]
                } else {
                    let idx = named as usize;
                    if idx < 16 {
                        self.palette.ansi[idx]
                    } else {
                        default
                    }
                }
            }
            AC::Indexed(idx) => {
                if let Some(rgb) = colors[idx as usize] {
                    [rgb.r, rgb.g, rgb.b, 255]
                } else if (idx as usize) < 16 {
                    self.palette.ansi[idx as usize]
                } else if idx < 232 {
                    let i = idx - 16;
                    [(i / 36) * 51, ((i % 36) / 6) * 51, (i % 6) * 51, 255]
                } else {
                    let v = 8 + (idx - 232) * 10;
                    [v, v, v, 255]
                }
            }
        };
        [
            rgba[0] as f32 / 255.0,
            rgba[1] as f32 / 255.0,
            rgba[2] as f32 / 255.0,
            rgba[3] as f32 / 255.0,
        ]
    }

    pub fn is_selected(
        col: usize,
        line: i32,
        sel_start: Option<(usize, i32)>,
        sel_end: Option<(usize, i32)>,
    ) -> bool {
        let (start, end) = match (sel_start, sel_end) {
            (Some(s), Some(e)) => {
                if (s.1, s.0) <= (e.1, e.0) {
                    (s, e)
                } else {
                    (e, s)
                }
            }
            _ => return false,
        };
        if line < start.1 || line > end.1 {
            return false;
        }
        if line == start.1 && line == end.1 {
            return col >= start.0 && col <= end.0;
        }
        if line == start.1 {
            return col >= start.0;
        }
        if line == end.1 {
            return col <= end.0;
        }
        true
    }

    pub fn is_mouse_mode(terminal: &TerminalState) -> bool {
        if let Some(t) = terminal.term() {
            let mode = *t.mode();
            use alacritty_terminal::term::TermMode;
            mode.intersects(TermMode::MOUSE_MODE)
        } else {
            false
        }
    }

    /// Subdued amber for non-current matches; terminal text remains readable.
    const SEARCH_MATCH_BG: [f32; 4] = [0.30, 0.23, 0.03, 1.0];
    /// The current result deliberately uses a different hue and strong contrast.
    const SEARCH_CURRENT_BG: [f32; 4] = [0.00, 0.83, 1.00, 1.0];
    const SEARCH_CURRENT_FG: [f32; 4] = [0.02, 0.05, 0.07, 1.0];

    /// Render a single terminal grid into the current frame's encoder.
    ///
    /// `search_highlights`: when `None`, search highlighting is skipped (legacy path).
    /// Selection strictly overrides search backgrounds.
    pub fn render_to_pass(
        &mut self,
        gpu: &GpuState,
        view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        terminal: &TerminalState,
        cursor_visible: bool,
        sel_start: Option<(usize, i32)>,
        sel_end: Option<(usize, i32)>,
        search_highlights: Option<SearchHighlights<'_>>,
    ) {
        self.begin_pane_frame();
        self.prepare_pane_draw(
            gpu,
            "single-pane",
            PaneRenderRect::new(
                self.viewport_x,
                self.viewport_y,
                self.viewport_width,
                self.viewport_height,
            ),
            terminal,
            cursor_visible,
            sel_start,
            sel_end,
            search_highlights,
        );
        self.render_prepared_panes(gpu, view, encoder);
    }

    /// Discard the previous frame's prepared draws while retaining bounded GPU
    /// buffers for reuse.
    pub fn begin_pane_frame(&mut self) {
        self.prepared_pane_draws.clear();
        self.next_pane_slot = 0;
    }

    /// Compatibility wrapper for callers that render a single pane.
    pub fn render_pane_to_pass(
        &mut self,
        gpu: &GpuState,
        view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        pane: PaneRenderRect,
        terminal: &TerminalState,
        cursor_visible: bool,
        sel_start: Option<(usize, i32)>,
        sel_end: Option<(usize, i32)>,
        search_highlights: Option<SearchHighlights<'_>>,
    ) {
        self.begin_pane_frame();
        self.prepare_pane_draw(
            gpu,
            "single-pane",
            pane,
            terminal,
            cursor_visible,
            sel_start,
            sel_end,
            search_highlights,
        );
        self.render_prepared_panes(gpu, view, encoder);
    }

    /// Build and upload one pane's instances without opening a render pass.
    ///
    /// The first [`MAX_CACHED_PANE_DRAW_SLOTS`] draws reuse one stable uniform,
    /// bind group, and grow-only instance buffer each. Extremely large layouts
    /// fall back to frame-scoped transient resources instead of growing the
    /// persistent cache without bound.
    pub fn prepare_pane_draw(
        &mut self,
        gpu: &GpuState,
        pane_key: &str,
        pane: PaneRenderRect,
        terminal: &TerminalState,
        cursor_visible: bool,
        sel_start: Option<(usize, i32)>,
        sel_end: Option<(usize, i32)>,
        search_highlights: Option<SearchHighlights<'_>>,
    ) {
        let pane = match clamp_pane_render_rect(pane, gpu.width, gpu.height) {
            Some(pane) => pane,
            None => return,
        };
        let draw_index = self.next_pane_slot;
        self.next_pane_slot = self.next_pane_slot.saturating_add(1);
        let slot_index = cached_pane_slot_index(draw_index);
        let signature = PaneContentSignature {
            pane_key: pane_key.to_owned(),
            terminal_revision: terminal.render_revision(),
            style_revision: self.style_revision,
            cursor_visible,
            selection_start: sel_start,
            selection_end: sel_end,
            search_fingerprint: search_highlights_fingerprint(search_highlights),
        };
        let uniforms = Uniforms {
            surface_size: [gpu.width as f32, gpu.height as f32],
            atlas_size: [
                self.atlas.atlas_width as f32,
                self.atlas.atlas_height as f32,
            ],
            pane_origin: [pane.pane_x, pane.pane_y],
            _padding: [0.0; 2],
        };
        if let Some(slot_index) = slot_index {
            if let Some(slot) = self.cached_pane_slots.get(slot_index) {
                if slot.content_signature.as_ref() == Some(&signature) {
                    gpu.queue
                        .write_buffer(&slot.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
                    if slot.instance_count > 0 {
                        self.prepared_pane_draws.push(PreparedPaneDraw {
                            rect: pane,
                            instance_count: slot.instance_count,
                            resources: PreparedPaneResources::Cached(slot_index),
                        });
                    }
                    return;
                }
            }
        }
        let term = match terminal.term() {
            Some(t) => t,
            None => return,
        };

        let content = term.renderable_content();
        let cw = self.atlas.cell_width;
        let ch = self.atlas.cell_height;
        // Cell positions stay pane-local; the uniform supplies the pane origin.
        let offset_x = 0.0f32;
        let bg_default_f = [
            self.palette.background[0] as f32 / 255.0,
            self.palette.background[1] as f32 / 255.0,
            self.palette.background[2] as f32 / 255.0,
            1.0,
        ];

        let mut instances: Vec<CellInstance> = Vec::with_capacity(8192);
        let cursor = content.cursor;
        let display_offset = term.grid().display_offset() as i32;

        use alacritty_terminal::term::cell::Flags as CellFlags;

        for indexed in content.display_iter {
            let cell = &indexed.cell;
            let col_idx = indexed.point.column.0;
            // Absolute grid line for search classification (scrollback-aware).
            let abs_line = indexed.point.line.0;
            // display_iter 的行号可能是负数（scrollback 区域），
            // 重新映射到 0~screen_lines：line + display_offset + 1 = 视口行号
            let visual_row = (abs_line + display_offset + 1) as f32;
            let px = offset_x + col_idx as f32 * cw;
            let py = visual_row * ch;

            let flags = cell.flags;

            // Hidden cells and wide-char spacers are not drawn; primary wide glyph
            // already covers both grid columns via cell_w = cw * 2.
            if flags.contains(CellFlags::HIDDEN) || flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                continue;
            }

            let mut fg = self.color_to_f32(content.colors, cell.fg, self.palette.foreground);
            let mut bg = self.color_to_f32(content.colors, cell.bg, self.palette.background);

            if flags.contains(CellFlags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if flags.contains(CellFlags::DIM) {
                fg[0] *= 0.5;
                fg[1] *= 0.5;
                fg[2] *= 0.5;
            }

            let selected = Self::is_selected(col_idx, abs_line, sel_start, sel_end);
            let search_kind = search_highlights
                .as_ref()
                .map(|h| search_highlight_kind(abs_line, col_idx, h))
                .unwrap_or(SearchHighlightKind::None);

            match resolve_cell_background_source(selected, search_kind) {
                CellBackgroundSource::Selection => {
                    std::mem::swap(&mut fg, &mut bg);
                    if bg == bg_default_f {
                        bg = self.palette.selection;
                    }
                }
                CellBackgroundSource::SearchCurrent => {
                    bg = Self::SEARCH_CURRENT_BG;
                    fg = Self::SEARCH_CURRENT_FG;
                }
                CellBackgroundSource::SearchMatch => {
                    bg = Self::SEARCH_MATCH_BG;
                }
                CellBackgroundSource::Cell => {}
            }

            let mut gpu_flags: u32 = 0;
            if flags.intersects(CellFlags::ALL_UNDERLINES) {
                gpu_flags |= 1;
            }
            if flags.contains(CellFlags::STRIKEOUT) {
                gpu_flags |= 2;
            }

            let bold = flags.contains(CellFlags::BOLD);
            let italic = flags.contains(CellFlags::ITALIC);

            let ch_char = cell.c;
            if ch_char == ' ' || ch_char == '\0' {
                if bg != bg_default_f || gpu_flags != 0 {
                    instances.push(CellInstance {
                        pos: [px, py],
                        size: [cw, ch],
                        uv_pos: [0.0, 0.0],
                        uv_size: [0.0, 0.0],
                        glyph_offset: [0.0, 0.0],
                        glyph_size: [0.0, 0.0],
                        fg,
                        bg,
                        flags: gpu_flags,
                        _pad: [0; 3],
                    });
                }
                continue;
            }

            let glyph = self.atlas.ensure_glyph(
                &mut self.font_system,
                &mut self.swash_cache,
                ch_char,
                bold,
                italic,
            );

            let cell_w = if flags.contains(CellFlags::WIDE_CHAR) {
                cw * 2.0
            } else {
                cw
            };
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
                pos: [px, py],
                size: [cell_w, ch],
                uv_pos,
                uv_size,
                glyph_offset: g_offset,
                glyph_size: g_size,
                fg,
                bg,
                flags: gpu_flags,
                _pad: [0; 3],
            });
        }

        // Cursor
        if cursor_visible {
            let cx = offset_x + cursor.point.column.0 as f32 * cw;
            let cy = (cursor.point.line.0 + display_offset + 1) as f32 * ch;
            instances.push(CellInstance {
                pos: [cx, cy],
                size: [2.0, ch],
                uv_pos: [0.0, 0.0],
                uv_size: [0.0, 0.0],
                glyph_offset: [0.0, 0.0],
                glyph_size: [0.0, 0.0],
                fg: self.palette.cursor,
                bg: self.palette.cursor,
                flags: 0,
                _pad: [0; 3],
            });
        }

        if instances.is_empty() && slot_index.is_none() {
            return;
        }

        let instance_bytes = bytemuck::cast_slice(&instances);
        let resources = if let Some(slot_index) = slot_index {
            while self.cached_pane_slots.len() <= slot_index {
                let uniform_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Cached Terminal Pane Uniforms"),
                    size: std::mem::size_of::<Uniforms>() as wgpu::BufferAddress,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Cached Terminal Pane Bind Group"),
                    layout: &self.bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&self.atlas_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
                let instance_capacity = std::mem::size_of::<CellInstance>() as wgpu::BufferAddress;
                let instance_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Cached Terminal Cell Instances"),
                    size: instance_capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.cached_pane_slots.push(CachedPaneDrawSlot {
                    uniform_buffer,
                    bind_group,
                    instance_buffer,
                    instance_capacity,
                    instance_count: 0,
                    content_signature: None,
                });
            }

            let slot = &mut self.cached_pane_slots[slot_index];
            let required_capacity = instance_bytes.len() as wgpu::BufferAddress;
            if required_capacity > slot.instance_capacity {
                let instance_capacity = required_capacity.next_power_of_two();
                slot.instance_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Cached Terminal Cell Instances"),
                    size: instance_capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                slot.instance_capacity = instance_capacity;
            }
            gpu.queue
                .write_buffer(&slot.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
            if !instance_bytes.is_empty() {
                gpu.queue
                    .write_buffer(&slot.instance_buffer, 0, instance_bytes);
            }
            slot.instance_count = instances.len() as u32;
            slot.content_signature = Some(signature);
            PreparedPaneResources::Cached(slot_index)
        } else {
            let instance_buffer =
                gpu.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Transient Terminal Cell Instances"),
                        contents: instance_bytes,
                        usage: wgpu::BufferUsages::VERTEX,
                    });
            let uniform_buffer = gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Transient Terminal Pane Uniforms"),
                    contents: bytemuck::bytes_of(&uniforms),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Transient Terminal Pane Bind Group"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.atlas_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            PreparedPaneResources::Transient {
                bind_group,
                instance_buffer,
                _uniform_buffer: uniform_buffer,
            }
        };

        self.prepared_pane_draws.push(PreparedPaneDraw {
            rect: pane,
            instance_count: instances.len() as u32,
            resources,
        });
    }

    /// Render every prepared terminal pane in one pass.
    pub fn render_prepared_panes(
        &mut self,
        gpu: &GpuState,
        view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        if self.prepared_pane_draws.is_empty() {
            return;
        }

        if self.atlas.dirty {
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.atlas.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.atlas.atlas_width),
                    rows_per_image: Some(self.atlas.atlas_height),
                },
                wgpu::Extent3d {
                    width: self.atlas.atlas_width,
                    height: self.atlas.atlas_height,
                    depth_or_array_layers: 1,
                },
            );
            self.atlas.dirty = false;
        }

        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Terminal Panes Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
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
        rp.set_viewport(0.0, 0.0, gpu.width as f32, gpu.height as f32, 0.0, 1.0);
        rp.set_pipeline(&self.pipeline);

        for draw in &self.prepared_pane_draws {
            rp.set_scissor_rect(
                draw.rect.scissor_x,
                draw.rect.scissor_y,
                draw.rect.scissor_width,
                draw.rect.scissor_height,
            );
            match &draw.resources {
                PreparedPaneResources::Cached(slot_index) => {
                    let slot = &self.cached_pane_slots[*slot_index];
                    rp.set_bind_group(0, &slot.bind_group, &[]);
                    rp.set_vertex_buffer(0, slot.instance_buffer.slice(..));
                }
                PreparedPaneResources::Transient {
                    bind_group,
                    instance_buffer,
                    ..
                } => {
                    rp.set_bind_group(0, bind_group, &[]);
                    rp.set_vertex_buffer(0, instance_buffer.slice(..));
                }
            }
            rp.draw(0..6, 0..draw.instance_count);
        }
    }
}
