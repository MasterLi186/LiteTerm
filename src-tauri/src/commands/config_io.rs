use tauri::State;

use crate::config::connections::ConnectionStore;
use crate::config::settings::Settings;
use crate::state::AppState;

#[tauri::command]
pub async fn export_config() -> Result<String, String> {
    let path = Settings::config_dir().join("connections.toml");
    std::fs::read_to_string(&path).map_err(|e| format!("读取配置失败: {}", e))
}

#[tauri::command]
pub async fn import_config(
    state: State<'_, AppState>,
    content: String,
) -> Result<(), String> {
    // Validate by parsing
    let store: ConnectionStore =
        toml::from_str(&content).map_err(|e| format!("配置格式无效: {}", e))?;

    // Write to disk
    let path = Settings::config_dir().join("connections.toml");
    store.save_to(&path).map_err(|e| format!("写入配置失败: {}", e))?;

    // Reload into app state
    let mut connections = state.connections.lock().unwrap();
    *connections = store;

    Ok(())
}

#[tauri::command]
pub async fn read_text_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {}", e))
}
