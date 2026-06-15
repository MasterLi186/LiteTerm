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
        format!("guishell:ssh://{}@{}:{}", self.user, self.host, self.port)
    }

    pub fn attributes(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        map.insert("application".to_string(), "guishell".to_string());
        map.insert("user".to_string(), self.user.clone());
        map.insert("host".to_string(), self.host.clone());
        map.insert("port".to_string(), self.port.to_string());
        map
    }

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

    #[cfg(not(target_os = "linux"))]
    pub async fn store_password(&self, _password: &str) -> Result<(), Box<dyn std::error::Error>> {
        Err("密钥环仅在 Linux 上可用".into())
    }

    #[cfg(not(target_os = "linux"))]
    pub async fn retrieve_password(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
        Ok(None)
    }
}
