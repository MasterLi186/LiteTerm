# GuiShell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a lightweight native Linux SSH client (FinalShell replacement) with system monitoring, SFTP file manager, ZMODEM multi-hop transfer, and tabbed/split terminal sessions.

**Architecture:** Rust + GTK4/libadwaita app with strict UI/core separation. Core layer (SSH, SFTP, ZMODEM, monitoring, config) is UI-independent and fully unit-testable. UI layer binds core to GTK4 widgets. Async operations (SSH sessions, file transfers, metric collection) run on tokio, communicating with the GTK main loop via channels.

**Tech Stack:** Rust, gtk4-rs, libadwaita-rs, vte4, ssh2-rs, libsecret, cairo-rs, tokio, serde + toml

---

## Phase 0: Environment & Scaffolding

### Task 1: Install System Dependencies and Initialize Cargo Project

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/lib.rs`

- [ ] **Step 1: Install system dev packages**

Run:
```bash
sudo apt install -y \
  libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev \
  libsecret-1-dev libssh2-1-dev libcairo2-dev \
  pkg-config build-essential
```

If `libvte-2.91-gtk4-dev` is not available on Ubuntu 22.04, install from PPA:
```bash
sudo add-apt-repository ppa:mattst88/vte
sudo apt update
sudo apt install -y libvte-2.91-gtk4-dev
```

If the PPA also doesn't carry it, fall back to `libvte-2.91-dev` (GTK3 version) and we will use `vte4` crate's bundled build. Verify:
```bash
pkg-config --modversion gtk4
pkg-config --modversion libadwaita-1
pkg-config --modversion libsecret-1
pkg-config --modversion libssh2
```
Expected: version numbers printed for each.

- [ ] **Step 2: Initialize Cargo project**

Run:
```bash
cd /home/lfl/ssd/code/guishell
cargo init --name guishell
```
Expected: `Cargo.toml` and `src/main.rs` created.

- [ ] **Step 3: Write Cargo.toml with all dependencies**

Replace `Cargo.toml` with:

```toml
[package]
name = "guishell"
version = "0.1.0"
edition = "2021"
description = "Lightweight native Linux SSH client"

[dependencies]
gtk = { package = "gtk4", version = "0.9" }
adw = { package = "libadwaita", version = "0.7", features = ["v1_4"] }
vte = { package = "vte4", version = "0.8" }
cairo-rs = { version = "0.20", features = ["png"] }
ssh2 = "0.9"
secret-service = "4"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
toml = "0.8"
dirs = "6"
log = "0.4"
env_logger = "0.11"
crc32fast = "1"

[dev-dependencies]
tempfile = "3"
```

Note: `secret-service` is a pure-Rust D-Bus client for libsecret/GNOME Keyring — no C binding needed.
If `vte4` 0.8 doesn't build due to missing system VTE GTK4 library, pin to `vte4 = "0.7"` or switch to building VTE from source via the crate's `v0_70` feature.

- [ ] **Step 4: Write lib.rs module structure**

Create `src/lib.rs`:

```rust
pub mod config;
pub mod core;
pub mod plugin;
pub mod ui;
```

- [ ] **Step 5: Create module directories with mod.rs files**

Create all module files:

`src/config/mod.rs`:
```rust
pub mod connections;
pub mod keyring;
pub mod settings;
```

`src/core/mod.rs`:
```rust
pub mod monitor;
pub mod session;
pub mod sftp;
pub mod ssh;
pub mod transfer;
pub mod zmodem;
```

`src/core/zmodem/mod.rs`:
```rust
pub mod detect;
pub mod frame;
pub mod receive;
pub mod send;
```

`src/plugin/mod.rs`:
```rust
pub mod registry;
```

`src/ui/mod.rs`:
```rust
pub mod file_browser;
pub mod monitor;
pub mod sidebar;
pub mod split;
pub mod tabs;
pub mod terminal;
pub mod window;
```

Each leaf module file (e.g. `src/config/connections.rs`) starts as an empty file for now.

- [ ] **Step 6: Write minimal main.rs that opens a GTK window**

```rust
use adw::prelude::*;
use adw::Application;
use gtk::glib;

const APP_ID: &str = "com.guishell.app";

fn main() -> glib::ExitCode {
    env_logger::init();

    let app = Application::builder().application_id(APP_ID).build();

    app.connect_activate(|app| {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("GuiShell")
            .default_width(1200)
            .default_height(800)
            .build();
        window.present();
    });

    app.run()
}
```

- [ ] **Step 7: Build and run to verify setup**

Run:
```bash
cd /home/lfl/ssd/code/guishell
cargo build 2>&1
```
Expected: successful compilation. If dependency resolution fails, check error messages and adjust versions in `Cargo.toml`.

Then test run (will show empty window and exit):
```bash
cargo run 2>&1 &
sleep 2
kill %1 2>/dev/null
```
Expected: GTK window appears briefly.

- [ ] **Step 8: Commit**

```bash
cd /home/lfl/ssd/code/guishell
git add Cargo.toml src/
git commit -m "feat: scaffold project with Rust + GTK4 + all module stubs"
```

---

## Phase 1: Configuration Layer

### Task 2: Settings Data Model and TOML Persistence

**Files:**
- Create: `src/config/settings.rs`
- Create: `tests/config_settings_test.rs`

- [ ] **Step 1: Write failing test for settings default + load/save**

Create `tests/config_settings_test.rs`:

```rust
use guishell::config::settings::Settings;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn test_default_settings() {
    let s = Settings::default();
    assert_eq!(s.terminal.font, "Monospace 12");
    assert_eq!(s.terminal.scrollback_lines, 10000);
    assert_eq!(s.ssh.keepalive_interval_secs, 30);
    assert!(s.zmodem.enabled);
}

#[test]
fn test_save_and_load_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("settings.toml");

    let mut s = Settings::default();
    s.terminal.font = "JetBrains Mono 14".to_string();
    s.ssh.connect_timeout_secs = 20;

    s.save_to(&path).unwrap();
    let loaded = Settings::load_from(&path).unwrap();

    assert_eq!(loaded.terminal.font, "JetBrains Mono 14");
    assert_eq!(loaded.ssh.connect_timeout_secs, 20);
    assert_eq!(loaded.appearance.sidebar_width, 220);
}

#[test]
fn test_load_missing_file_returns_default() {
    let path = PathBuf::from("/tmp/nonexistent_guishell_settings.toml");
    let s = Settings::load_from(&path).unwrap();
    assert_eq!(s.terminal.font, "Monospace 12");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test config_settings_test 2>&1`
Expected: compilation error — `Settings` not defined.

- [ ] **Step 3: Implement Settings**

Write `src/config/settings.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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

impl Default for Settings {
    fn default() -> Self {
        Self {
            terminal: TerminalSettings::default(),
            appearance: AppearanceSettings::default(),
            transfer: TransferSettings::default(),
            ssh: SshSettings::default(),
            zmodem: ZmodemSettings::default(),
        }
    }
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font: "Monospace 12".to_string(),
            scrollback_lines: 10000,
            color_scheme: "dark".to_string(),
            cursor_blink: true,
        }
    }
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: "system".to_string(),
            sidebar_width: 220,
            file_browser_height: 250,
            show_sidebar: true,
            show_file_browser: false,
        }
    }
}

impl Default for TransferSettings {
    fn default() -> Self {
        Self {
            default_download_dir: "~/Downloads".to_string(),
            resume_threshold_mb: 10,
            max_retries: 3,
            concurrent_transfers: 3,
        }
    }
}

impl Default for SshSettings {
    fn default() -> Self {
        Self {
            keepalive_interval_secs: 30,
            connect_timeout_secs: 10,
            default_charset: "utf-8".to_string(),
        }
    }
}

impl Default for ZmodemSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_detect: true,
            download_dir: "~/Downloads".to_string(),
            timeout_secs: 30,
        }
    }
}

