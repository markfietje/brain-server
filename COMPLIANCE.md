# COMPLIANCE.md — Compliance Posture & Technical File

**Version:** 1.28.28 "Channel" · **Last updated:** 2026-08-25

This is the buyer-facing technical file: what brain-server IS, what it logs,
how it erases, and how it maps to the frameworks procurement asks about. It is
a documented engineering posture, **not a certification** — ISO/IEC 42001 /
SOC 2 attestation are organization-level audits that happen outside this
repository.

Companion engineering docs: `SECURITY.md` (OWASP Top 10:2025 map),
`THREAT_MODEL.md`, `AUDIT.md`.

**Verify, don't trust:** every claim in this file (and in `SECURITY.md` /
`OWASP_AGENTIC_2026.md`) is mapped to the release that shipped it and the exact
live `curl`/`brain` command that proves it in
[`docs/trust/proof-map.md`](./docs/trust/proof-map.md) (scripted walk-through:
[`docs/trust/reproduce.md`](./docs/trust/reproduce.md)). The buyer-facing
story lives in [`docs/product-site/`](./docs/product-site/) and the retrieval
mechanisms it cites are explained in [`docs/research/`](./docs/research/).

---

## 1. System Description

brain-server is a single-node, loopback-first memory component for an AI
system (the "client": an agent or an application using this server as its
memory backend). It stores knowledge chunks, their embeddings (vec0 KNN), a
lexical index (FTS5), and a knowledge graph (entities + relationships), and it
serves deterministic retrieval (`/recall`, `/search`). All data stays on the
host (SQLite; `~/.openclaw/workspace/brain.db` by default); there is no cloud,
no telemetry to third parties, and no data egress by default.

**Data flows** (all loopback HTTP unless stated):

```
client ── ingest ──► /ingest, /ingest/memory, /ingest/markdown ──► SQLite
client ── recall ──► /recall ──► embed → hybrid (vec0 + FTS5, RRF) → rank
                        └──► audit read-event (opt-in) ──► audit_events (hash chain)
operator ── DSAR ──► /dsar ──► locate → export → purge → tombstone → certificate
                          └──► Art 19 webhook (opt-in, outbound, HMAC-signed)
```

**Purpose limitation.** The system stores only what the client sends it. It has
no web crawler, no email, no location, no biometric collection. Ingestion
paths are explicit client calls; nothing is inferred or scraped.

## 2. Purpose Limitation & Data Minimization

- The server stores exactly the content it is given, chunked for retrieval. It
  does not enrich, infer, or profile.
- `POST /ingest` trusts the client's declared entities/relations — the client
  controls the schema of the graph.
- PII control is deterministic read-time output redaction for principals without
  `pii:read`/Admin (email/phone/Luhn-card patterns, conservative pattern match —
  "control, not a classifier"). No write-time placeholder vault; plaintext is
  never stored in a `pii_map` (removed v1.20.19).
- Read-event auditing is **off by default in loopback mode** (noise + the
  personal-use contract) and **on by default in JWT mode** (enterprise
  posture). Override: `BRAIN_AUDIT_READ_EVENTS`, sampling via
  `BRAIN_AUDIT_READ_SAMPLE_RATE`.

## 3. Logging Specification (EU AI Act Art 12 / Art 26(6) posture)

The audit is an append-only, tamper-evident hash chain (`audit_events`,
v1.1). Every row is SHA-256-chained to its predecessor (`prev_hash` over
`ts|kind|actor|target_hash|prev`), verified by `/audit/verify` and reported as
`brain_audit_chain_ok` in `/metrics`.

| Event kind | Recorded | Payload |
|---|---|---|
| Ingest (all paths) | always | hash-only target, kind, actor |
| AuthN/AuthZ | always | token-verified / rejected (reason) / authz-denied (principal/action/team/domain) |
| Webhook ingress | always | HMAC-verified (legacy GitHub `sha256=`; opt-in Standard Webhooks `v1,` when `BRAIN_WEBHOOK_TIMESTAMP_REQUIRED=1`, v1.20.4) |
| Reconcile / supersede / undo / DSAR | always | hash-only |
| **Read events (`recall`, `search`, `get`, `multi-get`)** | **opt-in (v1.15)** | caller principal (or `loopback`), target (query text / chunk refs), scope decision; recall also stores the replayable trace (chunk ids + scores + decision) in `recall_traces` |

