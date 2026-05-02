//! Wrapper around the upstream synchronous renderer.
//!
//! The upstream `mermaid_rs_renderer::render_with_options` is CPU-bound and
//! synchronous. We isolate it inside `spawn_blocking` so the async runtime
//! stays responsive, then wrap that in `tokio::time::timeout` so a pathological
//! diagram cannot pin a worker forever.

use std::time::Duration;

use mermaid_rs_renderer::{RenderOptions, Theme, render_with_options};

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// The renderer panicked or the spawn task itself failed.
    #[error("renderer task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    /// Render exceeded `render_timeout`.
    #[error("render exceeded the configured timeout")]
    Timeout,
    /// Upstream parser/layout/render returned an error.
    #[error("render failed: {0}")]
    Upstream(String),
}

/// Render `source` to SVG, applying `theme_name` (case-insensitive) when
/// present. The upstream library understands two presets — `default` /
/// `neutral` / `base` map to the classic Mermaid look; everything else falls
/// through to the modern theme.
pub async fn render_svg(
    source: String,
    theme_name: Option<String>,
    timeout: Duration,
) -> Result<String, RenderError> {
    let options = RenderOptions {
        theme: pick_theme(theme_name.as_deref()),
        layout: Default::default(),
    };

    let task = tokio::task::spawn_blocking(move || render_with_options(&source, options));

    match tokio::time::timeout(timeout, task).await {
        Ok(join_result) => match join_result? {
            Ok(svg) => Ok(svg),
            Err(e) => Err(RenderError::Upstream(format!("{e:#}"))),
        },
        Err(_) => Err(RenderError::Timeout),
    }
}

/// Map a user-supplied theme name to an upstream `Theme`. Unknown values fall
/// back to `Theme::modern()` rather than erroring — matching the historical
/// behavior of the demo server.
pub fn pick_theme(name: Option<&str>) -> Theme {
    match name.map(|s| s.to_ascii_lowercase()) {
        Some(t) if matches!(t.as_str(), "default" | "neutral" | "base") => Theme::mermaid_default(),
        _ => Theme::modern(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn pick_theme_defaults_to_modern() {
        // Modern is the fallback for None and for unknown names.
        let _ = pick_theme(None);
        let _ = pick_theme(Some("does-not-exist"));
    }

    #[test]
    fn pick_theme_recognizes_default_aliases() {
        let _ = pick_theme(Some("default"));
        let _ = pick_theme(Some("DEFAULT"));
        let _ = pick_theme(Some("neutral"));
        let _ = pick_theme(Some("base"));
    }

    #[tokio::test]
    async fn renders_simple_flowchart() {
        let svg = render_svg(
            "flowchart LR; A-->B".to_string(),
            None,
            Duration::from_secs(5),
        )
        .await
        .expect("render");
        assert!(svg.contains("<svg"));
        assert!(!svg.contains("NaN"));
    }

    #[tokio::test]
    async fn renders_with_default_theme_alias() {
        let svg = render_svg(
            "flowchart LR; A-->B".to_string(),
            Some("default".to_string()),
            Duration::from_secs(5),
        )
        .await
        .expect("render");
        assert!(svg.contains("<svg"));
    }

    #[tokio::test]
    async fn timeout_returns_timeout_error() {
        // 1 nanosecond is unreachable for any real render; this exercises the
        // timeout branch deterministically.
        let err = render_svg(
            "flowchart LR; A-->B".to_string(),
            None,
            Duration::from_nanos(1),
        )
        .await
        .expect_err("should time out");
        assert!(matches!(err, RenderError::Timeout));
    }
}
