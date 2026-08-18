# Features

Brain Server packs a lot of capability into a single Rust binary. This page is the complete feature tour — grouped by what the feature does for you. It is a **living inventory of what is shipped** (verified against the codebase up to v1.27.22); if a capability is described here, it exists in the current source.

## Retrieval

- **Hybrid retrieval** — vector KNN (`vec0`) + lexical FTS5 (BM25) fused via Reciprocal Rank Fusion, with deterministic PRF query expansion and full per-result provenance.
- **Structured query** — `QueryDoc` with `LexSpec` (phrases, exclusions, code paths), multi-source OR scope, temporal `since`/`as_of` predicates.
- **Optional graph leg** — Personalized PageRank over the knowledge graph as a third, opt-in `?graph=true` RRF leg (HippoRAG-2 style).
- **Noise-aware graph retrieval** (v1.12) — hub dampening + edge-type weights tame taxonomy-noise mega-hubs; the graph leg auto-engages as a rescue pass when the estimator says the query is ambiguous.
- **Calibrated abstention** (v1.5) — when retrieval quality is too low, `/recall` returns `{decision: "low_confidence", hits: []}` instead of top-1 garbage. No magic score cutoff — a calibrated multi-signal recommendation drives it.
- **Span verification** (v1.5) — `POST /verify` checks whether a claim is supported by a chunk's actual text (deterministic lexical match, no LLM).
- **Recall-gate QA** (`qa.rs`) — a pure scorecard that weighs in-scope / cited / confident / has-trace signals so an agent can decide when it has enough evidence to answer.

## Temporal & knowledge

- **Temporal evidence** — every ingest stamps `observed_at` / `valid_from` / `valid_to` / `authority`. Point-in-time recall returns the revision active at a timestamp.
- **Knowledge graph** — entities and relationships extracted from `[[relation::entity]]` syntax in markdown. Traverse, query, and follow links. `GET /graph/entity/{name}`, `GET /graph/relations`, `GET /graph/traverse`.
- **Faithful explanations** (v1.7) — `/graph/traverse?explain=true` returns structured hop chains (`A --works_at--> B --ceo_of--> C`), not a flat id string. Edge-type filter via `?kind=`.
- **Ordered procedures** (v1.10) — `POST /procedure` ingests a root + ordered steps in one transaction; `GET /procedure/{id}/steps` returns them via `next_step` edges.
- **Deterministic classification** (v1.10) — `POST /classify` routes text to a category by matched keywords (auditable); `POST /decision/{id}/evaluate` fires the matched branch of a stored decision rule. No LLM.

## Self-correction & maintenance

- **Self-correction** (v1.6) — operator-approved `supersedes` links atomically expire the prior fact; historical recall (`?at=<past>`) still returns it. `brain resolve` + `brain check-consistency` surface action items.
- **Automatic edge supersession** (v1.27.22) — re-ingesting a relation with a changed window retires the old edge (`superseded_at` set, old row preserved verbatim) and inserts the corrected belief; handoff is exact (`old.superseded_at == new.created_at`). Traversal + every graph read surface only current edges (no newer live same-triple row). `GET /graph/relationships/{id}/history` recovers the full version lineage (every version, four timestamps + `current` flag).
- **Reviewable proposals** (v1.8) — `/consolidate/propose` detects exact duplicates, subject conflicts, unresolved contradictions, **stale sources** (deleted vault files), and **near-duplicates** (cosine ≥ 0.95). `/consolidate/apply` applies, `/consolidate/undo` reverses prior resolutions without retrieval regression. `brain undo-resolve` drives the reverse.
- **Write-back gating** (v1.14) — `POST /ingest/proposal` scores a candidate (novelty via KNN, conflict via consolidation, salience via heuristics) but creates **no** `knowledge` row; it becomes memory only via human approval.
- **Approval binds to the displayed bytes** (v1.27.12) — `/proposals` serves the read-canonical review form (PII-redacted, markdown-ref-stripped, invisible-Unicode-free) plus a stable `content_digest`; approving with a stale digest is rejected (`409`), so a decision can never bless content that recall would render differently.

