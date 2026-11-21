use crate::serial::SerialSpec;
use crate::tab_manager::{TabManager, TabType};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const SESSION_VERSION: u32 = 1;
const MAX_SESSION_BYTES: u64 = 1024 * 1024;
const MAX_RESTORED_TABS: usize = 64;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceSession {
    pub version: u32,
    pub active_tab: usize,
    pub tabs: Vec<PersistedTab>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PersistedTab {
    Local {
        shell_path: String,
        label: String,
    },
    Ssh {
        label: String,
        host: String,
        port: u16,
        user: String,
        auth: String,
        key_path: String,
    },
    Serial {
        device: String,
        display_name: String,
        serial_number: Option<String>,
        baud_rate: u32,
    },
}

impl WorkspaceSession {
    pub fn capture(manager: &TabManager) -> Self {
        let mut tabs = Vec::new();
        let mut active_tab = 0;
        for (index, tab) in manager.tabs.iter().enumerate() {
            let persisted = match &tab.tab_type {
                TabType::Local { shell_path } => Some(PersistedTab::Local {
                    shell_path: shell_path.clone(),
                    label: tab.label.clone(),
                }),
                TabType::Ssh { label, params } => Some(PersistedTab::Ssh {
                    label: label.clone(),
                    host: params.host.clone(),
                    port: params.port,
                    user: params.user.clone(),
                    auth: params.auth.clone(),
                    key_path: params.key_path.clone(),
                }),
                TabType::Serial { spec } => Some(PersistedTab::Serial {
                    device: spec.device.clone(),
                    display_name: spec.display_name.clone(),
                    serial_number: spec.serial_number.clone(),
                    baud_rate: spec.baud_rate,
                }),
                TabType::Process { .. }
                | TabType::Network { .. }
                | TabType::Recording { .. }
                | TabType::Settings => None,
            };
            if let Some(persisted) = persisted {
                if index == manager.active_idx {
                    active_tab = tabs.len();
                }
                tabs.push(persisted);
            }
        }
        Self {
            version: SESSION_VERSION,
            active_tab,
            tabs,
        }
    }

    pub fn config_path() -> PathBuf {
        crate::settings::Settings::config_dir().join("native-session.toml")
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
        if metadata.len() > MAX_SESSION_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Native 会话文件超过 1 MiB 限制",
            ));
        }
        let content = fs::read_to_string(path)?;
        let mut session: Self = toml::from_str(&content)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if session.version != SESSION_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("不支持的 Native 会话版本：{}", session.version),
            ));
        }
        session.tabs.truncate(MAX_RESTORED_TABS);
        if session.tabs.is_empty() {
            session.active_tab = 0;
        } else {
            session.active_tab = session.active_tab.min(session.tabs.len() - 1);
        }
        Ok(session)
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to(&Self::config_path())
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).map_err(io::Error::other)?;
        let temp = path.with_extension(format!("tmp-{}", std::process::id()));
        fs::write(&temp, content.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
        }
        if let Err(error) = fs::rename(&temp, path) {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
        Ok(())
    }
}

impl PersistedTab {
    pub fn serial_spec(&self) -> Option<SerialSpec> {
        let Self::Serial {
            device,
            display_name,
            serial_number,
            baud_rate,
        } = self
        else {
            return None;
        };
        Some(SerialSpec {
            device: device.clone(),
            display_name: display_name.clone(),
            serial_number: serial_number.clone(),
            baud_rate: *baud_rate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_round_trip_never_contains_ssh_password() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.toml");
        let session = WorkspaceSession {
            version: SESSION_VERSION,
            active_tab: 1,
            tabs: vec![
                PersistedTab::Local {
                    shell_path: "/bin/bash".into(),
                    label: "构建".into(),
                },
                PersistedTab::Ssh {
                    label: "生产机".into(),
                    host: "server.example".into(),
                    port: 22,
                    user: "root".into(),
                    auth: "password".into(),
                    key_path: String::new(),
                },
            ],
        };

        session.save_to(&path).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("password ="));
        assert_eq!(WorkspaceSession::load_from(&path).unwrap(), session);
    }

    #[test]
    fn missing_session_is_an_empty_first_run() {
        let directory = tempfile::tempdir().unwrap();
        let session = WorkspaceSession::load_from(&directory.path().join("missing.toml")).unwrap();

        assert!(session.tabs.is_empty());
        assert_eq!(session.active_tab, 0);
    }
}
