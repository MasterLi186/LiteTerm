use std::collections::HashMap;
use cosmic_text::{
    Attrs, Buffer, CacheKey, FontSystem, Metrics, Shaping, SwashCache,
};

/// 纹理图集中一个字形的位置
#[derive(Clone, Copy, Debug)]
pub struct GlyphEntry {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub bearing_x: i32,
    pub bearing_y: i32,
    pub advance_width: f32, // 字符实际推进宽度（CJK = cell_width * 2）
}

pub fn is_wide_char(c: char) -> bool {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(1) >= 2
}

/// CPU 侧的字形纹理图集
pub struct GlyphAtlas {
    pub data: Vec<u8>,
    pub atlas_width: u32,
    pub atlas_height: u32,
    entries: HashMap<char, GlyphEntry>,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    pub dirty: bool,
    pub font_size: f32,
    pub cell_width: f32,
    pub cell_height: f32,
}

impl GlyphAtlas {
    pub fn new(font_system: &mut FontSystem, swash_cache: &mut SwashCache, font_size: f32) -> Self {
        let atlas_width = 1024;
        let atlas_height = 1024;
        let data = vec![0u8; (atlas_width * atlas_height) as usize];
        let line_height = font_size * 1.2;
        let cell_width = font_size * 0.6;

        let mut atlas = Self {
            data,
            atlas_width,
            atlas_height,
            entries: HashMap::new(),
            cursor_x: 1, // 留 1px 边距防采样溢出
            cursor_y: 1,
            row_height: 0,
            dirty: true,
            font_size,
            cell_width,
            cell_height: line_height,
        };

        // 预填充 ASCII 可打印字符
        for ch in (0x20u8..=0x7e).map(|b| b as char) {
            atlas.ensure_glyph(font_system, swash_cache, ch);
        }

        atlas
    }

    /// 确保字符在图集中，返回 entry
    pub fn ensure_glyph(
        &mut self,
        font_system: &mut FontSystem,
        swash_cache: &mut SwashCache,
        ch: char,
    ) -> Option<GlyphEntry> {
        if let Some(entry) = self.entries.get(&ch) {
            return Some(*entry);
        }

        let char_width = if is_wide_char(ch) { self.cell_width * 2.0 } else { self.cell_width };
        let metrics = Metrics::new(self.font_size, self.cell_height);
        let mut buf = Buffer::new(font_system, metrics);
        buf.set_size(font_system, Some(char_width * 2.0), Some(self.cell_height * 2.0));
        let attrs = Attrs::new().family(cosmic_text::Family::Monospace);
        let mut s = [0u8; 4];
        let ch_str = ch.encode_utf8(&mut s);
        buf.set_text(font_system, ch_str, attrs, Shaping::Advanced);
        buf.shape_until_scroll(font_system, false);

        for run in buf.layout_runs() {
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                if let Some(image) = swash_cache.get_image(font_system, physical.cache_key) {
                    let w = image.placement.width as u32;
                    let h = image.placement.height as u32;
                    if w == 0 || h == 0 {
                        return None;
                    }

                    // 换行
                    if self.cursor_x + w + 1 >= self.atlas_width {
                        self.cursor_x = 1;
                        self.cursor_y += self.row_height + 1;
                        self.row_height = 0;
                    }
                    // 图集满了
                    if self.cursor_y + h + 1 >= self.atlas_height {
                        return None;
                    }

                    // 复制字形数据到图集
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
                        bearing_y: physical.y - image.placement.top + self.font_size as i32,
                        advance_width: char_width,
                    };

                    self.entries.insert(ch, entry);
                    self.cursor_x += w + 1;
                    self.row_height = self.row_height.max(h);
                    self.dirty = true;

                    return Some(entry);
                }
            }
        }
        None
    }

    pub fn get(&self, ch: char) -> Option<&GlyphEntry> {
        self.entries.get(&ch)
    }
}
