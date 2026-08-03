use cosmic_text::{
    Attrs, Buffer, CacheKeyFlags, FontSystem, Metrics, PhysicalGlyph, Shaping, Style, SwashCache,
    Weight,
};
use std::collections::HashMap;

/// 字形样式键：(字符, 粗体, 斜体)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub ch: char,
    pub bold: bool,
    pub italic: bool,
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
}

pub fn is_wide_char(c: char) -> bool {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(1) >= 2
}

pub fn is_word_char(c: char) -> bool {
    !matches!(
        c,
        ' ' | '\t'
            | '\0'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '\''
            | '"'
            | '`'
            | ','
            | ';'
            | ':'
            | '<'
            | '>'
            | '|'
            | '&'
            | '（'
            | '）'
            | '【'
            | '】'
            | '「'
            | '」'
            | '『'
            | '』'
            | '《'
            | '》'
            | '〈'
            | '〉'
            | '，'
            | '。'
            | '；'
            | '：'
    )
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

#[derive(Clone, Copy)]
struct GlyphShapeRequest<'a> {
    family: &'a str,
    font_size: f32,
    cell_height: f32,
    char_width: f32,
    ch: char,
    bold: bool,
    italic: bool,
}

fn shape_glyph_with_italic_fallback(
    font_system: &mut FontSystem,
    request: GlyphShapeRequest<'_>,
) -> Option<PhysicalGlyph> {
    let metrics = Metrics::new(request.font_size, request.cell_height);
    let mut attrs = Attrs::new().family(cosmic_text::Family::Name(request.family));
    if request.bold {
        attrs = attrs.weight(Weight::BOLD);
    }

    if request.italic {
        let real_italic = shape_once(
            font_system,
            metrics,
            request.char_width * 2.0,
            request.cell_height * 2.0,
            request.ch,
            attrs.style(Style::Italic),
        );
        if real_italic
            .as_ref()
            .is_some_and(|glyph| glyph.cache_key.glyph_id != 0)
        {
            return real_italic;
        }

        // Some fallback families (notably Noto CJK) have no Italic face.
        // Shape with their real upright glyph, then let swash synthesize the slant.
        attrs = attrs.cache_key_flags(CacheKeyFlags::FAKE_ITALIC);
    }

    shape_once(
        font_system,
        metrics,
        request.char_width * 2.0,
        request.cell_height * 2.0,
        request.ch,
        attrs,
    )
    .filter(|glyph| glyph.cache_key.glyph_id != 0)
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
    pub font_size: f32,
    pub cell_width: f32,
    pub cell_height: f32,
    font_family: String,
}

impl GlyphAtlas {
    pub fn new(font_system: &mut FontSystem, swash_cache: &mut SwashCache, font_size: f32) -> Self {
        let atlas_width = 2048;
        let atlas_height = 2048;
        let data = vec![0u8; (atlas_width * atlas_height) as usize];

        // 对齐 guishell/xterm.js 实测值：size=26 → cellReal=13x26
        let cell_width = (font_size * 0.5).round();
        let line_height = font_size;

        let mut atlas = Self {
            data,
            atlas_width,
            atlas_height,
            entries: HashMap::new(),
            cursor_x: 1,
            cursor_y: 1,
            row_height: 0,
            dirty: true,
            font_size,
            cell_width,
            cell_height: line_height,
            font_family: "Ubuntu Mono".to_string(),
        };

        // 预填充 ASCII（normal weight）
        for ch in (0x20u8..=0x7e).map(|b| b as char) {
            atlas.ensure_glyph(font_system, swash_cache, ch, false, false);
        }

        atlas
    }

    pub fn font_family(&self) -> &str {
        &self.font_family
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn reset(&mut self, family: &str, size: f32) {
        self.font_family = family.to_string();
        self.font_size = size;
        self.cell_width = (size * 0.5).round();
        self.cell_height = size;
        self.entries.clear();
        self.data.fill(0);
        self.cursor_x = 1;
        self.cursor_y = 1;
        self.row_height = 0;
        self.dirty = true;
    }

    pub fn ensure_glyph(
        &mut self,
        font_system: &mut FontSystem,
        swash_cache: &mut SwashCache,
        ch: char,
        bold: bool,
        italic: bool,
    ) -> Option<GlyphEntry> {
        let key = GlyphKey { ch, bold, italic };
        if let Some(entry) = self.entries.get(&key) {
            return Some(*entry);
        }

        let char_width = if is_wide_char(ch) {
            self.cell_width * 2.0
        } else {
            self.cell_width
        };
        let physical = shape_glyph_with_italic_fallback(
            font_system,
            GlyphShapeRequest {
                family: &self.font_family,
                font_size: self.font_size,
                cell_height: self.cell_height,
                char_width,
                ch,
                bold,
                italic,
            },
        )?;
        let image = swash_cache
            .get_image(font_system, physical.cache_key)
            .as_ref()?;
        let w = image.placement.width;
        let h = image.placement.height;
        if w == 0 || h == 0 {
            return None;
        }

        if self.cursor_x + w + 1 >= self.atlas_width {
            self.cursor_x = 1;
            self.cursor_y += self.row_height + 1;
            self.row_height = 0;
        }
        if self.cursor_y + h + 1 >= self.atlas_height {
            return None;
        }

        for iy in 0..h {
            for ix in 0..w {
                let src = (iy * w + ix) as usize;
                let dst = ((self.cursor_y + iy) * self.atlas_width + self.cursor_x + ix) as usize;
                if src < image.data.len() && dst < self.data.len() {
                    self.data[dst] = image.data[src];
                }
            }
        }

        let entry = GlyphEntry {
            x: self.cursor_x,
            y: self.cursor_y,
            width: w,
            height: h,
            bearing_x: physical.x + image.placement.left,
            bearing_y: physical.y - image.placement.top + (self.cell_height * 0.8) as i32,
            advance_width: char_width,
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

        atlas.reset("Noto Sans Mono", 18.0);

        assert_eq!(atlas.font_family(), "Noto Sans Mono");
        assert_eq!(atlas.font_size, 18.0);
        assert_eq!(atlas.cell_width, 9.0);
        assert_eq!(atlas.cell_height, 18.0);
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

        let Some(glyph) =
            atlas.ensure_glyph(&mut font_system, &mut swash_cache, '中', false, false)
        else {
            eprintln!("skipping CJK atlas assertion: no CJK font is installed");
            return;
        };
        let has_ink = (glyph.y..glyph.y + glyph.height).any(|y| {
            (glyph.x..glyph.x + glyph.width)
                .any(|x| atlas.data[(y * atlas.atlas_width + x) as usize] != 0)
        });

        assert!(has_ink, "CJK 字形区域必须包含非零像素");
    }
}
