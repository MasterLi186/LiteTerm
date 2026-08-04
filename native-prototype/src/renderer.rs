use cosmic_text::{FontSystem, SwashCache};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use wgpu;
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::atlas::GlyphAtlas;
use crate::terminal::TerminalState;
use crate::terminal_search::SearchMatch;
use alacritty_terminal::selection::SelectionRange;

const BOOTSTRAP_FONT_SIZE: f32 = crate::settings::DEFAULT_TERMINAL_FONT_SIZE;

/// Borrowed view of search match ranges for per-cell highlight classification.
#[derive(Debug, Clone, Copy)]
pub struct SearchHighlights<'a> {
    pub matches: &'a [SearchMatch],
    pub current: Option<usize>,
}

/// How a cell participates in search highlighting (absolute line + col).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchHighlightKind {
    None,
    Match,
    Current,
}

/// Classify a cell against search highlights.
/// Current (valid index covering the cell) wins over plain Match.
/// Out-of-bounds / None current is safe and falls through to Match/None.
pub fn search_highlight_kind(
    abs_line: i32,
    col: usize,
    highlights: &SearchHighlights<'_>,
) -> SearchHighlightKind {
    if let Some(idx) = highlights.current {
        if let Some(m) = highlights.matches.get(idx) {
            if m.contains_cell(abs_line, col) {
                return SearchHighlightKind::Current;
            }
        }
    }
    for m in highlights.matches {
        if m.contains_cell(abs_line, col) {
            return SearchHighlightKind::Match;
        }
    }
    SearchHighlightKind::None
}

/// Resolved background source for a terminal cell (pure priority, no GPU).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellBackgroundSource {
    Selection,
    SearchCurrent,
    SearchMatch,
    Cell,
}

/// Priority: selection > search current > search match > cell background.
pub fn resolve_cell_background_source(
    selected: bool,
    kind: SearchHighlightKind,
) -> CellBackgroundSource {
    if selected {
        return CellBackgroundSource::Selection;
    }
    match kind {
        SearchHighlightKind::Current => CellBackgroundSource::SearchCurrent,
        SearchHighlightKind::Match => CellBackgroundSource::SearchMatch,
        SearchHighlightKind::None => CellBackgroundSource::Cell,
    }
}

fn cursor_screen_rect_for_metrics(
    viewport_x: f32,
    viewport_y: f32,
    cell_width: f32,
    cell_height: f32,
    point_line: i32,
    point_column: usize,
    display_offset: i32,
    rows: u16,
) -> Option<egui::Rect> {
    let visual_row = point_line + display_offset + 1;
    if visual_row < 0 || visual_row >= i32::from(rows) {
        return None;
    }
    Some(egui::Rect::from_min_size(
        egui::pos2(
            viewport_x + point_column as f32 * cell_width,
            viewport_y + visual_row as f32 * cell_height,
        ),
        egui::vec2(cell_width, cell_height),
    ))
}

fn cursor_screen_rect_for_viewport(
    viewport_x: f32,
    viewport_y: f32,
    cell_width: f32,
    cell_height: f32,
    viewport_height: f32,
    point_line: i32,
    point_column: usize,
    display_offset: i32,
) -> Option<egui::Rect> {
    if !viewport_height.is_finite()
        || !cell_height.is_finite()
        || viewport_height <= 0.0
        || cell_height <= 0.0
    {
        return None;
    }
    let visible_rows = (viewport_height / cell_height)
        .floor()
        .clamp(0.0, f32::from(u16::MAX)) as u16;
    cursor_screen_rect_for_metrics(
        viewport_x,
        viewport_y,
        cell_width,
        cell_height,
        point_line,
        point_column,
        display_offset,
        visible_rows,
    )
}

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
    surface_size: [f32; 2],
    atlas_size: [f32; 2],
    pane_origin: [f32; 2],
    _padding: [f32; 2],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PaneContentSignature {
    pane_key: String,
    terminal_revision: u64,
    style_revision: u64,
    cursor_visible: bool,
    selection: Option<SelectionRange>,
    search_fingerprint: u64,
}

