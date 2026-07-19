//! Authenticated MCP Streamable HTTP transport.
//!
//! The JSON response mode is deliberately small: POST is fully implemented,
//! while GET/SSE is rejected with 405 because Plasmate does not yet emit
//! server-initiated requests or notifications. Legacy 2025 sessions and the
//! stateless 2026-07-28 release-candidate transport are kept separate.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, RwLock};
use tower_http::timeout::TimeoutLayer;
use tracing::info;

use super::protocol::{self, ProtocolState};
use super::server::{
    self, JsonRpcError, JsonRpcRequest, JsonRpcResponse, INVALID_REQUEST, METHOD_NOT_FOUND,
    PARSE_ERROR,
};
use super::sessions::SessionManager;
use crate::auth::capability::{bearer_matches, generate_token, validate_token};
use crate::cache::store::{CacheConfig, SomCache};
use crate::network::{fetch, security};

pub const TOKEN_ENV: &str = "PLASMATE_MCP_HTTP_TOKEN";
pub const DEFAULT_PORT: u16 = 9272;
const ENDPOINT: &str = "/mcp";
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const PROTOCOL_SESSION_IDLE_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_PROTOCOL_SESSIONS: usize = 128;

const MCP_PROTOCOL_VERSION: HeaderName = HeaderName::from_static("mcp-protocol-version");
const MCP_SESSION_ID: HeaderName = HeaderName::from_static("mcp-session-id");
const MCP_METHOD: HeaderName = HeaderName::from_static("mcp-method");
const MCP_NAME: HeaderName = HeaderName::from_static("mcp-name");

#[derive(Clone, Debug)]
pub struct McpHttpConfig {
    pub host: String,
    pub port: u16,
    pub token: Option<String>,
    pub allowed_origins: Vec<String>,
}

impl Default for McpHttpConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: DEFAULT_PORT,
            token: None,
            allowed_origins: Vec::new(),
        }
    }
}

#[derive(Clone)]
struct AppState {
    token: Arc<str>,
    allowed_origins: Arc<[HeaderValue]>,
    client: reqwest::Client,
    browser_sessions: Arc<SessionManager>,
    cache: Arc<SomCache>,
    protocol_sessions: Arc<RwLock<HashMap<String, Arc<ProtocolSession>>>>,
}

struct ProtocolSession {
    state: Mutex<ProtocolState>,
    last_used: std::sync::Mutex<Instant>,
}

impl ProtocolSession {
    fn new(state: ProtocolState) -> Self {
        Self {
            state: Mutex::new(state),
            last_used: std::sync::Mutex::new(Instant::now()),
        }
    }

    fn touch(&self) {
        if let Ok(mut last_used) = self.last_used.lock() {
            *last_used = Instant::now();
        }
    }

    fn is_expired_at(&self, now: Instant) -> bool {
        self.last_used
            .lock()
            .map(|last_used| now.saturating_duration_since(*last_used) >= PROTOCOL_SESSION_IDLE_TTL)
            .unwrap_or(true)
    }
}

fn prune_expired(sessions: &mut HashMap<String, Arc<ProtocolSession>>, now: Instant) {
    sessions.retain(|_, session| !session.is_expired_at(now));
}

fn has_protocol_session_capacity(sessions: &HashMap<String, Arc<ProtocolSession>>) -> bool {
    sessions.len() < MAX_PROTOCOL_SESSIONS
}

