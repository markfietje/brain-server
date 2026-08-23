# The memory lifecycle

*How a fact becomes memory — from capture to admission, storage, retention,
recall, and erasure. Every claim here is read from the source
(`src/handlers/ingest.rs`, `src/handlers/gate.rs`, `src/gate.rs`,
`src/chunker.rs`, `src/main.rs`, `plugin/index.ts`).*

This is the end-to-end companion to the two half-lifecycle documents: the
**write** gate in [Human in the loop](./human-in-the-loop.md) (the human's
review job) and the **remove** gate in that page's [§7 the erasure
procedure](./human-in-the-loop.md#7-the-erasure-procedure). Here you get the
whole loop as one flow.

---

## The two capture topologies

Every bit of knowledge enters through **one of two paths**, and which one a
given source uses is fixed by its entry point:

| Topology | What happens | Used by |
|---|---|---|
| **Gated (proposal)** | A candidate is screened, scored, and held in the review queue. It becomes memory **only after a human approves** it. | Agent autoCapture + the `memory_store` tool under the default `captureMode: "proposal"`. |
| **Direct** | The candidate is screened and written straight to memory in one transaction. | `POST /ingest` (structured), `/ingest/memory`, `/ingest/markdown`, `/add`, `ingest-dir`, UMP, connectors. |

Direct writes are still **screened** by the server injection gate — "direct" means
*no human approval step*, not *no safety control*. The two modes are the plugin's
`captureMode`; everything else is inherently direct.

---

## Step 0 — The entry points

All knowledge enters through one of these handlers. The **source** column is
the ingest kind; it drives the **origin** marker (`human` / `model` / `imported`)
and, for connectors, a confidence discount.

| Entry | Route / trigger | Source (`knowledge.source`) | Origin | Path |
|---|---|---|---|---|
| Agent autoCapture | Plugin `before_prompt_build` → `submitProposal` (`proposal`) or `store` (`direct`) | `agent_end` (proposal) / `structured` (direct) | imported | gated or direct |
| `memory_store` agent tool | plugin tool → same routing by `captureMode` | `memory_store` (proposal) / `structured` (direct) | imported | gated or direct |
| Structured (KG) | `POST /ingest` | `structured` | imported | **direct** |
| UMP records | `POST /ingest?format=ump` / `?format=ump-md`, `POST /ump/remember` | `structured` + UMP overlay | imported | direct |
| Legacy memory | `POST /ingest/memory` | `memory` | **model** | direct |
| Single chunk | `POST /add` | — | — | direct |
| Markdown import | `POST /ingest/markdown` | `markdown` | imported | direct |
| Directory / vault | `brain ingest-dir <path>` | `markdown` / `vault` | imported | direct |
| Source reconcile | `brain reconcile` / `POST /sources/reconcile` | — | imported | direct |
| Connectors | github / webhook | contains `connector`/`github`/`web` | imported | direct (confidence ×0.9) |

**Origin mapping (from `gate::origin_for_source`):** `manual` → `human`,
`memory` → `model`, everything else → `imported`. The safe fallback is
`imported`. Note this means modern agent captures land as **imported**, not
`model` — their sources are `agent_end` / `memory_store` / `structured`, none of
which equals `memory`. Only the legacy `/ingest/memory` path (source `memory`)
is marked `model`; only interactive `manual` writes claim human authorship.

**Bounds (from `handlers/mod.rs`):** `MAX_TITLE` 500 chars, `MAX_CONTENT`
1,000,000 chars, `MAX_ENTITIES` = `MAX_RELATIONS` = 200, `MAX_QUERY` (proposal
content) 2,000 chars, `MAX_SOURCE_PROMPT` 2,048 bytes.

---

## Step 1 — Injection screening (every write)

Every write path — structured, memory, markdown, and proposal — first runs the
content through the **two-layer injection screen** (`src/screen.rs`): a
deterministic blocklist plus an optional classifier. The outcome is one of:

- **`Reject`** → HTTP `400`, never persisted. (For proposals this means the
  review queue only ever sees `clean` or `quarantine`.)
- **`Quarantine`** → content is stored **but flagged**: excluded from retrieval
  and its knowledge-graph edges are skipped, so a flagged plant can't pollute
  recall or the graph. The badge is recomputed deterministically at read time so
  a reviewer can't miss it.
- **`Clean`** → proceeds normally.

The `source_prompt` (the exact capture trigger an agent sends) is bounded to
2,048 bytes and **PII-screened at persist** (`gate::screen_source_prompt`) so an
email/phone/card in the trigger text never lands raw in the review queue.

---

## Step 2 — The gate: score, then hold (proposal path only)

For gated captures, `POST /ingest/proposal`
(`src/handlers/gate.rs::ingest_proposal`) does **no `knowledge` insert**. It
computes three deterministic scores and stores a row in `proposals`:

- **Novelty** — `1 − max cosine` against current chunks via the vec0 KNN
  (`gate::novelty`). No existing chunks → `1.0` (first memory).
- **Conflict** — whether a live chunk's subject conflicts (`find_conflict`,
  reusing the consolidation machinery). Surfaced so a reviewer sees the
  trade-off, never a silent overwrite.
