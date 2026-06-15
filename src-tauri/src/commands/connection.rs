use tauri::State;

use crate::config::connections::HostConfig;
use crate::config::settings::Settings;
use crate::state::AppState;

#[tauri::command]
pub async fn load_connections(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let store = state.connections.lock().unwrap();
    serde_json::to_value(&*store).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_connection(
    state: State<'_, AppState>,
    group_id: String,
    group_label: String,
    group_color: String,
    host_id: String,
    config: HostConfig,
) -> Result<(), String> {
    let mut store = state.connections.lock().unwrap();
    if !store.groups.contains_key(&group_id) {
        store.add_group(&group_id, &group_label, &group_color);
    }
    store.add_host(&group_id, &host_id, config);

    let path = Settings::config_dir().join("connections.toml");
    store.save_to(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_connection(
    state: State<'_, AppState>,
    group_id: String,
    host_id: String,
) -> Result<(), String> {
    let mut store = state.connections.lock().unwrap();
    store.remove_host(&group_id, &host_id);

    let path = Settings::config_dir().join("connections.toml");
    store.save_to(&path).map_err(|e| e.to_string())
}
