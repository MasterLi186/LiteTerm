use serde::{Deserialize, Deserializer, Serialize};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::shortcuts::ShortcutSettings;

#[cfg(target_os = "windows")]
pub const DEFAULT_TERMINAL_FONT_FAMILY: &str = "Consolas";
#[cfg(not(target_os = "windows"))]
pub const DEFAULT_TERMINAL_FONT_FAMILY: &str = "Ubuntu Mono";
pub const DEFAULT_TERMINAL_FONT_SIZE: f32 = 22.0;
pub const DEFAULT_TERMINAL_COLOR_SCHEME: &str = "AdventureTime";
pub const MIN_SIDEBAR_WIDTH: u32 = 150;
pub const MAX_SIDEBAR_WIDTH: u32 = 500;
pub const DEFAULT_SIDEBAR_WIDTH: u32 = 220;
pub const MIN_ZMODEM_TIMEOUT_SECS: u32 = 5;
pub const MAX_ZMODEM_TIMEOUT_SECS: u32 = 3600;
static SETTINGS_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Settings {
    pub terminal: TerminalSettings,
    pub appearance: AppearanceSettings,
    pub transfer: TransferSettings,
    pub ssh: SshSettings,
    pub zmodem: ZmodemSettings,
    pub shortcuts: ShortcutSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalSettings {
    #[serde(deserialize_with = "deserialize_font")]
    pub font: String,
    pub font_size: f32,
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

// --- Font normalization ---

fn normalize_font(raw: &str) -> String {
    let trimmed = raw.trim();
    let mut tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if let Some(last) = tokens.last() {
        if let Ok(n) = last.parse::<f32>() {
            if n.is_finite() {
                tokens.pop();
            }
        }
    }
    let joined = tokens.join(" ");
    if joined.is_empty() {
        DEFAULT_TERMINAL_FONT_FAMILY.to_string()
    } else if cfg!(target_os = "windows") && joined.eq_ignore_ascii_case("Ubuntu Mono") {
        // Older Native builds used Ubuntu Mono on every platform. Windows does
        // not ship that family, so migrate the legacy default to the same
        // Consolas face used by the classic Command Prompt.
        DEFAULT_TERMINAL_FONT_FAMILY.to_string()
    } else {
        joined
    }
}

fn deserialize_font<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    Ok(normalize_font(&raw))
}

// --- Default implementations ---

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font: DEFAULT_TERMINAL_FONT_FAMILY.to_string(),
            font_size: DEFAULT_TERMINAL_FONT_SIZE,
            scrollback_lines: 10000,
            color_scheme: DEFAULT_TERMINAL_COLOR_SCHEME.to_string(),
            cursor_blink: true,
        }
    }
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: "Adwaita".to_string(),
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            file_browser_height: 200,
            show_sidebar: true,
            show_file_browser: true,
        }
    }
}

impl Default for TransferSettings {
    fn default() -> Self {
        Self {
            default_download_dir: default_download_directory().to_string_lossy().into_owned(),
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
            download_dir: default_download_directory().to_string_lossy().into_owned(),
            timeout_secs: 60,
        }
    }
}

/// Resolve a usable cross-platform download location without assuming that
/// `~/Downloads` exists. `dirs::download_dir` respects the OS/user profile;
/// home, temp and current directory are progressively safer fallbacks.
pub fn default_download_directory() -> PathBuf {
    [
        dirs::download_dir(),
        dirs::home_dir(),
        Some(std::env::temp_dir()),
        std::env::current_dir().ok(),
    ]
    .into_iter()
    .flatten()
    .find(|path| path.is_absolute() && path.is_dir())
    .unwrap_or_else(std::env::temp_dir)
}

pub fn resolve_existing_download_directory(raw: &str) -> Result<PathBuf, String> {
    let path = resolve_zmodem_download_dir(raw)?;
    if path.is_dir() {
        Ok(path)
    } else {
        Err(format!("下载目录不存在或不是目录：{}", path.display()))
    }
}

/// Choose the initial local directory for the bottom file manager. A stale or
/// cross-platform path from settings must never leave the browser on a missing
/// directory after moving the config between Linux, Windows, and macOS.
pub fn file_browser_local_directory(configured: &str) -> PathBuf {
    resolve_existing_download_directory(configured).unwrap_or_else(|_| default_download_directory())
}

