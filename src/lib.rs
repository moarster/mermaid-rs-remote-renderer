//! HTTP server exposing a kroki / mermaid.ink-compatible GET endpoint backed
//! by [`mermaid_rs_renderer`].
//!
//! Routes:
//! - `GET /svg/:encoded` – returns `image/svg+xml`
//! - `GET /health`       – returns `ok`
//! - `GET /`             – HTML landing page (API reference + credits)
//! - `GET /favicon.svg`  – the page favicon (also reused as the project mark)
//!
//! `:encoded` accepts both formats produced by the reference Java encoder:
//! - plain base64url of the raw Mermaid source, or
//! - `pako:` + base64url of zlib-deflated JSON `{"code": "...", "mermaid": {"theme": "..."}}`
//!
//! Production hardening (configurable via [`ServerConfig`]):
//! - Per-IP token-bucket rate limiting (`tower_governor`).
//! - Global concurrency cap with load shedding.
//! - Request body size limit + per-request timeout.
//! - Per-render timeout via `spawn_blocking`.
//! - Optional bearer-token gate.
//! - Structured `tracing` spans with method, path, status, latency, client IP,
//!   and a propagated `x-request-id`.

mod config;
mod decode;
mod render;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use tokio::sync::Semaphore;
use tower::ServiceBuilder;
use tower_governor::{
    GovernorLayer,
    governor::GovernorConfigBuilder,
    key_extractor::{PeerIpKeyExtractor, SmartIpKeyExtractor},
};
use tower_http::{
    LatencyUnit,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing::{Level, Span};

pub use config::ServerConfig;
pub use decode::{DecodeError, DecodedRequest, decode_request};
pub use render::{RenderError, render_svg};

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// Shared state injected into every handler.
#[derive(Clone)]
struct AppState {
    config: Arc<ServerConfig>,
    /// `None` when concurrency limiting is disabled (`max_concurrent_renders == 0`).
    render_semaphore: Option<Arc<Semaphore>>,
}

/// Build the axum router using the given configuration.
///
/// Exposed for embedding and integration tests; production callers should use
/// [`serve`] which also binds a TCP listener and wires graceful shutdown.
pub fn app(config: ServerConfig) -> Router {
    let config = Arc::new(config);

    let state = AppState {
        render_semaphore: (config.max_concurrent_renders > 0)
            .then(|| Arc::new(Semaphore::new(config.max_concurrent_renders))),
        config: config.clone(),
    };

    let mut router = Router::new()
        .route("/", get(index))
        .route("/favicon.svg", get(favicon))
        .route("/health", get(health))
        .route("/svg/{encoded}", get(render_svg_route))
        .with_state(state);

    // Optional API token gate. Applied before the heavier middleware so we
    // reject unauthenticated requests cheaply.
    if config.api_token.is_some() {
        let token_state = config.clone();
        router = router.layer(middleware::from_fn_with_state(
            token_state,
            require_api_token,
        ));
    }

    // Per-request hard timeout. Wraps everything below (including the render).
    let request_timeout = config.request_timeout;

    // Tracing layer: emit a span per request with method/path/status/latency.
    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(make_request_span)
        .on_response(
            DefaultOnResponse::new()
                .level(Level::INFO)
                .latency_unit(LatencyUnit::Millis),
        );

    let middleware = ServiceBuilder::new()
        .layer(SetRequestIdLayer::new(REQUEST_ID_HEADER, MakeRequestUuid))
        .layer(trace_layer)
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            request_timeout,
        ));

    router = router
        .layer(DefaultBodyLimit::max(config.max_body_bytes))
        .layer(middleware);

    // Per-IP rate limit. Disabled when `requests_per_minute == 0`. Applied
    // after the trace layer so rejected requests still appear in logs.
    if config.requests_per_minute > 0 && config.rate_burst > 0 {
        router = attach_rate_limit(router, &config);
    }

    router
}

fn attach_rate_limit(router: Router, config: &ServerConfig) -> Router {
    // governor's quota is `replenish 1 every period`; convert RPM to a period.
    // For e.g. 60 RPM → one token per second.
    let period_ms = (60_000 / config.requests_per_minute as u64).max(1);
    let burst = config.rate_burst.max(1);

    if config.trust_forwarded_for {
        let gov = GovernorConfigBuilder::default()
            .period(Duration::from_millis(period_ms))
            .burst_size(burst)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .expect("valid governor config");
        router.layer(GovernorLayer::new(gov))
    } else {
        let gov = GovernorConfigBuilder::default()
            .period(Duration::from_millis(period_ms))
            .burst_size(burst)
            .key_extractor(PeerIpKeyExtractor)
            .finish()
            .expect("valid governor config");
        router.layer(GovernorLayer::new(gov))
    }
}

