use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post, put, delete},
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::app_log;

/// HTTP API 服务器共享状态
struct ApiState {
    app_handle: AppHandle,
    token: String,
}

/// API 统一返回类型
type ApiResult = Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)>;

/// 验证 Bearer token 认证
fn check_auth(headers: &HeaderMap, token: &str) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth == format!("Bearer {}", token) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Unauthorized"})),
        ))
    }
}

/// 启动 HTTP API 服务器，绑定 127.0.0.1:19526
const DEFAULT_PORT: u16 = 19526;
const MAX_PORT_TRIES: u16 = 10;

pub async fn start_api_server(app_handle: AppHandle) {
    // 端口选择：环境变量 > 自动探测(19526 起，最多试 10 个)
    let base_port = std::env::var("LITETERM_API_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);

    let mut listener = None;
    let mut actual_port = base_port;

    if std::env::var("LITETERM_API_PORT").is_ok() {
        // 指定了端口，只尝试该端口
        match tokio::net::TcpListener::bind(format!("127.0.0.1:{}", base_port)).await {
            Ok(l) => { listener = Some(l); }
            Err(e) => {
                app_log!("API", "WARNING: 指定端口 {} 绑定失败: {}", base_port, e);
                return;
            }
        }
    } else {
        // 自动探测：从默认端口开始，被占用则递增
        for offset in 0..MAX_PORT_TRIES {
            let port = base_port + offset;
            match tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await {
                Ok(l) => {
                    listener = Some(l);
                    actual_port = port;
                    if offset > 0 {
                        app_log!("API", "默认端口 {} 被占用，使用 {}", base_port, port);
                    }
                    break;
                }
                Err(_) => continue,
            }
        }
    }

    let listener = match listener {
        Some(l) => l,
        None => {
            app_log!("API", "WARNING: 端口 {}-{} 全部被占用，HTTP API 不可用", base_port, base_port + MAX_PORT_TRIES - 1);
            return;
        }
    };

    // 生成 32 字节随机 token 并 hex 编码为 64 字符字符串
    use rand::Rng;
    let token: String = {
        let mut rng = rand::thread_rng();
        let bytes: Vec<u8> = (0..32).map(|_| rng.gen()).collect();
        hex::encode(bytes)
    };

    // 写入 token + 端口信息(绑定成功后才写，保证文件与实际服务一致)
    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("guishell");
    std::fs::create_dir_all(&config_dir).ok();

    let token_path = config_dir.join("api_token");
    if let Err(e) = std::fs::write(&token_path, &token) {
        app_log!("API", "WARNING: 写入 api_token 失败: {}", e);
    }

    // api_port 文件写入 JSON: {"port": N, "pid": N}
    let pid = std::process::id();
    let port_info = serde_json::json!({"port": actual_port, "pid": pid});
    let port_path = config_dir.join("api_port");
    if let Err(e) = std::fs::write(&port_path, port_info.to_string()) {
        app_log!("API", "WARNING: 写入 api_port 失败: {}", e);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600)).ok();
        std::fs::set_permissions(&port_path, std::fs::Permissions::from_mode(0o600)).ok();
    }

    app_log!("API", "API token 已写入: {}", token_path.display());
    app_log!("API", "API port 已写入: {} (port={}, pid={})", port_path.display(), actual_port, pid);

    let api_state = Arc::new(ApiState {
        app_handle,
        token,
    });

    let app = Router::new()
        .route("/api/v1/tabs", get(list_tabs))
        .route("/api/v1/tabs/local", post(open_local))
        .route("/api/v1/tabs/ssh", post(open_ssh))
        .route("/api/v1/tabs/{id}/focus", put(focus_tab))
        .route("/api/v1/tabs/{id}/write", post(write_tab))
        .route("/api/v1/tabs/{id}/read", get(read_tab))
        .route("/api/v1/tabs/{id}", delete(close_tab))
        .with_state(api_state);

    app_log!("API", "HTTP API 服务器已启动: http://127.0.0.1:{}", actual_port);
    if let Err(e) = axum::serve(listener, app).await {
        app_log!("API", "HTTP API 服务器异常退出: {}", e);
    }
}

// ---------------------------------------------------------------------------
// GET /api/v1/tabs — 列出所有标签页
// ---------------------------------------------------------------------------

