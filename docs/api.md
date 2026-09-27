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
| GET | `/` · `/app/*` | The Dioxus console SPA, served from `BRAIN_CLIENT_DIST` when built and mounted (static asset surface — the JSON API routes are unaffected) |
| GET | `/health` | Liveness probe (minimal `{status, version}`; detail on `/health/db`) |
| GET | `/ready` | Readiness probe for load balancers; includes the redacted `gdl_provider` posture (`disabled`, `configured`, or `invalid`; invalid is `NOT_READY`) |
| GET | `/health/db` | Admin-gated detail (v1.28.70: the full body — capacity, pool, hardening, model, otel, DPO, concurrency, durability — is operator telemetry); a Read credential gets the reduced probe `{status, version, db_ok}`; Read-only dashboards add the admin credential for the full body |
| GET | `/stats`, `/version` | Counts, model, version |
| GET | `/openapi.yaml` | Full API contract |
| GET | `/.well-known/security.txt` · `/.well-known/openid-configuration` · `/.well-known/jwks.json` | RFC 9116 disclosure file, OIDC discovery (RFC 8414), JWKS key set (RFC 7517) — all public, no auth |
| GET | `/.well-known/ump.json` · `/.well-known/ai-notice` · `/.well-known/ai-literacy` · `/.well-known/cop-notice` | UMP discovery + EU AI Act transparency notices (Art 4 literacy, Art 50, CoP self-attestation) — all public |
| POST | `/v1/embeddings` | OpenAI-compatible embeddings endpoint |
| POST | `/ingest/memory` | Structured memory ingest |
| POST | `/ingest/markdown` | Markdown ingest + graph extraction |
| POST | `/ingest` | Structured ingest (explicit entities/relations). Since v1.28.74 accepts optional `origin_context: "owner"\|"channel"` — absent = owner (byte-compat); `channel` stores the row with origin `channel-capture` (recall labels it; the plugin can exclude); any other value is `400` |
| POST | `/sources/reconcile` · DELETE `/sources/{id}` | Sweep deleted sources / retire a source |
| POST | `/recall` | Structured recall — the primary endpoint |
| GET | `/search` | Semantic search *(deprecated; use `/recall`)* |
| GET | `/get/{id}` · POST `/multi-get` | Fetch chunk(s) by id |
| GET | `/recall/{trace_id}/trace` | Recall-trace replay (decision-path evidence) |
| POST | `/verify` | Span verification — is a claim supported by a chunk's text? Binds the `X-Brain-Domain` label in SQL (an id cannot cross domains in shim mode) + the record gate. |
| POST | `/reindex` | Rebuild indexes |
| GET | `/metrics` | Prometheus metrics (auth-gated) |
| GET | `/events` | SSE broadcast of memory events; `?kinds=` filters. Since 1.28.19 the bus also carries drained `workflow/*` outbox events under kind `workflow` — additive + default-off (only explicit `?kinds=workflow` subscribers receive them), per-subscriber run-domain Read-gated at fan-out, payloads sanitized before broadcast. Since v1.28.72 an authorization failure is an HTTP `403` BEFORE the stream opens (was: 200 + in-band error event); mid-stream failures still arrive as `error` events |
| POST | `/webhooks/{kind}` | Webhook delivery receiver (HMAC-verified). `kind` is the connector vocabulary — e.g. `gh` resolves the same handler as the historical `/webhooks/gh` path |
| POST | `/webhooks/channel/{kind}` | Channel bridge inbound (v1.28.43 Switchboard): Standard-Webhooks HMAC against the bridge's own 0600 `channel-{kind}-{tenant}.json` secret → replay-cap on (bridge, external_id) → sanitize + injection-screen BEFORE threading → thread map / auto-opened `care/case` under the bridge domain (`[case N]` overrides) → screened case note + audit row. Since Caravel: `attachment_digests[]` (≤8 × SHA-256 hex64) are recorded verbatim ON the landed note — media bytes stay quarantined on the edge, never proxied; a `status {state∈[sent,delivered,read,failed], ref}` projection lands ONE `case/channel_status` lineage event (refs never bodies); a `quality {number_alias, old_tier?, new_tier}` projection audits + fires a metadata-only operator alert on downgrades. The subscription handshake (`hub.challenge`) is answered by the EDGE process and never reaches the kernel |
| POST | `/webhooks/channel/{kind}/drain` | Bridge crank pulls approved outbound envelopes from the `channel/out` topic (approved acts or consented alert forwards ONLY; never the metadata-only alert bus, never SSE); batch marked delivered atomically. The drained `source_payload` now carries `kind` + `template` so edges deliver template acts as templates. Since Herald the response also carries `pings[]` — Relay handover pings with the receiving operator's mapped platform refs, the case room, and the I-PASS completeness state (refs only, never case content) |
| POST | `/webhooks/channel/{kind}/drain/ack` | The at-least-once close of the drain (v1.28.78): the bridge confirms delivered `event_id`s and those rows mark delivered. Same HMAC seam as the drain; acks are thread-scoped (a bridge cannot ack another bridge's rows) and idempotent — unacked rows stay pending and redrain |
| POST | `/webhooks/channel/{kind}/console` | Bridge-relayed operator console (Herald): the same HMAC seam carrying the console INTO the kernel for an operator in Slack/Teams. Closed action vocabulary — `pending` (renderable proposals with the canonical review digest; requires the mapped actor's `read` capability — since v1.28.69 every console action role-checks), `decide` (approve/reject + digest + actor_ref), `due` (valet due listing), `crank` (bounded steward-harness crank). The kernel resolves every actor through the proposal-maintained `channel_user_map` (platform identity NEVER auto-trusted), role-checks the mapped principal against the role store, then reuses the existing console verbs — so the digest binding holds TWICE: bridge-side against the rendered digest, server-side inside the approve verb (`digest_required`). Replay of a decided proposal is refused (404), never a second approval |
| POST | `/workflow/channel/user-map` | File a `channel/user_map` proposal (Herald): the ONLY way identity mappings enter the system. Payload: `{action: add\|remove, channel, tenant, platform_user_id (opaque id, never a display name), principal, roles[] (≤8, each must exist in the role store)}`. Probe-validated + audited at file time; the table's ONLY writer is the approval path. Write on global required |

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
| POST | `/ingest/proposal` · `/proposals/{id}/approve[?supersedes=N][&digest=...]` · `/reject` · `/proposals/{id}/edit` | Human-in-the-loop write-back (v1.14). Since v1.27.12 `approve` is bound to the bytes the reviewer saw: the `digest` (SHA-256 of the read-canonical review form, as served by `GET /proposals` — field `content_digest`) is **required** (`400 digest_required` when absent) and any drift → `409`. Since v1.28.74 accepts optional `origin_context: "channel"` — stamps the proposal's source `channel-capture` (the review-queue badge the operator approves against; the promoted row carries the origin). Caravel: kind `channel/template` is proposal-only — content is the JSON packet `{tenant, conversation_ref, template, body}`; approving CASes it approved and dispatches the governed send in ONE tx (window + consent + approved proposal all verified kernel-side; business-initiated cold contact additionally opens its `care/case`). Replay-safe: a decided id returns `{moved:false}`, never a second send. If the kernel's own gates refuse after the human decision, the refusal is audited and reported (`{moved:true, enqueued:false, reason}` or 409 with nothing written) |
| POST | `/ingest/proposal` (kind `registry_lifecycle`) | Proposal-only lifecycle intent. `content` is the exact serialized `{action,id,version,row_digest,row}`: `action` is `promote\|retire`, `id` and `version` identify the row, `row_digest` comes from the single-row detail response, and `row` is the exact current `RegistryRow`. Creation makes no status/knowledge change (no registry status transition and no knowledge/vector write); only the existing human approval gate disposes it. Non-empty `evaluation_refs` are refused. This is not a generic signature record and does not make `evaluated` reachable |
| GET | `/proposals?status=&domain=` · `/decayed` | Approval queue + decayed review. Each row is a `ProposalView` (`content` = read-canonical form, `content_digest` = SHA-256 the approve verb binds to, v1.27.12; v1.28.53 "Triage": rows carry their `domain` label + optional `title`, and `?domain=` scopes the queue — the read gate checks the REQUESTED domain, fail-closed 403 for a foreign one; approve/reject/edit re-check the ROW's domain before the CAS, so a foreign-domain proposal is never decided by a caller its domain never answered for) |
| POST | `/consolidate/propose` · `/apply` · `/undo` | Reviewable consolidation, supersession, undo |
| POST | `/suggest` · `/suggest/feedback` · GET `/suggest/metrics` | Opt-in anticipation + false-positive metric. Hits carry `untrusted: true` (v1.28.65, recall/search parity — suggested content is data, never instructions) |
| POST | `/verify` | Claim span verification |
| POST | `/classify` · `/decision/{id}/evaluate` | Deterministic categorization / decision rules |
| POST | `/procedure` · GET `/procedure/{id}/steps` | Ordered procedures (steps bind the `X-Brain-Domain` label + record gate) |

---

## Profiles, roles & connectors (policy)

| Method | Path | Purpose |
|---|---|---|
| GET | `/profiles` · GET/POST `/profiles/{name}` | Preset system (v1.21): fetch/upsert a typed knob bundle |
| GET/POST | `/domains/{name}/profile` | Per-domain profile binding: read the resolved bundle for a domain / bind one (the preset API + the domain binding) |
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
| POST | `/dsar` | Locate → export → purge → deletion certificate (supports `dry_run` footprint preview). Since v1.28.87 every content write is owner-stamped — the acting principal's `sub`, or the fixed `loopback` label for opaque-mode (no-principal) writes — so the locate covers operator-authored ingests; pre-.87 rows with a NULL owner stay stamp-blind by declaration (F7-02) |
| GET | `/dsar` | DSAR ledger (admin, newest-first, per-row deadline) |
| GET | `/tombstones?subject=&since=` | Deletion registry |
| GET | `/dsar/{id}/certificate` | Re-fetch certificate + live chain check |
| GET | `/audit` · `/audit/verify` | Append-only audit log + chain integrity (v1.27.31: verify covers every registered domain; rows carry their `domain` tag in multi-db mode) |
| GET | `/quarantine` | Injection review (GET = list). Decisions are POSTs: `/quarantine/{id}/release` · `/quarantine/{id}/delete` |
| GET | `/retention` · POST `/retention` · GET `/art30` · GET `/retention/report` | Per-kind retention policy + Art 30 record + per-domain×kind retention report |
| GET | `/legal/rules?since=` | The curated law-version diff over the legal-rules DB (read-only; Admin gate + DPO role). Reads `BRAIN_LEGAL_DB_PATH` READ-ONLY per request; unset/unreadable refuses NAMED (`legal_db_unconfigured` / `legal_db_unavailable`), an unknown `since` refuses `404 law_version_unknown`. The DB is populated by the DPO's quarterly import — no auto-pull, no enforcement on this surface |
| GET | `/snapshot/status` | Point-in-time snapshot state |

---

## Domains & routing

| Method | Path | Purpose |
|---|---|---|
| POST | `/domains` | Create a domain pool (200 = existed, 201 = created; body `{name}`) |
| GET | `/domains` | List known domains (single global pool when multi-db is off) |
| DELETE | `/domains/{name}?confirm=<name>` | Delete a domain + all its data (echo-confirm guard, `global` protected) |
| POST | `/domains/{name}/vacuum` | `VACUUM` one domain pool (returns `{name, vacuumed: true}`) |
| GET | `/domains/{name}/export` | Consistent SQLite snapshot download (`VACUUM INTO`, `attachment; filename="brain-<name>.db"`) — Read in multi-db; **Admin in shim mode** (the snapshot is the whole shared pool there) |
| POST | `/domains/{name}/import` | Restore a snapshot into a NEW domain (raw bytes body; 201 `{name, imported: true, bytes}`) |
| POST | `/domains/recompute` | One-shot centroid recompute sweep over every domain (`{recomputed: [[domain, n], …]}`) |
| POST | `/domains/move` | Move chunks to another domain |

---

## Knowledge parcels (v1.28.30)

Signed, human-gated site-to-site knowledge — deliberately slower than live
federation (a v3.x concern): every crossing of a site boundary is a signed,
reviewed act.

| Method | Path | Purpose |
|---|---|---|
| POST | `/parcels/export` | Build + sign a parcel of a domain's approved knowledge (Admin). Only promoted (non-quarantined) rows leave; residency stamps are copied read-only; signed with the UMP operator key; the export crossing is ledgered + audited in-tx. `400 parcel_too_large` over the 500-row cap; `409 operator_key_missing` without a key |
| POST | `/parcels/import` | Verify-then-import: signature checked BEFORE any write (`400 parcel_unsigned` / `parcel_tampered`). Since v1.28.67 "Pin" the publisher is NAMED, always: `expected_signer` is REQUIRED (missing → `400 signer_required`), and with a local operator key an `expected_signer` aliasing OUR did on a foreign-produced parcel refuses `409 signer_alias` — no one imports parcels "from us" that we did not produce. Rows land as PENDING proposals stamped with the TARGET domain — never direct knowledge writes — deduplicated by content hash against the domain's knowledge plus its own and global pendings; injection-screened rows refused and counted. Ledger + audit in-tx |
| GET | `/parcels` | The parcel ledger: direction (in/out), hash, signer did, row count, reviewer — bounded (`limit` ≤ 200) |

CLI: `brain parcel export --domain <d> [--since <ts>] --out <file>` ·
`brain parcel import --file <file> --domain <d> [--expected-signer <did>]` ·
`brain parcel ledger [--domain <d>]`.

Honest ceilings: pre-Triage proposals read `domain='global'` forever (no
heuristic re-attribution — provenance beats guessing); the by-id verbs still
gate at the queue's global posture, so a domain-scoped approver needs the
global grant plus the row-domain grant (the row re-auth can only deny, never
widen); approval promotion still stamps knowledge `global` (the proposal's
domain does not yet flow into the promoted chunk); parcels sign with the UMP
operator key, not minisign; no encryption-at-rest on the bundle yet; gold-set
packs do not ride the envelope.

---

## UMP (Universal Memory Protocol)

| Method | Path | Purpose |
|---|---|---|
| GET | `/ump/capabilities` | Protocol negotiation (conformance level, retrieval signals, `max_recall`, writable, audit) |
| POST | `/ump/remember` · `/ump/revise` · `/ump/forget` · `/ump/feedback` | Record / patch / soft-delete / outcome-feedback |
| POST | `/ump/recall` | Ranked recall with per-result signals |
| GET | `/ump/memory/{id}` | Read one record with on-read integrity re-verification |
| GET | `/ump/subscribe` | SSE broadcast of memory events |
| POST | `/ump/audit` · GET `/ump/audit/verify` | UMP-scoped audit row family + chain verification. Since v1.28.67 "Pin" the verify response carries the additive `integrity` census `{verified, signed, hash_only}` over the UMP record population under the current serve posture, plus `note: hash_only_records_present` when the operator key exists and hash-only records were seen (visibility, not gating) |

---

## Legal hold & breach (v1.22 / v1.25)

| Method | Path | Purpose |
|---|---|---|
| POST | `/legal-hold` · `/legal-hold/{id}/release` · GET `/legal-holds` | Per-domain legal holds; held ids are frozen (purge/DSAR defer) |
| POST | `/breach` · `/breach/{id}/event` · `/breach/{id}/close` | Breach-notification workflow (open / append event / close) |
| GET | `/breaches` · `/breaches/{id}` | Breach register + detail |
| GET | `/workflow/scoreboard` | Workflow outcome/efficiency scoreboard over recent runs (DPO/admin; rates in integer ten-thousandths, fail-closed audit linkage). Since v1.28.62 carries the ASI09 approval-fatigue telemetry: `review_independence_risk` (0\|1, the client detector's verdict server-side), `approval_uniformity_ratio` (integer ten-thousandths), `review_decisions_window` — parity-pinned to the console's rubber-stamp arithmetic |
| GET | `/workflow/reflection/corpus?since=&limit=&partition=all\|train\|holdout` | The de-identified disagreement-corpus export (DPO/admin dual gate; audited per call). Bounded page (1..=500), every row carries its frozen train/holdout partition (pure function of the run id over a pinned constant — stable across exports), identifiers render as content digests, excerpts pass the read-seam sanitizer with unconditional PII masking; the raw case input never exports |
| POST | `/workflow/calibration/sign` | Monthly human-signed workflow calibration gate (DPO/admin; one signature per calendar month, audited) |
| POST | `/accounts` | Create the account record — the deliberately-not-a-CRM record layer (accounts are workflow_runs rows of kind `account`). Body `{name, domain}`; the name is screened + bounded 1..=256 (control/invisible-refused), id/owner/status/clock are server-derived; record + audit land in ONE tx (Write on the domain + `workflow` role) |
| GET | `/accounts/{id}` | The account view: record + derived stage (latest `pipeline` row else `lead`) + the pipeline timeline. Absent and non-account ids answer the SAME probe-blind 404 (Write on the domain + `workflow` role) |
| POST | `/accounts/{id}/pipeline` | Advance the stage over the CLOSED ratified vocabulary (`lead → qualified → proposal → closed_won \| closed_lost`; self-transitions refuse). `decision_ref` REQUIRED — `400 decision_ref_required`/`decision_ref_invalid`; unknown stage → `pipeline_stage_unknown`; illegal edge → `illegal_stage_transition` naming source→target; archived refuses (`account_archived`). The appended row carries `{stage, decision_ref, prev_stage}` + audit in ONE tx. The classifier NEVER advances a stage |
| POST | `/accounts/{id}/requests/{run_id}/link` | Attach one request run to the account: an additive `account:link` row under the ACCOUNT's run id + audit, atomic; re-links append new audited rows (never mutated); archived refuses (Write on the account's domain + `workflow` role) |
| GET | `/accounts/{id}/requests?limit=` | The bounded per-account history (1..=500, default 100): link rows joined to their request runs' headlines + recorded decision rows — the pure decision join (Write on the domain + `workflow` role) |
| GET | `/accounts?limit=` | The bounded account listing (1..=500, default 100) — THE exfiltration surface: DPO/admin dual gate + an audited global row per call naming the principal, the filter, and the count |
| GET | `/workflow/kappa/queue?limit=` | The κ labeling bench's rater queue: the mined disagreement tuples assigned to the caller's slot (the slot derives from the authenticated principal, never the client; echoed in the response), both frozen partitions, bounded 1..=500 (default 100). Rows carry the machine's proposal (read-seam masked), the phase, the partition, and the rater's OWN latest label — never another rater's, never the governed truth (Write on global + `calibrate`) |
| POST | `/workflow/kappa/labels` | Capture one blind judgment: body `{digest, label, run_id}`, the label from the CLOSED ratified vocabulary (`agree \| disagree \| uncertain`), the slot from the principal. Exactly-once + append-only under the tuple's run id: a re-submitted latest judgment is the no-op receipt, a changed judgment appends a supersession row; ONE audit row per created label (ids + counts, never label text). Absent tuple and absent assignment answer the SAME probe-blind 404 (Write on global + `calibrate`) |
| GET | `/workflow/kappa/report?limit=` | The per-rater-pair κ report: one cell per (domain × frozen partition × slot pair), integer ten-thousandths, latest-wins; degenerate pairs name themselves (NO_KAPPA + the κ fn's own refusal). `meets_bar` (κ ≥ 0.70 = 7000 units) is REPORTED DATA — the κ value never auto-gates anything. THE exfiltration surface: DPO/admin dual gate + `calibrate` + an audited global row per call (1..=500, default 100) |
| GET | `/workflow/wizard/packs` | The ratified wizard pack catalog: the three operator-ratified packs as read-only, validated data — `{packs: [{id, question_count, pack}], count}`, every entry re-validated through the total pack validator at read time; the templates are compile-time-embedded from the committed corpus files (ONE source of truth), never runtime-fs. The SvelteTauri shell renderer branches CLIENT-SIDE on the packs' total next-maps; answers never ride this route (Read on global — any authenticated principal) |
| POST | `/workflow/decision-runs` | Execute one decision-pipeline run over the request's ask and persist its trace (digests and refs only — the raw `query` is hashed before anything durable). Body `{config, rules_config, run_id, mode: deterministic\|exploratory, request_id, question_id?, question_kind?, question_ids, query, proposal?}`; the rules table must digest to the config's bound model (`400 model_digest_mismatch` otherwise) and hostile configs refuse named (`400 config_invalid`). With `proposal: true` AND an escalated outcome, an escalation proposal queues in the SAME transaction carrying the run's provenance ref — the promotion gate reads the ref's mode: an EXPLORATORY run can propose, never promote (`exploratory_mode_not_promotable`); the human path for exploratory output is re-running deterministically. 201 `{trace_id, action, escalation?, output?, records, proposal_id?}` (Write on the run's domain + `workflow` role; absent/foreign run = probe-blind 404) |
| GET | `/workflow/decision-runs/{id}` | The stored trace document verbatim by ROW id — run identity, pipeline version, mode, config hash, model refs, input/context digests, per-stage records (digests, trust tiers, timing), the outcome; the raw query text is unrepresentable in it. Absent ids answer the probe-blind 404; the read is audited (Read on global + `workflow` role) |
| POST | `/workflow/decision-runs/{id}/replay-diff` | Re-execute a stored run under ITS OWN recorded conditions: the supplied config must canonical-hash to the trace's `config_hash` (`409 config_hash_mismatch` otherwise) and the rules table must digest to the bound model. Re-runs with LIVE retrieval (a changed corpus shows up as an honest mismatch — poison visibility) and reports `{trace_id, config_hash, config_hash_match, replay_input_digest, input_digest_match, stages: [{stage, match, stored_outputs_digest, replayed_outputs_digest}], all_match}` — DATA, never a status; timing is provenance and never compared; the replay persists NOTHING. POST (not GET) because the config + rules documents are large structured bodies and the body re-carries the run's input fields (the trace binds them only as digests) (Write on the run's domain + `workflow` role) |
| GET | `/workflow/decision-runs?limit=&run_id=` | The bounded decision-run listing (1..=50, default 20, newest-first, optional `run_id` filter): `{rows: [{id, run_id, mode, pipeline_version, config_hash, created_at, stage_count}], count}` — bounded columns ONLY, the trace documents never ride a listing. THE exfiltration surface: DPO/admin dual gate + an audited global row per call |
| GET | `/workflow/model-registry?limit=&status=&kind=` | The bounded model-registry listing (1..=50, default 20, newest-first; optional closed `status` and `kind` filters): `{rows, count}`. Artifact digests are represented only by `artifact_digest_present`; digest values ride the single-row read. Admin plus DPO role, audited global read (the registry's exfiltration surface) |
| GET | `/workflow/model-registry/{model_ref}` | One registered model identity by the whole-segment `id@version` citation. Read on global + audited; malformed refs are `400 model_ref_invalid`, and absent rows use the probe-blind 404. The response requires a server-computed lowercase 64-hex `row_digest` (SHA-256 over the canonical compact `RegistryRow` serialization); copy the exact `row` and `row_digest` into a `registry_lifecycle` proposal. The row carries identity, vocabulary, lifecycle, and digest references only — never weights or evaluation contents. Promotion and retirement have no direct route: they use the existing human proposal gate |
| POST | `/workflow/model-registry/register` | Register an operator-supplied model identity as `candidate`. The deterministic-rules arm requires the in-body rules document and the server derives/stores only its identity and canonical digest; learned/reranker arms declare identity and digest references. Admin on global + audited; learned registrations require `artifact_digest`; duplicate identities are a loud `409 model_already_registered` |
| POST | `/workflow/decision-evals` | Evaluate a bounded, digest-pinned, explicitly non-authoritative operator-declared judgment manifest against persisted decision traces. The body is `{idempotency_key, target, judgment_set}`; it carries closed labels, evidence IDs, and digests only—never raw query/evidence text. Missing/invalid source data is `400 judgment_set_unavailable`; learned targets require an artifact digest. The record and checked human/operator acceptance audit commit atomically; `acceptance_state=operator_accepted_non_authoritative` is not a detached signature. Admin on global + DPO; no registry status change or automatic promotion. |
| GET | `/workflow/decision-evals/{id}` | Read one digest-verified evaluation record by stable `eval_<32 hex>` id. Admin on global + DPO, audited when found, probe-blind 404 for absent records. The response contains bounded manifest metadata, aggregate leg statuses, and acceptance data only; no raw case content, weights, or secrets. |
| GET | `/workflow/decision-evals?limit=` | Bounded newest-first evaluation metadata listing (1..=50, default 20), Admin on global + DPO, audited per call. Full manifests and reports never ride the listing; missing legs remain explicit `unavailable` values rather than zeroes. |
| POST | `/workflow/delivery/runs` | Open a delivery run on the EXISTING run engine with `kind=delivery` — no second engine, no schema widening. The body is `{domain, goal, tier, policy_digest?, config_digest?, budgets?}`; `tier` is the closed set `observe\|propose\|bounded-auto\|delegated` and the trace mode is DERIVED from it, never taken from the client. Any `budgets` supplied are STORED as evidence and are **not enforced** — no route consults a ceiling. A delivery run carries no jurisdiction, so no law-version stamp is written. Write on the target domain + the `workflow` role. |
| POST | `/workflow/delivery/runs/{id}/advance` | Advance one phase. ONE transaction: the step row, the revision CAS, the trace row, and a fail-closed audit row commit together or not at all. Legal only for the five ADJACENT phases — a skip and a rewind are both `409`; a lost CAS is `409 delivery_gate_stale_revision` and the whole pass rolls back rather than overwriting the winner. The run closes `completed` only at the terminal phase, inside the engine's existing closed status set. An optional `artifact` (`{id, content, quality_gate?}`) rides the pass and is filed, **in the same transaction**, as a PENDING proposal with no disposition — the executor proposes, only the gate disposes. On a `build` pass the `quality_gate` is evaluated first and an artifact whose evidence is not a live surface is `409 delivery_quality_gate_refused` with nothing written; artifact content that the content screen rejects is `400 artifact_screened_reject` and a quarantine verdict is `409 artifact_screened_quarantine`. The artifact's SHA-256 is derived **server-side** (there is no digest field to supply) and the content is stored verbatim so the approval digest binds one shape. The response's `proposal_id` is that proposal, or `0` when the pass carried none. Write on the run's domain + the `workflow` role. |
| POST | `/workflow/delivery/runs/{id}/answer` | Clear the run's `pending_question` through the same revision CAS, recording that an answer happened. The answer is operator-authored prose on a run the operator owns: stored in the run's own state, bounded to 2000 chars, and never copied into a trace row. A run with no pending question is `409`. Write on the run's domain + the `workflow` role. |
| POST | `/workflow/delivery/runs/{id}/gates` | Evaluate the phase gate. A **DISPOSITION, never a mutation**: the run's phase, status, and revision are untouched and the only writes are the trace row and its audit. Pure and offline; deny wins. A terminal phase and an illegal move are `denied`; a value outside the closed phase vocabulary is `denied`/`closed-vocabulary` rather than a nearest-match guess; a tier that may not promote is told `prompt`, and the human's advance route is the disposal. `200` whatever the verdict — a deny is a recorded outcome, not a transport error. No budget ceiling is consulted. Write on the run's domain + the `workflow` role. |
| GET | `/workflow/delivery/bindings` | The standing authorities this machine holds to read external systems on behalf of ONE domain: `{bindings, intents_pending, observed_pending, untrusted_pending}`, each binding carrying `{id, domain, target_kind, target_ref, endpoint, authority_digest, capabilities, active, updated_at}`. `domain` is a **required query parameter** and the surface is domain-scoped, because a binding resolves to one tenant's authority and an unscoped resolve would be a cross-tenant leak. The `authority_digest` covers the endpoint, the stable external ref, and the secret's FILE NAME — **never the secret and never its path**; a digest computed over secret material is a credential at rest in a hash column. `capabilities` is the operator's declared surface parsed with `deny_unknown_fields`: an unknown field or capability is a REFUSED binding, and a block this server cannot parse renders as the literal `"unparseable"` rather than as a default that would read as unconstrained. `registry`/`deploy`/`pm`/`incident` are declared and **consumer-less** — no adapter reads them. The pending counters are the ops signal that an intent which is merely not-yet-promoted is distinguishable from one that was lost, and from a FORGED row whose key is not a kernel mint (`untrusted_pending` should be zero). **There is no write route**: consent is given by configuring a binding at boot and withdrawn with `active = 0`, never by a request, because a request must never be able to create or widen an authority. Serving this list grants no authority, approves nothing, and makes **no compliance finding** — authorship is not authority. Read on the queried domain + the `workflow` role. |
| POST | `/workflow/delivery/releases` | File a governed release: the machine's proposal to move ONE artifact toward ONE external authority. The kernel names everything that binds — the artifact digest is derived from the run's own typed-artifact bytes (never a request field) and the authority binding is resolved from the run's own domain and the named target kind — while the request names only `{run_id, target_kind, ref, environment, commit_sha?}`. Lands `proposed`. The agent preset is refused **before any work** (agents hold `write:*`; this is the write family whose consequences reach another system). Write on the run's domain + the `workflow` role. |
| POST | `/workflow/delivery/releases/{id}/approve` | Record the approval as COLUMNS on the release row — no sixth table. The binding is **three-way**: the content digest (kernel-written from the release row), the authority digest recomputed from the binding row as it is now, and the run's state revision as it is now. An approval that binds content but not the target is replayable against a different external system; one that binds both but not the revision is replayable across a later phase pass. The expiry is measured from `approved_at` and is evaluated inside the promote transaction, fail-closed at the boundary. The approving principal is recorded from the authenticated caller, never asserted from the body. Approve and promote are separate requests **by design**. Write on the release's domain + the `workflow` role; the agent preset is refused. |
| POST | `/workflow/delivery/releases/{id}/promote` | The promotion gate. One transaction re-verifies everything before the pure crate gate reads anything: the signature chain (offline verifier — a broken chain is a typed refusal before the gate), the live digest re-derived from the artifact bytes as they exist now, the authority (recomputed; drift is `409`), the run's revision (unchanged since the approval), the approver's principal (the kill-switch), and the tier (the run's state and the chain's signed predicate must agree). Then the crate's total gate decides, deny-wins, first reason reported in push order. The trace mode is carried and **deliberately unread** — authority comes from the tier, never from how a trace was produced. A permitted promotion walks the crate's one-step-at-a-time transition law in the same transaction, lands `promoted`, records the post-hoc budget draw (elapsed minutes and the one artifact moved; `spent` moves only when a producer exists), and mints the dispatch intents — **promotion IS the outbox write**, so nothing here touches the network. The ledger's belief moves only when the inbound authority observation reconciles; the crank never writes `verified_at`. **Budgets are enforced at PROMOTION TIME, inside the promote transaction** — not at a hostcall seam, which the delivery loop never touches (the hostcall `Budget` is a 30 s wall clock with no run/kind/spend; the DO's clause was stale on four measured grounds and the re-scope is recorded). Every enforced budget kind needs explicit, unexhausted headroom; `blast_radius` is never enforced (crate law). `confirm` is the human disposition act on a `prompt` verdict. Write on the release's domain + the `workflow` role; the agent preset is refused. |
| POST | `/webhooks/delivery/{kind}` | An inbound **authority observation**, on the delivery sub-family of the EXISTING public `/webhooks/` family. It adds **no new public path**: it authenticates with the **shipped** GitHub HMAC verifier over the raw body and lands in the same bounded queue as every other verified webhook, so the replay window, the delivery-id idempotency, and the flood cap are the consent boundary it actually passes through rather than properties it re-implements. The observation is **never trusted ahead of reconciliation** — a verified body says only that these bytes came from the configured sender, and what the ledger believes comes from the authority itself, read through the shared egress family; the `200` reports the **reconciled verdict**, not the claim. The run, the domain, and the secret root are resolved **server-side** from the configured binding, so a body claiming a different tenant is ignored; an observation with no open run in that domain is **refused** rather than attached to an arbitrary one. `kind` is `github` (a `vcs` binding) or `actions` (a `ci` binding), and anything else is refused by name. A mismatch is recorded as typed evidence for a human to decide: whether an external system's data may be read, retained, or re-published is **a question for a human with the contract in hand**, and this surface decides none of it. The signature shows the holder of the configured secret sent these bytes; it says nothing about whether their contents are true. |
| GET | `/workflow/delivery/runs/{id}/attestations` | The run's signed attestation chain and its **UNCONDITIONAL** verification verdict: `{run_id, chain, verdict}`, each link carrying `verified` and, when false, a named `refusal` from a closed vocabulary. `?verify=1` is accepted and is an explicit request for the IDENTICAL payload — no parameter can switch verification off, and a non-verifying chain is reported per link rather than hidden or degraded into a mark that reads as verified. The single `409` is a chain that could not be READ. The raw signed envelope is not returned: it is canonical bytes carrying a base64 signature. Read on the run's domain + the `workflow` role, probe-blind. **Not DSSE** — the project envelope convention, which verifies against no DSSE verifier; the `subject_digest`/`predicate_type`/`predicate` names mirror the in-toto Statement v1 model as **adjacency only** (not an in-toto Statement, no `_type`); **no SLSA provenance and no SLSA build level**; the IETF WIMSE agent-audit drafts are contemporaneous prior art, not a standard. **Authorship is not authority** — `signer_did` proves who signed, with no PKI, no revocation oracle, and no key epoch, so a rotated key leaves history verifiable. A valid signature says nothing about whether the act was permitted. |
| GET | `/workflow/delivery/runs/{id}/replay-verify` | Re-derives the run's stored trace and reports whether it is **internally consistent**: `{run_id, window, order_ok, compared, matched, mismatched, diffs, event_log, generated_at}`. For each trace row, in ORDINAL order, the row's content address is recomputed from its own stored columns and compared with the address stored beside it; the ordinal series is separately checked for contiguity, and a gap or descent is reported as an `order` diff. A mismatch is **DATA, never a status** — the request is `200` and the reader is handed what was stored, what the columns imply, and which comparison failed. Models are **never re-run**: the comparator lives in a crate whose entire dependency set is `serde`/`serde_json`/`sha2`, so the zero-model property is structural, and the verdict says nothing about whether an outcome was *correct*. **ADJACENCY:** `POST /workflow/decision-runs/{id}/replay-diff` publishes a similar concept under similar wire keys; the two are **not unified** and share no code — that route RE-EXECUTES the pipeline and loads a bound model, where this one does not re-execute anything. **THE CEILING:** this is tamper **EVIDENCE** over stored bytes, not tamper-proofing — an attacker who edits a column AND recomputes the address leaves nothing to detect here; it does **not** bind a row to the signed attestation chain (the chain is what binds; this checks); and it is **not a compliance finding** — a verified replay authorises nothing, because authorship is not authority. Classification, retention, and any legal sufficiency of this output are operator-and-counsel determinations. The window is bounded at 500 rows and the bound is **disclosed** in every response. Read on the run's domain + the `workflow` role, probe-blind. |
| GET | `/workflow/delivery/runs/{id}/trace` | The run's stored trace rows in **ordinal** order, plus the attestation chain head READ from storage (`null` before the first link, never a fabricated address) and the same bounded, self-disclosing `ddl_*` narrative appendix the replay verdict carries: `{run_id, window, rows, attestation_root, event_log, generated_at}`. It rides the **same read function and the same window function** as the verdict, so the two apply **identical logic** to storage: any difference you observe between them is a change in storage, not a difference of method. They are two separate requests with no shared snapshot, so this is **not** a consistency guarantee across a moving run — this one answers "what is actually there", which is the question a reader has when the verdict reports that something did not line up. Serving these bytes is not an endorsement of them — the rows are operator-authored text and digests, returned as stored. The window is bounded at 500 rows and the bound is **disclosed** in every response. Read on the run's domain + the `workflow` role, probe-blind. |
| POST | `/workflow/delivery/runs/{id}/advance` (R40 additions) | The pass now also signs an **attestation link** and appends it to the run's chain, in the SAME transaction (step row → CAS → trace row → link → proposal seam → session log → audit last), and the trace row's `attestation_root` names the chain head. An optional `model` (`{key, config_digest}`) names the registry row the pass executed under: the server resolves it, and the signed predicate carries the row's **artifact digest**, so a model name with no bytes behind it is `409 delivery_model_digest_missing`; the registry refusals are four distinct codes (`delivery_model_not_registered` / `delivery_model_not_promoted` / `delivery_model_retired` / `delivery_model_digest_missing`). **Key posture, fail-closed:** a pass REFUSES with `409 delivery_attestation_refused` when the host has no usable operator key — an absent key and a refused one are different causes of the same code, and neither ever degrades into an unsigned link. A run on a keyless host therefore never advances past its admission. `delivery_traces` also gained a stored `seq` ordinal, so every `trc_` id is re-addressed once (consumer-affecting). |
| GET | `/workflow/runs/{id}` · `/workflow/runs/{id}/steps` · `/workflow/runs/{id}/suggestions` | Run row (state sanitized at the read seam), steps, retrieval-backed suggestions (Read on the run's domain). Since v1.28.72 the **suggestions** response carries `evidence_recorded: true\|false` — the KCS evidence side-effect fires only for callers holding Write on the domain AND the `workflow` role (Read-only callers get the body unchanged, nothing recorded) |
| GET | `/workflow/runs/{id}/report` | The run's recorded-rows report at a pinned law version — a pure rendering of its gate records (`workflow_steps`) and workflow audit rows, labeled with the pinned `law_version` (absent pin = the run's own intake stamp); `law_version_mismatch` is advisory ONLY; reads are not audited so the report stays byte-reproducible (Read on the run's domain) |
| POST | `/workflow/cases/{id}/gdl` | The operator case-launch boundary: launch one GDL case episode on a FRESH run (kind `troubleshoot`, status `active`, revision 0, empty state) through the real server-configured provider. The accepted body is `{ticket}` only. Provider destination/model/secret are server-owned via `BRAIN_GDL_PROVIDER_BASE_URL`, `BRAIN_GDL_PROVIDER_MODEL`, `BRAIN_GDL_PROVIDER_SECRET_FILE`, and `BRAIN_GDL_PROVIDER_SECRET_ROOT`; legacy caller fields return `400 gdl_request_migrated` and are never used. JWT callers need domain Write plus the `workflow` role; role-less JWTs, unknown roles, and `agent@loopback` bearers are refused before secret/DNS/provider work. Production endpoints require HTTPS, reject userinfo/fragments/queries/unsafe shapes, pass the existing address screen with DNS pinning, and never follow redirects. The request has a 25-second total body deadline; receiver cancellation drops the in-flight HTTP future. A provider failure after admission is durably terminal and non-retryable: the first launch returns `503 gdl_provider_failed`, and a later launch against that run returns `409 gdl_provider_failed` without replaying provider work. Outcomes otherwise use the existing GDL vocabulary — a pending capture PROPOSAL (human-approved later) or a Handoff/route/escalation; nothing publishes automatically. Provider errors expose stable codes only; raw bodies, credentials, secret paths, and secret-bearing URLs are not reflected. |
| POST | `/workflow/runs/{id}/steering` | Queue a steering message: blocklist-screened, Write + approve-class role gate, bounded inbox drop-oldest at 100 |
| POST | `/workflow/runs` | Open a governed run (`{domain, kind, state_json}` → `{run_id, revision}`); Write + `workflow` role gate; open + audit row commit atomically. `valet/%` kinds vet the envelope at the fence: the `what` label must pass the injection screen (`400 screen_rejected`) and the state must be a readable valet envelope (`400 valet_state_invalid`) — v1.28.63 |
| GET | `/workflow/runs/{id}/state` | Engine-exact `{state_json, revision}` (machine CAS round-trip; NOT read-seam sanitized — the human view is `GET /workflow/runs/{id}`); Read + `workflow` role gate; audited read |
| PUT | `/workflow/runs/{id}/state` | CAS advance (`200 {revision}` / `409 {actual_revision}`); Write + `workflow` role gate. `status` is a CLOSED vocabulary — `active \| cancelled \| closed \| completed \| fired \| resolved` (v1.28.63); unknown values refuse `400 unknown_status` with an audit row |
| POST | `/workflow/runs/{id}/events` | Outbox enqueue, exactly-once by idempotency key (`{first, event_id}`; optional `parent_event_id` links ancestry); Write + `workflow` role gate. RESERVED topics (v1.28.63): `channel/*`, `steering`, `workflow/valet*` are kernel-only — the route refuses them `400 topic_reserved` (audited `outbox_reserved_refused` on the workflow chain) |
| GET | `/workflow/runs/{id}/events?branch=` | The lineage read: ordered events with `parent_id` links (Read on the run's domain); `branch=<event_id>` narrows to that event's ancestor chain, root-first; `since=<event_id>` backfills a reconnect gap |
| GET | `/workflow/runs/{id}/context?at_event=&budget=` | The derived context window (Fathom): latest checkpoint at-or-before the anchor + delta + finding digests + open question; field-budgeted, delta drops oldest-first (`truncated` flag) — the consumer contract for unbounded sessions (Read on the run's domain) |
| POST | `/workflow/runs/{id}/rewind` | Rewind = branch, never delete: verify the target is a `workflow/checkpoint` event (or the run root), CAS-restore its state snapshot appending a `branches[]` marker, audit — one tx (`{ok, revision, branched_from}`); Write + approve role gate |
| GET | `/workflow/runs/{id}/handoff` | The I-PASS handoff packet assembled from the run's records (illness/patient/action/situation/safety + `handoff_complete = status=="completed"`); Read on the run's domain |
| POST | `/workflow/runs/{id}/handover/offer` | Relay: offer a one-click handover `{to_principal, overlap_minutes?}` — gated by the packet-completeness check (open question, un-breached SLA, current step, linked evidence/checkpoint, resolved escalation); an incomplete packet refuses `400 packet_incomplete` with `details.missing` and writes nothing. Offer + lineage event (`workflow/handover`) + audit land in one tx; retried POSTs are idempotent (Write on the run's domain + `workflow` role gate) |
| POST | `/workflow/runs/{id}/handover/{offer_id}/accept` | Accept an offer: in ONE WorkflowTx the offer state moves and the run `owner` CAS-transfers to the acceptor; the SLA clock is untouched and the reply points at the resume-at checkpoint. Deciding a decided offer replays `{moved:false}` (Write on the run's domain) |
| POST | `/workflow/runs/{id}/handover/{offer_id}/decline` | Decline an offer with a REQUIRED reason `{reason}` — screened, ≤ 4000 chars, stored + audited (an audited refusal beats a silent bounce). `400 reason_required` / `reason_too_long` (Write on the run's domain) |
| POST | `/workflow/runs/{id}/handoff/decision` | The operator's handoff decision `{transition: delivered\|cancelled, decision_ref}` — the machine-generated handoff moves ONLY on an operator decision carrying a decision reference (screened, ≤ 256 chars, the audit-recovery handle); the lifecycle row + audit land in ONE tx. `400 decision_ref_required` (the machine never closes a handoff on its own authority) / `decision_ref_invalid` / `unknown_transition` (Write on the run's domain + `workflow` role gate) |
| POST | `/workflow/runs/{id}/back-referral/return` | The receiver's release: `{contract_key, report, decision_ref}` flips the return contract to `returned`. A report missing a required field refuses `400 report_incomplete` with `details.missing` (the B3 law at the surface); `late` is computed at the server clock; release row + audit in ONE tx. `400 decision_ref_required` / `decision_ref_invalid` / `contract_key_required` / `report_invalid`; contract-absent answers 404 probe-blind (Write on the run's domain + `workflow` role gate) |
| POST | `/workflow/runs/{id}/complaint/lifecycle` | Goodwill: advance the ISO 10002 lifecycle one legal step `{to}` over the CLOSED table (`received → acknowledged → investigated → remedy_proposed → remedy_approved → closed → adr_referred`); anything else refuses `400 complaint_invalid`. Lineage event (`workflow/complaint`) + audit land in ONE tx (Write on the run's domain + `workflow` role gate) |
| POST | `/workflow/runs/{id}/complaint/remedy` | Goodwill: propose a remedy from the matrix `{kind: repair\|replace\|refund\|goodwill_payment\|explanation_only, amount_cents, code_clause_id, tier}` — always a PENDING HITL proposal citing its legal basis and its published code-of-conduct clause; a contradiction with the published promise is flagged on the packet, never silently blocked; nothing financial executes here. Approval rides the standard gate with deterministic role-tier caps; over cap it escalates exactly one level with the packet attached. Response carries the Attestation `provenance` mark (Art 50(2) AIGEN, ed25519-signed; `provenance::verify` refuses tampering) (Write on the run's domain + `workflow` role gate) |
| GET | `/workflow/runs/{id}/complaint/adr-packet?member_state=` | Goodwill: the ISO 10003 external-dispute packet — run identity, audited remedy history, and the competent NATIONAL ADR body from the DPO-maintained registry (`knowledge.source='adr_body'`). The EU ODR platform is discontinued (Reg. 2024/3228); every packet states that basis. Humans file. Unregistered member state denies loudly. Carries the Attestation `provenance` mark over the post-read-seam boundary bytes (Read on the run's domain) |
| POST | `/workflow/runs/{id}/complaint/ack` | Advocate: acknowledge the complaint — the legal `received → acknowledged` step with its dedicated audit marker so the monthly register measures ack-SLA attainment (ISO 10002: within the hour). Lineage event + audit in ONE tx (Write on the run's domain + `workflow` role gate) |
| POST | `/workflow/complaints/ack-sweep` | Advocate: one overdue-acknowledgment sweep over every active complaint past its ack deadline — exactly one `workflow/complaint/ack_overdue` alert per run on the alert bus, audited, idempotent per run, bounded. Global scope (Write + `workflow` role gate) |
| POST | `/workflow/outreach/campaign` | Outreach: propose a campaign `{domain, channel: email\|sms\|call, purpose: care_followup\|retention\|recall_notice, template_id, audience[]≤1000}` as a pending HITL proposal — raw audience identifiers are hashed at the door, and the deterministic consent gate excludes every recipient without an in-force grant BEFORE filing (each included recipient carries its consent proof; zero eligible recipients refuses `400 outreach_invalid`). NOTHING sends here — approved campaigns export for CRM-side execution (Write + `workflow` role gate) |
| GET | `/workflow/outreach/campaign/{id}` | Outreach: the export packet for an APPROVED campaign only — recipients with their consent proof plus the template reference, for the CRM connector feed or operator export; pending/rejected campaigns export nothing (`404`). Emitted text passes the read seam, then the Attestation `provenance` mark signs the boundary bytes (Read + `workflow` role gate) |
| GET | `/workflow/outreach/consent?subject=&channel=&purpose=&domain=` | Outreach: the deterministic verdict for one (hashed subject, channel, purpose) triple — absent/revoked/expired all DENY, only an in-force grant reads `granted`; the proof row (granted_at/expires_at/provenance) rides every verdict. The raw subject never leaves the handler (Read + `workflow` role gate) |
| POST | `/workflow/runs/{id}/outreach/followup` | Outreach / Order-of-Care: schedule the post-close proactive check for a CLOSED complaint run whose state carries `subject` — one pending HITL proposal due at the policy interval (default 7 days after close), gated on an in-force `care_followup` consent. No consent → loud `400 outreach_invalid` and nothing filed (a gate, not a warning); lineage event + audit land in ONE tx (Write on the run's domain + `workflow` role gate) |
| GET | `/ops/handovers?domain=&now=` | The follow-the-sun board: active runs ranked by SLA remaining (recorded deadline wins, else P3-from-created), flagged while `now` sits inside the ring boundary's derived overlap window (Read on the domain) |
| POST | `/workflow/runs/{id}/notes` | Channel: post a case note `{content}` — screened at write (empty/≤4000/prompt-injection blocklist) and stored through the invisible-strip + markdown-ref seam; `@skill:<tag>` / `@principal` mentions resolve into swarm invites (invite row + `case/note` lineage event that drains to `/events` as the Crew ping — visible to `?kinds=workflow` subscribers holding Read on the domain). Dead mentions refuse `400 mentions_unresolved` with the list (over-vocabulary tokens included); >16 resolved invitees refuse `400 invite_limit`; a run at its channel ceiling refuses `409 channel_full` — evidence is never drop-oldest-deleted. `{content, kind:"reask"}` additionally marks the operator re-ask: the note rides as usual PLUS one `case/reask` lineage event (the effort proxy's marked source; the CLI twin is `brain workflow note <run> <text> --reask`). Note + invites + events + audit land in ONE tx (Write on the run's domain) |
| GET | `/workflow/runs/{id}/notes?limit=&offset=` | The channel view: chronological notes + invites for one run, policy-expired rows hidden before the page split (`case-note` retention kind), every string on the read seam, bounded page 1..=500 (Read on the domain) |
| POST | `/workflow/runs/{id}/notes/{invite_id}/accept` | Accept an invite into the channel: CAS `pending → accepted` on the invite row in one tx with its lineage event + audit; replaying a decided invite returns `{moved:false}`. Ownership never moves (Write on the run's domain) |
| POST | `/ops/agents/cards` | Mesh: provision (or re-sign) an agent's A2A-shaped card `{domain, principal, name, description?, capabilities?}` — Ed25519-signed with the UMP operator key at provisioning; no key refuses `409 operator_key_missing` (Admin on the domain) |
| GET | `/ops/agents/cards?domain=` | The domain's verified agent cards — each re-verified against the current operator key before it leaves the server; a tampered card fails the whole list closed (`400 card_tampered`) (Read on the domain) |
| POST | `/ops/agents/revoke` | The ASI03/07 kill-switch: revoke a principal `{principal, reason?}` — every card use, delegation dispatch, and result submission re-checks revocation and refuses closed (`403 principal_revoked`); every ACTIVE run where the principal owns in-flight delegation work drains through the existing run-cancel path; revocation + hash-chained audit + drain in ONE tx; identity-wide (Admin on `global`) |
| GET | `/ops/agents/revocations` | The kill-switch register: newest-first `{principal, revoked_at, reason, revoked_by}` rows; the hash-chained audit chain carries the full story (Read on `global`) |
| GET | `/ops/agents/bom` | Live agent bill of materials (AgBOM, CycloneDX 1.6 shape): models, knowledge stores, enforcement posture — regenerated per request, never a build snapshot (Read on `global`) |
| POST | `/workflow/runs/{id}/delegations` | Mesh delegation `{to_principal, task}`: the target's card is verified FIRST (unknown/tampered refuses `400 agent_unknown` / `card_tampered`, nothing written); then row + `delegation/request` lineage event (ids+actors only, never task content) + audit in ONE tx. Task screened like notes; per-run ceiling refuses `409 delegations_full` (Write on the run's domain) |
| GET | `/workflow/runs/{id}/delegations?limit=&offset=` | The run's delegation view: chronological work orders with state (`requested`/`completed`) and results, every string on the read seam, bounded page (Read on the domain) |
| POST | `/workflow/runs/{id}/delegations/{delegation_id}/result` | The delegatee's exactly-once result `{result}` — screened, CAS `requested → completed` in one tx with the `delegation/result` child lineage event + audit; non-delegatees refuse `400 not_delegatee`, replays refuse `409 result_already_submitted` (Write on the run's domain) |
| POST | `/workflow/runs/{id}/answer` | The AskHuman closer: digest-bound to the live `pending_question`, appends `answers[]`, clears the question, CAS — one tx; Write + approve role gate |
| GET | `/workflow/runs/{id}/steering?since=` | Drain the advisory steering outbox (Read on the run's domain) |
| POST | `/workflow/plugins/mount` | UI-plugin mount/unmount evidence (Art 12 record-keeping): server verifies the claimed bundle SHA-256 against the boot manifest before writing the audited row (`409` on uncertified bytes) |

### Frontdesk worktype intake (Frontdesk)

Post-sale intake is typed: 13 intent classes route every case to a worktype,
with policy rows (SLA envelope from the SDK `stamp_envelope` vocabulary) and an
entitlement vocabulary (coverage windows, withdrawal rights, region checks)
parsed from the run's state. Honest ceiling: the close-decision arbiter
(`evaluate_close` / effort proxy) ships as pure SDK logic but is **not yet
wired into the run-close flow** — closing remains operator-driven.

### KCS article lifecycle (Evolve)

Every solved case can become knowledge; the capture generator emits HITL
proposals (`kcs_new_article` / `kcs_update_article` / `kcs_link_only`) that a
human approves through `/proposals/{id}/approve`. Approved articles are born
`kcs_state='draft'` — nothing auto-publishes.

| Method | Path | Description |
|---|---|---|
| GET | `/kcs/articles?state=&stale=1` | The content-health worklist: KCS-carrying articles, filterable by lifecycle state; `stale=1` keeps articles past their freshness-review deadline or carrying open improve flags (Read, per-domain visibility) |
| POST | `/kcs/articles/{id}/approve` | Move a draft article to `approved`, stamping the 90-day freshness-review deadline (Write on the domain + `approve` role; `409` when not draft; audited in-tx) |
| POST | `/workflow/runs/{id}/status-ref` | Keystone: mint\|rotate\|revoke the run's public case-status ref — an unguessable HMAC token naming the static `status/<ref>.json` page that `brain kb build --with-case-status` emits. Mint is idempotent-per-run; rotation kills the old token; revocation removes the page from the next build and refuses fresh mints (a revoked page does not resurrect). brain NEVER sends the ref anywhere — it ships by human/CRM channel. Audited in ONE tx (Write on the run's domain + `approve` role gate) |
| POST | `/kcs/translate` | Keystone: file a pending `kcs_translate` HITL proposal for a human per-locale translation of a published article `{knowledge_id, locale, title, body_md}`. The tool files and governs, it never machine-translates; approval is the ONLY writer of an approved `kcs_translations` row, pinned to `based_revision` so a source advance lands the translation on the stale worklist (Write + `workflow` role; audited in-tx) |
| POST | `/kcs/articles/{id}/publish` | Propose publishing to the public KB (`kcs_publish` proposal; approval needs `approve` + the distinct `publish` capability). `action=retract` returns a published article to `approved` — the next build drops its page (Write to propose) |
| GET | `/kcs/articles/{id}/preview` | The exact sanitized public page for an approved/published article — same render path as `brain kb build`, unconditional PII redaction, no operator bypass (Read) |
| GET | `/ops/shifts?domain=&now=` | The shift-ring view: which site owns the queue at `now` (`queue_scope_site` re-scopes to the incoming site at the start of the derived overlap window — the queue follows the sun, cases don't), overlap state, next boundary, and the newest 500 shifts for the domain. Deterministic read-time arithmetic; no scheduler daemon (Read on the domain) |
| POST | `/ops/shifts` | Declare a site's on-call window `(site, tz, start/end epoch, overlap_minutes ≤ 120, roster)`. `400` on bad window/overlap/tz/roster bounds (tz ≤ 64 chars, roster ≤ 64 ids × ≤ 256 chars), `409 shift_double_booked` when the window starts before the earlier shift's final overlap period; validation + insert + audit ride one tx (Admin — pure operator configuration). Read capped at the newest 500 shifts |
| GET | `/ops/crew?domain=&now=` | The crew roster: TTL-decayed presence (active < 5 min, away < 30 min, offline beyond — computed at read; no background worker), Watchbill site badges from the shift ring, role + skills tags. Presence shows the KIND of act only (closed vocabulary: cranking/reviewing/idle) plus an opaque `current_case_ref` — never case content. Hidden entirely when the DPO switch is off or unreadable (Read on the domain) |
| GET | `/ops/skills?domain=` | The WFM skills feed: the domain's HITL-maintained skill registry, grouped by principal — the documented interop boundary for workforce-management tools (no forecasting engine is built; centers keep their WFM tool). Bounded at the newest 1000 rows (Read on the domain) |
| POST | `/ops/skills` | Propose a skills change `{principal, add[], remove[]}` → one pending `crew_skills_update` proposal. Tags are lowercase alnum+hyphen, ≤ 32 chars, ≤ 32 per principal; approval (HITL `approve`) is the ONLY write path to `principal_skills`, applying the change in the approval transaction (Write on the domain) |
| POST | `/ops/crew/config` | The DPO presence switch `{domain?, presence_enabled}` — off (or unreadable) means every roster reads empty. Flip + audit ride one tx (Admin on the domain) |
| GET | `/ops/workload?domain=&now=` | Per-principal workload from lineage only: concurrent open envelopes, pending outbound handover burden, accepted transfers-in on open runs, re-ask load, confirm-gate backlog — plus fatigue signals (consecutive-shift + open-load patterns) that alert the scheduling human and NEVER reassign work. Read-only by construction; no case content (Read on the domain). Stamped `schema_version: wfm/1` (docs/wfm-seam.md) |
| GET | `/ops/coverage?domain=` | Competence coverage: one row per demanded worktype — required routing tags, principals whose HITL-maintained skills cover every tag, open demand depth, covered flag. Same routing data as the colleague board; deterministic read (Read on the domain). Stamped `schema_version: wfm/1` |

The WFM interop boundary (`/ops/shifts`, `/ops/skills`, the two views above,
and the generic `brain wfm-import <file.csv|file.json>` adapter) is
versioned and additive-only — the contract lives in `docs/wfm-seam.md`.

### Public knowledge base (Beacon)

The public KB is a **generated static artifact**, never a live data path:
`brain kb build --domain <d> --out <dir>` emits a deterministic static site
(article pages under strict sanitization, index, client-side-only search
index, sitemap, robots, 404, redirect pages for superseded slugs, and a
SHA-256 `kb_manifest.json`). The operator hosts it and verifies the hosted
bytes against the manifest. Since v1.28.62 the manifest also carries the
Art 50(2) `provenance` seal — mark/generator/generated_at plus an ed25519
signature over the canonical digests body (`provenance::verify` refuses any
tampering); the per-file digests the operator checks are byte-unchanged, and
without an operator key the mark is present but visibly unsigned. On-page
"Did this solve it?" votes return through
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
| GET/POST | `/clients/{name}/dpa` | Per-client DPA record: fetch / file the data-processing-agreement register row |
| POST | `/clients/{name}/dsar` | Per-client jurisdiction-aware DSAR + certificate |
| POST | `/clients/{name}/hold` | Per-client legal hold (resolves the client's domain) |
| POST | `/clients/{name}/end` | Termination: purge-or-return + archive + certificate |
| GET | `/clients/{name}/proposals` · POST `/clients/{name}/proposals/{id}/coach` | Supervisor QA queue (same `ProposalView` shape as `/proposals`) + coaching note (v1.27.8, Admin) |

---

## Auth & discovery (JWT mode)

| Method | Path | Purpose |
|---|---|---|
| POST | `/auth/refresh` · `/logout` · `/revoke` | Token lifecycle (`/revoke` is Admin-gated; refresh/logout need only a valid bearer). Logout/revoke denylist rows live exactly as long as the token's real `exp` (clamped 24h) — the row dies when the token dies. Access tokens verify `iat` (future-issued refused) and a 24h maximum lifetime (`401 lifetime_exceeded` past it, so revocation always covers the full life); refresh families are per-login sessions with reuse detection burning the family |
| GET | `/.well-known/openid-configuration` · `/.well-known/jwks.json` | OIDC + JWKS |
| GET | `/.well-known/security.txt` | RFC 9116 security disclosure (public) |

**Identity revocation (the kill-switch at authN).** A JWT or capability
bearer whose identity sits in `revoked_principals` is refused `401
identity_revoked` on EVERY route, before authorization — the identity is
dead, not unauthorized. Denials are byte-identical for every revoked
principal (probe-blind) and audited path-only (never the token). Capability
tokens deny on their issuer principal. Opaque-loopback bearers have no
principal id to revoke (the opaque operator/agent split is a later line);
JWT key records pin their algorithm per `kid` (`401 alg_mismatch_for_kid`
on a header/record mismatch).

---

## Valet — the personal assistant (v1.28.42)

Cron-cranked (never a daemon), consent-gated, digest-bound across the Signal
bridge:

| Method | Path | Notes |
|---|---|---|
| POST | `/workflow/valet/due` | The crank: fires due `valet/*` envelopes (idempotent per `valet-{run}-{due_at}`), re-arms repeats via CAS, enqueues metadata-only alert envelopes. Write + `workflow` role. |
| GET | `/workflow/valet/brief` | Today's derived context: due/overdue, pending drafts with ADVISORY lint scores, evening notes. Read + `workflow` role. |
| PUT | `/workflow/valet/consent` | The one-subject Outreach-lite registry (subject `owner`, channel `signal` only). Write + `workflow` role. |
| POST | `/webhooks/{kind}` (kind `signal`) | Inbound Signal commands from the relay: `[case N] text` → screened steering; `[draft N] approve <digest>` → digest-bound approval (Gateweld crosses into Signal). HMAC + replay-capped. |

CLI: `brain valet add|due|brief|consent`. Relay: `tools/valet-relay/` (holds
no brain credentials — pinned by `relay_holds_no_brain_credentials`).

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