/// Bind to `config.bind` and serve until the process receives SIGINT/SIGTERM.
pub async fn serve(config: ServerConfig) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let bound = listener.local_addr()?;
    tracing::info!(
        %bound,
        max_body_bytes = config.max_body_bytes,
        rpm = config.requests_per_minute,
        burst = config.rate_burst,
        max_concurrent = config.max_concurrent_renders,
        render_timeout_ms = config.render_timeout.as_millis() as u64,
        request_timeout_ms = config.request_timeout.as_millis() as u64,
        token_gated = config.api_token.is_some(),
        trust_xff = config.trust_forwarded_for,
        "mermaid-rs-remote-renderer listening"
    );

    let app = app(config);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}

// ---------- handlers ----------

const INDEX_HTML: &str = include_str!("../assets/index.html");
const FAVICON_SVG: &str = include_str!("../assets/icon.svg");

async fn index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        INDEX_HTML,
    )
}

async fn favicon() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        FAVICON_SVG,
    )
}

async fn health() -> &'static str {
    "ok"
}

#[derive(Debug, Deserialize)]
struct RenderQuery {
    /// Override the theme requested in the encoded payload.
    theme: Option<String>,
    /// Optional API token (alternative to the `X-Api-Token` header).
    #[allow(dead_code)] // consumed by `require_api_token`, present here only for query parsing
    token: Option<String>,
}

async fn render_svg_route(
    State(state): State<AppState>,
    Path(encoded): Path<String>,
    Query(query): Query<RenderQuery>,
) -> Response {
    let decoded = match decode_request(&encoded) {
        Ok(d) => d,
        Err(e) => return text_error(StatusCode::BAD_REQUEST, format!("decode error: {e}")),
    };

    // Optional global concurrency cap. We acquire *after* decoding so cheap
    // rejections (bad base64) don't burn a slot.
    let _permit = match state.render_semaphore.as_ref() {
        Some(sem) => match sem.clone().try_acquire_owned() {
            Ok(p) => Some(p),
            Err(_) => {
                return text_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "server is at capacity, please retry".to_string(),
                );
            }
        },
        None => None,
    };

    let theme = query.theme.or(decoded.theme);

    match render_svg(decoded.source, theme, state.config.render_timeout).await {
        Ok(svg) => svg_response(svg),
        Err(RenderError::Timeout) => text_error(
            StatusCode::GATEWAY_TIMEOUT,
            "render exceeded the configured timeout".to_string(),
        ),
        Err(RenderError::Upstream(msg)) => {
            text_error(StatusCode::BAD_REQUEST, format!("render error: {msg}"))
        }
        Err(RenderError::Join(e)) => {
            tracing::error!(error = %e, "render task panicked");
            text_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal renderer error".to_string(),
            )
        }
    }
}

fn svg_response(svg: String) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml; charset=utf-8"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    (StatusCode::OK, headers, svg).into_response()
}

fn text_error(status: StatusCode, msg: String) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        msg,
    )
        .into_response()
}

// ---------- middleware ----------

/// Reject requests that don't carry the configured API token. Public routes
/// (`/`, `/health`) are exempt so health checks and uptime monitors keep
/// working without credentials.
async fn require_api_token(
    State(config): State<Arc<ServerConfig>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if matches!(path, "/" | "/health" | "/favicon.svg") {
        return next.run(request).await;
    }

    let Some(expected) = config.api_token.as_deref() else {
        return next.run(request).await;
    };

    let supplied = request
        .headers()
        .get("x-api-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| request.uri().query().and_then(extract_token_from_query));

    match supplied {
        Some(t) if constant_time_eq(t.as_bytes(), expected.as_bytes()) => next.run(request).await,
        _ => text_error(StatusCode::UNAUTHORIZED, "missing or invalid token".into()),
    }
}

fn extract_token_from_query(q: &str) -> Option<String> {
    for kv in q.split('&') {
        if let Some(v) = kv.strip_prefix("token=") {
            return Some(urlencoded_decode(v));
        }
    }
    None
}

fn urlencoded_decode(s: &str) -> String {
    // Tiny inline decoder — we only ever feed it short tokens.
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let Ok(b) =
                    u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
                {
                    out.push(b);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ---------- tracing ----------

fn make_request_span(request: &Request) -> Span {
    let method = request.method().as_str().to_string();
    let path = request.uri().path().to_string();
    let request_id = request
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string();
    let client_ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .or_else(|| {
            request
                .headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(',').next().map(|s| s.trim().to_string()))
        })
        .unwrap_or_else(|| "-".to_string());

    tracing::info_span!(
        "http_request",
        method = %method,
        path = %path,
        request_id = %request_id,
        client_ip = %client_ip,
    )
}
