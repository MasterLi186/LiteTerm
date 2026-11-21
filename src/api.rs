//! Loopback-only HTTP automation API primitives.
//!
//! The module deliberately does not depend on winit. `Bridge` accepts a small
//! dispatcher abstraction so `main.rs` can turn a [`MainThreadCall`] into its
//! own `UserEvent::Api` without giving the HTTP runtime access to UI state.

use std::{
    collections::{HashMap, VecDeque},
    fmt,
    fs::{self, OpenOptions},
    io::{self, Write},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant},
};

use axum::{
    body::{to_bytes, Body, Bytes},
    extract::{FromRequest, FromRequestParts, Path as AxumPath, Query, Request, State},
    http::{header::AUTHORIZATION, request::Parts, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use openssl::rand::rand_bytes;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio::{
    net::TcpListener,
    sync::oneshot,
    task::JoinHandle,
    time::{timeout, timeout_at, Instant as TokioInstant},
};
use tower::limit::ConcurrencyLimitLayer;

pub const OUTPUT_CAPACITY: usize = 1024 * 1024;
pub const MAX_READ_BYTES: usize = 256 * 1024;
pub const MAX_BODY_BYTES: usize = 64 * 1024;
pub const MAX_CONCURRENT_REQUESTS: usize = 32;
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
pub const BODY_READ_TIMEOUT: Duration = Duration::from_secs(5);
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
pub const TOKEN_FILE_NAME: &str = "native-api-token";
pub const PORT_FILE_NAME: &str = "native-api-port";

// ---------------------------------------------------------------------------
// Typed requests, operations, and replies
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct OpenLocalRequest {
    pub shell_path: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct OpenSshRequest {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub user: String,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub auth_method: Option<String>,
    #[serde(default)]
    pub key_path: Option<String>,
    #[serde(default)]
    pub proxy_jump: Option<String>,
}

fn default_ssh_port() -> u16 {
    22
}

impl fmt::Debug for OpenSshRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenSshRequest")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("auth_method", &self.auth_method)
            .field("key_path", &self.key_path.as_ref().map(|_| "[REDACTED]"))
            .field("proxy_jump", &self.proxy_jump)
            .finish()
    }
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct WriteRequest {
    pub data: String,
}

impl fmt::Debug for WriteRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteRequest")
            .field(
                "data",
                &format_args!("[REDACTED; {} bytes]", self.data.len()),
            )
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PaneDto {
    pub id: String,
    #[serde(default)]
    pub active: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TabDto {
    pub id: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panes: Vec<PaneDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_pane_id: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub enum ApiOperation {
    ListTabs,
    OpenLocal(OpenLocalRequest),
    OpenSsh(OpenSshRequest),
    Focus {
        tab_id: String,
        pane_id: Option<String>,
    },
    Write {
        tab_id: String,
        pane_id: Option<String>,
        data: Vec<u8>,
    },
    Close {
        tab_id: String,
        pane_id: Option<String>,
    },
    ResolvePane {
        tab_id: String,
    },
}

impl fmt::Debug for ApiOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ListTabs => f.write_str("ListTabs"),
            Self::OpenLocal(request) => f.debug_tuple("OpenLocal").field(request).finish(),
            Self::OpenSsh(request) => f.debug_tuple("OpenSsh").field(request).finish(),
            Self::Focus { tab_id, pane_id } => f
                .debug_struct("Focus")
                .field("tab_id", tab_id)
                .field("pane_id", pane_id)
                .finish(),
            Self::Write {
                tab_id,
                pane_id,
                data,
            } => f
                .debug_struct("Write")
                .field("tab_id", tab_id)
                .field("pane_id", pane_id)
                .field("data", &format_args!("[REDACTED; {} bytes]", data.len()))
                .finish(),
            Self::Close { tab_id, pane_id } => f
                .debug_struct("Close")
                .field("tab_id", tab_id)
                .field("pane_id", pane_id)
                .finish(),
            Self::ResolvePane { tab_id } => f
                .debug_struct("ResolvePane")
                .field("tab_id", tab_id)
                .finish(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiReply {
    Tabs(Vec<TabDto>),
    Opened(TabDto),
    Focused { tab_id: String, pane_id: String },
    Written { bytes: usize },
    Closed,
    PaneResolved { pane_id: String },
}

pub type ApiReplyResult = Result<ApiReply, ApiError>;

// ---------------------------------------------------------------------------
// Redacted errors and deadline-aware main-thread bridge
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "bridge_unavailable",
            message,
        )
    }

    pub fn timeout() -> Self {
        Self::new(
            StatusCode::GATEWAY_TIMEOUT,
            "main_thread_timeout",
            "主线程响应超时",
        )
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Debug for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiError")
            .field("status", &self.status)
            .field("code", &self.code)
            .field("message", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": self.message,
            "code": self.code,
        });
        (self.status, Json(body)).into_response()
    }
}

pub struct ApiResponseSender {
    deadline: Instant,
    sender: oneshot::Sender<ApiReplyResult>,
}

impl ApiResponseSender {
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// Replies only while the original absolute deadline is still live.
    ///
    /// A late UI event is intentionally discarded instead of being mistaken
    /// for a response to a newer request.
    pub fn respond(self, reply: ApiReplyResult) -> Result<(), ApiReplyResult> {
        if self.is_expired() {
            return Err(reply);
        }
        self.sender.send(reply)
    }
}

impl fmt::Debug for ApiResponseSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiResponseSender")
            .field("expired", &self.is_expired())
            .finish_non_exhaustive()
    }
}

