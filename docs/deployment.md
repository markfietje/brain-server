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
- `brain` — the operator CLI (status, query, explain, ingest-dir, reconcile, resolve, backup, …).
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
| `BRAIN_DB_PATH` | `~/.openclaw/workspace/brain.db` | SQLite database path |
| `CORS_ORIGINS` | `http://localhost:3000,http://localhost:8080` | CORS allowlist (scheme included) |
| `AUTH_TOKEN` / `AUTH_TOKEN_FILE` | — | Opaque bearer token(s); newline-separated = live rotation; off if unset |
| `BRAIN_JWT_ISSUER` | — | Enables JWT mode when set + keys loaded |
| `INJECTION_POLICY` | `quarantine` | `quarantine` \| `reject` \| `allow` |
| `BRAIN_AUDIT_READ_EVENTS` | `on` (JWT) / `off` (loopback) | Read-event audit |
| `BRAIN_AUDIT_RETENTION_DAYS` | unset = forever | Audit retention window |
| `BRAIN_WEBHOOK_TIMESTAMP_REQUIRED` | `0` | `1` = require the Standard Webhooks header set on `/webhooks/*` and verify `v1,` HMAC-SHA256 over `{id}.{timestamp}.{body}` (v1.20.4) — an opt-in hard replay window for first-party senders. GitHub sends no such timestamp; its replay protection is `x-github-delivery` idempotency, so the default `0` leaves the legacy `sha256=` path unchanged |

See [Configuration](./configuration.md) and `src/config.rs` for the full list,
including the JWT key directory, PRF tuning, suggest kill-switch, and DSAR webhook.

---

## Security posture in deployment

- **Loopback-safe by default** — refuses `0.0.0.0` unless `BIND_PUBLIC=1`. In
  addition (v1.20.29) the server **fails closed on startup**: a non-loopback bind
  with no auth configured (no bearer token, no JWT keys) refuses to start, so an
  unauthenticated superuser API is never exposed off the loopback.
- **Two auth modes**:
  - **Opaque bearer** (default): `AUTH_TOKEN` / `AUTH_TOKEN_FILE`, constant-time
    compare, multiple tokens for rotation.
  - **JWT/JWS** (opt-in): set `BRAIN_JWT_ISSUER` + generate keys with
    `brain key generate`. RS256/ES256/EdDSA only; revocation + refresh-chain reuse
    detection; per-route AuthZ.
- **Auth token file is 0600.** The install script relocates any plaintext token out
  of the launchd plist into the secret file.

See [Security](./security.md) for the full model.

---

## Health & operations

```bash
brain doctor          # health + readiness
brain status          # counts, model, version
brain check-consistency   # duplicates, conflicts, stale sources
```

The audit log is read via the HTTP API (`GET /audit`) or the client console, not the `brain`
CLI (the CLI has no audit subcommand).

`/health` reports liveness plus a `capacity` object (docs / DB size / RSS) and a
`hardening` object (unsafe blocks, panics caught). Writes are guarded by a capacity
envelope — reads are never blocked.

---

## Security operations runbook (v1.20.5)

### Token rotation

The v1.20.2 machine-identity pattern: **agents are not shared service accounts.**
Give each agent principal its own token and rotate on a cadence (≤90d
recommended).

```bash
# opaque bearer: rotate atomically — fresh 0600 temp, fsync, rename (v1.27.12)
brain token rotate
# (or, manually: write a new token into the 0600 file; file-watch hot-reloads it)
umask 077 && head -c 32 /dev/urandom | base64 > ~/.config/brain-server/auth-token
# JWT mode: mint a fresh key, let the old one drain, then prune
brain key generate
# …wait ≥ max token lifetime (24h refresh)…
brain key prune
scripts/install-service.sh   # reload the key set
```

`brain token rotate` refuses to replace a group/world-readable token file and
the server fails closed at startup on wide secret modes (token file, JWT keys,
webhook signing secret, UMP signing keys — v1.27.12). Restart the server after
rotating (`scripts/install-service.sh`) to load the new token.

### Incident response — suspected memory poisoning

If a recall result, review item, or audit row looks planted:

1. **Review the blast radius** — `brain check-consistency` (near-dups +
   contradictions) + `GET /decayed` to see what is currently decayed.
