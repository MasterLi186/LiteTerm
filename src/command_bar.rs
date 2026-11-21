use serde::{Deserialize, Serialize};

mod popup_geometry;

use popup_geometry::{above_button, constrained_size};

pub const COMMAND_BAR_HEIGHT: f32 = 50.0; // 两行：按钮 + 输入框

const MAX_HISTORY: usize = 50;
const HISTORY_POPUP_WIDTH: f32 = 384.0;
const HISTORY_POPUP_MAX_HEIGHT: f32 = 288.0;
const HISTORY_POPUP_MARGIN: f32 = popup_geometry::POPUP_MARGIN;
const HISTORY_POPUP_ROW_HEIGHT: f32 = 28.0;
const HISTORY_POPUP_EMPTY_HEIGHT: f32 = 80.0;
const HISTORY_POPUP_CHROME_HEIGHT: f32 = 48.0;
const HISTORY_POPUP_ACTION_SLOT_WIDTH: f32 = 20.0;
const HISTORY_POPUP_ACTION_COUNT: f32 = 4.0;
const HISTORY_POPUP_ACTION_TRAILING_PADDING: f32 = 4.0;
const HISTORY_POPUP_MIN_COMMAND_REGION_WIDTH: f32 = 56.0;
const HISTORY_POPUP_ACTIONS_WIDTH: f32 = HISTORY_POPUP_ACTION_SLOT_WIDTH
    * HISTORY_POPUP_ACTION_COUNT
    + HISTORY_POPUP_ACTION_TRAILING_PADDING;
const HISTORY_POPUP_ACTIONS_MIN_ROW_WIDTH: f32 =
    HISTORY_POPUP_ACTIONS_WIDTH + HISTORY_POPUP_MIN_COMMAND_REGION_WIDTH;

fn history_popup_size(history_len: usize, screen: egui::Rect) -> egui::Vec2 {
    let desired_height = if history_len == 0 {
        HISTORY_POPUP_EMPTY_HEIGHT
    } else {
        HISTORY_POPUP_CHROME_HEIGHT + HISTORY_POPUP_ROW_HEIGHT * history_len.min(MAX_HISTORY) as f32
    }
    .min(HISTORY_POPUP_MAX_HEIGHT);

    constrained_size(screen, egui::vec2(HISTORY_POPUP_WIDTH, desired_height))
}

fn history_popup_position(button: egui::Rect, size: egui::Vec2, screen: egui::Rect) -> egui::Pos2 {
    above_button(button, size, screen)
}

const FAVORITES_POPUP_WIDTH: f32 = 360.0;
const FAVORITES_POPUP_MAX_HEIGHT: f32 = 280.0;
const FAVORITES_POPUP_EMPTY_HEIGHT: f32 = 96.0;
const FAVORITES_POPUP_CHROME_HEIGHT: f32 = 72.0;
const FAVORITES_POPUP_ROW_HEIGHT: f32 = 24.0;

fn favorites_popup_size(favorites_len: usize, screen: egui::Rect) -> egui::Vec2 {
    let desired_height = if favorites_len == 0 {
        FAVORITES_POPUP_EMPTY_HEIGHT
    } else {
        (FAVORITES_POPUP_CHROME_HEIGHT + FAVORITES_POPUP_ROW_HEIGHT * favorites_len as f32)
            .min(FAVORITES_POPUP_MAX_HEIGHT)
    };
    constrained_size(screen, egui::vec2(FAVORITES_POPUP_WIDTH, desired_height))
}

fn history_popup_layer_ids() -> (egui::LayerId, egui::LayerId) {
    (
        egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("command_history_backdrop"),
        ),
        egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("command_history_popup"),
        ),
    )
}

fn history_popup_row_id(index: usize) -> egui::Id {
    egui::Id::new(("command_history_row", index))
}

