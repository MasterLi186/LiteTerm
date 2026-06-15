use secret_service::EncryptionType;
use secret_service::SecretService;
use std::collections::HashMap;

/// Represents a keyring entry for an SSH connection credential.
///
/// Stores user, host, and port information, and provides methods
/// to interact with the GNOME Keyring (or any Secret Service provider)
/// via D-Bus.
pub struct KeyringEntry {
    user: String,
    host: String,
    port: u16,
}

impl KeyringEntry {
    /// Create a new keyring entry for the given SSH connection parameters.
    pub fn new(user: &str, host: &str, port: u16) -> Self {
        Self {
            user: user.to_string(),
            host: host.to_string(),
            port,
        }
    }

    /// Return the label used to identify this entry in the keyring.
    ///
    /// Format: `guishell:ssh://{user}@{host}:{port}`
    pub fn label(&self) -> String {
        format!(
            "guishell:ssh://{}@{}:{}",
            self.user, self.host, self.port
        )
    }

    /// Return a set of searchable attributes for this entry.
    ///
    /// Keys: `application`, `user`, `host`, `port`.
    pub fn attributes(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        map.insert("application".to_string(), "guishell".to_string());
        map.insert("user".to_string(), self.user.clone());
        map.insert("host".to_string(), self.host.clone());
        map.insert("port".to_string(), self.port.to_string());
        map
    }

    /// Store a password in the keyring for this entry.
    ///
    /// Requires a running D-Bus session with a Secret Service provider.
    pub async fn store_password(&self, password: &str) -> Result<(), secret_service::Error> {
        let ss = SecretService::connect(EncryptionType::Dh).await?;
        let collection = ss.get_default_collection().await?;

        let attrs = self.attributes();
        let attr_refs: HashMap<&str, &str> = attrs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        collection
            .create_item(
                &self.label(),
                attr_refs,
                password.as_bytes(),
                true, // replace existing item with same attributes
                "text/plain",
            )
            .await?;

        Ok(())
    }

    /// Retrieve the stored password from the keyring for this entry.
    ///
    /// Returns `None` if no matching entry is found.
    /// Requires a running D-Bus session with a Secret Service provider.
    pub async fn retrieve_password(&self) -> Result<Option<String>, secret_service::Error> {
        let ss = SecretService::connect(EncryptionType::Dh).await?;

        let attrs = self.attributes();
        let attr_refs: HashMap<&str, &str> = attrs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let results = ss.search_items(attr_refs).await?;

        let item = match results.unlocked.first() {
            Some(item) => item,
            None => match results.locked.first() {
                Some(item) => {
                    item.unlock().await?;
                    item
                }
                None => return Ok(None),
            },
        };

        let secret = item.get_secret().await?;
        let password = String::from_utf8(secret)
            .map_err(|_| secret_service::Error::Crypto("invalid UTF-8 in stored password"))?;

        Ok(Some(password))
    }

    /// Delete the stored password from the keyring for this entry.
    ///
    /// Requires a running D-Bus session with a Secret Service provider.
    pub async fn delete_password(&self) -> Result<(), secret_service::Error> {
        let ss = SecretService::connect(EncryptionType::Dh).await?;

        let attrs = self.attributes();
        let attr_refs: HashMap<&str, &str> = attrs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let results = ss.search_items(attr_refs).await?;

        for item in results.unlocked.iter().chain(results.locked.iter()) {
            let _ = item.unlock().await;
            item.delete().await?;
        }

        Ok(())
    }
}
