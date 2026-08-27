# channel-bridge — the governed-edge process for brain-server channels

One binary, three governed edges. The config FILENAME kind segment selects
the edge: `channel-{kind}-{tenant}.json` with kind ∈ `whatsapp` | `slack` |
`teams`. A config of a kind this binary does not ship refuses at load.

| Kind | Inbound surface | Outbound transport |
|---|---|---|
| `whatsapp` | loopback axum listener (`/webhooks/channel/whatsapp`; TLS terminates at the operator's reverse proxy) — Meta Cloud API webhooks, hub-signature verified before parse | Meta Cloud API, tier-paced |
| `slack` | **NONE — no inbound listener exists by construction** (Socket Mode dial-OUT over WSS) | Slack Web API `chat.postMessage` |
| `teams` | loopback axum listener (`POST /messaging`; TLS at the operator proxy) — Bot Framework activities, RS256-verified before parse | Bot Connector API (proactive activities) |

## The kernel seam (the ONLY way data crosses)

All kernel traffic is Standard-Webhooks signed with the config's
`webhook_secret` (`webhook-id` / `webhook-timestamp` / `webhook-signature`
headers, `v1,<base64 hmac-sha256>` tags):

- `POST {brain_url}/webhooks/channel/{kind}` — inbound envelopes
- `POST {brain_url}/webhooks/channel/{kind}/drain` — outbox claim (returns
  `envelopes` and, optionally, Relay handover `pings`)
- `POST {brain_url}/webhooks/channel/{kind}/console` — the operator console
  relay (`pending` / `decide` / `due` / `crank`)

**The bridge holds NO brain credentials, EVER.** No kernel bearer token, no
kernel database path — the HMAC `webhook_secret` is the only kernel
credential this process has. Platform bearer tokens (Slack, Bot Framework,
Graph) live in 0600 files referenced from the 0600 config.

## Least privilege at the workspace-app level (LAW)

A Teams/Slack token never grants anything beyond its mapped channels:

- **Slack**: install the workspace app with access to ONLY the mapped
  channels (the app is scoped to those conversations at install time — do
  not grant workspace-wide scopes). The `xapp-` app-level token is used for
  Socket Mode (`apps.connections.open`) ONLY; the `xoxb-` bot token is used
  for `chat.postMessage` ONLY. No reads of channel history, no user
  directory, nothing else. Both tokens live in 0600 files.
- **Teams**: the bot is registered for its team; activities from
  conversations outside `mapped_channels` are dropped before they cross
  the seam, and deliveries go only to mapped conversations. The Bot
  Framework client secret lives in a 0600 file.
- unmapped channels never cross in EITHER direction (drop + log), even
  when a token could technically reach them.

## Envelope contract (locked)

```json
{"envelope":{"conversation_ref":"C0123ABCD","text":"hello","external_id":"1712345678.123456","actor_ref":"U0PING1"}}
```

- `conversation_ref`: Slack channel id / Teams conversation id — MUST be in
  `mapped_channels` or the event is dropped.
- `external_id`: Slack message `ts` / Teams activity `id` — the kernel
  replay-caps on it.
- `actor_ref`: the platform USER id of the human sender, opaque, ≤128
  chars. NEVER a display name; display-name fields are never read.
- `text`: ≤ 4000 chars (truncation noted inside the text as `…[truncated]`).

## THE DIGEST LAW

Proposals render as Slack Blocks / Adaptive Cards with their digest shown
AND embedded in every Approve/Reject action payload. The bridge remembers
each digest it rendered (capped cache, oldest evicted); an approval action
MUST carry a 64-hex digest matching the cached digest EXACTLY or it is
refused before any kernel relay (logged at warn, never forwarded). The
kernel re-verifies the digest against stored bytes server-side — two
independent enforcement points. Slack slash-command approvals require a
rendered proposal in the current session; otherwise the operator is told
to use the proposal card buttons.

## Config: `channel-whatsapp-acme.json` (unchanged since Caravel)

```json
{
  "domain": "acme",
  "webhook_secret": "whsec-...",
  "verify_token": "...",
  "phone_number_id": "1234567890",
  "app_secret_path": "app_secret.txt",
  "access_token_path": "token.txt",
  "graph_api_version": "v21.0"
}
```

## Config: `channel-slack-acme.json`

```json
{
  "domain": "acme",
  "webhook_secret": "whsec-...",
  "app_token_path": "slack_app_token.txt",
  "bot_token_path": "slack_bot_token.txt",
  "mapped_channels": ["C0123ABCD", "C0HANDOV1"],
  "handover_channel": "C0HANDOV1"
}
```

- `slack_app_token.txt` (0600): the `xapp-` Socket Mode app-level token —
  used ONLY for `apps.connections.open`. Prefix-verified at load.
- `slack_bot_token.txt` (0600): the `xoxb-` bot token — used ONLY for
  `chat.postMessage`. Prefix-verified at load.
- `mapped_channels`: 1..=16 Slack channel ids, each `[A-Z0-9]+` ≤ 32 chars.
- `handover_channel` (optional): fallback target for Relay handover pings
  on roomless cases; absent = those pings are dropped with a warn log.

## Config: `channel-teams-acme.json`

```json
{
  "domain": "acme",
  "webhook_secret": "whsec-...",
  "bot_app_id": "00000000-0000-0000-0000-000000000001",
  "bot_tenant_id": "11111111-1111-1111-1111-111111111111",
  "bot_password_path": "teams_bot_password.txt",
  "mapped_channels": ["19:abc@thread.tacv2"],
  "handover_channel": "19:def@thread.tacv2"
}
```

- `bot_app_id` / `bot_tenant_id`: the Azure bot registration + tenant ids
  (GUID-ish, ≤ 64 chars).
- `teams_bot_password.txt` (0600): the Bot Framework client secret, used
  for client-credentials tokens toward the Bot Connector API (and, with
  the Graph scope, for `--list-channels`).
- `mapped_channels`: 1..=16 conversation ids, each ≤ 128 chars,
  control-char-free.

## Teams: the Bot Framework + Adaptive Cards route (supported path only)

- Every `POST /messaging` carries a Bot Framework JWT. The bridge verifies
  RS256 against the `login.botframework.com` JWKS (cached ≤ 1h, refreshed
  once on an unknown `kid`), requires `iss == "https://api.botframework.com"`
  and `aud == bot_app_id` — ALL before a single byte of the body is parsed.
  Verification failure = 401, body unread.
  - Ceiling: region-hosted channels that mint `ServiceUrl`-bound token
    audiences are not covered by `aud == bot_app_id`; the standard
    Emulator-off path is. Extend deliberately if you must serve one.
- Delivery-time `serviceUrl` (echoed by inbound activities) is host-
  ALLOWLISTED before any request: only the Microsoft regional relay
  `smba.trafficmanager.net` and the connector host `ccs.botframework.com`
  are accepted (https, case-insensitive, exact host — look-alike suffixes
  and userinfo smuggling refused). The activity body is JWT-envelope-
  verified but not signed, so the echoed `serviceUrl` is treated as
  untrusted data; the bot's connector bearer token never leaves toward
  anything else (`teams_service_url_is_host_allowlisted`).
- `type: "message"` activities with text become case notes;
  `Action.Submit` from Adaptive Cards (`value` without `text`) obeys the
  digest law and relays `decide`. The handler always answers `200 {}`
  quickly — Bot Framework retries aggressively.
- The deprecated O365-connector path (the Office-365-style connectors and
  any incoming-webhook-shaped surface) is deliberately NOT implemented.
  This bridge speaks Bot Framework and Adaptive Cards only — pinned by a
  source-text test so procurement reviewers can verify.
- Drain-time delivery uses the conservative regional service host
  `https://smba.trafficmanager.net/{bot_tenant_id}` — inside the same
  allowlist as the per-activity echo path.

## `--list-channels` (admin-gated, read-only)

```
channel-bridge --config channel-teams-acme.json --list-channels
```

Enumerates the operator's joined teams + their channels via Graph
(client-credentials token with the Graph scope), following pagination up to
5 pages per listing, and prints `id<TAB>name` lines to copy into
`mapped_channels`. Never writes anything; refuses on non-teams configs.

## Slack: Socket Mode, no listener (LAW)

The slack edge opens NO inbound socket — no listener, no accept, nothing to
expose. The process runs only the Socket Mode dial-OUT loop (a fresh
`wss://` URL per connection from `apps.connections.open`) and the drain
crank, multiplexed with `tokio::select!`. Socket Mode envelopes are ACKed
before any processing; one malformed frame never closes the socket;
reconnects back off exponentially with jitter (capped at 30s) and always
mint a fresh URL. Pinned by
`socket_mode_never_opens_an_inbound_listener`, including a source-text pin
over the adapter.

## Relay handover pings

Drained `pings` render per-platform (Slack `<@U…>` mentions / Teams
`<at>U…</at>`) carrying the I-PASS completeness state and any missing
items. Target: the case channel; roomless cases fall back to
`handover_channel`; with neither configured, the ping is dropped with a
warn log — never silently delivered somewhere random.

## Running

```sh
cargo build --release --bin channel-bridge
./target/release/channel-bridge --config /path/to/channel-teams-acme.json --port 8791
```

Validation:

```sh
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```
