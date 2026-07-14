use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
pub struct GroupConfig {
    pub label: String,
    pub color: String,
    #[serde(default)]
    pub hosts: BTreeMap<String, HostConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
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
}