/// Expand the supported home-directory shorthand and require an absolute path.
///
/// Directory existence is intentionally not checked here: the transfer runtime
/// must validate it again immediately before creating a receiver.
pub fn resolve_zmodem_download_dir(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("ZMODEM 下载目录不能为空".into());
    }
    if trimmed.chars().any(char::is_control) {
        return Err("ZMODEM 下载目录包含控制字符".into());
    }

    let path = if trimmed == "~" {
        dirs::home_dir().ok_or_else(|| "无法解析 ZMODEM 下载目录中的 ~".to_string())?
    } else if let Some(relative) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        dirs::home_dir()
            .ok_or_else(|| "无法解析 ZMODEM 下载目录中的 ~".to_string())?
            .join(relative)
    } else {
        PathBuf::from(trimmed)
    };

    if !path.is_absolute() {
        return Err("ZMODEM 下载目录展开后必须是绝对路径".into());
    }
    Ok(path)
}

pub fn validate_zmodem_timeout(timeout_secs: u32) -> Result<(), String> {
    if !(MIN_ZMODEM_TIMEOUT_SECS..=MAX_ZMODEM_TIMEOUT_SECS).contains(&timeout_secs) {
        return Err(format!(
            "ZMODEM 超时必须在 {MIN_ZMODEM_TIMEOUT_SECS} 到 {MAX_ZMODEM_TIMEOUT_SECS} 秒之间"
        ));
    }
    Ok(())
}

// --- Persistence ---

fn resolve_config_base(
    platform_config: Option<PathBuf>,
    home_dir: Option<PathBuf>,
    current_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    platform_config
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home_dir
                .map(|home| home.join(".config"))
                .filter(|path| path.is_absolute())
        })
        .or_else(|| {
            current_dir
                .map(|current| current.join(".config"))
                .filter(|path| path.is_absolute())
        })
}

fn atomic_write_with<F>(path: &Path, content: &[u8], before_rename: F) -> io::Result<()>
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "设置路径缺少文件名"))?;

    let mut temp_path = None;
    let result = (|| {
        let mut file = loop {
            let sequence = SETTINGS_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let mut temp_name = OsString::from(".");
            temp_name.push(file_name);
            temp_name.push(format!(".{}.{}.tmp", std::process::id(), sequence));
            let candidate = parent.join(temp_name);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&candidate) {
                Ok(file) => {
                    temp_path = Some(candidate);
                    break file;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        };
        file.write_all(content)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        let temp = temp_path.as_deref().expect("已创建的设置临时文件缺失");
        before_rename(temp)?;
        fs::rename(temp, path)
    })();

    if result.is_err() {
        if let Some(temp) = temp_path {
            let _ = fs::remove_file(temp);
        }
    }
    result
}

impl Settings {
    /// Returns the platform-specific config directory for guishell.
    /// Typically `~/.config/guishell/` on Linux.
    pub fn config_dir() -> PathBuf {
        let base = resolve_config_base(
            dirs::config_dir(),
            dirs::home_dir(),
            std::env::current_dir().ok(),
        )
        .unwrap_or_else(|| {
            let temp = std::env::temp_dir().join(".config");
            if temp.is_absolute() {
                temp
            } else {
                PathBuf::from("/tmp/.config")
            }
        });
        base.join("guishell")
    }

    /// Returns the Native-specific settings path.
    pub fn config_path() -> PathBuf {
        Self::config_dir().join("native-settings.toml")
    }

    /// Load settings from the default config location.
    /// Returns defaults if the file does not exist.
    pub fn load() -> io::Result<Self> {
        let path = Self::config_path();
        Self::load_from(&path)
    }