pub async fn run_server(config: McpHttpConfig) -> Result<(), Box<dyn std::error::Error>> {
    let (addr, app, generated_token) = prepare_server(config).await?;
    if let Some(token) = generated_token {
        eprintln!("MCP HTTP capability token: {token}");
    }
    eprintln!("MCP HTTP endpoint: http://{addr}{ENDPOINT}");
    let listener = TcpListener::bind(addr).await?;
    info!(%addr, "MCP Streamable HTTP server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn prepare_server(
    config: McpHttpConfig,
) -> Result<(SocketAddr, Router, Option<String>), Box<dyn std::error::Error>> {
    if !security::is_loopback_bind_host(&config.host) {
        return Err(format!(
            "MCP HTTP refuses non-loopback bind host '{}'; use 127.0.0.1, ::1, or localhost",
            config.host
        )
        .into());
    }
    let ip = if config.host.eq_ignore_ascii_case("localhost") {
        IpAddr::from([127, 0, 0, 1])
    } else {
        config.host.trim_matches(['[', ']']).parse::<IpAddr>()?
    };
    let addr = SocketAddr::new(ip, config.port);

    let environment_token = std::env::var(TOKEN_ENV).ok();
    let supplied_token = config.token.or(environment_token);
    let generated_token = supplied_token.is_none().then(generate_token);
    let token = supplied_token
        .or_else(|| generated_token.clone())
        .expect("token");
    validate_token(&token).map_err(|error| format!("invalid MCP HTTP token: {error}"))?;

    let allowed_origins = config
        .allowed_origins
        .iter()
        .map(|origin| validate_origin(origin))
        .collect::<Result<Vec<_>, _>>()?;
    let jar = Arc::new(reqwest::cookie::Jar::default());
    let client = fetch::build_client_h1_fallback(None, jar, None)?;
    let state = AppState {
        token: Arc::from(token),
        allowed_origins: Arc::from(allowed_origins),
        client,
        browser_sessions: Arc::new(SessionManager::new()),
        cache: Arc::new(SomCache::new(CacheConfig::default())),
        protocol_sessions: Arc::new(RwLock::new(HashMap::new())),
    };
    let app = build_router(state);
    Ok((addr, app, generated_token))
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route(
            ENDPOINT,
            post(handle_post).get(handle_get).delete(handle_delete),
        )
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        // Covers header-authenticated slow bodies as well as handler work.
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT + Duration::from_secs(5),
        ))
        // Authentication and Origin checks run before request-body extraction.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            authorize_request,
        ))
        .with_state(state)
}

fn validate_origin(origin: &str) -> Result<HeaderValue, Box<dyn std::error::Error>> {
    let parsed = url::Url::parse(origin)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("allowed MCP Origin must be an exact http(s) origin".into());
    }
    Ok(HeaderValue::from_str(origin.trim_end_matches('/'))?)
}

async fn authorize_request(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        if !state
            .allowed_origins
            .iter()
            .any(|allowed| allowed == origin)
        {
            return rpc_error_response(
                StatusCode::FORBIDDEN,
                None,
                INVALID_REQUEST,
                "Forbidden Origin",
                None,
            );
        }
    }
    let supplied = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if !bearer_matches(supplied, &state.token) {
        return rpc_error_response(
            StatusCode::UNAUTHORIZED,
            None,
            INVALID_REQUEST,
            "Missing or invalid bearer capability token",
            None,
        );
    }
    next.run(request).await
}

async fn handle_get() -> Response {
    method_not_allowed("POST, DELETE")
}

