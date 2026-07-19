//! Local HTTP bridge server for Chrome extension integration.
//!
//! Provides endpoints for the Plasmate extension to push cookies directly
//! instead of using clipboard copy.

use crate::auth::{
    capability::{bearer_matches, generate_token, validate_token},
    store::{self, CookieEntry, CookieProfile},
};
use axum::{
    extract::{Json, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tracing::{error, info};

/// Default port for the bridge server
pub const DEFAULT_PORT: u16 = 9271;
pub const BRIDGE_TOKEN_ENV: &str = "PLASMATE_AUTH_BRIDGE_TOKEN";
pub const BRIDGE_ORIGIN_ENV: &str = "PLASMATE_AUTH_BRIDGE_ORIGIN";

#[derive(Clone)]
struct BridgeState {
    token: Arc<str>,
    allowed_origin: Option<HeaderValue>,
}

#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// Exact extension origin, for example `chrome-extension://<extension-id>`.
    /// When absent, browser CORS access is disabled; header-authenticated local
    /// clients without an Origin header can still connect.
    pub allowed_origin: Option<String>,
    /// Capability token required in `Authorization: Bearer <token>`.
    pub token: String,
}

impl BridgeConfig {
    pub fn from_environment() -> Result<Self, String> {
        let token = match std::env::var(BRIDGE_TOKEN_ENV) {
            Ok(value) => validate_token(&value)
                .map(|()| value)
                .map_err(|error| format!("invalid {BRIDGE_TOKEN_ENV}: {error}"))?,
            Err(std::env::VarError::NotPresent) => generate_token(),
            Err(error) => return Err(format!("invalid {BRIDGE_TOKEN_ENV}: {error}")),
        };
        let allowed_origin = std::env::var(BRIDGE_ORIGIN_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty());
        if let Some(origin) = &allowed_origin {
            validate_extension_origin(origin)?;
        }
        Ok(Self {
            allowed_origin,
            token,
        })
    }
}

fn validate_extension_origin(origin: &str) -> Result<(), String> {
    let parsed = url::Url::parse(origin).map_err(|e| format!("invalid bridge origin: {e}"))?;
    if parsed.scheme() != "chrome-extension"
        || parsed.host_str().is_none()
        || parsed.path() != ""
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(
            "bridge origin must be an exact chrome-extension://<extension-id> origin".to_string(),
        );
    }
    Ok(())
}

/// Request body for POST /api/cookies
#[derive(Debug, Deserialize)]
pub struct CookiesRequest {
    pub domain: String,
    pub cookies: HashMap<String, CookieValue>,
    #[serde(default)]
    pub expiry: HashMap<String, i64>,
}

/// Cookie value - can be a simple string or an object with expiry
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum CookieValue {
    Simple(String),
    WithExpiry {
        value: String,
        #[serde(rename = "expirationDate")]
        expiration_date: Option<f64>,
    },
}

impl CookieValue {
    pub fn into_entry(self, expiry_override: Option<i64>) -> CookieEntry {
        match self {
            CookieValue::Simple(value) => CookieEntry::with_expiry(value, expiry_override),
            CookieValue::WithExpiry {
                value,
                expiration_date,
            } => {
                // Prefer explicit expiry override, then embedded expirationDate
                let exp = expiry_override.or(expiration_date.map(|ts| ts as i64));
                CookieEntry::with_expiry(value, exp)
            }
        }
    }
}

/// Response body for GET /api/status
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub ok: bool,
    pub version: String,
    pub profiles: Vec<String>,
}

/// Query params for GET /api/wait
#[derive(Debug, Deserialize)]
pub struct WaitQuery {
    /// Domain to wait for (e.g., "x.com")
    pub domain: String,
    /// Timeout in seconds (default: 120, max: 300)
    #[serde(default = "default_wait_timeout")]
    pub timeout: u64,
}

fn default_wait_timeout() -> u64 {
    120
}

