//! Runtime configuration. See `.env.example` at the repo root for an
//! operator-facing reference of every knob (env var ↔ purpose ↔ default).
//! CLI flags / env-var bindings live in `main.rs`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub max_body_bytes: usize,
    pub requests_per_minute: u32,
    pub rate_burst: u32,
    pub max_concurrent_renders: usize,
    pub render_timeout: Duration,
    pub request_timeout: Duration,
    pub api_token: Option<String>,
    pub trust_forwarded_for: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 3000),
            max_body_bytes: 64 * 1024,
            requests_per_minute: 60,
            rate_burst: 20,
            max_concurrent_renders: 16,
            render_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(15),
            api_token: None,
            trust_forwarded_for: false,
        }
    }
}

impl ServerConfig {
    /// Test-only constructor: ephemeral port, every gate disabled so
    /// integration tests can assert handler behavior without fighting limits.
    pub fn for_tests() -> Self {
        Self {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            requests_per_minute: 0,
            rate_burst: 0,
            max_concurrent_renders: 0,
            render_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            ..Self::default()
        }
    }
}
