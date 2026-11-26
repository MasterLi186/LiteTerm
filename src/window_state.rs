use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const WINDOW_STATE_VERSION: u32 = 1;
/// 没有历史记录时的默认窗口尺寸（逻辑像素），与窗口创建时的旧默认值一致。
pub const DEFAULT_WIDTH: f32 = 1280.0;
pub const DEFAULT_HEIGHT: f32 = 800.0;
/// 与窗口最小尺寸保持一致，避免恢复出无法操作的窗口。
pub const MIN_WIDTH: f32 = 640.0;
pub const MIN_HEIGHT: f32 = 400.0;
const MAX_DIMENSION: f32 = 16384.0;
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// 上次退出时记录的窗口状态（逻辑像素，DPI 无关）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub version: u32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            version: WINDOW_STATE_VERSION,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            maximized: false,
        }
    }
}

impl WindowState {
    pub fn config_path() -> PathBuf {
        crate::settings::Settings::config_dir().join("native-window.toml")
    }

    pub fn load() -> io::Result<Self> {
        Self::load_from(&Self::config_path())
    }

    pub fn load_from(path: &Path) -> io::Result<Self> {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        if metadata.len() > MAX_FILE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "窗口状态文件超过 64 KiB 限制",
            ));
        }
        let content = fs::read_to_string(path)?;
        let mut state: Self = toml::from_str(&content)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if state.version != WINDOW_STATE_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("不支持的窗口状态版本 {}", state.version),
            ));
        }
        state.sanitize();
        Ok(state)
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to(&Self::config_path())
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        let mut state = *self;
        state.sanitize();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(&state)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::write(path, content)
    }

    /// 非法尺寸回落到默认值，并夹在窗口最小/最大允许范围内。
    pub fn sanitize(&mut self) {
        if !self.width.is_finite() || self.width <= 0.0 {
            self.width = DEFAULT_WIDTH;
        }
        if !self.height.is_finite() || self.height <= 0.0 {
            self.height = DEFAULT_HEIGHT;
        }
        self.width = self.width.clamp(MIN_WIDTH, MAX_DIMENSION);
        self.height = self.height.clamp(MIN_HEIGHT, MAX_DIMENSION);
    }
}