**Hash-only invariant.** The audit row stores hashes, never content. The raw
query/identifier must never appear in the row (pinned by test).

**Retention.** `BRAIN_AUDIT_RETENTION_DAYS` (unset = keep forever for personal
use). When set, rows older than the window are pruned on read-event writes and
the hash chain is re-anchored — the oldest surviving row becomes the new
genesis. Deployers subject to AI Act Art 26(6) guidance should set ≥180.

**Decision-path evidence (Art 22 "meaningful information about the logic").**
`/recall?trace=true` returns a `trace_id`; `GET /recall/{trace_id}/trace`
replays the exact injected chunks, fused scores, abstention decision, access
scope applied, principal, and domains searched. The trace is the replayable
artifact proving *what* informed a retrieval — the log next to the answer.

## 3.5 Zero Trust for AI (ZT4AI) Posture (v1.20.5)

The enterprise posture, stated against what ships. Control-by-control mapping
to the 2026 OWASP agentic frameworks is in `docs/OWASP_AGENTIC_2026.md`; the
operative stance:

- **Workload identity** — agents are not shared service accounts: JWT
  (`sub`/`jti`) or did:key (v1.17.4) + capability tokens, per-principal, with a
  rotation cadence (tokens ≤90d).
- **Least agency** — the openclaw plugin holds exactly one verb (recall) on the
  automatic path, two (recall + proposal) after v1.20.1; write approval is
  per-action and outside the model's prompt (the LLM03/ASI01 policy-gateway
  pattern).
- **Rule of Two** — the `[A untrusted input, B sensitive data, C state
  change/external comms]` class requires per-action human approval; the v1.20.1
  gate is that approval for the memory-write action.
- **Egress** — exactly one outbound HTTP path (opt-in Art 19 DSAR webhook,
  HMAC-signed); that URI is the network action-boundary allowlist.

## 3.6 Audit-Ready-Replay Playbook (v1.20.5)

The 2026 production-readiness bar is *"if a system can't replay the agent's
reasoning and decision path, it is not ready for production."* brain-server has
the raw material; the evidence bundle for an incident / SOC 2 review:

1. **What happened** — `/audit` hash chain (SHA-256, tamper-evident) +
   `/audit/verify`.
2. **Why** — recall traces (`GET /recall/{trace_id}/trace`: query, decision,
   domains, actor) + the proposal-gate trail (`source_prompt` +
   proposed/approved/rejected/expired audit rows, v1.20.1). Since v1.27.12 an
   approval additionally records the SHA-256 `content_digest` of the
   read-canonical form the reviewer saw — the bundle can prove *exactly which
   bytes* were approved, and any drift would have been rejected (`409`).
3. **To whom** — principal pillar on every response; DSAR certificates +
   tombstone registry; `origin` per row (v1.18.2).
4. **For how long** — per-kind retention (`/retention`, v1.17.1) +
   `BRAIN_AUDIT_RETENTION_DAYS`.

Export paths already exist (`/export`, `/audit`, DSAR certificate); the playbook
is a documentation of how to assemble the bundle, no new code.

## 4. DSAR Workflow (GDPR Art 15/17/19 + CCPA/ADMT)

`POST /dsar {subject, action: export|purge|both}` (Admin-only):

1. **Locate** — every `knowledge` row with `owner = subject`, plus all
   transitive `derived_from` descendants (bounded walk, depth 8).
2. **Export** — portable JSON bundle of the located rows (no `pii_map`
   values), plus the subject's **channel rows** (`case_notes` by author /
   addressee / content match — the same arms the purge erases; since
   1.28.28). A legal hold defers affected targets: listed on the
   certificate (`held_ids` / held runs), never silently skipped.
3. **Purge** — hard delete in one transaction:
   `knowledge` + `vec_knowledge` + `relationships` + `evidence_links` +
   `proposals` references + the governed-workflow family (matched
   `workflow_runs` and every FK child: steps, outbox lineage events,
   findings, contradictions, `handover_offers`; `crm_cases` links are
   UNLINKED — the external CRM case outlives its erased run) +
   **channel rows** (notes/invites authored by or addressed to the subject,
   or carrying the subject in content — over-match, erasure-safe direction)
   + people-metadata (presence rows, skills tags; shift rosters rewritten to
   drop the subject); a tombstone row is left with the reason
   (`owner:<subject>` / `derived`) and, for derived chunks, the root
   `origin_id`.
