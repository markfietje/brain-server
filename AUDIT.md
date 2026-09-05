# Audit Register — brain-server

Working log of security/correctness/quality audits, findings, and the research
each finding is grounded in. Each audit ships its gaps closed or carries them
forward with a documented reason. The register is additive — older entries stay
as the historical record, newest at the bottom.

---

## 2026-08-02 — v1.11.0 "Associate" pre-release audit (G1–G8)

Source: a post-v1.10.0 audit of the write-path AuthZ surface + dependency
comments + config hygiene, performed before the v1.11.0 HippoRAG release.
Research map at the bottom of this entry.

### Findings + dispositions

| # | Finding | Severity | Disposition |
|---|---|---|---|
| G1 | `authorize()` was never called in production code (v1.2.0 wired the AuthZ surface but no handler invoked it) | High | **Closed this session** — wired into every write-path handler (see below) |
| G2 | `Principal::is_superuser()` treated empty scopes as superuser; an authenticated token with zero grants silently got everything | Medium | **Closed this session** — empty scopes = deny-all; explicit superuser requires `admin:*/*` |
| G3 | (see sweep) — carried | — | Carried to v2.0 (see sweep table in `IMPLEMENTATION_PLAN_v1.11.0_HippoRAG.md`) |
| G4 | CORS no-wildcard-escape verification | Low | **Verified + hardened** — origins are exact-matched; `*` now stripped at the config choke point |
| G5 | Three stale dependency comments in `Cargo.toml` (rusqlite "Absolute Latest" claim, uuid "UUIDv7" claim, sqlite-vec) | Low | **Closed this session** — comments corrected, NO version bump (deliberate pin documented) |
| G6/G7 | (see sweep) — carried | — | Carried to v2.0 |
| G8 | model2vec single-source risk (boot-time HF fetch is the sole embedding source) | Low | **Closed this session** — `ponytail:` ceiling comment names the upgrade path |

### G1 wiring detail (the "all write routes" pass)

The v1.2.0 AuthZ gate existed but had zero production callers. Every handler
that mutates state or returns chunk content now calls
`handlers::authorize(&principal.0, Action::X, "", domain)?` at entry.
`principal` is `OptPrincipal` (an `Option<Principal>`); `None` = the v1.1
opaque-token / no-JWT back-compat path (superuser), so no existing install
changes behavior. Enforcement binds only when a scoped JWT principal is present.

| Route | Handler | Action | Domain scope |
|---|---|---|---|
| `POST /ingest` | `handlers::ingest::ingest` | Write | request `domain` or `global` |
| `DELETE /memory/{id}` | `handlers::forget::forget` | Write | `global` |
| `POST /sources/reconcile` | `handlers::sources::reconcile` | Write | `global` |
| `DELETE /sources/{id}` | `handlers::sources::delete_source` | Write | `global` |
| `POST /consolidate/apply` | `handlers::consolidate::apply` | Write | `global` |
| `POST /consolidate/undo` | `handlers::consolidate::undo` | Write | `global` |
| `POST /procedure` | `handlers::procedure::create` | Write | request `domain` or `global` |
| `POST /classify` | `handlers::procedure::classify` | Read | `global` (stateless pure fn, uniform gating) |
| `POST /decision/{id}/evaluate` | `handlers::procedure::evaluate` | Read | `global` |
| `POST /suggest` | `handlers::suggest::suggest` | Read | request `domain` or `global` (returns chunk content — audit S1) |
| `POST /suggest/feedback` | `handlers::suggest::feedback` | Write | `global` |
| `POST /domains` | `handlers::domains::create_domain` | Write | the new domain |
| `DELETE /domains/{name}` | `handlers::domains::delete_domain` | Admin | the domain |
| `POST /domains/{name}/vacuum` | `handlers::domains::vacuum_domain` | Admin | the domain |
| `GET /domains/{name}/export` | `handlers::domains::export_domain` | Read | the domain |
| `POST /domains/{name}/import` | `handlers::domains::import_domain` | Admin | the domain |
| `POST /add` (legacy) | `add_chunk` | Write | `global` (legacy error shape, not HTTP 403) |
| `POST /ingest/memory` (legacy) | `ingest_memory` | Write | `global` (legacy error shape) |
| `POST /ingest/markdown` | `ingest_markdown` | Write | `global` (HTTP 403 via new `AppError::Forbidden`) |
| `POST /reindex` (legacy) | `reindex` | Write | `global` (legacy error shape) |
| `POST /quarantine/{id}/release` | `release_quarantine` | Admin | `global` (HTTP 403) |
| `POST /quarantine/{id}/delete` | `delete_quarantine` | Admin | `global` (HTTP 403) |