async fn handle_delete(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if header_text(&headers, &MCP_PROTOCOL_VERSION) == Some(protocol::MODERN_RC_VERSION) {
        return method_not_allowed("POST");
    }
    if let Err(response) = validate_stable_protocol_header(&headers, None) {
        return *response;
    }
    let Some(session_id) = header_text(&headers, &MCP_SESSION_ID) else {
        return rpc_error_response(
            StatusCode::BAD_REQUEST,
            None,
            INVALID_REQUEST,
            "Mcp-Session-Id is required",
            None,
        );
    };
    let removed = {
        let mut sessions = state.protocol_sessions.write().await;
        prune_expired(&mut sessions, Instant::now());
        sessions.remove(session_id)
    };
    if removed.is_none() {
        return rpc_error_response(
            StatusCode::NOT_FOUND,
            None,
            INVALID_REQUEST,
            "Unknown or terminated MCP session",
            None,
        );
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn handle_post(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    if !content_type_is_json(&headers) {
        return rpc_error_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            None,
            INVALID_REQUEST,
            "Content-Type must be application/json",
            None,
        );
    }
    if !accepts_required_types(&headers) {
        return rpc_error_response(
            StatusCode::NOT_ACCEPTABLE,
            None,
            INVALID_REQUEST,
            "Accept must include application/json and text/event-stream",
            None,
        );
    }

    let raw: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => {
            return rpc_error_response(
                StatusCode::BAD_REQUEST,
                None,
                PARSE_ERROR,
                &format!("Parse error: {error}"),
                None,
            )
        }
    };
    let request_id = raw.get("id").cloned();
    let request: JsonRpcRequest = match serde_json::from_value::<JsonRpcRequest>(raw.clone()) {
        Ok(request) if request.jsonrpc == "2.0" => request,
        Ok(_) => {
            return rpc_error_response(
                StatusCode::BAD_REQUEST,
                request_id,
                INVALID_REQUEST,
                "Invalid JSON-RPC version",
                None,
            )
        }
        Err(error) => {
            return rpc_error_response(
                StatusCode::BAD_REQUEST,
                request_id,
                INVALID_REQUEST,
                &format!("Invalid JSON-RPC request: {error}"),
                None,
            )
        }
    };

    let modern_version =
        protocol::request_protocol_version(request.params.as_ref()).map(str::to_owned);
    if request.method == "initialize" {
        if modern_version.is_some()
            || header_text(&headers, &MCP_PROTOCOL_VERSION) == Some(protocol::MODERN_RC_VERSION)
        {
            if let Err(response) =
                validate_modern_headers(&headers, &request, modern_version.as_deref())
            {
                return *response;
            }
            return rpc_error_response(
                StatusCode::NOT_FOUND,
                request.id,
                METHOD_NOT_FOUND,
                "Method not found: initialize is not part of MCP 2026-07-28",
                None,
            );
        }
        return handle_stable_initialize(&state, &headers, request).await;
    }

    if modern_version.is_some()
        || header_text(&headers, &MCP_PROTOCOL_VERSION) == Some(protocol::MODERN_RC_VERSION)
    {
        handle_modern_request(&state, &headers, request, modern_version.as_deref()).await
    } else {
        handle_stable_request(&state, &headers, request).await
    }
}

async fn handle_stable_initialize(
    state: &AppState,
    headers: &HeaderMap,
    request: JsonRpcRequest,
) -> Response {
    if headers.contains_key(&MCP_SESSION_ID) {
        return rpc_error_response(
            StatusCode::BAD_REQUEST,
            request.id,
            INVALID_REQUEST,
            "Mcp-Session-Id must not be supplied during initialize",
            None,
        );
    }
    let body_version = request
        .params
        .as_ref()
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str);
    if body_version != Some(protocol::STABLE_VERSION) {
        return rpc_error_response(
            StatusCode::BAD_REQUEST,
            request.id,
            INVALID_REQUEST,
            "Streamable HTTP initialize supports only MCP 2025-11-25",
            Some(json!({
                "supported": [protocol::STABLE_VERSION],
                "requested": body_version
            })),
        );
    }
    if let Some(version) = header_text(headers, &MCP_PROTOCOL_VERSION) {
        if body_version != Some(version) {
            return header_mismatch(request.id, "MCP-Protocol-Version does not match initialize");
        }
    }

    let mut protocol_state = ProtocolState::default();
    let response = server::handle_request(
        &request,
        &state.client,
        &state.browser_sessions,
        &state.cache,
        &mut protocol_state,
    )
    .await;
    if response.error.is_some() {
        return (StatusCode::OK, Json(response)).into_response();
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    {
        let mut sessions = state.protocol_sessions.write().await;
        prune_expired(&mut sessions, Instant::now());
        if !has_protocol_session_capacity(&sessions) {
            return rpc_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                request.id,
                -32603,
                "MCP protocol session capacity reached; terminate an existing session",
                Some(json!({ "maxSessions": MAX_PROTOCOL_SESSIONS })),
            );
        }
        sessions.insert(
            session_id.clone(),
            Arc::new(ProtocolSession::new(protocol_state)),
        );
    }
    let mut http_response = (StatusCode::OK, Json(response)).into_response();
    if let Ok(value) = HeaderValue::from_str(&session_id) {
        http_response.headers_mut().insert(MCP_SESSION_ID, value);
    }
    http_response
}

