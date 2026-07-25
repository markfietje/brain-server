# Changelog — brain-server

All notable changes are documented here. The format is a simplified keep-a-changelog
style. Version numbers follow `Cargo.toml`; "released" means the binary and docs
are consistent at that tag.

Honesty note: retrieval-quality claims below describe *what the code does*, not
measured parity against external engines (e.g. QMD). Where a benchmark has not
been run, it is marked **pending** rather than asserted.

---

## [Unreleased]

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