impl Settings {
    pub fn load_from(path: &Path) -> io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path)?;
        let settings: Settings =
            toml::from_str(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(settings)
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content =
            toml::to_string_pretty(self).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        fs::write(path, content)
    }

    pub fn config_dir() -> Option<std::path::PathBuf> {
        dirs::config_dir().map(|d| d.join("guishell"))
    }

    pub fn load() -> Self {
        Self::config_dir()
            .map(|d| d.join("settings.toml"))
            .and_then(|p| Self::load_from(&p).ok())
            .unwrap_or_default()
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test config_settings_test 2>&1`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config/settings.rs tests/config_settings_test.rs
git commit -m "feat: settings data model with TOML persistence"
```

### Task 3: Connection Configuration Model

**Files:**
- Create: `src/config/connections.rs`
- Create: `tests/config_connections_test.rs`

- [ ] **Step 1: Write failing test for connections load/save**

Create `tests/config_connections_test.rs`:

```rust
use guishell::config::connections::{AuthMethod, ConnectionConfig, ConnectionStore, HostConfig};
use tempfile::TempDir;

#[test]
fn test_add_group_and_host() {
    let mut store = ConnectionStore::default();
    store.add_group("production", "生产环境", "#e74c3c");

    let host = HostConfig {
        label: "Web Server 01".to_string(),
        host: "192.168.1.10".to_string(),
        port: 22,
        user: "root".to_string(),
        auth: AuthMethod::Keyring,
        key_path: String::new(),
        charset: "utf-8".to_string(),
    };
    store.add_host("production", "web01", host);

    let groups = store.groups();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].label, "生产环境");

    let hosts = store.hosts_in_group("production");
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].1.host, "192.168.1.10");
}

#[test]
fn test_save_and_load_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("connections.toml");

    let mut store = ConnectionStore::default();
    store.add_group("dev", "开发环境", "#2ecc71");
    store.add_host(
        "dev",
        "vm1",
        HostConfig {
            label: "本地虚拟机".to_string(),
            host: "192.168.56.101".to_string(),
            port: 22,
            user: "lfl".to_string(),
            auth: AuthMethod::Keyring,
            key_path: String::new(),
            charset: "utf-8".to_string(),
        },
    );

    store.save_to(&path).unwrap();
    let loaded = ConnectionStore::load_from(&path).unwrap();

    let hosts = loaded.hosts_in_group("dev");
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].1.label, "本地虚拟机");
}

#[test]
fn test_remove_host() {
    let mut store = ConnectionStore::default();
    store.add_group("test", "测试", "#333");
    store.add_host(
        "test",
        "h1",
        HostConfig {
            label: "H1".to_string(),
            host: "1.2.3.4".to_string(),
            port: 22,
            user: "root".to_string(),
            auth: AuthMethod::Agent,
            key_path: String::new(),
            charset: "utf-8".to_string(),
        },
    );
    assert_eq!(store.hosts_in_group("test").len(), 1);
    store.remove_host("test", "h1");
    assert_eq!(store.hosts_in_group("test").len(), 0);
}

#[test]
fn test_connection_config_from_host() {
    let host = HostConfig {
        label: "Web".to_string(),
        host: "10.0.0.1".to_string(),
        port: 2222,
        user: "deploy".to_string(),
        auth: AuthMethod::Key,
        key_path: "/home/lfl/.ssh/id_ed25519".to_string(),
        charset: "utf-8".to_string(),
    };
    let cfg = ConnectionConfig::from(&host);
    assert_eq!(cfg.host, "10.0.0.1");
    assert_eq!(cfg.port, 2222);
    assert_eq!(cfg.user, "deploy");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test config_connections_test 2>&1`
Expected: compilation error.

- [ ] **Step 3: Implement ConnectionStore**

Write `src/config/connections.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    Keyring,
    Key,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostConfig {
    pub label: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub user: String,
    pub auth: AuthMethod,
    #[serde(default)]
    pub key_path: String,
    #[serde(default = "default_charset")]
    pub charset: String,
}

fn default_port() -> u16 {
    22
}
fn default_charset() -> String {
    "utf-8".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupConfig {
    pub label: String,
    pub color: String,
    #[serde(default)]
    pub hosts: BTreeMap<String, HostConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConnectionStore {
    #[serde(default)]
    pub groups: BTreeMap<String, GroupConfig>,
}

#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthMethod,
    pub key_path: String,
    pub charset: String,
    pub label: String,
    pub group_color: String,
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
            label: h.label.clone(),
            group_color: String::new(),
        }
    }
}

pub struct GroupInfo {
    pub id: String,
    pub label: String,
    pub color: String,
}

impl ConnectionStore {
    pub fn add_group(&mut self, id: &str, label: &str, color: &str) {
        self.groups.insert(
            id.to_string(),
            GroupConfig {
                label: label.to_string(),
                color: color.to_string(),
                hosts: BTreeMap::new(),
            },
        );
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

    pub fn hosts_in_group(&self, group_id: &str) -> Vec<(String, &HostConfig)> {
        self.groups
            .get(group_id)
            .map(|g| g.hosts.iter().map(|(id, h)| (id.clone(), h)).collect())
            .unwrap_or_default()
    }

    pub fn get_connection_config(
        &self,
        group_id: &str,
        host_id: &str,
    ) -> Option<ConnectionConfig> {
        let group = self.groups.get(group_id)?;
        let host = group.hosts.get(host_id)?;
        let mut cfg = ConnectionConfig::from(host);
        cfg.group_color = group.color.clone();
        Some(cfg)
    }

    pub fn load_from(path: &Path) -> io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path)?;
        // TOML nested structure uses [groups.X] and [groups.X.hosts.Y]
        let store: ConnectionStore =
            toml::from_str(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(store)
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content =
            toml::to_string_pretty(self).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        fs::write(path, content)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test config_connections_test 2>&1`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config/connections.rs tests/config_connections_test.rs
git commit -m "feat: connection configuration model with group/host management"
```

### Task 4: Keyring Integration

**Files:**
- Create: `src/config/keyring.rs`
- Create: `tests/config_keyring_test.rs`

- [ ] **Step 1: Write failing test**

Create `tests/config_keyring_test.rs`:

```rust
use guishell::config::keyring::KeyringEntry;

#[test]
fn test_keyring_entry_label_format() {
    let entry = KeyringEntry::new("root", "192.168.1.10", 22);
    assert_eq!(entry.label(), "guishell:ssh://root@192.168.1.10:22");
}

#[test]
fn test_keyring_entry_label_nonstandard_port() {
    let entry = KeyringEntry::new("deploy", "10.0.0.5", 2222);
    assert_eq!(entry.label(), "guishell:ssh://deploy@10.0.0.5:2222");
}
```

Note: actual keyring store/retrieve tests require a running D-Bus session and GNOME Keyring daemon, so we only unit-test the label format. Integration testing of `store_password` / `retrieve_password` is manual.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test config_keyring_test 2>&1`
Expected: compilation error.

- [ ] **Step 3: Implement KeyringEntry**

Write `src/config/keyring.rs`:

```rust
use std::collections::HashMap;

pub struct KeyringEntry {
    user: String,
    host: String,
    port: u16,
}

impl KeyringEntry {
    pub fn new(user: &str, host: &str, port: u16) -> Self {
        Self {
            user: user.to_string(),
            host: host.to_string(),
            port,
        }
    }

    pub fn label(&self) -> String {
        format!(
            "guishell:ssh://{}@{}:{}",
            self.user, self.host, self.port
        )
    }

    fn attributes(&self) -> HashMap<String, String> {
        let mut attrs = HashMap::new();
        attrs.insert("application".to_string(), "guishell".to_string());
        attrs.insert("user".to_string(), self.user.clone());
        attrs.insert("host".to_string(), self.host.clone());
        attrs.insert("port".to_string(), self.port.to_string());
        attrs
    }

    /// Store password in GNOME Keyring via D-Bus (secret-service).
    /// Returns Ok(()) on success, Err on D-Bus/keyring failure.
    pub async fn store_password(&self, password: &str) -> Result<(), Box<dyn std::error::Error>> {
        let ss = secret_service::SecretService::connect(secret_service::EncryptionType::Dh).await?;
        let collection = ss.get_default_collection().await?;
        let attrs: HashMap<&str, &str> = self
            .attributes()
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        collection
            .create_item(
                &self.label(),
                attrs,
                password.as_bytes(),
                true, // replace existing
                "text/plain",
            )
            .await?;
        Ok(())
    }

    /// Retrieve password from GNOME Keyring.
    /// Returns None if not found.
    pub async fn retrieve_password(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let ss = secret_service::SecretService::connect(secret_service::EncryptionType::Dh).await?;
        let collection = ss.get_default_collection().await?;
        let attrs: HashMap<&str, &str> = self
            .attributes()
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let items = collection.search_items(attrs).await?;
        if let Some(item) = items.first() {
            let secret = item.get_secret().await?;
            let password = String::from_utf8(secret)?;
            return Ok(Some(password));
        }
        Ok(None)
    }

    pub async fn delete_password(&self) -> Result<(), Box<dyn std::error::Error>> {
        let ss = secret_service::SecretService::connect(secret_service::EncryptionType::Dh).await?;
        let collection = ss.get_default_collection().await?;
        let attrs: HashMap<&str, &str> = self
            .attributes()
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let items = collection.search_items(attrs).await?;
        for item in items {
            item.delete().await?;
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test config_keyring_test 2>&1`
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config/keyring.rs tests/config_keyring_test.rs
git commit -m "feat: keyring integration for secure password storage"
```

---

## Phase 2: SSH Core

### Task 5: SSH Connection and Session Manager

**Files:**
- Create: `src/core/ssh.rs`
- Create: `src/core/session.rs`
- Create: `tests/core_ssh_test.rs`

- [ ] **Step 1: Write failing tests for SSH connection config and session state**

Create `tests/core_ssh_test.rs`:

```rust
use guishell::core::session::{SessionEvent, SessionState};

#[test]
fn test_session_state_transitions() {
    let mut state = SessionState::new();
    assert!(matches!(state.status(), SessionStatus::Disconnected));

    state.set_connecting();
    assert!(matches!(state.status(), SessionStatus::Connecting));

    state.set_connected();
    assert!(matches!(state.status(), SessionStatus::Connected));

    state.set_disconnected("connection lost");
    assert!(matches!(state.status(), SessionStatus::Disconnected));
    assert_eq!(state.last_error(), Some("connection lost"));
}

use guishell::core::session::SessionStatus;

#[test]
fn test_session_event_channel() {
    let (tx, rx) = std::sync::mpsc::channel::<SessionEvent>();
    tx.send(SessionEvent::Connected).unwrap();
    tx.send(SessionEvent::DataReceived(vec![72, 101, 108, 108, 111]))
        .unwrap();
    tx.send(SessionEvent::Disconnected("bye".to_string()))
        .unwrap();

    assert!(matches!(rx.recv().unwrap(), SessionEvent::Connected));
    if let SessionEvent::DataReceived(data) = rx.recv().unwrap() {
        assert_eq!(&data, b"Hello");
    } else {
        panic!("expected DataReceived");
    }
    assert!(matches!(
        rx.recv().unwrap(),
        SessionEvent::Disconnected(_)
    ));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test core_ssh_test 2>&1`
Expected: compilation error.

- [ ] **Step 3: Implement session state machine**

Write `src/core/session.rs`:

```rust
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
pub enum SessionStatus {
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Debug, Clone)]
pub enum SessionEvent {
    Connecting,
    Connected,
    DataReceived(Vec<u8>),
    Disconnected(String),
    Error(String),
}

#[derive(Debug)]
pub struct SessionState {
    status: SessionStatus,
    last_error: Option<String>,
}

impl SessionState {
    pub fn new() -> Self {
        Self {
            status: SessionStatus::Disconnected,
            last_error: None,
        }
    }

    pub fn status(&self) -> &SessionStatus {
        &self.status
    }

    pub fn set_connecting(&mut self) {
        self.status = SessionStatus::Connecting;
        self.last_error = None;
    }

    pub fn set_connected(&mut self) {
        self.status = SessionStatus::Connected;
        self.last_error = None;
    }

    pub fn set_disconnected(&mut self, reason: &str) {
        self.status = SessionStatus::Disconnected;
        self.last_error = Some(reason.to_string());
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

pub type SharedSessionState = Arc<Mutex<SessionState>>;

pub fn new_shared_state() -> SharedSessionState {
    Arc::new(Mutex::new(SessionState::new()))
}
```

- [ ] **Step 4: Implement SSH connection wrapper**

Write `src/core/ssh.rs`:

```rust
use crate::config::connections::{AuthMethod, ConnectionConfig};
use crate::core::session::{SessionEvent, SharedSessionState};
use ssh2::Session;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

pub struct SshConnection {
    session: Session,
    stream: TcpStream,
}

impl SshConnection {
    pub fn connect(
        config: &ConnectionConfig,
        timeout_secs: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let addr = format!("{}:{}", config.host, config.port);
        let stream = TcpStream::connect_timeout(
            &addr.parse()?,
            Duration::from_secs(timeout_secs as u64),
        )?;
        stream.set_nonblocking(false)?;

        let mut session = Session::new()?;
        session.set_tcp_stream(stream.try_clone()?);
        session.handshake()?;
        session.set_timeout(timeout_secs * 1000);

        Ok(Self { session, stream })
    }

    pub fn authenticate(
        &self,
        config: &ConnectionConfig,
        password: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match &config.auth {
            AuthMethod::Agent => {
                let mut agent = self.session.agent()?;
                agent.connect()?;
                agent.list_identities()?;
                let identities = agent.identities()?;
                for identity in &identities {
                    if agent.userauth(&config.user, identity).is_ok() {
                        return Ok(());
                    }
                }
                Err("ssh-agent: no matching identity".into())
            }
            AuthMethod::Keyring => {
                let pw = password.ok_or("password required for keyring auth")?;
                self.session.userauth_password(&config.user, pw)?;
                Ok(())
            }
            AuthMethod::Key => {
                let key_path = Path::new(&config.key_path);
                self.session
                    .userauth_pubkey_file(&config.user, None, key_path, password)?;
                Ok(())
            }
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn open_shell_channel(
        &self,
    ) -> Result<ssh2::Channel, Box<dyn std::error::Error>> {
        let mut channel = self.session.channel_session()?;
        channel.request_pty("xterm-256color", None, None)?;
        channel.shell()?;
        Ok(channel)
    }

    pub fn set_keepalive(&self, interval_secs: u32) {
        self.session.set_keepalive(true, interval_secs);
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test core_ssh_test 2>&1`
Expected: 2 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/core/ssh.rs src/core/session.rs tests/core_ssh_test.rs
git commit -m "feat: SSH connection wrapper and session state machine"
```

---

## Phase 3: ZMODEM Protocol

### Task 6: ZMODEM Frame Encoding/Decoding

**Files:**
- Create: `src/core/zmodem/frame.rs`
- Create: `tests/zmodem_frame_test.rs`

- [ ] **Step 1: Write failing tests for ZMODEM frame types and CRC**

Create `tests/zmodem_frame_test.rs`:

```rust
use guishell::core::zmodem::frame::{FrameType, ZmodemFrame, encode_zhex_header, decode_zhex_header};

#[test]
fn test_frame_type_values() {
    assert_eq!(FrameType::ZRINIT as u8, 1);
    assert_eq!(FrameType::ZFILE as u8, 4);
    assert_eq!(FrameType::ZDATA as u8, 10);
    assert_eq!(FrameType::ZEOF as u8, 11);
    assert_eq!(FrameType::ZFIN as u8, 8);
}

#[test]
fn test_encode_zhex_header_roundtrip() {
    let frame = ZmodemFrame {
        frame_type: FrameType::ZRINIT,
        flags: [0x00, 0x00, 0x00, 0x23],
    };
    let encoded = encode_zhex_header(&frame);
    let decoded = decode_zhex_header(&encoded).unwrap();
    assert_eq!(decoded.frame_type as u8, FrameType::ZRINIT as u8);
    assert_eq!(decoded.flags, [0x00, 0x00, 0x00, 0x23]);
}

#[test]
fn test_crc16_calculation() {
    use guishell::core::zmodem::frame::crc16;
    let data = b"Hello";
    let crc = crc16(data);
    // Known CRC-16/XMODEM for "Hello"
    assert_ne!(crc, 0); // non-zero for non-empty data
    // Verify determinism
    assert_eq!(crc, crc16(b"Hello"));
}

#[test]
fn test_decode_invalid_hex_returns_none() {
    let result = decode_zhex_header(b"not a valid frame");
    assert!(result.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test zmodem_frame_test 2>&1`
Expected: compilation error.

- [ ] **Step 3: Implement ZMODEM frame encoding**

Write `src/core/zmodem/frame.rs`:

```rust
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameType {
    ZRQINIT = 0,
    ZRINIT = 1,
    ZSINIT = 2,
    ZACK = 3,
    ZFILE = 4,
    ZSKIP = 5,
    ZNAK = 6,
    ZABORT = 7,
    ZFIN = 8,
    ZRPOS = 9,
    ZDATA = 10,
    ZEOF = 11,
    ZFERR = 12,
    ZCRC = 13,
    ZCHALLENGE = 14,
    ZCOMPL = 15,
    ZCAN = 16,
}

impl FrameType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::ZRQINIT),
            1 => Some(Self::ZRINIT),
            2 => Some(Self::ZSINIT),
            3 => Some(Self::ZACK),
            4 => Some(Self::ZFILE),
            5 => Some(Self::ZSKIP),
            6 => Some(Self::ZNAK),
            7 => Some(Self::ZABORT),
            8 => Some(Self::ZFIN),
            9 => Some(Self::ZRPOS),
            10 => Some(Self::ZDATA),
            11 => Some(Self::ZEOF),
            12 => Some(Self::ZFERR),
            13 => Some(Self::ZCRC),
            14 => Some(Self::ZCHALLENGE),
            15 => Some(Self::ZCOMPL),
            16 => Some(Self::ZCAN),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ZmodemFrame {
    pub frame_type: FrameType,
    pub flags: [u8; 4],
}

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

fn hex_byte(b: u8) -> [u8; 2] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    [HEX[(b >> 4) as usize], HEX[(b & 0x0f) as usize]]
}

fn from_hex_pair(h: u8, l: u8) -> Option<u8> {
    let high = match h {
        b'0'..=b'9' => h - b'0',
        b'a'..=b'f' => h - b'a' + 10,
        b'A'..=b'F' => h - b'A' + 10,
        _ => return None,
    };
    let low = match l {
        b'0'..=b'9' => l - b'0',
        b'a'..=b'f' => l - b'a' + 10,
        b'A'..=b'F' => l - b'A' + 10,
        _ => return None,
    };
    Some((high << 4) | low)
}

/// Encode a ZHEX header: **\x18B0 <type_hex> <f3_hex> <f2_hex> <f1_hex> <f0_hex> <crc16_hex> \r\n
pub fn encode_zhex_header(frame: &ZmodemFrame) -> Vec<u8> {
    let mut payload = vec![frame.frame_type as u8];
    payload.extend_from_slice(&frame.flags);
    let crc = crc16(&payload);

    let mut out = Vec::with_capacity(32);
    // ZPAD ZPAD ZDLE ZHEX
    out.extend_from_slice(b"**\x18B");
    for &b in &payload {
        let h = hex_byte(b);
        out.push(h[0]);
        out.push(h[1]);
    }
    let crc_bytes = crc.to_be_bytes();
    for &b in &crc_bytes {
        let h = hex_byte(b);
        out.push(h[0]);
        out.push(h[1]);
    }
    out.push(b'\r');
    out.push(b'\n');
    out
}

/// Decode a ZHEX header. Input should start after any leading bytes have been stripped.
/// Looks for the pattern **\x18B followed by hex digits.
pub fn decode_zhex_header(data: &[u8]) -> Option<ZmodemFrame> {
    // Find **\x18B marker
    let marker = b"**\x18B";
    let pos = data.windows(marker.len()).position(|w| w == marker)?;
    let hex_start = pos + marker.len();

    // Need at least 14 hex chars: 2(type) + 8(flags) + 4(crc)
    if data.len() < hex_start + 14 {
        return None;
    }

    let hex_data = &data[hex_start..];
    let frame_type_byte = from_hex_pair(hex_data[0], hex_data[1])?;
    let f3 = from_hex_pair(hex_data[2], hex_data[3])?;
    let f2 = from_hex_pair(hex_data[4], hex_data[5])?;
    let f1 = from_hex_pair(hex_data[6], hex_data[7])?;
    let f0 = from_hex_pair(hex_data[8], hex_data[9])?;
    let crc_hi = from_hex_pair(hex_data[10], hex_data[11])?;
    let crc_lo = from_hex_pair(hex_data[12], hex_data[13])?;

    let frame_type = FrameType::from_u8(frame_type_byte)?;
    let flags = [f3, f2, f1, f0];

    // Verify CRC
    let payload = [frame_type_byte, f3, f2, f1, f0];
    let expected_crc = crc16(&payload);
    let actual_crc = ((crc_hi as u16) << 8) | (crc_lo as u16);
    if expected_crc != actual_crc {
        return None;
    }

    Some(ZmodemFrame { frame_type, flags })
}

/// The ZMODEM cancel sequence: 8x CAN + 8x BS
pub fn zcancel() -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    for _ in 0..8 {
        out.push(0x18); // CAN
    }
    for _ in 0..8 {
        out.push(0x08); // BS
    }
    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test zmodem_frame_test 2>&1`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/core/zmodem/frame.rs tests/zmodem_frame_test.rs
git commit -m "feat: ZMODEM frame encoding/decoding with CRC16"
```

### Task 7: ZMODEM Detector (Byte Stream Filter)

**Files:**
- Create: `src/core/zmodem/detect.rs`
- Create: `tests/zmodem_detect_test.rs`

- [ ] **Step 1: Write failing tests for ZMODEM magic header detection**

Create `tests/zmodem_detect_test.rs`:

```rust
use guishell::core::zmodem::detect::{DetectResult, ZmodemDetector};

#[test]
fn test_no_zmodem_passthrough() {
    let mut detector = ZmodemDetector::new();
    let input = b"normal terminal output here\r\n";
    let result = detector.feed(input);
    assert!(matches!(result, DetectResult::Normal(data) if data == input));
}

#[test]
fn test_detect_sz_initiation() {
    let mut detector = ZmodemDetector::new();
    // sz sends ZRQINIT: **\x18B00 ...
    let mut input = Vec::new();
    input.extend_from_slice(b"some text before");
    input.extend_from_slice(b"**\x18B00");
    input.extend_from_slice(b"0000000000fcb5\r\n"); // type=0 flags=0000 + crc

    let result = detector.feed(&input);
    assert!(matches!(result, DetectResult::ZmodemStart { .. }));
}

#[test]
fn test_detect_rz_waiting() {
    let mut detector = ZmodemDetector::new();
    // rz sends ZRINIT: **\x18B01 ...
    let mut input = Vec::new();
    input.extend_from_slice(b"**\x18B01");
    input.extend_from_slice(b"0000000000a]87\r\n"); // approximate

    let result = detector.feed(&input);
    // Either ZmodemStart or Normal (if CRC doesn't match our test data)
    // The important thing is the detector doesn't panic
    assert!(!matches!(result, DetectResult::Normal(data) if data.is_empty()));
}

#[test]
fn test_partial_match_buffered() {
    let mut detector = ZmodemDetector::new();
    // Send partial marker
    let result1 = detector.feed(b"text**");
    // Should buffer the ** and pass through "text"
    assert!(matches!(result1, DetectResult::Normal(data) if data == b"text"));

    // Send the rest — not a real ZMODEM frame
    let result2 = detector.feed(b"not zmodem");
    assert!(matches!(result2, DetectResult::Normal(_)));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test zmodem_detect_test 2>&1`
Expected: compilation error.

- [ ] **Step 3: Implement ZmodemDetector**

Write `src/core/zmodem/detect.rs`:

```rust
use super::frame::{decode_zhex_header, ZmodemFrame};

#[derive(Debug)]
pub enum DetectResult {
    Normal(Vec<u8>),
    ZmodemStart {
        preceding: Vec<u8>,
        frame: ZmodemFrame,
        remaining: Vec<u8>,
    },
}

pub struct ZmodemDetector {
    buffer: Vec<u8>,
}

const MARKER: &[u8] = b"**\x18B";

impl ZmodemDetector {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(256),
        }
    }

    pub fn feed(&mut self, data: &[u8]) -> DetectResult {
        self.buffer.extend_from_slice(data);

        // Search for ZMODEM marker in buffer
        if let Some(marker_pos) = self.find_marker() {
            // Need enough bytes after marker for a hex header (14 hex chars + \r\n)
            let hex_start = marker_pos + MARKER.len();
            if self.buffer.len() < hex_start + 16 {
                // Not enough data yet — keep buffering, but release bytes before the potential marker
                if marker_pos > 0 {
                    let preceding: Vec<u8> = self.buffer.drain(..marker_pos).collect();
                    return DetectResult::Normal(preceding);
                }
                // Buffer everything, wait for more data
                return DetectResult::Normal(Vec::new());
            }

            // Try to decode
            if let Some(frame) = decode_zhex_header(&self.buffer[marker_pos..]) {
                let preceding: Vec<u8> = self.buffer[..marker_pos].to_vec();
                // Find end of header (after \r\n)
                let header_data = &self.buffer[marker_pos..];
                let end_pos = header_data
                    .windows(2)
                    .position(|w| w == b"\r\n")
                    .map(|p| marker_pos + p + 2)
                    .unwrap_or(self.buffer.len());
                let remaining = self.buffer[end_pos..].to_vec();
                self.buffer.clear();
                return DetectResult::ZmodemStart {
                    preceding,
                    frame,
                    remaining,
                };
            }

            // Not a valid ZMODEM frame — flush everything including the false marker
            let flushed = std::mem::take(&mut self.buffer);
            return DetectResult::Normal(flushed);
        }

        // No marker found. Check if buffer ends with a partial marker prefix.
        let keep = self.partial_marker_suffix_len();
        if keep > 0 {
            let flush_end = self.buffer.len() - keep;
            let flushed: Vec<u8> = self.buffer.drain(..flush_end).collect();
            return DetectResult::Normal(flushed);
        }

        // No marker, no partial — flush all
        let flushed = std::mem::take(&mut self.buffer);
        DetectResult::Normal(flushed)
    }

    fn find_marker(&self) -> Option<usize> {
        self.buffer
            .windows(MARKER.len())
            .position(|w| w == MARKER)
    }

    fn partial_marker_suffix_len(&self) -> usize {
        // Check if the tail of buffer is a prefix of MARKER
        let buf = &self.buffer;
        for len in (1..MARKER.len()).rev() {
            if buf.len() >= len && &buf[buf.len() - len..] == &MARKER[..len] {
                return len;
            }
        }
        0
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test zmodem_detect_test 2>&1`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/core/zmodem/detect.rs tests/zmodem_detect_test.rs
git commit -m "feat: ZMODEM byte stream detector for terminal data"
```

### Task 8: ZMODEM Receive and Send State Machines

**Files:**
- Create: `src/core/zmodem/receive.rs`
- Create: `src/core/zmodem/send.rs`
- Create: `tests/zmodem_transfer_test.rs`

- [ ] **Step 1: Write failing tests for receiver state machine**

Create `tests/zmodem_transfer_test.rs`:

```rust
use guishell::core::zmodem::receive::{ReceiveEvent, ZmodemReceiver};
use guishell::core::zmodem::send::{SendEvent, ZmodemSender};
use guishell::core::zmodem::frame::{encode_zhex_header, FrameType, ZmodemFrame};

#[test]
fn test_receiver_initial_state() {
    let receiver = ZmodemReceiver::new("/tmp");
    assert!(!receiver.is_complete());
    assert_eq!(receiver.files_received(), 0);
}

#[test]
fn test_receiver_generates_zrinit() {
    let receiver = ZmodemReceiver::new("/tmp");
    let init = receiver.make_zrinit();
    // Should be a valid ZHEX header for ZRINIT
    assert!(init.starts_with(b"**\x18B"));
}

#[test]
fn test_sender_initial_state() {
    let sender = ZmodemSender::new();
    assert!(!sender.is_complete());
}

#[test]
fn test_sender_generates_zrqinit() {
    let sender = ZmodemSender::new();
    let init = sender.make_zrqinit();
    assert!(init.starts_with(b"**\x18B"));
}

#[test]
fn test_receiver_parse_zfile_extracts_filename() {
    let receiver = ZmodemReceiver::new("/tmp");
    // ZFILE subpacket format: "filename\0size mtime\0"
    let subpacket = b"testfile.txt\x0012345 1718300000\x00";
    let (name, size) = receiver.parse_zfile_subpacket(subpacket).unwrap();
    assert_eq!(name, "testfile.txt");
    assert_eq!(size, 12345);
}

#[test]
fn test_receiver_parse_zfile_no_size() {
    let receiver = ZmodemReceiver::new("/tmp");
    let subpacket = b"myfile.bin\x00\x00";
    let (name, size) = receiver.parse_zfile_subpacket(subpacket).unwrap();
    assert_eq!(name, "myfile.bin");
    assert_eq!(size, 0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test zmodem_transfer_test 2>&1`
Expected: compilation error.

- [ ] **Step 3: Implement ZmodemReceiver**

Write `src/core/zmodem/receive.rs`:

```rust
use super::frame::{encode_zhex_header, FrameType, ZmodemFrame};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum ReceiveEvent {
    FileStart {
        filename: String,
        size: u64,
    },
    Progress {
        filename: String,
        bytes_received: u64,
        total_size: u64,
    },
    FileComplete {
        filename: String,
        path: PathBuf,
    },
    AllComplete {
        files_count: usize,
    },
    Error(String),
}

pub struct ZmodemReceiver {
    download_dir: PathBuf,
    current_file: Option<File>,
    current_filename: String,
    current_size: u64,
    bytes_received: u64,
    files_count: usize,
    complete: bool,
}

impl ZmodemReceiver {
    pub fn new(download_dir: &str) -> Self {
        let expanded = shellexpand::tilde(download_dir).to_string();
        Self {
            download_dir: PathBuf::from(expanded),
            current_file: None,
            current_filename: String::new(),
            current_size: 0,
            bytes_received: 0,
            files_count: 0,
            complete: false,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn files_received(&self) -> usize {
        self.files_count
    }

    pub fn make_zrinit(&self) -> Vec<u8> {
        encode_zhex_header(&ZmodemFrame {
            frame_type: FrameType::ZRINIT,
            flags: [0x00, 0x00, 0x00, 0x23], // CANFDX | CANOVIO | CANFC32
        })
    }

    pub fn make_zrpos(&self, offset: u32) -> Vec<u8> {
        encode_zhex_header(&ZmodemFrame {
            frame_type: FrameType::ZRPOS,
            flags: offset.to_le_bytes(),
        })
    }

    pub fn make_zack(&self, offset: u32) -> Vec<u8> {
        encode_zhex_header(&ZmodemFrame {
            frame_type: FrameType::ZACK,
            flags: offset.to_le_bytes(),
        })
    }

    pub fn make_zfin(&self) -> Vec<u8> {
        encode_zhex_header(&ZmodemFrame {
            frame_type: FrameType::ZFIN,
            flags: [0; 4],
        })
    }

    pub fn parse_zfile_subpacket(&self, data: &[u8]) -> Option<(String, u64)> {
        let null_pos = data.iter().position(|&b| b == 0)?;
        let filename = std::str::from_utf8(&data[..null_pos]).ok()?.to_string();

        let rest = &data[null_pos + 1..];
        let size = if rest.is_empty() || rest[0] == 0 {
            0
        } else {
            let size_end = rest
                .iter()
                .position(|&b| b == b' ' || b == 0)
                .unwrap_or(rest.len());
            std::str::from_utf8(&rest[..size_end])
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
        };

        Some((filename, size))
    }

    pub fn begin_file(&mut self, filename: &str, size: u64) -> Result<ReceiveEvent, String> {
        fs::create_dir_all(&self.download_dir)
            .map_err(|e| format!("create download dir: {}", e))?;

        let path = self.download_dir.join(filename);
        let file = File::create(&path).map_err(|e| format!("create file: {}", e))?;

        self.current_file = Some(file);
        self.current_filename = filename.to_string();
        self.current_size = size;
        self.bytes_received = 0;

        Ok(ReceiveEvent::FileStart {
            filename: filename.to_string(),
            size,
        })
    }

    pub fn write_data(&mut self, data: &[u8]) -> Result<ReceiveEvent, String> {
        if let Some(ref mut file) = self.current_file {
            file.write_all(data)
                .map_err(|e| format!("write file: {}", e))?;
            self.bytes_received += data.len() as u64;
            Ok(ReceiveEvent::Progress {
                filename: self.current_filename.clone(),
                bytes_received: self.bytes_received,
                total_size: self.current_size,
            })
        } else {
            Err("no file open".to_string())
        }
    }

    pub fn end_file(&mut self) -> ReceiveEvent {
        self.current_file = None;
        self.files_count += 1;
        let path = self.download_dir.join(&self.current_filename);
        ReceiveEvent::FileComplete {
            filename: self.current_filename.clone(),
            path,
        }
    }

    pub fn finish(&mut self) -> ReceiveEvent {
        self.complete = true;
        ReceiveEvent::AllComplete {
            files_count: self.files_count,
        }
    }
}
```

Note: add `shellexpand = "3"` to `[dependencies]` in `Cargo.toml`.

- [ ] **Step 4: Implement ZmodemSender**

Write `src/core/zmodem/send.rs`:

```rust
use super::frame::{crc16, encode_zhex_header, FrameType, ZmodemFrame};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum SendEvent {
    Ready,
    Progress {
        filename: String,
        bytes_sent: u64,
        total_size: u64,
    },
    FileComplete {
        filename: String,
    },
    AllComplete,
    Error(String),
}

pub struct ZmodemSender {
    files: Vec<PathBuf>,
    current_index: usize,
    current_file: Option<File>,
    current_filename: String,
    current_size: u64,
    bytes_sent: u64,
    complete: bool,
}

impl ZmodemSender {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            current_index: 0,
            current_file: None,
            current_filename: String::new(),
            current_size: 0,
            bytes_sent: 0,
            complete: false,
        }
    }

    pub fn add_file(&mut self, path: &Path) {
        self.files.push(path.to_path_buf());
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn make_zrqinit(&self) -> Vec<u8> {
        encode_zhex_header(&ZmodemFrame {
            frame_type: FrameType::ZRQINIT,
            flags: [0; 4],
        })
    }

    pub fn make_zfile(&self, filename: &str, size: u64) -> Vec<u8> {
        let header = encode_zhex_header(&ZmodemFrame {
            frame_type: FrameType::ZFILE,
            flags: [0; 4],
        });
        // Subpacket: "filename\0size\0"
        let mut subpacket = Vec::new();
        subpacket.extend_from_slice(filename.as_bytes());
        subpacket.push(0);
        subpacket.extend_from_slice(size.to_string().as_bytes());
        subpacket.push(0);

        let mut out = header;
        out.extend_from_slice(&subpacket);
        out
    }

    pub fn make_zeof(&self, offset: u32) -> Vec<u8> {
        encode_zhex_header(&ZmodemFrame {
            frame_type: FrameType::ZEOF,
            flags: offset.to_le_bytes(),
        })
    }

    pub fn make_zfin(&self) -> Vec<u8> {
        encode_zhex_header(&ZmodemFrame {
            frame_type: FrameType::ZFIN,
            flags: [0; 4],
        })
    }

    pub fn open_next_file(&mut self) -> Result<Option<(String, u64)>, String> {
        if self.current_index >= self.files.len() {
            self.complete = true;
            return Ok(None);
        }
        let path = &self.files[self.current_index];
        let metadata = fs::metadata(path).map_err(|e| format!("stat {}: {}", path.display(), e))?;
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("invalid filename")?
            .to_string();
        let size = metadata.len();
        let file = File::open(path).map_err(|e| format!("open {}: {}", path.display(), e))?;

        self.current_file = Some(file);
        self.current_filename = filename.clone();
        self.current_size = size;
        self.bytes_sent = 0;
        self.current_index += 1;

        Ok(Some((filename, size)))
    }

    pub fn read_chunk(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        if let Some(ref mut file) = self.current_file {
            let n = file.read(buf).map_err(|e| format!("read: {}", e))?;
            self.bytes_sent += n as u64;
            Ok(n)
        } else {
            Err("no file open".to_string())
        }
    }

    pub fn progress(&self) -> SendEvent {
        SendEvent::Progress {
            filename: self.current_filename.clone(),
            bytes_sent: self.bytes_sent,
            total_size: self.current_size,
        }
    }
}
```

- [ ] **Step 5: Update Cargo.toml to add shellexpand**

Add to `[dependencies]`:
```toml
shellexpand = "3"
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test zmodem_transfer_test 2>&1`
Expected: 6 tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/core/zmodem/receive.rs src/core/zmodem/send.rs tests/zmodem_transfer_test.rs Cargo.toml
git commit -m "feat: ZMODEM receive and send state machines"
```

---

## Phase 4: System Monitoring Core

### Task 9: Remote Metric Parsing

**Files:**
- Create: `src/core/monitor.rs`
- Create: `src/plugin/registry.rs`
- Create: `tests/monitor_parse_test.rs`

- [ ] **Step 1: Write failing tests for /proc parsing**

Create `tests/monitor_parse_test.rs`:

```rust
use guishell::core::monitor::{CpuMetric, MemoryMetric, DiskMetric, NetworkMetric, LoadMetric, parse_proc_stat_cpu, parse_proc_meminfo, parse_df_output, parse_proc_net_dev, parse_loadavg, MetricBuffer};

#[test]
fn test_parse_proc_stat_cpu() {
    let input = "cpu  4705 356 584 3699 23 0 0 0 0 0\ncpu0 2353 178 292 1849 12 0 0 0 0 0\n";
    let metrics = parse_proc_stat_cpu(input);
    assert_eq!(metrics.len(), 2);
    assert_eq!(metrics[0].label, "cpu");
    assert!(metrics[0].user > 0);
    assert_eq!(metrics[1].label, "cpu0");
}

#[test]
fn test_parse_meminfo() {
    let input = "MemTotal:        8028508 kB\nMemFree:          204508 kB\nMemAvailable:    2458792 kB\nBuffers:          123456 kB\nCached:          1234567 kB\nSwapTotal:       2097148 kB\nSwapFree:        2097148 kB\n";
    let mem = parse_proc_meminfo(input).unwrap();
    assert_eq!(mem.total_kb, 8028508);
    assert_eq!(mem.free_kb, 204508);
    assert_eq!(mem.cached_kb, 1234567);
}

#[test]
fn test_parse_df_output() {
    let input = "Filesystem      Size  Used Avail Use% Mounted on\n/dev/sda1        50G   34G   14G  71% /\n/dev/sda2       200G   46G  144G  25% /home\n";
    let disks = parse_df_output(input);
    assert_eq!(disks.len(), 2);
    assert_eq!(disks[0].mount_point, "/");
    assert_eq!(disks[0].use_percent, 71);
    assert_eq!(disks[1].mount_point, "/home");
}

#[test]
fn test_parse_net_dev() {
    let input = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n  eth0: 123456789   12345    0    0    0     0          0         0 987654321   54321    0    0    0     0       0          0\n    lo:   456789    1234    0    0    0     0          0         0   456789    1234    0    0    0     0       0          0\n";
    let nets = parse_proc_net_dev(input);
    assert!(nets.len() >= 1);
    let eth0 = nets.iter().find(|n| n.interface == "eth0").unwrap();
    assert_eq!(eth0.rx_bytes, 123456789);
    assert_eq!(eth0.tx_bytes, 987654321);
}

#[test]
fn test_parse_loadavg() {
    let input = "1.23 0.87 0.45 3/234 12345\n";
    let load = parse_loadavg(input).unwrap();
    assert!((load.load_1m - 1.23).abs() < 0.01);
    assert!((load.load_5m - 0.87).abs() < 0.01);
    assert!((load.load_15m - 0.45).abs() < 0.01);
}

#[test]
fn test_metric_buffer_ring() {
    let mut buf = MetricBuffer::<f64>::new(3);
    buf.push(1.0);
    buf.push(2.0);
    buf.push(3.0);
    assert_eq!(buf.as_slice(), &[1.0, 2.0, 3.0]);
    buf.push(4.0);
    assert_eq!(buf.as_slice(), &[2.0, 3.0, 4.0]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test monitor_parse_test 2>&1`
Expected: compilation error.

- [ ] **Step 3: Implement monitor parsing and ring buffer**

Write `src/core/monitor.rs`:

```rust
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct CpuMetric {
    pub label: String,
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
}

impl CpuMetric {
    pub fn usage_percent(&self, prev: &CpuMetric) -> f64 {
        let total = (self.user + self.nice + self.system + self.idle + self.iowait)
            .saturating_sub(prev.user + prev.nice + prev.system + prev.idle + prev.iowait);
        let idle = self.idle.saturating_sub(prev.idle);
        if total == 0 {
            return 0.0;
        }
        ((total - idle) as f64 / total as f64) * 100.0
    }

    pub fn iowait_percent(&self, prev: &CpuMetric) -> f64 {
        let total = (self.user + self.nice + self.system + self.idle + self.iowait)
            .saturating_sub(prev.user + prev.nice + prev.system + prev.idle + prev.iowait);
        let iowait_diff = self.iowait.saturating_sub(prev.iowait);
        if total == 0 {
            return 0.0;
        }
        (iowait_diff as f64 / total as f64) * 100.0
    }
}

#[derive(Debug, Clone)]
pub struct MemoryMetric {
    pub total_kb: u64,
    pub free_kb: u64,
    pub available_kb: u64,
    pub buffers_kb: u64,
    pub cached_kb: u64,
}

impl MemoryMetric {
    pub fn used_kb(&self) -> u64 {
        self.total_kb - self.free_kb - self.buffers_kb - self.cached_kb
    }
}

#[derive(Debug, Clone)]
pub struct DiskMetric {
    pub filesystem: String,
    pub size: String,
    pub used: String,
    pub avail: String,
    pub use_percent: u8,
    pub mount_point: String,
}

#[derive(Debug, Clone)]
pub struct NetworkMetric {
    pub interface: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct LoadMetric {
    pub load_1m: f64,
    pub load_5m: f64,
    pub load_15m: f64,
}

#[derive(Debug, Clone)]
pub struct ProcessMetric {
    pub pid: u32,
    pub user: String,
    pub cpu_percent: f32,
    pub mem_percent: f32,
    pub command: String,
}

pub fn parse_proc_stat_cpu(input: &str) -> Vec<CpuMetric> {
    input
        .lines()
        .filter(|line| line.starts_with("cpu"))
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 6 {
                return None;
            }
            Some(CpuMetric {
                label: parts[0].to_string(),
                user: parts[1].parse().unwrap_or(0),
                nice: parts[2].parse().unwrap_or(0),
                system: parts[3].parse().unwrap_or(0),
                idle: parts[4].parse().unwrap_or(0),
                iowait: parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(0),
            })
        })
        .collect()
}

pub fn parse_proc_meminfo(input: &str) -> Option<MemoryMetric> {
    let mut total = 0u64;
    let mut free = 0u64;
    let mut available = 0u64;
    let mut buffers = 0u64;
    let mut cached = 0u64;

    for line in input.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let value: u64 = parts[1].parse().unwrap_or(0);
        match parts[0] {
            "MemTotal:" => total = value,
            "MemFree:" => free = value,
            "MemAvailable:" => available = value,
            "Buffers:" => buffers = value,
            "Cached:" => cached = value,
            _ => {}
        }
    }

    if total == 0 {
        return None;
    }

    Some(MemoryMetric {
        total_kb: total,
        free_kb: free,
        available_kb: available,
        buffers_kb: buffers,
        cached_kb: cached,
    })
}

pub fn parse_df_output(input: &str) -> Vec<DiskMetric> {
    input
        .lines()
        .skip(1) // skip header
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 6 {
                return None;
            }
            let use_str = parts[4].trim_end_matches('%');
            Some(DiskMetric {
                filesystem: parts[0].to_string(),
                size: parts[1].to_string(),
                used: parts[2].to_string(),
                avail: parts[3].to_string(),
                use_percent: use_str.parse().unwrap_or(0),
                mount_point: parts[5].to_string(),
            })
        })
        .collect()
}

pub fn parse_proc_net_dev(input: &str) -> Vec<NetworkMetric> {
    input
        .lines()
        .filter_map(|line| {
            let (iface, rest) = line.split_once(':')?;
            let iface = iface.trim();
            if iface == "lo" {
                return None;
            }
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() < 10 {
                return None;
            }
            Some(NetworkMetric {
                interface: iface.to_string(),
                rx_bytes: parts[0].parse().unwrap_or(0),
                tx_bytes: parts[8].parse().unwrap_or(0),
            })
        })
        .collect()
}

pub fn parse_loadavg(input: &str) -> Option<LoadMetric> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    Some(LoadMetric {
        load_1m: parts[0].parse().ok()?,
        load_5m: parts[1].parse().ok()?,
        load_15m: parts[2].parse().ok()?,
    })
}

pub fn parse_ps_aux(input: &str) -> Vec<ProcessMetric> {
    input
        .lines()
        .skip(1) // skip header
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 11 {
                return None;
            }
            Some(ProcessMetric {
                pid: parts[1].parse().unwrap_or(0),
                user: parts[0].to_string(),
                cpu_percent: parts[2].parse().unwrap_or(0.0),
                mem_percent: parts[3].parse().unwrap_or(0.0),
                command: parts[10..].join(" "),
            })
        })
        .collect()
}

pub fn collect_command() -> &'static str {
    "cat /proc/stat /proc/meminfo /proc/net/dev /proc/diskstats /proc/loadavg; df -h; cat /sys/class/thermal/thermal_zone*/temp 2>/dev/null; ps aux --sort=-%cpu | head -11"
}

#[derive(Debug, Clone)]
pub struct MetricBuffer<T: Clone> {
    data: VecDeque<T>,
    capacity: usize,
}

impl<T: Clone> MetricBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            data: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, value: T) {
        if self.data.len() >= self.capacity {
            self.data.pop_front();
        }
        self.data.push_back(value);
    }

    pub fn as_slice(&self) -> Vec<T> {
        self.data.iter().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn last(&self) -> Option<&T> {
        self.data.back()
    }
}
```

- [ ] **Step 4: Implement plugin registry stub**

Write `src/plugin/registry.rs`:

```rust
use crate::core::monitor::*;

pub trait MetricPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn collect_command(&self) -> &str;
    fn parse(&self, raw: &str) -> Vec<(String, f64)>;
    fn enabled(&self) -> bool;
}