async fn handle_stable_request(
    state: &AppState,
    headers: &HeaderMap,
    request: JsonRpcRequest,
) -> Response {
    if let Err(response) = validate_stable_protocol_header(headers, request.id.clone()) {
        return *response;
    }
    let Some(session_id) = header_text(headers, &MCP_SESSION_ID) else {
        return rpc_error_response(
            StatusCode::BAD_REQUEST,
            request.id,
            INVALID_REQUEST,
            "Mcp-Session-Id is required after initialize",
            None,
        );
    };
    let protocol_session = {
        let mut sessions = state.protocol_sessions.write().await;
        prune_expired(&mut sessions, Instant::now());
        sessions.get(session_id).cloned()
    };
    let Some(protocol_session) = protocol_session else {
        return rpc_error_response(
            StatusCode::NOT_FOUND,
            request.id,
            INVALID_REQUEST,
            "Unknown or terminated MCP session",
            None,
        );
    };
    protocol_session.touch();

    let is_notification = request.id.is_none();
    let response = {
        let mut protocol_state = protocol_session.state.lock().await;
        tokio::time::timeout(
            REQUEST_TIMEOUT,
            server::handle_request(
                &request,
                &state.client,
                &state.browser_sessions,
                &state.cache,
                &mut protocol_state,
            ),
        )
        .await
    };
    if is_notification {
        return match response {
            Ok(response) if response.error.is_none() => StatusCode::ACCEPTED.into_response(),
            Ok(response) => (StatusCode::BAD_REQUEST, Json(response)).into_response(),
            Err(_) => timeout_response(request.id),
        };
    }
    match response {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(_) => timeout_response(request.id),
    }
}

async fn handle_modern_request(
    state: &AppState,
    headers: &HeaderMap,
    request: JsonRpcRequest,
    body_version: Option<&str>,
) -> Response {
    if let Err(response) = validate_modern_headers(headers, &request, body_version) {
        return *response;
    }
    let mut protocol_state = ProtocolState::default();
    let is_notification = request.id.is_none();
    let response = tokio::time::timeout(
        REQUEST_TIMEOUT,
        server::handle_request(
            &request,
            &state.client,
            &state.browser_sessions,
            &state.cache,
            &mut protocol_state,
        ),
    )
    .await;
    if is_notification {
        return match response {
            Ok(response) if response.error.is_none() => StatusCode::ACCEPTED.into_response(),
            Ok(response) => (StatusCode::BAD_REQUEST, Json(response)).into_response(),
            Err(_) => timeout_response(request.id),
        };
    }
    match response {
        Ok(response)
            if response
                .error
                .as_ref()
                .is_some_and(|error| error.code == METHOD_NOT_FOUND) =>
        {
            (StatusCode::NOT_FOUND, Json(response)).into_response()
        }
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(_) => timeout_response(request.id),
    }
}

fn validate_stable_protocol_header(
    headers: &HeaderMap,
    id: Option<Value>,
) -> Result<(), Box<Response>> {
    match header_text(headers, &MCP_PROTOCOL_VERSION) {
        Some(protocol::STABLE_VERSION) => Ok(()),
        Some(version) => Err(Box::new(rpc_error_response(
            StatusCode::BAD_REQUEST,
            id,
            INVALID_REQUEST,
            &format!("Unsupported MCP-Protocol-Version for initialized session: {version}"),
            None,
        ))),
        None => Err(Box::new(rpc_error_response(
            StatusCode::BAD_REQUEST,
            id,
            INVALID_REQUEST,
            "MCP-Protocol-Version is required after initialize",
            None,
        ))),
    }
}

fn validate_modern_headers(
    headers: &HeaderMap,
    request: &JsonRpcRequest,
    body_version: Option<&str>,
) -> Result<(), Box<Response>> {
    let id = request.id.clone();
    let Some(header_version) = header_text(headers, &MCP_PROTOCOL_VERSION) else {
        return Err(Box::new(header_mismatch(
            id,
            "MCP-Protocol-Version is required",
        )));
    };
    let Some(body_version) = body_version else {
        return Err(Box::new(header_mismatch(
            id,
            "MCP-Protocol-Version has no matching request _meta value",
        )));
    };
    if header_version != body_version {
        return Err(Box::new(header_mismatch(
            id,
            "MCP-Protocol-Version does not match request _meta",
        )));
    }
    if body_version != protocol::MODERN_RC_VERSION {
        return Err(Box::new(rpc_error_response(
            StatusCode::BAD_REQUEST,
            id,
            -32022,
            "Unsupported protocol version",
            Some(json!({
                "supported": [protocol::MODERN_RC_VERSION],
                "requested": body_version
            })),
        )));
    }
    if header_text(headers, &MCP_METHOD) != Some(request.method.as_str()) {
        return Err(Box::new(header_mismatch(
            request.id.clone(),
            "Mcp-Method is missing or does not match the request method",
        )));
    }
    let name_source = match request.method.as_str() {
        "tools/call" | "prompts/get" => Some("name"),
        "resources/read" => Some("uri"),
        _ => None,
    };
    if let Some(source_field) = name_source {
        let body_name = request
            .params
            .as_ref()
            .and_then(|params| params.get(source_field).and_then(Value::as_str));
        let header_name = headers
            .get(&MCP_NAME)
            .and_then(|value| value.to_str().ok())
            .and_then(decode_header_value);
        if header_name.as_deref() != body_name {
            return Err(Box::new(header_mismatch(
                request.id.clone(),
                &format!("Mcp-Name is missing, malformed, or does not match params.{source_field}"),
            )));
        }
    }
    Ok(())
}

