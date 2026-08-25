# API

Brain Server exposes a versioned HTTP API. Every response carries an
`X-Api-Version` header. This page is the informational overview; the complete,
machine-readable contract is at **`GET /openapi.yaml`** at runtime and
[openapi.yaml](https://github.com/markfietje/brain-server/blob/main/openapi.yaml) in the repo, with the full written contract in
[API_CONTRACT.md](./API_CONTRACT.md).

---

## Core routes

| Method | Path | Purpose |
|---|---|---|
| GET | `/health` | Liveness probe (minimal `{status, version}`; detail on `/health/db`) |
| GET | `/ready` | Readiness probe for load balancers |
| GET | `/health/db` | Read-gated detail — capacity, pool, hardening, model, otel, DPO |
| GET | `/stats`, `/version` | Counts, model, version |
| GET | `/openapi.yaml` | Full API contract |
| GET | `/.well-known/security.txt` · `/openid-configuration` · `/jwks.json` | RFC 9116 disclosure file, OIDC discovery (RFC 8414), JWKS key set (RFC 7517) — all public, no auth |
| GET | `/.well-known/ump.json` · `/ai-notice` · `/ai-literacy` · `/cop-notice` | UMP discovery + EU AI Act transparency notices (Art 4 literacy, Art 50, CoP self-attestation) — all public |
| POST | `/v1/embeddings` | OpenAI-compatible embeddings endpoint |
| POST | `/ingest/memory` | Structured memory ingest |
| POST | `/ingest/markdown` | Markdown ingest + graph extraction |
| POST | `/ingest` | Structured ingest (explicit entities/relations) |
| POST | `/sources/reconcile` · DELETE `/sources/{id}` | Sweep deleted sources / retire a source |
| POST | `/recall` | Structured recall — the primary endpoint |
| GET | `/search` | Semantic search *(deprecated; use `/recall`)* |
| GET | `/get/{id}` · POST `/multi-get` | Fetch chunk(s) by id |
| GET | `/recall/{trace_id}/trace` | Recall-trace replay (decision-path evidence) |
| POST | `/verify` | Span verification — is a claim supported by a chunk's text? Binds the `X-Brain-Domain` label in SQL (an id cannot cross domains in shim mode) + the record gate. |
| POST | `/reindex` | Rebuild indexes |
| GET | `/metrics` | Prometheus metrics (auth-gated) |
| GET | `/events` | SSE broadcast of memory events; `?kinds=` filters. Since 1.28.19 the bus also carries drained `workflow/*` outbox events under kind `workflow` — additive + default-off (only explicit `?kinds=workflow` subscribers receive them), per-subscriber run-domain Read-gated at fan-out, payloads sanitized before broadcast |
| POST | `/webhooks/{kind}` · `/webhooks/gh` | Webhook delivery receiver (HMAC-verified) |

---

## Retrieval

**`POST /recall`** takes a structured query document (`QueryDoc`; the `query`/`limit`
fields are the `/recall`-specific ones — `q`/`k` are the `GET /search` equivalents):

```json
{
  "query": "blueberry alternative",
  "limit": 5,
  "sources": ["memory", "vault"],
  "provenance": true,
  "graph": false
}
```

- **Lexical control** — a `LexSpec` with terms, quoted phrases, exclusions
  (`-"..."`), and exact code paths.
- **Filters** — `source`/`sources` (ingest kind), `since` (ISO timestamp),
  `domain`, `min_relevance`, `include_decayed`.
- **Provenance** — per-retriever ranks, fused score, expansion terms, and
  per-hit `source` / `node_kind` / `lawful_basis` / `region` tags (present when
  stored; absorbed into the `RecallHit` wire shape, v1.27.12).
- **Abstention** — returns `{decision: "low_confidence", hits: []}` rather than
  top-1 garbage when quality is too low.

---

## Knowledge graph

| Method | Path | Purpose |
|---|---|---|
| GET | `/graph/entity/{name}` | Entity + 1-hop relations |
| GET | `/graph/relations?from=&to=` | Relations between entities |
| GET | `/graph/traverse?start=&max_depth=&explain=&kind=` | Bounded walk (depth ≤ 4); `explain=true` returns structured hop paths; `kind=` filters by edge type |
| GET | `/graph/relationships/{id}/history` (Admin) | Edge supersession lineage — every version of an edge triple (v1.27.22) |

---

## Governance & write-back

| Method | Path | Purpose |
|---|---|---|
| POST | `/ingest/proposal` · `/proposals/{id}/approve[?supersedes=N][&digest=...]` · `/reject` · `/proposals/{id}/edit` | Human-in-the-loop write-back (v1.14). Since v1.27.12 `approve` accepts an optional `digest` (SHA-256 of the read-canonical review form, as served by `GET /proposals`); any drift → `409` — the approval binds to the bytes the reviewer saw |
| GET | `/proposals?status=` · `/decayed` | Approval queue + decayed review. Each row is a `ProposalView` (`content` = read-canonical form, `content_digest` = SHA-256 the approve verb binds to, v1.27.12) |
| POST | `/consolidate/propose` · `/apply` · `/undo` | Reviewable consolidation, supersession, undo |
| POST | `/suggest` · `/suggest/feedback` · GET `/suggest/metrics` | Opt-in anticipation + false-positive metric |
| POST | `/verify` | Claim span verification |
| POST | `/classify` · `/decision/{id}/evaluate` | Deterministic categorization / decision rules |
| POST | `/procedure` · GET `/procedure/{id}/steps` | Ordered procedures (steps bind the `X-Brain-Domain` label + record gate) |

---

## Profiles, roles & connectors (policy)

| Method | Path | Purpose |
|---|---|---|
| GET | `/profiles` · GET/POST `/profiles/{name}` | Preset system (v1.21): fetch/upsert a typed knob bundle |
| GET | `/roles` · GET/POST `/roles/{name}` | Role postures + capability sets (v1.23) |
| GET | `/connectors` | Registered connector registry (v1.24) |
| POST | `/connectors/register` | Validate + register a connector against the domain's profile gate (v1.24) |

---

## Privacy & audit

| Method | Path | Purpose |
|---|---|---|
| GET | `/export` | Portable JSON export |
| POST | `/purge` | Hard, audited deletion by id or owner |
| DELETE | `/memory/{id}` | Hard, audited deletion of one chunk (human-only erasure; the agent tool was removed v1.20.25) |
| POST | `/dsar` | Locate → export → purge → deletion certificate (supports `dry_run` footprint preview) |
| GET | `/dsar` | DSAR ledger (admin, newest-first, per-row deadline) |
| GET | `/tombstones?subject=&since=` | Deletion registry |
| GET | `/dsar/{id}/certificate` | Re-fetch certificate + live chain check |
| GET | `/audit` · `/audit/verify` | Append-only audit log + chain integrity (v1.27.31: verify covers every registered domain; rows carry their `domain` tag in multi-db mode) |
| GET | `/quarantine` · `/quarantine/{id}/release` · `/delete` | Injection review |
| GET | `/retention` · POST `/retention` · GET `/art30` · GET `/retention/report` | Per-kind retention policy + Art 30 record + per-domain×kind retention report |
| GET | `/snapshot/status` | Point-in-time snapshot state |

---

## Domains & routing

| Method | Path | Purpose |
|---|---|---|
| POST | `/domains` | Create a domain pool (200 = existed, 201 = created; body `{domain}`) |
| GET | `/domains` | List known domains (single global pool when multi-db is off) |
| DELETE | `/domains/{name}?confirm=<name>` | Delete a domain + all its data (echo-confirm guard, `global` protected) |
| POST | `/domains/{name}/vacuum` | `VACUUM` one domain pool (returns `{name, vacuumed: true}`) |
| GET | `/domains/{name}/export` | Consistent SQLite snapshot download (`VACUUM INTO`, `attachment; filename="brain-<name>.db"`) — Read in multi-db; **Admin in shim mode** (the snapshot is the whole shared pool there) |
| POST | `/domains/{name}/import` | Restore a snapshot into a NEW domain (raw bytes body; 201 `{name, imported: true, bytes}`) |
| POST | `/domains/recompute` | One-shot centroid recompute sweep over every domain (`{recomputed: [[domain, n], …]}`) |
| POST | `/domains/move` | Move chunks to another domain |

---

## UMP (Universal Memory Protocol)

| Method | Path | Purpose |
|---|---|---|
| GET | `/ump/capabilities` | Protocol negotiation (conformance level, retrieval signals, `max_recall`, writable, audit) |
| POST | `/ump/remember` · `/ump/revise` · `/ump/forget` · `/ump/feedback` | Record / patch / soft-delete / outcome-feedback |
| POST | `/ump/recall` | Ranked recall with per-result signals |
| GET | `/ump/memory/{id}` | Read one record with on-read integrity re-verification |
| GET | `/ump/subscribe` | SSE broadcast of memory events |
| POST | `/ump/audit` · GET `/ump/audit/verify` | UMP-scoped audit row family + chain verification |

---

## Legal hold & breach (v1.22 / v1.25)

| Method | Path | Purpose |
|---|---|---|
| POST | `/legal-hold` · `/legal-hold/{id}/release` · GET `/legal-holds` | Per-domain legal holds; held ids are frozen (purge/DSAR defer) |
| POST | `/breach` · `/breach/{id}/event` · `/breach/{id}/close` | Breach-notification workflow (open / append event / close) |
| GET | `/breaches` · `/breaches/{id}` | Breach register + detail |
| GET | `/workflow/scoreboard` | Workflow outcome/efficiency scoreboard over recent runs (DPO/admin; rates in integer ten-thousandths, fail-closed audit linkage) |
| POST | `/workflow/calibration/sign` | Monthly human-signed workflow calibration gate (DPO/admin; one signature per calendar month, audited) |
| GET | `/workflow/runs/{id}` · `/workflow/runs/{id}/steps` · `/workflow/runs/{id}/suggestions` | Run row (state sanitized at the read seam), steps, retrieval-backed suggestions (Read on the run's domain) |
| POST | `/workflow/runs/{id}/steering` | Queue a steering message: blocklist-screened, Write + approve-class role gate, bounded inbox drop-oldest at 100 |
| POST | `/workflow/runs` | Open a governed run (`{domain, kind, state_json}` → `{run_id, revision}`); Write + `workflow` role gate; open + audit row commit atomically |
| GET | `/workflow/runs/{id}/state` | Engine-exact `{state_json, revision}` (machine CAS round-trip; NOT read-seam sanitized — the human view is `GET /workflow/runs/{id}`); Read + `workflow` role gate; audited read |
| PUT | `/workflow/runs/{id}/state` | CAS advance (`200 {revision}` / `409 {actual_revision}`); Write + `workflow` role gate |
| POST | `/workflow/runs/{id}/events` | Outbox enqueue, exactly-once by idempotency key (`{first, event_id}`; optional `parent_event_id` links ancestry); Write + `workflow` role gate |
| GET | `/workflow/runs/{id}/events?branch=` | The lineage read: ordered events with `parent_id` links (Read on the run's domain); `branch=<event_id>` narrows to that event's ancestor chain, root-first; `since=<event_id>` backfills a reconnect gap |
| GET | `/workflow/runs/{id}/context?at_event=&budget=` | The derived context window (Fathom): latest checkpoint at-or-before the anchor + delta + finding digests + open question; field-budgeted, delta drops oldest-first (`truncated` flag) — the consumer contract for unbounded sessions (Read on the run's domain) |
| POST | `/workflow/runs/{id}/rewind` | Rewind = branch, never delete: verify the target is a `workflow/checkpoint` event (or the run root), CAS-restore its state snapshot appending a `branches[]` marker, audit — one tx (`{ok, revision, branched_from}`); Write + approve role gate |
| GET | `/workflow/runs/{id}/handoff` | The I-PASS handoff packet assembled from the run's records (illness/patient/action/situation/safety + `handoff_complete = status=="completed"`); Read on the run's domain |
| POST | `/workflow/runs/{id}/handover/offer` | Relay: offer a one-click handover `{to_principal, overlap_minutes?}` — gated by the packet-completeness check (open question, un-breached SLA, current step, linked evidence/checkpoint, resolved escalation); an incomplete packet refuses `400 packet_incomplete` with `details.missing` and writes nothing. Offer + lineage event (`workflow/handover`) + audit land in one tx; retried POSTs are idempotent (Write on the run's domain + `workflow` role gate) |
| POST | `/workflow/runs/{id}/handover/{offer_id}/accept` | Accept an offer: in ONE WorkflowTx the offer state moves and the run `owner` CAS-transfers to the acceptor; the SLA clock is untouched and the reply points at the resume-at checkpoint. Deciding a decided offer replays `{moved:false}` (Write on the run's domain) |
| POST | `/workflow/runs/{id}/handover/{offer_id}/decline` | Decline an offer with a REQUIRED reason `{reason}` — screened, ≤ 4000 chars, stored + audited (an audited refusal beats a silent bounce). `400 reason_required` / `reason_too_long` (Write on the run's domain) |
| GET | `/ops/handovers?domain=&now=` | The follow-the-sun board: active runs ranked by SLA remaining (recorded deadline wins, else P3-from-created), flagged while `now` sits inside the ring boundary's derived overlap window (Read on the domain) |
| POST | `/workflow/runs/{id}/notes` | Channel: post a case note `{content}` — screened at write (empty/≤4000/prompt-injection blocklist) and stored through the invisible-strip + markdown-ref seam; `@skill:<tag>` / `@principal` mentions resolve into swarm invites (invite row + `case/note` lineage event that drains to `/events` as the Crew ping — visible to `?kinds=workflow` subscribers holding Read on the domain). Dead mentions refuse `400 mentions_unresolved` with the list; >16 resolved invitees refuse `400 invite_limit`. Note + invites + events + audit land in ONE tx (Write on the run's domain) |
| GET | `/workflow/runs/{id}/notes?limit=&offset=` | The channel view: chronological notes + invites for one run, policy-expired rows hidden before the page split (`case-note` retention kind), every string on the read seam, bounded page 1..=500 (Read on the domain) |
| POST | `/workflow/runs/{id}/notes/{invite_id}/accept` | Accept an invite into the channel: CAS `pending → accepted` on the invite row in one tx with its lineage event + audit; replaying a decided invite returns `{moved:false}`. Ownership never moves (Write on the run's domain) |
| POST | `/workflow/runs/{id}/answer` | The AskHuman closer: digest-bound to the live `pending_question`, appends `answers[]`, clears the question, CAS — one tx; Write + approve role gate |
| GET | `/workflow/runs/{id}/steering?since=` | Drain the advisory steering outbox (Read on the run's domain) |
| POST | `/workflow/plugins/mount` | UI-plugin mount/unmount evidence (Art 12 record-keeping): server verifies the claimed bundle SHA-256 against the boot manifest before writing the audited row (`409` on uncertified bytes) |

### KCS article lifecycle (Evolve)

Every solved case can become knowledge; the capture generator emits HITL
proposals (`kcs_new_article` / `kcs_update_article` / `kcs_link_only`) that a
human approves through `/proposals/{id}/approve`. Approved articles are born
`kcs_state='draft'` — nothing auto-publishes.

| Method | Path | Description |
|---|---|---|
| GET | `/kcs/articles?state=&stale=1` | The content-health worklist: KCS-carrying articles, filterable by lifecycle state; `stale=1` keeps articles past their freshness-review deadline or carrying open improve flags (Read, per-domain visibility) |
| POST | `/kcs/articles/{id}/approve` | Move a draft article to `approved`, stamping the 90-day freshness-review deadline (Write on the domain + `approve` role; `409` when not draft; audited in-tx) |
| POST | `/kcs/articles/{id}/publish` | Propose publishing to the public KB (`kcs_publish` proposal; approval needs `approve` + the distinct `publish` capability). `action=retract` returns a published article to `approved` — the next build drops its page (Write to propose) |
| GET | `/kcs/articles/{id}/preview` | The exact sanitized public page for an approved/published article — same render path as `brain kb build`, unconditional PII redaction, no operator bypass (Read) |
| GET | `/ops/shifts?domain=&now=` | The shift-ring view: which site owns the queue at `now` (`queue_scope_site` re-scopes to the incoming site at the start of the derived overlap window — the queue follows the sun, cases don't), overlap state, next boundary, and the newest 500 shifts for the domain. Deterministic read-time arithmetic; no scheduler daemon (Read on the domain) |
| POST | `/ops/shifts` | Declare a site's on-call window `(site, tz, start/end epoch, overlap_minutes ≤ 120, roster)`. `400` on bad window/overlap/tz/roster bounds (tz ≤ 64 chars, roster ≤ 64 ids × ≤ 256 chars), `409 shift_double_booked` when the window starts before the earlier shift's final overlap period; validation + insert + audit ride one tx (Admin — pure operator configuration). Read capped at the newest 500 shifts |
| GET | `/ops/crew?domain=&now=` | The crew roster: TTL-decayed presence (active < 5 min, away < 30 min, offline beyond — computed at read; no background worker), Watchbill site badges from the shift ring, role + skills tags. Presence shows the KIND of act only (closed vocabulary: cranking/reviewing/idle) plus an opaque `current_case_ref` — never case content. Hidden entirely when the DPO switch is off or unreadable (Read on the domain) |
| POST | `/ops/skills` | Propose a skills change `{principal, add[], remove[]}` → one pending `crew_skills_update` proposal. Tags are lowercase alnum+hyphen, ≤ 32 chars, ≤ 32 per principal; approval (HITL `approve`) is the ONLY write path to `principal_skills`, applying the change in the approval transaction (Write on the domain) |
| POST | `/ops/crew/config` | The DPO presence switch `{domain?, presence_enabled}` — off (or unreadable) means every roster reads empty. Flip + audit ride one tx (Admin on the domain) |

### Public knowledge base (Beacon)

The public KB is a **generated static artifact**, never a live data path:
`brain kb build --domain <d> --out <dir>` emits a deterministic static site
(article pages under strict sanitization, index, client-side-only search
index, sitemap, robots, 404, redirect pages for superseded slugs, and a
SHA-256 `kb_manifest.json`). The operator hosts it and verifies the hosted
bytes against the manifest. On-page "Did this solve it?" votes return through
an operator-hosted relay into `POST /webhooks/kb-feedback`
(Standard-Webhooks HMAC-gated via `BRAIN_KB_FEEDBACK_SECRET_FILE`; aggregate
counters only — no visitor identifiers by construction); deflection is
indicative only, see `docs/kb-deflection.md`.

The engine itself lives in `tools/steward-harness` (0.2.0 "FirstLight"): a
human-cranked loop (`brain workflow crank <run>`) that drives these routes
through the SDK `WorkflowHost` seam. No engine code runs in the server.

---

## Compliance pack (feature-gated)

These routes exist only when the binary is built with `--features compliance-pack`
(`scripts/install-service.sh` adds it by default). Without the feature the router
is empty — the paths return 404, they are not auth failures.

| Method | Path | Purpose |
|---|---|---|
| GET | `/compliance/inventory` | AI-system inventory (Art 12/13 record-keeping register) |
| POST | `/compliance/evaluation-record` | Persist one evaluation evidence record |
| GET · POST | `/ropa` | Records-of-processing-activities register |
| POST | `/ropa/{id}` | Upsert a RoPA entry |
| GET | `/audit/export` | Full audit export (JSONL + labelled PDF), every row tagged with its owning domain |

---

## Cross-border transfers (v1.26)

| Method | Path | Purpose |
|---|---|---|
| POST | `/transfers` · GET `/transfers` | Register / list cross-border transfers (validated mechanism + jurisdiction) |
| GET | `/transfers/{id}/tia` | Transfer-impact assessment (Schrems II, pre-filled evidence) |
| GET | `/transfers/{id}/dpa` | Data-processing agreement (Art 28, pre-filled evidence) |

---

## Clients register (v1.27 BPO)

| Method | Path | Purpose |
|---|---|---|
| POST | `/clients` · GET `/clients` | Register / list clients (one domain per client) |
| GET | `/clients/{name}` | Client detail (client-auditor: row-filtered to granted domains) |
| POST | `/clients/{name}/dsar` | Per-client jurisdiction-aware DSAR + certificate |
| POST | `/clients/{name}/hold` | Per-client legal hold (resolves the client's domain) |
| POST | `/clients/{name}/end` | Termination: purge-or-return + archive + certificate |
| GET | `/clients/{name}/proposals` · POST `/clients/{name}/proposals/{id}/coach` | Supervisor QA queue (same `ProposalView` shape as `/proposals`) + coaching note (v1.27.8, Admin) |

---

## Auth & discovery (JWT mode)

| Method | Path | Purpose |
|---|---|---|
| POST | `/auth/refresh` · `/logout` · `/revoke` | Token lifecycle |
| GET | `/.well-known/openid-configuration` · `/.well-known/jwks.json` | OIDC + JWKS |

---

## Versioning & deprecation

- Every response carries `X-Api-Version`.
- `POST /add` and `GET /search` are deprecated (migrate to `/ingest` + `/recall`)
  and emit an RFC 8594 `Deprecation` header.
- The written contract ([API_CONTRACT.md](./API_CONTRACT.md)) states the
  stability promise and the deprecation policy.

---

## Tooling clients

- **`brain` CLI** — status, query, get, explain, ingest-dir, reconcile, retention,
  domains, ump, backup/restore, key management, and more (see
  [CLI reference](./cli-reference.md)).
- **`mcp` binary** — search/recall/ingest exposed as MCP tools for agent clients.
- **Dioxus client** — the visual control surface served at `/app`.

---

## Next steps

- [Quickstart](./quickstart.md) — working examples.
- [Architecture](./architecture.md) — how the endpoints map to the engine.
