use crate::config::keyring::KeyringEntry;

#[tauri::command]
pub async fn store_password(user: String, host: String, port: u16, password: String) -> Result<(), String> {
    let entry = KeyringEntry::new(&user, &host, port);
    entry.store_password(&password).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn retrieve_password(user: String, host: String, port: u16) -> Result<Option<String>, String> {
    let entry = KeyringEntry::new(&user, &host, port);
    entry.retrieve_password().await.map_err(|e| e.to_string())
}