- **Salience** — a 0..1 length-band heuristic with an entity-density bump
  (`gate::salience`; filler < 24 chars scores low, verbatim logs > 3,000 chars
  cap low).

It also records an audit row (`proposal_pending`) and publishes a `pending`
alert (a `screen` alert fires separately if the injection screen tripped). The
plugin's `source_prompt` is stored (screened) so a reviewer can see *what the
agent was doing* when it captured.

```
 capture ─► screen(content,title) ──► Reject → 400 (never persisted)
                                        │ Quarantine → stored + badged, no graph edges
                                        │ Clean
                                        ▼
                    score: novelty (vec0 KNN) · conflict (consolidate) · salience
                                        │
                                        ▼
                        INSERT INTO proposals  + audit proposal_pending  + alert
                                        │
                                        ▼  (human) GET /proposals?status=pending
                         ┌──────────────┴──────────────┐
                         ▼                              ▼
                   approve (→ Step 3)              reject / expire
```

The review queue (`GET /proposals`) returns each candidate with its **score
components**, its **read-time screen verdict**, an **expiry deadline**
(`expires_at = created_at + BRAIN_PROPOSAL_TTL_SECS`, default **7 days**), the
SLA bands (`warn_secs` 1 hr, `critical_secs` 5 min), and — for decided rows —
`decided_at` (the v1.20.23 calibration signal). Since v1.27.12 the queue serves
the **read-canonical review form** (PII-redacted, markdown-ref-stripped,
invisible-Unicode-free) plus a stable SHA-256 `content_digest`; the approve
call may carry that digest and is rejected (`409`) on any drift — the decision
binds to the bytes shown. The default page limit is 50, hard-capped at
`MAX_PROPOSALS` = 200.

**TTL expiry:** a pending proposal older than the TTL is **refused** (neither
approve nor reject) — its capture context is unrecoverable. `expire_if_stale`
marks it `rejected` with `decided_at` and an `proposal_expired` audit row.

---

## Step 3 — Admission: approve (the write)

`POST /proposals/{id}/approve[?supersedes=<id>]`
(`src/handlers/gate.rs::approve_proposal`) promotes a candidate into long-term
memory in **one `IMMEDIATE` transaction** that:

1. Re-checks the TTL and **CAS-es the row** (`UPDATE … WHERE id=? AND
   status='pending'`) — a concurrent approve/reject can't double-promote.
