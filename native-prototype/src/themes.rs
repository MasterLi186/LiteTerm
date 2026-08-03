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

pub fn all_themes() -> &'static [TerminalTheme] {
    generated::TERMINAL_THEMES
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
        assert_eq!(all_themes().len(), 191);
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