4. **Certificate** — `{subject, action, found_count, purged_ids,
   tombstone_root, certified_at, chain_head}`; the `chain_head` is the audit
   chain tip at certification time. Re-fetchable via
   `GET /dsar/{id}/certificate` with a live `chain_verifies` recomputation.
5. **Art 19 onward-notification** — opt-in webhook (`BRAIN_DSAR_WEBHOOK_URL`
   [+ `BRAIN_DSAR_WEBHOOK_SECRET`]): on a completed purge, POSTs
   `{subject, certified_at, certificate_id}` HMAC-SHA256-signed
   (`X-Brain-Signature-256: sha256=<hex>`). Fail-soft: bounded retries, then
   logged; a webhook failure never rolls back the purge.

The deletion registry (`GET /tombstones?subject=&since=`) is the queryable,
hash-only, append-only tombstone view (the EDPB Coordinated Enforcement
Framework ask). Nothing is ever deleted autonomously; every deletion is an
explicit operator action.

## 5. Risk Controls (recap)

| Risk | Control |
|---|---|
| Memory injection / prompt poisoning | v1.14 proposal gate (`POST /ingest/proposal` → human approval); quarantine + flagged-row exclusion (v0.9.7) |
| Data leakage across tenants | JWT AuthN (v1.2) + per-record `access_scope` deny-by-default filter (v1.14) + per-domain pools (v1.0) |
| Unwarranted erasure | DSAR is Admin-only; purge is explicit + tombstoned + audited; `/purge` requires explicit ids/owner |
| Tampered audit | SHA-256 hash chain + `/audit/verify` + `/metrics` chain-ok gauge + DSAR certificates anchored to the chain |
| Unauthenticated access | Bearer token (opaque) or JWT/JWS + OIDC discovery (v1.2); loopback-first defaults |
| PII at rest | Not encrypted at rest — documented posture: full-disk encryption (LUKS/FileVault) is the operator's layer; PII control is deterministic read-time output redaction, no write-time placeholder vault (v1.20.19) |
| Exfiltration | No outbound HTTP by default; the only outbound path is the opt-in Art 19 webhook |

## 6. Framework Maps

### 6.1 ISO/IEC 42001 (AI management systems) + NIST AI RMF + SOC 2

| Control area | brain-server evidence | ISO 42001 | NIST AI RMF | SOC 2 |
|---|---|---|---|---|
| Identity & access | JWT/JWS + OIDC/JWKS, opaque bearer, AuthZ matrix per route (handler-entry gates, test-pinned) | 8.3 | Govern 1.1 / Map 3 | CC6.1, CC6.6 |
| Change management | semver releases, schema-contract + route-coverage tests, migration rehearsal tool | 9.4 | Measure 4.1 | CC8.1 |
| Monitoring | hash-chained audit + `/metrics` + `/audit/verify` chain-ok | 9.3 | Measure 4.2 | CC7.1–CC7.4 |
| Logging | append-only hash chain; read-event audit (opt-in, v1.15) | 9.3 | Measure 4.4 | CC7.2 |
| Data lifecycle | proposal gate, decay, DSAR export/purge/certificate, tombstones, retention pruning | 9.2 | Govern 2.1 | CC6.1, CC9.1 |
| System ops | `/health` + capacity envelopes + `bench --envelope` ship gate | 8.4 | Measure 5.1 | CC7.3 |

### 6.2 Intent-Based Auditing 4/4

| Pillar | Evidence |
|---|---|
| Identity | JWT principal (`sub`/`tenant`/`scopes`), OIDC discovery, JWKS (v1.2) |
| Decision-path (memory reads/writes) | Read-event audit + `GET /recall/{trace_id}/trace` replay (v1.15) |
| Policy attribution | AuthZ matrix at handler entry (v1.12.1, test-pinned route-by-route) + record-level `access_scope` (v1.14) |
| Selective-forgetting | DSAR locate→export→purge→certificate + tombstone registry (v1.15) |

### 6.3 Jurisdiction Posture

- **Philippines (DPA RA 10173):** personal data stays on the host; the DSAR
  workflow + deletion certificate are the data subject's rights-of-erasure
  implementation; breach notification is operator-side (the server provides
  the audit evidence).
