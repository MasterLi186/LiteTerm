use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

/// Authentication method for an SSH connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    Keyring,
    Key,
    Agent,
}

/// Configuration for a single SSH host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostConfig {
    pub label: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthMethod,
    pub key_path: String,
    pub charset: String,
    #[serde(default)]
    pub proxy_jump: String,
}

/// A named group of hosts, with a display label and color.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupConfig {
    pub label: String,
    pub color: String,
    #[serde(default)]
    pub hosts: BTreeMap<String, HostConfig>,
}

/// The top-level store holding all connection groups.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectionStore {
    #[serde(default)]
    pub groups: BTreeMap<String, GroupConfig>,
}

/// Flat group info returned for UI listing.
#[derive(Debug, Clone)]
pub struct GroupInfo {
    pub id: String,
    pub label: String,
    pub color: String,
}

/// Runtime connection configuration derived from a HostConfig.
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthMethod,
    pub key_path: String,
    pub charset: String,
    pub proxy_jump: String,
}

impl From<&HostConfig> for ConnectionConfig {
    fn from(h: &HostConfig) -> Self {
        Self {
            host: h.host.clone(),
            port: h.port,
            user: h.user.clone(),
            auth: h.auth.clone(),
            key_path: h.key_path.clone(),
            charset: h.charset.clone(),
            proxy_jump: h.proxy_jump.clone(),
        }
    }
}

impl ConnectionStore {
    /// Add a new group. If it already exists, this overwrites label/color
    /// but preserves existing hosts.
    pub fn add_group(&mut self, id: &str, label: &str, color: &str) {
        let entry = self
            .groups
            .entry(id.to_string())
            .or_insert_with(|| GroupConfig {
                label: label.to_string(),
                color: color.to_string(),
                hosts: BTreeMap::new(),
            });
        entry.label = label.to_string();
        entry.color = color.to_string();
    }

    /// Add a host to an existing group. If the group does not exist, this is a no-op.
    pub fn add_host(&mut self, group_id: &str, host_id: &str, host: HostConfig) {
        if let Some(group) = self.groups.get_mut(group_id) {
            group.hosts.insert(host_id.to_string(), host);
        }
    }

    /// Remove a host from a group.
    pub fn remove_host(&mut self, group_id: &str, host_id: &str) {
        if let Some(group) = self.groups.get_mut(group_id) {
            group.hosts.remove(host_id);
        }
    }

    /// Return a list of all groups as flat GroupInfo values (sorted by id).
    pub fn groups(&self) -> Vec<GroupInfo> {
        self.groups
            .iter()
            .map(|(id, g)| GroupInfo {
                id: id.clone(),
                label: g.label.clone(),
                color: g.color.clone(),
            })
            .collect()
    }

    /// Return all hosts in a group as `(host_id, &HostConfig)` pairs.
    /// Returns an empty vec if the group does not exist.
    pub fn hosts_in_group(&self, group_id: &str) -> Vec<(String, &HostConfig)> {
        match self.groups.get(group_id) {
            Some(group) => group
                .hosts
                .iter()
                .map(|(id, h)| (id.clone(), h))
                .collect(),
            None => Vec::new(),
        }
    }

    /// Look up a specific host and return a runtime ConnectionConfig.
    pub fn get_connection_config(
        &self,
        group_id: &str,
        host_id: &str,
    ) -> Option<ConnectionConfig> {
        self.groups
            .get(group_id)
            .and_then(|g| g.hosts.get(host_id))
            .map(ConnectionConfig::from)
    }

    /// Load a ConnectionStore from a TOML file.
    /// Returns an empty default store if the file does not exist.
    pub fn load_from(path: &Path) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(content) => {
                let store: ConnectionStore = toml::from_str(&content)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                Ok(store)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(ConnectionStore::default()),
            Err(e) => Err(e),
        }
    }

    /// Save the ConnectionStore to a TOML file, creating parent directories if needed.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| io::Error::other(e))?;
        fs::write(path, content)
    }
}