async fn list_tabs(
    State(api): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> ApiResult {
    check_auth(&headers, &api.token)?;
    let state = api.app_handle.state::<crate::state::AppState>();
    let tabs: Vec<serde_json::Value> = state
        .tab_registry
        .lock()
        .unwrap()
        .values()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "label": t.label,
                "type": t.tab_type,
            })
        })
        .collect();
    Ok(Json(serde_json::json!(tabs)))
}

// ---------------------------------------------------------------------------
// POST /api/v1/tabs/local — 打开本地终端
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct OpenLocalBody {
    shell_path: Option<String>,
}

async fn open_local(
    State(api): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult {
    check_auth(&headers, &api.token)?;
    let state = api.app_handle.state::<crate::state::AppState>();

    // 解析可选请求体
    let shell_path = if body.is_empty() {
        None
    } else {
        let parsed: OpenLocalBody = serde_json::from_slice(&body).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("无效 JSON: {}", e)})),
            )
        })?;
        parsed.shell_path.filter(|s| !s.is_empty())
    };

    // 打开本地终端
    let id = crate::commands::terminal::do_open_terminal(&state, &api.app_handle, shell_path)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
        })?;

    // 生成标签名(计数已有的本地终端数)
    let count = state
        .tab_registry
        .lock()
        .unwrap()
        .values()
        .filter(|t| t.tab_type == "local")
        .count();
    let label = format!("本地终端 {}", count + 1);

    // 注册到标签页注册表
    state.tab_registry.lock().unwrap().insert(
        id.clone(),
        crate::state::TabInfo {
            id: id.clone(),
            label: label.clone(),
            tab_type: "local".to_string(),
        },
    );

    // 通知前端同步 React state
    let _ = api.app_handle.emit(
        "api-tab-opened",
        serde_json::json!({"id": &id, "label": &label, "type": "local"}),
    );

    Ok(Json(serde_json::json!({"id": id, "label": label})))
}

// ---------------------------------------------------------------------------
// POST /api/v1/tabs/ssh — 打开 SSH 连接
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SshConnectBody {
    host: String,
    port: Option<u16>,
    user: String,
    password: Option<String>,
    auth_method: Option<String>,
    key_path: Option<String>,
    proxy_jump: Option<String>,
}

async fn open_ssh(
    State(api): State<Arc<ApiState>>,
    headers: HeaderMap,
    bytes: axum::body::Bytes,
) -> ApiResult {
    check_auth(&headers, &api.token)?;
    let body: SshConnectBody = serde_json::from_slice(&bytes).map_err(|e| {
        (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": format!("无效请求: {}", e)})))
    })?;
    let state = api.app_handle.state::<crate::state::AppState>();

    let host = body.host;
    let user = body.user;
    let port = body.port.unwrap_or(22);
    let password = body.password;
    let auth_method = body.auth_method.unwrap_or_else(|| "keyring".to_string());
    let key_path = body.key_path;
    let proxy_jump = body.proxy_jump;
    let label = format!("{}@{}", user, host);

    let id = crate::commands::ssh::do_ssh_connect(
        &state,
        &api.app_handle,
        host.clone(),
        port,
        user.clone(),
        password.clone(),
        auth_method.clone(),
        key_path.clone(),
        label.clone(),
        proxy_jump.clone(),
        None,
        None,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
    })?;

    // 注册到标签页注册表
    state.tab_registry.lock().unwrap().insert(
        id.clone(),
        crate::state::TabInfo {
            id: id.clone(),
            label: label.clone(),
            tab_type: "ssh".to_string(),
        },
    );

    // 通知前端（携带 sshParams 以支持断线重连 UI）
    let _ = api.app_handle.emit(
        "api-tab-opened",
        serde_json::json!({
            "id": &id, "label": &label, "type": "ssh",
            "sshParams": {
                "host": host, "port": port, "user": user,
                "password": password, "authMethod": auth_method,
                "keyPath": key_path, "proxyJump": proxy_jump,
            }
        }),
    );

    Ok(Json(serde_json::json!({"id": id, "label": label})))
}

// ---------------------------------------------------------------------------
// PUT /api/v1/tabs/{id}/focus — 切换活跃标签页
// ---------------------------------------------------------------------------