## Human in the loop

- **Meaningful control, not a checkpoint** — the human review is a real job with tooling, time, and consequences, built against the four failure modes of supervised automation (out-of-the-loop skill loss, automation bias, the explainability paradox, moral crumple zones). See [**Human in the loop**](./human-in-the-loop.md).
- **A reviewable, not rubber-stamped, queue** — every proposal card carries a novelty/conflict/salience breakdown, a PII-screened sourcing prompt, and a screen verdict; raw evidence (verbatim span, `source_uri`, revision, heading, line range) opens on demand via `GET /get/{id}`.
- **The queue is a clock** (v1.20.6) — the Memory Operations panel shows a live SLA countdown per pending proposal and a gate-health strip (over-rejecting / under-reviewing / expired) so review *load* and *drift* are visible, not hidden in a log.
- **Reviewer calibration** (v1.20.23) — the client computes approve-rate / median decision latency / edit-rate / screen-override-rate from `ProposalView.decided_at` and warns when the queue drifts into rubber-stamping.
- **Provenance ledger** (v1.20.9) — the Agent Memory Register partitions the store by `origin` (`human` / `model` / `imported`) with owner/source/kind filters and drill-down evidence, so how much of the store is model-originated is auditable at a glance.
- **Consequential and recorded** — every approve / reject / supersede / expire is appended to the SHA-256 audit chain, making each operator decision reconstructable. (A free-text reject rationale is a client-side affordance; the server records the decision itself, not the reason.)
- **Human-only erasure** — agents can read and propose, but *only a human can delete* memory. The `memory_forget` agent tool was removed (v1.20.25); erasure runs through the audited console / HTTP API paths (`DELETE /memory/{id}`, `POST /purge`, DSAR). The `ump.forget` tool is fence-gated by the legal-hold guard (`409 legal_hold_active` when the id is held).

## Anticipation & suggestions

- **Opt-in anticipation** (v1.9) — `POST /suggest` returns related-but-not-surfaced chunks (tagged `reason: "anticipated"`); `POST /suggest/feedback` records accept/dismiss; `GET /suggest/metrics` reports the false-positive rate. No push, no decay, no hidden personalization — the agent asks explicitly.

## Source lifecycle & connectors

- **Source lifecycle** — every chunk carries provenance (`source` + immutable `revision`). Connectors backfill external sources through a supervised pipeline; `POST /sources/reconcile` sweeps orphans from deleted sources; `DELETE /sources/{id}` retires a source.
- **Connectors** (v1.24) — a profile-gated registry (`POST /connectors/register`) over a fixed vocabulary (CRM / Slack / Jira-Linear / read-only HRIS-EHR / GitHub) with a shared supervised translate+ingest pipeline. The `github` connector is the only runnable network backfill binary; the others ship in registry + translate-template form. Reconcile is never auto-sync; translated records flow through the injection screen (poisoned records quarantine, not memory).

## Governance, privacy & compliance