- **EU (GDPR):** Art 15/17 (DSAR + erasure certificate), Art 19 (onward-
  notification webhook), Art 22 (trace replay = meaningful information about
  the logic), Art 26(6) guidance (retention ≥180 days where required).
- **California (CCPA/CPRA + ADMT):** the Art 22 trace maps to the ADMT
  right-to-know logic explanation; `/export` is the data-portability response;
  `/tombstones` evidences completed deletion requests.
- **Residency:** loopback-first means the data physically never leaves the
  host; cross-border deployment is purely a deployment choice, not an
  architectural one.
- **EU CRA horizon:** the embedded-dependency story is documented in
  `SECURITY.md` (bundled SQLite 3.53.2, cargo-audit gate); no change to the
  data path.

### 6.4 EU AI Act Art 4 (AI literacy)

The deployer's literacy evidence for the memory component: the dashboard's
controls (`/proposals` review queue, `/quarantine`, `/audit`, `/tombstones`)
and the trace endpoint make the component's decisions inspectable —
"what informed this retrieval, and who approved this write" — which is the
operational substance of Art 4 literacy for a memory component. The playbook
is served as `GET /.well-known/ai-literacy` (public) and mirrored in
`docs/AI_LITERACY.md`.

### 6.5 OWASP ASI06 — Memory & Context Poisoning (Agentic Top 10 2026)

OWASP's GenAI guidance splits into two documents, meant to be read together.
The **GenAI LLM Top 10 2026** (published 2026-08-04, grounded in 7,714 real AI
incidents, 6,639 classified) is the LLM list (LLM01–LLM10; memory-adjacent
entry **LLM09 Vector and Embedding Weaknesses**). The memory-persistence
attack surface itself is named **ASI06 Memory and Context Poisoning** in the
companion **OWASP Top 10 for Agentic Applications**, which launched
**2025-12-09** (ASI06 has been an entry since that initial release) — a
distinct agentic list, not an entry of the LLM Top 10. This section maps
brain-server's controls to ASI06's prescribed defenses and to the 2026
research that motivated the category.

**Why the category exists (2026 disclosure timeline).** Two independent
disclosures showed persistent-memory poisoning succeeding against current
agents: **MemGhost** ("When Claws Remember but Do Not Tell", arXiv 2607.05189,
disclosed 2026-07-06; CSA research note 2026-07-23) achieved 87.5% end-to-end
success in background-execution mode (75.0% foreground, 100% stealth) against
OpenClaw on GPT-5.4 via a single crafted email; **GhostWriter** ("When Agents
Remember Too Much", arXiv 2607.06595) achieved ~98% memory injection (and ~60%
activation of the poisoned entries during later benign tasks) across five
state-of-the-art agents. Both converge on the same root cause the mitigation
literature names: **memory writes are a first-class, auditable action**, not a
silent side effect of reading untrusted input.

**Control mapping (brain-server → ASI06 + research mitigations):**

