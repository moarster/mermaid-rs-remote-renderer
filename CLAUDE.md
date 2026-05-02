# CLAUDE.md

Guidance for future Claude Code sessions in this repo.

## What this is

HTTP wrapper around the upstream
[`mermaid-rs-renderer`](https://github.com/1jehuang/mermaid-rs-renderer)
crate, exposing a kroki / mermaid.ink-compatible `GET /svg/{encoded}`
endpoint plus production hardening (rate limit, concurrency cap, timeouts,
body limit, optional token gate, structured logging).

**Derivative work**, not a fork of the renderer. Don't change rendering
behavior here — file rendering issues upstream
(`/home/ivan/Work/mermaid-rs-renderer`).

## Build / test / lint

CI in `.github/workflows/ci.yml` is authoritative — match it locally:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --all-targets
docker build -t mmdr-remote .
```

## Gotchas (the real reason this file exists)

- **Upstream is forgiving.** The renderer produces an SVG even for garbage
  input like `@@@ not mermaid @@@` (falls back to a single-node flowchart).
  Don't write tests expecting parse errors for loosely-malformed sources;
  exercise our own decode/timeout/limit paths instead.
- **`PeerIpKeyExtractor` requires `ConnectInfo`.** The router is served via
  `into_make_service_with_connect_info::<SocketAddr>()` in `serve()`.
  Integration tests inject `ConnectInfo` manually because `Router::oneshot`
  bypasses the `MakeService`.
- **Body size limit uses `axum::extract::DefaultBodyLimit`**, not
  `tower_http::limit::RequestBodyLimitLayer`. The tower-http version
  produces a `ResponseBody<Body>` wrapper that doesn't satisfy axum's body
  trait bounds when applied via `Router::layer`.
- **`api_token` empty string is treated as unset** by `main.rs`
  (`.filter(|s| !s.is_empty())`). `MMDR_API_TOKEN=` (empty) reliably
  disables the gate without removing the env var.
- **`trust_forwarded_for` toggles `SmartIpKeyExtractor`** for rate limiting.
  Enable only behind a trusted reverse proxy.
- **Renderer is sync + CPU-bound** — always wrap in `tokio::task::spawn_blocking`
  + `tokio::time::timeout` (see `render::render_svg`). Never call from an
  async handler directly.
- **Middleware order in `app()`** (outermost first): API token gate →
  SetRequestId → Trace → PropagateRequestId → request timeout →
  body-size limit → routes → optional GovernorLayer. Keep the token gate
  first so unauthenticated requests are rejected cheaply.

## Deployment

Push to `main` → CD builds and pushes `ghcr.io/<owner>/<repo>:sha-<full-sha>`
+ `:latest` to GHCR, then optionally POSTs to `secrets.DEPLOY_WEBHOOK_URL`.
The bundled `docker-compose.yml` is **local-dev only** — don't extend it
with reverse proxy / ACME / hostname config; those belong in the operator's
private infra repo.

To redeploy without code changes:
`git commit --allow-empty -m "redeploy" && git push`.
