use std::collections::HashMap;
use std::sync::Mutex;

use tauri::State;

use crate::state::AppState;

pub struct Recording {
    pub file_path: String,
    pub start_time: std::time::Instant,
    pub width: u32,
    pub height: u32,
    pub events: Vec<String>,
}

pub type RecordingMap = Mutex<HashMap<String, Recording>>;

#[tauri::command]
pub fn start_recording(
    state: State<'_, AppState>,
    terminal_id: String,
    file_path: String,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let header = serde_json::json!({
        "version": 2,
        "width": width,
        "height": height,
        "timestamp": timestamp,
        "title": "LiteTerm Recording"
    });

    let recording = Recording {
        file_path,
        start_time: std::time::Instant::now(),
        width,
        height,
        events: vec![header.to_string()],
    };

    let mut recordings = state.recordings.lock().map_err(|e| e.to_string())?;
    recordings.insert(terminal_id, recording);
    Ok(())
}

#[tauri::command]
pub fn stop_recording(
    state: State<'_, AppState>,
    terminal_id: String,
) -> Result<String, String> {
    let mut recordings = state.recordings.lock().map_err(|e| e.to_string())?;
    let recording = recordings
        .remove(&terminal_id)
        .ok_or_else(|| "没有进行中的录制".to_string())?;

    let expanded = shellexpand::tilde(&recording.file_path);
    let path = std::path::Path::new(expanded.as_ref());

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("保存录制失败: {}", e))?;
    }

    let content = recording.events.join("\n") + "\n";
    std::fs::write(path, content).map_err(|e| format!("保存录制失败: {}", e))?;

    Ok(expanded.into_owned())
}

#[tauri::command]
pub fn record_event(
    state: State<'_, AppState>,
    terminal_id: String,
    data: String,
) -> Result<(), String> {
    let mut recordings = state.recordings.lock().map_err(|e| e.to_string())?;
    if let Some(recording) = recordings.get_mut(&terminal_id) {
        let elapsed = recording.start_time.elapsed().as_secs_f64();
        let event = serde_json::json!([elapsed, "o", data]);
        recording.events.push(event.to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn is_recording(
    state: State<'_, AppState>,
    terminal_id: String,
) -> Result<bool, String> {
    let recordings = state.recordings.lock().map_err(|e| e.to_string())?;
    Ok(recordings.contains_key(&terminal_id))
}
