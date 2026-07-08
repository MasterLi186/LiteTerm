use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Settings {
    pub terminal: TerminalSettings,
    pub appearance: AppearanceSettings,
    pub transfer: TransferSettings,
    pub ssh: SshSettings,
    pub zmodem: ZmodemSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalSettings {
    pub font: String,
    pub scrollback_lines: u32,
    pub color_scheme: String,
    pub cursor_blink: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    pub theme: String,
    pub sidebar_width: u32,
    pub file_browser_height: u32,
    pub show_sidebar: bool,
    pub show_file_browser: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TransferSettings {
    pub default_download_dir: String,
    pub resume_threshold_mb: u32,
    pub max_retries: u32,
    pub concurrent_transfers: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SshSettings {
    pub keepalive_interval_secs: u32,
    pub connect_timeout_secs: u32,
    pub default_charset: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ZmodemSettings {
    pub enabled: bool,
    pub auto_detect: bool,
    pub download_dir: String,
    pub timeout_secs: u32,
}

// --- Default implementations ---


impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font: "Monospace 12".to_string(),
            scrollback_lines: 10000,
            color_scheme: "Tango".to_string(),
            cursor_blink: true,
        }
    }
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: "Adwaita".to_string(),
            sidebar_width: 220,
            file_browser_height: 200,
            show_sidebar: true,
            show_file_browser: true,
        }
    }
}

impl Default for TransferSettings {
    fn default() -> Self {
        Self {
            default_download_dir: "~/Downloads".to_string(),
            resume_threshold_mb: 10,
            max_retries: 3,
            concurrent_transfers: 2,
        }
    }
}

impl Default for SshSettings {
    fn default() -> Self {
        Self {
            keepalive_interval_secs: 30,
            connect_timeout_secs: 10,
            default_charset: "UTF-8".to_string(),
        }
    }
}

impl Default for ZmodemSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_detect: true,
            download_dir: "~/Downloads".to_string(),
            timeout_secs: 60,
        }
    }
}

// --- Persistence ---

impl Settings {
    /// Returns the platform-specific config directory for guishell.
    /// Typically `~/.config/guishell/` on Linux.
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("guishell")
    }

    /// Load settings from the default config location.
    /// Returns defaults if the file does not exist.
    pub fn load() -> io::Result<Self> {
        let path = Self::config_dir().join("settings.toml");
        Self::load_from(&path)
    }

    /// Load settings from a specific path.
    /// Returns defaults if the file does not exist.
    pub fn load_from(path: &Path) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(content) => {
                let settings: Settings = toml::from_str(&content).map_err(|e| {
                    io::Error::new(io::ErrorKind::InvalidData, e)
                })?;
                Ok(settings)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(e),
        }
    }

    /// Save settings to a specific path, creating parent directories if needed.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).map_err(|e| {
            io::Error::other(e)
        })?;
        fs::write(path, content)
    }

    /// Save settings to the default config location.
    pub fn save(&self) -> io::Result<()> {
        let path = Self::config_dir().join("settings.toml");
        self.save_to(&path)
    }
}