2. **Propose the cleanup** — `GET /consolidate/propose` surfaces the duplicate /
   conflicting / stale-source candidates; approve the resolutions you trust.
3. **Purge the planted rows** — `POST /purge` by id/owner (hard, audited,
   tombstoned) or `POST /dsar {subject, action: purge}` for a subject-scoped
   sweep. Every purge leaves a tombstone + audit row.
4. **Re-verify the chain** — `GET /audit/verify` → `{"ok": true}`; the audit is
   tamper-evident, so the purge itself is provable.
5. **Rotate tokens** — steps above, so the planted session (if any) dies with
   the old credential.

### Classifier operations (v1.20.3, layer 2)

The optional ONNX classifier is off by default; when enabled:

- **FPR calibration** — watch the quarantine rate (`/audit` `quarantined` rows;
  the client Security panel surfaces the flag count). Tune
  `BRAIN_INJECTION_THRESHOLD_HIGH/LOW` — policy + thresholds read per call, so a
  flip takes effect without a restart (only the model load is cached).
- **Retrain trigger** — re-run adaptive evals on a threat-model shift (new
  obfuscation technique or delivery vector observed); the blocklist + quarantine
  stay the always-on defense while a retrain is pending.
- **Model artifact hash-pin** — pin the model file with `sha256sum` in the
  deployment config and verify on boot; the model file is itself a supply-chain
  artifact (LLM04/ASI04), so it is trusted like a dependency, not like a blob.

```bash
# pin the model artifact (the gate in the feature's docs)
sha256sum /path/to/model.onnx >> models.sha256
```

---

## Backup & restore

```bash
brain backup <out-path>    # AES-256-GCM encrypted, checksummed, excludes secrets (DB from BRAIN_DB_PATH/default)
brain restore <in-path>
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

See [Client GUI](./client-gui.md).

---

## Edge deployment (Jetson Nano / Raspberry Pi)

- Set `BRAIN_WORKER_THREADS=2` to trim RSS and context-switch overhead.
- The release profile is speed-optimized (`opt-level = 2`) and the memory ceiling is bounded and
  configurable (default `CAPACITY_MAX_RSS_MIB=512` on a 4 GB ARM device; RSS is
  an advisory soft signal, not a hard kill).
- No GPU, no embedding API, no Docker stack required.

---

## CRM case intake (v1.28.22 "Bridges")

`brain-connector-crm` (feature `connector-crm`) pulls support cases from
Zendesk, Salesforce, or Genesys Cloud into the universal loop — operator-
cranked via cron, one loop per invocation. Case bodies enter as proposals
under `BRAIN_WRITE_POSTURE=review`; envelopes open governed runs and post
`crm/case/updated` / `crm/case/closed` events. Config: 0600 JSON in
`~/.config/brain-server/connectors/` (`zendesk-*.json` =
`{subdomain, email, api_token_file}`; `salesforce-*.json` =
`{instance_url, client_id, client_secret_file, api_version?}`;
`genesys-*.json` = `{region, client_id, client_secret_file, worktype?, org_id?}`).

```cron
# Zendesk — every 5 minutes (respects the ~10 req/min incremental cap)
*/5 * * * * brain-connector-crm --source zendesk \
  --config ~/.config/brain-server/connectors/zendesk-acme.json \
  --checkpoint ~/.openclaw/workspace/brain.db >> ~/Library/Logs/brain-crm.log 2>&1

# Salesforce — incremental by SystemModstamp
*/5 * * * * brain-connector-crm --source salesforce \
  --config ~/.config/brain-server/connectors/salesforce-acme.json \
  --checkpoint ~/.openclaw/workspace/brain.db >> ~/Library/Logs/brain-crm.log 2>&1

# Genesys Cloud — workitems by worktype
*/10 * * * * brain-connector-crm --source genesys \
  --config ~/.config/brain-server/connectors/genesys-acme.json \
  --checkpoint ~/.openclaw/workspace/brain.db >> ~/Library/Logs/brain-crm.log 2>&1
```

Cursors persist in `crm-state-{source}-{org}.json` beside each config file.
Custom CRMs: see [connector-crm-custom.md](./connector-crm-custom.md).

---

## Next steps

- [Architecture](./architecture.md) — how the pieces fit together.
- [Security](./security.md) — the full threat model.
- [Compliance](./compliance.md) — regulatory mapping and data handling.