pub struct MainThreadCall {
    operation: ApiOperation,
    response: ApiResponseSender,
}

impl MainThreadCall {
    pub fn deadline(&self) -> Instant {
        self.response.deadline()
    }

    pub fn is_expired(&self) -> bool {
        self.response.is_expired()
    }

    /// Obtains work only if its absolute deadline is still current.
    ///
    /// Callers must use this as the first step on the UI thread. An expired
    /// call drops its operation, so stale mutations cannot accidentally run.
    pub fn into_current_parts(self) -> Result<(ApiOperation, ApiResponseSender), ExpiredCall> {
        if self.is_expired() {
            Err(ExpiredCall)
        } else {
            Ok((self.operation, self.response))
        }
    }

    pub fn respond(self, reply: ApiReplyResult) -> Result<(), ApiReplyResult> {
        self.response.respond(reply)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpiredCall;

impl fmt::Display for ExpiredCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("main-thread API call expired")
    }
}

impl std::error::Error for ExpiredCall {}

impl fmt::Debug for MainThreadCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MainThreadCall")
            .field("operation", &self.operation)
            .field("expired", &self.is_expired())
            .finish()
    }
}

pub trait ApiEventDispatcher: Send + Sync + 'static {
    /// Dispatches ownership of the call into the UI event queue.
    ///
    /// On a closed event loop, return the original call so its reply channel is
    /// closed immediately and no sensitive operation needs to be cloned.
    fn dispatch(&self, call: MainThreadCall) -> Result<(), Box<MainThreadCall>>;
}

impl<F> ApiEventDispatcher for F
where
    F: Fn(MainThreadCall) -> Result<(), Box<MainThreadCall>> + Send + Sync + 'static,
{
    fn dispatch(&self, call: MainThreadCall) -> Result<(), Box<MainThreadCall>> {
        self(call)
    }
}

#[derive(Clone)]
pub struct Bridge {
    dispatcher: Arc<dyn ApiEventDispatcher>,
    timeout: Duration,
}

impl Bridge {
    pub fn new<D>(dispatcher: D, timeout: Duration) -> Self
    where
        D: ApiEventDispatcher,
    {
        Self {
            dispatcher: Arc::new(dispatcher),
            timeout,
        }
    }

