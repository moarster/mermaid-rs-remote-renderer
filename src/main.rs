//! HTTP server binary: kroki / mermaid.ink-compatible GET endpoint backed by
//! the upstream `mermaid-rs-renderer` library.

use std::net::SocketAddr;
use std::time::Duration;

use clap::Parser;
use mermaid_rs_remote_renderer::{ServerConfig, serve};

#[derive(Debug, Parser)]
#[command(
    name = "mermaid-rs-remote-renderer",
    version,
    about = "HTTP server for the mermaid-rs-renderer pipeline (kroki / mermaid.ink-compatible)"
)]
struct Args {
    /// Address to bind, e.g. 0.0.0.0:3000.
    #[arg(long, env = "MMDR_BIND", default_value = "0.0.0.0:3000")]
    bind: SocketAddr,

    /// Maximum request body size in bytes. Mermaid sources are tiny; the
    /// default of 64 KiB is generous and rejects abuse cheaply.
    #[arg(long, env = "MMDR_MAX_BODY_BYTES", default_value_t = 64 * 1024)]
    max_body_bytes: usize,

    /// Per-IP requests per minute (token-bucket replenish rate). Set to 0 to
    /// disable rate limiting entirely.
    #[arg(long, env = "MMDR_RPM", default_value_t = 60)]
    requests_per_minute: u32,

    /// Burst size: how many requests an IP may issue back-to-back before
    /// being throttled to `--requests-per-minute`.
    #[arg(long, env = "MMDR_BURST", default_value_t = 20)]
    rate_burst: u32,

    /// Maximum number of concurrent renders across the whole process.
    /// Excess requests get a 503 (load shed). Set to 0 to disable the cap.
    #[arg(long, env = "MMDR_MAX_CONCURRENT", default_value_t = 16)]
    max_concurrent_renders: usize,

    /// Hard timeout (seconds) for a single render. Exceeding this returns 504.
    #[arg(long, env = "MMDR_RENDER_TIMEOUT_SECS", default_value_t = 10)]
    render_timeout_secs: u64,

    /// Hard timeout (seconds) for the entire HTTP request. Should be
    /// >= render timeout to leave headroom for decode and response framing.
    #[arg(long, env = "MMDR_REQUEST_TIMEOUT_SECS", default_value_t = 15)]
    request_timeout_secs: u64,

    /// If set, every render request must carry this token in either the
    /// `X-Api-Token` header or the `?token=` query string. Health checks
    /// remain public.
    #[arg(long, env = "MMDR_API_TOKEN")]
    api_token: Option<String>,

    /// Trust `X-Forwarded-For` / `X-Real-IP` / `Forwarded` headers when
    /// determining the client IP for rate-limit bucketing. Only enable this
    /// when the server sits behind a trusted reverse proxy.
    #[arg(long, env = "MMDR_TRUST_FORWARDED_FOR", default_value_t = false)]
    trust_forwarded_for: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    let config = ServerConfig {
        bind: args.bind,
        max_body_bytes: args.max_body_bytes,
        requests_per_minute: args.requests_per_minute,
        rate_burst: args.rate_burst,
        max_concurrent_renders: args.max_concurrent_renders,
        render_timeout: Duration::from_secs(args.render_timeout_secs),
        request_timeout: Duration::from_secs(args.request_timeout_secs),
        api_token: args.api_token.filter(|s| !s.is_empty()),
        trust_forwarded_for: args.trust_forwarded_for,
    };

    serve(config).await
}
