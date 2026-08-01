# Changelog — brain-server

All notable changes are documented here. The format is a simplified keep-a-changelog
style. Version numbers follow `Cargo.toml`; "released" means the binary and docs
are consistent at that tag.

Honesty note: retrieval-quality claims below describe *what the code does*, not
measured parity against external engines (e.g. QMD). Where a benchmark has not
been run, it is marked **pending** rather than asserted.

---

## [Unreleased]

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