- **Append-only audit log** — ingest and auth-denial events recorded hash-only in a SHA-256 hash chain; `GET /audit` reads it, `GET /audit/verify` verifies the whole chain.
- **Prompt-injection quarantine** — suspicious content stored but excluded from retrieval until reviewed. `GET /quarantine` lists it; `POST /quarantine/{id}/release` / `/delete` resolve it. The quarantine flag is one-shot at construction and rides a `#[serde(skip)]` flag through every read seam (a recalled chunk cannot forge or lose its taint).
- **Read-event audit** (v1.15) — recall/search/get emit rows into the hash chain (opt-in), plus a replayable **recall trace** (`GET /recall/{trace_id}/trace`).
- **DSAR workflow** (v1.15) — `POST /dsar` locate → export → purge → chain-verifiable deletion certificate; `GET /dsar` ledger (per-row deadline); `GET /tombstones` registry; `GET /dsar/{id}/certificate` re-fetches the certificate + live chain check. `dry_run` returns a write-free `Footprint` preview. Per-jurisdiction deadlines via `JurisdictionRule`.
- **GDPR export/purge** (v1.14) — `GET /export` portable JSON; `POST /purge` hard audited delete by id or owner.
- **PII controls** (v1.14) — deterministic read-time output redaction (`[redacted:…]`); no write-time placeholder vault (v1.20.19).
- **Profiles** (v1.21) — a Profile is a typed JSON bundle of *existing* knob defaults (default access scope, PII posture, per-kind retention, audit level, kind vocabulary). Apply invariant: **the profile sets defaults, the row wins.** A bound profile's `retention` block replaces the server-wide policy for that domain. `GET /profiles`, `GET|POST /profiles/{name}`. 12 USE_CASES presets seeded.
- **Roles** (v1.23) — named bundles of scopes + default panel visibility + an action `can` allowlist, mapped onto the existing `access_scope`/`owner` mechanism. Role names come from the JWT `roles` claim; definitions live in the editable `roles` store. `GET /roles`, `GET|POST /roles/{name}`. Role-gated console views in the client.
- **Legal hold** (v1.22) — freeze a knowledge id against every erasure path (decay, `/purge`, DSAR) until every hold is explicitly released. `POST /legal-hold`, `POST /legal-hold/{id}/release`, `GET /legal-holds`. Held ids are *deferred* (never purged) and reported on the DSAR certificate's `held_ids[]`.
- **Retention** (v1.17.1 / v1.22) — per-kind `ttl_days` decay marks expired rows into `/decayed`; the client surfaces "next to expire". `GET`/`POST /retention` edits the policy; `GET /retention/report` is the per-domain × kind → count → expiring-within-30d evidence report; `GET /art30` emits the Article 30 processing record.
- **Cross-border transfers** (v1.26) — the evidence + tagging layer for a PH BPO serving US/UK/EU/AU/SG/CA clients: a validated transfer register (`POST`/`GET /transfers`, curated mechanism + jurisdiction vocabularies), per-jurisdiction DSAR deadlines, and pre-filled **TIA** (`/transfers/{id}/tia`, Schrems II) + **DPA** (`/transfers/{id}/dpa`, Art 28) templates a human DPO signs. Honestly framed: evidence, not enforcement.
- **Breach notification** (v1.25) — human-opened (by the DPO role) append-only incident workflow with a notification/knowledge event log, per-jurisdiction notification deadlines, and every event hash-chained into the audit. `POST /breach`, `/breach/{id}/event`, `/breach/{id}/close`, `GET /breaches`, `GET /breaches/{id}`.
- **BPO client register** (v1.27) — one row per operating client (name, isolation domain, jurisdiction, bound profile, status) in the global DB — the spine of the BPO arc. `POST`/`GET /clients`, `GET /clients/{name}`, per-client DSAR (`/clients/{name}/dsar`), legal hold (`/clients/{name}/hold`), and termination (`/clients/{name}/end`). Client-auditor role tokens see only their granted domains (`read:team/*` wildcards only reach the shared `global` pool).
- **Supervisor QA queue** (v1.27.8) — `/clients/{name}/proposals` (same `ProposalView` shape as `/proposals`) + `POST /clients/{name}/proposals/{id}/coach` coaching notes, so a supervisor can review an agent's proposed memories before promotion.

## Domains & routing

- **Domain isolation** — each knowledge domain is its own SQLite pool (`POST /domains`). `GET /domains`, `DELETE /domains/{name}` (echo-confirm), `POST /domains/{name}/vacuum`, `GET /domains/{name}/export` (consistent `VACUUM INTO` snapshot), `POST /domains/{name}/import` (restore into a NEW domain), `POST /domains/recompute` (one-shot centroid sweep), `POST /domains/move` (relabel chunks).
- **Capacity envelopes** — a config exceeding a documented capacity refuses new ingests with HTTP 507; read routes are never blocked.
- **Alert feed** — decision-critical events (pending/expiry/injection/chain-verify) stream to the `/ops` panel via SSE (`GET /events`) and optionally to a signed webhook (`BRAIN_ALERT_WEBHOOK_URL`).
- **Observability** — `GET /health` (+ capacity + hardening incl. the monotonic `audit_commit_failures` counter), `/health/db`, `/ready`, `/version`, `/stats`, and Prometheus text `/metrics` (auth-gated).