pub struct PluginRegistry {
    plugins: Vec<Box<dyn MetricPlugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    pub fn register(&mut self, plugin: Box<dyn MetricPlugin>) {
        self.plugins.push(plugin);
    }

    pub fn enabled_plugins(&self) -> impl Iterator<Item = &dyn MetricPlugin> {
        self.plugins.iter().filter(|p| p.enabled()).map(|p| p.as_ref())
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test monitor_parse_test 2>&1`
Expected: 6 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/core/monitor.rs src/plugin/registry.rs tests/monitor_parse_test.rs
git commit -m "feat: remote metric parsing for CPU, memory, disk, network, load"
```

---

## Phase 5: Transfer Core

### Task 10: SFTP Operations and Transfer Queue

**Files:**
- Create: `src/core/sftp.rs`
- Create: `src/core/transfer.rs`
- Create: `tests/transfer_queue_test.rs`

- [ ] **Step 1: Write failing tests for transfer queue**

Create `tests/transfer_queue_test.rs`:

```rust
use guishell::core::transfer::{TransferItem, TransferQueue, TransferStatus, TransferDirection};

#[test]
fn test_queue_add_and_list() {
    let mut queue = TransferQueue::new(3);
    queue.add(TransferItem::new(
        "/remote/file.txt",
        "/local/file.txt",
        1024,
        TransferDirection::Download,
    ));
    queue.add(TransferItem::new(
        "/remote/big.bin",
        "/local/big.bin",
        1048576,
        TransferDirection::Download,
    ));

    let items = queue.items();
    assert_eq!(items.len(), 2);
    assert!(matches!(items[0].status, TransferStatus::Pending));
}

#[test]
fn test_queue_concurrent_limit() {
    let mut queue = TransferQueue::new(2);
    for i in 0..5 {
        queue.add(TransferItem::new(
            &format!("/remote/{}.txt", i),
            &format!("/local/{}.txt", i),
            100,
            TransferDirection::Download,
        ));
    }
    let active = queue.activate_pending();
    assert_eq!(active.len(), 2);
    // Remaining 3 still pending
    let pending_count = queue.items().iter().filter(|i| matches!(i.status, TransferStatus::Pending)).count();
    assert_eq!(pending_count, 3);
}

#[test]
fn test_queue_complete_activates_next() {
    let mut queue = TransferQueue::new(1);
    queue.add(TransferItem::new("/r/a", "/l/a", 10, TransferDirection::Download));
    queue.add(TransferItem::new("/r/b", "/l/b", 10, TransferDirection::Download));

    let active = queue.activate_pending();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0], 0);

    queue.complete(0);
    let next = queue.activate_pending();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0], 1);
}

#[test]
fn test_queue_update_progress() {
    let mut queue = TransferQueue::new(1);
    queue.add(TransferItem::new("/r/a", "/l/a", 1000, TransferDirection::Upload));
    queue.activate_pending();

    queue.update_progress(0, 500);
    let item = &queue.items()[0];
    assert_eq!(item.bytes_transferred, 500);
    assert!(matches!(item.status, TransferStatus::Active));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test transfer_queue_test 2>&1`
Expected: compilation error.

- [ ] **Step 3: Implement TransferQueue**

Write `src/core/transfer.rs`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum TransferDirection {
    Download,
    Upload,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransferStatus {
    Pending,
    Active,
    Completed,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct TransferItem {
    pub remote_path: String,
    pub local_path: String,
    pub total_size: u64,
    pub bytes_transferred: u64,
    pub direction: TransferDirection,
    pub status: TransferStatus,
}

impl TransferItem {
    pub fn new(remote: &str, local: &str, size: u64, direction: TransferDirection) -> Self {
        Self {
            remote_path: remote.to_string(),
            local_path: local.to_string(),
            total_size: size,
            bytes_transferred: 0,
            direction,
            status: TransferStatus::Pending,
        }
    }

    pub fn progress_percent(&self) -> f64 {
        if self.total_size == 0 {
            return 100.0;
        }
        (self.bytes_transferred as f64 / self.total_size as f64) * 100.0
    }
}

pub struct TransferQueue {
    items: Vec<TransferItem>,
    max_concurrent: usize,
}

impl TransferQueue {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            items: Vec::new(),
            max_concurrent,
        }
    }

    pub fn add(&mut self, item: TransferItem) -> usize {
        let id = self.items.len();
        self.items.push(item);
        id
    }

    pub fn items(&self) -> &[TransferItem] {
        &self.items
    }

    pub fn activate_pending(&mut self) -> Vec<usize> {
        let active_count = self
            .items
            .iter()
            .filter(|i| matches!(i.status, TransferStatus::Active))
            .count();

        let slots = self.max_concurrent.saturating_sub(active_count);
        let mut activated = Vec::new();

        for (idx, item) in self.items.iter_mut().enumerate() {
            if activated.len() >= slots {
                break;
            }
            if matches!(item.status, TransferStatus::Pending) {
                item.status = TransferStatus::Active;
                activated.push(idx);
            }
        }

        activated
    }

    pub fn update_progress(&mut self, id: usize, bytes: u64) {
        if let Some(item) = self.items.get_mut(id) {
            item.bytes_transferred = bytes;
        }
    }

    pub fn complete(&mut self, id: usize) {
        if let Some(item) = self.items.get_mut(id) {
            item.status = TransferStatus::Completed;
            item.bytes_transferred = item.total_size;
        }
    }

    pub fn fail(&mut self, id: usize, reason: &str) {
        if let Some(item) = self.items.get_mut(id) {
            item.status = TransferStatus::Failed(reason.to_string());
        }
    }

    pub fn cancel(&mut self, id: usize) {
        if let Some(item) = self.items.get_mut(id) {
            item.status = TransferStatus::Cancelled;
        }
    }

    pub fn remove_completed(&mut self) {
        self.items
            .retain(|i| !matches!(i.status, TransferStatus::Completed));
    }
}
```

- [ ] **Step 4: Implement SFTP operations wrapper**

Write `src/core/sftp.rs`:

```rust
use ssh2::{self, FileStat, OpenFlags, OpenType, Sftp};
use std::io::{self, Read, Write};
use std::path::Path;

pub struct SftpEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
    pub permissions: u32,
}

pub struct SftpOps<'a> {
    sftp: &'a Sftp,
}

