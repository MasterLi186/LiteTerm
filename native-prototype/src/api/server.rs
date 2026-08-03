use super::*;

#[derive(Clone)]
pub(super) struct ApiToken([u8; 32]);

impl ApiToken {
    fn generate() -> Result<Self, ApiError> {
        let mut bytes = [0_u8; 32];
        rand_bytes(&mut bytes).map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "random_failed",
                "无法生成 API token",
            )
        })?;
        Ok(Self(bytes))
    }

    #[cfg(test)]
    pub(super) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn encoded(&self) -> String {
        hex::encode(self.0)
    }

    fn authorize(&self, headers: &HeaderMap) -> bool {
        let Some(value) = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
        else {
            return false;
        };

        let mut candidate = [0_u8; 32];
        let decoded =
            (value.len() == 64 && hex::decode_to_slice(value, &mut candidate).is_ok()) as u8;
        (candidate.ct_eq(&self.0).unwrap_u8() & decoded) == 1
    }
}

impl fmt::Debug for ApiToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiToken([REDACTED])")
    }
}

#[derive(Clone)]
struct ApiState {
    token: ApiToken,
    bridge: Bridge,
    outputs: OutputRegistry,
}

pub fn build_router(token_bytes: [u8; 32], bridge: Bridge, outputs: OutputRegistry) -> Router {
    build_router_with_token(ApiToken(token_bytes), bridge, outputs)
}

fn build_router_with_token(token: ApiToken, bridge: Bridge, outputs: OutputRegistry) -> Router {
    let state = ApiState {
        token,
        bridge,
        outputs,
    };
    Router::new()
        .route("/api/v1/tabs", get(list_tabs))
        .route("/api/v1/tabs/local", post(open_local))
        .route("/api/v1/tabs/ssh", post(open_ssh))
        .route("/api/v1/tabs/{id}/focus", put(focus_tab))
        .route("/api/v1/tabs/{id}/write", post(write_tab))
        .route("/api/v1/tabs/{id}/read", get(read_tab))
        .route("/api/v1/tabs/{id}", delete(close_tab))
        .fallback(api_not_found)
        .method_not_allowed_fallback(api_method_not_allowed)
        .with_state(state.clone())
        .layer(middleware::from_fn(enforce_body_limit))
        .layer(middleware::from_fn_with_state(state, require_auth))
        // Last added is outermost, including unauthenticated connections.
        .layer(ConcurrencyLimitLayer::new(MAX_CONCURRENT_REQUESTS))
}

async fn require_auth(
    State(state): State<ApiState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if !state.token.authorize(request.headers()) {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Unauthorized",
        ));
    }
    Ok(next.run(request).await)
}

async fn enforce_body_limit(request: Request, next: Next) -> Result<Response, ApiError> {
    let (parts, body) = request.into_parts();
    let body = timeout(BODY_READ_TIMEOUT, to_bytes(body, MAX_BODY_BYTES))
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::REQUEST_TIMEOUT,
                "body_timeout",
                "读取请求体超时",
            )
        })?
        .map_err(|_| {
            ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "body_too_large",
                "请求体超过 64KiB",
            )
        })?;
    let request = Request::from_parts(parts, Body::from(body));
    Ok(next.run(request).await)
}

struct ApiBody(Bytes);

impl<S> FromRequest<S> for ApiBody
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        Bytes::from_request(request, state)
            .await
            .map(Self)
            .map_err(|rejection| {
                let status = rejection.status();
                if status == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::new(status, "body_too_large", "请求体超过 64KiB")
                } else {
                    ApiError::new(status, "body_read_failed", "无法读取请求体")
                }
            })
    }
}

struct ApiQuery<T>(T);

impl<S, T> FromRequestParts<S> for ApiQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(query)| Self(query))
            .map_err(|_| ApiError::bad_request("无效查询参数"))
    }
}

#[derive(Debug, Default, Deserialize)]
struct PaneQuery {
    pane_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ReadQuery {
    pane_id: Option<String>,
    cursor: Option<u64>,
    stream_id: Option<u64>,
    limit: Option<usize>,
    raw: Option<bool>,
}

async fn list_tabs(State(state): State<ApiState>) -> Result<Json<serde_json::Value>, ApiError> {
    match state.bridge.call(ApiOperation::ListTabs).await? {
        ApiReply::Tabs(tabs) => Ok(Json(serde_json::to_value(tabs).map_err(json_error)?)),
        _ => Err(unexpected_reply()),
    }
}

async fn open_local(
    State(state): State<ApiState>,
    ApiBody(body): ApiBody,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request = if body.is_empty() {
        OpenLocalRequest::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("无效 JSON 请求体"))?
    };
    match state.bridge.call(ApiOperation::OpenLocal(request)).await? {
        ApiReply::Opened(tab) => Ok(Json(serde_json::json!({
            "id": tab.id,
            "label": tab.label,
        }))),
        _ => Err(unexpected_reply()),
    }
}

