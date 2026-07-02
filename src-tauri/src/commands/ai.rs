use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;

use tauri::{AppHandle, Manager, State};

use crate::app_log;
use crate::state::AppState;

const MODEL_URL: &str =
    "https://huggingface.co/OpenBMB/BitCPM-CANN-8B-gguf/resolve/main/bitcpm4-0.5b-tq2_0.gguf";

// ---------------------------------------------------------------------------
// Sidecar lifecycle (non-command helpers)
// ---------------------------------------------------------------------------

/// Start the 158BitNet `openai_server` sidecar process.
pub fn start_sidecar(state: &AppState) -> Result<(), String> {
    let settings = state.settings.lock().unwrap();
    if !settings.ai.enabled {
        return Err("AI 功能未启用".into());
    }

    // Expand ~ in model path
    let model_path_raw = settings.ai.model_path.clone();
    drop(settings); // release lock early

    let model_path = shellexpand::tilde(&model_path_raw).to_string();
    let model_path = PathBuf::from(&model_path);
    if !model_path.exists() {
        return Err(format!("模型文件不存在: {}", model_path.display()));
    }

    // Locate openai_server binary: dev layout first, then prod
    let exe_dir = std::env::current_exe()
        .map_err(|e| format!("获取可执行文件路径失败: {}", e))?
        .parent()
        .ok_or("无法获取可执行文件目录")?
        .to_path_buf();

    let dev_bin = exe_dir.join("../../158BitNet/build/openai_server");
    let prod_bin = exe_dir.join("openai_server");
    let server_bin = if dev_bin.exists() {
        dev_bin
    } else if prod_bin.exists() {
        prod_bin
    } else {
        return Err("未找到 openai_server 可执行文件".into());
    };

    // Find a free port
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("绑定端口失败: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("获取端口失败: {}", e))?
        .port();
    drop(listener);

    // Spawn sidecar
    let child = Command::new(&server_bin)
        .arg(model_path.to_str().unwrap_or_default())
        .args(["--host", "127.0.0.1"])
        .args(["--port", &port.to_string()])
        .args(["--ctx", "2048"])
        .args(["--max-tokens", "256"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("启动 openai_server 失败: {}", e))?;

    let pid = child.id();
    *state.ai_child.lock().unwrap() = Some(pid);
    *state.ai_port.lock().unwrap() = Some(port);

    app_log!("AI", "sidecar 已启动 pid={} port={}", pid, port);
    Ok(())
}

/// Stop the sidecar by sending SIGTERM to its PID.
pub fn stop_sidecar(state: &AppState) {
    if let Some(pid) = state.ai_child.lock().unwrap().take() {
        #[cfg(unix)]
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        app_log!("AI", "sidecar 已停止 pid={}", pid);
    }
    *state.ai_port.lock().unwrap() = None;
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn ai_download_model(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    let model_path_raw = state.settings.lock().unwrap().ai.model_path.clone();
    let model_path_str = shellexpand::tilde(&model_path_raw).to_string();
    let model_path = PathBuf::from(&model_path_str);

    // Already have the model
    if model_path.exists() {
        return Ok(());
    }

    // Guard against concurrent downloads
    if state.ai_downloading.swap(true, Ordering::SeqCst) {
        return Err("模型正在下载中".into());
    }

    // Clone AppHandle — it is cheap to clone and gives us access to managed
    // state inside the spawned task via app_handle.state::<AppState>().
    let app_handle = app.clone();

    tokio::spawn(async move {
        app_log!("AI", "开始下载模型: {}", MODEL_URL);

        let result: Result<(), String> = async {
            let response = reqwest::get(MODEL_URL)
                .await
                .map_err(|e| format!("下载请求失败: {}", e))?;

            if !response.status().is_success() {
                return Err(format!("下载失败 HTTP {}", response.status()));
            }

            // Create parent directories
            if let Some(parent) = model_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建目录失败: {}", e))?;
            }

            let bytes = response
                .bytes()
                .await
                .map_err(|e| format!("读取响应失败: {}", e))?;

            std::fs::write(&model_path, &bytes)
                .map_err(|e| format!("写入模型文件失败: {}", e))?;

            app_log!("AI", "模型下载完成: {}", model_path.display());
            Ok(())
        }
        .await;

        // Retrieve AppState from the managed state via AppHandle
        let st: tauri::State<'_, AppState> = app_handle.state();

        match result {
            Ok(()) => {
                if let Err(e) = start_sidecar(&st) {
                    app_log!("AI", "下载后启动 sidecar 失败: {}", e);
                }
            }
            Err(e) => {
                app_log!("AI", "模型下载失败: {}", e);
            }
        }

        st.ai_downloading.store(false, Ordering::SeqCst);
    });

    Ok(())
}

#[tauri::command]
pub async fn ai_chat(
    state: State<'_, AppState>,
    system_prompt: String,
    user_message: String,
    max_tokens: Option<u32>,
) -> Result<String, String> {
    let port = state
        .ai_port
        .lock()
        .unwrap()
        .ok_or("AI sidecar 未运行")?;

    let url = format!("http://127.0.0.1:{}/v1/chat/completions", port);
    let body = serde_json::json!({
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_message },
        ],
        "max_tokens": max_tokens.unwrap_or(256),
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("AI 请求失败: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("AI 返回错误 HTTP {}", resp.status()));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 AI 响应失败: {}", e))?;

    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();

    Ok(content)
}

#[tauri::command]
pub fn ai_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let settings = state.settings.lock().unwrap();
    let enabled = settings.ai.enabled;
    let model_path_raw = settings.ai.model_path.clone();
    drop(settings);

    let model_path = shellexpand::tilde(&model_path_raw).to_string();
    let model_exists = PathBuf::from(&model_path).exists();
    let downloading = state.ai_downloading.load(Ordering::SeqCst);
    let port = *state.ai_port.lock().unwrap();
    let sidecar_running = port.is_some();

    Ok(serde_json::json!({
        "enabled": enabled,
        "model_exists": model_exists,
        "downloading": downloading,
        "sidecar_running": sidecar_running,
        "port": port,
    }))
}
