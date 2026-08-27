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

> **Optional:** `brain-connector-crm` (feature `connector-crm`) is built
> best-effort by `install-service.sh` — present only when the feature was
> enabled for a prior build; the script compiles it on the first run that
> needs it and skips cleanly otherwise, same posture as `brain-connector-gh`.
> The cron recipes in [CRM case intake](#crm-case-intake-v12822-bridges)
> below need it installed.

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

## The personal assistant crank (v1.28.42 "Valet")

The trinity holds: cron or socket, never a daemon in the kernel. A reminder
is just a governed run whose SLA envelope came due; `brain valet due` is a
request-scoped, idempotent crank (outbox key `valet-{run}-{due_at}` — a double
cron never double-fires). The Signal bridge is a separate zero-dependency
edge process (`tools/valet-relay/relay.js`) holding ONLY its own 0600 config:
it receives the server's signed alert envelopes and forwards `valet/due`
pings; your replies flow back through `/webhooks/signal` (HMAC-verified,
replay-capped, injection-screened — every inbound byte is untrusted).

```cron
# The scheduler IS the cron recipe — every 15 minutes, weekdays.
*/15 * * * 1-5 brain valet due >> ~/Library/Logs/brain-valet.log 2>&1

# The morning brief, once a day at 07:30.
30 7 * * * brain valet brief >> ~/Library/Logs/brain-valet.log 2>&1
```

Setup: `brain valet consent grant` (the one-subject Outreach-lite registry —
without it, envelopes fire locally but nothing is sent), then run the relay
under launchd/KeepAlive with `BRAIN_ALERT_WEBHOOK_URL` pointing at its
`/alert` listener and `BRAIN_SIGNAL_WEBHOOK_SECRET_FILE` mirroring the relay
secret. Content-plan import: `scripts/import-content-plan.ts plan.csv
[--dry-run]` creates one `valet/reminder` run per planned post.

---

## The WhatsApp governed edge (v1.28.44 "Caravel")

WhatsApp is governance MAPPING, not invention — Meta enforces the discipline;
the adapter translates platform law onto kernel law. The edge is a separate
Rust process (`tools/channel-bridge`, config-off by default: absent config =
channel dark) that owns the PUBLIC webhook surface so brain-server never does:

- **Handshake + signature.** Meta's subscription GET (`hub.challenge`) is
  answered BY THE EDGE — the kernel never sees a challenge. Every POST is
  verified against `X-Hub-Signature-256` (raw-body HMAC-SHA256 with the app
  secret, length-checked, constant-time) BEFORE any parse; only then are
  payloads projected into normalized envelopes, signed Standard-Webhooks
  style, and forwarded to `POST /webhooks/channel/whatsapp`. Verified bytes
  are the ONLY thing the kernel receives.
- **The 24-hour window rides the kernel gate exactly.** Free-form replies
  inside 24h of the customer's last inbound; outside it ONLY template
  messages — and a template send is a PROPOSAL (`channel/template`):
  double-approved by construction (Meta's registry AND ours; ours carries
  the content digest). Business-initiated contact needs ALL THREE gates
  every time: template + standing consent in the shared registry + approved
  digest-bound proposal.
- **Statuses become lineage.** sent/delivered/read/failed receipts land as
  `case/channel_status` outbox events on the thread's case — hashes and refs
  on the audit chain, bodies never.
- **Quality tiers throttle deterministically.** The tier state lives in a
  0600 file under the state dir; a FRESH state is the MOST RESTRICTIVE tier
  until a status webhook upgrades it (fail-closed). Downgrades alert the
  operator via the bus metadata-only (number alias + old/new tiers).
- **Media digests-and-quarantine.** Attachments downloaded by the edge are
  SHA-256'd; bytes sit in the retention dir named by digest, never auto-
  opened, never proxied through brain-server to a browser. Only the hash
  rides inbound (recorded verbatim ON the case note).

### Config (`$BRAIN_CONNECTOR_CONFIG_DIR/channel-whatsapp-{tenant}.json`, 0600)

The SAME substrate file both sides read (domain + webhook_secret for the
kernel seam; the WhatsApp keys for the edge):

```json
{
  "domain": "acme",
  "webhook_secret": "whsec-…",
  "verify_token": "…",
  "phone_number_id": "1234567890",
  "app_secret_path": "app_secret.txt",
  "access_token_path": "access_token.txt"
}
```

Secret files are 0600, referenced by path (relative resolves beside the
config); upward traversal refuses. Optional `graph_api_version` pins the
Cloud API (default `v21.0`) — re-verify the account-quality webhook taxonomy
against the pinned version at deploy.

### Running

```sh
# Build the edge.
cargo build --release -p channel-bridge --manifest-path tools/channel-bridge/Cargo.toml

# Run (TLS terminates at YOUR reverse proxy in front of the loopback port).
tools/channel-bridge/target/release/channel-bridge \
  --config $BRAIN_CONNECTOR_CONFIG_DIR/channel-whatsapp-acme.json \
  --port 8791 --brain-url http://127.0.0.1:8765 \
  --retention-dir /var/lib/brain-server/channel-media \
  --state-dir /var/lib/brain-server/channel-bridge-state \
  --tick-secs 5
```

Run it under launchd/systemd KeepAlive like any governed edge. No extra cron:
outbound drain is an internal tick loop paced by the tier table (throttled
rows defer to later ticks). Registration evidence posts at boot over the
same HMAC seam (`channel:whatsapp` mount, config-digest recomputed server-
side). Template sends use parameterless templates (parameterized components
are a documented ceiling).

---

## The Slack and Teams operator annexes (v1.28.45 "Herald")

The channels operators already live in become the console's ANNEXES: case
rooms, Relay handover pings, and digest-bound approvals where the people
are. Both adapters are edge processes in the SAME `tools/channel-bridge`
binary (config-off by default: absent config = channel dark), and the
kernel-side pieces they ride are the SAME two HMAC seams as WhatsApp plus
ONE new console seam:

### Slack (Socket Mode)

- **No inbound listener exists by construction.** The bridge DIALS Slack
  over the Socket-Mode WebSocket (`apps.connections.open` → wss, reconnect
  with capped exponential backoff + jitter). The `slack` kind binds NOTHING
  — pinned by `socket_mode_never_opens_an_inbound_listener`.
- `message` events in the config's `mapped_channels` become screened case
  notes through the ordinary inbound seam (thread map or `[case N]`);
  the sender's OPAQUE user id rides as `actor_ref` (display names are never
  read).