async fn open_ssh(
    State(state): State<ApiState>,
    ApiBody(body): ApiBody,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request: OpenSshRequest =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("无效 JSON 请求体"))?;
    if request.host.trim().is_empty() || request.user.trim().is_empty() {
        return Err(ApiError::bad_request("host 和 user 不能为空"));
    }
    match state.bridge.call(ApiOperation::OpenSsh(request)).await? {
        ApiReply::Opened(tab) => Ok(Json(serde_json::json!({
            "id": tab.id,
            "label": tab.label,
        }))),
        _ => Err(unexpected_reply()),
    }
}

async fn focus_tab(
    State(state): State<ApiState>,
    AxumPath(tab_id): AxumPath<String>,
    ApiQuery(query): ApiQuery<PaneQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    match state
        .bridge
        .call(ApiOperation::Focus {
            tab_id,
            pane_id: clean_pane_id(query.pane_id)?,
        })
        .await?
    {
        ApiReply::Focused { .. } => Ok(Json(serde_json::json!({"ok": true}))),
        _ => Err(unexpected_reply()),
    }
}

async fn write_tab(
    State(state): State<ApiState>,
    AxumPath(tab_id): AxumPath<String>,
    ApiQuery(query): ApiQuery<PaneQuery>,
    ApiBody(body): ApiBody,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request: WriteRequest =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("无效 JSON 请求体"))?;
    match state
        .bridge
        .call(ApiOperation::Write {
            tab_id,
            pane_id: clean_pane_id(query.pane_id)?,
            data: request.data.into_bytes(),
        })
        .await?
    {
        ApiReply::Written { .. } => Ok(Json(serde_json::json!({"ok": true}))),
        _ => Err(unexpected_reply()),
    }
}

async fn read_tab(
    State(state): State<ApiState>,
    AxumPath(tab_id): AxumPath<String>,
    ApiQuery(query): ApiQuery<ReadQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let pane_id = match clean_pane_id(query.pane_id)? {
        Some(pane_id) => pane_id,
        None => match state
            .bridge
            .call(ApiOperation::ResolvePane {
                tab_id: tab_id.clone(),
            })
            .await?
        {
            ApiReply::PaneResolved { pane_id } => pane_id,
            _ => return Err(unexpected_reply()),
        },
    };
    let read = state.outputs.read_text(
        &tab_id,
        &pane_id,
        query.stream_id,
        query.cursor,
        query.limit.unwrap_or(MAX_READ_BYTES),
        !query.raw.unwrap_or(false),
    )?;
    Ok(Json(serde_json::json!({
        "data": read.data,
        "cursor": read.cursor,
        "truncated": read.truncated,
        "stream_id": read.stream_id,
        "pane_id": pane_id,
    })))
}

async fn close_tab(
    State(state): State<ApiState>,
    AxumPath(tab_id): AxumPath<String>,
    ApiQuery(query): ApiQuery<PaneQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    match state
        .bridge
        .call(ApiOperation::Close {
            tab_id,
            pane_id: clean_pane_id(query.pane_id)?,
        })
        .await?
    {
        ApiReply::Closed => Ok(Json(serde_json::json!({"ok": true}))),
        _ => Err(unexpected_reply()),
    }
}

fn clean_pane_id(pane_id: Option<String>) -> Result<Option<String>, ApiError> {
    pane_id
        .map(|pane_id| {
            let pane_id = pane_id.trim();
            if pane_id.is_empty() {
                Err(ApiError::bad_request("pane_id 不能为空"))
            } else {
                Ok(pane_id.to_owned())
            }
        })
        .transpose()
}

fn unexpected_reply() -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "unexpected_main_thread_reply",
        "主线程返回了不匹配的响应",
    )
}

fn json_error(_: serde_json::Error) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "json_encode_failed",
        "无法编码 JSON 响应",
    )
}

async fn api_not_found() -> ApiError {
    ApiError::not_found("API endpoint not found")
}

async fn api_method_not_allowed() -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "Method not allowed",
    )
}

// ---------------------------------------------------------------------------
// Loopback server and Native-specific discovery
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ApiServerConfig {
    /// Exact directory containing Native discovery files.
    pub config_dir: PathBuf,
    /// `0` requests an ephemeral port and is intended for isolated tests.
    pub port: u16,
}

impl ApiServerConfig {
    pub fn new(config_dir: PathBuf, port: u16) -> Self {
        Self { config_dir, port }
    }
}

impl Default for ApiServerConfig {
    fn default() -> Self {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("guishell");
        Self {
            config_dir,
            port: 19526,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiscoveryPort {
    pub port: u16,
    pub pid: u32,
    pub instance: String,
}

struct DiscoveryGuard {
    token_path: PathBuf,
    token_contents: Vec<u8>,
    port_path: PathBuf,
    port_contents: Vec<u8>,
}

impl DiscoveryGuard {
    fn cleanup(&self) {
        remove_if_owned(&self.port_path, &self.port_contents);
        remove_if_owned(&self.token_path, &self.token_contents);
    }
}

impl Drop for DiscoveryGuard {
    fn drop(&mut self) {
        self.cleanup();
    }
}

impl fmt::Debug for DiscoveryGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscoveryGuard")
            .field("token_path", &self.token_path)
            .field("token_contents", &"[REDACTED]")
            .field("port_path", &self.port_path)
            .field("port_contents", &"[REDACTED]")
            .finish()
    }
}

pub struct ApiServerHandle {
    address: SocketAddrV4,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), io::Error>>>,
}