2. **Embeds** the content (static model2vec).
3. **Inserts the `knowledge` row** — `node_kind` = the proposal's kind,
   `assertion_kind` = `stated`, `confidence` computed from source/conflict/
   assertion (`gate::confidence`), `origin` = `origin_for_source(source)`,
   `owner` = the principal's subject (or NULL for loopback).
4. **Inserts `vec_knowledge`** (`vec_quantize_int8(…,'unit')` + binary).
5. **Optionally supersedes** `?supersedes=<id>` → `resolve_supersession` in the
   same tx (approving a conflicting fact atomically expires the old one).
6. Sets `status='approved'`, `decided_at`, and audits `proposal_approved`.

`POST /proposals/{id}/reject` and `POST /proposals/{id}/edit` handle the other
outcomes; a rejection is audited (the decision enters the chain, not a free-text
rationale) and **never deletes** the proposal row.

---

## Step 4 — Direct admission (structured, memory, markdown)

The direct paths write through one shared core (`ingest.rs::ingest_one` for
structured, `main.rs::ingest_memory` / `ingest_markdown` for the others):

1. **Validate + screen** (bounds, injection screen).
2. **Dedup** — compute `content_hash` = xxh3-64 of the content; an existing
   row with the same hash returns `duplicate` (idempotent, no new row).
3. **Embed** the content (one static-model pass).
4. **Route the domain** — forced if given, else auto-routed to the nearest
   centroid (`domain_router`); no confident centroid → `global`.
5. **Write, in one transaction:** `knowledge` + `vec_knowledge` + (for
   structured) `entities` / `relationships`. The graph upserts are idempotent:
   a re-ingested relation with an **unchanged** window is a no-op (no history
   churn); a re-ingested relation with a **changed** window retires the old edge
   (`superseded_at` = transaction-time end, old row preserved verbatim) and
   inserts the corrected version as the new current belief (v1.27.22). Relations
   auto-create missing endpoint entities and carry a four-timestamp
   bi-temporal model — `valid_at` / `invalid_at` (valid time) + `created_at` /
   `superseded_at` (transaction time) — with explicit caller value winning over
   a deterministic extractor over the content.
6. **Recompute the domain centroid** (best-effort) so future queries route to
   it.
7. Record `pii` flag from `gate::scan_pii` (email / phone / Luhn card).

Markdown import chunks with a **CommonMark-aware splitter**
(`src/chunker.rs::chunk_markdown`, heading-boundary splits, code-fence-safe,
`MAX_CHUNK_BYTES` = 1,000) — one `knowledge` row per chunk. Legacy memory
(`/ingest/memory`) parses `## [ … ]`-headed blocks into `(title, text)` entries
(`parse_memory_content`) and strips reasoning traces + `BRAIN_INGEST_SKIP_PATTERNS`
prefixes at the door.

UMP records lower into the structured path with an **overlay** persisted onto
the row (`node_kind`, `assertion_kind`, `confidence`, `access_scope`,
`expires_at`, `observed_at`, `valid_from/to`, `ump_meta`), and compute a
content-addressed `ump_id` = `domain \0 content` so re-imports land on the same
id.

---

## Step 5 — Storage layout

| Store | What lives there | Written by |
|---|---|---|
| `knowledge` | The row: `title`, `content`, `source`, `content_hash`, `domain`, `pii`, `owner`, `node_kind`, `assertion_kind`, `confidence`, `access_scope`, `expires_at`, `valid_from/to`, `observed_at`, `authority`, `origin`, `ump_id`/`ump_meta` | all paths |
| `vec_knowledge` | int8 (`vec_quantize_int8 'unit'`) + binary embeddings | all paths |
| FTS5 | tokenized text for lexical recall | all paths |
| `entities` / `relationships` | the knowledge graph, four-timestamp bi-temporal (valid + transaction time; `superseded_at IS NULL` = current belief) | structured (+ consolidate + v1.27.22 edge supersession) |
| `proposals` | gated candidates + scores + `decided_at` | proposal path |
| `sources` | reconciled source bookkeeping | ingest/sources |