- **Approve-by-button:** pending renderable proposals render as Slack Blocks
  with the content preview AND the digest in the block; Approve/Reject
  buttons carry that digest in their value. A click whose digest is missing
  or mismatched is refused BRIDGE-SIDE (logged, never relayed) — and the
  kernel re-verifies it server-side. Two independent enforcement points.
- **Slash commands** `/brain due`, `/brain crank <run>`, `/brain approve
  <id>`, `/brain pending [limit]` relay over the console seam; the kernel
  maps the clicking user through the user map and role-checks there.
- **User map:** a Slack user is NOBODY until an approved
  `channel/user_map` proposal maps their opaque id to a principal with
  explicit roles. There is no auto-trust path.
- **Presence:** mapped operator activity feeds the Crew roster as the
  closed activity kind `channel` — activity KINDS only, never content, and
  only while the domain's Crew DPO switch is on.

### Teams (Bot Framework + Adaptive Cards)

- The supported Bot Framework route ONLY: the bridge registers an Azure
  bot, exposes `POST /messaging` behind the operator's TLS proxy, verifies
  every activity's Bot Framework JWT (JWKS, `iss`/`aud` pinned) BEFORE any
  parse, and answers with Adaptive Cards. The deprecated O365-connector
  path is deliberately NOT implemented.
- Activities in mapped conversations become screened case notes (same
  threading law); proposal cards carry the digest field and `Action.Submit`
  returns it — the same digest binding as Slack buttons.
