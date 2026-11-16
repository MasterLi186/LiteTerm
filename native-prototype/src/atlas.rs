use cosmic_text::{
    Attrs, Buffer, CacheKeyFlags, FontSystem, Metrics, PhysicalGlyph, Shaping, Style, SwashCache,
    SwashContent, Weight,
};
use std::collections::HashMap;

/// 字形样式键：(字符, 粗体, 斜体, 是否 emoji 呈现)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub ch: char,
    pub bold: bool,
    pub italic: bool,
    pub color_emoji: bool,
}

/// 纹理图集中一个字形的位置
#[derive(Clone, Copy, Debug)]
pub struct GlyphEntry {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub bearing_x: i32,
    pub bearing_y: i32,
    pub advance_width: f32,
    /// 彩色位图（emoji）需要按原 RGBA 着色，而不是终端前景色。
    pub is_color: bool,
}

pub fn is_wide_char(c: char) -> bool {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(1) >= 2
}

fn shape_once(
    font_system: &mut FontSystem,
    metrics: Metrics,
    buffer_width: f32,
    buffer_height: f32,
    ch: char,
    attrs: Attrs<'_>,
) -> Option<PhysicalGlyph> {
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, Some(buffer_width), Some(buffer_height));
    let mut encoded = [0_u8; 4];
    buffer.set_text(
        font_system,
        ch.encode_utf8(&mut encoded),
        attrs,
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(font_system, false);
    buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter())
        .next()
        .map(|glyph| glyph.physical((0.0, 0.0), 1.0))
}

/// 终端字符格度量（物理像素）。
#[derive(Clone, Copy, Debug, PartialEq)]
struct CellMetrics {
    width: f32,
    height: f32,
    /// 基线到字符格顶部的距离。
    baseline: f32,
}

/// 按主字体的真实度量计算字符格：宽 = '0' 的 advance，高 = ascent + descent + leading，
/// 各项按整像素取整（与 GNOME 默认开启 hint metrics 时的 VTE 一致）。
/// 旧实现写死 0.5em × 1.0em（旧版 Ubuntu Mono 的比例），新版 Ubuntu Mono / Ubuntu Sans
/// Mono 的 advance 是 0.56em、行高 1.12~1.2em，会被横向压扁、行距过挤。
fn measure_cell_metrics(
    font_system: &mut FontSystem,
    family: &str,
    font_size: f32,
) -> Option<CellMetrics> {
    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, font_size * 2.0));
    buffer.set_size(font_system, None, None);
    buffer.set_text(
        font_system,
        "0",
        Attrs::new().family(cosmic_text::Family::Name(family)),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(font_system, false);
    let glyph = buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter())
        .next()?
        .clone();
    let font = font_system.get_font(glyph.font_id)?;
    let metrics = font.as_swash().metrics(&[]);
    let units_per_em = f32::from(metrics.units_per_em);
    if !(glyph.w.is_finite() && glyph.w > 0.0 && units_per_em > 0.0) {
        return None;
    }
    let ascent = metrics.ascent / units_per_em * font_size;
    let descent = metrics.descent / units_per_em * font_size;
    let leading = metrics.leading.max(0.0) / units_per_em * font_size;
    if !(ascent.is_finite() && descent.is_finite() && ascent > 0.0) {
        return None;
    }
    let width = glyph.w.round().max(1.0);
    let ascent = ascent.round();
    let leading = leading.round();
    let height = (ascent + descent.round() + leading).max(1.0);
    // 行间距上下均分。
    let baseline = ascent + (leading * 0.5).floor();
    Some(CellMetrics {
        width,
        height,
        baseline,
    })
}

/// 字体度量不可用时的旧比例。
fn fallback_cell_metrics(font_size: f32) -> CellMetrics {
    CellMetrics {
        width: (font_size * 0.5).round(),
        height: font_size,
        baseline: font_size * 0.8,
    }
}

