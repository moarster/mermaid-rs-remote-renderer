//! HTTP integration tests.
//!
//! Mirrors the fixture-driven style of the upstream `mermaid-rs-renderer`
//! suite (`tests/layout_suite.rs`): a small set of `.mmd` files in
//! `tests/fixtures/`, a couple of helper assertions for SVG well-formedness,
//! and one `#[test]` per behavior under exercise.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use miniz_oxide::deflate::compress_to_vec_zlib;
use tower::ServiceExt;

use mermaid_rs_remote_renderer::{ServerConfig, app};

const FIXTURE_DIR: &str = "tests/fixtures";

// ---------- helpers ----------

fn fixture(name: &str) -> String {
    let path = Path::new(FIXTURE_DIR).join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

fn b64(source: &str) -> String {
    URL_SAFE_NO_PAD.encode(source.as_bytes())
}

fn pako(source: &str, theme: Option<&str>) -> String {
    let json = match theme {
        Some(t) => format!(
            "{{\"code\":{:?},\"mermaid\":{{\"theme\":{:?}}}}}",
            source, t
        ),
        None => format!("{{\"code\":{:?}}}", source),
    };
    let compressed = compress_to_vec_zlib(json.as_bytes(), 6);
    format!("pako:{}", URL_SAFE_NO_PAD.encode(&compressed))
}

fn req_with_ip(uri: &str, ip: [u8; 4]) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .extension(ConnectInfo(SocketAddr::from((ip, 12345))))
        .body(Body::empty())
        .unwrap()
}

fn req(uri: &str) -> Request<Body> {
    req_with_ip(uri, [127, 0, 0, 1])
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read body");
    String::from_utf8(bytes.to_vec()).expect("utf8 body")
}

fn assert_valid_svg(svg: &str, fixture: &str) {
    assert!(
        svg.contains("<svg"),
        "{fixture}: response is missing <svg> opening tag"
    );
    assert!(
        svg.contains("</svg>"),
        "{fixture}: response is missing </svg> closing tag"
    );
    assert!(
        !svg.contains("NaN") && !svg.contains("inf"),
        "{fixture}: SVG contains NaN/inf coordinate"
    );
}

fn unlimited_test_config() -> ServerConfig {
    // Disable rate-limit/concurrency caps so they don't interfere with
    // success-path assertions.
    ServerConfig::for_tests()
}

// ---------- success path ----------