- **Room mapping:** `channel-bridge --config channel-teams-acme.json
  --list-channels` enumerates the bot's teams/channels via Graph
  (read-only, operator-run) so the operator can copy ids into
  `mapped_channels`.

### Relay handover pings

When a handover OFFER is created, ONE `channel/ping` outbox row is enqueued
with the I-PASS completeness state (refs only). The bridge drain resolves
the receiving operator's mapped platform refs + the case room and posts the
ping in-channel (the case's room; else the config's `handover_channel`);
an unmapped principal is audited loud and consumed — the drain never
wedges. Accept/decline stays on the console (the ping coaches; the human
decides there).

### The user map (kernel side)

`POST /workflow/channel/user-map` FILES a `channel/user_map` proposal
(`{action: add|remove, channel, tenant, platform_user_id, principal,
roles[]}`); approval is the ONLY writer of the `channel_user_map` table
(schema 1.28.45, additive). Roles resolve against the role store at file
AND apply time. The console seam denies any actor that is unmapped,
unroled, or lacking the action's capability — 403, audited.

### Config examples (0600, same substrate law as WhatsApp)

```json
// channel-slack-acme.json
{
  "domain": "acme",
  "webhook_secret": "whsec-…",
  "mapped_channels": ["C0123ABCD"],
  "handover_channel": "C09HANDOVER",
  "app_token_path": "slack_app_token.txt",
  "bot_token_path": "slack_bot_token.txt"
}

// channel-teams-acme.json
{
  "domain": "acme",
  "webhook_secret": "whsec-…",
  "mapped_channels": ["19:…@thread.tacv2"],
  "bot_app_id": "00000000-0000-0000-0000-000000000000",
  "bot_tenant_id": "00000000-0000-0000-0000-000000000000",
  "bot_password_path": "teams_bot_password.txt"
}
```

**Least privilege at the workspace-app level:** install the Slack app with
access scoped to the mapped channels only, and the Teams bot to its team
only; channel tokens grant nothing beyond their mapped channels. Tokens
live in 0600 files referenced by path — the bridge holds NO brain token,
ever (pinned house-wide by self-grep).

### Running

```sh
cargo build --release -p channel-bridge --manifest-path tools/channel-bridge/Cargo.toml

# Slack: dials OUT; binds nothing.
tools/channel-bridge/target/release/channel-bridge \
  --config $BRAIN_CONNECTOR_CONFIG_DIR/channel-slack-acme.json \
  --brain-url http://127.0.0.1:8765 --tick-secs 5

# Teams: one loopback listener behind YOUR TLS proxy.
tools/channel-bridge/target/release/channel-bridge \
  --config $BRAIN_CONNECTOR_CONFIG_DIR/channel-teams-acme.json \
  --port 8792 --brain-url http://127.0.0.1:8765 --tick-secs 5

# Teams room mapping (operator-run, read-only):
tools/channel-bridge/target/release/channel-bridge \
  --config $BRAIN_CONNECTOR_CONFIG_DIR/channel-teams-acme.json --list-channels
```

---

## Deployment tiers (ISO 18295-1 applicability: any size)

The standard applies to a centre of any size; so does this server. The same
binary scales from one operator to a global BPO by configuration, not by
forks. Pick the tier that matches the operation — every tier ships the full
audit chain and fail-closed gates.

**Tiers are config, not forks:** each tier is a checked-in env profile —
`deploy/tiers/t1.env`, `deploy/tiers/t2.env`, `deploy/tiers/t3.env`,
`deploy/tiers/t4.env` — that CI boots as part of the tier-smoke matrix, and a
meta-test (`guide_and_profiles_never_drift`) fails if a profile sets a key
this guide does not document (or vice versa). Copy the profile into your
service environment and add only site-specific values (`BRAIN_DB_PATH`,
`BIND_PORT`, auth material).