    pub fn with_default_timeout<D>(dispatcher: D) -> Self
    where
        D: ApiEventDispatcher,
    {
        Self::new(dispatcher, DEFAULT_REQUEST_TIMEOUT)
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub async fn call(&self, operation: ApiOperation) -> ApiReplyResult {
        let now = Instant::now();
        let deadline = now.checked_add(self.timeout).unwrap_or(now);
        self.call_with_deadline(operation, deadline).await
    }

    pub async fn call_with_deadline(
        &self,
        operation: ApiOperation,
        deadline: Instant,
    ) -> ApiReplyResult {
        if Instant::now() >= deadline {
            return Err(ApiError::timeout());
        }

        let (sender, receiver) = oneshot::channel();
        let call = MainThreadCall {
            operation,
            response: ApiResponseSender { deadline, sender },
        };
        if self.dispatcher.dispatch(call).is_err() {
            return Err(ApiError::unavailable("主线程事件队列已关闭"));
        }

        match timeout_at(TokioInstant::from_std(deadline), receiver).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => Err(ApiError::unavailable("主线程未返回响应")),
            Err(_) => Err(ApiError::timeout()),
        }
    }
}

impl fmt::Debug for Bridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bridge")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Per-pane, reconnect-safe bounded output
// ---------------------------------------------------------------------------

#[path = "api/output.rs"]
mod output;
#[path = "api/server.rs"]
mod server;