Notes:
- Modern handlers return a real HTTP 403 (`HandlerError::forbidden`). The three
  legacy `/add`-family handlers keep their `{success:false}` shape (HTTP 200
  with error body) to stay shape-compatible — same choice the capacity guard
  already makes — documented inline at each call site.
- `ingest_markdown` + quarantine routes return a real 403 via the new
  `AppError::Forbidden(String)` variant added to `src/main.rs`.
- Read routes that return content (`/suggest`, `/classify`, `/evaluate`,
  `/domains/{name}/export`) are gated with `Action::Read` so a read-only
  principal can use them without a write grant.

### G2 decision

Empty-scopes `Some(principal)` is now deny-all, NOT superuser. The `None`
principal (opaque-token/no-JWT back-compat) stays superuser in
`handlers::authorize`. Explicit superuser is the `*:*/*` scope (`admin:*/*`).
Updated `empty_scopes_principal_is_deny_all_not_superuser` pins both arms.

### G4 verification

The CORS layer (`build_app` in `src/main.rs`) exact-matches origin strings via
`AllowOrigin::predicate` — no wildcard is ever honored by the layer. The only
escape was a config foot-gun: `CORS_ORIGINS=*` silently matched nothing (a
deployer would think it was open when it was closed). `config::cors_origins()`
now strips the literal `*` at the single choke point; `sanitize_origins` is a
pure fn pinned by two tests.

### G5 correction

Three `Cargo.toml` comments corrected (no version bump — a rusqlite bump is a
behavior-affecting change, out of scope for a comment-cleanup release):
1. `# Database Stack - Verified Absolute Latest` → documents the deliberate
   pin at rusqlite 0.38.0 (locked) and sqlite-vec 0.1.6 (resolves 0.1.9).
2. `uuid` comment claimed UUIDv7 `jti` minting; the code uses
   `Uuid::new_v4()` — corrected.
3. sqlite-vec pinned-version note corrected to match the lockfile.

### G8 ponytail

`StaticModel::from_pretrained` at boot is the single source of truth for every
embedding. A transient HF outage and a model-repo takeover present the same
failure mode. `ponytail:` comment at the load site names the upgrade path:
vendor the weights at install time and load from a local path (air-gapped
Jetson already ships them separately).

### Research map

| Topic | Source | Date | What it grounded |
|---|---|---|---|
| Graphiti / Zep bi-temporal edges + `resolve_edge_contradictions` | context7 `/getzep/graphiti` | 2026-08-01 | v1.6 supersession semantics (valid-time vs wall-clock) |
| MemConflict / MOSAIC | roadmap §v1.6 | 2026-08-01 | manual-first conflict resolution (no auto-delete) |
| HippoRAG 2 PPR-over-KG | 2026-08 research (HippoRAG/PRP/IPR literature) | 2026-08-02 | v1.11.0 "Associate" third RRF leg |
| ColBERT / ColPali | 2026-08 survey | 2026-08-02 | recorded as future option, NOT scoped (model-load cost) |
| Matryoshka embeddings | 2026-08 survey | 2026-08-02 | recorded as future option (truncation trade-off) |
| Mem0 corpus + feedback analytics | context7 `/mem0ai/mem0` | 2026-08-02 | v1.9 suggest feedback metric shape |
| Letta / MemGPT anticipatory memory | context7 `/letta-ai/letta` | 2026-08-02 | v1.9 suggest is reviewable pull, never push |
| OWASP API Security Top 10 2026 | OWASP | 2026-08-02 | AuthZ wiring priority (G1), deny-by-default (G2) |