| ASI06 / research mitigation | brain-server evidence |
|---|---|
| Memory sanitization with provenance | Per-row model-vs-human `origin` marker (v1.18.2) + `source` / `assertion_kind` / `confidence`; `/export` provenance_summary (below, §7) |
| Write-time gating (not silent auto-commit) | v1.14 proposal gate (`POST /ingest/proposal` → human approval) + quarantine/flagged-row exclusion (v0.9.7); no autonomous write promotion |
| Trust-aware retrieval | Superseded rows excluded from default recall (`valid_to IS NULL`); quarantined rows never served; `min_relevance` + scope-filter-before-ranking |
| Behavior / write audit (MemGhost + GhostWriter's AM-Sentry audit trail) | Append-only SHA-256 hash-chained audit (§3); every ingest/approve/supersede/DSAR recorded (hash-only); read-event audit + replayable `/recall/{id}/trace` (v1.15) |
| Erasure / forensic recovery of a poisoned entry | DSAR locate→purge→tombstone→certificate (§4); `/purge` by id/owner; supersession (`valid_to`) retires a bad entry and its traceable dependents |
| Integrity (verify-before-emit) | UMP §2.8 content-hash + Ed25519 signature, verify-on-read — tampered records dropped, never served (§9) |

**Honest posture.** This is a control map to ASI06's recommendations, not a
claim of ASI06 conformance: the category is recent, its recommended control set
is still consolidating, and brain-server is a single-node memory component (no
federated store, no ingestion of email/calendar — the MemGhost/GhostWriter
delivery vectors). What the component does provide is the memory-governance
layer those attacks found missing: provenance at write time, a human gate, a
hash-chained audit of every memory change, and a tombstone path for the entries
an attacker would plant. See `docs/MEMGHOST_MITIGATION.md` for the operative
playbook.

### 6.6 EU AI Act applicability after the Digital Omnibus (Regulation (EU) 2026/1744)

**For procurement reviewers tracking the August 2026 headlines:** the EU AI
Act was **not** delayed as a whole, and this component's posture does not
depend on the deferral. The Digital Omnibus on AI — Regulation (EU)
2026/1744, published in the Official Journal 24 July 2026 and in force
27 July 2026 — moved only the high-risk regime: obligations for stand-alone
high-risk systems (Annex III) now apply from **2 December 2027**, and for
AI embedded in regulated products (Annex I) from **2 August 2028**. Both are
fixed statutory dates, not conditional triggers. Everything else held its
original schedule:

| Obligation | Applies | Status |
|---|---|---|
| Art 5 prohibited practices | 2 Feb 2025 | in force |
| Art 4 AI literacy | 2 Feb 2025 | in force (§6.4) |
| GPAI model obligations (Art 51–56) | 2 Aug 2025 | in force |
| General application + enforcement | 2 Aug 2026 | live |
| **Art 50 transparency** | **2 Aug 2026** | **live — see §7** |
| High-risk, stand-alone (Annex III) | ~~2 Aug 2026~~ → **2 Dec 2027** | deferred by 2026/1744 |
| High-risk, product-embedded (Annex I) | ~~2 Aug 2027~~ → **2 Aug 2028** | deferred by 2026/1744 |

**Where brain-server sits.** This component is a memory store inside a wider
AI system — it generates no content, makes no autonomous decisions, and by
itself is not an Annex III use case. If the DEPLOYER's overall system is
classified high-risk, the deferral moves that conformity deadline to
2 December 2027; it does not remove any obligation. The deployer-side
groundwork the high-risk regime expects is already built and documented here:
Art 12-style logging (§3), Art 13-style technical documentation (this file),
Art 14-style human oversight (the v1.14 proposal gate + review queue), and
the audit chain every claim above is pinned to. A reviewer reading the
deferral as a reason to deprioritize should read §7 instead: the obligations
that were NOT deferred are the ones this component serves directly.

**Art 50 went live on schedule — and so did this server's disclosure.**
Article 50 transparency obligations apply from **2 August 2026** (untouched
by the Omnibus), and `GET /.well-known/ai-notice` has carried
`art_50: true` with `effective_date: 2026-08-02` since that exact date. Providers of
generative-AI systems already placed on the market before
2 August 2026 have a grace period ending 2 December 2026 to comply with the
Article 50(2) machine-readable-marking duty; systems placed on the market
from 2 August 2026 mark from day one.

## 7. Machine-Readable Origin Metadata (Art 50 transparency bridge)

Every `knowledge` row carries an ingest path (`source`: manual | vault |
memory | markdown | structured) and, where present, `assertion_kind`
(stated/observed/inferred) + `confidence`. Since v1.18.2, every row also
carries an explicit model-vs-human **`origin`** marker: `human` (interactive
`manual` writes, the only path that claims human authorship), `model`
(`memory` auto-capture / assistant writes), or `imported` (the safe default
for `markdown`/`structured` bulk imports and any unknown path). `/export`
emits `source` + `origin` + `assertion_kind` + `confidence` with the content,
adds an `export_format_version: 2` envelope, and computes a
`provenance_summary` (`total` / `by_origin` / `by_source`) — so a memory
export can state *how a memory entered the system and who produced it*, the
Art 50 model-vs-human transparency bridge. (Forward link: UMP wire-format
conformance is a later release.)

**Posture as of 2026-08-25** (corrected; the previous note here misdated the
GPAI timeline). The AI Act's phased application is set by Regulation (EU)
2024/1689 itself: **GPAI model obligations (Art 51–56) have applied since
2 August 2025**, and the Commission's enforcement powers over GPAI providers
begin 2 August 2026 — neither date was changed by the Digital Omnibus
(§6.6). Regulation (EU) 2026/1744 is the high-risk deferral instrument, not a
GPAI one. The **2 December 2026** date ends the Article 50(2) grace period that the
Digital Omnibus grants to providers of generative-AI systems already on the
market before go-live for machine-readable marking of their synthetic
content output. brain-server does not train or host GPAI models —
the GPAI obligations fall on the model providers whose outputs may be
ingested — so the memory component's duty is provenance, not watermarking:
every stored row keeps `source` / `origin` / `assertion_kind` / `confidence`
(above) so a deployer can attribute AI-generated content and honor the
transparency expectations of Regulation (EU) 2026/1744 for content that
passes through its systems.

**Enforcement (v1.18.2).** Art 50 transparency obligations are enforced by
the national market surveillance authorities (not the central AI Office). The
penalty tier for Art 50 violations is **€15M or 3% of worldwide annual
turnover (Art 99(3))** — the **€35M / 7%** tier is Art 99(2), which applies to
Art 5 prohibited practices and GPAI provider obligations (Art 66), not to the
Art 50 transparency line. Do not conflate the tiers when quoting this
posture.

**Machine-readable disclosure.** The server also serves the Art 50 disclosure
itself at `GET /.well-known/ai-notice` (public, no auth) — a JSON document
with `art_50: true`, the human-readable disclosure that stored content may be
AI-generated, the `origin_metadata` fields a consumer can read per row
(`source` / `origin` / `assertion_kind` / `confidence`),
`effective_date: 2026-08-02`, and `jurisdiction: EU AI Act Article 50
(Regulation (EU) 2024/1689)`. A deployer can point a consumer or auditor at
the URL and at an `/export` bundle to close the model-origin transparency loop
without pasting a policy.

### 7.1 EU AI Act Code of Practice marker (v1.17.1 M6)

The server serves `GET /.well-known/cop-notice` (public, no auth) — a
machine-readable, **self-attested** conformity marker the client's CoP icon
lane renders: the covered commitments (human-in-the-loop write-back, no
autonomous generation, inspectable decisions, full audit chain), the
self-assessment pointer (`COMPLIANCE.md`), and a `last_review` timestamp. It
is declarative — a posture statement, not a certification badge (external
conformity assessment is an operator gate).

## 8. Retention Classes

| Class | Contents | Default | Mechanism |
|---|---|---|---|
| Permanent | audit chain (excluding pruned window), tombstones, `dsar_requests`, proposals ledger | forever (personal use) | `BRAIN_AUDIT_RETENTION_DAYS` opt-in prune with chain re-anchor |
| Transient | read-event traces (`recall_traces`) | same window as the audit rows they key | pruned with their audit row |
| Content | knowledge chunks | until explicitly purged | `/purge` (ids/owner), DSAR, supersession (`valid_to`) |

## 8.5 Release Evidence & Supply Chain (v1.17.5)

Evidence attached to every release, so compliance claims are checkable from
the artifact itself:

- **SBOM ships with the release** (EU CRA Art 13/14; OWASP A03:2025). The tag
  release workflow generates a CycloneDX SBOM from `Cargo.lock` via
  `scripts/sbom.sh` (`cargo-cyclonedx`) and stages it into `dist/` alongside
  the binaries; the same script is the local operator path. `SECURITY.md`
  §SBOM explains scanning use.
- **Retrieval-quality gate runs in CI.** A `recall-gate` job seeds the frozen
  10-doc smoke corpus into a scratch instance and runs `brain eval` with
  `--floor r5=0.85,r10=0.85,mrr=0.85` — a regression on the frozen judged set
  fails the build. Baseline recorded in `BENCHMARKS.md` (37 queries: r@5
  0.919, r@10 0.919, nDCG@10 0.911, MRR 0.905). This is a wiring/regression
  gate, **not** evidence of parity: per the BENCHMARKS.md protocol, parity
  rows stay `PENDING` until ≥100 judged queries on a representative corpus on
  target hardware (incl. the 4 GB ARM edge).
- **`brain eval` itself is now live.** The v1.17.5 CLI fix restored the
  endpoint that the v1.17.1 M3 ship gate was built on (`brain eval` had been
  sending GET to the POST-only `/recall`, so every run returned 405 and the
  gate never scored); the frozen-fixture runs above are real.
- **UMP conformance gate runs in CI** (see §9): the reference suite's
  `UMP 1.0 / L3` badge line is asserted on every push, keeping the README
  badge honest.

## Honest Ceilings

- Single-process audit chain (distributed audit = v2.1 multi-instance work).
- Read events default off in loopback mode (personal-use contract); JWT mode
  on. A loopback deployment must set `BRAIN_AUDIT_READ_EVENTS=on` explicitly
  to collect read traces.
- No PII encryption at rest — full-disk encryption is the operator's layer.
- DSAR export is brain-server JSON (the `?format=ump` wire form covers the
  `/export` portability path, not the DSAR certificate envelope).
- This file is a documented posture, not a certification.
- The CoP marker is self-attested; certification is an external gate.
- Retention is query-time + kind-default (`/retention`); no per-record TTL
  roll-up worker, no autonomous archival.
- The trace endpoint serves recorded events only; there is no historical
  backfill for recalls that predate v1.15.0.

## 9. UMP 1.0 Integrity + Consent Controls (v1.17.3)

> **v1.17.5 (2026-08-09):** UMP conformance is now **externally verified, not
> self-attested**. The reference conformance suite
> (`@universalmemoryprotocol/core` 1.0.0) scores **13/13 checks, UMP 1.0 / L3**
> against a fresh keyed instance, and a CI job re-runs the suite on every push
> and asserts the badge line — the README's `UMP 1.0 / L3 verified` badge
> cannot go stale. The suite's L1.remember dedup (`merged` on rerun against a
> persistent store) is content-dedup by design; the correct runner target is a
> throwaway keyed instance with a fresh DB, same as the reference `ump-serve`.

