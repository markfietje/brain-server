# signal-gateway — the presage Signal-daemon edge

A lightweight Signal daemon edge for brain-server's Switchboard channel line:
a Rust process that IS a linked Signal device (via
[presage](https://github.com/whisperfish/presage) — no signal-cli/JVM) and,
optionally, bridges that identity to the kernel over the governed Switchboard
seam. Tool root: `tools/signal-gateway/` (`README.md`, `Cargo.toml`,
`config.example.yaml`, `src/`, `tests/`).

> Era pin: this page is measured against the tree as read (package
> `signal-gateway` `0.99.0`, tracking the libsignal `v0.99.0` stack in
> `tools/signal-gateway/Cargo.toml` / `Cargo.lock`; Switchboard seam
> v1.28.43+ per `config.example.yaml` and `src/main.rs`). Correct this page
> when the tree moves — never the other way round.

## What it is

`signal-gateway` (`tools/signal-gateway/src/main.rs`) has two subcommands and
nothing else:

```sh
signal-gateway link   --config config.yaml --device-name signal-gateway
signal-gateway serve  --config config.yaml
```

- `link` generates a secondary-device link URL (`SignalHandle::
  link_secondary_device`): scan it with the primary Signal app to pair this
  process as a linked device. The identity persists in the presage SQLite
  store under `signal.data_dir` (`signal.db`).
- `serve` loads the linked account (`AppState::init_signal`), optionally arms
  the brain adapter (only when `brain:` is configured — otherwise it logs
  `no brain config — running channel-dark` and serves the local API only),
  then serves the local HTTP surface on `server.address`.

The presage worker (`src/signal/`: `worker.rs`, `commands.rs`, `types.rs`)
sends/receives over the live identity's websocket, with reactions and typing
indicators; the local API (`src/api/mod.rs`) exposes health, account info,
`POST /v2/send`, JSON-RPC (`POST /api/v1/rpc`: `sendMessage`, `sendReaction`,
`sendTyping`, …), recipient-cache seeding (`POST /v1/cache/seed`), and an SSE
stream (`GET /api/v1/events`). `#![forbid(unsafe_code)]` is compile-enforced
(`src/main.rs`, `src/lib.rs`, `Cargo.toml` `[lints.rust]`).

This is a working edge with a worker, an HTTP surface, a kernel adapter, and
integration tests (`tests/s8_01_bind_coupled_auth.rs`,
`tests/s8_04_rate_limit_wired.rs`, `tests/s9_02_cache_wiring.rs`) — not a
stub, not an experiment. Its ceilings are real anyway; they are listed under
[Honest limits](#honest-limits-ceilings).

## How it differs from valet-relay and channel-bridge

Three edges, three jobs. Do not substitute one for another:

| | `tools/signal-gateway` (this page) | `tools/valet-relay` | `tools/channel-bridge` |
|---|---|---|---|
| Runtime / transport | Rust-native via presage; IS the Signal device (linked secondary) | Zero-dependency Node; drives a **signal-cli REST** backend it does not own | Rust; speaks **Meta Cloud API / Slack Web API / Bot Framework** — no Signal at all |
| Documented in | This file; one passing mention in [architecture](./architecture.md) ("the channel bridge, the Signal gateway and the steward harness are separate packages under `tools/`") | [valet](./valet.md) ("Delivery edge") + `tools/valet-relay/README.md` | [deployment](./deployment.md) (Caravel/Herald sections) + `tools/channel-bridge/README.md`; kernel seams in [api](./api.md); pointer in [connectors](./connectors.md) |
| Kernel seam | Switchboard v1.28.43+: `POST /webhooks/channel/{kind}` (inbound), `POST …/drain` (outbound crank), `POST /workflow/plugins/mount` (boot registration) — all Standard-Webhooks HMAC with the shared bridge secret | Valet-era (v1.28.42): alert sink `/alert` + `POST /webhooks/signal` | Switchboard: same `/webhooks/channel/{kind}` + `/drain` (+ `/console` for Herald) for kinds `whatsapp` \| `slack` \| `teams` (kind selected by the config FILENAME segment) |
| Scope | Full-duplex Signal identity: any direct conversation, both directions | Valet ONLY: `valet/due` (later `valet/brief`) pings out, owner replies back | Case threads, Relay handover pings, digest-bound approvals in WhatsApp/Slack/Teams |
| Without kernel config | Runs **channel-dark**: local Signal API only (`src/state/mod.rs`, `src/main.rs`) | N/A (relay config is its whole job) | Runs channel-dark (absent config = channel dark) |

Concretely: if you need reminders on Signal, read [valet](./valet.md) and run
the relay. If you need WhatsApp/Slack/Teams case rooms, read the Caravel and
Herald sections of [deployment](./deployment.md) and run the bridge. If you
need a governed, kernel-attached **Signal identity** on the Switchboard seam,
you are in the right file.

## Setup / operation

1. Copy `tools/signal-gateway/config.example.yaml` to `config.yaml` and set
   `chmod 600` — `Config::load` (`src/config/mod.rs`) **refuses** any config
   with group/world bits set, because the file carries `server.auth_token`.
2. Set `signal.data_dir` / `attachments_dir` (the store holds identity keys
   and registration data; `AppState::new` in `src/state/mod.rs` tightens the
   dir to `0700` and `signal.db` to `0600`, warning loudly on failure).
3. `signal-gateway link --config config.yaml` — scan the printed URL with the
   primary app. Optionally set `signal.display_name` (see Privacy below).
4. `signal-gateway serve --config config.yaml` — serves loopback
   `127.0.0.1:8080` by default. A non-loopback `server.address` is refused
   unless `SIGNAL_GATEWAY_ALLOW_REMOTE=1` is exported at boot, AND a remote
   bind additionally requires `server.auth_token` — the two halves are the one
   coupled decision in `resolve_api_auth` (`src/lib.rs`), pinned by
   `tests/s8_01_bind_coupled_auth.rs`.
5. For kernel attachment, add the `brain:` section (era: Switchboard
   v1.28.43+): `url`, `bridge_config_path` (the SHARED 0600
   `channel-{kind}-{tenant}.json` the server also reads from its
   `BRAIN_CONNECTOR_CONFIG_DIR`), `drain_interval_secs` (default 30, floored
   to 5 in `start_brain_adapter`). Omit the section to stay channel-dark —
   the documented rollback posture.

Request-rate posture (all from `src/lib.rs` / `src/ratelimit.rs`, wired in
`src/main.rs` via `apply_rate_limit` on the FINISHED router, OUTSIDE auth so
the tokenless loopback arm is bounded too): one global budget of **100
requests per 60 s** (`API_RATE_LIMIT_MAX_REQUESTS` /
`API_RATE_LIMIT_WINDOW_SECS`, key `API_RATE_LIMIT_KEY = "api"`); over budget
is a bare `429` with `RETRY-AFTER: 60` and an empty body. Distinct from the
send path's concurrency cap: `max_sends_per_second` (5 in the example config)
bounds in-flight sends, not request rate — both bounds are live. The 100/60
constants are NOT operator-tunable by design (named constants in the library
target, shared by binary and tests).

Input validation (`src/validation.rs`): recipients must be UUID, E.164 phone
(`+` + 7–14 digits), or ACI (`u:<uuid>`); messages must be non-empty and
≤ 10000 chars. The recipient cache (`src/cache.rs`) is bounded (cap 4096,
oldest-quarter eviction; TTL on the phone leg) and never logs operands —
phone numbers and ACIs are identifiers.

## Credential posture — what it holds, what it never holds

HOLDS (all 0600-or-tighter, all its own):

- The presage Signal store (`signal.data_dir/signal.db`) — the linked
  identity's keys and registration data.
- Its own `config.yaml` — carries `server.auth_token`, hence the 0600 refusal
  at load.
- The SHARED bridge credential file (`channel-{kind}-{tenant}.json`:
  `domain` + `webhook_secret`), read from `bridge_config_path`. Owner-only
  permissions REQUIRED (`BridgeConfig::load` in `src/brain.rs` refuses
  otherwise); the filename's `channel-{kind}-{tenant}` segments select kind
  and tenant. One credential copy, read by both sides.
- The local API bearer token (`server.auth_token`), gating the FULL surface
  (reads and sends — both are identity-bearing; constant-time compare in
  `src/api/mod.rs`). Empty string counts as NO credential.

NEVER HOLDS (the governed-edge law, stated in `tools/signal-gateway/README.md`
and `src/main.rs`, pinned upstream by `bridge_holds_no_brain_credentials`):

- No brain-server token, no `Authorization` header toward the kernel, no
  brain database path. The ONLY kernel credential is the HMAC
  `webhook_secret`. The kernel stays channel-free by construction.
- Egress discipline mirrors the bridge: `BrainClient` (`src/brain.rs`) uses a
  15 s timeout and `redirect(Policy::none())` — signed webhook headers never
  ride a cross-origin redirect.

Kernel protocols (`src/brain.rs`, all HMAC-signed
`v1,<base64 hmac-sha256("{id}.{ts}.{body}")>`): INBOUND posts each received
direct text message as the normalized envelope projection
`{envelope: {conversation_ref, text, external_id}}` (sender UUID as
conversation ref; `external_id` = sender-uuid + platform timestamp, stable
across restarts for the replay cap); OUTBOUND drain crank claims approved
`channel/out` envelopes only; REGISTRATION posts mount evidence (SHA-256 of
the shared config file bytes, recomputed server-side) to
`/workflow/plugins/mount`, retried 5× with linear backoff.

Privacy posture (hidden & anonymous, per `README.md` + `src/signal/worker.rs`):
set `signal.display_name` to the Signal username created on the primary app
with number-discovery OFF — every API response, log line, and broadcast
payload then carries the label; unset falls back to masked digits (`+63…67`,
see `present_self_number`). Recipient addressing accepts usernames, resolved
server-side via presage `lookup_username` and cached as ACI
(`resolve_via_manager`). Ceiling, stated honestly upstream: Signal's servers
still know the account's number (protocol truth); anonymity here is from
CONTACTS AND OBSERVERS, not from Signal.

## Verification

What exists in-tree (cite only what is real):

- `signal-gateway serve` logs the linkage state at boot (`Signal linked` /
  `Signal not linked. Use 'link' command to pair.`), the auth posture
  (`API auth: bearer token required` vs loopback-only), and the rate-limit
  line — read them before sending anything.
- Liveness without identity: `GET /v1/health` → `{"status":"ok","version":
  "0.99.0"}`; `GET /v1/about` and `GET /api/v1/accounts` report the linked
  account (masked per the privacy posture). `GET /api/v1/events` opens the
  SSE stream (refuses unlinked with `{"error": "Not linked"}`).
- Kernel seam: `brain adapter armed for {kind}/{tenant} → {url}` plus
  `mount evidence registered for …` at boot; inbound posts and drain
  deliveries are logged per envelope (`external_id` / `event_id`).
- Test suite in-tree: unit tests in `src/` (`brain.rs` signature-vs-server-
  scheme, envelope projection, forwardability; `lib.rs` auth postures;
  `ratelimit.rs`; `config/mod.rs` 0600 refusal) plus `tests/s8_01_*` (coupled
  bind+auth), `tests/s8_04_*` (limiter behaviour + end-to-end 429s + a
  structural pin that fails if the wrap is removed), `tests/s9_02_*` (cache
  wiring). Run from the tool dir with `cargo test` (`Cargo.toml` notes CI
  runs test/clippy with `--locked` so the pinned presage/libsignal stack
  cannot re-resolve under a green build).
- Era note on the audit record: `docs/audit8/02-satellites-supply-chain.md`
  S8-01 (remote bind servable unauthenticated) and S8-04 (rate limiter a dead
  module) describe the PRE-FIX tree. The current `src/lib.rs` + `src/main.rs`
  + `tests/s8_*` show both closed (coupled `resolve_api_auth`; limiter
  wrapped outermost). Trust the sources cited here over the finding text if
  they ever disagree — and re-check before quoting either.

## Honest limits (ceilings)

- **Direct conversations only.** `forwardable` (`src/brain.rs`) admits
  non-empty text with NO group id; group messages are dropped on the
  inbound leg today ("group threading rides the line roadmap"). Outbound
  drain delivers to `conversation_ref` as given.
- **At-least-once with a loud edge.** The drain marks rows delivered
  server-side; a Signal send that then fails CANNOT be retried by the crank
  — `drain_once` (`src/state/mod.rs`) logs `DELIVERY FAILED` at error. Watch
  the edge logs; the server will not redeliver.
- **Mount evidence is bounded.** Registration retries 5×, then stops with
  `mount evidence NOT registered after 5 attempts` — the loss surfaces as a
  chain gap upstream, not as silence. Do not assume a quiet edge is a
  registered edge.
- **The 100-request burst still reaches Signal.** The rate limiter bounds the
  HTTP surface, not the network: a full budget spent on `/v2/send` is 100
  real sends, and the SSE long-poll on `/api/v1/events` draws from the same
  global budget. Size operators' expectations (and tokens) accordingly.
- **Pinned crypto stack, deliberately.** Package version tracks the libsignal
  tag (`0.99.0` via presage rev `f74b96e0…`); the stack-policy note in
  `Cargo.toml` says riding presage forward past this rev is a deliberate,
  reviewed act (re-lock + version bump together), because cargo `[patch]`
  cannot re-point same-URL git pins. `serde_yaml` is held at `0.9.34`
  (deprecated upstream; the rename to `serde_yml`/`serde_norway` is
  behavioural, not a bump). Quote `0.99.0` with its date, not as "latest".
- **Number-less accounts are not supported upstream.** Fully
  self-registering without a phone number is not something presage/Signal
  offers; the privacy posture hides the number from contacts and observers,
  never from Signal's servers.
- **Partial API surfaces.** `GET /v1/receive/{number}` is a stub that answers
  `{"error": "Use /api/v1/events for SSE stream"}` (no WebSocket);
  `listGroups`/`getGroups` answer `{"groups": []}`; `sendReadReceipt`/
  `markRead` answer `null` (no-op). `POST /v1/cache/seed` is integrity-
  bearing (a wrong phone→UUID mapping misdelivers) and is therefore logged
  at WARN with SHA-256 digests, never operands.
- **Loopback is the only unauthenticated posture.** Anything routable demands
  `SIGNAL_GATEWAY_ALLOW_REMOTE=1` AND a token; there is no flag that waives
  authentication, only one that permits reaching the port.
