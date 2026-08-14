# Docker Deployment (A1)

> Enterprise plan §33.2 Phase A1 / §33.3 item 2. `docker compose up` should
> put a pilot online in under five minutes — the first buyer conversation
> happens in a browser, not a terminal.

## Image facts

- Multi-arch: **linux/amd64 + linux/arm64** (matches the release workflow).
- The embedding model (`minishlab/potion-retrieval-32M`, ~124 MB) is **baked
  into the image at build time** (`HF_HOME=/opt/brain-model`), so the
  container boots **offline** — no HuggingFace call at first start. This is
  the enterprise/air-gapped posture; the pinned revision (`HF_COMMIT` build
  arg) makes the bake reproducible.
- Runtime: `debian:bookworm-slim`, non-root user `brain` (uid 1000),
  `read_only` rootfs + tmpfs, `cap_drop: ALL`, `no-new-privileges`.
- Healthcheck: `curl /health` (the endpoint is always auth-exempt by design).
- Loopback-safe default preserved: `BIND_HOST=127.0.0.1`; public binding
  requires `BIND_PUBLIC=1` explicitly.

## Build

```bash
docker build -t brain-server:local .
# change the pinned model revision if you ever need to:
docker build --build-arg HF_COMMIT=<revision> -t brain-server:local .
```

## Run (single container)

```bash
docker run -d --name brain-server \
  -p 127.0.0.1:8765:8765 \
  -v "$PWD/data:/data" \
  -e BIND_HOST=0.0.0.0 -e BIND_PUBLIC=1 \
  -e AUTH_TOKEN=<token> \
  brain-server:local
```

State lives under `/data` in the container:

| Path | Purpose |
|---|---|
| `/data/brain.db` | SQLite store (`BRAIN_DB_PATH`) |
| `/data/keys/` | JWT signing/verification PEMs (`BRAIN_JWT_KEY_DIR`); the UMP operator Ed25519 key lives under `/data/ump/` (`BRAIN_UMP_KEY_DIR`) |
| `/data/auth-token` | opaque bearer token file (0600) |

## Compose (recommended)

```bash
docker compose up -d                      # API-first pilot, loopback only
docker compose --profile sso up -d        # + OAuth2-Proxy SSO edge
```

See `docker-compose.yml` for the full service definition and
[docs/proxy-sso.md](./proxy-sso.md) for the SSO profile.

## Web client (optional)

The Dioxus GUI is **not** built into the image (it is a separate crate served
from `client/dist`). To serve the UI from the container, build the bundle
(`client/deploy-web.sh`) and mount it:

```yaml
    volumes:
      - ./client/dist:/app/client/dist:ro
    environment:
      BRAIN_CLIENT_DIR: /app/client/dist
```

## Backup / restore

The `brain` CLI is in the image:

```bash
docker exec brain-server brain backup /data/backup-$(date +%F).bin
# restore (with the server stopped):
docker stop brain-server
docker run --rm -v "$PWD/data:/data" brain-server:local \
  brain restore /data/backup-YYYY-MM-DD.bin --passphrase-file /data/pass
docker start brain-server
```

Retention + restore drill are queued as **v1.19 A5** (`BRAIN_BACKUP_RETENTION`).

## Publishing (A1 follow-up)

Image publish to GHCR/Docker Hub (`markfietje/brain-server`) is the remaining
distribution step — the Dockerfile and compose land first; publish is a
workflow + credentials item (see report Round 27).
