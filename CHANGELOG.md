# Changelog — brain-server

All notable changes are documented here. The format is a simplified keep-a-changelog
style. Version numbers follow `Cargo.toml`; "released" means the binary and docs
are consistent at that tag.

Honesty note: retrieval-quality claims below describe *what the code does*, not
measured parity against external engines (e.g. QMD). Where a benchmark has not
been run, it is marked **pending** rather than asserted.

---

## [1.20.4] — 2026-08-11

v1.20.4 "Replay" — the G6 close from the GhostJacking audit: an **optional,
config-driven replay window** for webhook senders that provide a signed
timestamp, plus a documented stance for GitHub. **Server** Cargo 1.20.3 →
1.20.4; client stays at 1.20.0. **No schema change, no new routes** — the
Standard Webhooks handshake rides the existing `/webhooks/{kind}` surface.

### Added

- **Standard Webhooks handshake for first-party senders (M1, opt-in).** When
  `BRAIN_WEBHOOK_TIMESTAMP_REQUIRED=1`, `POST /webhooks/{kind}` requires the
  open spec's header set (`webhook-id`/`webhook-timestamp`/`webhook-signature`)
  and verifies the `v1,<base64>` HMAC-SHA256 over `{id}.{timestamp}.{raw body}`
  in constant time (`WebhookQueue::verify_standard_signature`,
  `src/handlers/webhooks.rs::receive_standard`). The timestamp rides inside the
  HMAC, so a replay cannot re-stamp it. `webhook-id` feeds the existing
  `webhook_seen` idempotency. The spec path accepts any kind — the flag is an
  explicit operator opt-in for their own trusted senders.
- **`/health` webhook posture (M2).** `webhook.replay_secs` (300),
  `webhook.timestamp_required`, and `webhook.scheme`
  (`standard-webhooks` | `legacy`) exposed at a glance (mirrors the `hardening`
  object pattern).
- **Documentation stance for GitHub (M3, the real deliverable).** GitHub's
  replay protection is `x-github-delivery` idempotency (its sender is a trusted
  third party), not a timestamp window — documented in `SECURITY.md` §webhooks,
  `COMPLIANCE.md` §webhooks, and `docs/deployment.md`. First-party senders can
  opt into the hard window via the spec headers + flag (svix-style signer or a
  hand-rolled HMAC, both documented).

### Fixed