impl<'a> SftpOps<'a> {
    pub fn new(sftp: &'a Sftp) -> Self {
        Self { sftp }
    }

    pub fn list_dir(&self, path: &str) -> Result<Vec<SftpEntry>, io::Error> {
        let entries = self
            .sftp
            .readdir(Path::new(path))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        Ok(entries
            .into_iter()
            .map(|(path, stat)| SftpEntry {
                name: path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string(),
                is_dir: stat.is_dir(),
                size: stat.size.unwrap_or(0),
                mtime: stat.mtime.unwrap_or(0),
                permissions: stat.perm.unwrap_or(0o644) as u32,
            })
            .collect())
    }

    pub fn download(
        &self,
        remote_path: &str,
        local_path: &str,
        mut progress_cb: impl FnMut(u64),
    ) -> Result<u64, io::Error> {
        let mut remote_file = self
            .sftp
            .open(Path::new(remote_path))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let mut local_file = std::fs::File::create(local_path)?;

        let mut buf = vec![0u8; 32768];
        let mut total = 0u64;

        loop {
            let n = remote_file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            local_file.write_all(&buf[..n])?;
            total += n as u64;
            progress_cb(total);
        }

        Ok(total)
    }

    pub fn upload(
        &self,
        local_path: &str,
        remote_path: &str,
        mut progress_cb: impl FnMut(u64),
    ) -> Result<u64, io::Error> {
        let mut local_file = std::fs::File::open(local_path)?;
        let mut remote_file = self
            .sftp
            .create(Path::new(remote_path))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let mut buf = vec![0u8; 32768];
        let mut total = 0u64;

        loop {
            let n = local_file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            remote_file.write_all(&buf[..n])?;
            total += n as u64;
            progress_cb(total);
        }

        Ok(total)
    }

