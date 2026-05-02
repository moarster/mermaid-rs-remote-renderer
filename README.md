# Remote

[![CI](https://github.com/moarster/mermaid-rs-remote-renderer/actions/workflows/ci.yml/badge.svg)](https://github.com/moarster/mermaid-rs-remote-renderer/actions/workflows/ci.yml)
[![CD](https://github.com/moarster/mermaid-rs-remote-renderer/actions/workflows/cd.yml/badge.svg)](https://github.com/moarster/mermaid-rs-remote-renderer/actions/workflows/cd.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 2024](https://img.shields.io/badge/rust-2024-orange.svg?logo=rust)](https://www.rust-lang.org)
[![Container: GHCR](https://img.shields.io/badge/container-ghcr.io-2496ED?logo=docker&logoColor=white)](https://github.com/moarster/mermaid-rs-remote-renderer/pkgs/container/mermaid-rs-remote-renderer)
[![PRs welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](#contributing)

A small, production-ready HTTP service that exposes the
[`mermaid-rs-renderer`](https://github.com/1jehuang/mermaid-rs-renderer)
pipeline as a kroki / mermaid.ink-compatible GET endpoint.

```
GET /svg/<base64url(source)>
GET /svg/pako:<base64url(zlib_deflate(json))>
GET /svg/<encoded>?theme=default|modern
GET /health
```

The actual rendering — Mermaid parsing, layout, and SVG emission — is done by
the upstream library. This project adds an HTTP front door, request decoding,
structured logging, per-IP rate limiting, concurrency caps, request/render
timeouts, an optional bearer-token gate, a multi-stage Docker image, and CI/CD
to publish images to GHCR.

---

## Attribution

This is a **derivative work** based on the open-source project
[`mermaid-rs-renderer`](https://github.com/1jehuang/mermaid-rs-renderer),
distributed under the MIT License. All rendering logic comes from that
upstream crate; this project only wraps it.

See [`NOTICE`](./NOTICE) for the full credit statement and
[`LICENSE`](./LICENSE) for the MIT terms that apply to both the upstream code
and the additions in this repository.

---

## Features

- **Pure-Rust render path.** No browser, no Puppeteer, no Node — just the
  upstream library compiled in. Cold start is milliseconds; the whole image
  is < 80 MB.
- **kroki / mermaid.ink-compatible URLs.** Both plain `base64url(source)` and
  the `pako:` + zlib-deflate JSON form are accepted. Padded and unpadded
  base64 both decode.
- **Structured request logging** via `tracing`: each request gets a span
  carrying `method`, `path`, `status`, `latency`, `client_ip`, `request_id`.
  An incoming `x-request-id` is reused; otherwise a UUID v4 is minted and
  echoed in the response.
- **Abuse prevention out of the box:**
  - Per-IP token-bucket rate limit (`tower_governor`).
  - Global concurrency cap with load-shed (503 when full).
  - Request body size limit, per-request timeout, per-render timeout.
  - Optional API token gate (`X-Api-Token` header or `?token=` query param).
- **Production-ready container.** Multi-stage Dockerfile, distroless-style
  runtime (debian-slim + tini), non-root UID, read-only FS friendly, signal
  handling, `HEALTHCHECK`.

---

## Quickstart

### Run from source

```bash
cargo run --release
# binds 0.0.0.0:3000 with sensible defaults
```

```bash
# Encode a diagram and fetch the SVG.
ENC=$(printf 'flowchart LR; A-->B-->C' | base64 | tr '+/' '-_' | tr -d '=')
curl -s "http://127.0.0.1:3000/svg/${ENC}" > out.svg
```

### Run via Docker

```bash
docker build -t mmdr-remote .
docker run --rm -p 3000:3000 \
  -e MMDR_RPM=120 -e MMDR_BURST=40 -e RUST_LOG=info \
  mmdr-remote
```

### Run via docker compose (local dev)

```bash
docker compose up -d
docker compose logs -f mmdr
```

---

## Configuration reference

Every knob is exposed as **both** a CLI flag and an environment variable.
CLI flags win when both are set. Defaults are tuned for a small public
endpoint.

| Flag | Env var | Default | Purpose |
|------|---------|---------|---------|
| `--bind` | `MMDR_BIND` | `0.0.0.0:3000` | TCP socket to listen on. |
| `--max-body-bytes` | `MMDR_MAX_BODY_BYTES` | `65536` | Reject request bodies larger than this. Mermaid sources are tiny, so 64 KiB is generous. |
| `--requests-per-minute` | `MMDR_RPM` | `60` | Per-IP token-bucket replenish rate. `0` disables rate limiting. |
| `--rate-burst` | `MMDR_BURST` | `20` | Per-IP burst size. Number of back-to-back requests allowed before throttling kicks in. |
| `--max-concurrent-renders` | `MMDR_MAX_CONCURRENT` | `16` | Global cap on in-flight renders. Excess requests get HTTP 503. `0` disables the cap. |
| `--render-timeout-secs` | `MMDR_RENDER_TIMEOUT_SECS` | `10` | Hard deadline for the synchronous render. Exceeding it returns HTTP 504. |
| `--request-timeout-secs` | `MMDR_REQUEST_TIMEOUT_SECS` | `15` | Hard deadline for the entire HTTP request. Should be ≥ render timeout. |
| `--api-token` | `MMDR_API_TOKEN` | _(unset)_ | If set, every `/svg/*` request must carry this token in the `X-Api-Token` header or `?token=` query param. `/` and `/health` remain public. |
| `--trust-forwarded-for` | `MMDR_TRUST_FORWARDED_FOR` | `false` | Trust `X-Forwarded-For` / `X-Real-IP` / `Forwarded` for client IP. **Only enable behind a trusted reverse proxy.** |
| _(env only)_ | `RUST_LOG` | `info` | `tracing-subscriber` filter. Try `RUST_LOG=mermaid_rs_remote_renderer=debug,tower_http=debug`. |

Run `mermaid-rs-remote-renderer --help` for the canonical, version-stamped
output.

---

## API

### `GET /svg/{encoded}` → `image/svg+xml`

`{encoded}` is one of:

- **Plain base64url:** `base64url(source)`. URL-safe alphabet, padding
  optional. Example: `Zmxvd2NoYXJ0IExSOyBBLS0-Qg`
  (= `flowchart LR; A-->B`).
- **Compressed:** `pako:` + `base64url(zlib_deflate(json))` where the JSON
  payload is `{ "code": "<source>", "mermaid": { "theme": "<name>" } }`.

Optional query params:

- `theme=default|modern` — overrides the theme baked into the `pako:`
  payload. `default` / `neutral` / `base` map to the classic Mermaid look;
  anything else (or absent) yields the modern theme.

### `GET /health` → `text/plain`

Returns the literal string `ok`. Suitable for load-balancer probes;
exempt from the API token gate.

### `GET /` → `text/plain`

Short usage page.

### Examples

```bash
# Plain encoding.
ENC=$(printf 'flowchart LR; A-->B-->C' | base64 | tr '+/' '-_' | tr -d '=')
curl -s "http://127.0.0.1:3000/svg/${ENC}" > out.svg

# pako:-compressed encoding (Python 3).
python3 - <<'PY'
import base64, zlib, json, urllib.request
src = "sequenceDiagram\nAlice->>Bob: Hello"
payload = json.dumps({"code": src, "mermaid": {"theme": "default"}}).encode()
encoded = base64.urlsafe_b64encode(zlib.compress(payload)).rstrip(b"=").decode()
url = f"http://127.0.0.1:3000/svg/pako:{encoded}"
print(urllib.request.urlopen(url).read()[:80])
PY

# Theme override via query param.
curl -s "http://127.0.0.1:3000/svg/${ENC}?theme=default" > out_default.svg

# With token gate enabled.
curl -s -H "X-Api-Token: my-secret" "http://localhost:3000/svg/${ENC}"
curl -s "http://localhost:3000/svg/${ENC}?token=my-secret"
```

---

## Build, test, lint

```bash
cargo build              # debug build
cargo build --release    # production build
cargo test               # unit + integration tests
cargo fmt --all          # format
cargo clippy --all-targets -- -D warnings
```

The integration tests live in `tests/http_endpoints.rs` and use a
fixture-driven style mirroring the upstream library (see
`tests/fixtures/*.mmd`). They exercise:

- success paths for each fixture (plain + pako encoding, theme override),
- malformed inputs (bad base64, garbage zlib, empty payload),
- rate limiting per-IP,
- oversized requests,
- render-timeout → 504,
- token gate (missing / wrong / via header / via query / health-exempt),
- response headers (`x-request-id`, `Cache-Control`).

---

## Contributing

Contributions are welcome — bug reports, feature ideas, and pull requests.

**Before opening a PR**, please make sure the same checks CI runs locally pass:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --all-targets
```

A few conventions to keep diffs reviewable:

- **Scope.** Keep PRs focused — one logical change per PR. Separate refactors
  from behavior changes.
- **Tests.** New behavior needs a test. The integration suite in
  `tests/http_endpoints.rs` is fixture-driven; add a `.mmd` fixture under
  `tests/fixtures/` if your change touches a new diagram type.
- **Renderer bugs go upstream.** Layout / parsing / SVG output is owned by
  [`mermaid-rs-renderer`](https://github.com/1jehuang/mermaid-rs-renderer).
  If `out.svg` looks wrong, please file the issue there — this repo only
  wraps the renderer in HTTP.
- **No deployment-specific config in repo files.** `docker-compose.yml`,
  workflows, and docs stay generic; reverse proxy / TLS / hostname details
  belong in the operator's private setup.
- **Commit messages** follow the imperative present (`add foo`, not
  `added foo`); a short subject line plus body when it helps reviewers.
- **Licensing.** By submitting a PR you agree your contribution is licensed
  under the same [MIT terms](./LICENSE) as the rest of the repo (and the
  upstream renderer it wraps).

Not sure if a change is a good fit? Open an issue first to discuss.

---

## License

[MIT](./LICENSE) — see [`NOTICE`](./NOTICE) for upstream attribution.