| Tier | Who | Shape | Profile |
|---|---|---|---|
| **T1 solo** | One operator / micro-centre | loopback bind, single domain, single DB, no roles | [`deploy/tiers/t1.env`](../deploy/tiers/t1.env) |
| **T2 team** | A small team (≤ ~25 agents) | roles enabled, HITL proposal review queue on, crew presence visible | [`deploy/tiers/t2.env`](../deploy/tiers/t2.env) |
| **T3 site** | A site or BPO campaign | multi-domain/multi-DB, calibration + public KB feedback live, WFM feeds feeding the centre's tool | [`deploy/tiers/t3.env`](../deploy/tiers/t3.env) |
| **T4 global** | Multi-site / multi-region | T3 plus knowledge parcels, residency stamps, follow-the-sun handover via the shift ring | [`deploy/tiers/t4.env`](../deploy/tiers/t4.env) |

### Per-tier config matrix

| Variable | T1 solo | T2 team | T3 site | T4 global | Why |
|---|---|---|---|---|---|
| `BRAIN_WRITE_POSTURE` | `open` (the operator IS the reviewer; proposals still audited) | `review` | `review` | `review` | agent writes become HITL proposals from T2 up |
| `BIND_PUBLIC` | `0` | `0` | `0` | `0` | never expose without auth; the server refuses any non-loopback bind with none regardless of tier |
| `BRAIN_AUDIT_READ_EVENTS` | off (loopback default) | `on` | `on` | `on` | shared surfaces get read-audited once more than one person uses them |
| `BRAIN_MULTI_DB` | unset | unset | `1` | `1` | domain-per-campaign databases at site scale |
| `BRAIN_MAX_DOMAIN_DBS` | unset | unset | `16` | `64` | explicit cap under the bounds law; size to your domain count |
| `BRAIN_WEBHOOK_TIMESTAMP_REQUIRED` | `0` | `0` | `1` | `1` | replay-hard webhook intake for first-party senders at site scale |
| `BRAIN_OTEL_ENABLED` | unset | unset | optional | `1` | instrumented decision cores for multi-region ops visibility |
| `BRAIN_TRUST_PROXY` | unset | unset | optional | `1` | set only when TLS terminates on a trusted proxy chain |

### Sizing guidance

SQLite WAL headroom is the sizing lever, not heroics: keep the WAL under a
few hundred MB by running `brain backup` (which checkpoints) on the cadence
below, and promote to `BRAIN_MULTI_DB` when a single DB's write contention
or backup window stops fitting the maintenance slot. On edge ARM hardware
set `BRAIN_WORKER_THREADS=2` and keep `CAPACITY_MAX_RSS_MIB` at its 512
default. No sizing promise beyond what you measure — `bench` against YOUR
corpus before promoting a tier.

### Cadences (cron recipes)

| Cadence | T1 | T2 | T3 | T4 |
|---|---|---|---|---|
| CRM connector sync | — | daily | every 5–10 min (see [CRM case intake](#crm-case-intake-v12822-bridges)) | every 5–10 min per site |
| `brain backup` | weekly | nightly | nightly + pre-calibration | nightly per region |
| KB build / publish (`kb build`) | ad hoc | weekly | daily + feedback-loop driven | daily per locale set |
| Human-signed calibration | — | quarterly | monthly (the signed register extract rides it, v1.28.37) | monthly per site |
| Valet crank (`brain valet due`) | — | — | weekdays every 15 min (the personal-assistant heartbeat, v1.28.42) | same, per operator |
| Token rotation | ≤90d | ≤90d | ≤90d | ≤90d (staggered per principal) |

### Upgrade path

Tier promotion is additive: nothing configured at T1 blocks T4 features
later. Move up by merging the next profile's keys into your environment,
restarting, and re-running the smoke suite (`brain doctor`,
`brain check-consistency`, `GET /audit/verify`). There is no downgrade
migration either — drop back by removing keys, never by editing data. The
deliberate ceilings (workload visibility is measured, never enforced; no
forecasting/scheduling engines — WFM alignment is interop) hold at every
tier.

## Next steps

- [Architecture](./architecture.md) — how the pieces fit together.
- [Security](./security.md) — the full threat model.
- [Compliance](./compliance.md) — regulatory mapping and data handling.
