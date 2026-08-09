# syntax=docker/dockerfile:1

# Brain Server — container image (linux/amd64 + linux/arm64 via buildx).
#
# Two things make this image non-trivial:
#   1. The embedding model (minishlab/potion-retrieval-32M, ~124 MB) is
#      fetched by hf-hub at first boot. For enterprise / air-gapped pilots we
#      bake the HF cache into the image at build time, so the container boots
#      offline with zero network calls. HF_HOME points at the baked cache.
#   2. The server is loopback-safe by default: BIND_HOST=0.0.0.0 is refused
#      unless BIND_PUBLIC=1. The image keeps that fail-safe default; compose
#      opts in explicitly (see docker-compose.yml) and the SSO guide explains
#      why the edge proxy is the right place for 0.0.0.0.
#
# Build:  docker build -t brain-server:local .
# Verify: docker run --rm -p 127.0.0.1:8765:8765 \
#           -e BIND_HOST=0.0.0.0 -e BIND_PUBLIC=1 -e AUTH_TOKEN=dev brain-server:local
#         curl http://127.0.0.1:8765/health

# ── Stage 1: bake the HF model cache (offline boot) ────────────────────────
FROM debian:bookworm-slim AS model

# Pinned revision of minishlab/potion-retrieval-32M resolved from the main
# branch. `resolve/<commit>/<file>` URLs are immutable, so a future upstream
# move cannot change what this image ships.
ARG HF_COMMIT=6fc8051fab2a1e0ee76689cf08c853792ac285e7

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

# Replicate the exact hf-hub cache layout (blobs named by content sha256,
# snapshots/<rev> symlinking into blobs, refs/main holding the revision):
#   $HF_HOME/hub/models--minishlab--potion-retrieval-32M/{blobs,refs,snapshots}
RUN set -eux; \
    BASE=/opt/brain-model/hub/models--minishlab--potion-retrieval-32M; \
    mkdir -p "$BASE/blobs" "$BASE/refs" "$BASE/snapshots/$HF_COMMIT"; \
    echo "$HF_COMMIT" > "$BASE/refs/main"; \
    for f in config.json tokenizer.json model.safetensors; do \
        curl -fsSL "https://huggingface.co/minishlab/potion-retrieval-32M/resolve/$HF_COMMIT/$f" -o "/tmp/$f"; \
        sha=$(sha256sum "/tmp/$f" | cut -d' ' -f1); \
        mv "/tmp/$f" "$BASE/blobs/$sha"; \
        ln -s "../../blobs/$sha" "$BASE/snapshots/$HF_COMMIT/$f"; \
    done; \
    find "$BASE" -type f -o -type l | sort

# ── Stage 2: compile the Rust binaries ──────────────────────────────────────
FROM rust:1-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
# The client is a separate crate (Dioxus) served from client/dist; it is NOT
# built here — the image is API/CLI-first. Mount a pre-built client bundle and
# point BRAIN_CLIENT_DIR at it if you want the web UI in the container (see
# docs/docker.md).
RUN cargo build --release --locked \
    && ls -la target/release/ \
    && for b in brain-server brain mcp brain-connector-stub; do \
         test -x "target/release/$b" || { echo "missing binary: $b"; exit 1; }; \
       done

# ── Stage 3: minimal runtime ────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 1000 brain \
    && useradd --uid 1000 --gid brain --create-home --shell /usr/sbin/nologin brain \
    && mkdir -p /data/keys /app \
    && chown -R brain:brain /data /app

# Bake the offline model cache (Stage 1). COPY preserves the snapshot symlinks.
COPY --from=model /opt/brain-model /opt/brain-model
COPY --from=builder /build/target/release/brain-server /usr/local/bin/brain-server
COPY --from=builder /build/target/release/brain       /usr/local/bin/brain
COPY --from=builder /build/target/release/mcp         /usr/local/bin/mcp
COPY --from=builder /build/target/release/brain-connector-stub /usr/local/bin/brain-connector-stub

# Fail-safe defaults (loopback-only). Public exposure is an explicit opt-in
# (BIND_PUBLIC=1) so a misconfigured port mapping cannot silently expose data.
ENV BIND_HOST=127.0.0.1 \
    BIND_PORT=8765 \
    BRAIN_DB_PATH=/data/brain.db \
    BRAIN_UMP_KEY_DIR=/data/keys \
    HF_HOME=/opt/brain-model \
    RUST_LOG=info

VOLUME ["/data"]
WORKDIR /app
USER brain

EXPOSE 8765

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8765/health || exit 1

ENTRYPOINT ["brain-server"]