fn favorite_display_text(label: &str, command: &str) -> String {
    let label = label.trim();
    let command = command.trim();
    if label.is_empty() || label == command {
        command.to_owned()
    } else {
        format!("{label}  ·  {command}")
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct QuickCommand {
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub system: bool,
}

pub struct CommandBar {
    storage: CommandBarStorage,
    pub commands: Vec<QuickCommand>,
    pub input_text: String,
    /// 最近执行的命令（不含末尾换行）
    history: Vec<String>,
    /// 收藏命令
    favorites: Vec<QuickCommand>,
    show_history: bool,
    show_favorites: bool,
    show_add: bool,
    add_label: String,
    add_command: String,
    add_error: String,
    /// 编辑快捷命令的下标（None = 新增）
    edit_index: Option<usize>,
    last_history_button_rect: Option<egui::Rect>,
    last_favorites_button_rect: Option<egui::Rect>,
}

#[derive(Clone)]
struct CommandBarStorage {
    root: Option<std::path::PathBuf>,
}

impl CommandBarStorage {
    fn production() -> Self {
        Self {
            root: dirs::config_dir().map(|path| path.join("guishell")),
        }
    }

    #[cfg(test)]
    fn disabled() -> Self {
        Self { root: None }
    }

    #[cfg(test)]
    fn is_disabled(&self) -> bool {
        self.root.is_none()
    }

    fn load_json<T: for<'de> Deserialize<'de> + Default>(&self, name: &str) -> T {
        let Some(path) = self.root.as_ref().map(|root| root.join(name)) else {
            return T::default();
        };
        match std::fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => T::default(),
        }
    }

    fn save_json<T: Serialize>(&self, name: &str, value: &T) {
        let Some(path) = self.root.as_ref().map(|root| root.join(name)) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(contents) = serde_json::to_string_pretty(value) {
            let _ = std::fs::write(path, contents);
        }
    }
}

fn default_commands() -> Vec<QuickCommand> {
    vec![
        QuickCommand {
            label: "查看磁盘".into(),
            command: "df -h".into(),
            system: true,
        },
        QuickCommand {
            label: "查看内存".into(),
            command: "free -h".into(),
            system: true,
        },
        QuickCommand {
            label: "查看进程".into(),
            command: "top".into(),
            system: true,
        },
        QuickCommand {
            label: "网络连接".into(),
            command: "ss -tlnp".into(),
            system: true,
        },
        QuickCommand {
            label: "看文件".into(),
            command: "ls -la".into(),
            system: true,
        },
        QuickCommand {
            label: "加载docker".into(),
            command: "docker ps".into(),
            system: true,
        },
        QuickCommand {
            label: "nfs".into(),
            command: "showmount -e".into(),
            system: true,
        },
    ]
}

impl CommandBar {
    pub fn new() -> Self {
        let storage = CommandBarStorage::production();
        let mut commands: Vec<QuickCommand> = storage.load_json("native_quick_commands.json");
        if commands.is_empty() {
            commands = default_commands();
            storage.save_json("native_quick_commands.json", &commands);
        }
        let history: Vec<String> = storage.load_json("native_cmd_history.json");
        let favorites: Vec<QuickCommand> = storage.load_json("native_cmd_favorites.json");
        Self {
            storage,
            commands,
            input_text: String::new(),
            history,
            favorites,
            show_history: false,
            show_favorites: false,
            show_add: false,
            add_label: String::new(),
            add_command: String::new(),
            add_error: String::new(),
            edit_index: None,
            last_history_button_rect: None,
            last_favorites_button_rect: None,
        }
    }

    /// Whether the command bar currently owns terminal-directed keyboard input.
    pub fn has_blocking_dialog(&self) -> bool {
        self.show_add
    }

    fn push_history(&mut self, cmd: &str) {
        let cmd = cmd.trim_end_matches('\n').trim();
        if cmd.is_empty() {
            return;
        }
        self.history.retain(|h| h != cmd);
        self.history.insert(0, cmd.to_string());
        if self.history.len() > MAX_HISTORY {
            self.history.truncate(MAX_HISTORY);
        }
        self.save_history();
    }

    fn save_history(&self) {
        self.storage
            .save_json("native_cmd_history.json", &self.history);
    }

    fn save_commands(&self) {
        self.storage
            .save_json("native_quick_commands.json", &self.commands);
    }

    fn save_favorites(&self) {
        self.storage
            .save_json("native_cmd_favorites.json", &self.favorites);
    }
}

mod ui;

#[cfg(test)]
#[path = "command_bar/tests.rs"]
mod ui_tests;