/// 设置里的字体没安装时，退回到已安装的常见等宽字体，避免 cosmic-text 用比例字体顶替。
/// 返回字体库里的规范族名。
fn resolve_installed_family(font_system: &FontSystem, requested: &str) -> String {
    let installed = |wanted: &str| {
        font_system.db().faces().find_map(|face| {
            face.families
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
                .map(|(name, _)| name.clone())
        })
    };
    if let Some(name) = installed(requested) {
        return name;
    }
    for candidate in crate::font_support::monospace_fallback_families() {
        if let Some(name) = installed(candidate) {
            log::warn!("终端字体「{requested}」未安装，改用「{name}」");
            return name;
        }
    }
    requested.to_string()
}

#[derive(Clone, Copy)]
struct GlyphShapeRequest<'a> {
    family: &'a str,
    font_size: f32,
    cell_height: f32,
    char_width: f32,
    ch: char,
    bold: bool,
    italic: bool,
    /// 该字符按 emoji 呈现：优先尝试平台彩色 emoji 字体。
    color_emoji: bool,
}

fn shape_glyph_with_italic_fallback(
    font_system: &mut FontSystem,
    request: GlyphShapeRequest<'_>,
) -> Option<PhysicalGlyph> {
    let mut families = Vec::with_capacity(
        1 + crate::font_support::cjk_fallback_families().len()
            + crate::font_support::color_emoji_families().len(),
    );
    // emoji 呈现的字符优先用彩色字体；普通字符保持原有回退顺序。
    if request.color_emoji {
        families.extend(crate::font_support::color_emoji_families().iter().copied());
    }
    families.push(request.family);
    families.extend(
        crate::font_support::cjk_fallback_families()
            .iter()
            .copied()
            .filter(|family| *family != request.family),
    );

    for family in families {
        let mut attrs = Attrs::new().family(cosmic_text::Family::Name(family));
        if request.bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        let metrics = Metrics::new(request.font_size, request.cell_height);
        let mut shape = |attrs| {
            shape_once(
                font_system,
                metrics,
                request.char_width * 2.0,
                request.cell_height * 2.0,
                request.ch,
                attrs,
            )
            .filter(|glyph| glyph.cache_key.glyph_id != 0)
        };

        if request.italic {
            if let Some(glyph) = shape(attrs.style(Style::Italic)) {
                return Some(glyph);
            }

            // Some fallback families (notably Noto CJK) have no Italic face.
            // Shape with their real upright glyph, then let swash synthesize the slant.
            if let Some(glyph) = shape(attrs.cache_key_flags(CacheKeyFlags::FAKE_ITALIC)) {
                return Some(glyph);
            }
        } else if let Some(glyph) = shape(attrs) {
            return Some(glyph);
        }
    }

    None
}

/// 字体族是否有字重正好为 700 的粗体字面。cosmic-text 0.12 只接受字重完全匹配的 face：
/// Ubuntu Sans Mono / Ubuntu Mono 这类可变字体在 fontdb 里只登记为 400，请求粗体时
/// 会改用 Noto Sans Bold 等比例字体，等宽格子里就出现 “Publ i c” 这样的间距。
fn family_has_bold_face(font_system: &FontSystem, family: &str) -> bool {
    font_system.db().faces().any(|face| {
        face.weight == Weight::BOLD
            && face
                .families
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case(family))
    })
}

fn face_in_family(font_system: &FontSystem, id: cosmic_text::fontdb::ID, family: &str) -> bool {
    font_system.db().face(id).is_some_and(|face| {
        face.families
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(family))
    })
}

/// 假粗体的横向加粗像素数（与 FreeType 的 embolden 强度同量级，约字号的 1/20）。
fn synthetic_bold_px(font_size: f32) -> u32 {
    (font_size / 20.0).round().clamp(1.0, 4.0) as u32
}