/// Response body for GET /api/wait
#[derive(Debug, Serialize)]
pub struct WaitResponse {
    pub ok: bool,
    pub domain: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cookies: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Response body for POST /api/cookies
#[derive(Debug, Serialize)]
pub struct CookiesResponse {
    pub ok: bool,
    pub domain: String,
    pub cookies_stored: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Start the bridge HTTP server.
pub async fn start(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let config = BridgeConfig::from_environment()?;
    eprintln!("Auth bridge capability token: {}", config.token);
    if config.allowed_origin.is_none() {
        eprintln!(
            "Browser extension access is disabled. Set {} to the exact extension origin.",
            BRIDGE_ORIGIN_ENV
        );
    }
    start_with_config(port, config).await
}

pub async fn start_with_config(
    port: u16,
    config: BridgeConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let allowed_origin = config
        .allowed_origin
        .as_deref()
        .map(HeaderValue::from_str)
        .transpose()?;
    let app = build_router(config.token, allowed_origin)?;

    let listener = TcpListener::bind(addr).await?;
    info!(port = %port, "Auth bridge server listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

fn build_router(
    token: String,
    allowed_origin: Option<HeaderValue>,
) -> Result<Router, Box<dyn std::error::Error>> {
    let state = BridgeState {
        token: Arc::from(token),
        allowed_origin: allowed_origin.clone(),
    };
    let mut cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        .max_age(std::time::Duration::from_secs(86400));
    if let Some(origin) = allowed_origin {
        cors = cors.allow_origin(origin);
    }

    Ok(Router::new()
        .route("/api/status", get(handle_status))
        .route("/api/cookies", post(handle_cookies))
        .route("/api/wait", get(handle_wait))
        // Authenticate before body extractors deserialize attacker input.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            authorize_request,
        ))
        .layer(cors)
        .with_state(state))
}

async fn authorize_request(
    State(state): State<BridgeState>,
    request: Request,
    next: Next,
) -> Response {
    if request.method() == Method::OPTIONS {
        return next.run(request).await;
    }
    match authorize(&state, request.headers()) {
        Ok(()) => next.run(request).await,
        Err(status) => status.into_response(),
    }
}

/// Handle GET /api/status
async fn handle_status(State(state): State<BridgeState>, headers: HeaderMap) -> impl IntoResponse {
    if let Err(status) = authorize(&state, &headers) {
        return (
            status,
            Json(StatusResponse {
                ok: false,
                version: String::new(),
                profiles: Vec::new(),
            }),
        );
    }
    let profiles = store::list_profiles().unwrap_or_default();
    let response = StatusResponse {
        ok: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        profiles,
    };

    (StatusCode::OK, Json(response))
}

/// Handle POST /api/cookies
async fn handle_cookies(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(request): Json<CookiesRequest>,
) -> impl IntoResponse {
    if let Err(status) = authorize(&state, &headers) {
        return (
            status,
            Json(CookiesResponse {
                ok: false,
                domain: request.domain,
                cookies_stored: 0,
                error: Some("unauthorized".to_string()),
            }),
        );
    }
    // Convert cookies, applying expiry from the separate expiry map if provided
    let cookies: HashMap<String, CookieEntry> = request
        .cookies
        .into_iter()
        .map(|(k, v)| {
            let expiry = request.expiry.get(&k).copied();
            (k, v.into_entry(expiry))
        })
        .collect();

    let cookie_count = cookies.len();
    let domain = request.domain.clone();

    // Create and store profile
    let profile = CookieProfile {
        domain: request.domain,
        cookies,
        created_at: Some({
            let dur = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            format!("{}", dur.as_secs())
        }),
        notes: Some("Imported via extension bridge".to_string()),
    };

    match store::store_profile(&profile) {
        Ok(()) => {
            info!(
                domain = %domain,
                cookies = cookie_count,
                "Stored profile via bridge"
            );
            (
                StatusCode::OK,
                Json(CookiesResponse {
                    ok: true,
                    domain,
                    cookies_stored: cookie_count,
                    error: None,
                }),
            )
        }
        Err(e) => {
            error!(domain = %domain, error = %e, "Failed to store profile");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(CookiesResponse {
                    ok: false,
                    domain,
                    cookies_stored: 0,
                    error: Some(format!("Failed to store: {}", e)),
                }),
            )
        }
    }
}

/// Handle GET /api/wait?domain=x.com&timeout=120
///
/// Long-polls until a cookie profile exists for the given domain.
/// Returns immediately if the profile already exists.
/// Polls every 2 seconds up to the timeout (max 300s).
async fn handle_wait(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Query(params): Query<WaitQuery>,
) -> impl IntoResponse {
    if let Err(status) = authorize(&state, &headers) {
        return (
            status,
            Json(WaitResponse {
                ok: false,
                domain: params.domain,
                cookies: None,
                error: Some("unauthorized".to_string()),
            }),
        );
    }
    let timeout = params.timeout.min(300);
    let domain = params.domain;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);

