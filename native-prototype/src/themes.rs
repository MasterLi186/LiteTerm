#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalTheme {
    pub name: &'static str,
    pub background: [u8; 3],
    pub foreground: [u8; 3],
    pub cursor: [u8; 3],
    pub selection: [u8; 3],
    pub ansi: [[u8; 3]; 16],
}

mod generated {
    include!("themes_generated.rs");
}

/// 与 Ubuntu 自带 GNOME Terminal 一致的配色：背景/前景取自本机终端配置
/// （#282C34 / #FFFFFF），16 色为 GNOME Terminal 默认调色板；
/// 光标用前景色，选区沿用渲染器的反色绘制（与 VTE 默认一致）。
pub const GNOME_TERMINAL_THEME_NAME: &str = "GNOME Terminal";

const NATIVE_THEMES: &[TerminalTheme] = &[TerminalTheme {
    name: GNOME_TERMINAL_THEME_NAME,
    background: [0x28, 0x2c, 0x34],
    foreground: [0xff, 0xff, 0xff],
    cursor: [0xff, 0xff, 0xff],
    selection: [0xff, 0xff, 0xff],
    ansi: [
        [0x17, 0x14, 0x21],
        [0xc0, 0x1c, 0x28],
        [0x26, 0xa2, 0x69],
        [0xa2, 0x73, 0x4c],
        [0x12, 0x48, 0x8b],
        [0xa3, 0x47, 0xba],
        [0x2a, 0xa1, 0xb3],
        [0xd0, 0xcf, 0xcc],
        [0x5e, 0x5c, 0x64],
        [0xf6, 0x61, 0x51],
        [0x33, 0xda, 0x7a],
        [0xe9, 0xad, 0x0c],
        [0x2a, 0x7b, 0xde],
        [0xc0, 0x61, 0xcb],
        [0x33, 0xc7, 0xde],
        [0xff, 0xff, 0xff],
    ],
}];

static ALL_THEMES: std::sync::LazyLock<Vec<TerminalTheme>> = std::sync::LazyLock::new(|| {
    NATIVE_THEMES
        .iter()
        .chain(generated::TERMINAL_THEMES)
        .copied()
        .collect()
});

pub fn all_themes() -> &'static [TerminalTheme] {
    &ALL_THEMES
}

pub fn theme_by_name(name: &str) -> Option<&'static TerminalTheme> {
    all_themes().iter().find(|theme| theme.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn generated_catalog_matches_tabby_source_count() {
        assert_eq!(generated::TERMINAL_THEMES.len(), 191);
        assert_eq!(
            all_themes().len(),
            NATIVE_THEMES.len() + generated::TERMINAL_THEMES.len()
        );
    }

    #[test]
    fn adventure_time_matches_existing_native_palette() {
        let theme = theme_by_name("AdventureTime").unwrap();
        assert_eq!(theme.background, [0x1f, 0x1d, 0x45]);
        assert_eq!(theme.foreground, [0xf8, 0xdc, 0xc0]);
        assert_eq!(theme.ansi[1], [0xbd, 0x00, 0x13]);
    }

    #[test]
    fn theme_names_are_unique() {
        let names = all_themes()
            .iter()
            .map(|theme| theme.name)
            .collect::<HashSet<_>>();
        assert_eq!(names.len(), all_themes().len());
    }
}
