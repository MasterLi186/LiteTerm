use crate::app_log;
use std::collections::HashMap;
use std::path::PathBuf;

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

    fn storage_key(&self) -> String {
        format!("{}_{}_{}", self.user, self.host, self.port)
    }

    // ---- AES-256-GCM 文件存储(所有平台通用,100% 可靠) ----

    fn credentials_path() -> PathBuf {
        crate::config::settings::Settings::config_dir().join("credentials.enc")
    }

    fn derive_key() -> Vec<u8> {
        use openssl::hash::MessageDigest;
        use openssl::pkcs5::pbkdf2_hmac;

        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown-host".to_string());
        let username = whoami::username();
        let salt = format!("liteterm-v1-{}-{}", hostname, username);

        let mut key = vec![0u8; 32]; // AES-256
        pbkdf2_hmac(
            b"liteterm-credential-store", // pass: 应用固定密码
            salt.as_bytes(),               // salt: hostname+username(绑定机器)
            100_000,
            MessageDigest::sha256(),
            &mut key,
        ).expect("PBKDF2 失败");
        key
    }

    fn encrypt(plaintext: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use openssl::symm::{encrypt_aead, Cipher};
        use openssl::rand::rand_bytes;

        let key = Self::derive_key();
        let mut nonce = vec![0u8; 12];
        rand_bytes(&mut nonce)?;
        let mut tag = vec![0u8; 16];

        let ciphertext = encrypt_aead(
            Cipher::aes_256_gcm(),
            &key,
            Some(&nonce),
            &[],
            plaintext,
            &mut tag,
        )?;

        // 格式: nonce(12) + tag(16) + ciphertext
        let mut result = Vec::with_capacity(12 + 16 + ciphertext.len());
        result.extend_from_slice(&nonce);
        result.extend_from_slice(&tag);
        result.extend_from_slice(&ciphertext);
        Ok(result)
    }

    fn decrypt(data: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use openssl::symm::{decrypt_aead, Cipher};

        if data.len() < 28 {
            return Err("密文太短".into());
        }

        let key = Self::derive_key();
        let nonce = &data[..12];
        let tag = &data[12..28];
        let ciphertext = &data[28..];

        let plaintext = decrypt_aead(
            Cipher::aes_256_gcm(),
            &key,
            Some(nonce),
            &[],
            ciphertext,
            tag,
        )?;
        Ok(plaintext)
    }

    fn load_store() -> HashMap<String, String> {
        let path = Self::credentials_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => HashMap::new(),
        }
    }

    fn save_store(map: &HashMap<String, String>) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::credentials_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(map)?;
        std::fs::write(&path, &content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    fn file_store(&self, password: &str) -> Result<(), Box<dyn std::error::Error>> {
        let encrypted = Self::encrypt(password.as_bytes())?;
        let encoded = hex::encode(&encrypted);

        let mut map = Self::load_store();
        map.insert(self.storage_key(), encoded);
        Self::save_store(&map)?;

        // 从磁盘重读验证(确认文件写入完整)
        let reloaded = Self::load_store();
        let stored = reloaded.get(&self.storage_key()).ok_or("写入后磁盘读回找不到记录")?;
        let decoded = hex::decode(stored)?;
        let decrypted = Self::decrypt(&decoded)?;
        let verify = String::from_utf8(decrypted)?;
        if verify != password {
            return Err("store 后 verify 失败: 解密内容不匹配".into());
        }

        app_log!("KEYRING", "file store+verify 成功: {}", self.storage_key());
        Ok(())
    }

    fn file_retrieve(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let map = Self::load_store();
        match map.get(&self.storage_key()) {
            Some(encoded) => {
                let data = hex::decode(encoded)?;
                let plaintext = Self::decrypt(&data)?;
                let password = String::from_utf8(plaintext)?;
                app_log!("KEYRING", "file retrieve 成功: {}", self.storage_key());
                Ok(Some(password))
            }
            None => {
                app_log!("KEYRING", "file retrieve 无记录: {}", self.storage_key());
                Ok(None)
            }
        }
    }

    // ---- 公开接口 ----

    #[cfg(target_os = "linux")]
    pub async fn store_password(&self, password: &str) -> Result<(), Box<dyn std::error::Error>> {
        use secret_service::{EncryptionType, SecretService};
        match SecretService::connect(EncryptionType::Dh).await {
            Ok(ss) => {
                let collection = ss.get_default_collection().await?;
                let mut attrs = HashMap::new();
                attrs.insert("application", "guishell");
                let key = self.storage_key();
                attrs.insert("key", &key);
                collection.create_item(&self.storage_key(), attrs, password.as_bytes(), true, "text/plain").await?;
                app_log!("KEYRING", "secret-service store 成功: {}", self.storage_key());
                // 同时写文件备份,secret-service 不可用时还能取到
                let _ = self.file_store(password);
                Ok(())
            }
            Err(e) => {
                app_log!("KEYRING", "secret-service 不可用({}), 回退文件存储", e);
                self.file_store(password)
            }
        }
    }

    #[cfg(target_os = "linux")]
    pub async fn retrieve_password(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
        use secret_service::{EncryptionType, SecretService};
        match SecretService::connect(EncryptionType::Dh).await {
            Ok(ss) => {
                let mut attrs = HashMap::new();
                attrs.insert("application", "guishell");
                let key = self.storage_key();
                attrs.insert("key", &key);
                let results = ss.search_items(attrs).await?;
                let item = match results.unlocked.first() {
                    Some(item) => item,
                    None => match results.locked.first() {
                        Some(item) => { item.unlock().await?; item }
                        None => return self.file_retrieve(),
                    },
                };
                let secret = item.get_secret().await?;
                let password = String::from_utf8(secret)?;
                app_log!("KEYRING", "secret-service retrieve 成功: {}", self.storage_key());
                Ok(Some(password))
            }
            Err(e) => {
                app_log!("KEYRING", "secret-service 不可用({}), 回退文件", e);
                self.file_retrieve()
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub async fn store_password(&self, password: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.file_store(password)
    }

    #[cfg(not(target_os = "linux"))]
    pub async fn retrieve_password(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
        self.file_retrieve()
    }
}