async fn focus_tab(
    State(api): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult {
    check_auth(&headers, &api.token)?;
    let state = api.app_handle.state::<crate::state::AppState>();

    // 检查标签页是否存在
    if !state.tab_registry.lock().unwrap().contains_key(&id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "terminal not found"})),
        ));
    }

    let _ = api.app_handle.emit("api-tab-focus", serde_json::json!({"id": &id}));
    Ok(Json(serde_json::json!({"ok": true})))
}

// ---------------------------------------------------------------------------
// POST /api/v1/tabs/{id}/write — 向终端写入数据
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WriteBody {
    data: String,
}

async fn write_tab(
    State(api): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    bytes: axum::body::Bytes,
) -> ApiResult {
    check_auth(&headers, &api.token)?;
    let body: WriteBody = serde_json::from_slice(&bytes).map_err(|e| {
        (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": format!("无效请求: {}", e)})))
    })?;
    let state = api.app_handle.state::<crate::state::AppState>();
    let data = body.data.into_bytes();

    // 先查本地终端
    {
        let terms = state.local_terminals.lock().unwrap();
        if let Some(term) = terms.get(&id) {
            return match term.input_tx.send(data) {
                Ok(_) => Ok(Json(serde_json::json!({"ok": true}))),
                Err(e) => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": e.to_string()})),
                )),
            };
        }
    }
    // 再查 SSH 会话
    {
        let sessions = state.sessions.lock().unwrap();
        if let Some(session) = sessions.get(&id) {
            return match session.input_tx.send(data) {
                Ok(_) => Ok(Json(serde_json::json!({"ok": true}))),
                Err(e) => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": e.to_string()})),
                )),
            };
        }
    }
    Err((
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "terminal not found"})),
    ))
}

// ---------------------------------------------------------------------------
// GET /api/v1/tabs/{id}/read — 读取终端输出
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ReadQuery {
    cursor: Option<u64>,
    raw: Option<bool>,
}

async fn read_tab(
    State(api): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(params): Query<ReadQuery>,
) -> ApiResult {
    check_auth(&headers, &api.token)?;
    let state = api.app_handle.state::<crate::state::AppState>();

    // 读取缓冲区数据(块内释放锁)
    let (data, new_cursor, truncated) = {
        let bufs = state.output_buffers.lock().unwrap();
        let ob = bufs.get(&id).ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "terminal not found"})),
            )
        })?;
        let cursor = params.cursor.unwrap_or(0);
        ob.read_from(cursor)
    };

    // 默认过滤 ANSI 转义码，raw=true 时保留
    let raw = params.raw.unwrap_or(false);
    let text = if raw {
        String::from_utf8_lossy(&data).to_string()
    } else {
        let stripped = strip_ansi_escapes::strip(&data);
        String::from_utf8_lossy(&stripped).to_string()
    };

    Ok(Json(serde_json::json!({
        "data": text,
        "cursor": new_cursor,
        "truncated": truncated,
    })))
}

// ---------------------------------------------------------------------------
// DELETE /api/v1/tabs/{id} — 关闭标签页
// ---------------------------------------------------------------------------

async fn close_tab(
    State(api): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult {
    check_auth(&headers, &api.token)?;
    let state = api.app_handle.state::<crate::state::AppState>();

    // 检查标签页是否存在
    if !state.tab_registry.lock().unwrap().contains_key(&id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "terminal not found"})),
        ));
    }

    // 清理本地终端(drop input_tx + resize_tx 释放 writer/resize/reader 线程)
    if let Some(term) = state.local_terminals.lock().unwrap().remove(&id) {
        term.stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
        drop(term.input_tx);
        drop(term.resize_tx);
    }
    // 清理 SSH 会话
    if let Some(session) = state.sessions.lock().unwrap().remove(&id) {
        session
            .monitor_stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    // 清理 SFTP 会话
    state.sftp_sessions.lock().unwrap().remove(&id);
    // 清理输出缓冲区
    state.output_buffers.lock().unwrap().remove(&id);
    // 清理标签页注册表
    state.tab_registry.lock().unwrap().remove(&id);

    // 通知前端同步 React state
    let _ = api.app_handle.emit(
        "api-tab-closed",
        serde_json::json!({"id": &id}),
    );

    Ok(Json(serde_json::json!({"ok": true})))
}