    /// Load settings from a specific path.
    /// Returns defaults if the file does not exist.
    pub fn load_from(path: &Path) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(content) => {
                let settings: Settings = toml::from_str(&content)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                Ok(settings)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(e),
        }
    }

    /// Save settings to a specific path, creating parent directories if needed.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        let content = toml::to_string_pretty(self).map_err(io::Error::other)?;
        atomic_write_with(path, content.as_bytes(), |_| Ok(()))
    }

    /// Save settings to the default config location.
    pub fn save(&self) -> io::Result<()> {
        let path = Self::config_path();
        self.save_to(&path)
    }

    /// 校正从磁盘加载后的设置，返回用户可见中文警告列表。
    ///
    /// - 字号非有限 → 默认 22；有限但小于 8 → 8；大于 48 → 48
    /// - 快捷键 `validate()` 失败 → 整组回落 `ShortcutSettings::default`
    /// - ZMODEM 超时或任一下载目录无效 → 回落到真实存在的系统目录
    pub fn sanitize_loaded(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();

        self.appearance.sidebar_width = self
            .appearance
            .sidebar_width
            .clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);

        let size = self.terminal.font_size;
        if !size.is_finite() {
            self.terminal.font_size = DEFAULT_TERMINAL_FONT_SIZE;
            warnings.push(format!(
                "字号无效（非有限数值），已恢复默认 {DEFAULT_TERMINAL_FONT_SIZE}"
            ));
        } else if size < 8.0 {
            self.terminal.font_size = 8.0;
            warnings.push(format!("字号 {size} 过小，已调整为 8"));
        } else if size > 48.0 {
            self.terminal.font_size = 48.0;
            warnings.push(format!("字号 {size} 过大，已调整为 48"));
        }

        if let Err(err) = self.shortcuts.validate() {
            self.shortcuts = ShortcutSettings::default();
            warnings.push(format!("快捷键配置无效，已恢复默认：{err}"));
        }

        if let Err(err) = validate_zmodem_timeout(self.zmodem.timeout_secs) {
            self.zmodem.timeout_secs = ZmodemSettings::default().timeout_secs;
            warnings.push(format!("{err}，已恢复默认 {} 秒", self.zmodem.timeout_secs));
        }

        let trimmed_transfer_dir = self.transfer.default_download_dir.trim();
        if resolve_existing_download_directory(trimmed_transfer_dir).is_err() {
            self.transfer.default_download_dir =
                default_download_directory().to_string_lossy().into_owned();
            warnings.push(format!(
                "默认下载目录不存在，已回退为 {}",
                self.transfer.default_download_dir
            ));
        } else if self.transfer.default_download_dir != trimmed_transfer_dir {
            self.transfer.default_download_dir = trimmed_transfer_dir.to_string();
        }

        let trimmed_download_dir = self.zmodem.download_dir.trim();
        if resolve_existing_download_directory(trimmed_download_dir).is_err() {
            self.zmodem.download_dir = default_download_directory().to_string_lossy().into_owned();
            warnings.push(format!(
                "ZMODEM 下载目录不存在或无效，已回退为 {}",
                self.zmodem.download_dir
            ));
        } else if self.zmodem.download_dir != trimmed_download_dir {
            self.zmodem.download_dir = trimmed_download_dir.to_string();
        }

        warnings
    }
}