impl ApiServerHandle {
    pub fn address(&self) -> SocketAddrV4 {
        self.address
    }

    pub async fn shutdown(mut self) -> Result<(), ApiError> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let Some(mut task) = self.task.take() else {
            return Ok(());
        };
        let joined = match timeout(SHUTDOWN_TIMEOUT, &mut task).await {
            Ok(joined) => joined,
            Err(_) => {
                task.abort();
                let _ = task.await;
                return Err(ApiError::new(
                    StatusCode::GATEWAY_TIMEOUT,
                    "shutdown_timeout",
                    "HTTP API 服务关闭超时，任务已终止",
                ));
            }
        };
        match joined {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_failed",
                format!("HTTP API 服务异常退出: {error}"),
            )),
            Err(error) => Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_task_failed",
                format!("HTTP API 服务任务异常退出: {error}"),
            )),
        }
    }
}

impl Drop for ApiServerHandle {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

impl fmt::Debug for ApiServerHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiServerHandle")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

pub async fn start_server(
    config: ApiServerConfig,
    bridge: Bridge,
    outputs: OutputRegistry,
) -> Result<ApiServerHandle, ApiError> {
    let bind_address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, config.port);
    let listener = TcpListener::bind(bind_address).await.map_err(|error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "bind_failed",
            format!("无法绑定 Native HTTP API: {error}"),
        )
    })?;
    let address = match listener.local_addr().map_err(io_api_error)? {
        SocketAddr::V4(address) if *address.ip() == Ipv4Addr::LOCALHOST => address,
        _ => {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "non_loopback_listener",
                "Native HTTP API 必须绑定 IPv4 loopback",
            ));
        }
    };

    let token = ApiToken::generate()?;
    let guard = publish_discovery(&config.config_dir, address.port(), &token)?;
    let app = build_router_with_token(token, bridge, outputs);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let _guard = guard;
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
    });

    Ok(ApiServerHandle {
        address,
        shutdown: Some(shutdown_tx),
        task: Some(task),
    })
}

fn publish_discovery(
    config_dir: &Path,
    port: u16,
    token: &ApiToken,
) -> Result<DiscoveryGuard, ApiError> {
    create_private_dir(config_dir).map_err(io_api_error)?;

    let instance = random_hex(16)?;
    let token_contents = token.encoded().into_bytes();
    let port_contents = serde_json::to_vec(&DiscoveryPort {
        port,
        pid: std::process::id(),
        instance: instance.clone(),
    })
    .map_err(json_error)?;
    let token_path = config_dir.join(TOKEN_FILE_NAME);
    let port_path = config_dir.join(PORT_FILE_NAME);

    atomic_publish(&token_path, &token_contents, &instance).map_err(io_api_error)?;
    if let Err(error) = atomic_publish(&port_path, &port_contents, &instance) {
        remove_if_owned(&token_path, &token_contents);
        return Err(io_api_error(error));
    }

    Ok(DiscoveryGuard {
        token_path,
        token_contents,
        port_path,
        port_contents,
    })
}

fn create_private_dir(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_private_dir_metadata(path, &metadata)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = fs::DirBuilder::new();
                builder.recursive(true).mode(0o700).create(path)?;
            }
            #[cfg(not(unix))]
            fs::create_dir_all(path)?;
        }
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(path)?;
    validate_private_dir_metadata(path, &metadata)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn validate_private_dir_metadata(path: &Path, metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "discovery directory must not be a symbolic link: {}",
                path.display()
            ),
        ));
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("discovery path is not a directory: {}", path.display()),
        ));
    }
    Ok(())
}

fn atomic_publish(path: &Path, contents: &[u8], instance: &str) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing discovery parent"))?;
    let metadata = fs::symlink_metadata(parent)?;
    validate_private_dir_metadata(parent, &metadata)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid discovery path"))?;
    let temp_path = path.with_file_name(format!(".{file_name}.{instance}.tmp"));

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp_path)?;
    let publish_result = (|| {
        file.write_all(contents)?;
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        drop(file);
        fs::rename(&temp_path, path)
    })();
    if publish_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    publish_result
}

fn remove_if_owned(path: &Path, expected: &[u8]) {
    if fs::read(path).ok().as_deref() == Some(expected) {
        let _ = fs::remove_file(path);
    }
}

fn random_hex(bytes: usize) -> Result<String, ApiError> {
    let mut value = vec![0_u8; bytes];
    rand_bytes(&mut value).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "random_failed",
            "无法生成 API instance nonce",
        )
    })?;
    Ok(hex::encode(value))
}

fn io_api_error(error: io::Error) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "discovery_io_failed",
        format!("Native API discovery I/O 失败: {error}"),
    )
}
