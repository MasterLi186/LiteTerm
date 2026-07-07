use crate::app_log;
use crate::config::keyring::KeyringEntry;

#[tauri::command]
pub async fn store_password(user: String, host: String, port: u16, password: String) -> Result<(), String> {
    let entry = KeyringEntry::new(&user, &host, port);
    match entry.store_password(&password).await {
        Ok(()) => {
            app_log!("KEYRING", "store 成功: {}@{}:{}", user, host, port);
            Ok(())
        }
        Err(e) => {
            app_log!("KEYRING", "store 失败: {}@{}:{} err={}", user, host, port, e);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
pub async fn retrieve_password(user: String, host: String, port: u16) -> Result<Option<String>, String> {
    let entry = KeyringEntry::new(&user, &host, port);
    match entry.retrieve_password().await {
        Ok(Some(pw)) => {
            app_log!("KEYRING", "retrieve 成功: {}@{}:{}", user, host, port);
            Ok(Some(pw))
        }
        Ok(None) => {
            app_log!("KEYRING", "retrieve 无记录: {}@{}:{}", user, host, port);
            Ok(None)
        }
        Err(e) => {
            app_log!("KEYRING", "retrieve 失败: {}@{}:{} err={}", user, host, port, e);
            Err(e.to_string())
        }
    }
}
