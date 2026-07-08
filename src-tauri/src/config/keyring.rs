use std::collections::HashMap;
use crate::app_log;

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
        format!("guishell:ssh://{}@{}:{}", self.user, self.host, self.port)
    }

    pub fn credential_key(&self) -> String {
        format!("{}_{}_{}", self.user, self.host, self.port)
    }

    pub fn attributes(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        map.insert("application".to_string(), "guishell".to_string());
        map.insert("user".to_string(), self.user.clone());
        map.insert("host".to_string(), self.host.clone());
        map.insert("port".to_string(), self.port.to_string());
        map
    }

    // ---- 文件 fallback:base64 混淆存到 ~/.config/guishell/passwords.toml ----

    fn passwords_path() -> std::path::PathBuf {
        crate::config::settings::Settings::config_dir().join("passwords.toml")
    }

    fn load_passwords() -> HashMap<String, String> {
        let path = Self::passwords_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => HashMap::new(),
        }
    }

    fn save_passwords(map: &HashMap<String, String>) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::passwords_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(map)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    fn file_store(&self, password: &str) -> Result<(), Box<dyn std::error::Error>> {
        use base64::Engine;
        let mut map = Self::load_passwords();
        let encoded = base64::engine::general_purpose::STANDARD.encode(password.as_bytes());
        map.insert(self.credential_key(), encoded);
        Self::save_passwords(&map)?;
        app_log!("KEYRING", "file fallback store 成功: {}", self.credential_key());
        Ok(())
    }

    fn file_retrieve(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
        use base64::Engine;
        let map = Self::load_passwords();
        match map.get(&self.credential_key()) {
            Some(encoded) => {
                let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
                let pw = String::from_utf8(bytes)?;
                app_log!("KEYRING", "file fallback retrieve 成功: {}", self.credential_key());
                Ok(Some(pw))
            }
            None => Ok(None),
        }
    }

    // ---- Linux: secret-service (GNOME Keyring) ----

    #[cfg(target_os = "linux")]
    pub async fn store_password(&self, password: &str) -> Result<(), Box<dyn std::error::Error>> {
        use secret_service::{EncryptionType, SecretService};
        let ss = SecretService::connect(EncryptionType::Dh).await?;
        let collection = ss.get_default_collection().await?;
        let attrs = self.attributes();
        let attr_refs: HashMap<&str, &str> = attrs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        collection.create_item(&self.label(), attr_refs, password.as_bytes(), true, "text/plain").await?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub async fn retrieve_password(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
        use secret_service::{EncryptionType, SecretService};
        let ss = SecretService::connect(EncryptionType::Dh).await?;
        let attrs = self.attributes();
        let attr_refs: HashMap<&str, &str> = attrs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let results = ss.search_items(attr_refs).await?;
        let item = match results.unlocked.first() {
            Some(item) => item,
            None => match results.locked.first() {
                Some(item) => { item.unlock().await?; item }
                None => return Ok(None),
            },
        };
        let secret = item.get_secret().await?;
        let password = String::from_utf8(secret)?;
        Ok(Some(password))
    }

    // ---- 非 Linux: keyring crate + 文件 fallback ----

    #[cfg(not(target_os = "linux"))]
    pub async fn store_password(&self, password: &str) -> Result<(), Box<dyn std::error::Error>> {
        // 尝试系统 keyring
        let key = self.credential_key();
        if let Ok(entry) = keyring::Entry::new("guishell", &key) {
            if entry.set_password(password).is_ok() {
                // 验证是否真的存进去了
                if let Ok(pw) = entry.get_password() {
                    if pw == password {
                        return Ok(());
                    }
                }
                app_log!("KEYRING", "系统 keyring store 后 verify 失败,回退文件存储");
            }
        }
        // 回退到文件存储
        self.file_store(password)?;
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub async fn retrieve_password(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
        // 先尝试系统 keyring
        let key = self.credential_key();
        if let Ok(entry) = keyring::Entry::new("guishell", &key) {
            if let Ok(pw) = entry.get_password() {
                return Ok(Some(pw));
            }
        }
        // 回退到文件
        self.file_retrieve()
    }
}