fn search_highlights_fingerprint(highlights: Option<SearchHighlights<'_>>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match highlights {
        None => 0_u8.hash(&mut hasher),
        Some(highlights) => {
            1_u8.hash(&mut hasher);
            highlights.current.hash(&mut hasher);
            for item in highlights.matches {
                item.line.hash(&mut hasher);
                item.start_col.hash(&mut hasher);
                item.end_col.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

/// Surface-space rectangle used for one terminal pane draw.
///
/// Cell instances are local to this rectangle. The renderer translates them by
/// the original pane origin and only clips rasterization to the rectangle's
/// intersection with the surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneRenderRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PaneRenderRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ClampedPaneRenderRect {
    pane_x: f32,
    pane_y: f32,
    pane_width: f32,
    pane_height: f32,
    scissor_x: u32,
    scissor_y: u32,
    scissor_width: u32,
    scissor_height: u32,
}

fn clamp_pane_render_rect(
    rect: PaneRenderRect,
    surface_width: u32,
    surface_height: u32,
) -> Option<ClampedPaneRenderRect> {
    if surface_width == 0
        || surface_height == 0
        || !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || rect.width <= 0.0
        || rect.height <= 0.0
    {
        return None;
    }

    let right = rect.x + rect.width;
    let bottom = rect.y + rect.height;
    if !right.is_finite() || !bottom.is_finite() {
        return None;
    }

    let surface_width_f = surface_width as f32;
    let surface_height_f = surface_height as f32;
    let left = rect.x.clamp(0.0, surface_width_f);
    let top = rect.y.clamp(0.0, surface_height_f);
    let right = right.clamp(0.0, surface_width_f);
    let bottom = bottom.clamp(0.0, surface_height_f);
    if right <= left || bottom <= top {
        return None;
    }

    let scissor_x = left.floor() as u32;
    let scissor_y = top.floor() as u32;
    let scissor_right = (right.ceil() as u32).min(surface_width);
    let scissor_bottom = (bottom.ceil() as u32).min(surface_height);
    let scissor_width = scissor_right.saturating_sub(scissor_x);
    let scissor_height = scissor_bottom.saturating_sub(scissor_y);
    if scissor_width == 0 || scissor_height == 0 {
        return None;
    }

    Some(ClampedPaneRenderRect {
        pane_x: rect.x,
        pane_y: rect.y,
        pane_width: rect.width,
        pane_height: rect.height,
        scissor_x,
        scissor_y,
        scissor_width,
        scissor_height,
    })
}

const SHADER: &str = r#"
struct Uniforms {
    surface_size: vec2<f32>,
    atlas_size: vec2<f32>,
    pane_origin: vec2<f32>,
    _padding: vec2<f32>,
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
    let pixel = u.pane_origin + cell.pos + corner * cell.size;
    let ndc = vec2(
        pixel.x / u.surface_size.x * 2.0 - 1.0,
        1.0 - pixel.y / u.surface_size.y * 2.0,
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

    let glyph_local = in.local_pos - in.glyph_offset;
    if glyph_local.x >= 0.0 && glyph_local.y >= 0.0 &&
       glyph_local.x < in.glyph_size.x && glyph_local.y < in.glyph_size.y &&
       in.glyph_size.x > 0.0 {
        let glyph_uv = in.uv_origin + (glyph_local / in.glyph_size) * in.uv_extent;
        let alpha = textureSample(t_atlas, s_atlas, glyph_uv).r;
        color = mix(color, in.fg, vec4(alpha, alpha, alpha, alpha));
    }

    if (in.flags & 1u) != 0u {
        let underline_y = in.cell_size.y - 2.0;
        if in.local_pos.y >= underline_y && in.local_pos.y < underline_y + 1.0 {
            color = in.fg;
        }
    }

    if (in.flags & 2u) != 0u {
        let strike_y = in.cell_size.y * 0.5;
        if in.local_pos.y >= strike_y && in.local_pos.y < strike_y + 1.0 {
            color = in.fg;
        }
    }

    return color;
}
"#;

#[derive(Debug, Clone, Copy)]
pub struct TerminalPalette {
    pub background: [u8; 4],
    pub foreground: [u8; 4],
    pub cursor: [f32; 4],
    pub selection: [f32; 4],
    pub ansi: [[u8; 4]; 16],
}

impl TerminalPalette {
    pub fn from_theme(theme: &crate::themes::TerminalTheme) -> Self {
        let mut ansi = [[0u8; 4]; 16];
        for i in 0..16 {
            ansi[i] = [theme.ansi[i][0], theme.ansi[i][1], theme.ansi[i][2], 255];
        }
        Self {
            background: [
                theme.background[0],
                theme.background[1],
                theme.background[2],
                255,
            ],
            foreground: [
                theme.foreground[0],
                theme.foreground[1],
                theme.foreground[2],
                255,
            ],
            cursor: [
                theme.cursor[0] as f32 / 255.0,
                theme.cursor[1] as f32 / 255.0,
                theme.cursor[2] as f32 / 255.0,
                1.0,
            ],
            selection: [
                theme.selection[0] as f32 / 255.0,
                theme.selection[1] as f32 / 255.0,
                theme.selection[2] as f32 / 255.0,
                1.0,
            ],
            ansi,
        }
    }
}

/// Shared GPU state accessible from main for egui integration
pub struct GpuState {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub width: u32,
    pub height: u32,
}

impl GpuState {
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
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .expect("无法获取 GPU device");

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
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

        Self {
            surface,
            device,
            queue,
            config,
            width: size.width.max(1),
            height: size.height.max(1),
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

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }
}

const MAX_CACHED_PANE_DRAW_SLOTS: usize = 32;

mod core;

#[cfg(test)]
use core::cached_pane_slot_index;
pub use core::Renderer;

#[cfg(test)]
mod geometry_tests {
    use super::*;
    use crate::smart_completion::CompletionSessionKey;
    use crate::terminal::CompletionHarness;
    use crate::terminal_search::SearchMatch;
    use alacritty_terminal::grid::Dimensions;

    #[test]
    fn pane_rect_preserves_projection_and_only_clamps_scissor_to_surface() {
        assert_eq!(
            clamp_pane_render_rect(PaneRenderRect::new(-5.25, 8.5, 20.0, 30.75), 100, 25),
            Some(ClampedPaneRenderRect {
                pane_x: -5.25,
                pane_y: 8.5,
                pane_width: 20.0,
                pane_height: 30.75,
                scissor_x: 0,
                scissor_y: 8,
                scissor_width: 15,
                scissor_height: 17,
            })
        );
    }

    #[test]
    fn pane_rect_skips_empty_off_surface_and_invalid_rectangles() {
        assert_eq!(
            clamp_pane_render_rect(PaneRenderRect::new(100.0, 0.0, 10.0, 10.0), 100, 100),
            None
        );
        assert_eq!(
            clamp_pane_render_rect(PaneRenderRect::new(0.0, 0.0, 0.0, 10.0), 100, 100),
            None
        );
        assert_eq!(
            clamp_pane_render_rect(PaneRenderRect::new(f32::NAN, 0.0, 10.0, 10.0), 100, 100),
            None
        );
        assert_eq!(
            clamp_pane_render_rect(PaneRenderRect::new(0.0, 0.0, 10.0, 10.0), 0, 100),
            None
        );
    }

    #[test]
    fn pane_draw_cache_is_bounded_and_overflow_is_transient() {
        assert_eq!(cached_pane_slot_index(0), Some(0));
        assert_eq!(
            cached_pane_slot_index(MAX_CACHED_PANE_DRAW_SLOTS - 1),
            Some(MAX_CACHED_PANE_DRAW_SLOTS - 1)
        );
        assert_eq!(cached_pane_slot_index(MAX_CACHED_PANE_DRAW_SLOTS), None);
        assert_eq!(
            cached_pane_slot_index(MAX_CACHED_PANE_DRAW_SLOTS + 100),
            None
        );
    }

    #[test]
    fn cursor_rect_matches_render_formula_at_zero_display_offset() {
        let rect = cursor_screen_rect_for_metrics(220.0, 34.0, 10.0, 20.0, -1, 7, 0, 24).unwrap();

        assert_eq!(
            rect,
            egui::Rect::from_min_size(egui::pos2(290.0, 34.0), egui::vec2(10.0, 20.0))
        );
    }

    #[test]
    fn cursor_rect_rejects_a_cursor_outside_the_visible_scrollback_view() {
        assert_eq!(
            cursor_screen_rect_for_metrics(220.0, 34.0, 10.0, 20.0, -24, 7, 0, 24,),
            None
        );
        assert_eq!(
            cursor_screen_rect_for_metrics(220.0, 34.0, 10.0, 20.0, -1, 7, 24, 24,),
            None
        );
    }

    #[test]
    fn cursor_rect_keeps_the_bottom_physical_viewport_row_available_for_overlays() {
        let rect =
            cursor_screen_rect_for_viewport(220.0, 34.0, 10.0, 15.0, 722.0, 46, 7, 0).unwrap();

        assert_eq!(
            rect,
            egui::Rect::from_min_size(egui::pos2(290.0, 739.0), egui::vec2(10.0, 15.0))
        );
    }

    #[test]
    fn completion_cursor_remains_visible_after_terminal_output_scrolls_to_the_bottom() {
        let mut terminal =
            CompletionHarness::new(80, 47, CompletionSessionKey::new_for_test(1, "bottom-row"));
        terminal.feed("output\r\n".repeat(80).as_bytes());
        let term = terminal.terminal().term().unwrap();
        let point = term.grid().cursor.point;
        let display_offset = term.grid().display_offset() as i32;

        assert_eq!(display_offset, 0, "回归夹具应停留在实时终端底部");
        assert!(term.grid().history_size() > 0, "回归夹具必须产生滚屏历史");
        assert_eq!(
            point.line.0 + display_offset + 1,
            47,
            "回归夹具的光标必须位于最后一个完整物理行"
        );
        assert!(
            cursor_screen_rect_for_metrics(
                220.0,
                28.0,
                8.0,
                15.0,
                point.line.0,
                point.column.0,
                display_offset,
                47,
            )
            .is_none(),
            "回归夹具必须复现旧逻辑把底行光标误判为越界"
        );
        assert!(
            cursor_screen_rect_for_viewport(
                220.0,
                28.0,
                8.0,
                15.0,
                722.0,
                point.line.0,
                point.column.0,
                display_offset,
            )
            .is_some(),
            "滚屏后的底行光标仍应能作为补全弹窗锚点"
        );
    }

    // =========================================================================
    // P0 Task 4 RED-C: SearchHighlights pure cell classification
    // Locks SearchHighlights<'a> {matches,current} + SearchHighlightKind
    // and absolute line+col half-open judgement. Filter: search_
    // =========================================================================

    fn match_at(line: i32, start_col: usize, end_col: usize) -> SearchMatch {
        SearchMatch {
            line,
            start_col,
            end_col,
        }
    }

    /// Half-open absolute range: start inclusive, end exclusive; other lines None.
    #[test]
    fn search_highlight_kind_uses_absolute_line_col_half_open() {
        let matches = [match_at(5, 2, 5)];
        let hl = SearchHighlights {
            matches: &matches,
            current: None,
        };

        assert_eq!(
            search_highlight_kind(5, 1, &hl),
            SearchHighlightKind::None,
            "col before start_col"
        );
        assert_eq!(
            search_highlight_kind(5, 2, &hl),
            SearchHighlightKind::Match,
            "start_col inclusive"
        );
        assert_eq!(
            search_highlight_kind(5, 4, &hl),
            SearchHighlightKind::Match,
            "last col inside [start, end)"
        );
        assert_eq!(
            search_highlight_kind(5, 5, &hl),
            SearchHighlightKind::None,
            "end_col exclusive"
        );
        assert_eq!(
            search_highlight_kind(4, 3, &hl),
            SearchHighlightKind::None,
            "different absolute line"
        );
        assert_eq!(
            search_highlight_kind(6, 3, &hl),
            SearchHighlightKind::None,
            "different absolute line"
        );
    }

    /// Current match index overrides plain Match for its cells.
    #[test]
    fn search_highlight_kind_prefers_current_over_other_matches() {
        let matches = [
            match_at(0, 0, 2), // current
            match_at(0, 4, 6), // other
            match_at(1, 1, 3), // other line
        ];
        let hl = SearchHighlights {
            matches: &matches,
            current: Some(0),
        };

        assert_eq!(
            search_highlight_kind(0, 0, &hl),
            SearchHighlightKind::Current
        );
        assert_eq!(
            search_highlight_kind(0, 1, &hl),
            SearchHighlightKind::Current
        );
        assert_eq!(search_highlight_kind(0, 4, &hl), SearchHighlightKind::Match);
        assert_eq!(search_highlight_kind(0, 5, &hl), SearchHighlightKind::Match);
        assert_eq!(search_highlight_kind(1, 1, &hl), SearchHighlightKind::Match);
        assert_eq!(search_highlight_kind(0, 2, &hl), SearchHighlightKind::None);
        assert_eq!(search_highlight_kind(0, 3, &hl), SearchHighlightKind::None);
    }

    /// Out-of-bounds / None current must not panic; still classify plain matches.
    #[test]
    fn search_highlight_kind_out_of_bounds_current_does_not_panic() {
        let matches = [match_at(2, 1, 3)];

        let hl_none = SearchHighlights {
            matches: &matches,
            current: None,
        };
        assert_eq!(
            search_highlight_kind(2, 1, &hl_none),
            SearchHighlightKind::Match
        );

        let hl_oob = SearchHighlights {
            matches: &matches,
            current: Some(99),
        };
        assert_eq!(
            search_highlight_kind(2, 1, &hl_oob),
            SearchHighlightKind::Match,
            "oob current falls back to Match for covered cells"
        );
        assert_eq!(
            search_highlight_kind(2, 3, &hl_oob),
            SearchHighlightKind::None
        );

        let empty = SearchHighlights {
            matches: &[][..],
            current: Some(0),
        };
        assert_eq!(
            search_highlight_kind(0, 0, &empty),
            SearchHighlightKind::None
        );
    }

    /// Wide primary covering half-open [1, 3): col1 and col2 hit, col3 does not.
    #[test]
    fn search_highlight_kind_wide_char_primary_covers_both_columns() {
        // Wide glyph primary at col 1 with width 2 → match span [1, 3).
        let matches = [match_at(-1, 1, 3)];
        let hl = SearchHighlights {
            matches: &matches,
            current: Some(0),
        };

        assert_eq!(
            search_highlight_kind(-1, 0, &hl),
            SearchHighlightKind::None,
            "column before wide primary"
        );
        assert_eq!(
            search_highlight_kind(-1, 1, &hl),
            SearchHighlightKind::Current,
            "wide primary start col"
        );
        assert_eq!(
            search_highlight_kind(-1, 2, &hl),
            SearchHighlightKind::Current,
            "wide spacer / second grid col of primary"
        );
        assert_eq!(
            search_highlight_kind(-1, 3, &hl),
            SearchHighlightKind::None,
            "end_col exclusive — third col must not highlight"
        );
    }

    // =========================================================================
    // P0 Task 4 RED-C: cell background source priority (pure, no GPU)
    // selection > Current > Match > cell background
    // =========================================================================

    #[test]
    fn cell_background_source_priority_selection_then_search_then_cell() {
        // Selection wins over every search kind.
        assert_eq!(
            resolve_cell_background_source(true, SearchHighlightKind::Current),
            CellBackgroundSource::Selection
        );
        assert_eq!(
            resolve_cell_background_source(true, SearchHighlightKind::Match),
            CellBackgroundSource::Selection
        );
        assert_eq!(
            resolve_cell_background_source(true, SearchHighlightKind::None),
            CellBackgroundSource::Selection
        );

        // Without selection: Current > Match > Cell.
        assert_eq!(
            resolve_cell_background_source(false, SearchHighlightKind::Current),
            CellBackgroundSource::SearchCurrent
        );
        assert_eq!(
            resolve_cell_background_source(false, SearchHighlightKind::Match),
            CellBackgroundSource::SearchMatch
        );
        assert_eq!(
            resolve_cell_background_source(false, SearchHighlightKind::None),
            CellBackgroundSource::Cell
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{search_highlights_fingerprint, Renderer, SearchHighlights, TerminalPalette};
    use crate::terminal_search::SearchMatch;
    use crate::themes::theme_by_name;

    const F32_EPS: f32 = 1e-6;

    #[test]
    fn absolute_grid_selection_is_independent_of_viewport_scroll() {
        use alacritty_terminal::index::{Column, Line, Point};
        use alacritty_terminal::selection::SelectionRange;

        let selection = Some(SelectionRange::new(
            Point::new(Line(-12), Column(7)),
            Point::new(Line(-9), Column(2)),
            false,
        ));

        assert!(Renderer::is_selected(7, -12, false, selection));
        assert!(Renderer::is_selected(0, -10, false, selection));
        assert!(Renderer::is_selected(2, -9, false, selection));
        assert!(!Renderer::is_selected(6, -12, false, selection));
        assert!(!Renderer::is_selected(3, -9, false, selection));
    }

    #[test]
    fn block_selection_highlights_only_its_column_rectangle() {
        use alacritty_terminal::index::{Column, Line, Point};
        use alacritty_terminal::selection::SelectionRange;

        let selection = Some(SelectionRange::new(
            Point::new(Line(-3), Column(2)),
            Point::new(Line(-1), Column(4)),
            true,
        ));

        assert!(Renderer::is_selected(2, -3, false, selection));
        assert!(Renderer::is_selected(3, -2, false, selection));
        assert!(Renderer::is_selected(4, -1, false, selection));
        assert!(!Renderer::is_selected(1, -2, false, selection));
        assert!(!Renderer::is_selected(5, -2, false, selection));
    }

    #[test]
    fn selecting_a_wide_spacer_highlights_its_primary_glyph() {
        use alacritty_terminal::index::{Column, Line, Point};
        use alacritty_terminal::selection::SelectionRange;

        let selection = Some(SelectionRange::new(
            Point::new(Line(0), Column(1)),
            Point::new(Line(0), Column(1)),
            false,
        ));

        assert!(Renderer::is_selected(0, 0, true, selection));
        assert!(!Renderer::is_selected(0, 0, false, selection));
    }

    fn assert_rgba_f32_near(actual: [f32; 4], expected: [f32; 4]) {
        for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
            assert!(
                (a - e).abs() < F32_EPS,
                "component {i}: actual={a}, expected={e}, eps={F32_EPS}"
            );
        }
    }

    #[test]
    fn from_theme_maps_adventure_time_bytes_with_opaque_alpha() {
        let theme = theme_by_name("AdventureTime").unwrap();
        let palette = TerminalPalette::from_theme(theme);

        assert_eq!(palette.background, [0x1f, 0x1d, 0x45, 255]);
        assert_eq!(palette.foreground, [0xf8, 0xdc, 0xc0, 255]);
        assert_eq!(palette.ansi[1], [0xbd, 0x00, 0x13, 255]);
    }

    #[test]
    fn from_theme_maps_cursor_and_selection_to_normalized_f32() {
        let theme = theme_by_name("AdventureTime").unwrap();
        let palette = TerminalPalette::from_theme(theme);

        assert_rgba_f32_near(
            palette.cursor,
            [
                0xef as f32 / 255.0,
                0xbf as f32 / 255.0,
                0x38 as f32 / 255.0,
                1.0,
            ],
        );
        assert_rgba_f32_near(
            palette.selection,
            [
                0x26 as f32 / 255.0,
                0x4f as f32 / 255.0,
                0x78 as f32 / 255.0,
                1.0,
            ],
        );
    }

    #[test]
    fn from_theme_produces_distinct_palette_for_3024_day() {
        let adventure = TerminalPalette::from_theme(theme_by_name("AdventureTime").unwrap());
        let day = TerminalPalette::from_theme(theme_by_name("3024 Day").unwrap());

        assert_eq!(day.background, [0xf7, 0xf7, 0xf7, 255]);
        assert_eq!(day.foreground, [0x4a, 0x45, 0x43, 255]);
        assert_ne!(day.background, adventure.background);
        assert_ne!(day.foreground, adventure.foreground);
    }

    /// P0 Task 3 RED: 编译期锁定 `Renderer::set_font` 精确签名。
    /// 不创建 GPU/窗口；方法尚不存在时应编译失败。
    fn accept_set_font_fn(_f: fn(&mut super::Renderer, &super::GpuState, &str, f32)) {}

    #[test]
    fn set_font_signature_matches_live_font_application_api() {
        accept_set_font_fn(super::Renderer::set_font);
    }

    #[test]
    fn bootstrap_font_size_uses_shared_native_default() {
        assert_eq!(
            super::BOOTSTRAP_FONT_SIZE,
            crate::settings::DEFAULT_TERMINAL_FONT_SIZE
        );
    }

    #[test]
    fn search_fingerprint_changes_with_current_match_and_ranges() {
        let matches = [SearchMatch {
            line: 3,
            start_col: 2,
            end_col: 5,
        }];
        let first = search_highlights_fingerprint(Some(SearchHighlights {
            matches: &matches,
            current: None,
        }));
        let current = search_highlights_fingerprint(Some(SearchHighlights {
            matches: &matches,
            current: Some(0),
        }));
        let absent = search_highlights_fingerprint(None);
        assert_ne!(first, current);
        assert_ne!(first, absent);
    }
}
