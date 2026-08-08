# Deployment

Brain Server is designed to run as a persistent, self-managed service on a single
host. This page covers installing it, configuring it, keeping it healthy, and
backing it up.

---

## Service install (macOS)

`scripts/install-service.sh` builds the release binaries, installs them to
`~/.local/bin`, relocates the auth token from the launchd plist into a 0600 secret
file, restarts the service, and waits for `/health`. It is idempotent.

```bash
scripts/install-service.sh
```

This installs:

- `brain-server` — the server (launchd-managed, `KeepAlive=true`, `RunAtLoad=true`).
- `brain` — the operator CLI (status, query, ingest, reconcile, audit, …).
- `mcp` — the MCP bridge (search/recall/ingest as MCP tools).
- `bench` — the latency/recall harness.
- `brain-migrate-rehearse` — migration rehearsal / recovery.
- `brain-connector-stub` (and `brain-connector-gh` when the feature is enabled).

> **macOS note:** newly copied executables can get a `com.apple.provenance` xattr
> that Gatekeeper uses to SIGKILL on first exec (exit 137). The install script
> strips it. A manual `cp` does not.

---

## Configuration

Brain Server is configured through environment variables (all resolved in
`src/config.rs`). The most important:

| Variable | Default | Description |
|---|---|---|
| `BIND_HOST` | `127.0.0.1` | Bind address; `0.0.0.0` refused unless `BIND_PUBLIC=1` |
| `BIND_PORT` | `8765` | Listen port |
| `BRAIN_DB_PATH` | `brain.db` | SQLite database path |
| `CORS_ORIGINS` | `localhost:3000,localhost:8080` | CORS allowlist |
| `AUTH_TOKEN` / `AUTH_TOKEN_FILE` | — | Opaque bearer token(s); newline-separated = live rotation; off if unset |
| `BRAIN_JWT_ISSUER` | — | Enables JWT mode when set + keys loaded |
| `INJECTION_POLICY` | `quarantine` | `quarantine` \| `reject` \| `allow` |
| `BRAIN_AUDIT_READ_EVENTS` | `on` (JWT) / `off` (loopback) | Read-event audit |
| `BRAIN_AUDIT_RETENTION_DAYS` | unset = forever | Audit retention window |

See [README.md](../README.md#configuration) and `src/config.rs` for the full list,
including the JWT key directory, PRF tuning, suggest kill-switch, and DSAR webhook.

---

## Security posture in deployment

- **Loopback-safe by default** — refuses `0.0.0.0` unless `BIND_PUBLIC=1`.
- **Two auth modes**:
  - **Opaque bearer** (default): `AUTH_TOKEN` / `AUTH_TOKEN_FILE`, constant-time
    compare, multiple tokens for rotation.
  - **JWT/JWS** (opt-in): set `BRAIN_JWT_ISSUER` + generate keys with
    `brain key generate`. RS256/ES256/EdDSA only; revocation + refresh-chain reuse
    detection; per-route AuthZ.
- **Auth token file is 0600.** The install script relocates any plaintext token out
  of the launchd plist into the secret file.

See [Security](./security.md) and [SECURITY.md](../SECURITY.md) for the full model.

---

## Health & operations

```bash
brain doctor          # health + readiness
brain status          # counts, model, version
brain audit           # read the audit log
brain check-consistency   # duplicates, conflicts, stale sources
```

`/health` reports liveness plus a `capacity` object (docs / DB size / RSS) and a
`hardening` object (unsafe blocks, panics caught). Writes are guarded by a capacity
envelope — reads are never blocked.

---

## Backup & restore

```bash
brain backup <db> <out>    # AES-256-GCM encrypted, checksummed, excludes secrets
brain restore <backup> <db>
```

---

## The client GUI

The Dioxus control surface (`client/`) runs as a web app served by the server at
`/app`, and as a desktop / mobile app. It gives operators a visual surface for
review, recall, security, subjects (DSAR), audit, and health.

```bash
# In the client/ directory — build the web bundle, then deploy it
./deploy-web.sh
```

See [client/README.md](../client/README.md).

---

## Edge deployment (Jetson Nano / Raspberry Pi)

- Set `BRAIN_WORKER_THREADS=2` to trim RSS and context-switch overhead.
- The release profile is size-optimized and the memory ceiling is bounded
  (≤350 MB RSS on a 4 GB ARM device).
- No GPU, no embedding API, no Docker stack required.

---

## Next steps

- [Architecture](./architecture.md) — how the pieces fit together.
- [Security](./security.md) — the full threat model.
- [Compliance](./compliance.md) — regulatory mapping and data handling.