/// 横向加粗后的遮罩取样：取原遮罩中 [x - extra, x] 范围内的最大 alpha。
fn emboldened_mask_alpha(mask: &[u8], src_width: u32, x: u32, y: u32, extra: u32) -> u8 {
    let row = y as usize * src_width as usize;
    (x.saturating_sub(extra)..=x)
        .filter(|&sx| sx < src_width)
        .map(|sx| mask[row + sx as usize])
        .max()
        .unwrap_or(0)
}

/// CPU 侧的字形纹理图集
pub struct GlyphAtlas {
    pub data: Vec<u8>,
    pub atlas_width: u32,
    pub atlas_height: u32,
    entries: HashMap<GlyphKey, GlyphEntry>,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    pub dirty: bool,
    /// 自上次上传以来新增字形占用的行范围 [dirty_top, dirty_bottom)。
    dirty_top: u32,
    dirty_bottom: u32,
    pub font_size: f32,
    pub cell_width: f32,
    pub cell_height: f32,
    /// 基线到字符格顶部的距离（物理像素）。
    baseline: f32,
    font_family: String,
    /// 主字体族有真粗体字面；没有时粗体用本字体 + 假粗体绘制。
    family_has_bold: bool,
}

impl GlyphAtlas {
    pub fn new(font_system: &mut FontSystem, swash_cache: &mut SwashCache, font_size: f32) -> Self {
        let atlas_width = 2048;
        let atlas_height = 2048;
        // RGBA8：普通字形存白色 + alpha 遮罩，彩色 emoji 存原始 RGBA。
        let data = vec![0u8; (atlas_width * atlas_height * 4) as usize];

        let font_family = resolve_installed_family(font_system, "Ubuntu Mono");
        let cell = measure_cell_metrics(font_system, &font_family, font_size)
            .unwrap_or_else(|| fallback_cell_metrics(font_size));
        let family_has_bold = family_has_bold_face(font_system, &font_family);

        let mut atlas = Self {
            data,
            atlas_width,
            atlas_height,
            entries: HashMap::new(),
            cursor_x: 1,
            cursor_y: 1,
            row_height: 0,
            dirty: true,
            dirty_top: u32::MAX,
            dirty_bottom: 0,
            font_size,
            cell_width: cell.width,
            cell_height: cell.height,
            baseline: cell.baseline,
            font_family,
            family_has_bold,
        };

        // 预填充 ASCII（normal weight）
        for ch in (0x20u8..=0x7e).map(|b| b as char) {
            atlas.ensure_glyph(font_system, swash_cache, ch, false, false, false);
        }

        atlas
    }

    pub fn font_family(&self) -> &str {
        &self.font_family
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn reset(&mut self, font_system: &mut FontSystem, family: &str, size: f32) {
        self.font_family = resolve_installed_family(font_system, family);
        self.family_has_bold = family_has_bold_face(font_system, &self.font_family);
        self.font_size = size;
        let cell = measure_cell_metrics(font_system, &self.font_family, size)
            .unwrap_or_else(|| fallback_cell_metrics(size));
        self.cell_width = cell.width;
        self.cell_height = cell.height;
        self.baseline = cell.baseline;
        self.entries.clear();
        self.data.fill(0);
        self.cursor_x = 1;
        self.cursor_y = 1;
        self.row_height = 0;
        self.dirty = true;
        self.dirty_top = u32::MAX;
        self.dirty_bottom = 0;
    }

    /// 取出自上次上传以来新增字形的行范围 `[top, bottom)`。
    pub fn take_dirty_rows(&mut self) -> Option<(u32, u32)> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        let top = self.dirty_top.min(self.atlas_height);
        let bottom = self.dirty_bottom.min(self.atlas_height);
        self.dirty_top = u32::MAX;
        self.dirty_bottom = 0;
        (top < bottom).then_some((top, bottom))
    }

