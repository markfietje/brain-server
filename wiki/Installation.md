# Installation

Brain Server is designed to run as a persistent, self-managed service on a single host. This page covers installing it, running it as a service, and deploying to edge hardware.

## Building from source

```bash
# Build all binaries
cargo build --release --features bench

# With the GitHub connector
cargo build --release --features bench,connector-github
```

This produces:

| Binary | Purpose |
|---|---|
| `brain-server` | The server |
| `brain` | The operator CLI (status, query, ingest, reconcile, audit, …) |
| `mcp` | The MCP bridge (search/recall/ingest as MCP tools) |
| `bench` | The latency/recall harness |
| `brain-migrate-rehearse` | Migration rehearsal / recovery |
| `brain-connector-stub` | Reference connector (with `connector-github`: `brain-connector-gh`) |

## Service install (macOS)

`scripts/install-service.sh` builds the release binaries, installs them to `~/.local/bin`, relocates the auth token from the launchd plist into a 0600 secret file, restarts the service, and waits for `/health`. It is idempotent.

```bash
scripts/install-service.sh
```

> **macOS note:** newly copied executables can get a `com.apple.provenance` xattr that Gatekeeper uses to SIGKILL on first exec (exit 137). The install script strips it. A manual `cp` does not.

## Configuration

Brain Server is configured through environment variables (all resolved in `src/config.rs`). See the **[Configuration](Configuration)** page for the full reference. The most important:

| Variable | Default | Description |
|---|---|---|
| `BIND_HOST` | `127.0.0.1` | Bind address; `0.0.0.0` refused unless `BIND_PUBLIC=1` |
| `BIND_PORT` | `8765` | Listen port |
| `BRAIN_DB_PATH` | `~/.openclaw/workspace/brain.db` | SQLite database path |
| `CORS_ORIGINS` | `http://localhost:3000,http://localhost:8080` | CORS allowlist |
| `AUTH_TOKEN` / `AUTH_TOKEN_FILE` | — | Opaque bearer token(s); newline-separated = live rotation; off if unset |
| `BRAIN_JWT_ISSUER` | — | Enables JWT mode when set + keys loaded |
| `INJECTION_POLICY` | `quarantine` | `quarantine` \| `reject` \| `allow` |
| `BRAIN_AUDIT_READ_EVENTS` | `on` (JWT) / `off` (loopback) | Read-event audit |
| `BRAIN_AUDIT_RETENTION_DAYS` | unset = forever | Audit retention window |

## Health & operations

```bash
brain doctor          # health + readiness
brain status          # counts, model, version
brain audit           # read the audit log
brain check-consistency   # duplicates, conflicts, stale sources
```

`/health` reports liveness plus a `capacity` object (docs / DB size / RSS) and a `hardening` object (unsafe blocks, panics caught). Writes are guarded by a capacity envelope — reads are never blocked.

## Backup & restore

```bash
brain backup <db> <out>    # AES-256-GCM encrypted, checksummed, excludes secrets
brain restore <backup> <db>
```

## The client GUI

The Dioxus control surface (`client/`) runs as a web app served by the server at `/app`, and as a desktop / mobile app. It gives operators a visual surface for review, recall, security, subjects (DSAR), audit, and health.

```bash
# In the client/ directory — build the web bundle, then deploy it
./deploy-web.sh
```

See the **[Client GUI](Client-GUI)** page for details.

## Edge deployment (Jetson Nano / Raspberry Pi)

- Set `BRAIN_WORKER_THREADS=2` to trim RSS and context-switch overhead.
- The release profile is size-optimized and the memory ceiling is bounded (≤350 MB RSS on a 4 GB ARM device).
- No GPU, no embedding API, no Docker stack required.

## Next steps

- **[Configuration](Configuration)** — the full environment-variable reference.
- **[Security](Security)** — the threat model and controls.
- **[Governance & Compliance](Governance-and-Compliance)** — regulatory mapping and data handling.