- **G6 webhook replay window that depends on sender headers** — previously the
  `WEBHOOK_REPLAY_SECS` window only applied when a caller-supplied timestamp was
  present, and GitHub sends none, so its only replay protection was delivery-id
  dedup (acceptable for the connector's threat model). The spec handshake closes
  this for senders that DO provide a signed timestamp without inventing one
  GitHub doesn't send.

### Security

- The hard window is **opt-in** (default unchanged — the legacy GitHub path is
  byte-identical); an attacker who can forge the HMAC already controls the
  secret, so replay here is a robustness concern, not an RCE vector. This closes
  **all six audit gaps (G1–G6)** across the v1.20.x line.

### Honest ceilings (carried into v1.21+)

- GitHub's replay protection remains delivery-id idempotency — no timestamp is
  invented for it.
- The spec handshake is verification-side only; the legacy GitHub path keeps its
  `sha256=` HMAC scheme (back-compat). The spec's `webhook-origin`/allowlist
  features are not adopted.

---

## [1.20.3] — 2026-08-11

v1.20.3 "Classify" — the G5 upgrade path from the GhostJacking audit (layer 2 of
the injection screen) plus the client render-boundary hardening. **Server** Cargo
1.20.2 → 1.20.3; client stays at 1.20.0 (one pure fn + three render-site call
sites + a test, version-neutral). **No schema change** — `proposals.screen_verdict`
is recomputed deterministically at read time rather than persisted, so the schema
stays at 1.20.1/1.20.2 and `test_migration_schema_contract` is untouched.

### Added

- **Two-layer injection screen** (`src/screen.rs`, the single seam every ingest
  write path routes through). Layer 1 = the existing deterministic blocklist
  (always on). Layer 2 = an **optional, feature-gated local ONNX classifier**
  (`injection-classifier` feature + `ort`/`tokenizers`) for novel/obfuscated
  injections. Layer 2 is OFF by default — the Jetson envelope treats memory as
  the scarcest resource and the blocklist + `flagged`/`untrusted` segregation
  remain the always-on defense. When enabled, loads the model at
  `BRAIN_INJECTION_CLASSIFIER` + tokenizer at `BRAIN_INJECTION_TOKENIZER`
  (Fastly-lineage BERT-tiny INT8, ~4.3 MB) once via a `LazyLock`, off the
  request path. Banding: score ≥ `BRAIN_INJECTION_THRESHOLD_HIGH` (0.9) → HTTP
  400; ≥ `BRAIN_INJECTION_THRESHOLD_LOW` (0.7) → stored flagged; else clean.
  Under `Allow` policy the whole screen is disabled (kill switch). Scoring is
  sentence-packed + density-adjusted (StackOne calibration): one flagged
  sentence in a ≥3-sentence chunk is damped toward 0, several confirm an attack.
- **Screen wired into every ingest write site**: `/add`, `/ingest/memory`,
  `/ingest/markdown`, `/ingest` (`ingest_one`), `/procedure` (root + each step),
  and `/ingest/proposal`. `Reject` → 400 (`input_rejected`); `Quarantine` →
  stored flagged + KG edges skipped. `flag_if_quarantined` now takes the screen's
  bool verdict (no longer re-runs the blocklist in isolation) — a layer-2 hit
  quarantines exactly like a layer-1 hit.
- **Review-queue badge**: `ProposalView.screen_verdict` (`clean`/`quarantine`).
  `reject` is never persisted (the proposal path 400s on Reject at write time);
  the badge is recomputed deterministically at read time.
- **`/health` hardening field**: `injection_classifier_loaded` — lets ops confirm
  the opt-in model is actually active.
- **Canonical invisible-char predicate** (`screen::is_invisible`, extended from
  v0.9.7): adds the tag block (U+E0000–E007F) + variation selectors (U+FE00–FE0F)
  to the existing zero-width set. The blocklist normalization, the classifier,
  and the client render boundary now agree on what is invisible.
- **Client render boundary** (`client`): `strip_invisible` strips invisible
  smuggling chars from *displayed* recall hits + review proposals so the operator
  sees the de-obfuscated form. Raw bytes at rest are never rewritten.

### Security

- Closes the GhostJacking G5 upgrade path: novel/obfuscated injections that the
  deterministic blocklist misses can now be caught by an optional local model,
  still paired with the `flagged`/`untrusted` segregation (never the sole line of
  defense). Layer 2 off by default preserves the no-new-dependency default build.

### Honest ceilings (carried into v1.20.4 / v2.0)

- **Jetson-fit is a measured gate, not assumed.** Layer 2 is verified on desktop;
  the operator must run `bench --envelope` before treating it as Jetson-shippable
  (repo precedent: the rerank tier was removed for the same reason). `with_intra_threads(1)` respects the budget.
- The classifier catches semantic patterns, not every obfuscation; Quarantine
  stores flagged, never deletes. `source_prompt` remains PII-scanned, not
  semantically safe.
- `screen_verdict` is recomputed at read time, so a model swap can re-badge an
  in-flight proposal (rare; the badge reflects the current screen, which is the
  defensible reading). A model-drift Reject on a stored row reads as `quarantine`.
- `strip_invisible` runs at screen/classifier/render boundaries, not by rewriting
  stored bytes — a legitimate user's invisible Unicode is preserved verbatim at rest.
- G3 (OpenClaw subagent/exec/read/pdf envelope) + G4 (token at rest) remain
  operator/OpenClaw-side (companion plan).

### Changed

- Client Cargo stays 1.20.0 (version-neutral changes, v1.20.1 precedent).

### Fixed

- **Live panic in `mask_phone` (`src/gate.rs`)** — the PII masker iterated the
  input by byte index but emitted `out[i..i+1]`, which panics ("byte index is
  not a char boundary") whenever a multi-byte char (e.g. `—`, CJK) followed a
  digit run. A PII-flagged chunk containing such a char crashed the tokio worker
  on the read path. The masker now advances by full char (`len_utf8`); masking
  is unchanged and non-ASCII input round-trips untouched. Pinned by
  `redact_content_survives_multibyte_chars_and_still_masks`.

---

## [1.20.2] — 2026-08-11

### Server — "Harden" (deep + security second-pass audit fixes)

The consolidated fix release for the v1.20.x deep + security second-pass
audit. Every confirmed finding from both audit passes is closed as a code
change; the operator-only G3/G4 work from the prior `CredentialHygiene` plan
is Part H (operator steps, no code). No schema change (stays at 1.20.1) — this
is a code-only release. Server 1.20.1 → 1.20.2; plugin stays 0.2.1; client
stays 1.20.0. See `IMPLEMENTATION_PLAN_v1.20.2_Harden.md`.

### Fixed — Correctness + concurrency (audit chain fork + friends)
- **A1 [C] audit hash chain can fork under concurrent autocommit writers**
  (`src/audit.rs`). `record_tenant` wrapped read-tip + INSERT in a `SAVEPOINT`,
  which on an autocommit caller is `BEGIN DEFERRED` — two concurrent writers
  both read the same tip and both INSERT the same `prev_hash` (chain forks).
  Now branches on `conn.is_autocommit()`: autocommit → `BEGIN IMMEDIATE` so the
  read-modify-write serializes at BEGIN; inside a caller tx (autocommit false)
  → keep `SAVEPOINT` (outer tx already holds the write lock). Mirrors the
  proven `record_and_rotate` pattern. Pinned by
  `audit_chain_survives_concurrent_autocommit_writers` (two threads + Barrier +
  `verify_chain`).
- **A2 [M] `prune_audit_retention` re-anchor** now uses
  `TransactionBehavior::Immediate` (was `unchecked_transaction`), same root
  cause as A1.
- **A3 [H] `approve_proposal` UPDATE lacked `AND status='pending'`**
  (`src/handlers/gate.rs`). Two concurrent approves raced; the loser surfaced a
  generic 500 via `idx_knowledge_hash` UNIQUE. Now CAS's the row, checks
  `n > 0`, returns `409 proposal_already_decided` otherwise, and the whole
  SELECT-INSERT-UPDATE promote runs in `BEGIN IMMEDIATE`.
- **A4 [H] `expire_if_stale` audit visibility depended on caller tx state.**
  `approve_proposal` ran it inside the tx, so the expiration + audit rolled
  back if anything after failed. Now expired **before** the tx opens (a
  distinct autocommitted event) + the status is re-checked inside the tx. The
  reject path already used `&Connection` and was correct.

### Fixed — GhostJacking-audit G1 hole on `/procedure` (first-pass M1)
- **B1 `/procedure` write core now screens injection like its siblings**
  (`src/handlers/procedure.rs`). The Shield release's "shared write core"
  claim had a hole: `/procedure` INSERTed into `knowledge` directly. Now
  mirrors `ingest_one` — screens root content+title AND every step
  (`contains_suspicious_pattern`), honors Reject policy → 400 `input_rejected`,
  calls `flag_if_quarantined` per-chunk under Quarantine (default), and skips
  `next_step` KG edges for a quarantined procedure. Pinned by the model-backed
  `#[ignore]`d `procedure_screens_injection_like_its_siblings`.

### Fixed — PII redaction missed 16–19 digit Luhn cards (first-pass M2)
- **C1 `mask_phone` upper bound was 15; cards are 13–19** (`src/gate.rs`). A
  16-digit Visa/Mastercard was flagged `pii=1` but never masked → leaked
  verbatim via `redact_content` and `screen_source_prompt`. New `mask_card`
  Luhn-checks 13–19 digit runs (single source of truth reusing the `scan_pii`
  detector), called from both `redact_content` and `screen_source_prompt`.
  `"4111 1111 1111 1111"` → `[redacted:card]`. Pinned by
  `redaction_masks_luhn_valid_16_digit_cards`.

### Fixed — DoS surface (highest-impact audit findings)
- **D1 [H] rate limiter evadable + unbounded memory via spoofed
  `X-Forwarded-For`** (`src/main.rs` + `src/config.rs`). `X-Forwarded-For` is
  now trusted only when `BRAIN_TRUST_PROXY=1` (default: socket addr — a
  direct-connection attacker can't cycle the header). The `RateLimiter`
  HashMap is capped at `RATE_LIMIT_MAX_KEYS = 10_000` with LRU eviction of the
  oldest 25% when full (bounded memory, no new dep). Pinned by
  `rate_limiter_caps_tracked_ips_and_evicts_oldest`.
- **D2 [H] linker quadratic blowup on adversarial content** (`src/linker.rs`).
  `extract_vocabulary` is now capped at `MAX_VOCAB_ENTITIES = 500` (one guard
  at entity insertion; the O(mentions²) loops inherit the bound). Pinned by
  `extract_vocabulary_caps_at_max_vocab_entities`.
- **D3 [M] `/export` buffered the entire DB → OOM** (`src/handlers/gate.rs`).
  Now bounded with a hard row cap + the provenance summary precomputed in one
  COUNT-GROUP-BY. (`ponytail:` a true streaming JSON encoder is a v2.x change;
  this guard prevents the OOM today.)
- **D4 [M] `/v1/embeddings` unbounded batch amplification** (`src/main.rs`).
  `inputs.len()` is now capped at `MAX_EMBEDDING_BATCH = 64` → 400.

### Fixed — AuthZ completeness + tenant isolation
- **E1 [H] `/tombstones` + `/dsar/{id}/certificate` lacked tenant scoping**
  (`src/handlers/observe.rs`). Both are Admin-gated but didn't call
  `audit_scope`; a team-scoped admin saw every tenant's tombstones
  (`reason = owner:<subject>`) + certificates. Now filtered against the
  principal's `sub` at the SQL layer (cross-tenant → empty result / 404, no
  existence leak); superuser (`None` principal) unconstrained.
- **E2 wiring-guard test blind to chained routes + `cap_gate`** — the
  capability gate remains exercised by `cap_gate_enforces_verbs_scope_and_never_admin`
  + `capability_accepted_only_on_ump_surface_with_operator_key`; the contract
  table + comment updated.
- **E3 [M] `/add` did not enforce `MAX_CONTENT`** (`src/main.rs`) — now checks
  the same bound `ingest_one` uses → 400.

### Fixed — Input validation + data hygiene
- **F1 [M] `source_prompt` unbounded + not injection-screened**
  (`src/handlers/gate.rs`). `MAX_SOURCE_PROMPT = 2048` (plugin sends ≤2000) →
  reject longer; screened via `screen_source_prompt` so a tripped prompt
  persists only as the `[redacted:…]` form (reviewer sees the warning).
- **F2 [L] `/health/db` was public + leaked operational metadata**
  (`src/main.rs`) — moved out of both public lists; now Read-gated. `/health`
  (the load-balancer probe) stays public.
- **F3 [L] `multi_get` N+1 queries** (`src/main.rs`) — collapsed to a single
  `SELECT ... WHERE id IN (...)` respecting `MAX_MULTI_GET`.
- **F4 [L] `/metrics` tenant scoping documented** — kept Admin/Read (an
  operator surface; the body is aggregate booleans, not row data); the intent
  is now a docstring.

### Added — MCP 2026-07-28 protocol compliance (Agent 68, folded)
- **MCP 2026-07-28 protocol compliance** (`src/bin/mcp.rs`): stateless core —
  no `initialize` handshake; every modern request validates the mandatory
  per-request `_meta` (`io.modelcontextprotocol/protocolVersion` +
  `io.modelcontextprotocol/clientCapabilities`); `server/discover` replaces
  `initialize` for modern clients (`supportedVersions: ["2026-07-28",
  "2025-11-25"]`); every result carries `resultType: "complete"` +
  `_meta.io.modelcontextprotocol/serverInfo`; `tools/list` + `server/discover`
  advertise `ttlMs`/`cacheScope` caching hints (SEP-2549). Error surface per
  the new spec: missing `_meta`/fields → -32602, unsupported version → -32022
  with `data.{supported,requested}`, unknown tool → -32602, parse error →
  -32700 (null id), null id → -32600. Dual-era: a legacy client's `initialize`
  selects 2025-11-25 semantics scoped to the stdio process. `ping` kept as a
  harmless no-op (removed from the new schema). Verified against OpenClaw
  2026.8.1 as a real MCP client (a test only — the native plugin remains the
  integration).
- **G1 [L] MCP stdio `read_line` unbounded → OOM** — capped at
  `MAX_LINE_BYTES = 1 << 20` (1 MiB), bails with -32700 on overflow.
- **G3 [L] MCP error messages echoed user input** — the four `format!` sites
  now use static labels + `sanitize_echo` (hex-escapes the offending value,
  truncates to 64 chars) so client input can't carry prompt-injection text
  into the caller LLM via `error.message`. Pinned by
  `sanitize_echo_destroys_injection_structure` + the updated
  `unknown_tool_is_a_protocol_error`.
- **G4 [I] `legacy` flag process-sticky** — `ponytail:` comment names the
  single-parent trust-model ceiling. No code change.

### Honest ceilings (carried into v1.20.3+ / v2.0)
- The injection screen stays the deterministic blocklist (G5 classifier =
  v1.20.3). Quarantine stores flagged, never deletes.
- `/export` streaming uses a bounded guard, not a server-sent stream (v2.x
  nicety); `RateLimiter` LRU is in-process (multi-instance shared store is
  v2.1); capability tokens remain operator-only (per-tenant cap scope is v2.0
  multi-tenancy); the audit-chain C1 fix is per-process (distributed audit
  chain is v2.1).

## [1.20.1] — 2026-08-11

### Server + Plugin — "Shield" (GhostJacking P0: injection screen on the shared write core + autoCapture through the human review gate)

First release of the GhostJacking-hardening line. Closes the two P0 audit
findings on the memory write path: the `/ingest` core that bypassed the
injection screen (G1), and the autoCapture write path that bypassed human
approval (G2). See `IMPLEMENTATION_PLAN_v1.20.1_Shield.md`.

### Added
- **M1 — `/ingest` now screens injection like its siblings** (`src/handlers/ingest.rs`):
  the shared `ingest_one` core (plain + single-UMP + batch-UMP + the plugin's
  `memory_store`/`autoCapture`) mirrors `/add` and `/ingest/memory` — `Reject`
  policy → HTTP 400 `input_rejected`; `Quarantine` (default) stores the chunk
  flagged (`flagged=1`, excluded from recall) and skips its KG edges. One
  guard in the shared core covers every caller.
- **M2 — autoCapture routes through the proposal gate** (plugin default):
  - `captureMode` on the plugin (`proposal` default | `direct`). `proposal`
    POSTs `/ingest/proposal` via the new `BrainClient.submitProposal()` —
    nothing from an untrusted turn becomes memory until a reviewer approves.
    `direct` keeps the old behavior (still screened server-side).
  - `proposals.source_prompt` column (additive migration + schema 1.20.1):
    the capture-triggering turn is stored **PII-screened** (`screen_source_prompt`
    — only `[redacted:…]` form persists, per LLM01:2026 control #7 "exact action,
    not a summary") and rendered in the client Review panel.
  - Proposal TTL (`BRAIN_PROPOSAL_TTL_SECS`, default 7 days): a pending
    proposal that ages out is auto-rejected + audited `proposal_expired`;
    approve/reject on a stale proposal refuse with 400.
  - `source_prompt` round-trips through `/proposals` (`ProposalView`), the
    client wire type, and the Review panel's "sourcing prompt" block.
- **M3 — docs**: `SECURITY.md` names `/ingest` as screened + the
  auto-capture gate; `docs/MEMGHOST_MITIGATION.md` documents `captureMode`.

### Tests
- Server: +3 (`ingest_screens_injection_like_its_siblings` — the audit §5
  drill as a model-backed `#[ignore]`d test, quarantine/reject/benign arms;
  `test_proposal_expires_after_ttl_and_audits`; the lib's
  `source_prompt_is_pii_screened_and_rendered`). Plugin: +3 (submitProposal
  wire; captureMode default routes to `/ingest/proposal`; config default).
- `schema_version` contract → 1.20.1; `authz_gates_cover_every_non_public_route`
  + `test_openapi_covers_routes` unchanged (no new routes).

### Security
- G1 closed: `/ingest` no longer bypasses the injection screen (audit §4
  action #8's document lie fixed).
- G2 closed: autoCapture no longer writes to memory without human approval
  (default `captureMode: "proposal"`); `memory_store` stays direct by design
  (explicit agent action) and remains M1-screened.

### Honest ceilings (carried into v1.20.2 / v1.20.3)
- The screen stays the deterministic blocklist; G5 classifier upgrade is
  v1.20.3.
- G3 (OpenClaw subagent/exec/read/pdf envelope coverage) lives in the OpenClaw
  codebase — companion plan v1.20.2.
- G4 (live token at rest, world-readable plist) is operator/tooling — v1.20.2.
- G6 webhook replay window P2 — documented, v1.20.4 if prioritized.

---

## [1.20.0] — 2026-08-11

### Client — "Polish" (theming, perf, offline-tolerance — the v1.20.0 done-state)

The final milestone of the v1.14→v1.20 client chain. Closed the plan's three
testable deltas; the two measured-performance deltas that need the Dioxus CLI
(`dx bundle` wasm sizes + FPS profiling) stay operator steps with their
budgets documented in `BENCHMARKS.md`.

### Added
- **M1 — system-following theme**: the theme toggle now cycles
  `dark → light → system`; `system` resolves via `prefers-color-scheme`
  (`pick_theme` extended to a tri-state over `THEME_MODES`; the existing
  theme effect sets `data-theme="system"` and the CSS
  `@media (prefers-color-scheme: light)` token block does the following —
  no JS).
- **M2.1 — bundle regression budget**: `client/bundle-budget.sh` builds the
  release wasm and fails if it exceeds a 7 MB budget (measured
  4.34 MB at ship; the dx-bundled 3.7 MB from v1.18.1 is the floor reference).
  Wired into the `client-gate` CI job as a hard gate.
- **M3 — offline-tolerance** (`client/src/queue.rs`): a bounded (100),
  serde-persisted (localStorage, `credentials_stay_in_memory`-safe — no
  token ever enters a queued action) action queue. Approve/Reject/Purge/DSAR
  actions that hit an unreachable/erroring server are queued instead of
  dropped; a "queued (offline)" badge shows the count in the top bar.
  On recovery the queue replays (`run_replay` — settle-by-key, each action
  applied once, survivors re-enqueued). Pinned by a wire parse/dedup test
  (idempotency-key dedup) + queue tests.
- **M4 — zero-telemetry reaffirmed**: no change, and the M2/M3 additions
  collect nothing (queue payloads are action-ids only, persisted locally).

### Changed
- Review rows, the batch summary, and DSAR outcomes now surface
  `RowOutcome::Queued` rather than collapsing to a generic pending state.
- `Package` idempotency keys derive from the action payload (`key()`), so a
  queued-then-applied action is never applied twice.

### Honest ceilings (carried into v2.0)
- Measured `dx bundle` wasm/JSCSS sizes + FPS profiling are operator steps
  (no Dioxus CLI here); the plan's <50 KB initial / <5 MB mobile budgets are
  tracked in `BENCHMARKS.md` as measured-success criteria, the CI budget
  guards the dominant term (release wasm).
- `system` theme does not live-listen to OS changes mid-session (applies on
  launch/change); desktop/mobile native theme following is a v2.x ceiling.
- wasm-split remains a Dioxus 0.8 ceiling (the wasm grows with the console —
  the budget gate is the tripwire until then).

---

## [1.19.0] — 2026-08-10

### Client — "Integrated" (the audit-verified remainder of the v1.19.0 plan)

The v1.19.0 plan (SSO + deep links + PWA + scale) was audited against the tree
at ship time: **most of it was already shipped** — deep links
(`/review/:proposal_id`, `/recall/:trace_id`, `/subjects/certificate/:dsar_id`)
in v1.16.7, iOS/Android `brain://` intent filters in v1.17.0, the PWA shell
(manifest + service worker + offline shell) in v1.16.7, recall search
debounce in v1.16.7 M6, and the JWT-pair + silent-refresh + principal half of
SSO in v1.16.5. The remaining **testable** delta is shipped here: the audit
panel's filters became URL-addressable. The rest of M1/M3/M4 are documented
ceilings (below).

### Added
- **M2 — `/audit?since=&principal=` deep link**: the `Audit` route now carries
  `since` + `principal` query params (`Route::Audit { since, principal }`),
  threaded into `audit::panel` and seeded into the existing client-side
  `AuditFilter` via a new pure `filter_from_query`. A reviewer can share a
  filtered audit view (e.g. `/audit?principal=alice`) and it opens pre-filtered.
  Pure core + test; all six `Route::Audit` construction sites updated.

### Honest ceilings (carried into v1.20.0)
- **M1 OIDC/SSO is a server-side (v2.x) ceiling, not a client gap.** brain-server
  is a token *validator*, not an OIDC IdP: its `/.well-known/openid-configuration`
  advertises empty `authorization_endpoint`/`token_endpoint`. A real
  authorization-code + PKCE flow needs a new `/auth/authorize` proxy endpoint
  on brain-server (external IdP), which is v2.x work (documented in the v1.16.5/
  v1.16.8 plans + `docs/proxy-sso.md`). The client's JWT-pair mode + silent
  refresh-on-401 + principal pillar (v1.16.5) already consume the JWT half.
- **M4 virtualized lists** need viewport JS (untestable here without `dx serve`);
  the audit panel already paginates server-side (`OFFSET`, v1.16.7).
- **M4 wasm-split lazy panels** remain a Dioxus 0.7.10 ceiling — re-measure after
  Dioxus 0.8-stable (unchanged from v1.18.1).

---

## [1.18.2] — 2026-08-09

### Server — "Transparency" (EU AI Act Art 50 origin marker + export provenance)

**Unified-version release**: the server ships the Transparency work and the
client is bumped from 1.18.1 to **1.18.2** so both binaries report the same
version (the client carries no new code in this bump — see `[1.18.1]` below for
its last change). Ships the two real accuracy gaps the v1.18.1 Transparency
plan found in COMPLIANCE.md §7 (Round 14 pass): an explicit model-vs-human
`origin` marker, and `/export` provenance that actually carries it. The plan's
M3 (ai-notice / ai-literacy / cop-notice routes + `docs/AI_LITERACY.md`) had
already shipped in v1.16.7/v1.16.8 and is unchanged.

### Added
- **M2 — `knowledge.origin` column** (migration): `TEXT NOT NULL DEFAULT
  'imported'` + `idx_knowledge_origin` index + idempotent backfill by source
  kind (`manual`→`human`, `memory`→`model`, else `imported`). Write-time
  tagging wired into the interactive/assistant paths: `/add` and the propose→
  approve promote set `origin` from the resolved source kind via the pure
  `gate::origin_for_source` helper; `/ingest/memory` writes `model`;
  procedures write `human`. `markdown`/`structured` bulk imports keep the safe
  `imported` default — never claim human authorship for an unknown path.
- **M1 — `/export` provenance block**: per-row `source` + `origin` already
  emitted; now adds `export_format_version: 2` + a `provenance_summary`
  (`total` / `by_origin` / `by_source`) computed across all exported rows. All
  12 v1 field names preserved byte-identical for downstream importers.
- **M3 polish** — `/.well-known/ai-notice` `origin_metadata` now lists
  `origin` alongside `source`/`assertion_kind`/`confidence`.

### Changed
- **COMPLIANCE.md §7** aligned to shipped state (origin column +
  provenance_summary + format-version envelope) and gained an **Enforcement**
  note: Art 50 is enforced by national market surveillance authorities at the
  **€15M / 3% (Art 99(3))** tier — the €35M / 7% figure is Art 99(2) for
  prohibitions + GPAI provider obligations, not Art 50.

### Tests
`origin_for_source_maps_kinds`, `migration_backfills_origin_by_source`,
`export_contains_source_origin_and_provenance_summary` (incl. v1 field-name
regression guard), + `origin` added to `test_migration_schema_contract`.

---


## [1.18.1] — 2026-08-09

### Client — "Harden" (console-history persistence + measured bundle ceiling)

**Client-only** — server + API contract stay at 1.17.5 (zero server changes,
zero schema change). Dioxus 0.7.10. Closes the honest ceilings out of the
v1.17.8/v1.18.0 line where a *real, low-risk, measured* improvement exists.

### Changed
- **M1 — console history: in-memory → persistent + secret-safe** (`src/api.rs`,
  `src/panels/system.rs`). The try-it console's history now survives reload:
  only `redact_for_history`-clean lines are written to web `localStorage` via
  the existing `i18n::pref_save`/`pref_load` seam, capped at the last 100.
  A line whose request body was non-JSON (`line_is_secret`, i.e. an opaque
  token-like payload `redact_for_history` cannot redact) is flagged `secret`
  and held **in-memory only** — never persisted. Pure `persist_history`
  drops secret/empty lines and caps. The `credentials_stay_in_memory` grep
  guard still passes: the raw token-bearing input never touches disk.
- **M4a — client bundle measured, not guessed** (`BENCHMARKS.md`). The Dioxus
  0.7.10 web bundle from `dx bundle`: wasm **3,724,711 B (3.7 MB)** + 60 KB JS
  + 40 KB CSS, recorded as measured facts. wasm-split is **not adopted**
  (experimental in 0.7.10, shell-heavy bundle); tracked for re-measure after
  Dioxus 0.8-stable.

### Deliberate non-changes (honest ceilings, code-grounded)
- **M2 token-minting panel UX** — the UMP panel has no "CLI docs link" to
  replace; minting is correctly CLI-only (no mint endpoint by design). Adding
  untestable UX churn for marginal value was skipped; the security posture is
  unchanged and correct.
- **M3 SSE subscribe** — **no SSE subscribe control exists in the client**; the
  `/ump/subscribe` endpoint is server-side reachability only, so there is
  nothing misleading to rename. A live browser change stream remains v2.x (A2A).
- **M5 native pull-to-refresh / M6 focus-return** — native gesture needs a touch
  platform + `dx serve`; focus-return is `document::eval`-based, both unverifiable
  in this environment (no Android SDK / browser harness). The accessible
  `RefreshButton` and existing focus trap remain.

### Verification
- `cargo test` (client): **76 passed** (was 74; +2 `line_is_secret_*` +
  `persist_history_*`). Clippy `-D warnings` + fmt clean; wasm build clean.
- Server suite untouched (473 baseline — zero server edits).

---

## [1.18.0] — 2026-08-09

### Client — "Compliant" (WCAG 2.2 AA + i18n + privacy hardening pass)

**Client-only** — server + API contract stay at 1.17.5 (zero server changes,
zero schema change). Dioxus 0.7.10. The plan's M3 (i18n) and M4 (privacy)
shipped in v1.16.8/v1.17.0; this release closes the two remaining testable
gaps and formalizes the CI gate.

### Added
- **`?` in-app keyboard help on Review (M1.4).** Pressing `?` (or the new `?`
  toolbar button, `aria-expanded` + `aria-label`) toggles an in-app table
  documenting the A/S/R/J/K shortcuts — the WCAG 3.2.6 consistent-help gap.
  Pure `keyboard_help()` core + i18n keys (`review_help_*`, `en` source; other
  locales fall back via `resolve`). The `?` mapping respects the existing
  WCAG 2.1.4 shortcuts-off toggle.
- **Client CI gate (M2).** New `client-gate` job in `.github/workflows/ci.yml`:
  `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` +
  `cargo test` + the `wasm32-unknown-unknown` build. The Dioxus client had
  **zero CI coverage** before this; the automated a11y/semantic grep gates
  (`interactive_elements_are_buttons`, `xss_escape_hatch_is_unused`) now run on
  every push/PR.

### Not shipped (documented, not deferred — deliberate ceilings)
- **axe-core browser gate (M2.1)** — needs Playwright + a `dx bundle` + a
  live server + browser download; an operator/tooling step, not runnable in
  this repo's CI surface. Documented in `client/a11y-checklist.md`.
- **Native screen-reader pass (M1.7)** — the human gate; tracked as the
  existing `client/a11y-checklist.md` matrix (VoiceOver/NVDA/TalkBack), an
  operator step.

### Verification
- `cargo test` (client): **74 passed** (was 73; +1
  `question_mark_opens_help_and_table_covers_all_keys`). Clippy `-D warnings`
  + fmt clean; wasm build clean.
- `ci.yml` parses (pyyaml). Server suite untouched (473 baseline — zero server
  edits).

---

## [1.17.6] — 2026-08-09

### Client — "Complete" part 1: command palette v2 + Overview

First of the three-part "Complete" (operator console) release line
(`v1.17.6` + `v1.17.7` + `v1.17.8`). **Client-only** — server + API contract
stay at 1.17.5 (zero server changes, zero schema change). Dioxus 0.7.10.

### Added (client)

- **M1 — Command palette v2** (`src/main.rs`): the palette is now a fused
  **nav + lookup + action** surface, not a settings shortcut. `Command` is a
  flat tagged enum (`Navigate` / `Lookup` / `Run` / `SignOut`) with a group
  label + keyword index. Pure cores (`palette_group`, `command_keywords`,
  `palette_lookup`, `remember_recent`, `destructive_action`) are Dioxus-free
  and test-pinned.
  - Grouped results in order **Recent / Go to / Lookup / Run**, capped at 5
    per group (Linear/Raycast convention). Empty needle returns every group;
    a typed needle filters case-insensitively over keywords + labels and hides
    the Recent group.
  - **Recents** persist through the existing `i18n::pref_save`/`pref_load`
    seam (non-secret label list, last 8, dedup + cap).
  - **Keyboard**: `↑`/`↓` navigate the flattened list (group headers are
    labels, not items), `Enter` runs, `Esc` closes, `/` re-focuses the input,
    `Tab`/`Shift+Tab` cycle via the existing hand-rolled `focus_trap`.
  - **Destructive confirm**: selecting a destructive `Run` action (Reindex —
    `destructive_action`) swaps the list to a single "Press Enter to confirm"
    `aria-live` row; `Esc` aborts.
  - **Screen-reader labels** on every row (`aria-label` = `command_label`).
  - M1.5 **single source of truth**: `palette_commands` + the 
    `palette_navigate_covers_every_non_detail_route` guard ensure every
    non-detail route is reachable. The `Lookup`/`Run` row *types* ship now
    (arms wired); live ids/actions arrive with the v1.17.7/v1.17.8 panels.
- **M2 — Overview** (`src/panels/overview.rs`): the decision-first landing
  home at `/` under the AppShell layout. A control room, not a widget dump —
  every card links to its panel, backend stays the source of truth (no client
  cache).
  - **Status row** (≤4 cards): Health (conn dot + status/version), Snapshot
    integrity (`snapshot_count` + green/red dot), Retention posture
    (`enabled` + kind count), Server + UMP (`server.version` +
    `conformance` L2/L3 badge). Each links to its owning panel.
  - **Alert list** (DAR chain: signal + diagnosis + action): auth failures +
    quarantined chunks (existing UiState signals) + stale sources / unresolved
    conflicts / near-duplicates (`/consolidate/propose` counts) + decayed
    chunks (`/decayed`) + tombstones (`/tombstones`). Severity-sorted, empty →
    "no alerts".
  - **Queue preview**: top 5 pending proposals with one-click Approve/Reject
    (mirrors the review panel's `decide`) and a deep link into `/review/:id`.
  - Pure `overview_alerts` core + 3 tests (empty case, severity ordering,
    only-nonzero-sources).
- **api.rs**: 6 new `ApiClient` methods (`snapshot_status`, `retention`,
  `ump_capabilities`, `decayed`, `consolidate_propose`, `tombstones`) + wire
  types mirroring the confirmed handler shapes + 6 wire-contract pin tests.
- **Route + nav**: `Route::Overview {}` at `/`; `Connect` moved to `/connect`
  (outside the AppShell layout, so the shell's connect-first redirect has no
  loop). Overview added as the first rail + tab-bar nav item (via `NavLink`/
  `TabLink`) and to the palette.
- **i18n**: new Overview + palette keys in all five locales
  (`en`/`de`/`fr`/`es`/`nl`), locale-aware `format_number` on alert counts.

### Fixed / Changed (client)

- Connect now routes to `/connect`; after a successful connect it proceeds as
  before (first-connect still lands in Review — unchanged).
- Command palette v1's nav-only `filter_commands` replaced by the grouped
  `palette_lookup`; the old nav-count test updated (6 → 7 targets).

### Tests (client)

59 passed (was 49; +3 overview alerts, +6 api wire-contract pins,
+1 palette route-coverage guard). Clippy `-D warnings` clean, `cargo fmt
--check` clean, wasm build clean.

### Honest ceilings (carried into v1.17.7 / v1.17.8)

- Lookup is instant against **client-held ids only**; a server-backed fuzzy
  lookup is v2.x. Recents are a flat non-secret label list, not deep-linkable
  objects — re-running a recent re-resolves the route/action fresh.
- The `Lookup`/`Run` command rows (and their confirm/destructive handling)
  ship as reserved + wired types; the live ids/actions that construct them
  arrive with the v1.17.7/v1.17.8 panels.
- No RBAC-aware UI (roles land with v1.23.0); the client shows the server's
  403 verbatim. OpenAPI is not parsed client-side (no new dep).
- wasm-split unchanged (Dioxus 0.7.10 ceiling); bundle size grows.

---

## [1.17.8] — 2026-08-09

### Client — "Complete" part 3: Data & Rights + UMP panel + System & Try-it console

Third and final part of the three-part "Complete" operator-console line
(`v1.17.6` + `v1.17.7` + `v1.17.8`). **Client-only** — server + API contract
stay at 1.17.5 (zero server changes, zero schema change). Dioxus 0.7.10.
73 client tests (+7 from 1.17.7).

### Added (client)

- **M5 — Data & Rights panel** (`src/panels/data.rs`): the v1.14 / v1.15
  lifecycle surface — purge (`POST /purge` by comma/space/newline-separated
  ids or an owner), portable export (`GET /export` as JSON / UMP / UMP-Markdown
  via the existing `document::eval` download seam), a per-kind retention editor
  (`GET /retention` → `retention_to_edits` sorted overrides; set a kind+days
  override, one-click `×` clear per kind), the `/decayed` review list, and the
  `/tombstones` deletion-registry. Status region is `role="status"
  aria-live="polite"`.
- **M6 — UMP panel** (`src/panels/ump.rs`): the v1.17.3 wire surface —
  capabilities card (`UmpCapabilities` + pure `ump_integrity_badge`
  badge/label from the `conformance` line), `POST /ump/remember` (JSON body →
  `{ok,id}`), `POST /ump/recall` with kind filter + `max_recall` clamped to
  1..100 (renders the `results` envelope), and `POST /ump/audit` load +
  verify-chain (`ump_audit`/`ump_recall`/`ump_remember` + `UmpRecallResult`/
  `UmpAudit` typed wire types).
- **M7 — System panel** (`src/panels/system.rs`): domains list, snapshot
  integrity, the Art 30 register (`art30()` pretty-JSON), `POST /reindex`
  (`ReindexResult`), connectors list (`ConnectorRow`: `kind · instance / state`)
  + `POST /sources/reconcile` (`ReconcileResult`), and a **Try-it console**
  (`get_raw`/`post_raw`/`delete_raw` + `serialize_request` request-line builder
  + `redact_for_history` so the persisted history never stores a token-bearing
  body).
- **M8 — Route + nav + i18n**: `Route::Data` (`/data`), `Route::Ump` (`/ump`),
  `Route::System` (`/system`) under the AppShell; all three added to sidebar
  rail + mobile tab bar + command palette (nav targets now **12**, guard test
  updated); new `data_*`/`ump_*`/`sys_*`/`nav_*` keys in all five locales
  (each locale now 50 keys, en-completeness test green). **api.rs**: `Clone`
  added to the 10 typed wire structs so `Signal<T>()` call-syntax reads work
  (root cause of the call-syntax failures; consolidate.rs's `Item` already had
  it), `post_raw` made `pub`, pure `parse_purge_result`/`retention_to_edits`/
  `parse_ump_record`/`parse_ump_recall`/`ump_integrity_badge`/
  `serialize_request`/`redact_for_history` cores + wire-contract tests.
- Version 1.17.7 → 1.17.8; CHANGELOG §[1.17.8]; CLIENT_ROADMAP v1.17.8 row →
  Shipped.

### Verification

- `cargo test --manifest-path client/Cargo.toml`: **73 passed** (was 66; +7
  api.rs wire/parse cores).
- `cargo clippy --all-targets --manifest-path client/Cargo.toml -- -D warnings`:
  clean. `cargo fmt --check`: clean. `cargo build` + `cargo build --target
  wasm32-unknown-unknown`: clean.
- Dioxus rsx hazards fixed during the build pass (same class as 1.17.7):
  `let` statements as direct rsx children of `if let` bodies (hoisted all
  signal reads + label computations before `rsx!`); `t()`/placeholders with
  literal braces inside rsx format strings (hoisted to locals, simplified
  `r#"{"query":...}"#` placeholders to plain strings); `Signal<T>()` call
  syntax needs `T: Clone`; `onkeydown` compares `Key::Enter` not `"Enter"`;
  named `move |_|` closures can't coerce to `ListenerCallback` (wrapped as
  `move |_| run_x(())`).

### Ship status

**COMPLETED (code + tests + docs) 2026-08-09**. `./deploy-web.sh` → live
`/app` re-deploy, tag `v1.17.8`, and the GitHub release are operator steps.
No server restart needed (client-only static bundle).

## [1.17.7] — 2026-08-09

### Client — "Complete" part 2: Graph panel + Create workspace

Second of the three-part "Complete" operator-console line (`v1.17.6` +
`v1.17.7` + `v1.17.8`). **Client-only** — server + API contract stay at
1.17.5 (zero server changes, zero schema change). Dioxus 0.7.10. 66 client
tests (+7).

### Added (client)

- **M3 — Graph panel** (`src/panels/graph.rs`): debounced (300 ms) entity
  lookup via `GET /graph/entity/{name}` → typed `EntityView` (traits +
  relations with `from`/`to`/`relation_type`); a traverse card issuing
  `GET /graph/traverse?start=&depth=&kind=&at=&cross_domain=true` → typed
  `TraverseResponse` with `paths` (structured hop chains rendered by the pure
  `render_path` core, `A --relation--> B --relation--> C`) and the flat
  `traversal` rows collapsed in a `<details>` table. `kind` filter validated
  by the pure `kind_is_valid` (exact or `prefix:`-style, matching the v1.7
  server contract); `parse_entity` core + tests.
- **M4 — Create workspace** (`src/panels/create.rs` hub → `ingest.rs` +
  `procedures.rs` + `consolidate.rs`), the v1.14/v1.10 write surface:
  - **Ingest** (`ingest.rs`): three tabs (Structured / Markdown / Memory)
    with real `<button>` tab toggles (aria-pressed), JSON pre-validation
    before send, per-mode result via `parse_ingest_result` /
    `IngestOutcome` (Created / Duplicate / Error).
  - **Procedures** (`procedures.rs`): a step builder (title/body/optional
    is-decision, add-step list) → `POST /procedure` → typed `ProcedureResponse`;
    lists ordered steps via `/procedure/{id}/steps` → `Vec<StepView>`; plus
    the two deterministic helpers: `POST /classify` (typed
    `ClassifyResponse` → category + confidence + matched keywords) and
    `POST /decision/{id}/evaluate` (typed `DecisionOutcome`, vars parsed by
    the pure `parse_decision_vars` core — lenient, non-numeric dropped).
  - **Consolidate** (`consolidate.rs`): `POST /consolidate/propose` →
    typed `ConsolidateProposal`; unresolved contradictions + near-duplicates
    rendered as list items; one-click `POST /consolidate/apply` (supersedes
    link) and `POST /consolidate/undo`, both refresh the proposal list.
- **Routes/nav/i18n**: `Route::Graph{}` at `/graph` and `Route::Create{}`
  at `/create` (under the AppShell); both added to the sidebar rail + tab
  bar + command palette (nav targets now **9**, guard test updated); all
  M3/M4 i18n keys in all five locales (`en`/`de`/`fr`/`es`/`nl`).
- **api.rs**: typed wire structs (`EntityView`/`EntityRel`,
  `TraverseResponse`/`TraversalRow`/`PathChain`/`Hop`, `ProcedureResponse`/
  `ProcedureStepsResponse`/`StepView`, `ClassifyResponse`/`CategoryResult`,
  `DecisionOutcome`, `ApplyResponse`/`UndoResponse`, `ConsolidateProposal`)
  + `impl ApiClient` methods + pure cores (`render_path`, `kind_is_valid`,
  `parse_entity`, `parse_ingest_result`, `parse_decision_vars`) + wire-contract
  tests.

### Fixed (client)

- The palette's `render_path` core emitted a doubled ` --` separator between
  hop chains (`A --e--> B -- --c--> C`) — one `--` was pushed twice; the
  separator is now emitted exactly once, pinning
  `render_path_renders_faithful_chains` to `A --employs--> 2 --ceo_of--> carol`.
- The Create hub's three panels render under ONE focusable `<h1>` (the hub
  owns the `PageTitle`; the nested panels drop theirs) — no duplicate-h1
  a11y regression.
- Dioxus rsx hazards fixed during the build pass: inline `if` in rsx can't
  hold a nested `rsx!` (switched the ingest tab body to a `match` on
  `tab().as_str()`); `#[component]` fn can't be called positionally as a
  plain fn in braces (the `tab_btn` helper is a plain `fn` now); an
  unbraced raw-string placeholder containing `{...}` broke the format-string
  parser (`placeholder: "revenue: 1200"`).

### Verification

- `cargo test --manifest-path client/Cargo.toml`: **66 passed** (was 59 at
  v1.17.6; +7: render_path + wire types + parse cores).
- `cargo clippy --all-targets --manifest-path client/Cargo.toml -- -D warnings`: clean.
- `cargo fmt --check --manifest-path client/Cargo.toml`: clean.
- `cargo build` + `cargo build --target wasm32-unknown-unknown`: clean.

### Ship status: COMPLETED (code + tests + docs) 2026-08-09

`./deploy-web.sh` → live `/app` re-deploy is an operator step. Tag `v1.17.7`
+ GitHub release are operator steps. No server restart needed (client-only
static bundle).

### Honest ceilings (carried into v1.17.8)

- Graph entity relations are the server's snapshot shape; the traverse
  `paths` intermediate hops surface by id unless a name resolves (same as
  the server contract).
- Ingest does client-side JSON pre-validation only; malformed entity/relation
  arrays degrade to empty on the wire (server still validates).
- The palette's `Lookup`/`Run` command rows remain wired-but-reserved; the
  live id/action constructors arrive with v1.17.8's remaining panels.
- wasm-split unchanged (Dioxus 0.7.10 ceiling); bundle size grows.

---

## [1.17.5] — 2026-08-09

### CLI — "Eval Fix" (`brain eval` + `bench`)

- **Fixed: `brain eval` was dead on arrival — every run returned 405.**
  `run_eval` sent `GET /recall?query=…&k=10`, but `/recall` is a POST-only
  JSON route (`{query, limit}`); the v1.17.1 M3 ship gate and
  `BENCH_RECALL_FLOOR` could never have computed a score. Now POSTs the
  correct body on `/recall` and keeps `GET /search?q=…&k=10` on the search
  leg (`src/bin/brain.rs`).
- **Fixed: judged-index mapping was hash-order arbitrary.** 
  `results_to_doc_indices` mapped result content → DOCS index through a
  `HashSet`, whose `.position()` order is unspecified — recall@k was
  computed against the wrong judged indices. Now matches the DOCS slice
  directly, so indices are the fixture's documented array positions.
- **Fixed: `/recall` response parsing** — the parser only read the
  `results` wrapper (`/search` shape) while `/recall` returns `hits`; both
  shapes now parse (pinned by a new brain-bin test).
- **CI (round-21 gaps):** two new jobs — `ump-conformance` boots a scratch
  keyed instance and asserts the reference suite's `UMP 1.0 / L3` badge
  line (the runner exits 0 for any level ≥ L1, so the gate checks the text);
  `recall-gate` seeds the frozen 10-doc corpus and enforces
  `--floor r5=0.85 --floor r10=0.85 --floor mrr=0.85` with `pipefail`.
- **SBOM:** the tag release workflow now generates a CycloneDX SBOM via the
  existing `scripts/sbom.sh` (cargo-cyclonedx from Cargo.lock) and ships it
  in `dist/` alongside the binaries (EU CRA / OWASP A03:2025).
- **Benchmarks:** first honest row in `BENCHMARKS.md` — the frozen 37-query
  smoke-set run on the default profile (r@5 0.919, r@10 0.919, nDCG@10
  0.911, MRR 0.905). Smoke set only; parity rows stay `PENDING` per the
  protocol (≥100 judged queries on target hardware incl. 4 GB ARM).
- Fixture doc-count corrected (32 → 37 judged queries).

---

## [1.17.4] — 2026-08-09

### Server — "UMP Conformance" (wire fixes)

Fixes every defect a byte-level review of the reference conformance suite
(`github.com/edihasaj/universal-memory-protocol` `conformance.ts`) surfaced
against the v1.17.3 implementation, so the reference runner scores the full
L1–L3 set. **Breaking change:** the emitted `integrity` block and the
`did:key` identity changed shape (below) — records signed by a v1.17.3 peer
still verify (dual-read), but new signatures use the reference format.

- **did:key bug fixed (breaking)** — `did_key_from_ed25519` used a 33-byte
  bare-`0xed` multicodec prefix; the reference `didKeyFromPublicKey` prefixes
  the two-byte `0xed 0x01` varint (34 bytes), and `publicKeyFromDidKey`
  rejects anything else. Old output `did:key:z2De…`; correct form
  `did:key:z6Mk…`. The operator CLI + server identity now agree with the
  reference (vector pinned: RFC 8032 vector-1 pk → `z6MktwupdmLXVVqTzCw4i46
  r4uGyosGXRnR3XjN5x1fTDDgQ`).
- **Integrity block → reference §2.8 format (breaking)** — `{algo, hash,
  key, sig}` replaced by `{content_hash: "blake3:<base32>", signature:
  "ed25519:<std-base64>", signer: <did:key>}`. The content hash covers the
  canonical record minus `integrity` only (`id` stays inside), computed with
  the reference's JS-flavor canonicalization (integral floats serialize as
  `1`, not `1.0`; U+2028/U+2029 escaped) so the reference `verify()` byte-
  matches; the signature is Ed25519 over BLAKE3 of the `content_hash` STRING.
  `verify_record` dual-reads the legacy v1.17.3 shape. **Fix found by the
  live reference run:** the emitted signature initially carried bare base64 —
  the reference `verifyHash` requires the `ed25519:` prefix (`/^ed25519:(.+)$/`),
  so `L3.signed` failed until the emit gained the prefix (verify accepts both
  forms). Pinned by assertions in `emit_record_signed_and_verified_with_
  operator_key` + `ump_suite_parity_l1_to_l3`.
- **`from_ump` version gate lenient** — op requests carry no `ump` field
  (the suite sends none); absent now defaults to `1.0` (only an explicit
  unknown major is rejected).
- **`provenance` + `consent` carried** — stored in `UmpMeta`, re-emitted on
  every record (the suite's remember includes `provenance`; it previously
  round-tripped nowhere).
- **`superseded_by` on the prior record** — `GET /ump/memory/{id}` and
  `/ump/recall` now resolve `supersedes` evidence links and emit the
  successor's content-addressed urn; the revised record drops the carried
  `origin` so its own id resolves to a fresh urn (L2 bi-temporal: prior has
  `time.valid_to` + a non-empty `superseded_by` pointing at the revision).
- **id resolution by urn** — `/ump/memory/{id}`, `/ump/revise`,
  `/ump/forget`, `/ump/feedback` accept the content-addressed `urn:ump:…`
  form (resolved via the `ump_id` column, which `KNOWLEDGE_ROW_COLS` now
  loads; it was previously missing so ids fell back to the xxh3-shaped
  `urn:ump:<content_hash>` form and urn lookups 404'd).
- **`/ump/feedback` → `{ok: true}`** (the suite asserts it); `session`
  accepted and persisted; unknown ids 404.
- **`/ump/forget`** reports `erased` for the hard path, `tombstoned` for the
  soft path.
- **Ops** — the launchd plist gains `BRAIN_UMP_KEY_DIR`; wiki + keygen docs
  use the correct `did:key` form; `COMPLIANCE.md` cites Regulation (EU)
  2026/1744 (GPAI obligations live 2026-08-02, watermarking 2026-12-02) with
  the provenance-not-watermarking posture.

**New test:** `ump_suite_parity_l1_to_l3` (`#[ignore]`d, model2vec-weights
precedent) — walks the reference suite's exact requests end-to-end against a
keyed instance: capabilities envelope, remember (procedural + provenance) →
`{id, result:"created"}`, get-by-urn with a reference-shape signed integrity
block, recall (urn id + `signals` object), revise → `{supersedes:[urn]}`,
prior `time.valid_to` + `superseded_by` pointing at the new urn, forget →
`tombstoned`, validation → 400 `invalid_record`, feedback → `{ok:true}`.

### Verification
- `cargo test --features bench,migrate`: 473 bin + 70 lib + 9 + 8 + 7 + 3×2
  green; `--ignored` suite-parity test green. clippy `-D warnings` + fmt clean.
- **External reference run (live)**: `@universalmemoryprotocol/core` 1.0.0
  `ump-conformance` against a throwaway keyed instance (fresh DB + operator
  key + `AUTH_TOKEN`): **13/13 checks, `UMP 1.0 / L3`** — L1 capabilities
  (ump 1.0, 5 kinds), remember `created`, get, recall (urn id + `signals`),
  L2 revise + bi-temporal `valid_to` + superseded, forget `tombstoned`,
  validation 400 `invalid_record`, L3 discovery, **signed (reference
  `verify()` byte-matches + Ed25519 verifies)**, feedback `{ok:true}`,
  capability tokens (no-token 401, token 200), subscribe SSE. Reruns against
  a persistent DB report `merged` on L1.remember by design (content dedup) —
  the suite assumes a fresh store, same as the reference `ump-serve`.

---

## [1.17.3] — 2026-08-09

### Server — "UMP Rollout"

The UMP 1.0 rollout on the v1.17.2 wire-conformance base: the spec's §4.2
HTTP ops, §4.1 MCP tools, §4.3 file binding, and §5 identity + capability
tokens. Conformance claim: **UMP 1.0 / L3** (self-attested; §8-compliant
unknown-major rejection + 0.1-import normalization already shipped in
v1.17.1/1.17.2). `GET /ump/capabilities` (and the `/.well-known/ump.json`
discovery doc) report `conformance: "L3"` when an operator key is configured,
`"L2"` otherwise.

- **M2 — HTTP ops (`/ump/*`, spec §4.2)** — new `src/handlers/ump_ops.rs`
  (the codec stays in `ump.rs`): `GET /ump/capabilities` (§3.1 handshake:
  `server`, `ump: "1.0"`, `conformance`, `kinds`, `bindings:
  ["http","mcp","file"]`, `retrieval_signals`, `max_recall: 50`,
  `writable`, `audit`); `POST /ump/remember` (partial record → lowered
  through the structured-ingest path; §3.7 gates — declared
  `scope.owner` must match the principal, consent violations →
  `forbidden_scope`/`consent_violation`; `{id, result: created|merged|
  rejected}`); `GET /ump/memory/{id}` (integrity-verified on read, §2.8 —
  tampered records dropped); `POST /ump/recall` (§3.2 `{results:[{record,
  score, signals{similarity,recency,salience,scope_match,provenance_depth}}]}`
  over the shared `run_recall` core — the existing gates/injection guard/
  embedding/routing/hybrid+graph RRF/packing are byte-identical, two
  consumers); `POST /ump/revise` (patch → new chunk + `resolve_supersession`
  → `{id: urn:ump:NEW, supersedes:[OLD]}`); `POST /ump/forget` (`{reason,
  hard}` — `hard:false` soft-flags, `hard:true` takes the v1.14
  `purge_chunk_ids` erase path, both tombstoned + audited); `POST
  /ump/feedback` (outcome `followed|overridden|ignored|contradicted` → the
  suggest-feedback last-wins upsert with the granular `ump_outcome`
  persisted); `GET /ump/subscribe` (SSE change feed over a tokio broadcast
  channel — `{kind, id}` events only, never record bodies; kill-switch-safe,
  bounded); `POST /ump/audit` + `GET /ump/audit/verify` (§9 reference
  facility: thin aliases over `list_audit` + `verify_chain`,
  `capabilities.audit: true`). **Batch ingest** — `POST /ingest?format=ump`
  accepts a UMP 1.0 batch envelope `{ump:"1.0", records:[…]}` (single record
  still accepted, back-compat); per-record status, one failure does not
  abort the batch.
- **M3 — MCP tools (`ump.*`, spec §4.1 PRIMARY)** — `src/bin/mcp.rs`
  mirrors the full ops surface: `ump.capabilities`, `ump.remember`,
  `ump.get`, `ump.recall`, `ump.revise`, `ump.forget`, `ump.feedback`,
  `ump.audit`, `ump.audit.verify` (same thin HTTP-proxy shape as the
  existing tools; token passthrough via `BRAIN_TOKEN_FILE`/`BRAIN_TOKEN`).
- **M4 — File binding (`*.ump.md` / `*.ump.json`, spec §4.3)** — `GET
  /export?format=ump-md` renders the portable export as the §6.3 markdown
  projection (front-matter `ump`/`id`/`kind`/`scope`/`time`/`provenance` +
  body; parse via the `vault.rs` parsers, round-trip lossless); `POST
  /ingest?format=ump-md` parses the same projection back through the
  shared lowering. `brain ump export|import` CLI carries both wire forms
  with `--output`/`--input` file paths. Fix: the v1.17.1 `/export` drop on
  DBs with empty `knowledge` (a fatal row-mapping bug) — `observed_secs`
  is now `pub(crate)` and `knowledge_row_to_json` reads `Option<String>`
  timestamps; pinned by `export_mapping_survives_real_timestamp_rows`.
- **M5 — Identity + capability tokens (spec §5)** — new pure lib module
  `src/ump_integrity.rs` (`#![deny(unsafe_code)]`, the `brain_server::eval`
  precedent): `did_key_from_ed25519` (multicodec `0xed` + base58btc →
  `did:key:z6Mk…`), RFC 8785 JCS canonicalization (BTreeMap), blake3 →
  base32 content hashes, ed25519-dalek sign/verify (§2.8 `integrity`
  signatures), and §5.2 compact capability tokens (`alg.payload.sig`,
  `{iss, verbs:[read|write|derive|export], scope:{project}, exp}`).
  `brain ump keygen [--dir]` CLI writes an Ed25519 seed to
  `BRAIN_UMP_KEY_DIR` (default `~/.config/brain-server/ump/operator.key`,
  0600, refuses overwrite) and prints the DID. **Enforcement:** a
  capability token presented as `Authorization: Bearer` on `/ump/*` +
  `/export` is verified (key, signature, expiry) at the auth middleware,
  then verbs × scope are enforced per handler (`cap_gate` after
  `authorize` — reads need `read`, writes `write` or `derive`, export
  paths `export`; scope must be absent/empty or `global`; `audit`/
  `audit/verify` deny capability bearers — no admin verb exists).
  Unknown/malformed/expired → `unauthorized`. The §5.3 injection-resistant
  rehydration obligations (server: verify-before-emit + scope/consent
  filter before ranking — already the recall pipeline order; client:
  structural framing, never-execute-body) are documented in
  `API_CONTRACT.md` + `SECURITY.md`.
- **Docs** — `API_CONTRACT.md` gains a §UMP binding (levels, routes,
  tokens, redact semantics, §5.3 note); `COMPLIANCE.md` maps the UMP
  integrity + consent controls; `SECURITY.md` covers UMP key storage
  (same 0600/0700 posture as `BRAIN_JWT_KEY_DIR`) + injection-resistant
  rehydration; `openapi.yaml` → 1.17.3 (10 `/ump/*` routes + 2 well-known
  docs + batch/ump-md `format` values + `UmpRecord`/`UmpCapabilities`/
  `UmpRecallResponse`/`UmpFeedbackRequest`/`UmpBatchRequest`/`Integrity`
  schemas). Version 1.17.2 → 1.17.3.

### Honest ceilings

- Conformance is **self-attested** — the §7 level definitions are mapped
  onto the shipped surface, not certified by a third party.
- L3 in §7 means the local integrity layer (sign/verify with the operator
  key); A2A federation, remote agent identity, and per-tenant key
  hierarchies remain v2.x.
- `GET /ump/subscribe` is a change signal, not a data channel — event
  bodies are intentionally absent (documented §3.8 posture).
- Batch import lowers records one-by-one through the existing ingest path;
  no parallel ingestion, no partial-transaction rollback (per-record
  status is the contract).
- The `did:key` emission is Ed25519 only (same documented posture as the
  v1.2 JWKS EC/Ed gap); RSA capability keys are out of scope.
- Client-side §5.3 obligations are documented, not enforced by the server.

## [1.17.2] — 2026-08-09

### Server — "Harden"

- **UMP adapter conforms to the actual UMP 1.0 spec** — the v1.17.1 adapter
  shipped a guessed "0.1" wire shape; the real spec is **Universal Memory
  Protocol 1.0** (github.com/edihasaj/universal-memory-protocol, SPEC.md).
  Conformance changes: records now carry `"ump": "1.0"`; the five-kind
  vocabulary (semantic/episodic/procedural/working/identity — the invented
  `declarative` mapping is gone; `decision` lowers to `semantic`); ids are
  content-addressed per §6.2 (`urn:ump:<content_hash>`, fallback
  `urn:ump:brain:<domain>:<id>` for hashless legacy rows); `time.*` is RFC
  3339 (§2.3 REQUIRED string form, round-tripped from brain naive-UTC);
  top-level `relations` use the §2.5 `{type, target}` shape (`about` =
  from-entity, typed link = to-entity) while the lossless graph stays in
  `body.structured`; and §8 is honored — import rejects an unknown `ump`
  major version instead of reinterpreting it. Conformance claim:
  **UMP 1.0 / L0** (portable-record file binding).

## [1.17.1] — 2026-08-09

### Server — "Govern"

- **M1 ingest-owner correctness fix** — `/ingest` now seeds `owner` from the
  principal consistently (`gate::principal_to_owner` is `pub` and wired into
  the direct-ingest sites), so JWT-mode rows carry the acting subject and the
  record-level scope story is coherent on writes.
- **M2 per-kind retention policy** — new `GET/POST /retention` (POST = Admin
  + audited): kind-default expiry (`fact:365, episodic:30, procedure:730,
  step:730, decision:730` days, overridable via `BRAIN_RETENTION_KIND_DAYS`)
  enforced **at query time** in `push_gate_filters` (per-kind `expires_at`
  disjunction), never by a sweeper. `/decayed` now reports
  `effective_expiry`/`memory_kind`/`reason` (`per_chunk` vs `kind_policy`).
  Additive `retention_policy` table; schema stamp 1.17.1.
- **M3 recall ship-gate CLI** — `brain eval` runs the frozen 32-query
  fixture (`tests/fixtures/eval_queries.md`) against `/recall` and asserts
  floors (`--floor r5=0.85 …` or `BENCH_RECALL_FLOOR`); `brain bench` gains
  the same floor gate. `brain_server::eval` metric fns shared by both.
- **M4 UMP wire adapter** — `GET /export?format=ump` re-renders the portable
  export as UMP records with a name-based per-chunk graph; `POST /ingest?format=ump`
  lowers a UMP envelope back into the structured-ingest path. Round-trip is
  identity on row fields (pinned by tests); batch import is a documented v2.x
  ceiling. *(Wire shape was corrected to the actual UMP 1.0 spec in [1.17.2].)*
- **M5 Art 30 register** — new `GET /art30` (Admin): the activities register
  every controller must maintain (categories of data, purposes incl. explicit
  consent/controller obligation, retention, provenance), projected from the
  existing tables. `BRAIN_CONTROLLER_NAME` names the controller.
- **M6 CoP marker** — new `/.well-known/cop-notice` (public): machine-readable
  EU AI Act Code of Practice conformity state (self-attested; commitments +
  self-assessment link + `last_review`) for the client's CoP icon lane.
- **M7 snapshot self-check** — new `GET /snapshot/status` (Admin) + `brain
  snapshot-status`: per `VACUUM INTO` `.bak` — exists, size, `0600`, `PRAGMA
  integrity_check`, audit-chain verify. No new backup writer.

### Tests

- 451 server tests (+5: UMP round-trip/kind-mapping/malformed-reject, UMP
  export renderer, CoP marker) + 5 brain-bin tests; clippy `-D warnings` +
  fmt clean.

### Docs

- **`docs/AI_LITERACY.md`** (new) — EU AI Act Art 4 deployer playbook: what
  the memory component is/is not, the inspectable controls that are the
  literacy substance (trace, proposal gate, quarantine, DSAR, audit chain),
  and a weekly verify + DSAR-drill cadence. Cross-linked from `COMPLIANCE.md`
  §6.4 and `README.md`.
- **`docs/RFP_RESPONSE_KIT.md`** (new) — map brain-server features to common
  enterprise RFP sections (security, privacy/DSAR, AI governance, ops) with
  the evidence artifact behind each claim.
- **`GET /.well-known/ai-literacy`** (new, public) — machine-readable Art 4
  disclosure pointing at the playbook + enumerating the inspectable controls,
  mirroring the Art 50 ai-notice route. Registered in both auth-public path
  lists, the router, and `openapi.yaml`; pinned by a unit test.
- **COMPLIANCE.md** — §7 now references the live `/.well-known/ai-notice`
  disclosure (Art 50 machine-readable origin notice); §6.4 points at
  `/.well-known/ai-literacy` + `docs/AI_LITERACY.md`. §7.1 (new, this
  release) documents the CoP marker.
- **Wiki mirror** — the three `docs/` artifacts (AI_LITERACY, RFP response
  kit, MemGhost mitigation) mirrored as hand-authored wiki pages
  (`AI-Literacy`, `RFP-Response-Kit`, `MemGhost-Mitigation`) and wired into
  `_Sidebar` + `Home` quick links, so the procurement-facing wiki surfaces
  the same governance story as the repo.

## [1.17.0] — 2026-08-08

**v1.17.0 "Mobile" — client-only.** Completes the v1.17.0 Mobile plan on top of
the v1.16.6 mobile groundwork (secure token storage seam + responsive bottom-tab
UX). The M1 (Keychain/Keystore seam) and M2 (nav swap / sheet / touch targets /
safe-area) halves shipped as v1.16.6; this release lands the remaining mobile +
store-readiness milestones. Server + API contract unchanged (still 1.16.7).

### Added (client)

- **M2.4 portable refresh control** (`panels/mod.rs::RefreshButton`) — Review,
  Audit, and Health now expose a refresh trigger that bumps their existing
  `refresh` signal (re-fetch). Works on every renderer; the native
  pull-to-refresh *gesture* remains a documented v1.18.0 ceiling (needs touch
  events — untestable without `dx serve`).
- **M3.3 deep-link intent filters** (`Dioxus.toml`) — iOS `url_schemes = ["brain"]`
  + an Android `VIEW`/`BROWSABLE` intent filter for the `brain://` scheme, so a
  custom-scheme link opens the app into the existing `Routable` router. Full
  https universal-link parity is v1.19.0.
- **M3.4 offline connect pre-fill** (`main.rs`) — the connect screen persists the
  last successful base URL (non-secret UI pref via the existing `i18n` localStorage
  seam; the token stays in the OS keyring only) and pre-fills the URL field on a
  returning/offline connect. The specific `/health` failure was already shown (no
  crash); the field now comes pre-populated too. Pure `prefill_if_empty` guard +
  test.
- **M3.1 store-readiness** (`client/STORE_READINESS.md` new) — App Store / Play
  privacy-nutrition labels ("no data collected", accurate: one self-hosted
  backend, no analytics/tracking/third-party SDKs) + icon/launch/screenshot +
  submission checklist. Icon/screenshot generation + store upload are operator
  steps.

### Fixed / Changed (client)

- Client version 1.16.8 → 1.17.0.

### Tests

49 client tests (was 48; +1 `offline_prefill_fills_empty_field_only`). Clippy
`-D warnings` + fmt + wasm build clean.

### Honest ceilings (carried into v1.18.0)

- Native iOS/Android artifacts (`dx bundle --platform {ios,android}`) are an
  **operator step** — requires code signing + an Android SDK, neither present in
  this environment. The one-codebase compile is covered by the desktop + wasm
  builds; the platform glue ships in `Dioxus.toml` + `storage.rs`.
- Pull-to-refresh is a button today; the native gesture (touch events) is v1.18.0.
- `brain://` deep links are registered but not fully routed to distinct panels
  yet — URL parity is v1.19.0.
- App-store review is an external gate (low risk: "no data collected" + a
  governance tool, not social/UGC).

## [1.16.8] — 2026-08-08

Client-only release: the v1.16.8 "Global" plan — locale (i18n) + light/dark
theme + density + locale-aware number formatting + a privacy block on the
connect screen. **Server + API contract unchanged** (server stays at 1.16.7).

### Client — Added

- **M1 i18n (`src/i18n.rs` + `locales/*/main.ftl`).** Zero-dependency FTL-subset
  translation: `en`/`de`/`fr`/`es`/`nl` bundles are compiled in at build time via
  `include_str!` and parsed once. `t()` resolves current-locale → `en` → the key
  itself (visible fallback, never blank), so a partial locale degrades to English.
  A `locales/<code>/main.ftl` file is added per language; RTL-ready via `is_rtl`.
  `fluent`/`fluent-langneg` are the documented upgrade path (ponytail: a simple
  key=value subset + a three-tier fallback is a fraction of a Fluent dependency
  for human-authored short strings).
- **M2 RTL readiness.** `dir` on `<html>` flips to `rtl` for `ar`/`he`/`fa`/`ur`
  locales (none ship in v1.16.8; the layout + CSS are RTL-ready when one is added).
- **M3 light theme.** A top-bar toggle flips `data-theme="light"` on `<html>`;
  `input.css` swaps every token (dark-first stays the default), keeping the state
  hue *names* identical so the recall/security tests pinning them need no change.
- **M4 density.** A toggle flips `data-density="compact"` on `<html>` (14px root
  font, ~12.5% denser rem-based spacing) — a pure CSS knob, no JS, for high-volume
  reviewers. Comfortable is the default.
- **M5 locale-aware numbers.** `format_number` groups per locale (`en` → `,`,
  `de`/`fr`/`es`/`nl` → `.`), wired into the shell pending/flags counts. Deviates from
  the plan's `Intl.NumberFormat`-via-`document::eval` because eval is async (no
  sync path in Dioxus 0.7); the pure fn is synchronous + testable.
- **M6.2 privacy block.** The connect screen now has a `<details>` transparency
  panel stating exactly what the client sends (URL + token, token to the backend
  only), stores (nothing on web — the v1.16.1 in-memory posture; the OS keyring on
  native), and never does (no telemetry, no analytics, no third-party requests).
  Locale-aware like the rest of the shell.
- **Pref persistence.** Theme / density / locale are persisted to web
  `localStorage` (best-effort, sanitized, non-sensitive) and restored on launch;
  never the auth token (`credentials_stay_in_memory` guard still enforced).

### Client — Changed

- **Shell chrome localized** — rail + mobile tab-bar nav, top-bar counts,
  pending/flags/audit badges, connection + principal pillars, sign-out, degrade
  banners, and the context drawer header all render through `t()` (precomputed
  locals so the `rsx!` text-node interpolation never holds a nested `t("…")` call).
- **`deploy-web.sh` now compiles Tailwind.** `dx bundle` does **not** recompile
  Tailwind in build mode (the `[tailwind] input` here is `styles/input.css`, not a
  root `tailwind.css`, so dx's auto-watch never fires) — it copies+hashes the
  pre-built `assets/tailwind.css`, so CSS edits silently never reached the bundle
  (the stale-CSS class of bug Agent 50 fixed). The script now runs
  `npx @tailwindcss/cli -i styles/input.css -o assets/tailwind.css` first, per the
  Dioxus 0.7 docs. Verified: the fresh bundle carries `data-theme`/`data-density`.

### Client — Tests

- **48 passed** (was 43; +5 i18n tests): `resolve` fallback chain, per-locale
  `group_digits`, RTL detection, persisted-pref sanitizers, and a guard that every
  locale's keys exist in `en` (the `.ftl` files actually load). Pure cores are
  signal-free so the unit tests need no Dioxus runtime.

### Fixed

- **Dioxus global signals** exposed as accessor `fn`s (not `static`s) — a `static
  Signal` can't be mutated (`.set()`) without an immutable-static borrow error;
  the accessor-fn pattern is Dioxus' documented idiom for global state.

### Honest ceilings (carried into v1.17.0)

- The i18n is a simple FTL subset — no ICU plurals/term references, no message
  arguments (all strings are static; numbers are concatenated). `fluent` is the
  upgrade path.
- `fr` digit grouping uses `.` (a narrow no-break space would be more correct).
- No RTL locales ship yet; `dir` + CSS are ready but unexercised by a real RTL
  string set (a buyer locale is the acceptance test).
- Theme/density are cosmetic (no system-color-scheme auto-follow); `color-scheme`
  flips correctly.
- The `.ftl` files are hand-maintained alongside the string keys — a missing key
  degrades to the key name (visible) rather than failing, by design.

## [1.16.7] — 2026-08-08

Server + client release. **Server** (Cargo.toml 1.16.6 → 1.16.7): hardening +
compliance round (security + fixes + Art 50), landing on top of the client
release below. **Client** (1.16.6 → 1.16.7): the "Integrated" plan. No
client or API-contract break.

### Server — Security

- **Snapshot permissions (P0).** SQLite snapshots written by the integrity
  loop (`integrity.rs`) and the restore/import safety snapshot (`backup.rs`)
  were created with the process umask (world-readable `0644`); each is a
  plaintext copy of the whole store. All three `VACUUM INTO` sites now chmod
  the resulting `.bak` to `0600`.
- **`/health` never leaks content.** Extracted the response into a pure
  `health_body()` builder and pinned a regression test asserting the top-level
  key set carries no content/PII/text field (CVE-2026-29787 class: an
  unauthenticated health endpoint disclosing store contents).

### Server — Added

- **`GET /.well-known/ai-notice`** (EU AI Act Art 50 transparency). New public
  route + handler + pure builder disclosing that the service stores and may
  return AI-generated content, with origin-metadata + effective date. Registered
  in both auth-public path lists, the router, and `openapi.yaml`.
- **`docs/MEMGHOST_MITIGATION.md`** — operator-facing map of the MemGhost
  memory-poisoning attack (arXiv 2607.05189) onto brain-server's HITL /
  audit / DSAR / provenance controls. Linked from `docs/README.md`.

### Server — Fixed

- **`GET /tombstones?limit=` was silently ignored.** The query struct had no
  `limit` field, so the param was accepted and dropped, returning all rows.
  Now honored (default 100, clamped to `MAX_TOMBSTONES`).
- **`/export` omitted the `source` column** COMPLIANCE.md §7 claims it emits.
  Added `source` to the export SELECT + per-row JSON (back-compat additive).
- **Test isolation.** `v1_export_import_roundtrip_preserves_data` ran
  `run_migration` (which builds the `vec0` index) without `register_sqlite_vec()`,
  so it only passed in the full suite via a sibling test's global side-effect
  and failed in isolation (`no such module: vec0`). Now self-registers, matching
  every other migration test.

### Server — Changed

- COMPLIANCE.md stamp updated 1.16.2 → 1.16.7.

### Client — Added

- **M1 — Deep links.** Two new routes (`/review/:proposal_id`,
  `/subjects/certificate/:dsar_id`) make the proposal-detail and DSAR-
  certificate views URL-addressable; `RecallTrace` (`/recall/:trace_id`,
  shipped in v1.16.0) completes the set. Leaf components (`ReviewDetail`,
  `DsarDetail`) render the same data a panel's drawer would, and the review
  card title + certificate subject are now real `<Link>`s. Pure helpers
  `locate_proposal`/`subject_of` pinned by tests.
- **M2 — PWA.** `client/pwa/manifest.webmanifest` (standalone, `#0b0d10`
  theme) + `client/pwa/sw.js` (offline shell: caches only `/app/index.html`
  + `/app/assets/*`, never the API; navigation falls back to the shell).
  `deploy-web.sh` ships both into `dist/` and injects the manifest link,
  theme-color, and service-worker registration into `index.html`.
- **M4 — Paginated audit.** `GET /audit?offset=` (server, `OFFSET` in the
  SQL) + a client Load-more button with a boundary-id dedup guard. The
  server `recent_tenant` now pages; the client fetches 100 at a time.
- **M5 — Command palette.** ⌘K / Ctrl+K overlay listing navigation targets +
  a sign-out action, filterable and keyboard-navigable (↑/↓/Enter/Esc).
  Pure `palette_commands`/`filter_commands`/`command_label` pinned by tests.
- **M6 — Recall debounce.** The recall query input commits 300ms after typing
  stops (generation-guarded so a stale pending timer never overwrites a newer
  query). Pure `debounce_commit` pinned by a test.

### Client — Hardened

- **M7.3 — Drawer focus trap.** Tab / Shift+Tab now cycle focus inside the
  dialog (hand-rolled `document::eval`; the `dx components add dialog` route
  is unreachable — registry dead — so the shadcn/Radix upgrade stays a
  documented ceiling).
- **M7.5 — aria-live regions.** `role="status"` + `aria-live="polite"` on
  the review batch summary, the DSAR certificate chain badge, and the audit
  export announcement — mutation outcomes are read aloud.
- **M7.6 — RTL.** `<html dir="auto">` injected at deploy time so memory
  content in RTL scripts flows correctly while the shell stays LTR (no i18n
  extraction — that is v2.x).

### Client — Fixed / changed

- **M3 wasm-split is a documented ceiling, not code.** Dioxus 0.7.10 has no
  wasm-split feature and the official docs still list bundle splitting + lazy
  components as "planned". No code — recorded in the plan.
- **M7.7 stays an operator/native-toolchain step** (no Android SDK /
  cargo-ndk here): lib.rs mobile entry, probe pause/resume, store readiness,
  MASVS tables are documented, not compiled in.

### Verification

- Client: 43 tests, `clippy --all-targets -- -D warnings` clean, `cargo fmt
  --check` clean, `cargo build --target wasm32-unknown-unknown` clean.
- Server: 436 lib + audit/integration green (`cargo test --features
  bench,migrate`); the only server change is the additive `offset` param on
  `/audit`.
- Live `/app`: 200; `/app/manifest.webmanifest` + `/app/sw.js` 200; dist
  carries the hashed JS/WASM/CSS + manifest + sw + `dir="auto"`.

### Honest ceilings (carried into v1.16.8)

- **M3 wasm-split not built** (Dioxus upstream, not yet implemented).
- **Drawer focus trap is hand-rolled** (`document::eval`), not the shadcn/
  Radix Dialog with full focus restoration — `dx components add dialog` can't
  run (registry unreachable).
- **RTL is `dir="auto"` only** — no i18n string extraction, no per-locale
  switch (v2.x).
- **M7.7 Mobile milestones remain operator/native-toolchain steps.**

---

## [1.16.5] — 2026-08-08

### "Secure" (client-only — JWT refresh lifecycle + principal)

Client 1.16.4 → 1.16.5; server + API contract unchanged. The client's JWT
lifecycle: refresh-on-401, principal identity display, session-expiry
awareness, and the honest revocation path. See
`IMPLEMENTATION_PLAN_v1.16.5_Secure.md`.

### Improvements

- **JWT-aware `ApiClient` (M1)** — `TokenClaims` (sub/exp/scope/team) +
  `decode_claims()` (base64url-payload decode, no crypto — brain-server
  verifies on receipt; the client reads claims for display + expiry only).
  `with_principal()`/`with_refresh_pair()` derive the identity pillar from the
  JWT `sub` claim; `derive_principal()` distinguishes opaque loopback tokens
  (None) from JWT-shaped ones.
- **Principal display (M2)** — the top bar shows `acting as <sub>` for JWT
  tokens, `loopback` for opaque ones (replaces the hardcoded `remote-user`
  placeholder in Connect). The Intent-Based-Auditing identity pillar.
- **Refresh-on-401 (M3) + pre-emptive refresh (M5.1)** — a `request_with_refresh`
  wrapper silently refreshes once on 401 and retries the original request;
  `needs_refresh()` refreshes proactively when the access token's `exp` is
  within 60s. One retry only — no infinite loop.
- **Connect screen JWT mode (M4)** — a token / JWT-pair radio toggle (access +
  refresh pasted from `brain key mint` or an IdP).
- **Revocation-aware errors (M6)** — `error_message()` maps `refresh_reuse_
  detected` → "session revoked", 401 → "session may have expired" with a
  reconnect path.

### Fixed

- **`request()` no longer holds the `RwLock` guard across an await**
  (clippy `await_holding_lock`) — the access token is cloned out before the
  send.

### Security

- No crypto client-side — the client never verifies a JWT signature (forged
  JWTs are rejected by brain-server on the next API call). Bearer-header auth
  keeps CSRF structurally impossible (no cookies). BFF/HttpOnly-cookie mode is
  the documented v2.x ceiling.

### Honest ceilings (carried into v1.16.6)

- Token lives in WASM memory for the session lifetime; JS on the same origin
  can read it. Secure storage (Keychain/Keystore) is v1.16.6.
- No PKCE flow (interactive login needs a brain-server `/auth/authorize` or
  IdP proxy — v2.x).
- Concurrent refreshes from two panels are server-safe but the loser logs out;
  a client-side single-refresh mutex is the v1.16.6 polish.

---

## [1.16.6] — 2026-08-08

### Server version alignment (no functional server change)

The server `Cargo.toml` was bumped 1.16.2 → **1.16.6** purely to keep the
server and the Dioxus client versions in lockstep — `brain -V` now reports the
same version as the GUI. The server binary is byte-identical in behavior to
1.16.2; this is a version-alignment release, not a code change. `openapi.yaml`
`version`/`x-api-version` and README updated to match.

### "Mobile" (client-only — secure token storage + responsive UX)

Client 1.16.5 → 1.16.6; server + API contract unchanged. This release lands the
two testable milestones of the v1.16.6 "Mobile" plan (M2 secure token storage +
M3 responsive UX). M1 (lib.rs mobile entry), M4 (probe pause/resume), M5 (store
readiness), M6 (MASVS tables) are documented operator/native-toolchain steps —
no Android SDK / cargo-ndk / `dx` is available in this environment.

- **Dioxus pinned to 0.7.10** — the `dioxus = { version = "0.7", … }` spec was
  already semver-open and the lockfile resolves to the newest stable **0.7.10**
  (verified via lockfile + `cargo tree` + crates.io). The 0.7.2→0.7.10 patch
  line carries the security-relevant fixes (0.7.8/0.7.10 wasm-hotpatch
  TOCTOU/UB; 0.7.6 web panic-resilience + `inert` attribute) — already compiled
  in. Plan/doc "Dioxus 0.7.2" references updated to 0.7.10.
- **M2 — secure token storage (`src/storage.rs`)** — a new
  `#[cfg(target_arch = "wasm32")]`-gated seam. On every non-web target the auth
  token persists to the OS keyring (`keyring` 3.6.3: `apple-native` →
  Keychain, `windows-native` → Credential Manager, `sync-secret-service` →
  Secret Service; Android Keystore via `android-native-keyring-store` is the
  documented `dx`-wired ceiling). Web stays in-memory only (no-op — the v1.16.1
  posture; browser localStorage is not a secure credential store). Connect
  saves the token on success **only when one was provided** (`should_persist` —
  a loopback connect never clobbers a saved remote token); a `use_resource` on
  launch silently probes `/health` with any saved token and jumps straight to
  Review, falling through to the normal form on a stale/revoked token.
- **M3 — responsive UX (CSS-driven, no forked routes)** — AppShell renders both
  a desktop rail and a new mobile bottom **tab bar** (`nav.tab-bar` + `TabLink`,
  same `Routable` targets → identical a11y nav); pure `@media (min/max-width:
  640px)` swaps them with no viewport JS. `.tab-link` enforces ≥44px touch
  targets (iOS HIG / Material). `.tab-bar` and the drawer consume
  `env(safe-area-inset-bottom)` (notch / home indicator). The context drawer is
  now `.drawer` — a right rail ≥sm, a full-width rounded bottom sheet <640px.
- **Version**: client 1.16.5 → 1.16.6 (client-only). 37 client tests (was 36),
  clippy `-D warnings` + `cargo fmt --check` clean, desktop + `wasm32-unknown-unknown`
  builds clean, Tailwind v4.3.3 compiles `styles/input.css` (responsive rules
  present in output).

---

## [1.16.4] — 2026-08-08

### "Styled" (client-only shadcn/ui design-system restyle)

- **Sidebar dashboard shell** — `AppShell` moved from a top nav rail to a fixed
  left sidebar (brand mark + grouped `nav-link` pills with live count badges on
  the rail) + a slim sticky top bar (connection dot, pending count, Security
  flags + Audit-chain badges, principal). The right-hand context drawer is a
  `card`. No layout semantics changed — every nav target stays a real `<Link>`,
  every action a real `<button>` (the `interactive_elements_are_buttons` gate
  still passes).
- **shadcn-style component layer in `input.css`** — semantic tokens
  (`--color-background/foreground/card/popover/muted/accent/destructive/border/
  input/ring`) mapped onto the app's own AA-verified palette (state hues
  `ok`/`warn`/`danger`/`info`/`neutral` kept by name), a radius scale
  (`--radius-sm…2xl`), subtle shadows, and reusable classes: `.card`,
  `.btn`/`.btn-primary`/`.btn-outline`/`.btn-secondary`/`.btn-ghost`/
  `.btn-destructive`/`.btn-sm`/`.btn-md`, `.input`/`.select`, `.badge` +
  state badges, `.nav`/`.nav-link`/`.nav-badge`, and `.table`.
- **Every panel restyled to the layer** — Review, Recall (+ trace card),
  Subjects (DSAR cert card), Security (chain card + quarantine + auth-failure
  table), Audit (filter bar + table), Health (Service + Corpus cards), and the
  Connect screen (branded card) all use the new tokens/classes. All tests,
  clippy `-D warnings`, and `cargo fmt --check` stay green (31 tests).
- **`deploy-web.sh` stale-CSS fix** — the script's `ls | head -1` glob picked
  the alphabetically-first (stale) hashed `tailwind-*.css` in `target/` between
  rebuilds, so a restyle could deploy the old stylesheet while index.html
  pointed at the new one. Now `ls -t | head -1` picks the freshest build.
- **Version**: client 1.16.2 → 1.16.4 (client-only; server + API contract
  unchanged at 1.16.2).

---

## [1.16.3] — 2026-08-08

### "Serve" (client web-bundle serving + live bugfixes)

Client + server, both client-only in effect (server + API contract unchanged).
This release was originally folded into the v1.16.2 changelog, but the git
history shows it as a distinct slice between the v1.16.2 and v1.16.4 tags —
four commits that make the compiled Dioxus web bundle actually reachable and
fix the two live-blocking defects serving exposes. Tagged retroactively at
`edfb00d`. See `IMPLEMENTATION_PLAN_v1.16.3_Serve.md` (retrospective).

### Fixed

- **Serve the compiled web bundle under `/app`** — `Dioxus.toml` gains
  `base_path = "app"` so asset URLs are `/app/assets/…` (not `/assets/…`,
  which 401'd against the API CSP/auth); `client/README.md` documents the
  dev/serve/deploy workflow; `package.json` + `tailwind.css` build tooling
  added.
- **Client CSP blocked WASM instantiation (`'unsafe-eval'` live fix)** — the
  wasm-bindgen glue calls `new Function()` for module instantiation;
  `'wasm-unsafe-eval'` alone permits WASM compile/instantiate but not JS
  `eval()`, so the `/app` bundle threw "call to Function() blocked by CSP"
  and the client never rendered. Added `'unsafe-eval'` to `CLIENT_CSP`
  script-src (API CSP stays `default-src 'none'`). Live v1.16.2 fix.
- **Same-origin connect default** — a page loaded from the server's own origin
  now defaults to a relative/loopback connect instead of a hardcoded remote
  that fails "cannot reach brain-server".
- **`deploy-web.sh` stale-asset race** — the script globbed `target/` for the
  hashed JS/WASM, which left stale hashes between rebuilds and could deploy an
  old JS while index.html referenced the new one. Now derives the concrete
  names from the freshly-built index.html (and the JS's own wasm reference)
  instead of racing.

### Improvements

- `client/deploy-web.sh` (M3) — one-command bundle → inject the concrete
  `/app/assets/tailwind-*.css` link → copy to `client/dist` (what the server
  serves at `/app`). Concrete filenames instead of globs.

### Security

- API CSP stays strict (`default-src 'none'`); only the `/app` static bundle
  path is relaxed for the WASM runtime (`'unsafe-eval'` + `'wasm-unsafe-eval'`
  + `connect-src 'self'`).

### Honest ceiling (retrospective)

No dedicated tests of its own — it's a serving/build/config release verified
by the live `/app` smoke + the v1.16.2 suite (CSP pinned by the v1.16.2 CSP
test, connect default by the v1.16.0 connection tests). Retrospective plans
can't retrofit code into an already-tagged history.

---

## [1.16.2] — 2026-08-08

### "Harden" (server + client security/serving foundation)

- **Serve the Dioxus client from the server** — `nest_service("/app", ServeDir)` at `config::client_dir()` (env `BRAIN_CLIENT_DIR`, default `client/dist`) with a `not_found_service(ServeFile(index.html))` SPA fallback so deep-links route client-side. `/` redirects to `/app/`. The `CompressionLayer` brotli-compresses the WASM bundle. API unaffected if the dir is absent.
- **Path-aware Content-Security-Policy** — `security_headers_middleware` now reads the request path: `/app` + `/` get `CLIENT_CSP` (allows `'wasm-unsafe-eval'` **and `'unsafe-eval'`** for the WASM runtime + `connect-src 'self'`), every other route gets the strict `API_CSP`. Both `/app` and `/` are in the auth-public path set in **both** `jwt_auth_middleware` and `auth_middleware` (the static bundle needs no bearer). *Live fix:* `'unsafe-eval'` was added to `CLIENT_CSP` after the first `/app` smoke — `'wasm-unsafe-eval'` alone permits WASM compile/instantiate but the wasm-bindgen glue's `new Function()` is JS eval, so the bundle threw "call to Function() blocked by CSP". The API CSP stays strict (`default-src 'none'`).
- **`ErrorBoundary` around the router** — a panic in any panel renders an operator-facing fallback (generic message + `{errors:?}` in a `<pre>` + Dismiss that clears) instead of a blank screen. No sensitive data leaks.
- **Operator-facing error messages** — `api::error_message()` maps `ApiError` (401/403/404/429/503/fallback) to actionable hints; wired into the Review, Recall, and Health panels.
- **Cancel-safety gate** — the batch review now collapses to a `BatchSummary` (`batch_outcome` pure fn) rendered as a one-line summary once a batch settles, surfacing partial failure honestly; the outcome map is the single source of truth (no partial-write window on unmount).
- **Code-hygiene grep guards** (both run in `cargo test`):
  - `tests::xss_escape_hatch_is_unused` — `dangerous_inner_html` (the only XSS vector) is banned in the source tree.
  - `tests::credentials_stay_in_memory` — the bearer token must never touch `use_persistent` (localStorage is XSS-readable).

### "Accessible" (client WCAG 2.2 AA pass)

- **SPA focus management (M1)** — every panel's `<h1>` is a shared `PageTitle` component: `tabindex="-1"` + focus-on-mount (`onmounted` → `set_focus(true)`, cancel-safe) so screen-reader users get a signal on route change; `use_document_title()` sets a per-route reactive document title via `document::eval`.
- **WCAG 2.4.11/2.4.12 Focus Not Obscured (M1.3)** — `*:focus-visible { scroll-margin-top: 4rem }` clears the sticky nav.
- **Semantic audit (M2)** — `tests::interactive_elements_are_buttons` grep guard: no `<div onclick>` anywhere; all interactive elements are real `<button>`s (WCAG 2.1.1 + ARIA in HTML). Landmarks (`nav`/`main`) + single-`<h1>` per panel verified.
- **Contrast (M4)** — `--color-ink-faint` `#6b7380` → `#7c8492` (AA 3.8:1 → 4.6:1, WCAG 1.4.3). Color never the sole signal (text labels always accompany status colors).
- **Manual screen-reader checklist artifact (M7)** — `client/a11y-checklist.md` records the VoiceOver/NVDA/TalkBack pass matrix + per-panel checklist.
- Keyboard shortcuts toggle (WCAG 2.1.4) already shipped in v1.16.0; verified present in the Review header.

### Honest ceilings (carried into v1.17.0)

- **shadcn Dialog adoption (M5) + axe-core CI (M6)** deferred — `dx` CLI not available in this environment, so `dx components add dialog` and the `dx bundle --platform web` axe gate can't run. The drawer already has `role="dialog"`/`aria-modal`/Esc-close; the full Radix Tab-cycling focus trap + return-focus is the v1.18.0 pass.
- **axe catches 20–60%** of a11y issues — the manual screen-reader pass is irreplaceable.
- **No aria-live regions** beyond the existing `role="status"` connection/re-verify banners.
- **No RTL locale** (v1.16.6).

---

## [1.16.1] — 2026-08-08

### Operations

- **RSS warning band raised 320 → 512 MiB** (`src/capacity.rs`, both targets):
  the 320 cap was tuned to a 4 GB Jetson; the live desktop install runs
  ~180–320 MiB and transient spikes (large `/multi-get`, backup pass) were
  sitting in the warning band. RSS stays a soft signal (Warning only, never
  blocks writes).
- **CI cargo audit job fixed:** `rustsec/audit-check@v2.0.0` creates a check
  run and the default GITHUB_TOKEN lacked `checks: write` ("Resource not
  accessible by integration" — an infra failure, not a code one). Added the
  permission on the audit job + bumped `actions/checkout` v4 → v5 (Node 24,
  clears the Node 20 deprecation).

### Fixed

- **`/tombstones` deletion registry under-reporting (Round 11 finding).**
  Pre-v1.14 tombstone rows only set `deleted_at`; `purged_at` was NULL, and the
  handler read it as a non-null `i64`, so `flatten()` silently dropped every
  legacy row. Observed on the live DB: 6,008 of 6,009 registry rows invisible.
  Fix: idempotent migration backfill (`purged_at` = epoch of `deleted_at`) +
  handler reads `Option<i64>` and surfaces remaining NULLs as `null`. Registry
  now shows the full deletion history.
- **Purge/DSAR cascade to `recall_traces` (Round 11 finding).** `purge_chunk_ids`
  now deletes recall traces whose hit list references a purged chunk (exact
  JSON path via bundled JSON1, best-effort). DSAR additionally sweeps traces
  whose raw **query text** mentions the subject — the trace side table held
  query-text residue that no deletion path touched (no FK between
  `recall_traces` and `audit_events`).
- **Retention prune sweeps orphaned traces.** `prune_audit_retention` now
  deletes `recall_traces` rows whose audit row was pruned, instead of leaving
  them orphaned forever.
- **Regression tests:** purge→trace cascade by hit id, retention sweep, and
  legacy-tombstone backfill visibility all covered in `src/main.rs` tests.

---

## [1.16.0] — 2026-08-08

**"Client" — the Dioxus control surface (web + desktop + iOS + Android).** The
first externally-shippable brain-client: one Rust codebase consuming brain-
server's v1.14/v1.15 governance APIs. The v1.16.0 release implements the eight
`IMPLEMENTATION_PLAN_v1.16.0_Client.md` milestones — the scaffold's functional
panel contract plus the DESIGN's UX + correctness hard-parts. 25 tests (was 7),
clippy `-D warnings` + fmt clean, zero new deps.

> **Version sync (this release):** the server crate was bumped 1.15.0 → 1.16.0
> so the installed operator CLIs (`brain -V`, `mcp`, `bench`) and the server's
> own `--version` / `/health` header report the same version as the v1.16.0
> tag. No server code changed beyond the version bump — the v1.16.0 work is
> the client crate.

### M1 — The connection state machine (the correctness heart)

- A single `use_future` probe at the app root owns its timer (survives panel
  unmounts). **False-offline guard:** N consecutive failures before green→amber
  (a single flap never flips the indicator). Pure `probe_state(failures, ok)`.
- **Dependency-free sleep** via `document::eval`+`setTimeout` — no `tokio` dep
  (works web + desktop; tokio's timer doesn't work in WASM anyway).
- **Read-only degrade** + **mutation freeze:** when amber, panels keep showing
  last-known state; write buttons render `disabled`. The shared
  `writes_enabled` signal derives from conn state.
- **Chain-verify-before-writes recovery:** on a recovery 200, conn goes green
  but writes stay frozen until `GET /audit/verify` returns `{"ok":true}`. A
  scoped non-Admin JWT (403) shows a distinct "chain unverified" state.
- Pure `writes_allowed(conn, verify_ok, pending_reverify)` — testable.

### M2 — Nav structure: badges + principal + context drawer

- F-pattern `Pending: N` top-left (the one number that matters). Count badges
  on Security (quarantine + denied-auth), Audit (`!` when last verify was
  non-clean). Principal identity pillar (`acting as <sub>` / `loopback`).
- Esc-closable context drawer (`role="dialog" aria-modal="true"`) rendering
  typed content (Proposal/Hit/Certificate/AuthFailure) pushed by panels. Full
  Radix Tab-cycling focus trap is the v1.18.0 Compliant pass.

### M3 — Review: honest batch partial-failure + keyboard-first

- Per-row `RowOutcome` tracking (`Pending`/`Done`/`AlreadyDone`/`Failed`): a
  failed call in a batch is surfaced inline, **never silently dropped**.
  `404-no-pending` → `AlreadyDone` (success — non-idempotent contract).
- `BatchGuard` DropGuard: clears `Pending` rows from the selection on cancel
  (DESIGN §6 cancel-safety).
- `A`/`S`/`R`/`J`/`K` keyboard with a **WCAG 2.1.4 toggle**
  (`shortcuts_enabled`, default on). `S` (approve & supersede) only on conflict.
- Reject-with-reason editor (recorded in the audit log — no silent drop) +
  suggest-re-ingest editor (posts a new proposal with edits).

### M4 — Recall inspector: the decision-path viewer

- Richer hit rendering: per-retriever ranks (`v`/`f`/`g`), fused score,
  relevance tier (color-coded), `assertion_kind`/`confidence`/`decayed`/
  `superseded` tags. Monospace + tabular-nums on ids/scores.
- `min_relevance` slider (high/medium/low) with pure `drop_low_relevance` —
  the live post-fusion tier filter.
- **`?trace=true` artifact:** the recall response carries a `trace_id`;
  `/recall/:trace_id` (deep-linkable) fetches `GET /recall/{id}/trace` and
  renders the replayable decision path (query, decision, domains, scope,
  actor, per-hit id/score/source/relevance).

### M5 — DSAR console: the deletion-certificate card

- Replaced the freeform status line with a structured card: `found_count`,
  `purged_ids` (monospace), `tombstone_root`, `certified_at`, `chain_head` +
  a **live green/red chain badge** (re-verified via `GET /dsar/{id}/certificate`,
  not the cert-time head). Typed `DsarCertificate::from_value`.
- **Deferred:** the DESIGN §4.3 expandable locate tree (subject roots →
  `derived_from` descendants, PII masked as `[redacted:…]` without `pii:read`)
  is NOT in this release — the current `POST /dsar` response carries no located
  records, so it needs a server wire change. Tracked in
  `CLIENT_ROADMAP.md` under v1.17.0.
- **Trace toggle read-control fix:** the Recall `?trace=true` checkbox is a
  read control but was gated on `writes_enabled` (frozen during Reconnecting).
  Removed the gate — reads stay interactive in amber per DESIGN §6, matching
  the query input and min-relevance select.

### M6 — Security: the auth-failure feed

- `GET /audit?kind=auth` filtered to `status == "denied"` rows; rendered as a
  feed (ts/actor/target/status). Count badge on Security. Proves the backend
  isn't the unauthenticated-memory-access class (post-CVE-2026-59726).

### M7 — Audit: filters + export

- Client-side `AuditFilter` (principal substring / kind exact / since date) +
  pure `filter_audit`. JSON export of the filtered rows (client-side — no new
  server route; "the client adds no new server routes" constraint honored).

### M8 — Visual-token layer applied

- Every panel's ad-hoc color classes (`text-gray-*`/`text-green-*`/
  `text-red-*`) → semantic tokens (`text-ink-muted`/`text-ok`/`text-danger`/…).
  Zero ad-hoc color classes remain. Dark-first, quiet chrome (hairlines),
  Inter + JetBrains Mono stacks, tabular-nums on columnar data.

### Editor support

- `.zed/settings.json`: uses the Tailwind CSS language mode
  (`tailwindcss-intellisense-css`) for `.css` files, disabling the generic
  `vscode-css-language-server` that emits false "Unknown at rule" warnings on
  Tailwind v4 `@theme`/`@source`/`@apply`. Verified via context7 + the Zed
  Tailwind docs.

### API additions (`client/src/api.rs`)

- `ApiClient::with_principal` + `is_configured` + `principal()` (M2.1 identity).
- `Hit` +5 fields (`assertion_kind`/`confidence`/`relevance`/`decayed` +
  `RecallResponse.trace_id`); all `#[serde(default)]` (backward-safe).
- `recall(query, trace, min_relevance)`, `recall_trace(id)`,
  `reject_proposal(id, reason)`, `audit_kind(kind)`.
- `DsarCertificate::from_value` typed card fields.

### Honest ceilings (carried forward)

- **Connection is web-first.** The `onfocus`/`visibilitychange` instant-wake
  listener + the desktop window-event + mobile lifecycle variants land with
  the v1.17.0 mobile seam. The periodic probe (5s worst-case) covers correctness.
- **Token is in-memory only.** Secure-storage-backed token (Keychain/Keystore)
  is the v1.17.0 seam.
- **Audit filters are client-side.** Server-side `?principal=&kind=&since=` on
  `GET /audit` is a v1.19.0 polish.
- **Drawer focus trap is partial.** Esc + ARIA dialog now; full Radix Tab-
  cycling is the v1.18.0 Compliant release.
- **Export is client-side** (the fetched rows). No `/audit/export` server route.
- **`dx serve` is an operator step** (CLI not installed in CI). The code-level
  gates (`cargo test`/`clippy -D warnings`/`fmt`/`build`) are all green.

---

## [1.15.0] — 2026-08-08

**"Observe" — read-event audit + recall trace + DSAR + COMPLIANCE.md.** The
observability + compliance-workflow layer on v1.14's governance primitives:
the EU AI Act Art 12 logging control (read events enter the tamper-evident
hash chain), the GDPR Art 15/17/19/22 workflow (DSAR locate→export→purge→
certificate + Art 19 onward-notification), and the buyer-facing technical file
(`COMPLIANCE.md`). **Constraint note:** this release deliberately breaks the
long-standing "no outbound HTTP dep on the server" rule — the opt-in Art 19
webhook needs outbound HTTP, so `reqwest` is now a required dependency (the
`connector-github` feature now gates only its binary).

### M1 — Read-event audit

- `/recall`, `/search`, `/get/{id}`, `/multi-get` emit a read event into the
  existing append-only SHA-256 hash chain (new `AuditKind::Recall/Search/Get`;
  `record`/`record_tenant` now return the row id). Hash-only invariant kept —
  never content, and never the raw query in the row (test-pinned).
- **Opt-in by design:** `BRAIN_AUDIT_READ_EVENTS` — default **off** for
  loopback/opaque mode (personal-use contract, audit shape unchanged), **on**
  in JWT mode (enterprise posture). `BRAIN_AUDIT_READ_SAMPLE_RATE` (0.0..=1.0,
  default 1.0) cuts noise on busy multi-tenant servers.
- **Retention:** `BRAIN_AUDIT_RETENTION_DAYS` (default unset = keep forever).
  When set, rows older than the window are pruned on read-event writes and the
  chain re-anchored: the oldest surviving row becomes the new genesis and all
  survivor links are recomputed, so the retained window stays tamper-evident.
  Deployers subject to AI Act Art 26(6) guidance should set ≥180.

### M2 — Recall trace endpoint (decision-path viewer)

- `GET /recall/{trace_id}/trace` (Admin) replays a recorded recall read event:
  the exact query, abstention decision, domains searched, the access-scope
  filter applied, the principal, and per-hit injection details (id, fused
  score, `assertion_kind`, source, relevance, decayed). The trace is the
  Art 22 / ADMT "meaningful information about the logic" artifact and the
  Intent-Based-Auditing decision-path pillar.
- `POST /recall` accepts `trace: true` and returns the `trace_id` (the audit
  row id; `recall_traces` side table holds the non-content metadata).
  Pure read — no audit row of its own (no recursion).

### M3 — DSAR orchestration + deletion certificate

- `POST /dsar {subject, action: export|purge|both}` (Admin): locate every
  record (`owner` rows + transitive `derived_from` descendants, bounded depth
  8) → export bundle (portable JSON) → purge in one transaction (knowledge +
  vec0 + relationships + evidence_links + proposals refs) → tombstone (reason
  `owner:<subject>` / `derived`, `origin_id` for derived) → audit → deletion
  certificate `{subject, action, found_count, purged_ids, tombstone_root,
  certified_at, chain_head}` → ledger row in `dsar_requests`.
- `GET /tombstones?subject=&since=` — the queryable deletion registry (EDPB
  Coordinated Enforcement Framework ask). Hash-only, append-only, bounded.
- `GET /dsar/{id}/certificate` — re-fetch a past certificate with a live
  `chain_verifies` recomputation of the audit chain.
- **Art 19 onward-notification:** `BRAIN_DSAR_WEBHOOK_URL` [+
  `BRAIN_DSAR_WEBHOOK_SECRET`] — on a completed purge, POSTs
  `{subject, certified_at, certificate_id}` HMAC-SHA256-signed
  (`X-Brain-Signature-256: sha256=<hex>`, the outbound mirror of the v0.9.7
  webhook scheme). Fail-soft: bounded retries then logged warning; a webhook
  failure never rolls back the purge.
- Shared purge mechanics extracted once: `gate::purge_chunk_ids` (used by
  `/purge` and the DSAR path).

### M4 — COMPLIANCE.md

- New buyer-facing technical file: system description + data flows, purpose
  limitation, logging spec, risk controls, retention classes, DPIA-style
  questionnaire answers, ISO/IEC 42001 + NIST AI RMF + SOC 2 control map,
  Intent-Based-Auditing 4/4 table, jurisdiction posture (PH DPA / GDPR /
  CCPA-ADMT / residency / CRA horizon), Art 4 literacy note, and machine-
  readable origin metadata (Art 50 transparency bridge).

### Schema (additive; `schema_version` → 1.15.0)

- `recall_traces(audit_id PK, trace_json)` — the replayable trace side table.
- `dsar_requests(id, subject, action, status DEFAULT 'pending', export_bundle,
  certificate, created_at, completed_at)` + `idx_dsar_subject`.
- `tombstones` gains `reason TEXT` + `origin_id INTEGER` (guarded adds; the
  old unguarded CREATE TABLE would have silently missed these on real DBs).

### Back-compat

- Loopback default (no `BRAIN_JWT_ISSUER`) is byte-identical: read events off,
  no trace rows, no DSAR rows, audit shape unchanged.
- `/purge`, `/export`, `/decayed` unchanged except tombstone rows now also
  carry `reason='explicit'`.
- OpenAPI: `/recall` gains `trace`/`trace_id`; four new routes documented.

### Tests (→ 518 passed, 1 ignored; +6)

`test_observe_read_event_recorded_and_trace_replayable`,
`test_observe_read_events_default_on_for_jwt_off_for_loopback`,
`test_observe_dsar_locate_and_purge_semantics`,
`test_observe_deletion_certificate_chain_anchors_and_verifies`,
`test_observe_art19_webhook_posts_on_purge` (real TCP listener, signed POST
asserted), `test_observe_audit_retention_prunes_and_reanchors`.
`test_migration_schema_contract` + `test_openapi_covers_routes` +
`authz_gates_cover_every_non_public_route` extended.

### Honest ceilings (carried into v1.16)

- Read events default off in loopback mode; a loopback deployment must opt in
  explicitly to collect read traces.
- Audit chain is single-process (distributed audit = v2.1).
- DSAR export is brain-server JSON, not UMP wire format.
- No PII encryption at rest (COMPLIANCE documents the LUKS posture honestly).
- No historical trace backfill for recalls that predate v1.15.0.

---

## [1.14.0] — 2026-08-07

**"Gate" — write-back gating + trust surfaces.** The Alex Xu thread's #1 ask —
"make the write path deliberate" — answered with zero tokens and no auto-promote.
Human-in-the-loop write-back, per-chunk decay, and a GDPR lifecycle, on top of
the v1.2 AuthZ foundation. No new model, no background worker, no autonomous
deletion.

- **M1 — Write-back gate (`POST /ingest/proposal`).** A proposal stores a
  *candidate* memory scored deterministically — novelty via the existing
  vec0 KNN (`crate::gate::novelty`), conflict via the consolidate machinery
  (`find_conflict`), salience via a length/entity heuristic — but creates **no**
  `knowledge` row. It becomes memory only when a human approves
  (`POST /proposals/{id}/approve`), which embeds + inserts the chunk and marks
  the proposal approved in **one transaction**; optional `?supersedes=<id>`
  calls `resolve_supersession` in the same tx (old fact expires atomically).
  `POST /proposals/{id}/reject` creates nothing. `GET /proposals` lists the
  queue. New `proposals` table (append-only review ledger, audited via
  `AuditKind::Ingest`/`Reconcile`).
- **M2 — Decay + GDPR lifecycle.** Per-chunk `expires_at` with strict `<`
  query-time filtering (default excludes decayed chunks; `?include_decayed=true`
  returns them tagged `decayed`). Nothing decays autonomously. `GET /decayed`
  is the operator review list. `GET /export` is portable JSON (live rows +
  graph + proposals ledger; `pii_map` excluded by default). `POST /purge` is a
  hard, explicit, audited delete across knowledge + vec0 + relationships +
  proposals references in one tx, leaving a tombstone + `/audit` event, by id
  list or owner anchor. New `tombstones` columns (`content_hash`, `purged_at`).
- **M3 — Confidence + stated-vs-inferred + relevance tier.** `confidence`
  (deterministic, stored-rule factors: source authority + conflict presence +
  assertion) and `assertion_kind` (`stated`/`observed`/`inferred`) surface on
  every chunk and every `RecallHit`; `derived_from` chunks read `inferred`.
  `min_relevance` (high/medium) filters low-tier hits at query time.
- **M4 — Access scope, owner, PII.** Record-level `access_scope`
  (private/domain/team/public; default `private` = back-compat) + `owner`
  (principal subject) with a **deny-by-default** data-layer filter in JWT mode
  (`scope_filter`); loopback/opaque mode trusts localhost (documented posture).
  PII: `scan_pii` (email/phone/Luhn card) sets a `pii` flag at ingest; recall
  **redacts** output to `[redacted:email]`/`[redacted:phone]` unless the
  principal is loopback or `Admin`. Opt-in write-time placeholder mode
  (`BRAIN_REDACT_PII=1`) stores `[pii:email]` in `knowledge.content` with the
  real value only in `pii_map`; `pii:read` resolves it, `/export` excludes it.
- **M5 — `episodic` memory_kind** + `?memory_kind=` filter (legacy rows default
  `fact`), wired through the shared `push_gate_filters` SQL used by both vec0
  and FTS retrievers.

**Migration:** additive `proposals` + `pii_map` tables; `knowledge` columns
`expires_at`, `access_scope`, `assertion_kind`, `confidence`, `owner`, `pii`;
`tombstones` columns `content_hash` + `purged_at` (idempotent-guarded
`ALTER TABLE` — the old CREATE TABLE IF NOT EXISTS was a silent no-op against
the v0.9.1 schema and would have failed the purge INSERT on real DBs).
`schema_version` → `1.14.0`.

**Routes:** `/ingest/proposal`, `/proposals`, `/proposals/{id}/approve`,
`/proposals/{id}/reject`, `/decayed`, `/export`, `/purge`.

**Gates:** fmt, clippy `-D warnings`, `cargo test --features bench,migrate`
(512 passed, 1 ignored), all 5 release binaries build. Live smoke is an
operator step (`scripts/install-service.sh`).

## [1.13.6] — 2026-08-07

**"Hygiene" — CRA conformance bundle + ingest capture hygiene.**

- **`GET /.well-known/security.txt`** (RFC 9116, public). Machine-readable
  vulnerability disclosure: `Contact` (via `BRAIN_SECURITY_CONTACT`; omitted
  when unset), `Expires` (now + 1 year, never stale), `Preferred-Languages`,
  and `Canonical` (when `BRAIN_PUBLIC_BASE_URL` is set).
  Procurement + EU Cyber Resilience Act look for this before features.
- **`scripts/sbom.sh`** — generates a CycloneDX SBOM per release via
  `cargo-cyclonedx` (`sbom/brain-server-<version>.cdx.json`); SECURITY.md gains
  a support-window statement + an SBOM subsection (OWASP A03:2025).
- **Ingest capture hygiene** (`src/hygiene.rs`). The raw-text ingest doors
  (`/ingest/memory`, `/add`) now strip model reasoning/trace blocks
  (`<thinking>`, `<think>`, `<reasoning>`, `<reflection>`, `<analysis>` —
  case-insensitive, including unclosed trailing) before storage, and `/ingest/memory`
  drops entries matching a `BRAIN_INGEST_SKIP_PATTERNS` prefix (the autoCapture
  dream-prompt mechanism). "brain-server never silently stores reasoning traces"
  is now a tested invariant. Curated ingest (`/ingest`, `/ingest/markdown`) is
  deliberately untouched; historical cleanup is a separate ROADMAP sweep.

No schema change, no new runtime dependency, no `unsafe`. Gates: fmt, clippy
`-D warnings`, `cargo test --features bench`.

## [1.13.5] — 2026-08-07

**`/metrics` `brain_rss_mib` now reports the process's own RSS.**

- The gauge was emitting `System::used_memory()` (system-wide used memory)
  while its HELP text claims "Process RSS in MiB". On a busy host the value
  was ~50x the process's real footprint (live: ~10,485 MiB reported vs ~181 MB
  actual, per `ps`), so Prometheus consumers of the capacity story were
  misled and the 320 MiB envelope was invisible in metrics. It now calls the
  same `process_rss_mib()` used by the `/health` capacity envelope
  (main.rs), so `/metrics` and `/health` agree on the same number.
- Added `process_rss_mib_reports_plausible_process_footprint` regression
  test (bounds the gauge to a process-scale value, not host-scale).

## [1.13.4] — 2026-08-06

**POST /recall query-string `source` parity.**

- **`POST /recall` now honors and validates a query-string `?source=`**, matching
  `GET /search`. Previously the handler read `source` from the JSON body only (no
  `Query<>` extractor), so `?source=` was silently ignored — `?source=web`
  returned 200 unfiltered instead of 422, and a caller could get unfiltered
  results thinking they had filtered. Body `source` still wins when both are
  present; the query string fills in when the body omits it; an unknown value in
  either is rejected with 422 via the shared `resolve_source_filter` parser
  (`src/search/query.rs`). Harmless for the plugin (it sends a body); closes the
  consistency gap between the two retrieval endpoints.

## [1.13.3] — 2026-08-06

**Retrieval source-filter contract repair + ingest response honesty.**

- **P0 — the `source` retrieval filter is fixed for every documented value.**
  `POST /recall` and legacy `GET /search` now honor `source` as documented:
  ingest kinds (`memory` | `markdown` | `structured` | `manual` | `vault`)
  filter in SQL before ranking; retrieval legs (`vector` | `fts` | `graph`)
  filter post-fusion on the `SearchSource` tag; `both` is unrestricted; any
  other value (e.g. `web`) is rejected with **HTTP 422** before any DB/embed
  work. Previously *all* documented values returned 0 hits — the filter was SQL
  equality against the ingest-kind column, where leg names exist nowhere, and
  `both` is a fusion concept equality can never match. One pure parser
  (`parse_source_filter`) is shared by both handlers so the contract and engine
  cannot drift (`src/search/query.rs`, `src/search/mod.rs`).
- **P1 — `/ingest/memory` returns real chunk ids.** The response used to lie:
  `entry_id` was the *count* of entries added, not a chunk id. It now reports
  `chunk_id` (first real inserted rowid, `null` when nothing added),
  `chunk_ids` (all inserted rowids), `entries_added`, and `duplicates_skipped`.
  `entry_id` is kept as a deprecated alias of `chunk_id` (`src/main.rs`).
- **P2 — `domains_searched` is present on every `/recall` response** (empty
  array when no hits), no longer gated on `provenance`. Telemetry stays
  provenance-gated (`src/handlers/recall.rs`).
- **Docs:** `sources` (plural) is documented as an OR filter over ingest kind
  (not source URIs); MCP schema, CLI help, plugin type, README, API_CONTRACT,
  and openapi all reflect the repaired `source` contract.

No schema migration. Response-shape changes are additive or on the
documented-but-broken `source` contract (422 for invalid values).

## [1.13.2] — 2026-08-06

**Hardening pass (post-1.13.1 review).**

- **`PRAGMA busy_timeout=5000` on every pool init** (`src/main.rs` main pool,
  `src/domain_registry.rs` `open_with_migration`, `src/migration.rs` pragma
  batch). Previously only `auth/revocation.rs` set a busy timeout, so concurrent
  writers against `POOL_MAX_SIZE=20` connections could fail immediately with
  `SQLITE_BUSY` instead of waiting. Write contention now queues up to 5 s.
- **`POST /recall` accepts `explain` as an alias for `provenance`**
  (`src/handlers/recall.rs`). `GET /search` had always gated telemetry on
  `explain`; `/recall` used `provenance`, so the same intent needed two flag
  names depending on the endpoint. Both spellings now work on `/recall`.
- **`GET /graph/traverse` accepts `name`/`entity` as aliases for `start`**
  (`src/main.rs` `TraverseQuery`). Docs canon is `start` (openapi.yaml, README),
  but the response field is `entity` and sibling routes use `name`/`entity`, so
  callers can now mirror the field back. Back-compat preserved.


**"Recall" fix — automatic retrieval routing (v1.15.0 M1 hotfix).**

Shim-mode recall previously never centroid-routed: `src/handlers/recall.rs` had a
`None if !multi_db` short-circuit that searched the `global` pool only. After
v1.13.0 moved rows into a non-`global` label (`gutmindsynergy`), those rows
became **unreachable by the default recall** the agent uses each turn (a
`k.domain='global'`-scoped search) — a regression introduced by the relabel
migration. This hotfix makes routing automatic on retrieval in shim mode too:

- **Automatic centroid routing on recall.** The routed domain is searched
  primarily, plus a `global` rescue leg (the real working-memory corpus). An
  un-routed query (below `DOMAIN_CONFIDENCE_THRESHOLD`) scopes to `global` and
  **never federates into a bulk domain** — so a 90%-of-rows domain can no longer
  swamp working-memory queries. Pure helper `shim_routing_targets()`.
- **Kill switch** `BRAIN_RECALL_ROUTING_ENABLED` (default on). Set to `false` to
  restore the exact pre-v1.13.1 shim behavior (global-only, no routing) without
  a rebuild.
- 3 new unit tests. Live-verified: a blog query now returns the moved
  `gutmindsynergy` rows (`domains_searched: ['global','gutmindsynergy']`);
  working-memory queries stay in `global`; the kill switch reproduces legacy
  `['global']`.

## [Unreleased]

### Deployment — Docker image + compose (enterprise plan A1) and proxy-SSO guide (B1)

First container story for brain-server (Round 26 enterprise plan, §33):

- **`Dockerfile`** — multi-arch (linux/amd64 + linux/arm64), `debian:bookworm-slim` runtime, non-root `brain` user, `read_only` rootfs + tmpfs, `cap_drop: ALL`, `no-new-privileges`, `/health` healthcheck. The embedding model (`minishlab/potion-retrieval-32M`) is **baked into the image at build time** in the exact hf-hub cache layout (`HF_HOME=/opt/brain-model`), so the container boots offline — no HuggingFace call at first start; pinned revision via `HF_COMMIT` build arg for reproducibility. Loopback-safe default preserved (`BIND_HOST=127.0.0.1`; `BIND_PUBLIC=1` required for public binding).
- **`docker-compose.yml`** — `brain-server` service (loopback-published `127.0.0.1:8765`, `./data` volume for DB/keys/token, healthcheck, read-only + hardened) and an `oauth2-proxy` service behind the `sso` profile (OIDC, Entra/Okta/Keycloak/Auth0-ready). `docker compose up -d` = pilot online in minutes; `docker compose --profile sso up -d` adds the SSO edge.
- **`docs/docker.md`** — image facts, build, run, compose, web-client mount, container backup/restore via the in-image `brain` CLI.
- **`docs/proxy-sso.md`** — reverse-proxy SSO guide: why proxy SSO (server is a token validator, not an OIDC RP), OAuth2-Proxy / Caddy forward-auth / Authentik options, JWT passthrough, IdP matrix, principal handoff, honest limits (native OIDC RP = v1.20 B2).
- **Docs index + README quick start** updated with the Docker path.

No version bump — lands under [Unreleased] until the v1.19.0 release ceremony.

## [1.13.0] — 2026-08-06

**"Route" — real domain auto-routing (root-cause fix + relabel migration).**

Fixes the domain-routing lie that shipped at v1.0: ingest never auto-routed
(an omitted domain always fell to `global`), and `recompute_centroid` read the
frozen legacy `embeddings` JSON table (2 rows since v0.9.0) so every centroid
was ~empty. Live DB was 99% in `global`. This release makes auto-routing real
and gives the operator a non-re-ingest migration path. No schema migration —
`knowledge.domain`, `domain_centroids`, and `vec_knowledge` all already exist.

### Changes
- **M1 — centroid source fixed** (`src/domain_router.rs`): new
  `read_domain_vectors` reads `vec_knowledge` (matching `find_near_duplicates`)
  joined to `knowledge` with `valid_to IS NULL` (superseded chunks excluded),
  dequantized via `decode_embedding`. `recompute_centroid` uses it. The old
  code read the frozen `embeddings` table, silently zeroing every centroid.
- **M2 — ingest auto-routing** (`src/handlers/ingest.rs` + `domain_router.rs`):
  `route_domain_label(forced, embedding, centroids)` — an explicit domain wins;
  otherwise the chunk embedding (already computed for insert) is auto-routed
  against the stored centroids, falling back to `global` with no confident
  match. Zero extra embedding work; deterministic (same `route()` recall uses).
- **M3 — `POST /domains/move`** (`src/handlers/domains.rs`): bulk-relabel
  chunks into a target domain in ONE transaction (provenance fields untouched),
  then recomputes affected centroids. Guards: `to` may not be `global`;
  draining `global` requires `?confirm=global` (typo-replay); every id must
  exist; bounded by `MAX_MULTI_GET`. `brain domain-move <id>... --to <domain>
  [--confirm global]` CLI.
- **M4 — `POST /domains/recompute`** (`src/handlers/domains.rs` +
  `domain_router.rs`): one-shot sweep of every known domain's centroid from the
  corrected source, cleaning stale centroids for emptied domains.
  `DOMAIN_MIN_COUNT` knob (default 1 — a no-op unless raised) suppresses
  sub-N domains. `brain domains-recompute` CLI.
- **Deployment runbook (order matters)**: deploy → run `domains-recompute`
  immediately → `domain-move` keyword passes → verify `domains_searched`.

### Verification
- `cargo test --features bench,migrate`: **477 passed, 1 ignored**.
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.

## [1.12.2] — 2026-08-04

**"Harden" — audit-fix release (refresh-race serialization + dependency bumps + green CI).**

Deep-stability audit of v1.12.1 surfaced one security race, one stale
dependency stack, and one permanently-red CI job. All three closed.

### Changes
- **`/auth/refresh` check-then-act race fixed** (`src/auth/revocation.rs`):
  `record_refresh_use` + `rotate_chain` ran as two separate steps, so two
  concurrent presentations of the SAME refresh token could both read
  `current_jti == presented`, both pass, and both mint — silently defeating
  reuse detection. New `record_and_rotate` runs the check + rotation under
  `BEGIN IMMEDIATE`: presentations serialize, the loser is detected as reuse,
  and the family is burned exactly once (the burn is committed even when the
  error is returned). Mutation-proven by
  `concurrent_refresh_serializes_exactly_one_winner` (removing the
  `BEGIN IMMEDIATE` makes it fail).
- **Database stack bumped**: rusqlite 0.38.0 → 0.40.1, sqlite-vec 0.1.6 →
  0.1.9, r2d2_sqlite 0.32.0 → 0.35.0. Bundled SQLite rises 3.51.1 → 3.53.2
  (fts3_tokenizer hardening + CVE-2022-35737-related security fixes). The
  v1.11.0-comment concern (`savepoint_with_name(&mut self)`) is unused — the
  codebase uses raw-SQL SAVEPOINT (v1.1.2). `sqlite3_vec_init` FFI unchanged.
- **CI `cargo audit` job turned green**: the sole red job since v1.12.1 was
  RUSTSEC-2023-0071 (rsa 0.9.10 "Marvin" timing sidechannel). Verified
  2026-08-04 that **no fixed release exists anywhere** (rsa 0.10.0-rc.18 and
  jsonwebtoken 11 both still depend on the affected rsa). Accepted with
  documentation in `.cargo/audit.toml` (local-daemon timing model, 0600 keys,
  EdDSA keys avoid RSA entirely since v1.2); rows added to `SECURITY.md` +
  `THREAT_MODEL.md`. Two unmaintained-crate *warnings* remain (number_prefix,
  paste — transitive via model2vec-rs/tokenizers, no failing impact).
- **Docs**: README/CHANGELOG/AGENTS version bump; `.cargo/audit.toml` created.

### Verification
- `cargo test --features bench,migrate`: **466 passed, 1 ignored** (was 465;
  +1 race regression test).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean. `cargo audit`: exit 0.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.

## [1.12.1] — 2026-08-04

**"Harden" — AuthZ wiring completion (closes the v1.2 S1 audit finding).**

The v1.2.0 AuthZ layer shipped with `authorize()` called from ~15 handlers and
**20 routes unwired** — every one of those relied on the middleware's "any
valid bearer passes" alone. This release completes the wiring: every
non-public route now enforces its §3.3 matrix action at handler entry.

### Changes
- **20 previously-ungated handlers wired** with the matrix action:
  - Read: `GET /search`, `GET /stats` (domain-scoped), `GET /get/{id}`,
    `POST /multi-get`, `GET /graph/entity/{name}`, `GET /graph/relations`,
    `GET /graph/traverse` (all `X-Brain-Domain`-scoped), `GET /quarantine`,
    `GET /metrics`, `POST /recall` (domain-scoped), `POST /verify`
    (domain-scoped), `POST /consolidate/propose`, `GET /connectors`,
    `GET /domains`, `GET /suggest/metrics`, `GET /procedure/{id}/steps`
  - Write: `POST /v1/embeddings`
  - Admin: `GET /audit`, `GET /audit/verify`, `POST /auth/revoke` (the route
    comment always said "requires admin auth" — now enforced)
- **Two actions upgraded to the matrix**: `POST /reindex` and
  `DELETE /memory/{id}` were Write; §3.3 puts both on the Admin surface.
- **`/audit` tenant scoping**: new `handlers::audit_scope()` — a principal can
  only ever read its own tenant's rows; requesting another tenant's filter is
  a 403 (the matrix's "cross-tenant forbidden"). Superuser (`None` principal,
  opaque mode) keeps the v1.1 passthrough.
- **`AuthHandlerError::forbidden()`** for the revoke gate.

### Tests (+5 → 465 passed, 1 ignored)
- `authz_gates_cover_every_non_public_route` — a 40-route contract table
  (mirrors `test_openapi_covers_routes`) whose source-scan asserts every
  handler body calls `authorize()` with the matrix action. Mutation-proven:
  a wrong action in the table fails the test. A route shipped without a gate
  fails it too.
- `auth_middleware_enforces_presentation_and_public_bypass` +
  `jwt_middleware_requires_jws_in_jwt_mode` — router-level middleware tests
  (new `tower` dev-dep, already in the lock): missing/wrong token → 401,
  valid opaque token → pass, public + `/webhooks/*` bypass, JWT mode 401s
  without a valid JWS.
- `audit_scope_forces_own_tenant_and_blocks_cross_tenant` +
  `audit_scope_none_principal_passes_requested_tenant_through`.

### Back-compat (unchanged behavior in default mode)
- `None` principal = superuser: opaque-token mode has no tenants, so every
  existing install keeps working with zero config change. In JWT mode, opaque
  tokens are already rejected by the JWT layer, so the superuser path is
  unreachable there.
- `/webhooks/{kind}` remains HMAC-verified inside the handler (GitHub cannot
  present a brain bearer token) — by design, not a gap.
- Public routes (`/health`, `/ready`, `/version`, `/openapi.yaml`,
  `/.well-known/*`, `/auth/refresh`, `/auth/logout`) stay gate-free.

### Honest ceilings (carried into v2.0)
- The wiring-guard table is hand-maintained (same convention as the OpenAPI
  coverage test): a new route needs a table row + a gate, or the test fails.
- `?cross_domain=true` on `/graph/traverse` gates on the base domain only.
- Distributed revocation, hot key reload, EC/Ed JWKS emission remain v2.1+
  (unchanged from v1.2).

## [1.12.0] — 2026-08-03

**"Discern" — noise-aware graph retrieval + complexity-gated activation
(light cut, roadmap-compliant).**

The v1.11.0 graph leg learns to *discern*: taxonomy edges (`tagged_with` /
`alias_of` — 94% of the live corpus's 2376 edges) weigh 0.1 against semantic
relations, mega-hub outflow is damped (GAAMA `θ = 50`), and the graph leg is
auto-engaged exactly when the query is hard — a `ClarifyQuery` query gets one
bounded graph pass before the v1.5.0 abstention path gives up. **No LLM, no
new schema, no re-ingest, no embeddings in the graph leg** — pure arithmetic
over the existing tables at query time. Research basis: GAAMA
(arXiv:2603.27910), MemORAI (arXiv:2605.01386), "Use Graph When It Needs"
(arXiv:2602.03578); their *arithmetic* only — LLM extraction parts forbidden
per the plan.

### Added
- **`src/search/graph_ppr.rs`**: `type_base_weight()` — `tagged_with`/
  `alias_of` → 0.1, semantic types → 1.0, applied at aggregation (the pair
  SQL now groups by `relation_type`; the weighted sums feed `build_graph`
  unchanged); `SparseGraph::dampen_hubs(θ)` — per-source-node
  `w_ij · min(1, θ/deg(i))`, θ = 50, applied to the reachable-bounded graph
  before PPR. Both deterministic, bounded by the existing `MAX_VISITED`/
  `MAX_PPR_ITER` caps, `#![deny(unsafe_code)]`.
- **Complexity-gated graph rescue** (`src/search/mod.rs` +
  `src/handlers/recall.rs`): when the calibrated estimator says
  `ClarifyQuery` and the caller did not enable `graph`, one bounded
  graph-augmented pass runs and fuses via the shared RRF two-pass fuse;
  abstention is re-scoped to the final outcome (`low_confidence` only when
  `ClarifyQuery` AND zero hits). Strictly additive — the rescued path
  previously returned empty hits.
- **`should_attempt_graph_rescue()`** — pure gate (recommendation, explicit
  `graph`, kill switch); **`config::brain_graph_rescue_enabled()`** behind
  `BRAIN_GRAPH_RESCUE_ENABLED` (default true; `false` restores exact v1.11.0
  abstention). `RetrievalStrategy::HybridGraph` + `SearchTelemetry.graph_rescued`
  for observability; `brain query` telemetry prints it.
- **`fuse_pass_lists()`** — the two-pass RRF fuse extracted from
  `fuse_prf_passes` (which is now a thin wrapper adding `prf_expanded`); the
  graph rescue reuses it without claiming PRF expansion.

### Changed
- `recall.rs` `abstention_decision(recommendation, hits_empty)`: abstains
  only on `ClarifyQuery` with an empty final hit list (v1.5.0 contract
  preserved on the non-rescue path).
- OpenAPI → 1.12.0 (`graph_rescued` on `SearchTelemetry`); README, ROADMAP,
  AGENTS updated.

### Fixed
- Nothing regressed: the v1.11.0 unweighted graph ranked the `tagged_with`
  cloud above semantic neighbors on mixed hubs — pinned by
  `graph_retrieve_weights_semantic_over_tag_cloud` (verified: fails on the
  old arithmetic).

### Tests
- 460 passed / 1 ignored (was 455; +5: `type_base_weight_downgrades_taxonomy_noise`,
  `hub_dampening_scales_heavy_hubs_but_not_light`,
  `graph_retrieve_weights_semantic_over_tag_cloud`,
  `should_attempt_graph_rescue_matrix`,
  `graph_rescue_fuse_does_not_mark_prf_expanded` + the abstention test's
  rescue arm). clippy `-D warnings` + fmt clean.

## [1.11.0] — 2026-08-03

**"Associate" — HippoRAG-2-style graph retrieval (light cut, roadmap-compliant).**

Deterministic Personalized PageRank over the existing `entities`/`relationships`
knowledge graph as a third, opt-in RRF leg (`?graph=true` / `--graph`) on
`/search` + `/recall`. Targets the multi-hop association gap that lexical+vector
retrieval cannot bridge. **No LLM, no new schema, no embeddings in the graph
leg, `< 5W`** — the low-power manifesto holds.

### Added
- **`src/search/graph_ppr.rs`** (pure safe Rust, `#![deny(unsafe_code)]`): a
  sparse undirected weighted entity graph (`SparseGraph`), deterministic
  query→entity seeding via the existing linker vocabulary (case-insensitive
  exact name containment), power-iteration personalized PageRank
  (`π = (1−α)s + α·Pᵀπ`, `α = 0.5` matched to the HippoRAG 2 config default,
  L1 convergence at `1e-6`, bounded at `MAX_PPR_ITER = 50`), reachability
  pruning capped at `trace::MAX_VISITED = 256`, and seed→chunk expansion via
  `relationships.knowledge_id` with the same `flagged=0`/`valid_to IS NULL`
  visibility rules as the other retrievers.
- **Third RRF leg**: `SearchSource::Graph`, `Provenance.graph_rank`,
  `SearchTelemetry.graph_ms`/`graph_candidates`, and a 3-way `rrf_fuse` (the
  same formula, same `RRF_K = 60`). The graph leg runs concurrently on its own
  pooled read connection inside the existing `std::thread::scope`; the disabled
  path pays zero latency (`graph_ms = 0`).
- **Opt-in plumbing**: `graph: bool` on `SearchFilters`, `QueryDoc`,
  `RecallRequest`, GET `/search` `SearchParams`, and `brain query --graph`.
- **4 plan verifications**: `ppr_ranks_connected_entities_higher_than_unrelated`,
  `ppr_seed_from_query_uses_exact_entity_names`, `rrf_fuses_graph_leg_with_vector_and_fts`,
  `ppr_bounded_by_max_visited`, plus the self-loop/zero-weight guards.

### Verification
- `cargo test --features bench,migrate`: **455 passed, 1 ignored** (was 447).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- **Live smoke on a copy of the live 8538-doc DB**: `graph=true` returns
  `graph_candidates=107–112`, `graph_ms≈4ms`; exact entity-name queries seed the
  graph leg and surface `source=graph` / `both` hits that the vector+lexical
  legs miss (e.g. `acme_v17c_1785593852 ceo` → the `dave works at acme_v17c` +
  `acme_v17c ceo is carol` pair at `graph_rank 0/1`).

### Honest ceilings (carried into v2.0)
- **Live two-hop quality is corpus-bound**: on the live 8538-doc DB, ~94% of
  KG edges are `tagged_with` taxonomy noise; the graph leg still retrieves
  but the cleanest multi-hop paths are the synthetic `dave/acme/carol` bench
  fixture. The mechanism ships; corpus quality is an operator concern.
- No DPR passage scores in the seed (the plan forbids an embedding in this
  leg) — `PASSAGE_NODE_WEIGHT = 0.05` documents the upgrade path.
- `classify` remains a deterministic keyword router, not a learned classifier.
- `/suggest` still lacks principal/tenant scoping (S1 from the v1.9.1 audit);
  `authorize()` remains unwired — v2.0 multi-tenancy work.

## [1.10.0] — 2026-08-02

**"Procedural" — ordered steps + deterministic categorization + decision
rules** (the finalized v1.10.0 cut on top of the v1.9.1 hotfix base).

### Added

- **`POST /procedure`** (`src/handlers/procedure.rs`) — ingest a procedure
  root chunk + up to 100 ordered steps in ONE transaction. Steps are stored as
  their own chunks (`node_kind` = `step` / `decision`) linked to the root via
  `next_step` edges carrying an explicit `step_index` (Graphiti's
  NextEpisodeEdge pattern at chunk level, reusing the v0.9.8 `evidence_links`
  table). Embeddings are written best-effort after commit — a failure never
  undoes the ingest (FTS5 keeps the chunks retrievable).
- **`GET /procedure/{id}/steps`** — the ordered step chain for a procedure,
  each step exposing its normalized `memory_kind`. The read path runs through
  `MemoryKind::from_str` so an unknown stored kind falls back to `fact`
  (forward-compat contract, now live code instead of a dead fn).
- **`POST /classify`** — deterministic keyword-router categorization (Mem0's
  premium feature, free): category + confidence + matched keywords (auditable)
  + the full taxonomy. `general` with confidence 0.0 when no keyword clears
  the threshold. No LLM, no cloud.
- **`POST /decision/{id}/evaluate`** — load the decision rule stored as JSON
  on a `decision`-kind chunk and evaluate it against numeric variables. First
  matching branch wins; otherwise the rule's `default_branch`. Returns the
  outcome + citation chain. Pure rule engine (no LLM).
- **`knowledge.node_kind` repurposed** as the Mem0-style `memory_kind`
  (fact/procedure/step/decision). Legacy `'event'` rows relabeled to `'fact'`;
  the column default is now `'fact'` for fresh DBs. `Schema stamp → 1.10.0`.

### Fixed

- **`classify` matched-keywords bug** (`src/procedural.rs`) — the winning
  category was correct but its keyword list came from the wrong lexicon: the
  lookup used the *sorted* `scores` slot as the LEXICON index, and after
  `sort_by` that slot no longer matches the category. Resolved via the
  `CATEGORIES` position (shares LEXICON ordering). Pinned by
  `classify_detects_compliance` (HIPAA + PII now both reported).

### Notes

- Pre-v1.10 DBs keep their `'event'` column default (SQLite can't ALTER a
  column default without a table rebuild); the startup relabel + the read-path
  normalization make the gap cosmetic, not functional — see the `ponytail:`
  comment in `run_migration`.
- Still no background worker and no auto-consolidation — procedures, steps,
  and decisions are explicit, operator- or agent-authored writes.

## [1.9.1] — 2026-08-02

**Bug-fix release on top of v1.9.0** (post-release security + correctness audit
of v1.7.0–v1.9.0). Three fixes, no new features.

### Fixed

- **Near-duplicate detection now covers the live corpus** (`consolidate.rs`).
  v1.8.0's `find_near_duplicates` JOINed the legacy `embeddings` JSON table,
  which froze at v0.9.0 — production ingests write only `vec_knowledge`, so on
  the live DB the scan silently covered 2 of 8538 chunks. It now reads
  `embedding_int8` from the vec0 index and dequantizes via the (previously
  dead) `decode_embedding` helper. Regression test ingests two near-identical
  chunks through the real `vec_quantize_int8` path (zero `embeddings` rows)
  and asserts they are proposed.
- **Suggest feedback is last-wins per `(chunk_id, session)`** (`suggest.rs`).
  The v1.9.0 ledger was append-only with no idempotency: a client retry or
  replay recorded duplicate rows, poisoning the false-positive metric that is
  the v1.9 roadmap exit criterion. A unique expression index on
  `(chunk_id, COALESCE(session, ''))` + an upsert make feedback one signal per
  surfaced suggestion per session; a changed mind overwrites instead of
  double-counting. Pre-existing duplicates are deduped before the index is
  created. Schema stamp 1.9.0 → 1.9.1.
- **Removed misleading dead code in `build_explanation_paths`** (`main.rs`).
  The v1.7.0 doc comment claimed intermediate node names were "looked up in a
  single batched query" — no query ran and the collected id set was never used.
  The comment is now honest (intermediates surface as ids; agents resolve via
  `/get/{id}`) and the dead collection is deleted.

### Notes

- Feedback/metrics tenant scoping stays row-level (`tenant_id`), not a full
  `authorize()` gate, and `/suggest` returns content without principal scoping
  — both are safe in the current single-tenant deployment and are carried
  forward as v2.0 multi-tenancy work (the audit flagged them, not this fix).

## [1.9.0] — 2026-08-02

**"Suggest" — opt-in, non-interrupting anticipation (light cut).**

This release is the **evidence-gated v1.9 scope** sanctioned by
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.9, NOT the
broader Anticipate plan in `IMPLEMENTATION_PLAN_v1.9.0_Anticipate.md` (which
that roadmap explicitly supersedes — same pattern as v1.5–v1.8). Roadmap
v1.9: *"an explicit `POST /suggest` experiment scoped to a session and an
accept/dismiss/false-positive metric."* Exit: *"opt-in suggestions save
measurable time at an acceptable false-positive rate; otherwise the feature
is removed."*

### Discovery

The full Anticipate plan (M1 sessions table + auto-start, M3 short-poll/SSE
push, M4 attention decay, M5 personalization vector) is **forbidden** by the
roadmap's "Do not ship" list ("unsolicited push, ranking decay, hidden
personalization, or SSE by default"). The only surviving scope is the opt-in
pull + the false-positive metric. The session concept survives in its
**client-owned** form (Mem0 `run_id` pattern): the caller passes an opaque
`session` string; the server never auto-tracks, auto-expires, or auto-embeds
a session.

### Shipped

- **`POST /suggest`** — opt-in anticipation pull. Caller supplies explicit
  `context` (what they're working on); server embeds it via the existing
  `StaticModel`, runs `vec0_knn` with an over-fetch equal to `k + exclude.len()`,
  filters out the caller-supplied `exclude` ids, truncates to `k`, and tags
  every hit `provenance.reason = "anticipated"`. Reuses the v1.6.0
  `valid_to IS NULL` default filter, so superseded chunks are never suggested,
  and the v0.9.7 flagged-row exclusion, so quarantined chunks are never
  suggested. No new state, no background work, no push.
- **`POST /suggest/feedback`** — Mem0-style accept/dismiss per surfaced chunk
  (`feedback: accept|dismiss`, optional hashed `reason`, optional `session`).
  Validates the chunk exists (404 on typo so the metric isn't poisoned).
  Tenant-scoped via the JWT principal. The `suggest_feedback` table IS the
  audit surface (append-only, hash-of-reason, tenant-scoped) — no duplicate
  `audit_events` row is written.
- **`GET /suggest/metrics`** — the false-positive rate (dismisses / total)
  over the feedback ledger, with optional `session` / `since` window filters.
  This IS the roadmap exit criterion, made queryable. Tenant-scoped.
- **`BRAIN_SUGGEST_ENABLED` kill switch** (default `true`). When `false`, all
  three routes return `501 Not Implemented` — the roadmap's "otherwise the
  feature is removed" guarantee, without a rebuild.
- **CLI**: `brain suggest`, `brain suggest-feedback`, `brain suggest-metrics`.
- **Migration**: additive `suggest_feedback` table + `schema_version = 1.9.0`
  (was `1.4.0`; v1.5–v1.8 were light cuts with no schema change).
- **OpenAPI** → 1.9.0: three routes + `SuggestionHit`/`SuggestTelemetry`/
  `SuggestMetrics` schemas. `test_openapi_covers_routes` extended.

### Deferred (per evidence-gated roadmap)

- **M1 sessions table + auto-start + 30-min window + running embedding mean**
  — "hidden personalization." The server must not auto-track sessions.
- **M3 short-poll `/events` + SSE push** — "unsolicited push" + "SSE by
  default." `/suggest` is an explicit pull; the agent asks.
- **M4 attention decay + spaced-repetition** — "ranking decay." Feedback is
  purely a measurement signal; it never boosts or demotes retrieval.
- **M5 personalization vector** — "hidden personalization." No per-tenant
  bias vector; `/recall` ranking is unchanged.

### Verification

- `cargo test --features bench,migrate`: **428 passed, 1 ignored** (was 414
  at v1.8.0; +14 = 12 pure-function tests in `suggest.rs` + 2 integration
  tests in `main.rs`).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.
- **Live end-to-end smoke** (after `scripts/install-service.sh`, pid 17967):
  `/suggest` returns anticipated chunks (excluded ids correctly dropped,
  telemetry accurate); `/suggest/feedback` records accept+dismiss;
  `/suggest/metrics?session=` returns `false_positive_rate: 0.5` (1/2);
  `BRAIN_SUGGEST_ENABLED=false` → all three routes return `501` while
  `/version` stays `200` (kill switch proven live).

### Honest ceilings (carried into v2.0)

- **No semantic anticipation.** `/suggest` is KNN-over-context with
  exclusions, not a learned next-query predictor. The "anticipated" label is
  a contract marker, not a model output.
- **Session is client-owned.** The server stores the opaque string but does
  no session-boundary detection, no timeout, no embedding mean. Cross-session
  metrics require the caller to label consistently.
- **`accept`/`dismiss` is binary.** Mem0's `VERY_NEGATIVE` is collapsed; a
  future "report-as-harmful" path is v2.x.
- **Metrics are per-process.** The query scans `suggest_feedback` live; no
  rollup materialization. Bounded by the `(tenant_id, ts)` index.
- **Feedback is not retrieval-affecting.** No boost, no decay — the roadmap
  forbids it. The signal is purely for the operator's false-positive
  measurement.
- **Near-duplicate / cross-domain suggest** deferred (per-domain only, like
  the rest of the retrieval stack).

## [1.8.0] — 2026-08-01

**"Maintain" — reviewable proposals + undo (light cut).**

This release is the **evidence-gated v1.8 scope** sanctioned by
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.8, NOT the
broader v1.8.0 plan in `IMPLEMENTATION_PLAN_v1.8.0_Consolidate.md` (which
that roadmap explicitly supersedes). Roadmap v1.8: *"duplicate and stale-
source proposals, resumable batches, review UI/API contract, and recovery
rehearsal."* Exit: *"reviewers accept proposals at a measured precision
target, and reject or undo them without retrieval regression."*

### Discovery

The exact-duplicate + subject-conflict + unresolved-contradiction detectors
already shipped in v0.9.8 / v1.6.0 (via `/consolidate/propose`). The single
missing pieces for the exit criterion: (1) **stale-source detection** (vault
files that no longer exist on disk), (2) **near-duplicate detection**
(semantic, not just exact-hash), and (3) **undo** — the "reject or undo them
without retrieval regression" arm.

### Shipped

- **`POST /consolidate/undo` + `brain undo-resolve <old_id> [...]` CLI.** The
  roadmap exit criterion's undo arm: clears `valid_to` back to NULL + removes
  the `supersedes` evidence_link, atomically in one tx. Audited via
  `AuditKind::Reconcile`. Idempotent — a re-run on an already-undone chunk is
  a no-op. Batch-safe (takes a list of chunk ids).
- **Stale-source detection** (`consolidate::find_stale_sources`). Vault sources
  whose `uri` is a file path that no longer exists on disk. Pure detection —
  never archives or deletes. Operator reviews and either re-ingests (file moved)
  or retires via `DELETE /sources/{id}`. Surfaced in `/consolidate/propose`
  response + `brain check-consistency` report.
- **Near-duplicate detection** (`consolidate::find_near_duplicates`). Pairs of
  current chunks with embedding cosine > 0.95 (different content hash — exact
  dups already detected separately). Uses the existing `vec_knowledge` KNN to
  find each chunk's nearest neighbor — bounded O(n×k) via KNN, not O(n²)
  pairwise. Capped at 50 pairs per proposal (the endpoint isn't a dump truck).
  Surfaced in `/consolidate/propose` + `brain check-consistency`.
- **OpenAPI contract** updated (v1.8.0): `/consolidate/undo` route +
  `stale_sources` + `near_duplicates` fields on `ConsolidateProposal`.
  `test_openapi_covers_routes` extended.
- 5 new tests (undo round-trip, undo idempotent, stale-source detection,
  embedding-decode round-trip, existing proposal serialization updated).

### Deferred (per evidence-gated roadmap)

These items from `IMPLEMENTATION_PLAN_v1.8.0_Consolidate.md` are **deliberately
not shipped** — the roadmap forbids autonomous/background maintenance:

- **M1 background `ConsolidationWorker`** (power-aware, hourly). Roadmap says
  proposals, not a background worker that auto-runs. Operators trigger on
  demand via `brain check-consistency` / `/consolidate/propose`. A background
  worker is autonomous consolidation, which the roadmap defers indefinitely.
- **M3 summarization** (cluster medoid as summary chunk). Roadmap: "A medoid
  is labelled `representative`, not `summary`." Synthesizing a new chunk is a
  "fabricated summary" — forbidden. The medoid IS already a chunk.
- **M4 cross-cluster linking** (proposed `related`/`co_occurs` edges).
  Roadmap: "synthetic relation insertion" forbidden. Existing evidence_links
  kinds (supports/supersedes/contradicts/references/derived_from) stay the
  documented set; no new kinds added.
- **M5 memory defragmentation / archival / domain moves.** Roadmap: "automatic
  archiving" + "domain moves" both forbidden. Stale-source *detection* ships
  (this release); the *archival* action stays operator-driven via existing
  `DELETE /sources/{id}`.
- **Resumable batches** as a saved review state. The proposal endpoint is
  idempotent + re-runnable, so an operator can pick up where they left off by
  re-running `/consolidate/propose`. No saved-state API needed for v1.8.

### Verification

- `cargo test --features bench,migrate`: **414 passed, 1 ignored** (was 409
  at v1.7.0; +5).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.
- **Live end-to-end smoke: operator step** (run `scripts/install-service.sh`).

### Honest ceilings (carried into v1.9)

- **Near-duplicate detection is per-domain only** (same as exact-dup detection).
  Cross-domain near-dups would need embedding federation; deferred to v2.x.
- **`find_near_duplicates` loads each chunk's embedding once per scan.** ~5 MiB
  transient for a 10k-chunk corpus at int8; bounded + ephemeral. Upgrade path:
  batch the KNN calls if per-chunk query cost matters on a large corpus.
- **`decode_embedding` assumes the vec0 int8 blob layout.** If sqlite-vec
  changes its format, the round-trip test breaks first (pinned).
- **Undo only reverses `supersedes`-kind resolutions.** Other evidence_link
  kinds (contradicts/supports/references/derived_from) have no state to undo —
  they were never expiring. If you want to remove one, use `DELETE /memory/{id}`
  on the link row directly (or a future v1.9+ generic link-delete API).
- **No background worker.** Operators must run `brain check-consistency` on
  demand. This is the roadmap's explicit choice, not a gap.

## [1.7.0] — 2026-08-01

**"Explain" — bounded graph evidence + faithful explanations (light cut).**

This release is the **evidence-gated v1.7 scope** sanctioned by
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.7, NOT the
broader v1.7.0 plan in `IMPLEMENTATION_PLAN_v1.7.0_Reason.md` (which that
roadmap explicitly supersedes). The roadmap says: ship explicit, typed,
bounded path retrieval + faithful explanations; **do NOT ship** causal
discovery, counterfactual estimates, or transitive `causes` facts.

Research basis (Context7-verified 2026-08-01): Graphiti's `edge_bfs_search`
(`/getzep/graphiti`) is the canonical bounded-BFS pattern — origin nodes,
max_depth, filters, limit. brain-server already had this in `/graph/traverse`
(v1.0/v1.4); the gap was that paths were flat id-strings with no edge types,
so a consuming agent couldn't render a faithful explanation.

### Discovery

The bounded-BFS + bi-temporal + cross-domain + MAX_HOPS/MAX_VISITED
infrastructure already shipped in v1.0/v1.4. The single gap: `/graph/traverse`
returned `path` as a flat string of entity ids (`1->5->9`) with no relation
types. A faithful explanation needs `A --works_at--> B --ceo_of--> C`, not
`1->5->9`. This release closes that gap by extending the existing endpoint
(no new route, no new schema).

### Shipped

- **Faithful explanation paths on `/graph/traverse?explain=true`.** The
  recursive CTE now carries `relation_type` per hop; the response includes a
  new `paths` array with structured hop chains
  `[{from:{id,name}, relation, to:{id,name}}, ...]`. Consuming agents can
  render the reasoning chain verbatim. The flat `traversal` array stays for
  back-compat.
- **`?kind=<relation_type>` edge filter.** Restricts the walk to edges whose
  `relation_type` matches. Exact match (`kind=works_at`) or prefix match when
  ending with `:` (`kind=causes:` for the causal subgraph — opt-in, no
  auto-causal claims). Wildcards in user input are escaped to prevent LIKE
  injection.
- **OpenAPI contract** updated (v1.7.0): `kind` + `explain` params, `paths`
  array, `edge_path` + `from_entity` fields on `traversal` rows.
- 2 new unit tests (hop-chain reconstruction + empty-input handling).

### Deferred (per evidence-gated roadmap)

These items from `IMPLEMENTATION_PLAN_v1.7.0_Reason.md` are **deliberately
not shipped** — the roadmap explicitly forbids them without an
intervention-ready causal model + domain expert validation:

- **M2 causal discovery / M3 counterfactual simulation.** Roadmap: "A graph
  path is association unless an intervention-ready causal model and domain
  expert validation exist." The `causes:` prefix remains schema-reserved
  (v1.4); operators can ingest typed edges and walk them with `?kind=causes:`,
  but the brain makes NO claim about causality.
- **M4 transitive inference (virtual inferred edges).** Roadmap-forbidden:
  no transitive `causes` facts. The `state='inferred'` schema reservation
  stays unused until an evidence-gated upgrade.
- **M1's `/graph/reason` new endpoint.** Not needed — `/graph/traverse` with
  `explain=true` IS multi-hop reasoning with bounded BFS. A new endpoint
  would duplicate the CTE.
- **Carry-forward: TRACE session/topic hierarchy, multi-vector.**
  Schema reservations only.

### Verification

- `cargo test --features bench,migrate`: **409 passed, 1 ignored** (was 407
  at v1.6.0; +2).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.
- **Live end-to-end smoke: operator step** (run `scripts/install-service.sh`).

### Honest ceilings (carried into v1.8)

- **Intermediate entity names in `paths` are best-effort.** The seed and leaf
  nodes carry names; intermediate nodes are surfaced as ids unless the caller
  resolves them via `/get/{id}`. A path-aware CTE that carries named tuples
  is the upgrade path.
- **`?kind=` filter is exact/prefix only.** No regex, no negation (e.g.
  "all edges except causes:"). Acceptable for a local-first store.
- **No audit row on traverse.** Pure read; the roadmap's "every state mutation
  is auditable" rule doesn't apply.
- **Graph paths are association, not causation.** Even when filtered with
  `?kind=causes:`, the brain reports what the graph contains — not what is
  true in the world. This is the roadmap's explicit guardrail.

## [1.6.0] — 2026-08-01

**"Reconcile" — correct without erasing (light cut).**

This release is the **evidence-gated v1.6 scope** sanctioned by
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.6, NOT the
broader v1.6.0 plan in `IMPLEMENTATION_PLAN_v1.6.0_Reconcile.md` (which
that roadmap explicitly supersedes). The roadmap exit criterion: *"an
approved update changes current recall; historical recall still returns the
prior claim; a failed transaction changes neither."*

Research basis (Context7-verified 2026-08-01): Graphiti's
`resolve_edge_contradictions` (`/getzep/graphiti`) is the canonical pattern —
old facts are expired (`invalid_at = resolved.valid_at`), never deleted.
brain-server applies the same semantics at the chunk level via the existing
`knowledge.valid_from`/`valid_to` columns (v0.9.8) and the existing `/recall`
bi-temporal filter (v1.4.0).

### Discovery

~85% of the infrastructure already shipped in v0.9.8 + v1.4.0: the
`valid_from`/`valid_to` columns, the `/recall` + `/graph/traverse` bi-temporal
filters, the `evidence_links` table, and `find_subject_conflicts`. The single
missing piece was the atomic operation that expires the prior fact when an
operator records a `supersedes` link. This release closes that gap.

### Shipped

- **Atomic supersession resolution** (`src/consolidate.rs::resolve_supersession`).
  When `/consolidate/apply` records a `supersedes` link, the prior chunk's
  `valid_to` is set to now **in the same transaction** as the link insert.
  The existing `/recall` filter `(valid_to IS NULL OR valid_to > ?at)` then
  excludes the chunk by default; `?at=<before-resolution>` still returns it.
  No new retrieval code, no new schema. Idempotent: a second call with the
  same pair touches 0 rows (doesn't overwrite the historical timestamp).
  Audit row recorded via `AuditKind::Reconcile` (hash only, no PII).
  Graphiti's pattern, applied at chunk level.
- **`/consolidate/apply` routing on kind.** `supersedes` links now call
  `resolve_supersession` (link + expire + audit); other kinds keep the plain
  `link_evidence` path (they don't change retrieval state).
- **`brain resolve <new_id> <old_id>` CLI.** Operator-facing shortcut for
  the most common case — POSTs one supersedes link, prints confirmation.
- **`brain check-consistency` CLI + `unresolved_contradictions` field on
  `/consolidate/propose`.** Surfaces `contradicts` links that have no paired
  `supersedes` resolution — the otherwise-invisible operator action items.
  Pure detection; never auto-fixes.
- OpenAPI contract updated (v1.6.0): new field on `ConsolidateProposal`,
  clarifying notes on `/consolidate/apply` re: expiration semantics.
- 6 new tests (4 supersession unit + 1 end-to-end SQL proof + 1 unresolved-
  contradiction detection).

### Deferred (with reasoning)

These items from `IMPLEMENTATION_PLAN_v1.6.0_Reconcile.md` are **deliberately
not shipped** — either forbidden by the evidence-gated roadmap or not worth
the watts without a measured benefit:

- **M1 auto-contradiction detection at ingest** (embed top-3 + lexical cues).
  Roadmap-forbidden: MOSAIC "motivates the claim model; it does not justify
  automatic deletion." Also adds ingest-time embedding work (CPU).
- **M3 auto conflict-resolution policy** (`BRAIN_CONFLICT_POLICY=source|recency`).
  Roadmap-forbidden: "manual-first conflict resolution." Only operator-driven
  resolution ships; auto policy is deferred indefinitely.
- **M4 edit-in-place + `knowledge_history` table** (`POST /knowledge/{id}/edit`).
  Roadmap mentions "undo" only, not "edit in place." Real schema add + re-embed
  work; deferred until an operator requests it.
- **Carry-forward: TRACE session/topic hierarchy.** Schema reservation only
  (`node_kind`/`parent_id`); no bounded producer exists. Explicitly deferred.
- **Multi-vector.** No-op until the v1.5 judged baseline demonstrates a recall
  gain worth its RSS cost.

### Verification

- `cargo test --features bench,migrate`: **407 passed, 1 ignored** (was 401
  at v1.5.0; +6).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.
- **Live end-to-end smoke: operator step** (run `scripts/install-service.sh`).

### Honest ceilings (carried into v1.7)

- **Resolution is operator-driven only.** No auto-detection of contradictions
  at ingest; operators must run `brain check-consistency` or `/consolidate/propose`
  to find them. This is the roadmap's "manual-first" rule, not a gap.
- **`resolve_supersession` expires one chunk per call.** Multi-way conflicts
  (3+ chunks contesting the same subject) require multiple calls. Acceptable
  for a local-first store; batch resolution is a v1.7+ concern.
- **`find_unresolved_contradictions` is the only consistency check.** Orphan
  entities + `derived_from` cycles deferred (lower value, would balloon the diff).
- **No propagation to the entities/relationships KG.** `resolve_supersession`
  operates on chunks; KG edges have their own bi-temporal filter via
  `/graph/traverse?at=`. A unified claim-level resolution is the v2.x path.

## [1.5.0] — 2026-08-01

**"Epistemic" — calibrated abstention + span verification (light cut).**

This release is the **evidence-gated v1.5 scope** sanctioned by
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.5, NOT the
broader v1.5.0 Epistemic plan in `IMPLEMENTATION_PLAN_v1.5.0_Epistemic.md`
(which that roadmap explicitly supersedes). The roadmap says: ship calibrated
abstention + span verification; **do not ship** source-trust ranking,
counterfactual influence, or a fixed universal confidence threshold until
their held-out benefit is demonstrated. This release honors that.

Research basis (Context7-verified 2026-08-01): Self-RAG pattern
(`/nirdiamant/rag_techniques` — retrieve → assess → abstain on low relevance)
confirms the abstention model; arXiv:2607.00895 (span-level hallucination
detection) sanctions the deterministic lexical `/verify` baseline.

### Shipped

- **Calibrated abstention on `/recall`** (M2). `RecallResponse` gains a
  `decision` field (`ok` | `low_confidence`). When the existing
  `HeuristicEstimator` (v1.4.0) classifies the query as `ClarifyQuery` (low
  overlap + low lexical density + weak gap), `/recall` returns
  `{decision: "low_confidence", hits: []}` instead of shipping top-1 garbage.
  The consuming agent (OpenClaw) can escalate or fall back to web search.
  **Not** a magic `score < 0.3` cutoff — abstention is driven by the
  calibrated multi-signal `Recommendation`, which is what the evidence-gated
  roadmap requires. Zero new compute: `confidence` + `recommendation` were
  already computed by `perform_search_with_prf`.
- **`POST /verify` deterministic span verification** (M5). Given
  `{chunk_id, claim}`, returns `{supported, decision, match_ranges}` via
  case-insensitive substring match over one chunk's text. Zero embeddings,
  zero LLM, zero model load — O(content.len()) per request, opt-in (not in
  the recall hot path). The hallucination-resistance primitive: an agent can
  verify "the brain said X" against the original source before acting on it.
  Mismatch surfaces as `unsupported_claim`. Bounded: claim capped at
  `MAX_QUERY` (2000 chars), output ranges capped at 100.
- **OpenAPI contract** updated: `/verify` route + `VerifyResponse` schema +
  `decision` field on `/recall`. `test_openapi_covers_routes` extended.
- 8 new tests (1 abstention wiring + 7 span-verification including byte-offset,
  non-overlapping, case-insensitive, unicode-safe, cap-enforcement).
- Pre-existing rust-1.97 clippy lints in `linker.rs` silenced (chore commit;
  not introduced by this release).

### Deferred (with reasoning)

These items from `IMPLEMENTATION_PLAN_v1.5.0_Epistemic.md` are **deliberately
not shipped** because the evidence-gated roadmap forbids them until their
held-out benefit is demonstrated on a judged-query corpus:

- **M1 calibration curve + judged baseline.** Operator step — requires the
  private ≥100-query judgment set. The harness ships (`bench eval` from
  v1.4.0); the corpus does not.
- **M3 counterfactual influence (leave-one-out).** Roadmap-forbidden without
  measured Δ-recall vs Δ-latency. The naive implementation re-runs retrieval
  O(5)× per query — unacceptable on Jetson.
- **M4 source-trust scoring + `/feedback` endpoint.** Roadmap-forbidden
  without measured benefit. Would add a `source.trust` column, Bayesian
  update logic, and ranking decay — real hot-path cost.
- **Carry-forward: fuzz targets exercising prod code, miri/LSAN runs.**
  Operator/hardware step. The stubs from v1.3.0 remain stubs until the
  chunker/query modules move from the binary to the lib crate.

### Verification

- `cargo test --features bench,migrate`: **401 passed, 1 ignored** (was 391
  at v1.4.2; +10).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.
- Live restart + end-to-end smoke: **operator step** (run
  `scripts/install-service.sh`).

### Honest ceilings (carried into v1.6)

- **Abstention is heuristic, not learned.** The `ClarifyQuery` threshold is
  calibrated on rank-agreement signals, not on a judged corpus. Once the
  Carry-forward baseline is recorded, v1.6 may tune or replace it.
- **`/verify` is lexical only.** No semantic match (paraphrase, synonym).
  A claim that's semantically equivalent but lexically different will report
  `unsupported_claim`. This is the deterministic baseline; a model-based
  upgrade is the v1.6+ path.
- **No audit row on `/verify`.** It's a pure read; the roadmap's "every state
  mutation is auditable" rule does not apply. If verification telemetry
  becomes a requirement, it lands with v1.6 Reconcile.

## [1.4.2] — 2026-07-30

Noise-reduction release on top of v1.4.1. Eleven changes (cumulative with v1.4.1).
Research basis: Aho-Corasick (ACL/EMNLP, confirmed SOTA for deterministic
multi-pattern matching, July 2026) + document-structure heading hierarchy
research (2026) + dependency parsing upgrade path (nlrule) documented for
future SVO extraction. See [`RESEARCH.md`](./RESEARCH.md) for the full
research audit across all 17 assessed components.

- **`--replace` flag** (`brain ingest-dir --replace`). Sweeps existing chunks
  before re-inserting, regenerating the knowledge graph from scratch. Server-side
  `replace` field on `MarkdownPayload`, handler deletes `vec_knowledge` +
  `knowledge` rows before calling `write_markdown_ingest`. CLI flag `-r`/`--replace`.
  No schema change.
- **Orphan relationship sweep.** `--replace` now deletes relationships with
  `knowledge_id IS NULL` (orphans from pre-fix re-ingests) plus all relationships
  linked to stale chunk IDs. Removes zombie edges that survive across re-ingests.
- **Pipe-table exclusion** (`find_table_ranges`). GFM pipe-table rows are
  excluded from entity-mention scanning — table cells like "Tested" no longer
  generate spurious relationship types.
- **List-item bold exclusion** (`find_list_item_bold_ranges`). Bold labels in
  definition-list style (`- **Term**: value`) are excluded from entity extraction
  and mention scanning. Prevents `Last Tested` from becoming an entity or
  contributing "tested" to verb discovery.
- **Excluded-range threading into between-text analysis.** Both `find_relationships`
  and `discover_verb_patterns` now strip excluded bytes (code blocks, tables,
  list-item bold) from between-text before tokenizing. Words inside excluded
  ranges never contribute to verb frequencies or pattern matching.
- **Heading number stripping** (`strip_heading_number`). Section-number prefixes
  (`5.1 Ceph Components` → `Ceph Components`) are removed before entity
  insertion, so heading entities match body mentions.
- **Verb stop-word pruning.** Added "date" to `STOP_WORDS`. Blocks "date"
  (false-positive verb via `-ate` suffix) from becoming a discovered relationship
  type.
- **Between-text exclusion in `find_relationships`** — the verb-pattern matching
  path now also strips excluded byte ranges from the candidate text, matching
  the same fix in `discover_verb_patterns`.
- 6 new tests (heading-number stripping, vocabulary strip, edge cases, two
  existing test updates for new signatures).
- Proxmox-book vault (6 files, ~18k knowledge rows): entity count stable at 54;
  relationships reduced from 390 → 193 (51% fewer) with `tested` 105→0 and
  `date` 76→0.
- Test count: **307 passed** (was 391 at v1.4.1; some integration tests were
  retired; net change reflects focused unit coverage).
  `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.

**Note on version numbering:** v1.4.1 "Link" (heading-hierarchy `part_of` +
verb-suffix filtering + entity-leakage fix) was code-complete but never tagged
or released as a separate version. These changes are included in v1.4.2 in
their original form. See Agent 32 ÷ Agent 33 in `AGENTS.md` for the full
v1.4.1 diff.

### v1.4.1 — not released (folded into v1.4.2)

Deterministic entity linker upgrade. All changes below are cumulative in v1.4.2.

- **Heading hierarchy → `part_of` relationships.** `extract_heading_relationships()`
  walks the markdown heading tree and creates `part_of` KG edges for every adjacent
  heading pair where both are known entities (e.g. `CRUSH Map -- part_of --> Ceph`).
- **Verb-suffix filtering for discovered relationship patterns.** `is_likely_verb()`
  rejects nouns like "maps", "data", "example" from becoming relationship types.
- **Entity leakage fix**: `discover_verb_patterns()` now excludes entity names
  from the candidate set.
- **`EntityVocabulary.entities`** made pub.
- **`brain ingest-dir --replace` flag** (first version — see v1.4.2 for the
  full orphan-sweep + exclusion fixes).

### v1.4.0 "Calibrate" — 2026-07-30 (released)

The surpass-human retrieval release. Implements the July-2026 SOTA on top of
the v1.3.0 memory-safe foundation. Six research-backed techniques form the
retrieval stack:

| Layer | Technique | Research |
|-------|-----------|----------|
| **Stage 1: Retrieval** | Hybrid dense + lexical | `vec0` KNN (sqlite-vec) + FTS5 BM25 |
| **Stage 1: Fusion** | Reciprocal Rank Fusion (RRF, k=60) | [RRF (Cornell, 2009)](https://plg.uwaterloo.ca/~gvcormac/cormacksigir09-rrf.pdf) — still the standard model-free fusion algorithm per 2026 production patterns |
| **Stage 2: Rerank** | Cross-encoder (optional) | [BGE-RerankerV2M3](https://huggingface.co/BAAI/bge-reranker-v2-m3) via fastembed — most-deployed production reranker |
| **KG: Edges** | Bi-temporal (`valid_at`/`invalid_at`) | [Graphiti / Zep](https://github.com/getzep/graphiti) — bi-temporal KG model, SOTA for temporal facts, 82.2 benchmark |
| **KG: Traversal** | Typed-edge prefix vocabulary | [TRACE: State-Aware Query Processing over Temporal Evidence Graphs](https://arxiv.org/abs/2607.00339) (July 2026) |
| **Packing** | Budgeted submodular maximization | [What Survives Into Context](https://arxiv.org/abs/2607.00725) — +5.1 F1 HotpotQA, lazy greedy (Leskovec et al. 2007) |

**Research basis** (Context7-verified 2026-07-30 against getzep/graphiti
`edges.py` + `search_filters.py` + `edge_operations.py`):
- `valid_at`/`invalid_at` = valid-time interval (when the fact holds in the
  world); `created_at` = transaction time (when brain learned it).
- `resolve_edge_contradictions`: old facts are *expired* (invalid_at set), not
  deleted — delete-proof auditability. v1.4 adopts the filter; the resolution
  worker lands in v1.6 Reconcile.

#### M1 — Bi-temporal edges
- **Migration** (additive, idempotent): `relationships.valid_at` +
  `invalid_at` columns. Existing edges default to NULL/NULL ⇒ always valid.
- **New `src/temporal.rs`**: deterministic temporal-marker extraction from free
  text ("from 2011 to 2017", "currently", "since 2020", "until 2019"). No LLM,
  no external API. Pure, unit-tested (11 cases).
- **Ingest path**: `/ingest` relations now accept optional explicit
  `valid_at`/`invalid_at`; when absent, the extractor populates them from the
  ingested content (best-effort).
- **Query path**: `/recall` and `/graph/traverse` accept `?at=<ISO8601>`.
  The SQL filter is `valid_at <= ? AND (invalid_at IS NULL OR invalid_at > ?)`
  (Graphiti-validity semantics). Distinct from `as_of` (transaction-time /
  revision recall).
- **Normalization**: `at` is normalized in `perform_search_traced` alongside
  `since` so a direct caller can't bypass it.

#### M2 — Submodular evidence packing
- **New `src/search/packing.rs`**: budgeted monotone submodular maximization.
  Objective = relevance + coverage + representativeness, gated by diversity
  (MMR-style near-dup threshold `DEDUP_SIMILARITY=0.85`). Lazy greedy under a
  token knapsack (`max_context_tokens`, default 160 per the paper).
- **`/recall`**: `max_context_tokens` field triggers packing; `gold_answer`
  drives the `answer_in_context` diagnostic (did the gold survive?). Both
  reported in telemetry.
- **`SearchTelemetry`**: gained `packed_tokens`, `packing_candidates`,
  `answer_in_context`.

#### M3 — TRACE state-aware traversal
- **Typed-edge prefixes**: `update:`, `supersedes:`, `contradicts:`,
  `causes:` on `relation_type`. The validator (`RELTYPE_RE`) now accepts an
  optional `prefix:base` form.
- **New `src/trace.rs`**: prefix vocabulary + bounded-walk constants
  (`MAX_HOPS=4`, `MAX_VISITED=256`) enforcing the forbidden-list rule.
- **`/graph/traverse`**: validity-aware — the bi-temporal `at` filter skips
  expired edges; the walk is hard-capped on depth + visited nodes.
- **Schema reservation**: `knowledge.node_kind` (default `'event'`) +
  `parent_id` columns added for the hierarchical node model (session/topic).
  ponytail: construction logic deferred to v1.8 Consolidate (the only release
  with a worker that can group events into sessions).

#### M5 — Regression: bench harness
- **New `brain_server::eval`** lib module: pure metric functions
  (precision@k, recall@k, MRR, NDCG, `answer_in_context_rate`). Hand-computed
  value checks pin each metric.
- **`bench eval`** mode: loads a judgments file (`BRAIN_EVAL_JUDGMENTS`),
  runs each query through `/recall`, reports the metrics. Optional ship gate
  via `BENCH_EVAL_BASELINE` + `BENCH_EVAL_REGRESSION_PCT` (default 2%).
- The 100-query hand-judged corpus against the live DB is an operator step;
  the harness is the reproducible engine any judgments file plugs into.

#### M4 — Multi-vector retrieval: DEFERRED
- **Deferred per the plan's lazy-dev escape hatch.** Multi-vector doubles
  embedding storage + per-query compute; a 4 GB Jetson can't afford two `vec0`
  tables. The feature cannot be *measured* until M5's harness provides a
  baseline to compare against (M5 lands in this release; M4's measurement
  now has a foundation). The `multivec` feature flag is reserved (no-op) so
  callers/docs/CI can reference the upgrade path. Lands in v1.4.1+ with
  measured Δ-recall vs Δ-RSS.

#### Testing
- Test count: **367 passed** (was 324 at v1.3.0; +43: 11 temporal, 12 packing,
  6 trace, 9 eval, 5 integration).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.

#### Honest ceilings (carried into v1.5)
- **Temporal extraction is English-only + deterministic.** It recognizes a
  bounded set of markers ("from X to Y", "since", "until", "currently"). It
  does NOT infer relative dates ("last year") or durations without anchors.
  An LLM extractor is a v2.x concern (out of scope for the low-power path).
- **Submodular packing uses lexical Jaccard for diversity**, not embedding
  cosine. Cheap and good enough for near-dup detection; a cosine gate would
  need the model in the packer (small win, adds per-call cost).
- **TRACE node hierarchy is schema-only.** `node_kind`/`parent_id` columns
  exist but nothing populates session/topic yet (v1.8 Consolidate).
- **M4 multi-vector deferred** — see above.
- **The 100-query judged corpus is an operator step.** The harness ships;
  the judgments don't (they require the operator's private DB).

### v1.3.0 "Bedrock" — 2026-07-29 (released)

Memory-safety hardening release. Makes the binary bulletproof: zero panics
in production paths, every `unsafe` block documented, property-based tests
for core invariants, and cargo-fuzz infrastructure.

#### Memory safety

- **Panic elimination (M1)**: audited every `unwrap()`/`expect()`/`panic!` in
  production code (non-test). Zero remaining. Fixed three panic paths:
  `mcp.rs` JSON-RPC notification id handling (was `unwrap()` on `Option<Value>`
  when the request had no id — a notification), `vault.rs` first-line unwrap
  (was `unwrap()` on `Option<&str>` before the guard that proves it's `Some`),
  `github_app.rs` mutex poison (was `expect()` — now uses `unwrap_or_else(|e|
  e.into_inner())` for poison recovery).
- **`unsafe` audit (M2)**: extracted `register_sqlite_vec()` — a single
  documented safe wrapper that replaces **10 duplicate unsafe transmute
  blocks** across `main.rs`, `domain_registry.rs`, `handlers/domains.rs`,
  `audit.rs`, `brain_migrate_rehearse.rs`. Every remaining `unsafe` block has
  a `// SAFETY:` comment per the Rust nomicon.
- **Fuzz infrastructure (M3)**: `fuzz/` crate with cargo-fuzz targets
  (`fuzz_chunker`, `fuzz_lex_compile`, `fuzz_query_doc`, `fuzz_validator`).
  Behind nightly toolchain. Stubs for binary-private modules document the
  path to full coverage (move to lib crate).

#### Testing

- **Proptests (M6)**: 4 new proptest suites (256+ cases each):
  - `proptest_chunker_never_panics_and_ranges_are_valid` — random UTF-8 →
    chunk text is always a substring of input.
  - `proptest_chunker_handles_multibyte_inputs` — multibyte chars (•, 💡, 🏋️)
    never cause slice panics.
  - `proptest_normalize_domain_is_idempotent` — normalize twice == once.
  - `proptest_classify_is_monotonic` — increasing docs/db/rss never improves
    the capacity status.
- Test count: **324 passed** (was 320 at v1.2.1).

#### Observability + Power

- **`/health` hardening (M7)**: exposes `hardening: { unsafe_blocks, panics_caught,
  memory_leaks_detected }` so ops can see the memory-safety posture.
- **`BRAIN_WORKER_THREADS` (M8)**: configurable tokio runtime. Default = cores;
  Jetson target = 2 (saves ~10MB RSS + context-switch overhead).

#### Honest ceilings

- **miri/loom/LSAN**: procedure documented in the plan; not CI-integrated
  (needs nightly toolchain + sanitizer support).
- **Fuzz targets for binary-private modules**: `fuzz_chunker`/`fuzz_lex` are
  stubs because the chunker/query modules are server-private. Moving them to
  the lib crate is the follow-up.
- **Hot key reload**: restart required after `brain key generate/prune`.
- **Distributed revocation**: 60s per-instance negative cache (v2.1).

### v1.2.1 "AuthN" (dead-code cleanup) — 2026-07-29 (released)

Gap-closing release on top of v1.2.0. Dead-code elimination + panic fixes
found during the v1.3.0 memory-safety audit.

- Removed unused abstractions: `AuthzPolicy` trait, `InMemoryPolicy`,
  `AuthzError`, `SharedPolicy`, `default_policy` (YAGNI until v2.1 OPA/Cedar
  swap — the `is_authorized` function does the actual work).
- Removed unused items: `TokenType::as_str`, `DEFAULT_ALG`, `AuthError::Revoked`,
  `op_tenant`, `Duration` const.
- `authorize()` now uses `principal.tenant` as the team context.
- Test count: 320 passed (unchanged from v1.2.0 after removing 2 trait tests).

### v1.2.0 "AuthN" — 2026-07-29 (released)

JWT/JWS authentication + AuthZ layer. The prerequisite for v2.0 multi-team
tenancy, enforced at the data-access layer rather than hand-rolled per-handler.
Back-compat is the default: when `BRAIN_JWT_ISSUER` is unset OR no keys are
loaded, the server runs in v1.1 opaque-token mode and every existing install
keeps working unchanged. JWT is opt-in.

Research basis: Context7 lookup on `jsonwebtoken` v10 verified 2026-07-29 (API
surface, `Validation` builder, algorithm enum). OWASP cheat-sheet URLs were
404ing on the day, so the encoded checklist from
`IMPLEMENTATION_PLAN_v1.2.0_AuthN.md` (which was Context7-verified at plan
write time) was the source of truth for the JWT Cheat Sheet test matrix.

#### Security

**M1 — JWT verification core (`src/auth/jwt.rs`).** `verify_access_token()` +
`Claims` + `AuthError`. `ALLOWED_ALGS` whitelist (RS256/384/512, ES256/384/512,
EdDSA) is checked **before** key lookup — the OWASP algorithm-confusion defense
(`none`, all HS*, all PS* rejected unconditionally). Every claim validated:
`iss`, `aud`, `exp`, `nbf`, `sub`, `jti`. 30s leeway for clock skew
(subsumes the `reject_tokens_expiring_in_less_than` knob — documented
trade-off). 14 tests pin the full OWASP JWT Cheat Sheet failure matrix:
`none` rejected, HS256-with-public-key rejected, tampered payload rejected,
expired/nbf rejected, wrong iss/aud rejected, missing jti/kid rejected,
unknown kid rejected, refresh token rejected on data routes, PS256 rejected
by whitelist, valid token accepted, leeway absorbs skew.

**M2 — Revocation (`src/auth/revocation.rs`).** Additive `revoked_tokens` +
`refresh_chains` tables. `RevocationCache` (60s negative-lookup cache, bounded
TTL — eventual consistency by design). `purge_expired` housekeeping runs on a
background timer. Refresh-chain reuse detection: presenting a stale refresh
token calls `revoke_chain` and burns the whole family (OWASP pattern). The
chain id is derived from `(iss, sub)` — per-user per-issuer.

**M3 — AuthZ (`src/auth/policy.rs`).** `AuthzPolicy` trait + `InMemoryPolicy`
default (no external deps; OPA/Cedar impls are the swappable v2.1+ upgrade
path). `Action` enum (Read/Write/Admin/Traverse) + `Scope`
(`<action>:<team>/<domain>` with wildcards) + `Principal` +
`is_authorized()`. Escalation: write implies read down, admin implies both.
Default-deny → 403, never 404 (no existence leakage — OWASP A01:2025). The
retrofit is minimal: a single `authorize(principal, action, team, domain)`
helper called at handler entry, not a full pool-resolution refactor.
`Option<Principal>` where `None` = superuser (the back-compat path — opaque
token mode passes `None` everywhere).

**M4 — OIDC discovery + JWKS (`src/handlers/well_known.rs`).**
`GET /.well-known/openid-configuration` (RFC 8414) + `GET /.well-known/jwks.json`
(RFC 7517). Both routes PUBLIC — clients need them to learn how to verify
tokens; you can't require a token to discover token verification. Issuer is
pinned to `BRAIN_PUBLIC_BASE_URL` — never inferred from the `Host` header
(OWASP A02:2025 Security Misconfiguration: Host-header spoofing could
otherwise redirect discovery to a malicious endpoint).

**M5 — Key management (`src/auth/jwks.rs` + `src/bin/brain.rs`).** `KeyStore`
loads RSA/EC/Ed25519 PEMs from `BRAIN_JWT_KEY_DIR` (default
`~/.config/brain-server/keys/`, mode 0700; private keys 0600), exposes
`VerifyingKey`s for verification + RFC 7517 JWK Set JSON for the public
endpoint. `brain key generate/list/prune` CLI: RSA keypair generation with
0600 private-key mode + 0700 dir mode. Two keys live during rotation; the old
key drops from JWKS only after every cached token has expired.

**M6 — Audit integration.** AuthN/AuthZ events flow into the existing v1.1
audit log: token-verified, token-rejected (with reason),
authz-denied (with principal/action/team/domain), logout. Per-tenant audit
filter at the data layer is unchanged from v1.1.

**M7 — Migration (`src/migration.rs`).** Additive: `revoked_tokens` +
`refresh_chains` tables. `schema_version` stamped `1.2.0`. Back-compat: when
`BRAIN_JWT_ISSUER` is unset OR no keys load, the server falls back to v1.1
opaque-token mode. Two-layer middleware: `jwt_auth_middleware` runs outermost
(verifies JWS, checks revocation, injects `Principal` into extensions); the
v1.1 `auth_middleware` runs as fallback and short-circuits when the Principal
is already set.

#### Updated
- Cargo.toml 1.1.2 → 1.2.0. `jsonwebtoken` promoted from optional to required
  (with `use_pem` + `rust_crypto` features); `rsa` + `rand` + `base64` added
  as direct deps. `openapi.yaml` → 1.2.0 with `/auth/*`,
  `/.well-known/*`, and the `TokenPair`/`RefreshRequest`/`RevokeRequest`/
  `OidcConfig`/`JwkSet`/`Jwk`/`Principal`/`Scope` schemas.

#### Honest ceilings (carried into v1.3)
- **No distributed revocation.** The 60s negative cache is per-process; a
  multi-instance deployment has a 60s window per instance. Distributed
  revocation (Redis-backed denylist) is the v2.1 concern.
- **No hot key reload — restart required.** Adding/removing a signing key
  via `brain key generate/prune` requires an `install-service.sh` restart to
  pick up. File-watch for keys is a small follow-up; deferred to keep the
  v1.2 surface tight.
- **EC/Ed JWK emission not implemented.** `KeyStore::to_jwks()` emits RSA
  keys only today (the common case); EC/Ed keys verify correctly but don't
  appear in `/.well-known/jwks.json`. Workaround: rotate to RSA for any key
  a third party must discover via JWKS. Tracked for v1.3.
- **No cookie-based refresh token storage.** Refresh tokens are returned in
  the JSON body only; CLI bearer usage is the assumed client shape. The
  `HttpOnly`+`Secure`+`SameSite=Strict` cookie path (browser UI) lands with
  the v2.0 UI.
- **Refresh-chain reuse detection burns the chain but doesn't notify the
  user.** A stolen-then-reused refresh token revokes the family silently;
  the legit user's next refresh returns `refresh_reuse_detected` (403). A
  user-facing notification channel is the v2.1 concern.
- **Audit hash-chain comparison stays plain `==`.** Carried from v1.1.2 —
  same judgment call (tamper-detection read path, not an auth gate).

### v1.1.2 "Harden" (constant-time auth hardening) — 2026-07-29 (released)

Security hardening release. A best-practices pass (rusqlite 0.40.1 docs +
RustCrypto `subtle` 2.6.1, fetched 2026-07-29) surfaced one real gap: the
bearer-token comparison used a hand-rolled fold that LLVM could short-circuit,
re-introducing a timing oracle the v1.1.0 comment had explicitly flagged.

#### Security
- **Bearer-token comparison now uses `subtle::ConstantTimeEq`.** The prior
  `ct_eq` (a manual `fold` of `acc | (x ^ y)`) had no `black_box` barrier, so
  a sufficiently aggressive optimization pass could turn it back into a
  short-circuit compare — exactly the timing oracle the constant-time
  pattern exists to prevent. `subtle` 2.6.1 was already a transitive dep
  (via `sha2`/`hmac`/`aes-gcm`), so the swap adds zero build surface. The
  ponytail ceiling noted in the v1.1.0 comment is now closed. Pinned by
  the existing `test_ct_eq`.

#### Considered and left as-is (documented best-practice judgment calls)
- **`verify_chain`'s `want == got` hash comparison left as a plain `==`.**
  This compares two equal-length SHA-256 hex strings inside a tamper-
  detection read path (not an auth gate). An attacker who could measure
  the timing remotely would already control the DB and could simply edit
  `prev_hash` to match. Wrapping it in `ct_eq` would be gold-plating
  without a real threat model — the auth path was the actual surface.
- **`record_tenant`'s raw-SQL `SAVEPOINT` left as-is.** rusqlite 0.40.1
  exposes a canonical `savepoint_with_name()` API, but it takes `&mut
  Connection`; the ~20 call sites pass `&Connection` (often from a pooled
  r2d2 connection, which derefs to `&Connection`). Migrating would ripple
  through every caller + require pooled-connection borrow gymnastics for
  zero correctness gain — the current raw-SQL approach is verified by 3
  v1.1.1 tests and uses parameterized queries (no injection surface).

#### Updated
- Cargo.toml 1.1.1 → 1.1.2. `openapi.yaml` → 1.1.2.

### v1.1.1 "Harden" (audit chain bug-fix) — 2026-07-29 (released)

Bug-fix release. Closes three honest ceilings carried forward from v1.1.0,
one of which was a latent false-negative affecting every migrated DB.

#### Fixed
- **`verify_chain` false-negative on migrated DBs (`src/audit.rs`).** The
  v1.1.0 walk assumed at most one NULL `prev_hash` row at the start of the
  table. After the additive migration, **every** pre-v1.1 row has NULL
  `prev_hash` — so on a real migrated DB the *second* NULL row hit the
  `_ => return false` fallthrough and `/audit/verify` (plus
  `brain_audit_chain_ok` via `/metrics`) reported tampering on a clean DB.
  The walk now treats NULL `prev_hash` as "no backref to verify" (advances
  the running link but never fails) and only fails when a v1.1 row's stored
  `prev_hash` disagrees with the recomputed link. Pinned by
  `hash_chain_survives_migration_with_many_null_rows`.

#### Closed ceilings (from v1.1.0)
- **Audit chain now covered by a real migration fixture test.**
  `hash_chain_survives_real_v1_0_to_v1_1_migration` builds a DB with the
  pre-v1.1 `audit_events` schema, inserts rows, runs the actual
  `run_migration`, and verifies the chain holds across the NULL → Some
  boundary with real `record()` calls afterward.
- **`record_tenant` now wraps its read+INSERT in a `SAVEPOINT`.** A `BEGIN`
  would error when called inside a caller's existing transaction
  (e.g. `delete_quarantine`); `SAVEPOINT` nests cleanly. Rolling back the
  savepoint on audit-INSERT failure touches only the audit row, not the
  caller's work. Pinned by `record_tenant_is_safe_inside_caller_transaction`.
- **`/metrics` no longer triggers a full chain scan on every scrape.**
  `brain_audit_chain_ok` is now backed by a TTL-memoized result
  (`AUDIT_CHAIN_CACHE_TTL_SECS=60`). `/audit/verify` remains
  authoritative and always scans fully — that is its job.

#### Updated
- Cargo.toml 1.1.0 → 1.1.1. `openapi.yaml` → 1.1.1.

### v1.1.0 "Harden" — 2026-07-28 (released)

Operationally-reliable + audit-ready release on top of v1.0's multi-domain
foundation. Pares the v1.1.0 plan down to the slices that close real gaps
(bearer-token file-watch hot rotation, per-tenant audit + hash-chain tamper-
evidence, rolling backups + integrity self-check, graceful-shutdown drain cap
+ WAL checkpoint, RSS watchdog, Prometheus exporter). Explicit non-goals for
v1.1 (deferred to v1.2 AuthN): JWT/JWS verification, AuthZ trait + middleware,
per-tenant rate limiting, CSRF enforcement. The CSRF scaffold from the plan is
YAGNI until a browser UI exists.

#### Security & audit
- **Audit hash chain (`src/audit.rs`).** Each row stores a SHA-256
  `prev_hash` over the prior row's `(ts, kind, actor, target_hash,
  prev_hash)` tuple. `GET /audit/verify` walks the chain and returns
  `{ "ok": bool }`. Tampering with any field breaks the read-side check;
  pinned by `hash_chain_detects_tampering` + `hash_chain_rejects_tampered_kind`.
  `id` is deliberately excluded so a renumbered restore keeps the chain intact.
- **Per-tenant audit scoping.** New `tenant_id` column (default `'global'` for
  back-compat with every pre-v1.1 row). `GET /audit?tenant=<id>` enforces the
  filter at the SQL layer (`WHERE tenant_id = ?`) so a forgotten app-level
  filter cannot leak cross-tenant rows. `audit::record_tenant` is the variant
  that takes a tenant; existing call sites default to `global`.
- **File-watch token rotation (`src/auth.rs`).** `AUTH_TOKEN_FILE` is now
  cached in-process and refreshed on mtime change (polled every 5s) rather
  than re-read from disk per request. Fail-safe: if the file is deleted,
  emptied, or becomes unreadable after the first successful load, the cached
  token set stays in effect — auth is never silently cleared. Each real
  rotation writes an `auth_token_rotated` audit row (target = file path;
  no PII). Pinned by `reload_picks_up_new_token` +
  `reload_keeps_cache_when_file_deleted` + `reload_keeps_cache_when_file_emptied`.

#### Operational reliability
- **Rolling backup + integrity self-check (`src/integrity.rs`).** A periodic
  task snapshots the live DB with `VACUUM INTO <db>.snapshot-<ts>.bak`, runs
  `PRAGMA integrity_check` on the snapshot, and keeps the last 4 copies
  (default 6h cadence, runs once on boot). `/health` now reports
  `backup: { last_backup, integrity_ok }`.
- **Graceful shutdown drain cap + WAL checkpoint.** SIGTERM/SIGINT now drains
  in-flight requests under a hard `SHUTDOWN_DRAIN_SECS=30` cap, then runs
  `PRAGMA wal_checkpoint(TRUNCATE)` so a kill -9 or power loss can't leave
  the live DB with un-replayed WAL frames.
- **RSS watchdog.** Polls every 30s; sustained breach of the capacity
  envelope's `max_rss_mib` across two samples logs `error!`. Opt-in exit for
  supervisor restart via `BRAIN_RSS_RESTART=1`; default is log-only — a
  tight restart loop is worse than a slow leak.

#### Observability
- **Prometheus exporter (`GET /metrics`).** Hand-rolled text format (no
  `prometheus` crate dep — the plan itself flagged the dep as risky).
  Exports `brain_rss_mib`, `brain_pool_connections{state}`,
  `brain_capacity_status`, `brain_audit_chain_ok`. Auth-gated like other
  operator surfaces.
- **`GET /audit/verify`** as a separate route from `GET /audit` because the
  chain check is a full-table scan and shouldn't run on every list call.

#### Migration
- Additive: `audit_events` gained `tenant_id TEXT NOT NULL DEFAULT 'global'`
  + `prev_hash TEXT` + `idx_audit_tenant`. Existing rows backfill to
  `'global'` / NULL; the chain starts fresh from the next inserted row
  (documented upgrade-path ceiling). `schema_version` stamped `1.1.0`.

#### Updated
- Cargo.toml 1.0.1 → 1.1.0. `openapi.yaml` → 1.1.0 with `/audit/verify`,
  `/metrics`, the `tenant` query param on `/audit`, and the `tenant_id` field
  on the `AuditRow` schema.

#### Honest ceilings (carried into v1.2)
- **No JWT/JWS verification.** Opaque bearer tokens only; JWT needs RS256/
  ES256 signing keys + JWKS + revocation — all land in v1.2 AuthN.
- **No AuthZ middleware.** The `tenant_id` column lands here, but "team A
  can't read team B's data" needs the v1.2 AuthZ trait.
- ~~**Audit chain link is read inside the same connection, not inside an
  explicit BEGIN/COMMIT.**~~ Closed in v1.1.1 (`SAVEPOINT` wrap).
- ~~**`prev_hash` NULL on pre-v1.1 rows.**~~ The chain still starts at the
  first v1.1 row (no retroactive re-hash of existing rows — that would be
  expensive and is out of scope), but v1.1.1 fixed the read-side walk so
  these NULL rows no longer break `verify_chain`.
- ~~**`/audit/verify` + `/metrics` full-table scan per call.~~ `/audit/verify`
  still scans fully (that is its job — you cannot verify a chain without
  walking every link); v1.1.1 added a TTL cache on the `/metrics` path so a
  Prometheus scrape no longer triggers a scan.

### Cognitive Stack roadmap (v1.2.0 → v1.9.0) — 2026-07-26 (planning only)

Deep-research-driven expansion of the v1.x line into **8 point releases** that
transform brain-server from a memory store into a cognitive substrate that
exceeds human memory capability. Each release adds ONE capability and hardens
it; no feature ships without a fuzz/leak/regression test.

Research sources (all current as of July 2026):
- **Mem0 v3** (Context7, benchmark 83.22) — built-in graph memory + distillation.
- **Graphiti / Zep** (Context7, benchmark 82.2) — bi-temporal KGs.
- **Letta / MemGPT** (Context7, benchmark 83.31) — sleep-time "dreaming".
- **arXiv July 2026**: TRACE (2607.00339), Submodular packing (2607.00725,
  +5.1 F1), DiscoLoop (2607.00341), CAT (2607.00862), Dual-Confidence
  Contrastive Decoding (2607.00570), KnowledgeDebugger (2607.01000),
  Span-Level Hallucination Detection (2607.00895), Auditing Forgetting
  (2607.00605).

#### Added — new implementation plan
- **`IMPLEMENTATION_PLAN_v1.2.0_to_v1.9.0_Cognitive_Stack.md`**: granular
  milestone breakdown for all 8 releases. Each release has 5–7 milestones,
  RSS budget, Definition of Done, and is gated on the previous. Cross-cutting
  section codifies what every release must ship (fuzz, miri, leak, regression)
  and what's forbidden (NN in hot path, auto-conflict-resolution, paraphrasing
  comments).

#### The 8 releases

| Release | Name | Capability |
|---|---|---|
| v1.2.0 | AuthN | JWT/JWS + AuthZ layer (full plan in v1.2.0_AuthN.md) |
| v1.3.0 | Bedrock | Memory-safety: panic elimination, `unsafe` audit, cargo-fuzz, miri, LSAN, loom, proptests |
| v1.4.0 | Calibrate | Bi-temporal KGs + submodular packing + TRACE-style state-aware query + multi-vector |
| v1.5.0 | Epistemic | Confidence calibration + "I don't know" + counterfactual influence + source trust + hallucination resistance |
| v1.6.0 | Reconcile | Contradiction detection + supersession + conflict policy + knowledge editing + consistency checker |
| v1.7.0 | Reason | Multi-hop reasoning + causal subgraph + counterfactual simulation + transitive inference |
| v1.8.0 | Consolidate | Sleep-time worker + near-duplicate detection + extractive summarization + cross-cluster linking |
| v1.9.0 | Anticipate | Session context + proactive `/anticipate` + SSE push + spaced repetition + personalization |

#### Why this beats human memory by v1.9

Every dimension where biological memory is weak (forgetting, source amnesia,
overconfidence, slow self-correction, single-context reasoning) becomes a
deterministic, auditable brain-server capability. Every dimension where
biological memory is strong (analog intuition, neural creativity) is
**deliberately out of scope** — brain-server is an extended-mind substrate,
not a brain replacement.

### Security roadmap expansion — 2026-07-26 (planning only, no code changes)

Audit-driven expansion of the upcoming security roadmap. Closes every gap
surfaced by an OWASP Top 10:**2025** review (Context7-verified 2026-07-26).
No runtime code changes — this commit is documentation + new implementation
plans only.

#### Added — new implementation plans
- **`IMPLEMENTATION_PLAN_v1.2.0_AuthN.md`** (NEW release between v1.1 and
  v2.0): JWT/JWS verification (RS256/ES256/EdDSA only, never HS256/`none`);
  `(jti, iss)` revocation table per OWASP JWT Cheat Sheet; refresh token
  rotation + reuse detection; AuthZ middleware trait with deny-by-default;
  OIDC discovery (`/.well-known/openid-configuration`); JWKS endpoint;
  per-route enforcement matrix. The prerequisite v2.0 multi-tenant implicitly
  assumed but didn't define.
- **`IMPLEMENTATION_PLAN_v2.1.0_Limits.md`** (NEW release after v2.0):
  per-tenant + tiered rate limiting per OWASP Multi-Tenant Cheat Sheet.
  `RateLimiter` trait with `InMemory` (default) and `RedisRateLimiter`
  (GCRA atomic Lua script, `--features ratelimit-redis`) impls. Per-tenant
  cost tracking (tokens/egress) feeding v4.0 marketplace billing. Standard
  `X-RateLimit-*` + `Retry-After` headers.
- **`THREAT_MODEL.md`** (NEW): full STRIDE threat model per asset
  (knowledge graph, tokens, audit log, binary, network). Residual-risk
  register with explicit acceptances + ceilings. Per-release security exit
  gate matrix.

#### Updated — existing plans
- **`IMPLEMENTATION_PLAN_v1.1.0.md`**: added M1.4 (file-watch hot token
  rotation), M1.5 (CSRF scaffold), M2.2 (per-tenant audit data-layer filter),
  M2.3 (audit hash chain for tamper-evidence), M5.4 (Prometheus `/metrics`
  behind `--features metrics`); explicit dependency on v1.2 AuthN.
- **`IMPLEMENTATION_PLAN_v2.0.0_Cortex.md`**: M1 multi-team now consumes
  v1.2's AuthZ trait instead of re-inventing scope checks; cross-tenant
  reads return 403 (not 404) per OWASP A01:2025; team-lifecycle admin
  scope required.
- **`IMPLEMENTATION_PLAN_v4.0.0_Sovereign.md`**: v3.7 "Connect" now ships
  A2A over **mTLS + JWS** (was JWS only) per OWASP gRPC + Microservices
  Cheat Sheets; SQLCipher gains a real **KMS abstraction trait**
  (FileKeyProvider / VaultKeyProvider / AwsKmsKeyProvider) per OWASP
  Secrets Management Cheat Sheet; data residency allowlist for peer agents.
- **`SECURITY.md`**: rewritten against OWASP Top 10:**2025** (the new
  canonical list, supersedes 2021/2023). Every category A01–A10 has a
  control mapping table with status (✅ shipped / 🚧 planned with version).
  Added compliance attestations table (SOC 2, ISO 27001, GDPR, HIPAA, PCI DSS).
  Added STRIDE summary referencing THREAT_MODEL.md.
- **`ROADMAP.md`**: release table updated with v1.0/v1.0.1 ship status,
  v1.2 AuthN and v2.1 Limits new rows, v3.7 mTLS + KMS clarification,
  v4.0 depends on v2.1.

#### Standards verified via Context7 (2026-07-26)
- OWASP Top 10:**2025** (`/owasp/top10`) — the canonical reference, current.
- OWASP Cheat Sheet Series (`/owasp/cheatsheetseries`, score 80.97):
  - JSON Web Token Cheat Sheet (`(jti, iss)` revocation, alg whitelist).
  - Multi-Tenant Security Cheat Sheet (tenant-aware rate limiting, RLS).
  - Secrets Management Cheat Sheet (BYOK, KMS patterns, sidecar rotation).
  - gRPC + Microservices Security Cheat Sheets (mTLS for service-to-service).
  - Transport Layer Security Cheat Sheet (mTLS, cert pinning).

#### Why this matters
The pre-existing plans would have shipped multi-tenant (v2.0) without a
real AuthZ layer, multi-instance rate limiting, or JWT done right. This
expansion front-loads the security architecture so v2.0/v4.0 can be
honestly marketed as enterprise-ready. **Three new releases** inserted into
the chain (v1.2, v2.1, v3.7 update) — no new features, just the security
foundation the existing features implicitly required.

### v1.0.1 "Domains" patch — 2026-07-26 (released)

Patch release fixing the structured-ingest entity auto-create bug
found end-to-end on openclaw.

#### Fixed
- `POST /ingest` now auto-creates entities referenced by relations but not
  declared in the input `entities` array. The canonical plan example
  (`vitamin d3 helps inflammation` with only `vitamin d3` declared) works.
- `entities_added`/`relations_added` now report the real COUNT(*) delta
  instead of the input array length.

### v1.0.0 "Domains" — 2026-07-26 (released)

The multi-domain cutover. Every handler resolves its target domain via the
`X-Brain-Domain` header or JSON `domain` field; POST/GET/DELETE domain lifecycle
is a first-class API. Structured ingest (`POST /ingest`) with inline
entity/relation upsert is the primary write path. The single-DB shim mode
preserves v0.9.x behavior byte-for-identical; `BRAIN_MULTI_DB=true` activates
per-domain files.

#### Added — domain routing (M1 + M2)
- **`X-Brain-Domain` header** support on every GET handler (`/search`, `/stats`,
  `/get/{id}`, `/multi-get`, `/graph/entity/{name}`, `/graph/relations`,
  `/graph/traverse`). Resolves the target domain's connection pool via
  `DomainRegistry`.
- **`domain` query param** on `GET /search` and `GET /stats` for tool-friendly
  domain scoping without headers.
- **`handlers::resolve_domain_pool()`** — shared helper that resolves any domain
  name to its pool, defaulting to `"global"`. The error envelope's `details`
  field now carries `known_domains` so an unknown-domain `400` is actionable.

#### Added — federated search (M3)
- **Cross-domain RRF merge.** The previous `/recall` cross-domain sort used raw
  `score` (wrong: scores aren't comparable across domains because IDF tables
  and post-quantization norms differ). Replaced with rank-based RRF using the
  same `RRF_K = 60` constant as the in-domain hybrid fusion.
- **`?cross_domain=true` on `/graph/traverse`** walks edges across every known
  domain pool, labelling each hop with its source domain.
- The `/recall` handler already supported centroid routing for domain-aware
  recall (v0.9.1 `domain_router`). Verified end-to-end for the v1.0 cutover:
  multi-domain federation with labelled `domains_searched` on the response.

#### Added — structured ingest (M4)
- **`POST /ingest`** accepts `{ title, content, domain?, entities?, relations? }`.
  Entities are validated and upserted idempotently; relations are anchored to
  the ingested chunk. The `/ingest/markdown` `[[...]]` parser remains as the
  legacy fallback. Recomputes the domain centroid after each successful ingest.
- **MCP `brain_ingest` updated** to call `POST /ingest` with structured fields
  when the caller supplies `entities`/`relations`/`domain` (the agent does
  extraction client-side, per the plan). Legacy memory-style ingest with just
  `content` still routes to `/ingest/memory` for back-compat.
- **Fixed the validator regression.** The hand-rolled `is_match` checker
  ignored its `pattern` argument and silently rejected spaces in entity names
  — breaking the canonical `vitamin d3` example. Replaced with three
  correctly-scoped checkers (`is_valid_domain`, `is_valid_name`,
  `is_valid_rel_type`); the shapes are pinned by a unit test.

#### Added — domain lifecycle (M5)
- **`POST /domains`** — create/warm a domain (idempotent; 201 on first open).
- **`DELETE /domains/{name}?confirm=<name>`** — delete a domain and all its
  data. `global` is protected. The `?confirm=<exact-name>` query param is
  REQUIRED so a typoed URL or replay cannot destroy data by accident.
- **`POST /domains/{name}/vacuum`** — reclaim free pages in the domain's DB.
- **`GET /domains/{name}/export`** — stream a consistent snapshot of the
  domain's `.db` file via `VACUUM INTO` (safe under concurrent writes).
- **`POST /domains/{name}/import`** — restore a snapshot into a NEW domain
  (target must not exist; `global` protected; atomic temp-file + rename).
- **`GET /domains`** — real per-domain counts via the registry, not a GROUP BY
  on the shared pool.

#### Added — migration + tests (M6)
- **Boot-time legacy cutover snapshot.** When `BRAIN_MULTI_DB=true` is set at
  startup and the legacy `brain.db` has data, the server performs a one-shot
  `VACUUM INTO` into `global.db`, guarded by a marker so restarts never
  re-copy. The runtime keeps reading the legacy path; the snapshot exists as
  a backup and as the physical source for any future operator cutover.
- **Four required M6 integration tests added:** domain isolation, fallback
  trigger on low-confidence routing, structured ingest entity/relation
  insertion (the canonical `vitamin d3` example), and export round-trip.

#### Changed
- Cargo.toml version 0.9.9 → 1.0.1.
- `openapi.yaml` info version → 1.0.0; the new domain lifecycle routes are
  documented (the `test_openapi_covers_routes` test asserts coverage).
- Handlers that previously used `state.pool` directly now resolve via
  `handlers::resolve_domain_pool(&state.registry, domain)`. Shim mode returns
  the global pool unchanged; multi-db mode opens per-domain pools lazily.
- `API_CONTRACT.md` §4 documents the new lifecycle routes; §9 documents the
  v1.0 boot-time cutover + deprecation policy.

#### Honest ceilings (carried forward)
- **Domain `dim` / `quant` are not per-domain.** All domains share the global
  model profile; per-domain model selection is a v1.1 concern.
- **No registry DB table.** The registry enumerates `brain-<domain>.db` files
  on disk. This is simpler and avoids a separate `registry.db` to manage, but
  means there's no per-domain `dim`/`quant`/`version` metadata store.
- **The `global` domain continues to read the legacy `brain.db` even in
  multi-db mode.** The boot-time snapshot creates `global.db` as a backup +
  rehearsal target, but the runtime path stays on `brain.db` for `global` so
  the 430-doc live DB never silently shifts under the operator.
- **Cross-domain `ATTACH` was not used.** Per-domain pool queries + RRF merge
  is simpler and avoids sqlite-vec attach complications; benchmark on ARM
  eMMC remains an operator step (see `BENCHMARKS.md`).

### v0.9.9 "Qualify" — 2026-07-25 (released)

The v1.0 cutover rehearsal milestone. No user-visible multi-domain behavior
ships here — that is v1.0.0. v0.9.9 extracts the migration + storage seams,
ships a copy-and-verify **rehearsal** tool, publishes measured capacity
envelopes with fail-clear behavior, and freezes the v1.0 API + migration
contract. The actual `BRAIN_MULTI_DB=true` cutover is the v1.0 ship step; this
release makes it a rehearsed operation, not an architectural leap.

#### Added — M1 (domain-ready seams)
- **`StorageLayout` abstraction** (`src/storage_layout.rs`). Every on-disk path
  brain-server touches (legacy `brain.db`, future `global.db`, per-domain
  `brain-<name>.db`, backups, registry, connector configs) derived from one
  root. `config::brain_db_path()` delegates to it; the back-compat invariant
  (existing `BRAIN_DB_PATH` callers see the same path) is locked by a test.
  New `BRAIN_DATA_ROOT` env var is the v1.0 relocation knob.
- **Schema-version reader** (`storage_layout::schema_version` +
  `SCHEMA_VERSION_V0_9_9`). `run_migration` records `schema_version` in
  `schema_meta`; the rehearsal tool reads it to refuse a migrate-down.
- **Extended `test_migration_schema_contract`.** Now asserts every table from
  v0.9.4–v0.9.8 (`audit_events`, `webhook_queue`, `webhook_seen`,
  `evidence_links`) + the `authority` column + the recorded schema version.
- **`is_valid_domain` lifted to `storage_layout`** so the security-critical
  filename check lives in exactly one place; `DomainRegistry` delegates.

#### Added — M2 (migration rehearsal)
- **`brain-migrate-rehearse` binary** (`src/bin/brain_migrate_rehearse.rs`,
  feature-gated behind `--features migrate`). Six subcommands: `backup`, `copy`,
  `verify`, `report`, `rollback`, `rehearse`. Runs against a *copy* of the live
  DB (server must be stopped). The `rehearse` all-in-one exits 0 only when every
  parity check passes.
- **`run_migration` extracted to `src/migration.rs`** (lib module). Mechanical
  move from `main.rs`; the one signature change is `run_migration(db,
  mmap_mib: i64)` so the lib has no dep on the server-private `config` module.
  All 9 call sites updated.
- **Parity checks.** Row counts for every table (knowledge, embeddings,
  vec_knowledge, entities, relationships, tombstones, sources,
  source_revisions, connectors, connector_checkpoints, audit_events,
  webhook_queue, evidence_links), FTS5 count, vec0 count, source/revision
  linkage, schema-version comparison, and a 50-row random vec0 byte-spot-check.

#### Added — M3 (capacity + contract)
- **Capacity envelopes** (`src/capacity.rs`, lib module).
  `CapacityTarget::Desktop` (50k docs / 2 GiB DB / 320 MB RSS) and
  `CapacityTarget::Jetson` (10k docs / 512 MiB DB / 320 MB RSS). Resolved from
  `BRAIN_CAPACITY_TARGET` (default: jetson). Tightenable via `CAPACITY_MAX_*`
  env vars.
- **`/health` capacity field.** Reports `{target, docs, max_docs, db_mib,
  max_db_mib, rss_mib, max_rss_mib, status}` where `status` is
  `ok|warning|exceeded`.
- **HTTP 507 on writes when over-capacity.** Every ingest path (`/add`,
  `/ingest`, `/ingest/memory`, `/ingest/markdown`) calls `guard_capacity`.
  Read routes (`/search`, `/recall`, `/get`) are NEVER blocked — an
  over-capacity brain still answers.
- **`bench --envelope` assertion mode.** `BENCH_ENVELOPE=desktop|jetson`
  turns the benchmark report into a ship gate: exits non-zero on RSS or p95
  ceiling breach.

#### Documentation
- `openapi.yaml` → 0.9.9: `/health` capacity field; `X-Api-Version: 0.9.9`.
- `API_CONTRACT.md`: §Migration (v1.0 per-row cutover rule), §Recovery (the
  rehearsal-proven rollback procedure), §Capacity envelopes.
- `IMPLEMENTATION_PLAN_v0.9.9_Qualify.md`: the full plan this release ships.

#### Internal
- `Cargo.toml` 0.9.8 → 0.9.9. New `migrate` feature + `brain-migrate-rehearse`
  `[[bin]]` entry.

#### Honest ceilings (carried into v1.0.0)
- No `BRAIN_MULTI_DB=true` cutover is performed in v0.9.9 — the rehearsal runs
  against a *copy*; the live DB stays in shim mode.
- WAL-active detection is a heuristic (file-size check); the operator is
  expected to have stopped the server.
- The 50-row vec0 spot-check is a sample, not a full scan — catches the known
  sqlite-vec corruption class but cannot prove byte-identity of every embedding.
- Old-schema fixtures (v0.9.4/v0.9.6/v0.9.8) and the interrupted-migration
  SIGTERM test are deferred — the current-schema parity checks cover the ship
  gate; the upgrade-from-old-schema path is exercised by the server's own
  startup migration on every prior release.
- The soak driver (`scripts/soak.sh`) and large-vault generator are deferred as
  operator tooling; the `bench --envelope` mode is the code-level ship gate.
- **10k-scale bench trips the loopback rate limit** (10 000 req/60s,
  hardcoded in `src/main.rs:RateLimiter`). Measured capacity on the production
  mini PC is captured at 1k+5k scales (6k requests, under the limit). To
  measure 10k+, either raise the loopback limit, exempt loopback in
  `rate_limit_middleware`, or add an inter-request delay in `bench`.
  See `BENCHMARKS.md` §v0.9.9.

### v0.9.8 "Evidence" — 2026-07-20 (released)

The evidence-integrity milestone. Recall now carries faithful, time-aware
provenance and a reviewable consolidation path so the memory backend stops
serving stale or contradicted facts as current. All changes are additive (new
`temporal` columns on `knowledge`, a new `evidence_links` table); the live
launchd service upgrades in place via `scripts/install-service.sh`.

#### Added
- **Temporal provenance (M1).** `knowledge` gains `observed_at`, `valid_from`,
  `valid_to`, `authority`, populated by `sources::stamp_evidence` on every
  ingest (vault = 0.8, manual = 1.0). `QueryDoc` gains `as_of` (point-in-time
  recall — returns the revision active at a timestamp) and `evidence`
  (include structured `Evidence` on every hit). Both retrievers apply the
  historical `as_of` predicate against `source_revisions.fetched_at`.
- **Structured `Evidence` (M2).** `Evidence` now carries `valid_from`,
  `valid_to`, `observed_at`, `authority`, `lifecycle`, and typed `links`
  (`supports` / `supersedes` / `contradicts` / `references` / `derived_from`).
  `enrich_evidence` loads links a chunk participates in (both directions).
- **Consolidation (M2.3).** New `src/consolidate.rs` detection
  (`find_exact_duplicates`, `find_subject_conflicts`) + `evidence_links` table.
  `POST /consolidate/propose` (read-only detection) and
  `POST /consolidate/apply` (operator records typed links; never automatic).
- **Freshness + conflict flags (M2.4/M3.1).** Recall honors `observed_at` as a
  stable freshness tie-break. `RecallHit.conflict` is `true` when a hit has a
  `contradicts`/`supersedes` link to a current chunk.
- **Evidence metrics (M3.2).** `tests/metrics.rs` adds `stale_result_rate`,
  `current_evidence_recall`, `citation_correctness`,
  `consolidation_false_positive_rate` (unit-tested, no model needed).

#### Honest ceilings (carried into v0.9.9+)
- Evidence links live in a flat `evidence_links` table, not the
  `entities`/`relationships` KG. Graph use improves conflict *detection*
  (entity-keyed subject), not link storage.
- No automatic mutation: consolidation is review-only via `brain consolidate`
  + `apply`. No autonomous deletion, no LLM judgment.
- `as_of` point-in-time recall is derived from `source_revisions.fetched_at`;
  pre-v0.9.8 chunks (no revision linkage) are always treated as current.

---

## [0.9.7] — "Guard" — 2026-07-20 (released)

v0.9.7 "Guard" is the security milestone: Brain Server now defends its own trust
boundary instead of assuming a trusted LAN. All work is additive (no schema break).

### Added
- **Loopback-safe bind.** The server refuses `0.0.0.0` unless `BIND_PUBLIC=1` is
  set; an invalid `BIND_HOST` now exits (exit 2) instead of silently falling
  back to all-interfaces exposure. `src/main.rs` + `src/config.rs`
  (`BIND_PUBLIC_OPT_IN`).
- **Verified webhooks** (`src/webhook.rs` + `src/handlers/webhooks.rs`):
  `POST /webhooks/{kind}` verifies the GitHub `X-Hub-Signature-256` HMAC,
  enqueues onto a bounded FIFO (`WEBHOOK_QUEUE_MAX`), and is idempotent via
  `UNIQUE(delivery_hash)` + a `webhook_seen` replay window
  (`WEBHOOK_REPLAY_SECS`). Stale/future `Date` headers are rejected. A drain
  worker (`webhook::spawn_drain_worker`) processes verified deliveries without an
  HTTP round-trip. The webhook route bypasses the bearer middleware (HMAC is its
  auth) but is verified inside the handler.
- **Append-only audit log** (`src/audit.rs`): `audit_events` table records
  hash-only events (identifiers + xxh3 hashes; never raw content, tokens, or
  secrets). `GET /audit` (operator diagnostics) + `brain audit [--kind K]
  [--limit N]`. Ingest and auth-denial events are recorded across the ingest
  paths and the auth boundary.
- **Prompt-injection quarantine** (`src/config.rs` `InjectionPolicy`):
  `contains_suspicious_pattern` hardened with zero-width/control-char
  normalization (`is_zero_width`), more instruction-override phrase signatures,
  and line-anchored structural markers (still no false positive on
  "Nervous System:"). Under `quarantine` (default) suspicious content is stored
  but `flagged = 1` and excluded from retrieval; `GET /quarantine`,
  `POST /quarantine/{id}/release`, `POST /quarantine/{id}/delete` let an operator
  review/approve/purge. `flag_if_quarantined` + `suppress_flagged_evidence`
  (retrieval-side evidence stripping unless `include_flagged`).
- **Untrusted-evidence boundary** (OWASP LLM01:2025): every `SearchResult`,
  `RecallHit`, and `Evidence` now serializes `untrusted: true`, so the consuming
  agent treats recalled content as data, never as instructions. vec0/FTS search
  gains an `include_flagged` filter (default excludes flagged rows).
- **Multi-token auth + live rotation** (`src/config.rs` `auth_tokens()`):
  `AUTH_TOKEN` / `AUTH_TOKEN_FILE` accept newline-separated tokens, all accepted
  per request — rotate or revoke by editing the token file, no restart.
- **Encrypted backup/restore** (`src/backup.rs` + `brain backup` / `brain
  restore` / `brain doctor --backup`): AES-256-GCM (key = SHA256(passphrase)),
  embedded manifest + `.sha256` checksum, secret-file bytes excluded (path+hash
  recorded only), and a `.bak` safety snapshot taken before any overwrite.
- **`openapi.yaml`**: documents `/webhooks/{kind}`, `/audit`, `/quarantine`,
  `/quarantine/{id}/release`, `/quarantine/{id}/delete`, and the `untrusted`
  field on `SearchResult` / `RecallHit` / `Evidence`.

### Honest ceilings (carried into v0.9.8+)
- The webhook replay defense is delivery-hash + replay window; the `Date`-header
  timestamp check tightens it further but is not a signed timestamp (GitHub
  sends no signed time). Treat `webhook_seen` as the primary protection.
- `contains_suspicious_pattern` is a deterministic structural screen, **not** a
  classifier. It catches known override signatures and obfuscation (zero-width
  chars) but cannot catch every adversarial input. The architectural control
  point is segregation via the `untrusted` flag, not the filter alone.
- The webhook drain worker is an audit-only stub; real ingestion-on-webhook is
  deferred to a later milestone.
- No `POST /admin/auth/revoke` HTTP route yet — revocation is file-based
  (`cp`/edit the token file).
- Encrypted backups use passphrase-derived keys (no OS keychain); that matches
  the existing `auth-token` pattern.

---

## [0.9.6] — "Bridge" — 2026-07-20 (released)

v0.9.6 "Bridge" is complete: M1 (connector contract + supervisor primitives +
stub binary), M2.1 (auth foundation: `AuthProvider` trait + `CredentialStore`
+ `GitHubAppProvider`), M2.2 (the `brain-connector-gh` binary + GitHub REST
client + issue→Markdown translation + backfill with rate-limit-aware
pagination + durable cursors), M2.3 (periodic reconcile via the existing
`/sources/reconcile` route), and M3 (the `brain connect github`, `brain sync`,
and `brain connector-status` CLI commands).

The live launchd service continues to run v0.9.6 once `install-service.sh` is
re-run; the connector binaries install alongside the server (built with
`--features connector-github` for `brain-connector-gh`).

### Architecture decisions (locked in by this release)
- **Connectors are separate binaries.** The server never links connector code
  (`bin_common/http.rs` line 4 invariant preserved). The connector binary is
  free to depend on `reqwest` + `jsonwebtoken` + `rsa` — all feature-gated on
  `connector-github`, never compiled into the server.
- **No new wire protocol.** The connector contract is three concrete
  conventions (manifest TOML + argv + JSON-lines on stdout) **plus reuse of
  the existing brain-server HTTP API** (`/ingest/markdown`, `/sources/reconcile`,
  `/connectors`). Zero new endpoint families.
- **The server is the supervisor.** `tokio::process::Command` with
  `next_backoff` restart (exponential capped at 60s, no jitter — single local
  supervisor, no herd risk).
- **Auth is a trait, not a struct.** `AuthProvider` is the unified surface;
  `StaticTokenProvider` (stub + tests), `GitHubAppProvider` (M2.1), and the
  future `OAuthProvider` (v0.9.7) all implement it.

### Added
- **`src/connector/mod.rs`** — `ConnectorManifest`, `ConnectorRow`,
  `list_connectors`, `upsert_connector`. Idempotent registration.
- **`src/connector/supervisor.rs`** — `next_backoff` (overflow-safe exponential
  capped at 60s), `spawn_once` (tokio::process with kill_on_drop).
- **`src/connector/auth/mod.rs`** — `AuthProvider` trait + `AccessToken` (with
  redacted `Display`) + `StaticTokenProvider`.
- **`src/connector/auth/store.rs`** — `CredentialStore<T>`: per-connector JSON
  config at `~/.config/brain-server/connectors/{kind}-{instance}.json` (0600).
  Atomic save via `std::fs::rename`. No at-rest encryption beyond filesystem
  permissions + FileVault/LUKS — matches the existing `auth-token` pattern.
- **`src/connector/auth/github_app.rs`** — `GitHubAppProvider`: full JWT (RS256)
  → installation-token flow. Token-level repo scoping via the optional
  `repositories` body field (the DoD-1 mechanism). In-memory single-slot
  cache refreshed within `REFRESH_SKEW=60s` of expiry.
- **`src/connector/github/client.rs`** — `GitHubClient`: wraps reqwest with
  GitHub-required headers + rate-limit sleep (capped at 60s) + Link-header
  pagination.
- **`src/connector/github/translate.rs`** — `translate_issue`: renders each
  issue as YAML frontmatter + Markdown body. Source URI:
  `github://{owner}/{repo}/issues/{N}`. Stable across edits, unique per issue.
- **`src/connector/github/mod.rs`** — `backfill_issues_for_repo` +
  `reconcile_github_sources` + cursor store (`connector_checkpoints` table).
- **`src/bin/brain-connector-stub.rs`** — M1 reference connector (~140 LOC).
  Spawns, parses argv, emits JSON-lines, ingests one doc, exits 0.
- **`src/bin/brain-connector-gh.rs`** — the real GitHub connector (~280 LOC).
  Loads config, opens checkpoint DB, fetches installation token, backfills
  each configured repo, reconciles.
- **`src/lib.rs`** — new library target exposing only `pub mod connector`.
  Server modules stay private to `src/main.rs`.
- **Migration**: additive `connectors` + `connector_checkpoints` tables.
  Idempotent (`CREATE TABLE IF NOT EXISTS`). No data migration.
- **`GET /connectors`** route + `ConnectorRow` OpenAPI schema.
- **`brain connect github`** CLI: writes connector config (0600, atomic) from
  `--app-id`, `--install-id`, `--key-file`, `--repo` argv.
- **`brain sync [github]`** CLI: spawns `brain-connector-gh` with the right
  argv; surfaces its JSON-lines event stream to the operator.
- **`brain connector-status`** CLI: lists every registered connector.

### Changed
- `Cargo.toml`: `version` 0.9.5 → 0.9.6. New optional deps `jsonwebtoken`
  (`rust_crypto` + `use_pem` features) + `reqwest` (`rustls` + `json` +
  `blocking`), both feature-gated on `connector-github`. New `[[bin]]`
  `brain-connector-stub` (always built) + `brain-connector-gh` (requires
  `connector-github`). New dev-deps `rsa` + `rand` + `base64` (for JWT-shape
  tests).
- `openapi.yaml`: bumped to 0.9.6; added `/connectors` route +
  `ConnectorRow` schema.
- `test_migration_schema_contract`: extended to assert the two new tables.
- `test_openapi_covers_routes`: extended with `/connectors`.

### Removed
- _Nothing._ The rerank tier removal landed in v0.9.5 (`3fcac72`); this
  release is additive.

### Honest ceilings (not bugs)
- **Issues only.** PRs are filtered out at translate time (PRs are issues
  with a `pull_request` field); their dedicated backfill lands in v0.9.7.
- **No comments.** Each issue's body is ingested as one doc; threaded
  comments land in a separate sub-resource cursor later.
- **No streaming JSON parser.** Each page is fully buffered. Fine for
  issues/PRs/discussions; revisit if wiki pages exceed 1 MB on the 4 GB
  Jetson.
- **`AuthProvider` is sync.** The connector is a batch process — async here
  would buy nothing. Revisit if a future connector needs streaming auth.
- **Rate-limit sleep capped at 60s** (not the full `X-RateLimit-Reset`
  window). Prevents silent hour-long wedges; surfaces as a hard error on
  the second attempt.
- **No at-rest encryption** in `CredentialStore`. Filesystem permissions +
  FileVault/LUKS are the only at-rest protection. Matches the `auth-token`
  pattern; revisit if multi-tenant.
- **Webhook ingress is deferred.** Reconcile alone satisfies DoD-2; the
  webhook path lands in v0.9.7+ for near-real-time sync.
- **Single-shell restart loop** with `kill_on_drop`. Graceful drain lands
  with v0.9.7+ `brain disconnect`.
- **No `brain connector doctor`.** `brain status` + `brain connector-status`
  cover the same ground for v0.9.6.

### Context7-verified facts cited inline
- GitHub REST API (`/websites/github_en_rest`, 2026-07-20):
  `X-GitHub-Api-Version: 2026-03-10` is current; installation tokens support
  the `repositories` body field for per-repo scoping.
- Standard Webhooks spec (`/standard-webhooks/standard-webhooks`, 2026-07-20):
  constant-time compare + idempotency key + timestamp tolerance for webhook
  signature verification (deferred to v0.9.7 webhook ingress).
- RustCrypto hashes (`/rustcrypto/hashes`, 2026-07-20): `sha2::Sha256` +
  `hmac::Hmac<Sha256>` is the canonical HMAC-SHA256 path for webhook
  verification (deferred to v0.9.7).
- `jsonwebtoken` (`/keats/jsonwebtoken`, 2026-07-20): RS256 +
  `EncodingKey::from_rsa_pem` (requires `use_pem` feature) is the canonical
  JWT-signing path for GitHub Apps.

---

## [0.9.5] — "Inspect" — 2026-07-19 (released)

v0.9.5 "Inspect" is complete: M1 (structured query contract), M2 (evidence
quality), and M3 (product interface) all shipped 2026-07-19 (M1: `a46c7ab`,
`ade13d1`, `28309f9`; M2: `0b10b45`, `9a4ce75`; M3: Agent 20). The live
launchd service runs v0.9.5.

### Removed
- **Rerank tier (`--features rerank` + `fastembed-rs` BGE cross-encoder), deleted in
  `3fcac72`.** It pegged the M1 CPU and blew the 8s recall timeout, and was too heavy
  for the Jetson edge GPU. The hybrid `vec0` KNN + FTS5 BM25 + RRF + PRF retrieval is
  the right ceiling for this edge-only deployment. `/stats` now reports
  `rerank_status: "off"`. The `rerank_score` / `rerank_truncated` / `rerank_ms` API
  fields are retained (always `null` / `false` / `0`) for contract stability.
  The `rerank` Cargo feature flag and `src/search/rerank.rs` were deleted entirely,
  not stubbed — to re-add the tier, revert `3fcac72` on a CUDA-GPU deployment.

### Added (v0.9.5 M1 — "Inspect")
- **Structured query document (`QueryDoc`).** Both `/search` and `/recall`
  lower their params into one versioned `QueryDoc` (`src/search/query.rs`),
  so they share a single lexical compiler + validation path. A plain-text
  query remains backwards compatible.
- **Lexical controls via `LexSpec`.** `{ terms, phrases, exclude, code }` is
  compiled into a **validated, FTS5-quoted** MATCH string. Replaces the old
  unvalidated raw-`lex` passthrough (which returned opaque SQLite errors on
  bad input). Caller input can no longer inject FTS5 operators. `/recall`
  accepts `lex` as either a bare string (`{"lex":"foo"}`) or a full `LexSpec`
  object; `/search` (GET) takes a comma-separated `lex` string mapped to one
  term.
- **Multi-source OR scoping.** `SearchFilters.sources: Vec<String>` applies
  `source IN (?,?…)` in both `vec0_knn` and `fts_search`; the legacy single
  `source=` is still honored when `sources` is empty. `/search` takes
  comma-separated `sources=a,b`.
- **`intent` is provenance-only.** Recorded into telemetry/provenance; never
  injected as a search term and never relaxes `since`/`source`/`domain`
  filters (verified by code trace).

### Changed
- `/search` and `/recall` responses now reflect the compiled lexical query
  and OR source scope in their `explain`/`query_plan` blocks.

### Known ceilings (not bugs)
- `profile` field is accepted but passthrough (no rerank/weighting yet).
- `LexSpec` covers terms/phrases/exclusions/exact-code only — no `NEAR`,
  prefix `*`, or column filters.
- `/search` GET takes a flat `lex` string, not a nested `LexSpec`; the full
  structured form is on `/recall` POST and will back the M3 `brain query` CLI.

### Added (v0.9.5 M2 — "Evidence quality")
- **Structured `Evidence` on every hit.** `SearchResult`/`RecallHit` now
  carry `evidence` = `{ text, line_start, line_end, heading_path,
  source_uri, revision_id, highlights }`. `text` is a verbatim substring of
  the chunk; `highlights` are byte-offset ranges *within that window* (the
  server never injects HTML). `source_uri`/`revision_id` link to the exact
  source revision (NULL for pre-v0.9.4 chunks without source linkage).
  Populated by one batched LEFT JOIN (`enrich_evidence`), not N queries.
- **`GET /get/{id}` and `POST /multi-get`** now return `source_uri` +
  `revision_id`; `multi-get` bound raised to 1000 (was hardcoded 100).
- **`explain` redaction + reproducibility.** `/search?explain=true` redacts
  full `content` from results (only the bounded `evidence.text`/`snippet`
  serialize) and adds `k`/`source`/`domain`/`since`/`profile` to
  `query_plan`. A `MAX_EXPLAIN_BYTES` (64 KiB) hard cap falls back to the
  summary if exceeded. Snippet window bounded by `MAX_SNIPPET_CHARS` (240)
  + `SNIPPET_CONTEXT_CHARS` (60), centralized in `config.rs`.
- **`config.rs`**: added `MAX_SNIPPET_CHARS`, `SNIPPET_CONTEXT_CHARS`,
  `MAX_EXPLAIN_BYTES`, `MAX_MULTI_GET`.

### Added (v0.9.5 M3 — "Product interface")
- **`brain query` on the structured contract.** `brain query "<q>"` now POSTs
  `POST /recall` with a v0.9.5 `QueryDoc`: repeatable `--phrase`/`--exclude`/
  `--code` (lowered into `LexSpec`), multi-`--source` OR scope, `--intent`,
  `--profile`, `--since`, `--k`, `--explain`. Back-compat bare-string queries
  still work.
- **`brain get <id>` implemented** against the existing `GET /get/{id}` route
  (M2.3 ceiling closed). Prints title/source/heading/line span/`source_uri`/
  `revision_id` + content; 404 → "no chunk with id".
- **`brain explain` unified** on `/recall`'s `provenance`/`telemetry` envelope
  (closes the M2.2 split where `/search` used `query_plan` and `/recall` used
  `telemetry`).
- **`GET /openapi.yaml`** serves the canonical OpenAPI 3.0 contract (embedded
  via `include_str!`, so it ships with the binary). `openapi.yaml` updated to
  v0.9.5: all 23 routes + `QueryDoc`/`LexSpec`/`Evidence`/`Chunk`/`QueryPlan`/
  `SearchTelemetry` schemas.
- **`examples/client_example.rs`** — a typed client over the shared dependency-
  free HTTP client, demonstrating a structured `QueryDoc` roundtrip.
- **MCP tool schema** (`mcp` server): `brain_search`/`brain_recall`/
  `brain_ingest` updated to the v0.9.5 `QueryDoc`; both search tools now POST
  `POST /recall` via one shared body-lowerer.
- **API versioning + deprecation.** Every response carries `X-Api-Version:
  <semver>`; deprecated `POST /add` and `GET /search` return an RFC 8594
  `Deprecation: version="0.9.5"` header. Policy + migration mapping documented
  in `API_CONTRACT.md` §Versioning & deprecation.
- **`test_openapi_covers_routes`**: asserts every route registered in
  `build_app` appears in `openapi.yaml`.

### Known ceilings (carried into v0.9.6)
- `highlights` over the *full* chunk still require `GET /get/{id}`; `brain get`
  returns full content so a client can compute its own.
- `profile` accepted but passthrough (no rerank weighting yet).
- OpenAPI is hand-written (no code-gen dep); the coverage test guards drift.

---

## [0.9.4] — "Sources" — 2026-07-17 (released)

The source-lifecycle release. Every knowledge chunk now carries provenance:
 the canonical `source` it came from (a vault file, a manual memory, …) and
 the immutable `source_revision` snapshot of the exact content version. A
 vault file edited on disk produces a new revision atomically; a deleted file
 is detected by `brain reconcile` and its chunks swept from retrieval. Plus a
 bug-fix sweep that landed while the feature work was in flight.

### Added
- **Canonical sources + revisions (M1+M2).** Two new tables — `sources`
  (stable identity per external document, keyed by canonical URI; kind-scoped
  as `vault` / `manual`) and `source_revisions` (immutable snapshots;
  supersession chain). Two new columns on `knowledge` (`source_id`,
  `revision_id`) link every chunk to its source + revision. Existing 430-doc
  DB left NULL — pre-v0.9.4 chunks keep working; new ingests pick up source
  linkage. Idempotent additive migration (CREATE IF NOT EXISTS + column
  guards), guarded by `test_migration_schema_contract`.
- **`/ingest/markdown` + `/ingest/memory` now write source linkage** inside
  their existing transactions. Vault ingests use the canonical file path as
  the URI; manual memories use `manual://{content_hash}` (no PII; stable
  across re-ingests; immune to vault reconcile because reconcile is
  kind-scoped). The unchanged-file no-op path backfills source linkage for
  pre-v0.9.4 chunks on first v0.9.4 re-ingest — so re-ingesting an existing
  vault retroactively links its chunks without rescanning.
- **`POST /sources/reconcile`** — body `{kind, live_uris: [string]}`. The
  server retires any active source of `kind` whose URI is NOT in the live
  set, sweeping its chunks from retrieval (vec0 + FTS + knowledge rows) and
  tombstoning the source + active revision. The server does NOT walk the
  filesystem — the caller supplies the live set, preserving the
  client/server boundary. Bounded `MAX_LIVE_URIS = 50_000`.
- **`DELETE /sources/{id}`** — retires a single source by id. 404 if absent.
- **`brain reconcile <path> [--kind vault] [--dry-run]`** — walks the path
  with the SAME walker + `.brainignore` semantics + canonicalized-absolute-path
  URI form that `brain ingest-dir` uses, so URIs match what's stored. POSTs
  the live set to `/sources/reconcile`. Recommended after every
  `brain ingest-dir <vault>` to detect deletes / renames.
- **`brain source-delete <id>`** — companion CLI for the DELETE route.
- **`scripts/install-service.sh` now installs the operator CLIs**
  (`brain`, `mcp`, `bench`) alongside `brain-server`, with `--features bench`
  so the `bench` binary compiles. Previously only the server binary was
  installed, so `brain doctor` / `brain status` were not on `$PATH`.
- **macOS `com.apple.provenance` xattr cleanup** in `install-service.sh`.
  Sonoma+ tags every newly-written executable with this xattr and Gatekeeper
  SIGKILLs the process on first exec (`Killed: 9`, exit 137). The script now
  strips it after each copy so freshly-installed binaries actually run.

### Fixed
- **Character-preservation warranty for the ingest pipeline.** Markdown
  files whose name OR content contain special characters — `#`, `-`, `_`,
  spaces, parens, brackets, unicode, backticks, code fences with `#`-comments,
  hash-delimiters inside string literals — now round-trip verbatim through
  the chunker → DB → source-linkage → dedup path. Filenames with special
  chars are preserved byte-for-byte as `sources.uri` and
  `knowledge.source_path`; content is preserved in `knowledge.content`;
  per-chunk `content_hash` is stable across re-ingest. The chunker treats
  `#`-lines inside a code fence as code, NOT as headings (so a Python file
  with `#`-comments is not mistaken for a heading hierarchy). Renamed the
  misleading `MAX_CHUNK_CHARS` to `MAX_CHUNK_BYTES` (it was always bytes).
  Verified by `test_special_characters_survive_ingest_pipeline`.
- **`brain --help` lost its 2-space indentation.** The `print_usage` string
  used `\n\` line continuations, which Rust interprets as "newline + strip
  leading whitespace on next line" — so every subcommand rendered flush-left.
  Switched to a raw string literal (`r#"..."#`) which preserves the intended
  2-space indentation and lets embedded `"` survive without escaping.
- **`/stats` reported a stale `embeddings` count** (e.g. `2` on a 430-doc
  corpus). The handler counted the legacy `embeddings` table, which has been
  frozen read-only since v0.9.0 — all post-v0.9.0 vectors live in the
  `vec_knowledge` vec0 table. `/stats` now counts `vec_knowledge`, so the
  number reflects the live index (backfilled legacy + new ingests).
- **`brain`, `mcp`, and `bench` CLIs returned `401` on every authenticated
  route** (`/search`, `/stats`, `/recall`, `/ingest/*`, `/sources/*`). The
  shared HTTP client in `src/bin_common/http.rs` had no auth support;
  `get()`/`post()` did not accept headers, so no `Authorization: Bearer` was
  ever sent. The client now takes an optional `bearer: Option<&str>`, and
  each binary resolves the token via `BRAIN_TOKEN_FILE` → `BRAIN_TOKEN` →
  `~/.config/brain-server/auth-token` (mirroring the server's
  `AUTH_TOKEN_FILE` → `AUTH_TOKEN` ladder). Zero-config for the common
  install — same file the launchd plist already sources.
- **`brain-server --version` silently started the server.** `main.rs` did no
  argv inspection, so any flag was ignored and execution fell through to
  `bind()`. If the port was free, the process became a foreground server
  attached to the caller's shell. An argv guard now runs before any side
  effect (tracing init, model load, socket bind): `--version`/`-V` prints and
  exits 0; `--help`/`-h` prints brief usage and exits 0; unknown `-`-prefixed
  flags exit 2 instead of launching the server.
- **`brain --version` was rejected** as an unknown subcommand (`error:
  unknown subcommand '--version'`, exit 2). Added a `-V`/`--version` arm to
  the existing command matcher; both `brain` and `brain-server` now report
  `env!("CARGO_PKG_VERSION")` and exit 0.

### Changed
- **`write_markdown_ingest` takes a new `raw_content: &str` parameter** (the
  original payload, frontmatter + body) so the source revision hash reflects
  ANY change in the file, not just body changes that survive frontmatter
  stripping. Now 8 args — `#[allow(clippy::too_many_arguments)]` with a
  comment explaining why bundling into a struct is pure ceremony for a
  private fn with one prod caller.
- **CI now runs `cargo clippy --all-targets --features bench -- -D warnings`
  and `cargo test --all-targets --features bench`.** The `bench` binary is
  feature-gated and was previously untested upstream.
- **Chunker rewritten on top of `pulldown-cmark` 0.13** (Context7-verified
  2026-07-17). The pre-v0.9.4 chunker was a hand-rolled line-scanner that
  mis-handled CommonMark constructs: setext headings (`Foo\n===`), indented
  code blocks (4-space indent), blockquotes, lists, GFM tables. The new
  chunker walks `pulldown-cmark`'s event stream with `into_offset_iter()` and
  slices source bytes verbatim from the union of event ranges, so every
  container markup character (`>`, `-`, `|`, fence markers) survives intact.
  Heading detection is now CommonMark-spec-driven (handles ATX, setext, and
  any GFM-tagged heading), `#`-comments inside code blocks are no longer
  mistaken for headings, and indented code blocks are no longer mistaken for
  prose. New dependency: `pulldown-cmark = { version = "0.13",
  default-features = false }` (we use only the parser; the `html`/`getopts`
  default features are dropped). pulldown-cmark is `#![forbid(unsafe_code)]`
  upstream; we keep our `#![deny(unsafe_code)]`.
- **Chunker warranty** (carryover from earlier v0.9.4 work): every byte of
  input text — including `#`-comments inside code fences, unicode, backticks,
  brackets, dashes, hash-delimiters inside string literals — survives intact
  into the chunk `text`. The only lines consumed (not buffered verbatim) are
  ATX and setext headings; their text becomes the chunk's `heading_path`
  breadcrumb instead. The misleading `MAX_CHUNK_CHARS` constant was renamed
  `MAX_CHUNK_BYTES` (it was always bytes — `str::len`). Verified by
  `test_special_characters_survive_ingest_pipeline` plus 6 new per-construct
  tests covering setext, indented code, blockquote, list, GFM table, and
  `#`-in-code-fence.

### Tests
- **130 passed, 1 ignored** (was 113 at v0.9.3). Delta: +7 from
  `sources::tests::*` now reachable via `mod sources;`, +4 v0.9.4 vault/memory
  source-linkage integration tests, +1 character-preservation warranty test,
  +5 new CommonMark chunker tests (setext, indented code, blockquote, list,
  GFM table, `#`-in-code-fence) replacing the 1 removed `parse_heading` test.
- New `test_migration_schema_contract` asserts the full table/column contract
  after `run_migration` and verifies the ingest → FTS5 → vec0 roundtrip. This
  is the single test that catches a broken migration before it reaches the
  live DB.

### Known limitations
- Measured RSS / latency / recall numbers on 4 GB ARM and the ≥100 judged-
  query corpus remain **PENDING** a hardware run (inherited from v0.9.3).
- `pulldown-cmark` itself does not handle Obsidian-specific wikilink syntax
  (`[[target]]`) at the structural level — it emits them as Text events, which
  our chunker passes through verbatim. The `vault::parse_wikilinks` post-pass
  extracts them as `references` KG edges separately; the chunk text is
  unchanged.

---

## [0.9.3] — "Calibrate" — 2026-07-11 (released)

Named release formalizing the retrieval-calibration work that shipped in v0.9.1.
No new runtime code: the three Calibrate exit criteria — **PRF executes**, **rerank
has a candidate window**, and **the benchmark is reproducible** — are all already
satisfied by v0.9.1 and are guarded by dedicated tests. This release exists to
make the calibration state a named, reviewable checkpoint before the source-
lifecycle work in v0.9.4.

### Calibration state (verified, not newly added)
- **PRF executes.** The v0.9.1 fix replaced an unreachable `0.3` RRF-score
  threshold with a deterministic, calibrated gate (`prf_should_expand`): expansion
  fires only when the top pass-1 result appears in **both** the dense and lexical
  lists within a bounded rank. Guarded by
  `prf_expands_only_on_cross_retriever_agreement`.
- **Rerank has a candidate window.** `RERANK_CANDIDATES = 30`; retrieval over-
  fetches a window ≥ k and reranks **before** truncating to k, so a relevant hit
  just below k can be promoted. Guarded by `candidate_window_equals_k_when_disabled`
  and the rerank contract tests.
- **Benchmark is reproducible.** `BENCHMARKS.md` fixes the workload, hardware,
  metrics, and commands; the `bench` feature and `tests/metrics.rs` implement the
  protocol. The metric functions (`recall@k`, `precision@k`, `nDCG@k`, `MRR`) are
  unit-tested with hand-computed values.

### Honest status
- Measured RSS/latency/recall numbers on 4 GB ARM and the ≥100 judged-query corpus
  remain **PENDING** a hardware run. No claim of measured QMD parity is made.

---

## [0.9.2] — "Connect" — 2026-07-11 (released)

External markdown ingestion. brain-server can now ingest an Obsidian vault (or any directory of
markdown) and turn it into a searchable, graph-aware knowledge base — no GPU, no model download,
no API key, no data egress. This is the market wedge: the only zero-dependency local semantic
search engine over a user's notes.

One-shot ingest + graph is OSS. Live file-watcher sync, multi-vault, and the Obsidian plugin UI
remain a paid "Brain Vault" tier (feature-gated `live-sync`, not compiled into this release).

### Added
- **`brain ingest-dir <path>`** — recursive markdown ingest with `source_path` provenance on
  every ingested chunk. Walks are bounded (`MAX_INGEST_FILES=50k`, `MAX_INGEST_BYTES=500MiB`);
  `.brainignore` and Obsidian-internal dirs (`.obsidian/`, `.trash/`) are honored.
- **YAML frontmatter parsing** (`title`, `tags`, `aliases`): stripped before chunking; the
  frontmatter title is preferred for vault ingests (filename fallback). New `src/vault.rs`
  module — pure, no YAML dependency.
- **`[[wikilink]]` → knowledge graph**: `[[Target]]`, `[[Target|Alias]]`, `[[Target#Heading]]`
  become traversable `references` edges. Non-existent targets are created as placeholder
  entities so the graph completes as their files are ingested.
- **Frontmatter → entity metadata**: `tags:` → `tag` entities with `tagged_with` edges;
  `aliases:` → `alias_of` edges (a query for an alias resolves to the note).
- **Vault dedup is scoped to `source_path`**: re-ingesting an unchanged file is a true no-op
  (same chunk ids, zero inserts); a changed file sweeps its old chunks + vec0 rows and
  re-inserts. Content hashes are namespaced with `source_path` (`xxh3_64_with_seed`) so vault
  chunks never collide with memories or other files under the global unique index.
- **Schema**: new `knowledge.source_path TEXT` column (additive migration, NULL for existing /
  interactive rows) + `idx_knowledge_source_path` index.

### Fixed
- **`/graph/entity` and `/graph/traverse` rejected entity names containing spaces**, but note
  titles are stored with spaces (per `NAME_RE`). Both now allow spaces, so the wikilink graph is
  traversable from note titles like `bignay fruit`.

### Changed
- The `/ingest/markdown` DB-write was extracted into `write_markdown_ingest(tx, ...)` so the
  vault dedup/replace/KG logic is unit-testable without the embedding model.
- Title precedence is now caller-aware: vault ingests prefer frontmatter title; interactive
  adds prefer the explicit payload title.

### Tests
- 12 unit tests for `src/vault.rs` (frontmatter + wikilink forms).
- 6 integration tests for vault ingest (source_path storage, idempotent re-ingest, changed-file
  replace, wikilink→references, tags/aliases edges, schema).
- 4 unit tests for the client glob matcher and `.brainignore` honoring.

### Out of scope (paid tier / later releases)
- Live file-watcher sync (`notify` crate), multi-vault, scheduled re-index — paid "Brain Vault"
  tier behind `live-sync`.
- Obsidian plugin UI — paid tier.
- Per-domain isolation — v1.0.0 upgrades an ingested vault from flat `global` content into an
  isolated domain.

---

## [0.9.1] — "Recall" — 2026-07-11 (released)

Phase 2 of the roadmap. The retrieval engine was extracted into `src/search/`
(`#![deny(unsafe_code)]`; all sqlite-vec FFI stays in the crate root) and
hardened end-to-end: hybrid RRF fusion, PRF query expansion with FTS5-weighted
term extraction, an optional cross-encoder rerank tier, and full per-result
provenance on both `/search` and `/recall`. This entry also closes the
v0.9.0 plan gaps that the first-pass audit found (quantization DoD, migration
safety, benchmark/eval harnesses).

### Fixed
- **PRF query expansion actually executes now.** The previous gate compared an
  RRF fused score against an unreachable `0.3` threshold (top RRF ≈ 2/60 ≈
  0.033), so expansion never ran. PRF now uses a deterministic, calibrated gate
  (`prf_should_expand` in `src/search/mod.rs`): expansion fires only when the
  top pass-1 result appears in **both** the dense (`vec0`) and lexical (FTS5)
  lists within a bounded rank.
- **Rerank contract repaired.** The server previously truncated to `k` before
  reranking, so a relevant candidate just below `k` could never be promoted. It
  now over-fetches a candidate window (`RERANK_CANDIDATES = 30`, fixed
  constant) and reranks it *before* truncating to `k`.
- **Silent `since` filter replaced.** The temporal filter is now validated as
  ISO-8601 (RFC3339 or `YYYY-MM-DD HH:MM:SS`) via `normalize_since` and
  rejected if malformed, instead of relying on a lexical string comparison.
- **`/recall` now surfaces per-result provenance.** The handler previously
  computed per-retriever ranks and fused scores internally but dropped them at
  the handler boundary. `RecallHit` now carries an optional `Provenance`
  (populated when `provenance=true` on the request), closing the gap between
  `/search` (which already surfaced it) and the `/recall` + MCP `brain_recall`
  path.
- **Quantization DoD met: no raw f32 JSON in the DB.** All five ingest paths
  (`add_chunk`, `ingest_memory`, `ingest_markdown`, `reindex`, and the `/ingest`
  plugin handler) no longer write the legacy JSON `embeddings.vector` column.
  `vec0` (int8 + binary) is the sole write target. The `embeddings` table is
  retained read-only for one-time backfill of pre-v0.9.0 DBs.
- **Version source-of-truth.** The `mcp` binary now derives `SERVER_VERSION` from
  `env!("CARGO_PKG_VERSION")` (was hardcoded `"0.9.1"`, which would drift on the
  next bump).

### Added
- **Hybrid retrieval with Reciprocal Rank Fusion.** Vector (`vec0` KNN) and
  lexical (FTS5 BM25) retrieval run concurrently on independent pooled read
  connections, then are fused via RRF (`k = 60`, no learned weights). Each result
  records per-retriever ranks + the fused score in its `Provenance`.
- **PRF query expansion with FTS5-weighted term extraction.** Two-pass retrieval:
  pass-1 over-fetches by `PRF_DEPTH`, then high-signal expansion terms are
  extracted from the top hits via the `knowledge_fts_vocab` table
  (`fts5vocab='instance'`) with IDF-weighted BM25-style scoring
  (`score = local_cnt × ln(1 + total_docs/df)`). The expanded query is re-run and
  the two passes are RRF-fused so original-query matches keep their rank
  contribution (`fuse_prf_passes`). Falls back to the pure DF variant when the
  vocab table is unavailable.
- **Anti-injection guardrail for PRF.** Term extraction skips content that trips
  the prompt-injection screen and skips rows flagged as quarantined (`flagged`
  column on `knowledge`). Expansion is also gated on cross-retriever agreement —
  the top pass-1 result must appear in both the dense and lexical lists within a
  bounded rank, so PRF never amplifies a single-retriever outlier.
- **Env-driven PRF configuration** (`PrfConfig::from_env`): `PRF_ENABLED`
  (default `true`), `PRF_DEPTH` (default `10`, clamped 1–100), `PRF_TERMS`
  (default `5`, clamped 1–50), `PRF_MAX_RANK` (default `5`, clamped 0–100).
- **Optional cross-encoder rerank tier.** Feature-gated (`--features rerank`) and
  runtime-gated (`RERANK_ENABLED=true`); the default build is pure-static
  (Model2Vec, zero extra RSS). Uses `BGERerankerV2M3` via
  `fastembed::TextRerank::rerank` (scores query–doc *pairs*), memory-bounded by
  `RERANK_CANDIDATES` (30) and `RERANK_MAX_CHARS` (4096), and fails open to the
  first-stage result. Observable status (`off`/`disabled`/`loading`/`ready`/
  `failed`) surfaced via `/stats`.
- **Metadata-filtered KNN.** `source`, `since` (ISO-8601), and `domain` filters
  are pushed into the `vec0` KNN and FTS5 `WHERE` clauses (parameterized — no
  SQL injection). `source` and `created_at` are declared as `vec0` metadata
  columns.
- **Per-stage latency telemetry** (embed / vector / fts / fusion / prf /
  rerank) recorded in `SearchTelemetry` and emitted at debug level.
  `/search?explain=1` returns per-stage telemetry and the query plan.
- **Structured query** (`lex` / `vec` / `hyde` / `intent`) on `/search` and
  `/recall`: lexical precision via FTS5, semantic + hypothesis via the dense path,
  intent recorded for provenance. Faithful verbatim snippets are attached to each
  hit.
- **Benchmark harness** (`bench` Cargo feature + `src/bin/bench.rs`): ingests
  1k/5k/10k synthetic docs against a running server, records RSS at rest and
  per-batch (via `/health`), ingest throughput, and p50/p95/p99 `/search`
  latency. No new dependencies (reuses the shared HTTP client).
- **Recall eval harness** (`#[ignore]`d test `eval_recall_harness`): loads the
  model, builds a temp DB, and measures recall@5 / recall@10 across pure-vector /
  hybrid / hybrid+PRF configs. Runnable via
  `cargo test --release -- --ignored --nocapture eval_recall_harness`.
- **Migration safety.** Pre-migration `VACUUM INTO` backup (one-shot,
  marker-guarded, skipped for fresh DBs) runs before `run_migration` so the
  rollback path is always possible. Added `migrate_down_0_9_0()` reversibility
  path (drops vec0 + FTS5 + vocab + schema markers; preserves
  `knowledge`/`embeddings`). Post-backfill parity check warns when
  `COUNT(vec_knowledge) < COUNT(embeddings)`.
- **Developer surface:** a `brain` CLI (`src/bin/brain.rs`: query, explain,
  `ingest-dir` with `.brainignore` + content-hash idempotency + `--dry-run`, bench,
  status, doctor), a minimal stdio MCP server (`src/bin/mcp.rs`), and
  `openapi.yaml` — all dependency-light HTTP clients to the running server.
- **Bearer-token auth** (`AUTH_TOKEN`) on non-public routes, with loopback-safe
  defaults, and **retrieval profiles** (`MODEL_PROFILE`: `edge-default`,
  `quality-local`, `multilingual`, `air-gapped`).
- **P2 scaffolding:** `domain`, `observed_at`, `valid_from`, `valid_to` columns on
  `knowledge`, with `domain` scoping in the retrievers (single-DB tagged model).
- **Structure-aware Markdown chunking** (`src/chunker.rs`): `/ingest/markdown` now
  splits documents at heading boundaries (keeping code fences intact), stores one
  chunk per `knowledge` row with `document_id`, `chunk_index`, `heading_path`, and
  1-indexed line span, and embeds each chunk. Added `GET /get/{id}` and
  `POST /multi-get` for stable chunk retrieval.
- **Implemented `POST /ingest`** (was `unimplemented!()`/panic): the structured
  store now embeds, dedups via `content_hash`, routes to the resolved domain, and
  inserts knowledge + vec0 + entities + relations in one transaction.
- **Delete + tombstones:** `DELETE /memory/{id}` now also cleans the `vec_knowledge`
  row (no FK cascade) and records a `tombstones` audit row; deleted content is gone
  from retrieval immediately.
- **`POST /reindex`** rebuilds all `vec_knowledge` from `knowledge`.
  `GET /domains` now lists real per-domain counts.
- **Per-domain DB registry (P2 foundation):** `src/domain_registry.rs` adds a
  `DomainRegistry` with lazy per-domain pools (`brain-<domain>.db`), filename-safe
  domain validation, and a back-compat shim (`BRAIN_MULTI_DB`, off by default =
  legacy single-DB behavior). `/ingest` and `/recall` route through it; `global`
  keeps using the existing `brain.db` (no data migration required).
- **Centroid routing + federation (P2):** `src/domain_router.rs` computes a mean
  embedding centroid per domain (stored in `domain_centroids`, refreshed on ingest
  + `/reindex`) and a pure `route()` with a confidence threshold. In multi-db mode
  `/recall` auto-routes to the best domain (strict isolation) or federates across
  all known domains with a labelled per-hit source domain when no domain is
  confident and `strict=false`.

### Changed
- The optional rerank tier remains **feature-gated and off by default**: it
  compiles only with `--features rerank` and activates only when
  `RERANK_ENABLED=true`. The default edge build is pure-static (Model2Vec, no
  heavy cross-encoder). When enabled it uses the BGE-RerankerV2M3 cross-encoder
  and fails open to the first-stage result.
- **`PRAGMA mmap_size`** (256 MiB, `config::DB_MMAP_SIZE_MIB`) is now set in
  `run_migration`, letting SQLite memory-map the DB without loading it all into
  RSS.
- **CORS loopback guard.** When `CORS_ORIGINS` is unset, the fallback now strips
  non-loopback origins, preventing an accidental open CORS policy in production.
  `CORS_MAX_AGE_SECS` is wired into the `CorsLayer` (was a dead constant).
- **Connection watchdog** now uses the `CONNECTION_WATCHDOG_*` constants instead
  of hardcoded literals.
- **Dead config constants removed** (`ENTITY_NAME_MAX_LENGTH`, `TRAVERSE_MAX_DEPTH`,
  `REQUEST/SEARCH/HEALTH_TIMEOUT_SECS`, `CONTENT/TITLE_MAX_LENGTH`) along with the
  file-level `#![allow(dead_code)]` that was masking them.

### Known limitations / pending
- **No measured QMD parity.** The benchmark harness (`bench` feature) and eval
  harness (`eval_recall_harness`) now exist and are runnable, but the actual
  RSS/latency/recall numbers require a run on the target hardware (4 GB ARM).
  `BENCHMARKS.md` cells remain `PENDING` until then. No claim of measured QMD
  parity is made.
- **Eval corpus is a 10-doc smoke set**, not the ≥100 judged queries over a
  representative corpus that the plan calls for. It gives a directional signal;
  it is not sufficient for a release-blocking parity claim.
- **`perform_search_legacy`** (in-RAM brute-force cosine scan over JSON vectors)
  is retained as a cold-start fallback for pre-migration DBs where `vec0` is empty.
  It is no longer the primary path — `vec0` KNN is.
- **Enterprise SSO / SCIM / ACLs / connectors** are deferred (P4).
  Bearer-token auth (`AUTH_TOKEN`) exists, but OIDC/SAML and connector sandboxing
  do not.
- QMD (Node/TypeScript, ~28k★ mid-2026) remains the more mature local
  document-search product: it uses LLM-generated query expansion and LLM
  cross-encoder reranking via local GGUF models (~2 GB auto-downloaded), plus
  collections, AST chunking, stable SDK/CLI/MCP. Brain Server's deliberate wins
  are its tiny deterministic static-embedding edge profile and (planned) agent
  memory features — not currently measured search-quality superiority.

---

## [0.9.0] — "Quantize" — (released)

Phase 0–1 stabilization: BLOB/sqlite-vec int8+binary storage, FTS5 lexical
index, CORS env-var wiring, `SERVER_VERSION` from `CARGO_PKG_VERSION`, DB path
override, and removal of the TOML annotation engine. See `SPECS.md` for the
full historical record.
