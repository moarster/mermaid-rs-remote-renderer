# syntax=docker/dockerfile:1.7

# ---------- builder ----------
# Pinned Rust toolchain image; bumping this here is the single source of truth.
FROM rust:1.95-slim-bookworm AS builder

ENV CARGO_TERM_COLOR=always \
    RUSTFLAGS="-C target-cpu=x86-64-v2"

WORKDIR /build

# Cache deps separately from the source so source-only edits don't rebuild the world.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && \
    echo 'fn main() {}' > src/main.rs && \
    echo '' > src/lib.rs && \
    cargo build --release --locked && \
    rm -rf src

COPY src ./src

# Touch the entrypoints so cargo notices the real sources and recompiles them.
RUN touch src/main.rs src/lib.rs && \
    cargo build --release --locked --bin mermaid-rs-remote-renderer && \
    strip target/release/mermaid-rs-remote-renderer

# ---------- runtime ----------
# debian-slim is the smallest base that still ships glibc (musl would also work
# but requires a static build with extra effort). The whole pipeline is pure
# Rust — no Chromium, no fonts beyond what fontdb pulls in via the Rust crate
# — so the runtime image stays under ~80 MB.
FROM debian:bookworm-slim AS runtime

ARG APP_USER=app
ARG APP_UID=10001

# ca-certificates: needed if we ever fetch over HTTPS at runtime (defense in depth).
# tini: pid-1 init that reaps zombies and forwards signals (graceful shutdown).
# wget: used by HEALTHCHECK below.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tini wget \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# Non-root execution.
RUN groupadd --system --gid ${APP_UID} ${APP_USER} \
    && useradd  --system --uid ${APP_UID} --gid ${APP_UID} --no-create-home --shell /usr/sbin/nologin ${APP_USER}

WORKDIR /app
COPY --from=builder /build/target/release/mermaid-rs-remote-renderer /usr/local/bin/mermaid-rs-remote-renderer
COPY LICENSE NOTICE /app/

USER ${APP_USER}:${APP_USER}

# Defaults — override at `docker run` / compose level:
#   MMDR_BIND, MMDR_RPM, MMDR_BURST, MMDR_MAX_BODY_BYTES,
#   MMDR_MAX_CONCURRENT, MMDR_RENDER_TIMEOUT_SECS, MMDR_REQUEST_TIMEOUT_SECS,
#   MMDR_API_TOKEN, MMDR_TRUST_FORWARDED_FOR, RUST_LOG.
ENV MMDR_BIND=0.0.0.0:3000 \
    RUST_LOG=info

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -q -O - http://127.0.0.1:3000/health | grep -q '^ok$' || exit 1

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/mermaid-rs-remote-renderer"]