The UMP binding's §5 obligations mapped onto the existing controls:

| UMP obligation | Mechanism |
|---|---|
| Record integrity (§2.8) | Ed25519 signature by the operator key (`brain ump keygen`, `BRAIN_UMP_KEY_DIR`); verify-on-read — tampered records are dropped, never served |
| Capability-gated access (§5.2) | Bearer capability tokens on `/ump/*` + `/export`: signature/expiry at the middleware, verbs × scope at handler entry; `audit` surfaces deny token bearers |
| Consent (declared owner) | `scope.owner` must match the authenticated principal (or be absent → principal's); mismatch → `forbidden_scope`, recorded in the audit log |
| Erasure (Art 17 parity) | `POST /ump/forget` both arms (soft flag / hard `purge_chunk_ids`) tombstone + audit — the v1.14 erase path, not a bypass |
| Auditability (§9) | `/ump/audit` + `/ump/audit/verify` alias the existing SHA-256 hash-chained log; `capabilities.audit: true` |
| Change-feed privacy | `/ump/subscribe` carries `{kind,id}` only — event bodies never leave the DB (documented §3.8 posture) |
| Injection-resistant rehydration (§5.3) | Verify-before-emit + scope-filter-before-ranking on the server; client obligations documented in `SECURITY.md` §UMP (bodies are data, never commands) |

## 10. Regulated-Sector Compliance Pack (v1.22.0)

> **v1.22.0 "Regulated" (2026-08-15):** the *enforcement* behind the v1.21.0
> policy fields. Each subsection is a **documented posture** mapping the
> shipped controls to a regulated buyer's evidence need — it is **not a
> certification**, and certification remains the operator's own external audit.

### 10.1 HIPAA (45 CFR Part 164 — Security Rule + §164.502(g))
The health-memory use case trails whether a fact applies to a subject, can be
provably isolated to it, and is built on a strict-mode masking layer for PHI.

| HIPAA requirement | brain-server evidence (v1.22) |
|---|---|
| Access controls (§164.312(a)(1)) | JWT/JWS + OIDC/JWKS + opaque bearer; per-route AuthZ matrix (handler-entry gates, test-pinned); record-level `access_scope` (v1.14) is the min-necessary filter |
| Audit controls (§164.312(b)) | Append-only SHA-256 hash-chained audit (§3) of every ingest/approve/erase; read-events via `BRAIN_AUDIT_READ_EVENTS`; `/audit/verify` chain-ok |
| Integrity (§164.312(c)(1)) | Supersede-not-delete + per-row content hash + UMP §2.8 signature verify-on-read (§9) |
| Transmission security (§164.312(e)(1)) | Loopback-first (data physically never leaves the host); TLS is the operator's reverse-proxy layer |
| Minimum necessary (§164.502(b)) | `access_scope` + PII read-path redaction (`redact_content` for non-admin); `GET /proposals` review before any write promotes |
| PHI tokenization | v1.14.2 strict masking writes one-way `[redacted:*]` placeholders, not raw PHI, at the write boundary |
| Storage limitation / retention | per-domain + per-kind TTL policy (v1.21 profiles) now **reportable**: `GET /retention/report` is the Art 5(1)(e)/HIPAA retention-schedule evidence (kind → ttl_days → count → 30d-expiring) |
| Breach deferral / litigation hold | `legal_holds` freezes a chunk against every erasure path (decay skip, `/purge` + `/dsar` `409 legal_hold_active`) until released — the WORM-lite posture for a pending matter |
| Business Associate note | BAA is a contractual layer between the covered entity and the operator; brain-server provides the technical-file evidence the BAA's compliance annex refers to. |

### 10.2 SOX (17 CFR §229 / PCAOB AS 2201)
SOX centers audit integrity, retention, and anti-anti-fraud controls over the
financial reporting process; the memory store's role is to evidence *what was
known, when, and who changed it* without loss or silent alteration.

| SOX control theme | brain-server evidence |
|---|---|
| Immutable audit trail | Append-only SHA-256 hash-chained audit (§3); a tampered chain fails `/audit/verify` |
| No silent alteration | Supersede-not-delete: a revised record becomes a new revision and the old one is retired (`valid_to`), never overwritten |
| Records preservation | Retention classes (v1.21) + `GET /retention/report` (v1.22) prove the schedule; legal hold freezes records against erasure during an investigation |
| Erasure refusal (litigation hold) | `/purge` + `/dsar` refuse a held id with `409 legal_hold_active` + the hold reasons; release is explicit (`POST /legal-hold/{id}/release`), never automatic |
| Access for auditors | Admin-scoped `/audit`, `/legal-holds`, `/retention/report` endpoints; the certificate lists deferred-erasures (`held_ids[]` + reasons) so a matter is answerable |

### 10.3 FedRAMP / FISMA (NIST 800-53 control posture)
A posture against the *control families* a federal buyer would interrogate,
mapped to the shipped, test-pinned evidence. Single-tenant, loopback-bound:
the boundary, encryption, and monitoring layers the operator positions.

| NIST 800-53 family | brain-server evidence |
|---|---|
| AC (Access Control) | JWT/OIDC/JWKS + opaque bearer; per-route AuthZ matrix; record-level `access_scope` (v1.14) |
| AU (Audit & Accountability) | Hash-chained audit (§3), `/audit/verify`, `/metrics` continuous-monitoring feed, replayable `/recall/{id}/trace` (v1.15) |
| SC-28 (Protection at Rest) | No application-level encryption at rest — full-disk encryption is the operator's layer (explicitly documented, not assumed) |
| SC-7 (Boundary Protection) | Loopback-first binding (data physically never leaves the host); the network boundary + TLS terminate at the operator's reverse proxy |
| SI-12 (Information Handling) | Retention classes + `GET /retention/report` (v1.22) demonstrate storage-limitation; DSAR locate→purge→certificate (§4) is the deletion path |
| IR (Incident Response) | The hash-chained audit + tombstones supply the forensic evidence an IR plan consumes; legal hold preserves evidence during a response |

### 10.4 The new v1.22 controls (across the maps)
- **Legal hold** (`legal_holds`, §10.1/§10.2): a frozen id is absent from the
  `/decayed` registry, refused by `/purge` + `/dsar` (`409 legal_hold_active` +
  reasons), and deferred (with reasons) on a DSAR certificate — the legal-
  defensibility artifact for litigation/regulatory hold across every sector.
- **Retention reporting** (`GET /retention/report`, §10.1/§10.3): the
  storage-limitation proof regulators ask for, per domain × kind → ttl_days →
  count → 30d-expiring.
- **Region pin** (`BRAIN_REGION`): the residency stamp proves *where data
  lived* (is a built-in DSAR-certificate field: e.g. `eu-west-1`,
  `ph-manila`), the evidence a residency clause or a jurisdiction map (§6.3)
  points at. A stamp is never rewritten, so history is preserved across a
  region change.