---

## Step 6 — Retention & decay

Decay is **query-time and deterministic**, never a background worker:

- A chunk's own `expires_at` always wins.
- Otherwise the per-kind retention policy derives a default from the row's
  creation age (`gate::effective_expiry`).
- `/decayed` lists already-expired rows for human review; `retention_reason`
  distinguishes `per_chunk` vs `kind_policy` decay. Historical recall
  (`?at=<past>`) composes decay and supersession orthogonally.

---

## Step 7 — Retrieval

Recall is hybrid (vector + FTS5 + graph) with calibrated abstention
(`low_confidence`, no hits → "I don't know") and deterministic span
verification. Every emitted text field passes through `gate::sanitize_read`
(PII redaction for non-`pii:read` principals + invisible-Unicode strip).
See [Features](./features.md) and the API reference.

---

## Step 8 — Erasure

Erasure is **human-only and Admin-scoped**. Every delete path (DSAR subject
purge, Data-panel purge, quarantine delete) writes a tombstone + a SHA-256 audit
row; there is no agent-callable delete and the `brain` CLI has no erase command.
Follow the documented procedure in [Human in the loop §7](./human-in-the-loop.md#7-the-erasure-procedure).

---

## The honest framing

- **"Gated" applies to auto-capture, not to everything.** Structured ingest,
  markdown import, UMP, and connectors are **direct** — they go straight to
  memory (still screened). If a deployment wants *every* write human-gated, that
  is a policy choice at the caller, not a server invariant.
- **Scores rank, they never promote.** Novelty/conflict/salience are displayed
  so a human can decide; nothing auto-approves.
- **Deterministic, not learned.** Screen, scoring, PII scan, chunking, and
  temporal extraction are heuristic/deterministic — zero tokens, no LLM, no
  background worker. That is the design constraint, not a limitation.
- **Dedup is exact, not semantic.** `content_hash` (xxh3-64) catches identical
  re-ingests, not paraphrases — near-duplicates are a *review* concern
  (`check-consistency`), not a write-time one.

---

## See also

- [Human in the loop](./human-in-the-loop.md) — the review gate + erasure procedure.
- [Features](./features.md) — the capability tour.
- [API contract](./API_CONTRACT.md) — the endpoint reference for `/ingest` + the gate.
- [OpenClaw integration](./openclaw-integration.md) — the capture flows as wired in the plugin.

## The continuity contract (v1.28.21 Fathom)

Workflow runs are **unbounded durable sessions**: a case lives in ONE run from
intake to close — there is no session rotation, no "start a new chat when the
context fills". Consumers derive context on demand instead:

- **Derivation API** — `GET /workflow/runs/{id}/context?at_event=&budget=`
  returns the deterministic window: latest `workflow/checkpoint` at-or-before
  the anchor + the delta events after it + per-finding digests + the open
  question. Field-budgeted (delta drops oldest-first; anchor and question
  never drop) with a `truncated` marker. One counted field ≈ one token — an
  approximation, documented, not guessed.
- **Checkpoint cadence** — the engine emits checkpoints at fixed, replayable
  boundaries: every AskHuman pause, every phase transition (`Advance`), every
  N events (`BRAIN_CHECKPOINT_EVERY`, default 25, ceiling 100), and once at
  completion.
- **LLM-side compaction is the consumer's contract** — brain-server never
  summarizes (zero-token rule). The consumer calls the derivation API and
  compresses the returned window on its side.
- **Rewind replaces rotation** — a wrong turn is a branch
  (`POST /workflow/runs/{id}/rewind`), never a new session; history stays
  fully queryable.
- **Stream resume** — SSE consumers carry `Last-Event-ID`; the server replays
  the gap and `GET /workflow/runs/{id}/events?since=` backfills anything older.

See [OpenClaw integration](./openclaw-integration.md) for the plugin-side wiring.