    pub fn ensure_glyph(
        &mut self,
        font_system: &mut FontSystem,
        swash_cache: &mut SwashCache,
        ch: char,
        bold: bool,
        italic: bool,
        color_emoji: bool,
    ) -> Option<GlyphEntry> {
        let key = GlyphKey {
            ch,
            bold,
            italic,
            color_emoji,
        };
        if let Some(entry) = self.entries.get(&key) {
            return Some(*entry);
        }

        let char_width = if is_wide_char(ch) {
            self.cell_width * 2.0
        } else {
            self.cell_width
        };
        let request = GlyphShapeRequest {
            family: &self.font_family,
            font_size: self.font_size,
            cell_height: self.cell_height,
            char_width,
            ch,
            bold,
            italic,
            color_emoji,
        };
        // 主字体族没有粗体字面时，先按常规字重在本字体里塑形，再做假粗体；
        // 本字体不含的字符（CJK 等回退字体）仍按原逻辑请求真粗体。
        let mut synthetic_bold = bold && !self.family_has_bold;
        let mut physical = shape_glyph_with_italic_fallback(
            font_system,
            GlyphShapeRequest {
                bold: bold && !synthetic_bold,
                ..request
            },
        )?;
        if synthetic_bold
            && !face_in_family(font_system, physical.cache_key.font_id, request.family)
        {
            synthetic_bold = false;
            physical = shape_glyph_with_italic_fallback(font_system, request)?;
        }
        let image = swash_cache
            .get_image(font_system, physical.cache_key)
            .as_ref()?;
        let src_w = image.placement.width;
        let h = image.placement.height;
        if src_w == 0 || h == 0 {
            return None;
        }
        let is_mask = image.content == SwashContent::Mask;
        let embolden = if synthetic_bold && is_mask {
            synthetic_bold_px(self.font_size)
        } else {
            0
        };
        let w = src_w + embolden;

        if self.cursor_x + w + 1 >= self.atlas_width {
            self.cursor_x = 1;
            self.cursor_y += self.row_height + 1;
            self.row_height = 0;
        }
        if self.cursor_y + h + 1 >= self.atlas_height {
            return None;
        }

        // 彩色 emoji（Noto Color Emoji 等）返回 32 位 RGBA 位图；普通字形是单通道
        // alpha 遮罩。图集统一为 RGBA8：普通字形写白色 + alpha，彩色字形原样拷贝，
        // 由着色器按 is_color 决定用前景色还是位图颜色。
        let bytes_per_pixel = if is_mask { 1 } else { 4 };
        let required = (src_w as usize) * (h as usize) * bytes_per_pixel;
        if image.data.len() < required {
            return None;
        }
        for iy in 0..h {
            for ix in 0..w {
                let src = (iy as usize * src_w as usize + ix as usize) * bytes_per_pixel;
                let dst =
                    (((self.cursor_y + iy) * self.atlas_width + self.cursor_x + ix) as usize) * 4;
                if dst + 4 > self.data.len() || self.cursor_x + ix >= self.atlas_width {
                    continue;
                }
                if is_mask {
                    self.data[dst] = 255;
                    self.data[dst + 1] = 255;
                    self.data[dst + 2] = 255;
                    self.data[dst + 3] =
                        emboldened_mask_alpha(&image.data, src_w, ix, iy, embolden);
                } else {
                    self.data[dst..dst + 4].copy_from_slice(&image.data[src..src + 4]);
                }
            }
        }

        self.dirty_top = self.dirty_top.min(self.cursor_y);
        self.dirty_bottom = self.dirty_bottom.max(self.cursor_y + h);
        let entry = GlyphEntry {
            x: self.cursor_x,
            y: self.cursor_y,
            width: w,
            height: h,
            // 假粗体向右加宽了 embolden 像素，左移一半保持字形居中。
            bearing_x: physical.x + image.placement.left - (embolden / 2) as i32,
            bearing_y: physical.y - image.placement.top + self.baseline as i32,
            advance_width: char_width,
            is_color: !is_mask,
        };

        self.entries.insert(key, entry);
        self.cursor_x += w + 1;
        self.row_height = self.row_height.max(h);
        self.dirty = true;

        Some(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::{shape_glyph_with_italic_fallback, GlyphAtlas, GlyphShapeRequest};
    use cosmic_text::{CacheKeyFlags, FontSystem, Style, SwashCache};

    #[test]
    fn atlas_new_is_not_empty_after_ascii_prefill() {
        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        let atlas = GlyphAtlas::new(&mut font_system, &mut swash_cache, 26.0);
        assert!(!atlas.is_empty(), "GlyphAtlas::new 预填充 ASCII 后不应为空");
    }

    #[test]
    fn atlas_reset_updates_metrics_and_drops_cached_glyphs() {
        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        let mut atlas = GlyphAtlas::new(&mut font_system, &mut swash_cache, 26.0);

        atlas.reset(&mut font_system, "Ubuntu Mono", 18.0);

        assert!(!atlas.font_family().is_empty());
        assert_eq!(atlas.font_size, 18.0);
        // 字符格来自字体真实度量：等宽字体的 advance 约 0.5~0.65em，行高不小于 1em。
        assert!(
            (9.0..=12.0).contains(&atlas.cell_width),
            "{}",
            atlas.cell_width
        );
        assert!(
            (18.0..=24.0).contains(&atlas.cell_height),
            "{}",
            atlas.cell_height
        );
        assert!(atlas.baseline > 0.0 && atlas.baseline < atlas.cell_height);
        assert!(atlas.is_empty());
        assert!(atlas.dirty);
    }

    #[test]
    fn real_italic_face_is_preferred_when_it_contains_the_glyph() {
        let mut font_system = FontSystem::new();
        let glyph = shape_glyph_with_italic_fallback(
            &mut font_system,
            GlyphShapeRequest {
                family: "Ubuntu Mono",
                font_size: 22.0,
                cell_height: 22.0,
                char_width: 11.0,
                ch: 'A',
                bold: false,
                italic: true,
                color_emoji: false,
            },
        )
        .expect("Ubuntu Mono Italic should contain ASCII");
        let face = font_system
            .db()
            .face(glyph.cache_key.font_id)
            .expect("shaped font should remain in font database");

        assert_eq!(face.style, Style::Italic);
        assert!(!glyph.cache_key.flags.contains(CacheKeyFlags::FAKE_ITALIC));
    }

    #[test]
    fn missing_real_italic_glyph_uses_upright_fallback_with_synthetic_slant() {
        let mut font_system = FontSystem::new();
        let Some(glyph) = shape_glyph_with_italic_fallback(
            &mut font_system,
            GlyphShapeRequest {
                family: "Ubuntu Mono",
                font_size: 22.0,
                cell_height: 22.0,
                char_width: 22.0,
                ch: '你',
                bold: false,
                italic: true,
                color_emoji: false,
            },
        ) else {
            eprintln!("skipping CJK fallback assertion: no CJK font is installed");
            return;
        };
        let face = font_system
            .db()
            .face(glyph.cache_key.font_id)
            .expect("fallback font should remain in font database");

        assert_ne!(glyph.cache_key.glyph_id, 0);
        assert_eq!(face.style, Style::Normal);
        assert!(glyph.cache_key.flags.contains(CacheKeyFlags::FAKE_ITALIC));
    }

    #[test]
    fn normal_cjk_glyph_populates_non_empty_atlas_pixels() {
        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        let mut atlas = GlyphAtlas::new(&mut font_system, &mut swash_cache, 22.0);

        let Some(glyph) = atlas.ensure_glyph(
            &mut font_system,
            &mut swash_cache,
            '中',
            false,
            false,
            false,
        ) else {
            eprintln!("skipping CJK atlas assertion: no CJK font is installed");
            return;
        };
        let has_ink = (glyph.y..glyph.y + glyph.height).any(|y| {
            (glyph.x..glyph.x + glyph.width)
                .any(|x| atlas.data[((y * atlas.atlas_width + x) as usize) * 4 + 3] != 0)
        });

        assert!(has_ink, "CJK 字形区域必须包含非零像素");
    }
}
