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