## Security

- **Two authentication modes** — opaque bearer (default) or JWT/JWS (opt-in), with per-route AuthZ, record-level access scoping, and fail-closed identity (poisoned auth store → 500, configured-but-empty → 401, role-store outage → deny). `GET /roles` resolves capabilities.
- **Fail-closed erasure + fence** (v1.27.21) — the legal-hold fence guards every erasure path including `POST /ump/forget {"hard":true}` and the ingest-replace/vault sweep; empty `live_uris` reconcile requires `allow_empty: true`; `read:<team>/*` wildcard grants only the shared pool; a no-role token passes `require_dpo_role` only when no roles are defined at all.
- **Atomic token rotation** (v1.27.12) — `brain token rotate` replaces the bearer token via a 0600 temp file (fsync + rename); the server fails closed on group/world-readable tokens and signing keys.
- **Per-IP rate limiting** (v1.27.16) — a distinct bucket per peer `SocketAddr` (bounded key set, oldest-evicted), not a single shared global limiter.
- **Provenance-labeled recall** (v1.27.12) — recalled context carries per-hit `source` / `node_kind` / `lawful_basis` / `region` tags inside the `UNTRUSTED_*` fence, so the model can attribute — not just trust — what it recalls. The same `strip_sentinels` + `sanitizeForBlock` seam strips invisible/zero-width/bidi characters on the MCP envelope, CLI prints, and plugin render boundary.
- **Verified webhooks** — HMAC verification, replay-window enforcement, idempotency, signed sinks fail closed on wide permission modes.
- **Encrypted backup/restore** — AES-256-GCM with an Argon2id-derived per-backup key, GCM AAD header binding, 0600 + `create_new` snapshot hygiene (fail-closed, never clobbers a live file). Backup format v3 default (`--format v1|v2|v3`); v1/v2 files stay readable.
- **AI transparency + SSO discovery** — `/.well-known/ai-notice`, `/.well-known/security.txt`, `/.well-known/openid-configuration`, `/.well-known/jwks.json` for JWT/OIDC mode.

## Integration surface

- **OpenAI-compatible embeddings** — `POST /v1/embeddings`.
- **MCP server** — `mcp` binary exposes search/recall/ingest plus the UMP family (`ump.remember/revise/forget/feedback/recall/get/audit/capabilities`) as MCP tools.
- **`brain` CLI** — the operator surface: status, doctor, query, explain, get, ingest-dir, reconcile, resolve, undo-resolve, check-consistency, classify, procedure, evaluate, suggest (+feedback/metrics), retention, domains (move/recompute), clients, ump, backup, restore, token, key, setup, sync, connector-status, snapshot-status, eval, bench, and more. `--json` envelope mode on data commands.
- **UMP 1.0** — a full implementation of the open Universal Memory Protocol at conformance **L3** (L2 without an operator key): signed records, capability tokens, HTTP + MCP + file bindings, `GET /ump/capabilities`, `/ump/remember` / `revise` / `forget` / `feedback` / `recall` / `memory/{id}` / `subscribe` / `audit`.
- **Client control surface** (v1.16+) — a Dioxus app (web + desktop + iOS + Android) with connection state machine, honest-batch review (A/S/R/J/K), recall decision-path viewer, DSAR certificate card, auth-failure feed, audit filters + export, live SLA clocks, role-gated console views, and an i18n-clean WCAG 2.2 AA interface.
- **OpenClaw plugin** — `brain-server/plugin/` (TypeScript) calls `/recall` each turn via openclaw's `before_prompt_build` hook, renders recalled context inside the `UNTRUSTED_*` fence, and offers the offline-queue + token-ladder posture.

## Next steps

- See how it all works in **[Architecture](./architecture.md)**.
- Try the **[Quickstart](./quickstart.md)**.
- Browse the **[API Reference](./api.md)**.