fn decode_header_value(value: &str) -> Option<String> {
    if value.starts_with("=?base64?") && value.ends_with("?=") {
        let encoded = &value[9..value.len().checked_sub(2)?];
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()?;
        String::from_utf8(bytes).ok()
    } else if value.is_ascii()
        && !value.starts_with(char::is_whitespace)
        && !value.ends_with(char::is_whitespace)
    {
        Some(value.to_string())
    } else {
        None
    }
}

fn header_mismatch(id: Option<Value>, message: &str) -> Response {
    rpc_error_response(
        StatusCode::BAD_REQUEST,
        id,
        -32020,
        &format!("Header mismatch: {message}"),
        None,
    )
}

fn timeout_response(id: Option<Value>) -> Response {
    rpc_error_response(
        StatusCode::GATEWAY_TIMEOUT,
        id,
        -32603,
        "MCP request exceeded the 60 second transport timeout",
        None,
    )
}

fn rpc_error_response(
    status: StatusCode,
    id: Option<Value>,
    code: i32,
    message: &str,
    data: Option<Value>,
) -> Response {
    (
        status,
        Json(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.to_string(),
                data,
            }),
        }),
    )
        .into_response()
}

fn method_not_allowed(allow: &'static str) -> Response {
    let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static(allow));
    response
}