    info!(domain = %domain, timeout = timeout, "Waiting for cookie profile");

    loop {
        // Check if profile exists
        match store::load_profile(&domain) {
            Ok(Some(profile)) => {
                let count = profile.cookies.len();
                info!(domain = %domain, cookies = count, "Profile arrived");
                return (
                    StatusCode::OK,
                    Json(WaitResponse {
                        ok: true,
                        domain,
                        cookies: Some(count),
                        error: None,
                    }),
                );
            }
            Ok(None) => {}
            Err(e) => {
                error!(domain = %domain, error = %e, "Error checking profile");
            }
        }

        // Check timeout
        if tokio::time::Instant::now() >= deadline {
            info!(domain = %domain, "Wait timed out");
            return (
                StatusCode::REQUEST_TIMEOUT,
                Json(WaitResponse {
                    ok: false,
                    domain,
                    cookies: None,
                    error: Some("Timed out waiting for cookies".to_string()),
                }),
            );
        }

        // Poll interval
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

fn authorize(state: &BridgeState, headers: &HeaderMap) -> Result<(), StatusCode> {
    if let Some(origin) = headers.get(header::ORIGIN) {
        if state.allowed_origin.as_ref() != Some(origin) {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    let supplied = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if !bearer_matches(supplied, &state.token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

#[cfg(test)]
mod security_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn state() -> BridgeState {
        BridgeState {
            token: Arc::from("0123456789abcdef0123456789abcdef"),
            allowed_origin: Some(HeaderValue::from_static(
                "chrome-extension://abcdefghijklmnop",
            )),
        }
    }

    #[test]
    fn rejects_missing_and_wrong_capability_tokens() {
        let state = state();
        assert_eq!(
            authorize(&state, &HeaderMap::new()),
            Err(StatusCode::UNAUTHORIZED)
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong"),
        );
        assert_eq!(authorize(&state, &headers), Err(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn enforces_exact_origin_and_correct_token() {
        let state = state();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer 0123456789abcdef0123456789abcdef"),
        );
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("chrome-extension://other"),
        );
        assert_eq!(authorize(&state, &headers), Err(StatusCode::FORBIDDEN));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("chrome-extension://abcdefghijklmnop"),
        );
        assert_eq!(authorize(&state, &headers), Ok(()));
    }

    #[test]
    fn validates_extension_origin_shape() {
        assert!(validate_extension_origin("chrome-extension://abcdefghijklmnop").is_ok());
        assert!(validate_extension_origin("https://example.com").is_err());
        assert!(validate_extension_origin("chrome-extension://id/path").is_err());
    }

    #[tokio::test]
    async fn router_rejects_missing_token_before_handler() {
        let app = build_router(
            "0123456789abcdef0123456789abcdef".to_string(),
            Some(HeaderValue::from_static(
                "chrome-extension://abcdefghijklmnop",
            )),
        )
        .unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .header(header::ORIGIN, "chrome-extension://abcdefghijklmnop")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn router_rejects_wrong_origin_even_with_token() {
        let app = build_router(
            "0123456789abcdef0123456789abcdef".to_string(),
            Some(HeaderValue::from_static(
                "chrome-extension://abcdefghijklmnop",
            )),
        )
        .unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .header(header::ORIGIN, "chrome-extension://attacker")
                    .header(
                        header::AUTHORIZATION,
                        "Bearer 0123456789abcdef0123456789abcdef",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
