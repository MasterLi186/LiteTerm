use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub super_key: bool,
    pub key: String,
}

impl KeyChord {
    pub fn parse(s: &str) -> Result<KeyChord, String> {
        if s.is_empty() {
            return Err("快捷键不能为空".to_string());
        }

        let parts: Vec<&str> = s.split('+').collect();
        if parts.iter().any(|p| p.is_empty()) {
            return Err("快捷键格式无效".to_string());
        }

        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut super_key = false;
        let mut key: Option<String> = None;

        for part in parts {
            match normalize_modifier(part) {
                Some(Modifier::Ctrl) => {
                    if ctrl {
                        return Err("重复的修饰键".to_string());
                    }
                    ctrl = true;
                }
                Some(Modifier::Shift) => {
                    if shift {
                        return Err("重复的修饰键".to_string());
                    }
                    shift = true;
                }
                Some(Modifier::Alt) => {
                    if alt {
                        return Err("重复的修饰键".to_string());
                    }
                    alt = true;
                }
                Some(Modifier::Super) => {
                    if super_key {
                        return Err("重复的修饰键".to_string());
                    }
                    super_key = true;
                }
                None => {
                    if key.is_some() {
                        return Err(format!("未知修饰键：{part}"));
                    }
                    key = Some(normalize_key(part)?);
                }
            }
        }

        let key = key.ok_or_else(|| "快捷键缺少主键".to_string())?;

        Ok(KeyChord {
            ctrl,
            shift,
            alt,
            super_key,
            key,
        })
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.super_key {
            parts.push("Super");
        }
        parts.push(self.key.as_str());
        write!(f, "{}", parts.join("+"))
    }
}

enum Modifier {
    Ctrl,
    Shift,
    Alt,
    Super,
}

fn normalize_modifier(token: &str) -> Option<Modifier> {
    match token.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(Modifier::Ctrl),
        "shift" => Some(Modifier::Shift),
        "alt" => Some(Modifier::Alt),
        "super" | "meta" | "cmd" | "win" => Some(Modifier::Super),
        _ => None,
    }
}