fn header_text<'a>(headers: &'a HeaderMap, name: &HeaderName) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn content_type_is_json(headers: &HeaderMap) -> bool {
    header_text(headers, &header::CONTENT_TYPE)
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

fn accepts_required_types(headers: &HeaderMap) -> bool {
    let values = headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|value| value.split(';').next())
        .map(str::trim)
        .collect::<Vec<_>>();
    values
        .iter()
        .any(|value| value.eq_ignore_ascii_case("application/json") || *value == "*/*")
        && values
            .iter()
            .any(|value| value.eq_ignore_ascii_case("text/event-stream") || *value == "*/*")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Method, Request};
    use tower::ServiceExt;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    async fn app(origins: &[&str]) -> Router {
        let (_, app, _) = prepare_server(McpHttpConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            token: Some(TOKEN.to_string()),
            allowed_origins: origins.iter().map(|value| (*value).to_string()).collect(),
        })
        .await
        .unwrap();
        app
    }

    fn request(method: Method) -> axum::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(ENDPOINT)
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream")
    }

    async fn json(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), MAX_REQUEST_BYTES)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn modern_params() -> Value {
        json!({
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": protocol::MODERN_RC_VERSION,
                "io.modelcontextprotocol/clientInfo": { "name": "http-test", "version": "1" },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        })
    }

    fn modern_request(id: u64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[tokio::test]
    async fn rejects_cross_origin_missing_and_wrong_tokens() {
        let app = app(&["https://allowed.example"]).await;
        let response = app
            .clone()
            .oneshot(
                request(Method::POST)
                    .header(header::ORIGIN, "https://evil.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(ENDPOINT)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(ENDPOINT)
                    .header(header::AUTHORIZATION, "Bearer wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn accepts_exact_allowed_origin_and_rejects_malformed_origin() {
        let app = app(&["https://allowed.example"]).await;
        let body = modern_request(1, "tools/list", modern_params());
        let accepted = app
            .clone()
            .oneshot(
                request(Method::POST)
                    .header(header::ORIGIN, "https://allowed.example")
                    .header(MCP_PROTOCOL_VERSION, protocol::MODERN_RC_VERSION)
                    .header(MCP_METHOD, "tools/list")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);

        let rejected = app
            .oneshot(
                request(Method::POST)
                    .header(header::ORIGIN, "null")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn body_limit_runs_after_auth_and_before_json_parsing() {
        let response = app(&[])
            .await
            .oneshot(
                request(Method::POST)
                    .body(Body::from(vec![b'x'; MAX_REQUEST_BYTES + 1]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn modern_header_mismatch_and_unsupported_version_are_exact_errors() {
        let app = app(&[]).await;
        let body = modern_request(1, "tools/list", modern_params());
        let response = app
            .clone()
            .oneshot(
                request(Method::POST)
                    .header(MCP_PROTOCOL_VERSION, protocol::MODERN_RC_VERSION)
                    .header(MCP_METHOD, "tools/call")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json(response).await["error"]["code"], -32020);

        let mut params = modern_params();
        params["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("2099-01-01");
        let body = modern_request(2, "tools/list", params);
        let response = app
            .clone()
            .oneshot(
                request(Method::POST)
                    .header(MCP_PROTOCOL_VERSION, "2099-01-01")
                    .header(MCP_METHOD, "tools/list")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = json(response).await;
        assert_eq!(body["error"]["code"], -32022);
        assert_eq!(body["error"]["data"]["requested"], "2099-01-01");

        let mut params = modern_params();
        params["uri"] = json!("file:///body");
        let body = modern_request(3, "resources/read", params);
        let response = app
            .oneshot(
                request(Method::POST)
                    .header(MCP_PROTOCOL_VERSION, protocol::MODERN_RC_VERSION)
                    .header(MCP_METHOD, "resources/read")
                    .header(MCP_NAME, "file:///header")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(json(response).await["error"]["code"], -32020);
    }

    #[tokio::test]
    async fn modern_initialize_is_rejected_as_an_unknown_method() {
        let app = app(&[]).await;
        let body = modern_request(1, "initialize", modern_params());
        let response = app
            .oneshot(
                request(Method::POST)
                    .header(MCP_PROTOCOL_VERSION, protocol::MODERN_RC_VERSION)
                    .header(MCP_METHOD, "initialize")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(json(response).await["error"]["code"], METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn modern_is_stateless_and_validates_tool_name() {
        let app = app(&[]).await;
        let mut params = modern_params();
        params["name"] = json!("cache_status");
        params["arguments"] = json!({});
        let body = modern_request(1, "tools/call", params);
        let response = app
            .clone()
            .oneshot(
                request(Method::POST)
                    .header(MCP_PROTOCOL_VERSION, protocol::MODERN_RC_VERSION)
                    .header(MCP_METHOD, "tools/call")
                    .header(MCP_NAME, "wrong")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(json(response).await["error"]["code"], -32020);

        let response = app
            .oneshot(
                request(Method::POST)
                    .header(MCP_PROTOCOL_VERSION, protocol::MODERN_RC_VERSION)
                    .header(MCP_METHOD, "tools/call")
                    .header(MCP_NAME, "cache_status")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json(response).await["result"]["resultType"], "complete");
    }

    #[tokio::test]
    async fn stable_session_rejects_fixation_missing_unknown_and_deleted_sessions() {
        let app = app(&[]).await;
        let initialize = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": protocol::STABLE_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "http-test", "version": "1" }
            }
        });
        let fixed = app
            .clone()
            .oneshot(
                request(Method::POST)
                    .header(MCP_SESSION_ID, "attacker-chosen")
                    .body(Body::from(initialize.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(fixed.status(), StatusCode::BAD_REQUEST);

        let initialized = app
            .clone()
            .oneshot(
                request(Method::POST)
                    .body(Body::from(initialize.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(initialized.status(), StatusCode::OK);
        let session = initialized
            .headers()
            .get(&MCP_SESSION_ID)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let notification = json!({
            "jsonrpc": "2.0", "method": "notifications/initialized", "params": {}
        });
        let missing = app
            .clone()
            .oneshot(
                request(Method::POST)
                    .header(MCP_PROTOCOL_VERSION, protocol::STABLE_VERSION)
                    .body(Body::from(notification.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

        let unknown = app
            .clone()
            .oneshot(
                request(Method::POST)
                    .header(MCP_PROTOCOL_VERSION, protocol::STABLE_VERSION)
                    .header(MCP_SESSION_ID, "unknown")
                    .body(Body::from(notification.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

        let accepted = app
            .clone()
            .oneshot(
                request(Method::POST)
                    .header(MCP_PROTOCOL_VERSION, protocol::STABLE_VERSION)
                    .header(MCP_SESSION_ID, &session)
                    .body(Body::from(notification.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::ACCEPTED);

        let deleted = app
            .clone()
            .oneshot(
                request(Method::DELETE)
                    .header(MCP_PROTOCOL_VERSION, protocol::STABLE_VERSION)
                    .header(MCP_SESSION_ID, &session)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

        let after_delete = app
            .oneshot(
                request(Method::POST)
                    .header(MCP_PROTOCOL_VERSION, protocol::STABLE_VERSION)
                    .header(MCP_SESSION_ID, &session)
                    .body(Body::from(notification.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(after_delete.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn streamable_http_initialize_rejects_pre_2025_protocols() {
        let app = app(&[]).await;
        let initialize = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": protocol::LEGACY_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "http-test", "version": "1" }
            }
        });
        let headerless = app
            .clone()
            .oneshot(
                request(Method::POST)
                    .body(Body::from(initialize.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(headerless.status(), StatusCode::BAD_REQUEST);

        let with_header = app
            .oneshot(
                request(Method::POST)
                    .header(MCP_PROTOCOL_VERSION, protocol::LEGACY_VERSION)
                    .body(Body::from(initialize.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(with_header.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn delete_rejects_modern_and_wrong_legacy_versions() {
        let app = app(&[]).await;
        let modern = app
            .clone()
            .oneshot(
                request(Method::DELETE)
                    .header(MCP_PROTOCOL_VERSION, protocol::MODERN_RC_VERSION)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(modern.status(), StatusCode::METHOD_NOT_ALLOWED);

        let wrong_legacy = app
            .oneshot(
                request(Method::DELETE)
                    .header(MCP_PROTOCOL_VERSION, protocol::LEGACY_VERSION)
                    .header(MCP_SESSION_ID, "unknown")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wrong_legacy.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn protocol_sessions_are_capped_and_idle_sessions_are_pruned() {
        let now = Instant::now();
        let mut sessions = HashMap::new();
        for index in 0..MAX_PROTOCOL_SESSIONS {
            sessions.insert(
                index.to_string(),
                Arc::new(ProtocolSession::new(ProtocolState::default())),
            );
        }
        assert!(!has_protocol_session_capacity(&sessions));

        let expired = sessions.get("0").unwrap();
        *expired.last_used.lock().unwrap() =
            now - PROTOCOL_SESSION_IDLE_TTL - Duration::from_secs(1);
        prune_expired(&mut sessions, now);
        assert_eq!(sessions.len(), MAX_PROTOCOL_SESSIONS - 1);
        assert!(has_protocol_session_capacity(&sessions));
    }

    #[tokio::test]
    async fn get_is_truthfully_rejected_without_claiming_sse_support() {
        let response = app(&[])
            .await
            .oneshot(request(Method::GET).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers()[header::ALLOW], "POST, DELETE");
    }

    #[tokio::test]
    async fn non_loopback_bind_and_weak_tokens_are_refused() {
        let non_loopback = prepare_server(McpHttpConfig {
            host: "0.0.0.0".to_string(),
            port: 0,
            token: Some(TOKEN.to_string()),
            allowed_origins: Vec::new(),
        })
        .await;
        assert!(non_loopback.is_err());
        let weak = prepare_server(McpHttpConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            token: Some("weak".to_string()),
            allowed_origins: Vec::new(),
        })
        .await;
        assert!(weak.is_err());
        let whitespace = prepare_server(McpHttpConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            token: Some("0123456789abcdef 123456789abcdef".to_string()),
            allowed_origins: Vec::new(),
        })
        .await;
        assert!(whitespace.is_err());
        let malformed_origin = prepare_server(McpHttpConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            token: Some(TOKEN.to_string()),
            allowed_origins: vec!["https://allowed.example/path".to_string()],
        })
        .await;
        assert!(malformed_origin.is_err());
    }
}