/// 将多条设置加载警告合并为用户可见中文文本；空输入返回空串。
pub fn format_settings_load_warnings(warnings: &[String]) -> String {
    if warnings.is_empty() {
        String::new()
    } else {
        warnings.join("；")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Settings, DEFAULT_TERMINAL_COLOR_SCHEME, DEFAULT_TERMINAL_FONT_FAMILY,
        DEFAULT_TERMINAL_FONT_SIZE,
    };
    use std::path::PathBuf;

    #[test]
    fn old_settings_without_shortcuts_receive_defaults() {
        let settings: Settings = toml::from_str("[terminal]\nfont = 'Ubuntu Mono'\n").unwrap();
        assert_eq!(settings.shortcuts.search, "Ctrl+F");
    }

    #[test]
    fn settings_roundtrip_preserves_shortcuts() {
        let mut settings = Settings::default();
        settings.shortcuts.search = "Ctrl+F".into();
        let encoded = toml::to_string_pretty(&settings).unwrap();
        let decoded: Settings = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded.shortcuts.search, "Ctrl+F");
    }

    #[test]
    fn settings_roundtrip_preserves_zmodem_values() {
        let mut settings = Settings::default();
        settings.zmodem.enabled = false;
        settings.zmodem.auto_detect = false;
        settings.zmodem.download_dir = "/var/tmp/zmodem-downloads".into();
        settings.zmodem.timeout_secs = 300;

        let encoded = toml::to_string_pretty(&settings).unwrap();
        let decoded: Settings = toml::from_str(&encoded).unwrap();

        assert!(!decoded.zmodem.enabled);
        assert!(!decoded.zmodem.auto_detect);
        assert_eq!(decoded.zmodem.download_dir, "/var/tmp/zmodem-downloads");
        assert_eq!(decoded.zmodem.timeout_secs, 300);
    }

    #[test]
    fn settings_file_roundtrip_preserves_zmodem_values() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("native-settings.toml");
        let mut settings = Settings::default();
        settings.zmodem.enabled = false;
        settings.zmodem.auto_detect = false;
        settings.zmodem.download_dir = "~/ZMODEM Downloads".into();
        settings.zmodem.timeout_secs = 3600;

        settings.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path).unwrap();

        assert!(!loaded.zmodem.enabled);
        assert!(!loaded.zmodem.auto_detect);
        assert_eq!(loaded.zmodem.download_dir, "~/ZMODEM Downloads");
        assert_eq!(loaded.zmodem.timeout_secs, 3600);
    }

    #[test]
    fn atomic_settings_save_replaces_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("native-settings.toml");
        std::fs::write(&path, "old settings").unwrap();

        let mut settings = Settings::default();
        settings.zmodem.auto_detect = false;
        settings.save_to(&path).unwrap();

        let loaded = Settings::load_from(&path).unwrap();
        assert!(!loaded.zmodem.auto_detect);
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("old settings"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn atomic_settings_save_failure_preserves_original_and_cleans_temp() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("native-settings.toml");
        std::fs::write(&path, "original settings").unwrap();

        let error = super::atomic_write_with(&path, b"replacement", |_| {
            Err(std::io::Error::other("injected pre-rename failure"))
        })
        .unwrap_err();

        assert!(error.to_string().contains("injected"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original settings");
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            1,
            "failed save must remove its temporary file"
        );
    }

    #[test]
    fn zmodem_download_dir_accepts_absolute_and_home_relative_paths() {
        assert_eq!(
            super::resolve_zmodem_download_dir("/var/tmp/downloads").unwrap(),
            PathBuf::from("/var/tmp/downloads")
        );
        let home = dirs::home_dir().expect("测试环境应提供 home 目录");
        assert_eq!(
            super::resolve_zmodem_download_dir("~/Downloads").unwrap(),
            home.join("Downloads")
        );
        assert_eq!(super::resolve_zmodem_download_dir("~").unwrap(), home);
        assert_eq!(super::resolve_zmodem_download_dir("~\\").unwrap(), home);
        assert_eq!(
            super::resolve_zmodem_download_dir("  /var/tmp/downloads  ").unwrap(),
            PathBuf::from("/var/tmp/downloads")
        );
        assert_eq!(
            super::resolve_zmodem_download_dir("  ~/Downloads  ").unwrap(),
            home.join("Downloads")
        );
        assert!(super::resolve_zmodem_download_dir("relative/downloads").is_err());
        assert!(super::resolve_zmodem_download_dir("~other/downloads").is_err());
        assert!(super::resolve_zmodem_download_dir("  ").is_err());
        assert!(super::resolve_zmodem_download_dir("/tmp/has\ncontrol").is_err());
    }

    #[test]
    fn default_download_directory_is_existing_and_absolute() {
        let directory = super::default_download_directory();
        assert!(directory.is_absolute(), "默认下载目录必须是绝对路径");
        assert!(directory.is_dir(), "默认下载目录必须真实存在");
        assert!(super::resolve_existing_download_directory(&directory.to_string_lossy()).is_ok());
    }

    #[test]
    fn file_browser_directory_falls_back_when_configured_path_is_missing() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing-downloads");

        let resolved = super::file_browser_local_directory(&missing.to_string_lossy());

        assert!(resolved.is_absolute());
        assert!(resolved.is_dir());
    }

    #[test]
    fn zmodem_timeout_validation_includes_boundaries() {
        assert!(super::validate_zmodem_timeout(super::MIN_ZMODEM_TIMEOUT_SECS).is_ok());
        assert!(super::validate_zmodem_timeout(super::MAX_ZMODEM_TIMEOUT_SECS).is_ok());
        assert!(super::validate_zmodem_timeout(super::MIN_ZMODEM_TIMEOUT_SECS - 1).is_err());
        assert!(super::validate_zmodem_timeout(super::MAX_ZMODEM_TIMEOUT_SECS + 1).is_err());
    }

    /// Fresh Native settings use the Native product defaults.
    #[test]
    fn default_terminal_uses_platform_mono_22_and_adventure_time() {
        let settings = Settings::default();
        #[cfg(target_os = "windows")]
        assert_eq!(DEFAULT_TERMINAL_FONT_FAMILY, "Consolas");
        #[cfg(not(target_os = "windows"))]
        assert_eq!(DEFAULT_TERMINAL_FONT_FAMILY, "Ubuntu Mono");
        assert_eq!(DEFAULT_TERMINAL_FONT_SIZE, 22.0);
        assert_eq!(DEFAULT_TERMINAL_COLOR_SCHEME, "AdventureTime");
        assert_eq!(settings.terminal.font, DEFAULT_TERMINAL_FONT_FAMILY);
        assert_eq!(settings.terminal.font_size, DEFAULT_TERMINAL_FONT_SIZE);
        assert_eq!(
            settings.terminal.color_scheme,
            DEFAULT_TERMINAL_COLOR_SCHEME
        );
    }

    #[test]
    fn legacy_ubuntu_mono_is_migrated_only_on_windows() {
        #[cfg(target_os = "windows")]
        assert_eq!(super::normalize_font("Ubuntu Mono"), "Consolas");
        #[cfg(not(target_os = "windows"))]
        assert_eq!(super::normalize_font("Ubuntu Mono"), "Ubuntu Mono");
    }

    /// P0 Task 3 RED: 旧式 "family size" 字符串应规范化为纯 family，
    /// 且缺失显式 font_size 时使用 Native 默认 22.0，而非内嵌旧字号。
    #[test]
    fn deserializes_legacy_font_family_size_string_without_inheriting_embedded_size() {
        let settings: Settings = toml::from_str("[terminal]\nfont = 'Monospace 12'\n").unwrap();
        assert_eq!(settings.terminal.font, "Monospace");
        assert_eq!(settings.terminal.font_size, DEFAULT_TERMINAL_FONT_SIZE);
    }

    #[test]
    fn native_config_path_uses_isolated_native_settings_filename() {
        let path = Settings::config_path();
        assert!(
            path.is_absolute(),
            "Native config path must be absolute: {path:?}"
        );
        assert_eq!(path.parent(), Some(Settings::config_dir().as_path()));
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("native-settings.toml")
        );
        assert_ne!(path, Settings::config_dir().join("settings.toml"));
    }

    #[test]
    fn config_base_uses_absolute_platform_home_then_current_dir_fallbacks() {
        let platform = PathBuf::from("/xdg-config");
        let home = PathBuf::from("/home/native-user");
        let current = PathBuf::from("/workspace");

        assert_eq!(
            super::resolve_config_base(
                Some(platform.clone()),
                Some(home.clone()),
                Some(current.clone())
            ),
            Some(platform)
        );
        assert_eq!(
            super::resolve_config_base(None, Some(home.clone()), Some(current.clone())),
            Some(home.join(".config"))
        );
        assert_eq!(
            super::resolve_config_base(None, None, Some(current.clone())),
            Some(current.join(".config"))
        );
        assert_eq!(
            super::resolve_config_base(
                Some(PathBuf::from("relative-xdg")),
                Some(home.clone()),
                Some(current)
            ),
            Some(home.join(".config")),
            "relative candidates must not escape into a relative Native config path"
        );
    }

    /// P0 Task 3 RED: 显式 font / font_size / color_scheme 应完整保留。
    #[test]
    fn deserializes_explicit_font_size_and_color_scheme() {
        let toml = r#"[terminal]
font = 'Noto Sans Mono'
font_size = 18.5
color_scheme = '3024 Day'
"#;
        let settings: Settings = toml::from_str(toml).unwrap();
        assert_eq!(settings.terminal.font, "Noto Sans Mono");
        assert_eq!(settings.terminal.font_size, 18.5);
        assert_eq!(settings.terminal.color_scheme, "3024 Day");
    }

    /// P0 Task 3 RED: 空白或仅数字的旧 font 值应安全回落到 Ubuntu Mono，不 panic。
    #[test]
    fn blank_or_numeric_legacy_font_falls_back_to_ubuntu_mono() {
        let cases = [" 12 ", "12", "   ", ""];
        for font in cases {
            let toml = format!("[terminal]\nfont = '{font}'\n");
            let settings: Settings = toml::from_str(&toml).expect("deserialize must not panic");
            assert_eq!(
                settings.terminal.font, DEFAULT_TERMINAL_FONT_FAMILY,
                "font={font:?} should fall back to Ubuntu Mono"
            );
        }
    }

    // --- P0 Task 3 审查修复 RED: sanitize_loaded + 警告合并 ---

    /// font_size=0 与小于 8 应 clamp 为 8，并产生含「字号」的中文警告。
    #[test]
    fn sanitize_loaded_clamps_font_size_below_min_to_8() {
        let mut settings = Settings::default();
        settings.terminal.font_size = 0.0;
        let warnings = settings.sanitize_loaded();
        assert_eq!(settings.terminal.font_size, 8.0);
        assert!(
            warnings.iter().any(|w| w.contains("字号")),
            "零字号警告应指出字号: {warnings:?}"
        );

        settings.terminal.font_size = 7.9;
        let warnings = settings.sanitize_loaded();
        assert_eq!(settings.terminal.font_size, 8.0);
        assert!(
            warnings.iter().any(|w| w.contains("字号")),
            "小于 8 的字号警告应指出字号: {warnings:?}"
        );
    }

    /// font_size 大于 48 应 clamp 为 48，并产生含「字号」的中文警告。
    #[test]
    fn sanitize_loaded_clamps_font_size_above_max_to_48() {
        let mut settings = Settings::default();
        settings.terminal.font_size = 48.1;
        let warnings = settings.sanitize_loaded();
        assert_eq!(settings.terminal.font_size, 48.0);
        assert!(
            warnings.iter().any(|w| w.contains("字号")),
            "超大字号警告应指出字号: {warnings:?}"
        );

        settings.terminal.font_size = 100.0;
        let warnings = settings.sanitize_loaded();
        assert_eq!(settings.terminal.font_size, 48.0);
        assert!(warnings.iter().any(|w| w.contains("字号")));
    }

    /// NaN / ±∞ 字号应回落 Native 默认 22.0，并产生含「字号」的中文警告。
    #[test]
    fn sanitize_loaded_resets_non_finite_font_size_to_default_22() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut settings = Settings::default();
            settings.terminal.font_size = bad;
            let warnings = settings.sanitize_loaded();
            assert_eq!(
                settings.terminal.font_size, DEFAULT_TERMINAL_FONT_SIZE,
                "non-finite font_size={bad:?} 应回落 22.0"
            );
            assert!(
                warnings.iter().any(|w| w.contains("字号")),
                "non-finite 字号警告应指出字号: {warnings:?}"
            );
        }
    }

    /// 边界 8 / 48 与区间内合法字号不得被改写，也不应单独产生字号警告。
    #[test]
    fn sanitize_loaded_keeps_valid_font_size_unchanged() {
        for size in [8.0_f32, 18.5, 26.0, 48.0] {
            let mut settings = Settings::default();
            settings.terminal.font_size = size;
            let warnings = settings.sanitize_loaded();
            assert_eq!(settings.terminal.font_size, size);
            assert!(
                !warnings.iter().any(|w| w.contains("字号")),
                "合法字号 {size} 不应产生字号警告: {warnings:?}"
            );
        }
    }

    /// ShortcutSettings::validate 失败时整组快捷键回落默认，并产生含「快捷键」的中文警告。
    #[test]
    fn sanitize_loaded_resets_invalid_shortcuts_group_to_defaults() {
        use crate::shortcuts::ShortcutSettings;

        // 无法解析的 chord
        let mut settings = Settings::default();
        settings.shortcuts.search = "not-a-chord".into();
        let warnings = settings.sanitize_loaded();
        assert_eq!(settings.shortcuts, ShortcutSettings::default());
        assert!(
            warnings.iter().any(|w| w.contains("快捷键")),
            "非法 chord 警告应指出快捷键: {warnings:?}"
        );

        // 冲突 chord：copy 与 paste 相同
        let mut settings = Settings::default();
        settings.shortcuts.copy = settings.shortcuts.paste.clone();
        let warnings = settings.sanitize_loaded();
        assert_eq!(settings.shortcuts, ShortcutSettings::default());
        assert!(
            warnings.iter().any(|w| w.contains("快捷键")),
            "冲突快捷键警告应指出快捷键: {warnings:?}"
        );
    }

    /// 合法字号 + 合法快捷键：不改写字段，警告列表为空。
    #[test]
    fn sanitize_loaded_leaves_fully_valid_settings_unchanged() {
        let mut settings = Settings::default();
        settings.terminal.font_size = 18.0;
        settings.shortcuts.search = "Ctrl+F".into();
        let before = settings.clone();
        let warnings = settings.sanitize_loaded();
        assert_eq!(settings.terminal.font_size, before.terminal.font_size);
        assert_eq!(settings.shortcuts, before.shortcuts);
        assert_eq!(settings.terminal.font, before.terminal.font);
        assert_eq!(settings.terminal.color_scheme, before.terminal.color_scheme);
        assert!(warnings.is_empty(), "合法配置不应产生警告: {warnings:?}");
    }

    /// 字号与快捷键同时非法时应各自修正，并合并多条警告。
    #[test]
    fn sanitize_loaded_collects_font_and_shortcut_warnings_together() {
        use crate::shortcuts::ShortcutSettings;

        let mut settings = Settings::default();
        settings.terminal.font_size = 0.0;
        settings.shortcuts.search = "".into();
        let warnings = settings.sanitize_loaded();
        assert_eq!(settings.terminal.font_size, 8.0);
        assert_eq!(settings.shortcuts, ShortcutSettings::default());
        assert!(
            warnings.iter().any(|w| w.contains("字号")),
            "应含字号警告: {warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("快捷键")),
            "应含快捷键警告: {warnings:?}"
        );
        assert!(warnings.len() >= 2, "应至少两条独立警告: {warnings:?}");
    }

    #[test]
    fn sanitize_loaded_repairs_invalid_zmodem_values_and_warns() {
        let mut settings = Settings::default();
        settings.zmodem.timeout_secs = 0;
        settings.zmodem.download_dir = "relative/downloads".into();

        let warnings = settings.sanitize_loaded();

        assert_eq!(
            settings.zmodem.timeout_secs,
            super::ZmodemSettings::default().timeout_secs
        );
        assert!(
            super::resolve_existing_download_directory(&settings.zmodem.download_dir).is_ok(),
            "回退下载目录必须真实存在且可解析为绝对路径"
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("ZMODEM 超时")),
            "应包含 ZMODEM 超时警告: {warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("ZMODEM 下载目录")),
            "应包含 ZMODEM 下载目录警告: {warnings:?}"
        );
    }

    #[test]
    fn sanitize_loaded_repairs_missing_transfer_and_zmodem_directories() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing-downloads");
        let mut settings = Settings::default();
        settings.transfer.default_download_dir = missing.to_string_lossy().into_owned();
        settings.zmodem.download_dir = missing.to_string_lossy().into_owned();

        let warnings = settings.sanitize_loaded();

        assert!(super::resolve_existing_download_directory(
            &settings.transfer.default_download_dir
        )
        .is_ok());
        assert!(super::resolve_existing_download_directory(&settings.zmodem.download_dir).is_ok());
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("默认下载目录")));
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("ZMODEM 下载目录")));
    }

    /// 纯 helper：多条设置加载警告合并为用户可见中文文本（不依赖真实窗口）。
    #[test]
    fn format_settings_load_warnings_merges_into_user_visible_chinese() {
        assert_eq!(super::format_settings_load_warnings(&[]), "");

        let joined = super::format_settings_load_warnings(&[
            "字号已调整为有效范围".to_string(),
            "快捷键配置无效，已恢复默认".to_string(),
        ]);
        assert!(
            joined.contains("字号"),
            "合并文本应保留字号信息: {joined:?}"
        );
        assert!(
            joined.contains("快捷键"),
            "合并文本应保留快捷键信息: {joined:?}"
        );
        assert!(
            joined.contains("字号已调整为有效范围"),
            "合并文本应保留完整警告原文: {joined:?}"
        );
        assert!(
            joined.contains("快捷键配置无效，已恢复默认"),
            "合并文本应保留完整警告原文: {joined:?}"
        );
        // 两条警告均出现在同一用户可见字符串中
        let first = joined.find("字号").expect("字号");
        let second = joined.find("快捷键").expect("快捷键");
        assert!(first < second, "警告应按输入顺序合并: {joined:?}");
    }
}