Carried-forward gaps (G3/G6/G7 and the v1.9.1 carry-forwards) are tracked in
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` and the v2.0.0 Cortex
milestone in `ROADMAP.md`.

---

## Register — 2026-08-23 independent security audit (v1.28.8 line)

Single open-items register for the later audit series (the ATLAS F- / S2- /
S3- / adversarial / MEMORY_STACK_REPORT entries are folded into CHANGELOG.md
per release; this table is the live closure view). Findings F-*: audit
`BRAIN_SECURITY_AUDIT_2026-08-23.md`; remediation per the 1.28.9–1.28.14
operator prompt.

| Finding | Theme | Status | Closure |
|---|---|---|---|
| F-I1 write gate not exclusive | Seatbelt (1.28.10) | closed | `BRAIN_WRITE_POSTURE=review` routes six agent writes through the proposal pipeline (`review_posture_routes_writes_to_proposals`) |
| F-R4 digest-less approve | Gateweld (1.28.9) | closed | `400 digest_required` (`review_digest_matches_gates_stale_approval`) |
| F-L5 mount attestation spoofable | Gateweld (1.28.9) | closed | server-verified vs boot manifest, 409 pre-write (`plugin_mount_evidence_is_audited_and_input_gated`) |
| F-M3 Rust fence welding forge | Boundary (1.28.11) | closed | `fence::wrap_fenced`, control chars before sentinel strip (`wrap_fenced_blocks_control_char_welding`, mcp + CLI pins) |
| F-I2 taint dropped at boundary | Boundary (1.28.11) | closed | recall hits serialize origin/flagged/authority; UMP records `untrusted:true`; export labeled verbatim (`recall_hit_serializes_provenance_taint_labels`) |
| F-L1–L3 LITL decision UI | Anchor (1.28.12) | closed | dock full-content scroll box, overview link-only, actions above content (`dock_renders_full_content_not_a_clamp`, `overview_queue_is_link_only_no_inline_decide`) |
| F-B1–B3 hollow boot chain | Anchor (1.28.12) | closed | symlink containment, Ed25519-signed manifest + `/app/boot.pub`, embedded fetch-and-refuse loader, digest-stamped SW, external SW registration (`symlink_escaping_dist_is_refused`, client/tests/boot.test.mjs) |
| F-S1 unpinned CI refs | Bedrock (1.28.13) | closed | all `uses:` SHA-pinned with version comments; least-privilege permissions |
| F-S2 rerank CWD-relative model dir | Bedrock (1.28.13) | closed | absolute-or-env only (`resolve_model_dir`); `scripts/gen-model-manifest.sh` + installer provisioning |
| F-W2 UMP key dir warn-only | Bedrock (1.28.13) | closed | fail-closed at startup |
| F-B4 headers missing on 401/429 | Bedrock (1.28.13) | closed | headers layer outermost (`security_headers_present_on_401_and_429`) |
| F-B4 context drawer unstripped | Bedrock (1.28.13) | closed | strip_invisible on drawer content |
| F-I3 residual unicode screen evasion | Bedrock (1.28.13) | closed (bounded) | added U+180E/115F/1160/FFF9–FFFB; matching-time fullwidth fold — general NFKC/homoglyph folding stays a documented ceiling (zero-dep rule) |
| F-D1/D2/D3 doc drift | Bedrock (1.28.13) | down payment | THREAT_MODEL ↔ OWASP_AGENTIC cross-link; this register is the single findings view; full truth pass tracked separately |
| F-W1 shared static token | partial | mitigated | installer provisions a second agent token under review posture; full workload identity stays v3.7 |
| F-E1/E2 openclaw UI egress, F-M2 host fingerprint, F-M4 requiresToolAuthority | deferred-upstream (openclaw) | deferred | host/UI findings; not fixable in this repo |

---

## 2026-08-25 — v1.28.28 "Channel" third-pass deep hardening audit

Adversarial pass over the case-scoped channel surface (`src/workflow/channel.rs`,
`src/handlers/channel.rs`, the `case/%` SSE drain, and the Channel DSAR arms),
performed before release/tag. Method: OWASP Top-10 for LLM Applications v2025
(LLM01–LLM10) as the review frame + the 2025–26 agent-memory-poisoning
literature (AgentPoison NeurIPS'24; MINJA arXiv:2503.03704; Memory Poisoning
Attack & Defense arXiv:2601.05504; ConfusedPilot arXiv:2408.04870; TMA-NM
non-malleable origin-bound memory authority; SMSR certified defense; MemAudit
post-hoc attribution). Every disposition below is grep- or test-verified on
the shipped tree.

### Input-channel threat model (OWASP LLM01 auditor artifact)

| Channel into the system | Defense (one function each) | Verified by |
|---|---|---|
| Human note content (POST /notes) | `channel::screen_content`: trim-empty → ≤4000 → prompt-injection blocklist → invisible-strip → markdown-ref strip; stored viewer-independent | `notes_are_screened_and_case_scoped_only` |
| Mention tokens (@skill:x / @name) | exact-match resolution against server-side tables only; dead OR over-vocabulary tokens refuse loudly with the list — never skipped, never echoed as resolvable | `mention_resolves_skill_to_principals`, `oversized_mention_tokens_report_dead_not_skipped` |
| Invitee ids at insert | identity validation INSIDE `insert_note` (fence holds of the FUNCTION, not call-site discipline); invalid ids refuse before any row | `insert_note_validates_invitee_identity_before_any_write` |
| Lineage event payloads (engine-facing bus) | structural content-freedom: no emit payload carries note text — ids + actors only, so poisoned prose cannot ride `/events` into any agent context | `note_content_never_rides_lineage_payloads` |
| SSE live drain + Last-Event-ID replay | `sanitize_stored` once at drain + per-subscriber run-domain Read gate, fail-closed; admission default-off behind `?kinds=workflow` | pre-existing Witness pins + `channel_notes_drain_to_the_sse_bus` |
| Channel view reads | read seam on every emitted string + retention hide before page split | handler + `notes_honour_retention_and_dsar_sweep` |

### Findings + dispositions

| # | Finding | OWASP map | Severity | Disposition |
|---|---|---|---|---|
| H1 | No per-run cap on channel rows — an authorized writer could flood a run with notes (each costing note + lineage event + audit rows), unbounded storage growth | LLM10 Unbounded Consumption | Medium | **Closed this pass** — `MAX_NOTES_PER_RUN = 1000` shared budget (notes + invites), refused in-tx before any write with `409 channel_full`; REFUSES rather than steering's drop-oldest because case rooms are evidence (`channel_full_refuses_at_the_ceiling`) |
| H2 | Over-vocabulary mention tokens (>32-char skill tag, >256-char name) were SILENTLY SKIPPED by the parser — the author believes a mention fired when it didn't | LLM01 (detection-control completeness) | Low | **Closed this pass** — over-long tokens flow through and resolve as dead, reported in `details.unresolved` like any dead token (`oversized_mention_tokens_report_dead_not_skipped`) |
| H3 | `insert_note` trusted invitee ids from the caller; a future caller bypassing resolution could store unvalidated identities (invisible-char collision class, the Relay addressee lesson) | LLM01/LFP class | Low | **Closed this pass** — identity validation inside the core fn, refusal precedes all writes (`insert_note_validates_invitee_identity_before_any_write`) |
| H4 | DSAR asymmetry: purge erased subject-authored/addressed notes but the Art-15 EXPORT bundle never disclosed them (and content-bearing notes were never swept) | GDPR Art 15/17 symmetry | Medium | **Closed this pass** — sweep gains the content `LIKE %subject%` arm (proposals-sweep posture); export bundle carries `channel_notes[]` selected by the SAME three arms the purge erases, built pre-sweep in-tx (`dsar_export_bundle_builder_matches_live_shape`, extended erasure pin) |
| H5 | Note content reaching an agent's context would be the AgentPoison/MINJA poison sink | LLM01/LLM04 | Info (structural) | **Verified structurally absent** — notes are workflow-lineage data, NOT knowledge-corpus rows; no retriever indexes them; engines consume steering/intake topics only; lineage payloads carry ids only (H4's pin holds the boundary for Mesh .29) |
| H6 | Mention spoofing via confusable/homograph unicode | IFC/spoofing | Info | **Verified closed by construction** — resolution is byte-exact against server-side tables; display-side invisible strip covers rendering; write-time screen strips invisibles so stored ids cannot smuggle fence markers |
| H7 | SQL interpolation in new surfaces | classic inj. | Info | **Verified clean** — zero `format!`-interpolated SQL in channel/handler/alert paths (grep); every predicate parameterized |
| H8 | Accept-invite does not verify acceptor == addressee | authz | Accepted ceiling | Carried deliberately (Relay delegation posture): tightening strands cross-shift accepts when tokens rotate; Write-on-domain is the trust boundary |
| H9 | Retention is read-time enforcement; no worker deletes expired notes | LLM10/lifecycle | Accepted ceiling | Consistent with the repo's no-background-worker law; physical deletion rides run-level erasure; documented ceiling unchanged |
| H10 | Single-sanitize SSE drain posture (no per-subscriber PII redaction on a shared broadcast) | LLM02 | Accepted ceiling | Mitigated structurally: drained payloads carry NO note content (H4 pin); write-time screen is the guarantee; documented since Witness |

### Research-grounded posture notes
- The literature's consensus defense against memory poisoning is layered:
  write-time screening (shipped: one blocklist function per channel),
  provenance/origin binding (shipped: hash-chained audit per mutation,
  actor+target, tamper-evident chain + head pin), HITL gates for anything
  decision-shaped (steering stays approve-gated; notes carry no engine
  authority), and post-hoc causal attribution (shipped: per-note audit target
  `note:{id}` reconstructs authorship from the chain — the MemAudit goal).
- TMA-NM's non-malleability ideal maps to the existing content_digest law
  (ReviewArmour) + audit head pin; no new machinery warranted this pass.
- SMSR's result ("no provenance-free retrieval-time filter certifies against
  adaptive injection") is why notes are fenced OUT of retrieval entirely
  rather than filtered INTO it.

---

## 2026-08-26 — v1.28.41 "Terrain" — the Conformance Line series close-out

Source: the series-exit gate (G8) of the v1.28.37→.41 Conformance Line — the
full dogfood cycle re-audit of `docs/CONTACT_CENTER_STANDARDS.md` before the
v1.29.x Console inherits.

### Disposition of the line

| Gate | Release | Disposition |
|---|---|---|
| G1 ISO 10002 complaint lifecycle | v1.28.37 Advocate | Closed — register = audit chain, ack sweep + monthly extract ride signed calibration |
| G2 normative metric dictionary | v1.28.38 Lexicon | Closed — docs↔JSON↔schema parity meta-tests |
| G3+G4 WCAG 2.2 AA gate + RTL/pseudolocale | v1.28.39 Access | Closed — six new AA criteria release-blocking; ceilings honest in the ACR |
| G5+G7 WFM seam + workload visibility | v1.28.40 Handshake | Closed — `wfm/1` versioned additive seam; fatigue alerts never reassign |
| G6 COPC R8.0 performance mapping | v1.28.38 Lexicon | Closed — COMPLIANCE.md §6.7 rows → metric dictionary |
| G8 tier guide tested + series exit | v1.28.41 Terrain | Closed this pass — profiles + tier-smoke CI + drift meta-test; matrix every row green or ceiling/watch-marked (`series_exit_gate_checklist_green_or_ceiling_marked`) |
| G9 PCI boundary row | closed pre-Terrain | THREAT_MODEL §6 explicit non-scope row verified present this pass |

### Findings from the exit audit itself

| # | Finding | Severity | Disposition |
|---|---|---|---|
| X1 | Matrix rows for KCS loop, SLA envelopes, RTL (G4), WFM seam (G5), workload (G7) still carried stale 🟡/⚠️ statuses despite shipping in .36–.40 | Doc-drift | **Closed this pass** — matrix re-audited to green-or-ceiling-marked; the new meta-test forbids regression |
| X2 | Tier guide existed as prose only; no checked-in profile could prove a tier boots | Medium | **Closed this pass** — `deploy/tiers/t{1..4}.env` + CI tier-smoke matrix + two meta-tests |
| X3 | ISO/AWI 18295-1 revision pending upstream | Watch item | Ceiling-marked in the matrix (G10); cannot land silently |

**Inheritance test:** nothing in v1.29.x may backfill an Order-of-Care row —
if it must, this line failed and this register says so.

---

## 2026-09-03 — v1.28.52 "Cornerstone" — the Foundation Line close-out report

Source: the line-exit audit of the Foundation Line (v1.28.46 "Plumb" →
v1.28.52 "Cornerstone"). The line's promise: handlers hold ZERO SQL, the
service layer owns storage, and the law is machine-checked — "the repo has
ONE pattern, CI-enforced." This entry records the evidence, per the line's
executor contract.

### AMENDMENT (declared up front)

The Cornerstone executor prompt assumed v1.28.51 shipped an EMPTY allowlist.
It did not: `gate.rs` (the HITL proposal engine, 78 statements = 50 prod + 28
test) was Confluence's declared straggler. Per the prompt's own
"Deviations = STOP + amendment" rule the executor stopped; the operator chose
the AGENTS.md-prescribed path (option A): the final-vein extraction ran
INSIDE v1.28.52 as its opening act, then the flip proceeded exactly as
written. The extraction honored the line discipline — one surface per
commit, full gate per commit, baseline row lowered in the same commit.

### The final vein: gate.rs → `service::gate` (six commits)

| Commit | Surface | Floor |
|---|---|---|
| 1 | review-queue read (`ProposalView`, deadline/SLA, page SELECT pair, owner filter; 3 read pins ride) | 78 → 68 |
| 2 | creation insert (`NewProposal` + pending audit) + conflict pre-check | 68 → 66 |
| 3 | expire/reject (TTL write with wall-clock-as-arg; pending-fence read ONE-DEFINED across approve/reject/edit; reject CAS; content read) | 66 → 59 |
| 4 | edit path (8-col row read + re-score CAS) | 59 → 57 |
| 5 | approve family (pending-row read; decision CAS one-defined across SIX branches; article-state CAS typed so `public_slug_taken` keeps its 409; translation CAS with its verbatim `datetime('now')` quirk pinned+filed; KCS draft insert; vec shadow one-defined across both promote paths; case-article link; supersession link-follow; promote insert; the two promote-provenance pins moved onto the core driving the REAL insert) | 57 → 21 |
| 6 | export read (`export_bundle` = count pre-flight + four datasets; export/migration/pii_map pins ride; comment/identifier residue reworded to zero) | 21 → 0 |

### Scope 1 — the enforcing flip

`SQL_BASELINE` (29 rows), the floor pin, and the substring-absorption
machinery are DELETED — nothing is left to compare against.
`no_sql_in_handlers_enforced` walks `src/handlers/` RECURSIVELY and fails on
ANY counted statement — production, test fixture, or comment residue (the
substring counter is deliberately strict; the false-positive class the old
baseline absorbed now has nowhere to hide, so drained files are reworded
clean). Two anti-vacuity teeth: a ≥30-file sanity on the walk (the lipstyk
lesson — a guard that scans nothing must not smile) and the
`sql_statement_counter_still_fires` self-pin proving the counter still
detects all four statement openers, comment residue included, with a
negative control.

### Scope 2 — the layer grep

`service_layer_free_of_http_types` (renamed from
`service_layer_is_transport_free` at the flip) forbids `axum`, `StatusCode`,
`Json`, `AppState`, `Pool` in production source under `src/service/`. It was
born a hard error at the Plumb pin — there was never a warning phase — so the
prompt's "flip" is declarative: the name now matches the line plan, and both
guards ride CI through the `lint-test` job's `cargo test` steps (default +
bench), alongside the inventory guard.

### Pin + test counts across the line

| Release | Service-tree pins | Suite (bench) |
|---|---|---|
| v1.28.45 (baseline) | 0 (the layer did not exist) | 1268 passed / 7 ignored |
| v1.28.46 Plumb | 9 | — |
| v1.28.47 Quarry | 27 | — |
| v1.28.48 Masonry | 41 | — |
| v1.28.49 Terrace | 58 | — |
| v1.28.50 Aqueduct | 76 | — |
| v1.28.51 Confluence | 80 | 1316 passed / 7 ignored |
| v1.28.52 HEAD | **89** | **1308 passed / 7 ignored** |

HEAD arithmetic: 80 − 2 (baseline + floor deleted) + 2 (enforcing guard +
self-pin) + 9 (gate.rs) = 89. The suite count moved 1268 → 1308 over the
line and 1316 → 1308 across Cornerstone itself: the −8 is the drained
handler-side test region (queue-read, export, and mirror pins moved onto the
core where several were merged into REAL-path pins instead of re-stating
column lists) and the deleted freeze machinery, against the +2 flip pins and
the moved tests. Every milestone's pins still pass at HEAD (full suite green,
0 failed). Count ≥ v1.28.45 baseline: YES (1268 → 1308).

### Eval-floor history (v1.28.50 "Aqueduct")

The line's only retrieval-adjacent release gated EVERY extraction commit on
the frozen 25-doc corpus (fresh scratch instance, CI recipe):
pre-move baseline r@5 0.976 / r@10 0.991 / MRR 0.956; after the recall core
commit identical; after the ingest core commit identical — byte-identical
means AND per-query ranks on all 106 judged queries; floors (0.85) green at
every gate. The honest scope: this proves behavior preservation on the
frozen set, NOT external-engine parity (LongMemEval stays pending).
Confluence and Cornerstone touch no retrieval path and re-ran no eval gate.

### Smoke matrix per phase

| Phase | Live smoke (DB copy, release binary) |
|---|---|
| Plumb | old-vs-new smoke on identical copies (retention family) |
| Quarry | shim-mode copy: owned root + derived surface seeded; held row deferred with reasons |
| Masonry | two servers, one seeded copy, v1.28.46 vs then-current — lifecycle families only |
| Terrace | multi-db copy: client register flows, hold fence |
| Aqueduct | multi-db copy: 3-leg recall, trace replay, include_flagged posture, screened + quarantined ingest, dedup, /audit/verify throughout |
| Confluence | procedure evaluate, UMP ops read (integrity-verified), kcs worklist, forget (tombstone carries digest), suggest + feedback, Art.30 register read, webhook HMAC path (401s), /audit/verify throughout |
| Cornerstone | this release's smoke — see the Gates row below (gate-family flows on a DB copy) |

### Wire + schema identity (the line's core proof)

- **Routes:** the registered route set is BIT-IDENTICAL v1.28.45 → HEAD
  (147 `.route(` registrations, sorted-diff empty).
- **Route-authz gate table:** the `authz_gates_cover_every_non_public_route`
  table is md5-identical across the line (201 rows); the pin bodies of both
  wire guards (`authz_gates_cover_every_non_public_route`,
  `test_openapi_covers_routes`) are md5-identical — the contract tables were
  not touched to make a move pass.
- **openapi.yaml:** ONE line differs from the v1.28.45 baseline —
  `POST /ingest/proposal` `content.maxLength` 2000 → 10000, shipped in
  Confluence commit b8cb52c together with the matching server bound
  (`MAX_PROPOSAL_CONTENT = 10_000` replacing the borrowed `MAX_QUERY = 2000`
  in the propose/edit paths). FINDING: that release's "openapi.yaml
  diff-empty" claim is TRUE for routes and FALSE for this bound; the edit
  honored wire-contract discipline (contract + code in the same commit, the
  openapi-coverage test green) but was not declared in the release notes.
  DISPOSITION: declared here; the bound stays (widening is caller-visible
  but non-breaking, and reverting would break shipped callers); a
  `docs_truth`-style parity pin on the proposal bound is the follow-up.
  Every OTHER line release (46→47→48→49→50 and 51→52) is openapi diff-empty.
- **Schema:** `schema_meta.schema_version` still stamps **1.28.45** —
  untouched across all seven releases (no migration landed in the line; the
  line is storage-RELOCATION, not storage-CHANGE).

### Cornerstone gates (this release)

fmt clean; clippy `--all-targets --features bench -D warnings` green; full
suite 1308 passed / 7 ignored at HEAD (green at every one of the seven
commits); enforcing guard + self-pin + renamed layer pin green; mdbook build
green with the new architecture sections.

### Ceilings (honest)

- The compliance-pack TEST RUN owed from Confluence is STILL owed before
  push (clippy green; the one-time full rebuild is the cost).
- The translation CAS's `decided_at = datetime('now')` (SQL-side clock,
  inconsistent with every other branch's bound parameter) is preserved
  VERBATIM and needs a pin or fix — filed, not changed in the move.
- The maxLength parity pin (above) is a follow-up.
- The line proves pattern singularity, not schema evolution readiness: the
  storage-adapter deadline trigger (pre-v2.x) is the next forcing function.

## 2026-09-05 — v1.28.57 "Capstone" — the Spire Line close-out report

The Spire Line (v1.28.54 "Scaffold" → v1.28.55 "Buttress" → v1.28.56
"Vaulting" → v1.28.57 "Capstone") set out to dismantle the 19,906-line
main.rs without changing a byte of behavior, and to make the end state
IMPOSSIBLE TO UNDO QUIETLY. This report is the line's measured
before/after, re-measured at the tip with `wc`/`grep` — not from memory.

### The before/after table

| Measure (needle, measured the same way every time) | Scaffold open (freeze) | Buttress close | Vaulting close | **Capstone close (this audit)** |
|---|---|---|---|---|
| `wc -l src/main.rs` | 19,906 | 18,291 | 12,471 | **124** |
| test region (lines from `#[cfg(test)] mod tests` to EOF) | 13,342 | 12,302 | 12,294 | **absent** (absence-pinned) |
| route-registration sites in main.rs | 234 | 234 | 35 (test stubs) | **0** (pinned) |
| route-registration sites under src/server/router/** | — (n/a) | — (n/a) | 199 (floor gained) | **199** (floor held) |
| crate `#[test]` needle | 1,178 (src) | 1,185 (src) | 1,185 (src) | **1,198 = 1,076 src + 122 tests** (floor 1,196 over the widened subject) |
| guard-table rows (coverage / authz) | 151 / 141 | 161 / 145 | 161 / 145 | **161 / 145** (floored) |
| schema version | 1.28.45 | 1.28.45 | 1.28.45 | **1.28.45** (untouched across all 13 releases) |
| wire artifacts | diff-empty | diff-empty | diff-empty | **openapi.yaml diff-empty vs v1.28.56**; x-api-version moves with the release stamp |

Per-milestone deltas (net main.rs lines): Scaffold −624, Buttress −1,191,
Vaulting −5,820, Capstone −12,347. Nothing deleted: every test that ever
lived in main.rs lives in the tree today — relocated, never removed.

### What moved where (the module map)

- **Scaffold (1.28.54):** the ledger (`src/spire_inventory.rs`) + the
  route tables (`src/route_guards.rs`, born from arrays at main.rs ~L12k)
  + ten pure-unit pin families relocated verbatim to their subjects.
- **Buttress (1.28.55):** the pre-main library code stops pretending to
  be an entrypoint — `src/http_limit.rs` (RateLimiter, ConnectionTracker
  + RAII, connection/RSS watchdogs), the layer-1 blocklist + quarantine
  read-seam (`src/screen.rs`), the graph read mappers
  (`src/graph_read.rs`), the boot guards (`src/boot.rs`, folded into
  bootstrap at Vaulting) — each fn moved with its pins, ledger lowered
  same-commit.
- **Vaulting (1.28.56):** the monolith becomes the thin bin — middleware
  stack + auth middlewares → `src/server/router/{mod,auth}.rs`; `app(state)`
  → `src/server/router/mod.rs` as a pure function of `AppState`; the whole
  boot region → `src/server/bootstrap.rs` (protocol-free); six family
  builders (core 17 / memory 56+3 legacy+1 GiB import / ump 12 / compliance
  10+5 gated / workflow 82 / auth 9); THE LIB FLIP (the server tree behind
  `lib.rs`, main.rs consumes `brain_server::server::…`); the law-9 authz
  matrix → `tests/authz_matrix.rs` driving the lib from OUTSIDE the crate;
  law-13 contention gauges on /metrics + /health.
- **Capstone (1.28.57):** the test mass (12,294 lines, 109 plain + 60
  tokio fns) → `tests/main_suite.rs` verbatim (include_str anchors
  re-pointed CARGO_MANIFEST_DIR-absolute; the root use-block traveled with
  it so `use super::*` resolves exactly as before); `route_guards.rs`
  re-homed to `src/server/router/` (100% rename, content unchanged);
  `spire_inventory.rs` stays beside main.rs — its subject.

### The enforcement map (which gate guards which law)

| Law | Enforcing test | Home |
|---|---|---|
| routes register ONLY under src/server/router/** | `route_registrations_live_only_under_router` (hard gate; red-proofed against a planted registration in src/config.rs; mcp.rs fenced at exactly 1 site) | `src/spire_inventory.rs` |
| server::bootstrap stays protocol-free | `bootstrap_stays_protocol_free` (hard gate; word-boundary needles; red-proofed against a planted axum type in bootstrap.rs) | `src/spire_inventory.rs` |
| main.rs is wiring-only: ≤ 300 lines, no cfg(test) region | `spire_inventory_freezes_the_thin_binary` (MAIN_RS_LINES_MAX = 300 + the region-absence pin) | `src/spire_inventory.rs` |
| the crate's test mass never shrinks | `CRATE_TEST_FLOOR` over src/ + tests/ (1,196, never decreases) | `src/spire_inventory.rs` |
| the router's registrations never silently disappear | `ROUTER_SITES_FLOOR` (199) | `src/spire_inventory.rs` |
| the wire tables never shrink without their wire change | `OPENAPI_ROUTE_ROWS_FLOOR` (161) + `AUTHZ_TABLE_ROWS_FLOOR` (145) | `src/spire_inventory.rs` |
| every AUTHZ_GATES row × principal class through the composed app | the law-9 matrix | `tests/authz_matrix.rs` |
| zero SQL in handlers | `no_sql_in_handlers_enforced` (the Foundation flip) | `src/service/mod.rs` |
| read seam + wire-contract + docs truth | docs_truth + the route-coverage/authz pins + lipstyk (CI, diff-strict) | lib + CI |

Every scanner is self-pinned inline (the Cornerstone lesson: a counter
that cannot fire guards nothing) — each gate proves, inside its own test,
that it counts a planted violation string in a comment and stays quiet on
clean source.

### Capstone gates + validation

Two grep gates born hard (no warning phase, the Foundation precedent),
each red-proofed against a planted violation BEFORE its green commit:
the route gate caught a planted registration comment in src/config.rs
naming the file; the protocol gate reported `[axum::, Router]` on a
planted axum comment in bootstrap.rs. Both plants reverted. En route the
route gate flagged its own doc comment carrying the needle literal —
rewritten; the gate polices even its documentation.

Full suite 1,265 passed / 7 ignored (--features bench) at the tip, green
at every commit; clippy `-D warnings` (bench) clean; fmt clean; CI
dry-run green (default lint+test, engine-crates, steward-harness, otel
lint+test); lipstyk diff-strict green vs the v1.28.56 tip; live smoke on
the COPY instance green (/health, /audit/verify ok, the 413 + 408 paths,
one ingest → recall round-trip).

### Ceilings (honest)

- `src/bin/mcp.rs` keeps its own router: the MCP binary is a separate
  protocol edge, not the server's composition. The carve-out is fenced
  (exactly one site) and recorded here; folding it under
  src/server/router/** would be a behavior-adjacent refactor the line's
  no-behavior-change rule forbids.
- `tests/main_suite.rs` is one ~12k-line file: the mass moved as ONE
  verbatim block (exact-text relocation, zero churn in the pins);
  splitting it per-subject is churn without a forcing function.
- The ≤ 300 pin is a pin, not a proof of minimalism: main.rs could grow
  to 299 lines of wiring noise and pass. The gate that matters is the
  route gate — registrations cannot come back.
- The Capstone ledger numbers (124 lines, 1,198 pins) drift by
  doc-comment literals under the substring needles — the needles are
  measured identically every time; that is what a freeze needs.