#[tokio::test]
async fn health_returns_ok() {
    let response = app(unlimited_test_config())
        .oneshot(req("/health"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert_eq!(body, "ok");
}

#[tokio::test]
async fn index_returns_html_landing_page() {
    let response = app(unlimited_test_config())
        .oneshot(req("/"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("text/html"),
        "expected text/html, got {content_type}"
    );
    let body = body_string(response).await;
    assert!(body.contains("<!DOCTYPE html>"));
    assert!(body.contains("API reference"));
    assert!(body.contains("/svg/{encoded}"));
    assert!(body.contains("/health"));
    assert!(body.contains("mermaid-rs-renderer"));
    assert!(body.contains("/favicon.svg"));
}

#[tokio::test]
async fn favicon_is_served_as_svg() {
    let response = app(unlimited_test_config())
        .oneshot(req("/favicon.svg"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(content_type, "image/svg+xml");
    let body = body_string(response).await;
    assert!(body.contains("<svg"));
    // The differentiating accent dot — guards against accidentally serving
    // the upstream Mermaid icon if `icon.svg` ever gets reverted.
    assert!(body.contains("<circle"));
}

#[tokio::test]
async fn renders_each_fixture_via_plain_base64url() {
    let fixtures = ["flowchart_basic.mmd", "sequence_basic.mmd", "pie_basic.mmd"];

    for name in fixtures {
        let source = fixture(name);
        let encoded = b64(&source);
        let uri = format!("/svg/{encoded}");

        let response = app(unlimited_test_config())
            .oneshot(req(&uri))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "fixture {name}");
        let ct = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.starts_with("image/svg+xml"),
            "fixture {name}: unexpected content-type {ct}"
        );
        let svg = body_string(response).await;
        assert_valid_svg(&svg, name);
    }
}

#[tokio::test]
async fn renders_pako_payload_with_embedded_theme() {
    let src = fixture("flowchart_basic.mmd");
    let uri = format!("/svg/{}", pako(&src, Some("default")));

    let response = app(unlimited_test_config())
        .oneshot(req(&uri))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let svg = body_string(response).await;
    assert_valid_svg(&svg, "pako default");
}

#[tokio::test]
async fn query_param_overrides_embedded_theme() {
    let src = fixture("flowchart_basic.mmd");
    let uri = format!("/svg/{}?theme=modern", pako(&src, Some("default")));

    let response = app(unlimited_test_config())
        .oneshot(req(&uri))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_valid_svg(&body_string(response).await, "theme override");
}

#[tokio::test]
async fn svg_response_carries_long_lived_cache_header() {
    let src = fixture("flowchart_basic.mmd");
    let uri = format!("/svg/{}", b64(&src));

    let response = app(unlimited_test_config())
        .oneshot(req(&uri))
        .await
        .unwrap();
    let cache = response
        .headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(cache.contains("immutable"), "cache-control={cache}");
    assert!(cache.contains("max-age="), "cache-control={cache}");
}

#[tokio::test]
async fn response_propagates_request_id_header() {
    let response = app(unlimited_test_config())
        .oneshot(req("/health"))
        .await
        .unwrap();
    let id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(!id.is_empty(), "x-request-id header missing");
    // Whatever uuid version it is, it should at least be > 16 chars.
    assert!(id.len() >= 16, "x-request-id looks malformed: {id}");
}

// ---------- malformed input ----------

#[tokio::test]
async fn malformed_base64_returns_400() {
    let response = app(unlimited_test_config())
        .oneshot(req("/svg/!!!not-base64!!!"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_string(response).await;
    assert!(body.contains("decode"), "body: {body}");
}

#[tokio::test]
async fn empty_pako_payload_returns_400() {
    let response = app(unlimited_test_config())
        .oneshot(req("/svg/pako:"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn pako_with_garbage_zlib_returns_400() {
    let bogus = URL_SAFE_NO_PAD.encode(b"not zlib at all");
    let response = app(unlimited_test_config())
        .oneshot(req(&format!("/svg/pako:{bogus}")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ---------- abuse prevention ----------

#[tokio::test]
async fn rate_limit_kicks_in_after_burst() {
    let config = ServerConfig {
        // A hard ceiling: 2 requests then throttle.
        requests_per_minute: 60,
        rate_burst: 2,
        max_concurrent_renders: 0, // disable so it doesn't shadow the rate-limit
        ..ServerConfig::for_tests()
    };
    let app = app(config);
    let src = fixture("flowchart_basic.mmd");
    let uri = format!("/svg/{}", b64(&src));

    // Both burst requests should succeed.
    for i in 0..2 {
        let resp = app
            .clone()
            .oneshot(req_with_ip(&uri, [10, 0, 0, 1]))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "burst request {i}");
    }

    // The next one from the same IP should be rate-limited.
    let resp = app
        .clone()
        .oneshot(req_with_ip(&uri, [10, 0, 0, 1]))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

    // A request from a different IP should still succeed.
    let resp = app
        .clone()
        .oneshot(req_with_ip(&uri, [10, 0, 0, 2]))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn oversized_request_uri_is_rejected() {
    // Bodies for GET are empty, but the *encoded path* itself can blow out
    // the renderer's budget. We use a very small max_body and a very long
    // URI; axum returns 414 / 400 depending on hyper limits. The semantic
    // assertion: the server doesn't OOM and doesn't 200.
    let config = ServerConfig {
        max_body_bytes: 16,
        ..ServerConfig::for_tests()
    };
    // A 50 KB nonsense base64 string — far larger than any real diagram.
    let huge: String = "a".repeat(50_000);
    let uri = format!("/svg/{huge}");
    let resp = app(config).oneshot(req(&uri)).await.unwrap();
    assert!(
        !resp.status().is_success(),
        "oversized request unexpectedly succeeded: {}",
        resp.status()
    );
}

#[tokio::test]
async fn render_timeout_returns_504() {
    let config = ServerConfig {
        render_timeout: Duration::from_nanos(1),
        request_timeout: Duration::from_secs(5),
        ..ServerConfig::for_tests()
    };
    let src = fixture("flowchart_basic.mmd");
    let uri = format!("/svg/{}", b64(&src));
    let resp = app(config).oneshot(req(&uri)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::GATEWAY_TIMEOUT);
}

#[tokio::test]
async fn missing_api_token_returns_401_when_token_required() {
    let config = ServerConfig {
        api_token: Some("s3cret".to_string()),
        ..ServerConfig::for_tests()
    };
    let app = app(config);
    let src = fixture("flowchart_basic.mmd");
    let uri = format!("/svg/{}", b64(&src));

    // No token: 401.
    let resp = app.clone().oneshot(req(&uri)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Wrong token: 401.
    let mut wrong = req(&uri);
    wrong
        .headers_mut()
        .insert("x-api-token", "nope".parse().unwrap());
    let resp = app.clone().oneshot(wrong).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Correct token via header: 200.
    let mut good = req(&uri);
    good.headers_mut()
        .insert("x-api-token", "s3cret".parse().unwrap());
    let resp = app.clone().oneshot(good).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Correct token via query param: 200.
    let resp = app
        .clone()
        .oneshot(req(&format!("{uri}?token=s3cret")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Health is exempt even when token is required.
    let resp = app.clone().oneshot(req("/health")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