fn normalize_key(token: &str) -> Result<String, String> {
    if token.eq_ignore_ascii_case("tab") {
        return Ok("Tab".to_string());
    }

    if token.len() == 1 {
        let c = token.chars().next().unwrap();
        if c.is_ascii_alphanumeric() {
            return Ok(c.to_ascii_uppercase().to_string());
        }
    }

    // Function keys F1–F12
    if token.len() >= 2 && token.len() <= 3 {
        let upper = token.to_ascii_uppercase();
        if let Some(rest) = upper.strip_prefix('F') {
            if let Ok(n) = rest.parse::<u8>() {
                if (1..=12).contains(&n) {
                    return Ok(format!("F{n}"));
                }
            }
        }
    }

    Err(format!("不支持的按键：{token}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    NewTab,
    CloseTab,
    Search,
    Copy,
    Paste,
    NextTab,
    PreviousTab,
}

impl ShortcutAction {
    pub fn label(self) -> &'static str {
        match self {
            ShortcutAction::NewTab => "新建标签页",
            ShortcutAction::CloseTab => "关闭标签页",
            ShortcutAction::Search => "搜索",
            ShortcutAction::Copy => "复制",
            ShortcutAction::Paste => "粘贴",
            ShortcutAction::NextTab => "下一个标签页",
            ShortcutAction::PreviousTab => "上一个标签页",
        }
    }

    pub fn all() -> [ShortcutAction; 7] {
        [
            ShortcutAction::NewTab,
            ShortcutAction::CloseTab,
            ShortcutAction::Search,
            ShortcutAction::Copy,
            ShortcutAction::Paste,
            ShortcutAction::NextTab,
            ShortcutAction::PreviousTab,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ShortcutSettings {
    pub new_tab: String,
    pub close_tab: String,
    pub search: String,
    pub copy: String,
    pub paste: String,
    pub next_tab: String,
    pub previous_tab: String,
}

impl Default for ShortcutSettings {
    fn default() -> Self {
        Self {
            new_tab: "Ctrl+Shift+T".to_string(),
            close_tab: "Ctrl+Shift+W".to_string(),
            search: "Ctrl+F".to_string(),
            copy: "Ctrl+Shift+C".to_string(),
            paste: "Ctrl+Shift+V".to_string(),
            next_tab: "Ctrl+Tab".to_string(),
            previous_tab: "Ctrl+Shift+Tab".to_string(),
        }
    }
}

impl ShortcutSettings {
    pub fn binding(&self, action: ShortcutAction) -> &str {
        match action {
            ShortcutAction::NewTab => &self.new_tab,
            ShortcutAction::CloseTab => &self.close_tab,
            ShortcutAction::Search => &self.search,
            ShortcutAction::Copy => &self.copy,
            ShortcutAction::Paste => &self.paste,
            ShortcutAction::NextTab => &self.next_tab,
            ShortcutAction::PreviousTab => &self.previous_tab,
        }
    }

    pub fn set_binding(&mut self, action: ShortcutAction, binding: String) {
        match action {
            ShortcutAction::NewTab => self.new_tab = binding,
            ShortcutAction::CloseTab => self.close_tab = binding,
            ShortcutAction::Search => self.search = binding,
            ShortcutAction::Copy => self.copy = binding,
            ShortcutAction::Paste => self.paste = binding,
            ShortcutAction::NextTab => self.next_tab = binding,
            ShortcutAction::PreviousTab => self.previous_tab = binding,
        }
    }

    pub fn chord(&self, action: ShortcutAction) -> Result<KeyChord, String> {
        KeyChord::parse(self.binding(action))
    }

    pub fn validate(&self) -> Result<(), String> {
        let mut seen: Vec<(KeyChord, ShortcutAction)> = Vec::new();

        for action in ShortcutAction::all() {
            let chord = self.chord(action)?;
            if let Some((_, first)) = seen.iter().find(|(c, _)| c == &chord) {
                return Err(format!(
                    "快捷键冲突：{} 与 {}",
                    first.label(),
                    action.label()
                ));
            }
            seen.push((chord, action));
        }

        Ok(())
    }

    pub fn match_action(
        &self,
        key: &winit::keyboard::Key,
        modifiers: winit::keyboard::ModifiersState,
    ) -> Option<ShortcutAction> {
        let pressed = key_token_from_winit(key)?;
        let ctrl = modifiers.control_key();
        let shift = modifiers.shift_key();
        let alt = modifiers.alt_key();
        let super_key = modifiers.super_key();

        for action in ShortcutAction::all() {
            let Ok(chord) = self.chord(action) else {
                continue;
            };
            if chord.ctrl == ctrl
                && chord.shift == shift
                && chord.alt == alt
                && chord.super_key == super_key
                && chord.key == pressed
            {
                return Some(action);
            }
        }
        None
    }
}

fn key_token_from_winit(key: &winit::keyboard::Key) -> Option<String> {
    use winit::keyboard::{Key, NamedKey};

    match key {
        Key::Character(s) => {
            let s = s.as_str();
            if s.len() == 1 {
                let c = s.chars().next().unwrap();
                if c.is_ascii_alphanumeric() {
                    return Some(c.to_ascii_uppercase().to_string());
                }
            }
            None
        }
        Key::Named(NamedKey::Tab) => Some("Tab".to_string()),
        Key::Named(NamedKey::F1) => Some("F1".to_string()),
        Key::Named(NamedKey::F2) => Some("F2".to_string()),
        Key::Named(NamedKey::F3) => Some("F3".to_string()),
        Key::Named(NamedKey::F4) => Some("F4".to_string()),
        Key::Named(NamedKey::F5) => Some("F5".to_string()),
        Key::Named(NamedKey::F6) => Some("F6".to_string()),
        Key::Named(NamedKey::F7) => Some("F7".to_string()),
        Key::Named(NamedKey::F8) => Some("F8".to_string()),
        Key::Named(NamedKey::F9) => Some("F9".to_string()),
        Key::Named(NamedKey::F10) => Some("F10".to_string()),
        Key::Named(NamedKey::F11) => Some("F11".to_string()),
        Key::Named(NamedKey::F12) => Some("F12".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{KeyChord, ShortcutAction, ShortcutSettings};

    #[test]
    fn parses_normalized_chord() {
        let chord = KeyChord::parse("Ctrl+Shift+F").unwrap();
        assert!(chord.ctrl);
        assert!(chord.shift);
        assert_eq!(chord.key, "F");
        assert_eq!(chord.to_string(), "Ctrl+Shift+F");
    }

    #[test]
    fn rejects_invalid_and_empty_chords() {
        assert!(KeyChord::parse("").is_err());
        assert!(KeyChord::parse("NotAChord").is_err());
        assert!(KeyChord::parse("Ctrl+").is_err());
    }

    #[test]
    fn rejects_duplicate_shortcuts() {
        let mut settings = ShortcutSettings::default();
        settings.close_tab = settings.new_tab.clone();
        assert_eq!(
            settings.validate().unwrap_err(),
            "快捷键冲突：新建标签页 与 关闭标签页"
        );
    }

    /// P0 Task 3 RED: `ShortcutAction::all()` 契约 — 恰好七项且每项 label 非空。
    /// 直接引用当前私有 API，促使后续按需公开。
    #[test]
    fn shortcut_action_all_returns_exactly_seven_nonempty_labels() {
        let all = ShortcutAction::all();
        assert_eq!(all.len(), 7, "ShortcutAction::all() 必须恰好返回 7 项");
        for action in all {
            let label = action.label();
            assert!(!label.is_empty(), "{action:?} 的 label 不能为空");
        }
    }

    // --- P0 Task 3 RED-E: winit logical Key + ModifiersState → ShortcutAction ---

    /// 默认 Ctrl+Shift+T / Ctrl+Tab 必须能从 winit 逻辑键匹配到对应动作。
    #[test]
    fn match_action_default_new_tab_and_next_tab_from_winit_key() {
        use winit::keyboard::{Key, ModifiersState, NamedKey};

        let settings = ShortcutSettings::default();
        assert_eq!(
            settings.match_action(
                &Key::Character("t".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
            ),
            Some(ShortcutAction::NewTab),
            "默认 Ctrl+Shift+T 应匹配 NewTab"
        );
        assert_eq!(
            settings.match_action(&Key::Named(NamedKey::Tab), ModifiersState::CONTROL),
            Some(ShortcutAction::NextTab),
            "默认 Ctrl+Tab 应匹配 NextTab"
        );
    }

    /// 自定义 Ctrl+F 可匹配 Search；额外 Shift 必须精确失败，不能误触发。
    #[test]
    fn match_action_custom_ctrl_f_requires_exact_modifiers() {
        use winit::keyboard::{Key, ModifiersState};

        let mut settings = ShortcutSettings::default();
        settings.set_binding(ShortcutAction::Search, "Ctrl+F".into());

        assert_eq!(
            settings.match_action(&Key::Character("f".into()), ModifiersState::CONTROL),
            Some(ShortcutAction::Search),
            "自定义 Ctrl+F 应匹配 Search"
        );
        assert_eq!(
            settings.match_action(
                &Key::Character("f".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
            ),
            None,
            "额外 Shift 不得误触发仅绑定了 Ctrl+F 的 Search"
        );
    }
}