#[cfg(test)]
use output::mutex_lock;
pub use output::*;
#[cfg(test)]
use server::ApiToken;
pub use server::*;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::{Method, Request as HttpRequest},
    };
    use serde_json::Value;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    fn tab() -> TabDto {
        TabDto {
            id: "tab-1".into(),
            label: "本地终端 1".into(),
            kind: "local".into(),
            panes: vec![PaneDto {
                id: "pane-1".into(),
                active: true,
            }],
            active_pane_id: Some("pane-1".into()),
        }
    }

    fn immediate_bridge(dispatches: Arc<AtomicUsize>) -> Bridge {
        Bridge::new(
            move |call: MainThreadCall| {
                dispatches.fetch_add(1, Ordering::SeqCst);
                let (operation, response) = call
                    .into_current_parts()
                    .expect("test call must be current");
                let reply = match operation {
                    ApiOperation::ListTabs => ApiReply::Tabs(vec![tab()]),
                    ApiOperation::OpenLocal(_) | ApiOperation::OpenSsh(_) => {
                        ApiReply::Opened(tab())
                    }
                    ApiOperation::Focus { .. } => ApiReply::Focused {
                        tab_id: "tab-1".into(),
                        pane_id: "pane-1".into(),
                    },
                    ApiOperation::Write { data, .. } => ApiReply::Written { bytes: data.len() },
                    ApiOperation::Close { .. } => ApiReply::Closed,
                    ApiOperation::ResolvePane { .. } => ApiReply::PaneResolved {
                        pane_id: "pane-1".into(),
                    },
                };
                let _ = response.respond(Ok(reply));
                Ok(())
            },
            Duration::from_secs(1),
        )
    }

    fn authorized_request(method: Method, uri: &str, body: Body) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method(method)
            .uri(uri)
            .header(AUTHORIZATION, format!("Bearer {}", hex::encode([7_u8; 32])))
            .header("content-type", "application/json")
            .body(body)
            .unwrap()
    }

    async fn json_body(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn output_ring_wraps_and_uses_raw_byte_cursors() {
        let mut ring = OutputRing::with_capacity(9, 5);
        ring.append(b"abc");
        ring.append(&[0xe4, 0xb8, 0xad, b'Z']);
        assert_eq!(ring.cursor_range(), (2, 7));

        let read = ring.read(Some(9), Some(0), MAX_READ_BYTES);
        assert_eq!(read.data, &[b'c', 0xe4, 0xb8, 0xad, b'Z']);
        assert_eq!(read.cursor, 7);
        assert!(read.truncated);

        let limited = ring.read(Some(9), Some(3), 2);
        assert_eq!(limited.data, &[0xe4, 0xb8]);
        assert_eq!(limited.cursor, 5);
        assert!(!limited.truncated);
    }

    #[test]
    fn text_read_completes_utf8_without_replacement_at_small_or_arbitrary_cursors() {
        let mut ring = OutputRing::new(3);
        ring.append("中文Z".as_bytes());

        let first = ring.read_text(Some(3), Some(0), 2, false);
        assert_eq!(first.data, "中");
        assert_eq!(first.cursor, 3);
        assert!(!first.data.contains('\u{fffd}'));

        let middle = ring.read_text(Some(3), Some(1), 2, false);
        assert_eq!(middle.data, "文");
        assert_eq!(middle.cursor, 6);
        assert!(!middle.data.contains('\u{fffd}'));

        let second = ring.read_text(Some(3), Some(first.cursor), 2, false);
        assert_eq!(second.data, "文");
        assert_eq!(second.cursor, 6);
    }

    #[test]
    fn text_read_keeps_an_incomplete_utf8_tail_for_a_later_append() {
        let mut ring = OutputRing::new(8);
        ring.append(&[0xe4, 0xb8]);

        let incomplete = ring.read_text(Some(8), Some(0), MAX_READ_BYTES, true);
        assert_eq!(incomplete.data, "");
        assert_eq!(incomplete.cursor, 0);

        ring.append(&[0xad]);
        let completed = ring.read_text(Some(8), Some(incomplete.cursor), MAX_READ_BYTES, true);
        assert_eq!(completed.data, "中");
        assert_eq!(completed.cursor, 3);

        let mut limited_ring = OutputRing::new(9);
        limited_ring.append("中文".as_bytes());
        let limited = limited_ring.read_text(Some(9), Some(0), 2, true);
        assert_eq!(limited.data, "中");
        assert_eq!(limited.cursor, 3);
    }

    #[test]
    fn ansi_filter_never_exposes_control_suffixes_across_cursor_splits() {
        let mut ring = OutputRing::new(4);
        let bytes = b"\x1b[31mRED\x1b]0;secret title\x07!\x1b[0m";
        ring.append(bytes);

        let mut cursor = 0;
        let mut text = String::new();
        while cursor < bytes.len() as u64 {
            let chunk = ring.read_text(Some(4), Some(cursor), 2, true);
            assert!(chunk.cursor > cursor);
            assert!(!chunk.data.contains("[31m"));
            assert!(!chunk.data.contains("0;secret"));
            text.push_str(&chunk.data);
            cursor = chunk.cursor;
        }
        assert_eq!(text, "RED!");

        let inside_csi = ring.read_text(Some(4), Some(2), 4, true);
        assert!(!inside_csi.data.contains("[31m"));
    }

    #[test]
    fn reconnect_replaces_ring_and_old_sink_cannot_pollute_it() {
        let registry = OutputRegistry::new();
        let old = registry.begin_stream_with_capacity("tab", "pane", 16);
        old.append(b"old");
        let fresh = registry.begin_stream_with_capacity("tab", "pane", 16);
        assert_ne!(old.stream_id(), fresh.stream_id());
        fresh.append(b"new");
        old.append(b"-late");

        let read = registry
            .read("tab", "pane", Some(old.stream_id()), Some(3), 16)
            .unwrap();
        assert_eq!(read.data, b"new");
        assert_eq!(read.stream_id, fresh.stream_id());
        assert!(read.truncated);
    }

    #[test]
    fn one_read_is_capped_at_256_kib() {
        let mut ring = OutputRing::new(1);
        ring.append(&vec![b'x'; MAX_READ_BYTES + 17]);
        assert_eq!(
            ring.read(Some(1), Some(0), usize::MAX).data.len(),
            MAX_READ_BYTES
        );
    }

    #[tokio::test]
    async fn timed_out_call_cannot_expose_its_operation() {
        let held = Arc::new(Mutex::new(None::<MainThreadCall>));
        let held_by_dispatch = Arc::clone(&held);
        let bridge = Bridge::new(
            move |call: MainThreadCall| {
                *mutex_lock(&held_by_dispatch) = Some(call);
                Ok(())
            },
            Duration::from_millis(15),
        );

        let error = bridge.call(ApiOperation::ListTabs).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::GATEWAY_TIMEOUT);
        let late = mutex_lock(&held).take().unwrap();
        assert_eq!(late.into_current_parts().unwrap_err(), ExpiredCall);
    }

    #[tokio::test]
    async fn response_sender_rejects_a_late_reply() {
        let held = Arc::new(Mutex::new(None::<ApiResponseSender>));
        let held_by_dispatch = Arc::clone(&held);
        let bridge = Bridge::new(
            move |call: MainThreadCall| {
                let (_, response) = call.into_current_parts().unwrap();
                *mutex_lock(&held_by_dispatch) = Some(response);
                Ok(())
            },
            Duration::from_millis(15),
        );
        assert_eq!(
            bridge
                .call(ApiOperation::ListTabs)
                .await
                .unwrap_err()
                .status(),
            StatusCode::GATEWAY_TIMEOUT
        );
        let late = mutex_lock(&held).take().unwrap();
        assert!(late.respond(Ok(ApiReply::Tabs(vec![]))).is_err());
    }

    #[tokio::test]
    async fn already_expired_deadline_does_not_dispatch() {
        let dispatches = Arc::new(AtomicUsize::new(0));
        let bridge = immediate_bridge(Arc::clone(&dispatches));
        let error = bridge
            .call_with_deadline(ApiOperation::ListTabs, Instant::now())
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(dispatches.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn closed_bridge_reports_service_unavailable() {
        let bridge = Bridge::new(|call| Err(Box::new(call)), Duration::from_secs(1));
        let error = bridge.call(ApiOperation::ListTabs).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn debug_redacts_password_key_data_token_and_error_detail() {
        let operation = ApiOperation::OpenSsh(OpenSshRequest {
            host: "host".into(),
            port: 22,
            user: "user".into(),
            password: Some("super-secret-password".into()),
            auth_method: Some("key".into()),
            key_path: Some("/private/key/path".into()),
            proxy_jump: None,
        });
        let ssh_debug = format!("{operation:?}");
        assert!(!ssh_debug.contains("super-secret-password"));
        assert!(!ssh_debug.contains("/private/key/path"));

        let write = ApiOperation::Write {
            tab_id: "tab".into(),
            pane_id: None,
            data: b"secret terminal data".to_vec(),
        };
        assert!(!format!("{write:?}").contains("secret terminal data"));
        assert!(!format!("{:?}", ApiToken::from_bytes([7; 32])).contains(&hex::encode([7; 32])));
        assert!(
            !format!("{:?}", ApiError::bad_request("private detail")).contains("private detail")
        );
    }

    #[tokio::test]
    async fn auth_runs_before_body_extraction_and_bad_auth_never_dispatches() {
        let dispatches = Arc::new(AtomicUsize::new(0));
        let app = build_router(
            [7; 32],
            immediate_bridge(Arc::clone(&dispatches)),
            OutputRegistry::new(),
        );
        let request = HttpRequest::builder()
            .method(Method::POST)
            .uri("/api/v1/tabs/local")
            .header(AUTHORIZATION, "Bearer wrong")
            .body(Body::from(vec![b'x'; MAX_BODY_BYTES + 100]))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(dispatches.load(Ordering::SeqCst), 0);
        assert!(json_body(response).await.get("error").is_some());
    }

    #[tokio::test]
    async fn valid_bearer_auth_dispatches_legacy_list_endpoint() {
        let dispatches = Arc::new(AtomicUsize::new(0));
        let app = build_router(
            [7; 32],
            immediate_bridge(Arc::clone(&dispatches)),
            OutputRegistry::new(),
        );
        let response = app
            .oneshot(authorized_request(
                Method::GET,
                "/api/v1/tabs",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_body(response).await[0]["id"], "tab-1");
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn body_limit_is_json_and_oversize_body_never_dispatches() {
        let dispatches = Arc::new(AtomicUsize::new(0));
        let app = build_router(
            [7; 32],
            immediate_bridge(Arc::clone(&dispatches)),
            OutputRegistry::new(),
        );
        let request = authorized_request(
            Method::POST,
            "/api/v1/tabs/local",
            Body::from(vec![b'x'; MAX_BODY_BYTES + 1]),
        );
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(dispatches.load(Ordering::SeqCst), 0);
        let json = json_body(response).await;
        assert_eq!(json["code"], "body_too_large");
    }

    #[tokio::test]
    async fn malformed_query_and_unknown_route_use_json_errors_without_dispatch() {
        let dispatches = Arc::new(AtomicUsize::new(0));
        let app = build_router(
            [7; 32],
            immediate_bridge(Arc::clone(&dispatches)),
            OutputRegistry::new(),
        );
        let malformed = app
            .clone()
            .oneshot(authorized_request(
                Method::GET,
                "/api/v1/tabs/tab-1/read?cursor=not-a-number",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_body(malformed).await["code"], "bad_request");

        let missing = app
            .oneshot(authorized_request(
                Method::GET,
                "/api/v1/missing",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(json_body(missing).await["code"], "not_found");
        assert_eq!(dispatches.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn output_read_bypasses_ui_dispatch_when_pane_is_known() {
        let dispatches = Arc::new(AtomicUsize::new(0));
        let outputs = OutputRegistry::new();
        let sink = outputs.begin_stream("tab-1", "pane-1");
        sink.append(b"hello");
        let app = build_router([7; 32], immediate_bridge(Arc::clone(&dispatches)), outputs);
        let response = app
            .oneshot(authorized_request(
                Method::GET,
                "/api/v1/tabs/tab-1/read?pane_id=pane-1&limit=5",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_body(response).await["data"], "hello");
        assert_eq!(dispatches.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn output_read_without_pane_always_resolves_on_ui_thread() {
        let dispatches = Arc::new(AtomicUsize::new(0));
        let outputs = OutputRegistry::new();
        outputs.begin_stream("tab-1", "pane-1").append(b"resolved");
        let app = build_router([7; 32], immediate_bridge(Arc::clone(&dispatches)), outputs);
        let response = app
            .oneshot(authorized_request(
                Method::GET,
                "/api/v1/tabs/tab-1/read",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_body(response).await["data"], "resolved");
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn temp_discovery_is_private_loopback_and_removed_after_shutdown() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path().join("guishell");
        let server = start_server(
            ApiServerConfig::new(config_dir.clone(), 0),
            immediate_bridge(Arc::new(AtomicUsize::new(0))),
            OutputRegistry::new(),
        )
        .await
        .unwrap();
        assert_eq!(*server.address().ip(), Ipv4Addr::LOCALHOST);
        assert_ne!(server.address().port(), 0);

        let token_path = config_dir.join(TOKEN_FILE_NAME);
        let port_path = config_dir.join(PORT_FILE_NAME);
        assert_eq!(
            fs::metadata(&config_dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&token_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&port_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let token = fs::read_to_string(&token_path).unwrap();
        assert_eq!(token.len(), 64);
        let port: DiscoveryPort = serde_json::from_slice(&fs::read(&port_path).unwrap()).unwrap();
        assert_eq!(port.port, server.address().port());
        assert_eq!(port.pid, std::process::id());
        assert_eq!(port.instance.len(), 32);

        server.shutdown().await.unwrap();
        assert!(!token_path.exists());
        assert!(!port_path.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn discovery_rejects_a_symlink_config_directory() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        fs::create_dir(&real).unwrap();
        let linked = temp.path().join("linked");
        symlink(&real, &linked).unwrap();

        let error = start_server(
            ApiServerConfig::new(linked, 0),
            immediate_bridge(Arc::new(AtomicUsize::new(0))),
            OutputRegistry::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), "discovery_io_failed");
        assert!(!real.join(TOKEN_FILE_NAME).exists());
        assert!(!real.join(PORT_FILE_NAME).exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn guard_does_not_remove_another_instances_discovery_file() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path().join("guishell");
        let server = start_server(
            ApiServerConfig::new(config_dir.clone(), 0),
            immediate_bridge(Arc::new(AtomicUsize::new(0))),
            OutputRegistry::new(),
        )
        .await
        .unwrap();
        let token_path = config_dir.join(TOKEN_FILE_NAME);
        let port_path = config_dir.join(PORT_FILE_NAME);
        fs::write(&token_path, b"new-owner-token").unwrap();
        fs::write(&port_path, b"new-owner-port").unwrap();
        server.shutdown().await.unwrap();
        assert_eq!(fs::read(&token_path).unwrap(), b"new-owner-token");
        assert_eq!(fs::read(&port_path).unwrap(), b"new-owner-port");
    }
}
