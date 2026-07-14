use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    Keyring,
    Key,
    Agent,
    Password,
}

impl std::fmt::Display for AuthMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthMethod::Keyring => write!(f, "keyring"),
            AuthMethod::Key => write!(f, "key"),
            AuthMethod::Agent => write!(f, "agent"),
            AuthMethod::Password => write!(f, "password"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostConfig {
    pub label: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthMethod,
    #[serde(default)]
    pub key_path: String,
    #[serde(default)]
    pub charset: String,
    #[serde(default)]
    pub proxy_jump: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupConfig {
    pub label: String,
    pub color: String,
    #[serde(default)]
    pub hosts: BTreeMap<String, HostConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectionStore {
    #[serde(default)]
    pub groups: BTreeMap<String, GroupConfig>,
}

impl ConnectionStore {
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("guishell")
            .join("connections.toml")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                toml::from_str(&content).unwrap_or_else(|e| {
                    log::error!("Failed to parse connections.toml: {}", e);
                    ConnectionStore::default()
                })
            }
            Err(_) => ConnectionStore::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| format!("序列化失败: {}", e))?;
        std::fs::write(&path, content).map_err(|e| format!("写入失败: {}", e))
    }

    pub fn add_group(&mut self, id: &str, label: &str, color: &str) {
        let entry = self.groups.entry(id.to_string()).or_insert_with(|| GroupConfig {
            label: label.to_string(),
            color: color.to_string(),
            hosts: BTreeMap::new(),
        });
        entry.label = label.to_string();
        entry.color = color.to_string();
    }

    pub fn add_host(&mut self, group_id: &str, host_id: &str, host: HostConfig) {
        if let Some(group) = self.groups.get_mut(group_id) {
            group.hosts.insert(host_id.to_string(), host);
        }
    }

    pub fn remove_host(&mut self, group_id: &str, host_id: &str) {
        if let Some(group) = self.groups.get_mut(group_id) {
            group.hosts.remove(host_id);
        }
    }

    pub fn group_ids(&self) -> Vec<String> {
        self.groups.keys().cloned().collect()
    }
}