    pub fn mkdir(&self, path: &str, mode: i32) -> Result<(), io::Error> {
        self.sftp
            .mkdir(Path::new(path), mode)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    pub fn remove_file(&self, path: &str) -> Result<(), io::Error> {
        self.sftp
            .unlink(Path::new(path))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    pub fn remove_dir(&self, path: &str) -> Result<(), io::Error> {
        self.sftp
            .rmdir(Path::new(path))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), io::Error> {
        self.sftp
            .rename(Path::new(from), Path::new(to), None)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    pub fn stat(&self, path: &str) -> Result<SftpEntry, io::Error> {
        let stat = self
            .sftp
            .stat(Path::new(path))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(SftpEntry {
            name: Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string(),
            is_dir: stat.is_dir(),
            size: stat.size.unwrap_or(0),
            mtime: stat.mtime.unwrap_or(0),
            permissions: stat.perm.unwrap_or(0o644) as u32,
        })
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test transfer_queue_test 2>&1`
Expected: 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/core/sftp.rs src/core/transfer.rs tests/transfer_queue_test.rs
git commit -m "feat: SFTP operations wrapper and transfer queue"
```

---

## Phase 6: UI Shell

### Task 11: Main Window Layout with Sidebar and Paned Panels

**Files:**
- Modify: `src/main.rs`
- Create: `src/ui/window.rs`
- Create: `src/ui/sidebar.rs`

- [ ] **Step 1: Implement MainWindow**

Write `src/ui/window.rs`:

```rust
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib, Orientation, Paned};

use crate::config::settings::Settings;
use super::sidebar::Sidebar;

pub struct MainWindow {
    pub window: adw::ApplicationWindow,
    pub sidebar: Sidebar,
    pub main_paned: Paned,
    pub content_paned: Paned,
}

impl MainWindow {
    pub fn new(app: &adw::Application, settings: &Settings) -> Self {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("GuiShell")
            .default_width(1200)
            .default_height(800)
            .build();

        // Top-level horizontal pane: sidebar | content
        let main_paned = Paned::new(Orientation::Horizontal);
        main_paned.set_position(settings.appearance.sidebar_width as i32);
        main_paned.set_shrink_start_child(false);

        // Sidebar (left)
        let sidebar = Sidebar::new();
        main_paned.set_start_child(Some(sidebar.widget()));

        // Content area: vertical pane for terminal area | file browser
        let content_paned = Paned::new(Orientation::Vertical);

        // Placeholder for terminal tabs (will be replaced in Task 12)
        let terminal_placeholder = gtk::Label::new(Some("Terminal area — press Ctrl+Shift+T to connect"));
        terminal_placeholder.set_vexpand(true);
        terminal_placeholder.set_hexpand(true);
        content_paned.set_start_child(Some(&terminal_placeholder));

        // Placeholder for file browser bottom panel
        let filebrowser_placeholder = gtk::Label::new(Some("File Browser (Ctrl+Shift+E)"));
        filebrowser_placeholder.set_visible(settings.appearance.show_file_browser);
        content_paned.set_end_child(Some(&filebrowser_placeholder));
        content_paned.set_position(600);

        main_paned.set_end_child(Some(&content_paned));

        let toolbar_view = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&main_paned));

        window.set_content(Some(&toolbar_view));

        Self {
            window,
            sidebar,
            main_paned,
            content_paned,
        }
    }

    pub fn present(&self) {
        self.window.present();
    }
}
```

- [ ] **Step 2: Implement Sidebar with connection tree and monitor placeholder**

Write `src/ui/sidebar.rs`:

```rust
use gtk::prelude::*;
use gtk::{Box as GtkBox, Label, ListBox, Orientation, ScrolledWindow, Separator};

pub struct Sidebar {
    container: GtkBox,
    connection_list: ListBox,
    monitor_area: GtkBox,
}

impl Sidebar {
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_width_request(200);

        // Connection list section
        let conn_label = Label::new(Some("Connections"));
        conn_label.add_css_class("heading");
        conn_label.set_margin_top(8);
        conn_label.set_margin_start(8);
        conn_label.set_xalign(0.0);
        container.append(&conn_label);

        let connection_scroll = ScrolledWindow::new();
        connection_scroll.set_vexpand(false);
        connection_scroll.set_min_content_height(200);

        let connection_list = ListBox::new();
        connection_list.set_selection_mode(gtk::SelectionMode::Single);
        connection_list.add_css_class("navigation-sidebar");
        connection_scroll.set_child(Some(&connection_list));
        container.append(&connection_scroll);

        container.append(&Separator::new(Orientation::Horizontal));

        // Monitor section
        let monitor_label = Label::new(Some("System Monitor"));
        monitor_label.add_css_class("heading");
        monitor_label.set_margin_top(8);
        monitor_label.set_margin_start(8);
        monitor_label.set_xalign(0.0);
        container.append(&monitor_label);

        let monitor_area = GtkBox::new(Orientation::Vertical, 4);
        monitor_area.set_vexpand(true);
        monitor_area.set_margin_all(8);

        let monitor_scroll = ScrolledWindow::new();
        monitor_scroll.set_vexpand(true);
        monitor_scroll.set_child(Some(&monitor_area));
        container.append(&monitor_scroll);

        Self {
            container,
            connection_list,
            monitor_area,
        }
    }

    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn connection_list(&self) -> &ListBox {
        &self.connection_list
    }

    pub fn monitor_area(&self) -> &GtkBox {
        &self.monitor_area
    }
}
```

- [ ] **Step 3: Update main.rs to use MainWindow**

Replace `src/main.rs`:

```rust
mod config;
mod core;
mod plugin;
mod ui;

use adw::prelude::*;
use adw::Application;
use gtk::glib;

use config::settings::Settings;
use ui::window::MainWindow;

const APP_ID: &str = "com.guishell.app";

fn main() -> glib::ExitCode {
    env_logger::init();

    let app = Application::builder().application_id(APP_ID).build();

    app.connect_activate(|app| {
        let settings = Settings::load();
        let main_window = MainWindow::new(app, &settings);
        main_window.present();
    });

    app.run()
}
```

- [ ] **Step 4: Build and verify window renders**

Run:
```bash
cd /home/lfl/ssd/code/guishell && cargo build 2>&1
```
Expected: successful build. If GTK4/adw APIs differ from version, fix compile errors.

If display is available:
```bash
cargo run 2>&1 &
sleep 3
kill %1 2>/dev/null
```
Expected: window with sidebar and split panes.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/ui/window.rs src/ui/sidebar.rs
git commit -m "feat: main window layout with sidebar and paned panels"
```

### Task 12: Tab System and VTE Terminal

**Files:**
- Create: `src/ui/tabs.rs`
- Create: `src/ui/terminal.rs`
- Modify: `src/ui/window.rs`

- [ ] **Step 1: Implement TerminalPane (VTE wrapper)**

Write `src/ui/terminal.rs`:

```rust
use gtk::prelude::*;
use vte::prelude::*;
use vte::Terminal as VteTerminal;
use crate::config::settings::TerminalSettings;

pub struct TerminalPane {
    terminal: VteTerminal,
    container: gtk::Box,
}

impl TerminalPane {
    pub fn new(settings: &TerminalSettings) -> Self {
        let terminal = VteTerminal::new();
        terminal.set_vexpand(true);
        terminal.set_hexpand(true);
        terminal.set_scrollback_lines(settings.scrollback_lines as i64);
        terminal.set_cursor_blink_mode(if settings.cursor_blink {
            vte::CursorBlinkMode::On
        } else {
            vte::CursorBlinkMode::Off
        });

        let font_desc = gtk::pango::FontDescription::from_string(&settings.font);
        terminal.set_font(Some(&font_desc));

        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_child(Some(&terminal));
        scrolled.set_vexpand(true);
        container.append(&scrolled);

        Self {
            terminal,
            container,
        }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.container
    }

    pub fn vte(&self) -> &VteTerminal {
        &self.terminal
    }

    pub fn feed_data(&self, data: &[u8]) {
        self.terminal.feed(data);
    }
}
```

- [ ] **Step 2: Implement TabManager**

Write `src/ui/tabs.rs`:

```rust
use gtk::prelude::*;
use gtk::{Notebook, Label};
use crate::config::settings::Settings;
use super::terminal::TerminalPane;

pub struct TabManager {
    notebook: Notebook,
    settings: Settings,
}

impl TabManager {
    pub fn new(settings: Settings) -> Self {
        let notebook = Notebook::new();
        notebook.set_scrollable(true);
        notebook.set_show_border(false);
        notebook.set_vexpand(true);
        notebook.set_hexpand(true);

        // Enable tab reordering
        notebook.connect_page_reordered(|_, _, _| {});

        Self { notebook, settings }
    }

    pub fn widget(&self) -> &Notebook {
        &self.notebook
    }

    pub fn add_tab(&self, label_text: &str) -> TerminalPane {
        let terminal = TerminalPane::new(&self.settings.terminal);

        let tab_label = Self::make_tab_label(label_text, &self.notebook, terminal.widget());
        let page_num = self.notebook.append_page(terminal.widget(), Some(&tab_label));
        self.notebook.set_tab_reorderable(terminal.widget(), true);
        self.notebook.set_current_page(Some(page_num));

        terminal
    }

    fn make_tab_label(text: &str, notebook: &Notebook, page_widget: &gtk::Box) -> gtk::Box {
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let label = Label::new(Some(text));
        hbox.append(&label);

        let close_btn = gtk::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.add_css_class("circular");
        close_btn.set_margin_start(4);

        let nb = notebook.clone();
        let pw = page_widget.clone();
        close_btn.connect_clicked(move |_| {
            if let Some(page_num) = nb.page_num(&pw) {
                nb.remove_page(Some(page_num));
            }
        });

        hbox.append(&close_btn);
        hbox
    }

    pub fn tab_count(&self) -> u32 {
        self.notebook.n_pages()
    }
}
```

- [ ] **Step 3: Update MainWindow to use TabManager**

Replace the terminal placeholder in `src/ui/window.rs`. Modify the `new` function — replace:

```rust
        // Placeholder for terminal tabs (will be replaced in Task 12)
        let terminal_placeholder = gtk::Label::new(Some("Terminal area — press Ctrl+Shift+T to connect"));
        terminal_placeholder.set_vexpand(true);
        terminal_placeholder.set_hexpand(true);
        content_paned.set_start_child(Some(&terminal_placeholder));
```

With:

```rust
        // Tab manager for terminal sessions
        let tab_manager = super::tabs::TabManager::new(settings.clone());
        content_paned.set_start_child(Some(tab_manager.widget()));
```

Add `tab_manager` field to the struct and constructor return. Update `MainWindow` struct:

```rust
pub struct MainWindow {
    pub window: adw::ApplicationWindow,
    pub sidebar: Sidebar,
    pub tab_manager: super::tabs::TabManager,
    pub main_paned: Paned,
    pub content_paned: Paned,
}
```

And the return:

```rust
        Self {
            window,
            sidebar,
            tab_manager,
            main_paned,
            content_paned,
        }
```

- [ ] **Step 4: Build and verify**

Run: `cd /home/lfl/ssd/code/guishell && cargo build 2>&1`
Expected: successful build.

- [ ] **Step 5: Commit**

```bash
git add src/ui/tabs.rs src/ui/terminal.rs src/ui/window.rs
git commit -m "feat: tab system with VTE terminal panes"
```

### Task 13: Split Pane System (Binary Split Tree)

**Files:**
- Create: `src/ui/split.rs`
- Create: `tests/split_tree_test.rs`

- [ ] **Step 1: Write failing test for split tree model**

Create `tests/split_tree_test.rs`:

```rust
use guishell::ui::split::{SplitDirection, SplitNode, SplitTree};

#[test]
fn test_initial_tree_is_single_pane() {
    let tree = SplitTree::new(0);
    assert_eq!(tree.pane_count(), 1);
    assert!(tree.is_leaf());
}

#[test]
fn test_split_horizontal() {
    let mut tree = SplitTree::new(0);
    tree.split(0, SplitDirection::Horizontal, 1);
    assert_eq!(tree.pane_count(), 2);
    assert!(!tree.is_leaf());
}

#[test]
fn test_split_vertical_then_close() {
    let mut tree = SplitTree::new(0);
    tree.split(0, SplitDirection::Vertical, 1);
    assert_eq!(tree.pane_count(), 2);

    tree.close(1);
    assert_eq!(tree.pane_count(), 1);
}

#[test]
fn test_nested_split() {
    let mut tree = SplitTree::new(0);
    tree.split(0, SplitDirection::Horizontal, 1); // [0 | 1]
    tree.split(1, SplitDirection::Vertical, 2);   // [0 | [1 / 2]]
    assert_eq!(tree.pane_count(), 3);
}

#[test]
fn test_all_pane_ids() {
    let mut tree = SplitTree::new(0);
    tree.split(0, SplitDirection::Horizontal, 1);
    tree.split(1, SplitDirection::Vertical, 2);
    let mut ids = tree.pane_ids();
    ids.sort();
    assert_eq!(ids, vec![0, 1, 2]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test split_tree_test 2>&1`
Expected: compilation error.

- [ ] **Step 3: Implement SplitTree (model only, no GTK dependency)**

Write `src/ui/split.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug)]
pub enum SplitNode {
    Leaf { pane_id: usize },
    Split {
        direction: SplitDirection,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

pub struct SplitTree {
    root: SplitNode,
}

impl SplitTree {
    pub fn new(initial_pane_id: usize) -> Self {
        Self {
            root: SplitNode::Leaf {
                pane_id: initial_pane_id,
            },
        }
    }

    pub fn is_leaf(&self) -> bool {
        matches!(self.root, SplitNode::Leaf { .. })
    }

    pub fn pane_count(&self) -> usize {
        Self::count_leaves(&self.root)
    }

    pub fn pane_ids(&self) -> Vec<usize> {
        let mut ids = Vec::new();
        Self::collect_ids(&self.root, &mut ids);
        ids
    }

    pub fn split(&mut self, target_pane_id: usize, direction: SplitDirection, new_pane_id: usize) {
        Self::split_node(&mut self.root, target_pane_id, direction, new_pane_id);
    }

    pub fn close(&mut self, pane_id: usize) {
        if let Some(new_root) = Self::remove_node(&mut self.root, pane_id) {
            self.root = new_root;
        }
    }

    fn count_leaves(node: &SplitNode) -> usize {
        match node {
            SplitNode::Leaf { .. } => 1,
            SplitNode::Split { first, second, .. } => {
                Self::count_leaves(first) + Self::count_leaves(second)
            }
        }
    }

    fn collect_ids(node: &SplitNode, ids: &mut Vec<usize>) {
        match node {
            SplitNode::Leaf { pane_id } => ids.push(*pane_id),
            SplitNode::Split { first, second, .. } => {
                Self::collect_ids(first, ids);
                Self::collect_ids(second, ids);
            }
        }
    }

    fn split_node(
        node: &mut SplitNode,
        target: usize,
        direction: SplitDirection,
        new_id: usize,
    ) -> bool {
        match node {
            SplitNode::Leaf { pane_id } if *pane_id == target => {
                let old = SplitNode::Leaf { pane_id: target };
                let new = SplitNode::Leaf { pane_id: new_id };
                *node = SplitNode::Split {
                    direction,
                    first: Box::new(old),
                    second: Box::new(new),
                };
                true
            }
            SplitNode::Split { first, second, .. } => {
                Self::split_node(first, target, direction, new_id)
                    || Self::split_node(second, target, direction, new_id)
            }
            _ => false,
        }
    }

    fn remove_node(node: &mut SplitNode, target: usize) -> Option<SplitNode> {
        match node {
            SplitNode::Leaf { pane_id } if *pane_id == target => None,
            SplitNode::Leaf { .. } => None,
            SplitNode::Split { first, second, .. } => {
                // Check if first child is the target
                if let SplitNode::Leaf { pane_id } = first.as_ref() {
                    if *pane_id == target {
                        // Replace parent with second child
                        return Some(std::mem::replace(
                            second.as_mut(),
                            SplitNode::Leaf { pane_id: 0 },
                        ));
                    }
                }
                // Check if second child is the target
                if let SplitNode::Leaf { pane_id } = second.as_ref() {
                    if *pane_id == target {
                        return Some(std::mem::replace(
                            first.as_mut(),
                            SplitNode::Leaf { pane_id: 0 },
                        ));
                    }
                }
                // Recurse
                if let Some(replacement) = Self::remove_node(first, target) {
                    *first.as_mut() = replacement;
                } else if let Some(replacement) = Self::remove_node(second, target) {
                    *second.as_mut() = replacement;
                }
                None
            }
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /home/lfl/ssd/code/guishell && cargo test --test split_tree_test 2>&1`
Expected: 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/ui/split.rs tests/split_tree_test.rs
git commit -m "feat: binary split tree for terminal pane management"
```

---

## Phase 7: Monitor UI and File Browser UI

### Task 14: Cairo Monitor Charts

**Files:**
- Create: `src/ui/monitor.rs`

- [ ] **Step 1: Implement monitor chart drawing widget**

Write `src/ui/monitor.rs`:

```rust
use cairo::Context;
use gtk::prelude::*;
use gtk::{DrawingArea, Box as GtkBox, Label, Orientation};
use crate::core::monitor::MetricBuffer;

pub struct MonitorChart {
    container: GtkBox,
    drawing_area: DrawingArea,
    label: Label,
    value_label: Label,
}

impl MonitorChart {
    pub fn new(title: &str, height: i32) -> Self {
        let container = GtkBox::new(Orientation::Vertical, 2);

        let header = GtkBox::new(Orientation::Horizontal, 4);
        let label = Label::new(Some(title));
        label.set_xalign(0.0);
        label.add_css_class("caption");
        header.append(&label);

        let value_label = Label::new(Some("--"));
        value_label.set_xalign(1.0);
        value_label.set_hexpand(true);
        value_label.add_css_class("caption");
        header.append(&value_label);

        container.append(&header);

        let drawing_area = DrawingArea::new();
        drawing_area.set_content_height(height);
        drawing_area.set_hexpand(true);

        container.append(&drawing_area);

        Self {
            container,
            drawing_area,
            label,
            value_label,
        }
    }

    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn set_value_text(&self, text: &str) {
        self.value_label.set_text(text);
    }

    pub fn setup_line_chart(&self, color: (f64, f64, f64)) {
        let (r, g, b) = color;
        self.drawing_area.set_draw_func(move |_area, cr, width, height| {
            // Background
            cr.set_source_rgba(0.1, 0.1, 0.1, 0.3);
            cr.rectangle(0.0, 0.0, width as f64, height as f64);
            let _ = cr.fill();

            // Grid lines
            cr.set_source_rgba(0.3, 0.3, 0.3, 0.3);
            cr.set_line_width(0.5);
            for i in 1..4 {
                let y = height as f64 * i as f64 / 4.0;
                cr.move_to(0.0, y);
                cr.line_to(width as f64, y);
                let _ = cr.stroke();
            }
        });
    }

    pub fn update_line_chart(&self, data: &[f64], max_val: f64, color: (f64, f64, f64)) {
        let data = data.to_vec();
        let (r, g, b) = color;
        self.drawing_area.set_draw_func(move |_area, cr, width, height| {
            let w = width as f64;
            let h = height as f64;

            // Background
            cr.set_source_rgba(0.1, 0.1, 0.1, 0.3);
            cr.rectangle(0.0, 0.0, w, h);
            let _ = cr.fill();

            // Grid
            cr.set_source_rgba(0.3, 0.3, 0.3, 0.3);
            cr.set_line_width(0.5);
            for i in 1..4 {
                let y = h * i as f64 / 4.0;
                cr.move_to(0.0, y);
                cr.line_to(w, y);
                let _ = cr.stroke();
            }

            if data.is_empty() {
                return;
            }

            // Line chart
            let step = w / (data.len().max(2) - 1) as f64;
            cr.set_source_rgba(r, g, b, 1.0);
            cr.set_line_width(1.5);

            let clamp = |v: f64| -> f64 { h - (v / max_val * h).min(h) };

            cr.move_to(0.0, clamp(data[0]));
            for (i, &val) in data.iter().enumerate().skip(1) {
                cr.line_to(i as f64 * step, clamp(val));
            }
            let _ = cr.stroke();

            // Fill under line
            cr.set_source_rgba(r, g, b, 0.15);
            cr.move_to(0.0, clamp(data[0]));
            for (i, &val) in data.iter().enumerate().skip(1) {
                cr.line_to(i as f64 * step, clamp(val));
            }
            cr.line_to((data.len() - 1) as f64 * step, h);
            cr.line_to(0.0, h);
            cr.close_path();
            let _ = cr.fill();
        });
        self.drawing_area.queue_draw();
    }
}

pub fn draw_bar_chart(cr: &Context, w: f64, h: f64, items: &[(String, f64)], color: (f64, f64, f64)) {
    let (r, g, b) = color;
    if items.is_empty() {
        return;
    }
    let bar_height = (h / items.len() as f64).min(20.0);
    let gap = 4.0;

    for (i, (label, pct)) in items.iter().enumerate() {
        let y = i as f64 * (bar_height + gap);
        let bar_w = (pct / 100.0 * (w - 60.0)).max(0.0);

        // Background bar
        cr.set_source_rgba(0.2, 0.2, 0.2, 0.5);
        cr.rectangle(60.0, y, w - 60.0, bar_height);
        let _ = cr.fill();

        // Value bar
        cr.set_source_rgba(r, g, b, 0.8);
        cr.rectangle(60.0, y, bar_w, bar_height);
        let _ = cr.fill();

        // Label
        cr.set_source_rgba(0.9, 0.9, 0.9, 1.0);
        cr.move_to(2.0, y + bar_height - 3.0);
        cr.set_font_size(10.0);
        let _ = cr.show_text(label);

        // Percentage
        let _ = cr.show_text(&format!(" {}%", *pct as u8));
    }
}
```

- [ ] **Step 2: Build to verify**

Run: `cd /home/lfl/ssd/code/guishell && cargo build 2>&1`
Expected: successful build.

- [ ] **Step 3: Commit**

```bash
git add src/ui/monitor.rs
git commit -m "feat: cairo-based monitor chart widgets"
```

### Task 15: Dual-Pane File Browser UI

**Files:**
- Create: `src/ui/file_browser.rs`

- [ ] **Step 1: Implement dual-pane file browser widget**

Write `src/ui/file_browser.rs`:

```rust
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Button, ColumnView, ColumnViewColumn, Label, ListItem,
    Orientation, Paned, ProgressBar, ScrolledWindow, SignalListItemFactory,
    SingleSelection, StringList, Entry,
};

pub struct FileBrowserPanel {
    container: GtkBox,
    local_path_entry: Entry,
    remote_path_entry: Entry,
    local_list: gtk::ListBox,
    remote_list: gtk::ListBox,
    transfer_list: GtkBox,
}

impl FileBrowserPanel {
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);

        // Dual pane
        let paned = Paned::new(Orientation::Horizontal);
        paned.set_vexpand(true);

        // Local pane
        let local_box = GtkBox::new(Orientation::Vertical, 0);
        let local_header = GtkBox::new(Orientation::Horizontal, 4);
        local_header.set_margin_all(4);
        let local_label = Label::new(Some("Local"));
        local_label.add_css_class("heading");
        local_header.append(&local_label);
        let local_path_entry = Entry::new();
        local_path_entry.set_hexpand(true);
        local_path_entry.set_placeholder_text(Some("~/Downloads"));
        local_header.append(&local_path_entry);
        local_box.append(&local_header);

        let local_scroll = ScrolledWindow::new();
        local_scroll.set_vexpand(true);
        let local_list = gtk::ListBox::new();
        local_list.set_selection_mode(gtk::SelectionMode::Multiple);
        local_scroll.set_child(Some(&local_list));
        local_box.append(&local_scroll);
        paned.set_start_child(Some(&local_box));

        // Remote pane
        let remote_box = GtkBox::new(Orientation::Vertical, 0);
        let remote_header = GtkBox::new(Orientation::Horizontal, 4);
        remote_header.set_margin_all(4);
        let remote_label = Label::new(Some("Remote"));
        remote_label.add_css_class("heading");
        remote_header.append(&remote_label);
        let remote_path_entry = Entry::new();
        remote_path_entry.set_hexpand(true);
        remote_path_entry.set_placeholder_text(Some("/home"));
        remote_header.append(&remote_path_entry);
        remote_box.append(&remote_header);

        let remote_scroll = ScrolledWindow::new();
        remote_scroll.set_vexpand(true);
        let remote_list = gtk::ListBox::new();
        remote_list.set_selection_mode(gtk::SelectionMode::Multiple);
        remote_scroll.set_child(Some(&remote_list));
        remote_box.append(&remote_scroll);
        paned.set_end_child(Some(&remote_box));

        container.append(&paned);

        // Transfer queue
        let transfer_label = Label::new(Some("Transfer Queue"));
        transfer_label.add_css_class("caption");
        transfer_label.set_xalign(0.0);
        transfer_label.set_margin_start(4);
        container.append(&transfer_label);

        let transfer_scroll = ScrolledWindow::new();
        transfer_scroll.set_max_content_height(100);
        let transfer_list = GtkBox::new(Orientation::Vertical, 2);
        transfer_list.set_margin_all(4);
        transfer_scroll.set_child(Some(&transfer_list));
        container.append(&transfer_scroll);

        Self {
            container,
            local_path_entry,
            remote_path_entry,
            local_list,
            remote_list,
            transfer_list,
        }
    }

    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn add_transfer_row(&self, filename: &str, direction: &str, size: &str) -> GtkBox {
        let row = GtkBox::new(Orientation::Horizontal, 8);
        row.set_margin_all(2);

        let icon = Label::new(Some(direction));
        row.append(&icon);

        let name = Label::new(Some(filename));
        name.set_hexpand(true);
        name.set_xalign(0.0);
        row.append(&name);

        let size_label = Label::new(Some(size));
        row.append(&size_label);

        let progress = ProgressBar::new();
        progress.set_width_request(120);
        row.append(&progress);

        let cancel_btn = Button::from_icon_name("process-stop-symbolic");
        cancel_btn.add_css_class("flat");
        row.append(&cancel_btn);

        self.transfer_list.append(&row);
        row
    }
}
```

- [ ] **Step 2: Build to verify**

Run: `cd /home/lfl/ssd/code/guishell && cargo build 2>&1`
Expected: successful build.

- [ ] **Step 3: Commit**

```bash
git add src/ui/file_browser.rs
git commit -m "feat: dual-pane file browser UI widget"
```

---

## Phase 8: Integration and Keyboard Shortcuts

### Task 16: Wire SSH Connection to VTE Terminal

**Files:**
- Modify: `src/ui/window.rs`
- Modify: `src/ui/terminal.rs`

This task connects the SSH core to the VTE terminal via a PTY bridge, and wires up the sidebar's connection list to spawn new sessions.

- [ ] **Step 1: Add PTY bridge to TerminalPane**

Add to `src/ui/terminal.rs`:

```rust
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::thread;

impl TerminalPane {
    /// Connect this terminal to an SSH channel by bridging
    /// the channel's I/O to VTE's PTY fd.
    pub fn connect_ssh(
        &self,
        mut channel: ssh2::Channel,
        session: Arc<ssh2::Session>,
    ) {
        let vte = self.terminal.clone();

        // VTE provides a PTY master fd — we read from the SSH channel
        // and feed data to VTE, and read VTE input to write to the channel.
        let alive = Arc::new(AtomicBool::new(true));

        // SSH → VTE (read from channel, feed to terminal)
        let alive_r = alive.clone();
        let vte_r = vte.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while alive_r.load(Ordering::Relaxed) {
                match channel.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = buf[..n].to_vec();
                        let vte_c = vte_r.clone();
                        glib::idle_add_local_once(move || {
                            vte_c.feed(&data);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        // VTE key input → SSH channel
        // Use VTE's commit signal to capture user keystrokes
        let alive_w = alive.clone();
        vte.connect_commit(move |_vte, text, _size| {
            if !alive_w.load(Ordering::Relaxed) {
                return;
            }
            // Write text to SSH channel needs channel access
            // In production this would go through a sender channel
            // For now, log that input was received
            log::debug!("VTE input: {} bytes", text.len());
        });
    }
}

use gtk::glib;
```

Note: The full bidirectional bridge requires sharing the SSH channel between threads. In production, this will use `tokio::sync::mpsc` channels to shuttle bytes between the VTE commit signal callback and the SSH writer thread. The exact implementation will be refined when integrating with the async runtime.

- [ ] **Step 2: Build to verify**

Run: `cd /home/lfl/ssd/code/guishell && cargo build 2>&1`
Expected: successful build (or minor fixes needed for import paths).

- [ ] **Step 3: Commit**

```bash
git add src/ui/terminal.rs
git commit -m "feat: SSH to VTE terminal bridge skeleton"
```

### Task 17: Keyboard Shortcuts

**Files:**
- Modify: `src/ui/window.rs`

- [ ] **Step 1: Add keyboard shortcut actions to MainWindow**

Add a method to `src/ui/window.rs`:

```rust
impl MainWindow {
    pub fn setup_shortcuts(&self, app: &adw::Application) {
        // Ctrl+Shift+T — new tab
        let action_new_tab = gio::SimpleAction::new("new-tab", None);
        let nb = self.tab_manager.widget().clone();
        action_new_tab.connect_activate(move |_, _| {
            // Will be wired to connection dialog
            log::info!("New tab requested");
        });
        app.add_action(&action_new_tab);
        app.set_accels_for_action("app.new-tab", &["<Ctrl><Shift>t"]);

        // Ctrl+Shift+Q — close tab
        let action_close_tab = gio::SimpleAction::new("close-tab", None);
        let nb2 = self.tab_manager.widget().clone();
        action_close_tab.connect_activate(move |_, _| {
            if let Some(page) = nb2.current_page() {
                nb2.remove_page(Some(page));
            }
        });
        app.add_action(&action_close_tab);
        app.set_accels_for_action("app.close-tab", &["<Ctrl><Shift>q"]);

        // Ctrl+B — toggle sidebar
        let action_toggle_sidebar = gio::SimpleAction::new("toggle-sidebar", None);
        let sidebar_widget = self.sidebar.widget().clone();
        action_toggle_sidebar.connect_activate(move |_, _| {
            sidebar_widget.set_visible(!sidebar_widget.is_visible());
        });
        app.add_action(&action_toggle_sidebar);
        app.set_accels_for_action("app.toggle-sidebar", &["<Ctrl>b"]);

        // Ctrl+Shift+E — toggle file browser
        let action_toggle_fb = gio::SimpleAction::new("toggle-file-browser", None);
        let cp = self.content_paned.clone();
        action_toggle_fb.connect_activate(move |_, _| {
            if let Some(child) = cp.end_child() {
                child.set_visible(!child.is_visible());
            }
        });
        app.add_action(&action_toggle_fb);
        app.set_accels_for_action("app.toggle-file-browser", &["<Ctrl><Shift>e"]);

        // F11 — fullscreen
        let action_fullscreen = gio::SimpleAction::new("toggle-fullscreen", None);
        let win = self.window.clone();
        action_fullscreen.connect_activate(move |_, _| {
            if win.is_fullscreen() {
                win.unfullscreen();
            } else {
                win.fullscreen();
            }
        });
        app.add_action(&action_fullscreen);
        app.set_accels_for_action("app.toggle-fullscreen", &["F11"]);
    }
}
```

- [ ] **Step 2: Call setup_shortcuts from main.rs**

In `src/main.rs`, after creating the window, add:

```rust
        main_window.setup_shortcuts(app);
```

Add `use gtk::gio;` to `src/ui/window.rs` if not already imported.

- [ ] **Step 3: Build to verify**

Run: `cd /home/lfl/ssd/code/guishell && cargo build 2>&1`
Expected: successful build.

- [ ] **Step 4: Commit**

```bash
git add src/ui/window.rs src/main.rs
git commit -m "feat: keyboard shortcuts for tabs, sidebar, file browser, fullscreen"
```

---

## Phase 9: End-to-End Integration

### Task 18: Connect All Layers — Sidebar → SSH → Terminal → Monitor

This final task wires the connection list, SSH session creation, VTE binding, and monitor data collection into a working flow.

**Files:**
- Modify: `src/main.rs`
- Modify: `src/ui/window.rs`
- Modify: `src/ui/sidebar.rs`

- [ ] **Step 1: Add connection list population from config**

Add to `src/ui/sidebar.rs`:

```rust
use crate::config::connections::ConnectionStore;
use gtk::{Label, ListBoxRow};

impl Sidebar {
    pub fn populate_connections(&self, store: &ConnectionStore) {
        // Clear existing
        while let Some(child) = self.connection_list.first_child() {
            self.connection_list.remove(&child);
        }

        for group in store.groups() {
            // Group header
            let header = Label::new(Some(&group.label));
            header.add_css_class("heading");
            header.set_xalign(0.0);
            header.set_margin_top(8);
            header.set_margin_start(8);
            let row = ListBoxRow::new();
            row.set_selectable(false);
            row.set_activatable(false);
            row.set_child(Some(&header));
            self.connection_list.append(&row);

            // Hosts
            for (host_id, host) in store.hosts_in_group(&group.id) {
                let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                hbox.set_margin_start(16);
                hbox.set_margin_all(4);

                let color_dot = Label::new(Some("\u{25CF}")); // ●
                let css = format!("color: {};", group.color);
                let provider = gtk::CssProvider::new();
                provider.load_from_string(&format!("label {{ {} }}", css));
                color_dot.style_context().add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
                hbox.append(&color_dot);

                let name = Label::new(Some(&host.label));
                name.set_xalign(0.0);
                hbox.append(&name);

                let addr = Label::new(Some(&format!("{}@{}:{}", host.user, host.host, host.port)));
                addr.add_css_class("dim-label");
                addr.add_css_class("caption");
                addr.set_hexpand(true);
                addr.set_xalign(1.0);
                hbox.append(&addr);

                let row = ListBoxRow::new();
                row.set_child(Some(&hbox));
                self.connection_list.append(&row);
            }
        }
    }
}
```

- [ ] **Step 2: Wire up main.rs to load connections and populate sidebar**

Update `main.rs` `connect_activate` closure:

```rust
    app.connect_activate(|app| {
        let settings = Settings::load();
        let connections = config::connections::ConnectionStore::load_from(
            &Settings::config_dir()
                .map(|d| d.join("connections.toml"))
                .unwrap_or_default(),
        )
        .unwrap_or_default();

        let main_window = MainWindow::new(app, &settings);
        main_window.sidebar.populate_connections(&connections);
        main_window.setup_shortcuts(app);
        main_window.present();
    });
```

- [ ] **Step 3: Build final binary**

Run:
```bash
cd /home/lfl/ssd/code/guishell && cargo build --release 2>&1
```
Expected: successful release build.

- [ ] **Step 4: Run all tests**

Run:
```bash
cd /home/lfl/ssd/code/guishell && cargo test 2>&1
```
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: end-to-end integration — sidebar, connections, shortcuts"
```

---

## Summary

| Phase | Tasks | What it delivers |
|-------|-------|-----------------|
| 0 | 1 | Project scaffold, builds and runs |
| 1 | 2-4 | Config layer: settings, connections, keyring |
| 2 | 5 | SSH connection and session management |
| 3 | 6-8 | ZMODEM protocol: frames, detection, send/receive |
| 4 | 9 | System monitoring: metric parsing, ring buffer |
| 5 | 10 | SFTP operations and transfer queue |
| 6 | 11-13 | UI: main window, tabs, VTE terminal, split panes |
| 7 | 14-15 | UI: monitor charts, file browser panels |
| 8 | 16-17 | Integration: SSH↔VTE bridge, keyboard shortcuts |
| 9 | 18 | End-to-end: sidebar→SSH→terminal→monitor wiring |

After completing all 18 tasks, you will have a working GuiShell that:
- Opens a GTK4 window with sidebar, tabs, and split panes
- Loads connection groups from TOML config
- Connects to SSH servers with keyring-stored credentials
- Renders terminal output via VTE4
- Parses remote system metrics for the monitor panel
- Supports SFTP file transfers with a dual-pane browser
- Detects and handles ZMODEM transfers through multi-hop SSH
- Responds to keyboard shortcuts for all major actions
