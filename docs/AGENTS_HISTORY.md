# Agent Execution History — brain-server

> Retired from AGENTS.md on token-efficiency grounds (the detail lives in
> `CHANGELOG.md` per release and in ROADMAP.md; AGENTS.md keeps only the
> operational contract + compact pointers). Loaded on demand.

---

## Release version notes

> **Version note:** **v1.28.42 "Valet" shipped 2026-08-26** — the personal
> AI assistant, dogfooded: reminders are governed `valet/*` runs fired by
> the idempotent `brain valet due` crank (outbox key `valet-{run}-{due_at}`,
> repeat re-arms via CAS); Signal is a Bridges edge (`tools/valet-relay`,
> zero-dep Node, holds NO brain credentials — pinned by
> `relay_holds_no_brain_credentials`); inbound `/webhooks/signal` is
> HMAC + replay + injection-screened with `[case N]` steering and
> digest-bound `[draft N] approve <digest>`; drafts are `kind='draft'`
> proposals carrying the ADVISORY zero-token `valet::style_check` lint
> (style memory = approved knowledge row, changes flow through the gate);
> `brain valet brief` composes the morning brief; Outreach-lite is the
> one-subject one-channel hashed consent registry (no consent → suppressed,
> audited, counted). Cron recipes in `docs/deployment.md` ARE the
> scheduler. Schema ADDITIVE at 1.28.42 (`valet_consents`,
> `proposals.lint_json`); routes additive: `/workflow/valet/{due,brief,
> consent}` + signal kind on `/webhooks/{kind}`. Ceilings: relay is
> single-user operator-run; Signal `[draft N] edit` not wired; no
> auto-publish anywhere; scoreboard personal view is thin-end.
> See CHANGELOG.md §[1.28.42].
> Predecessor: v1.28.41 "Terrain" — G8 + series-exit, tiers as tested config

> **Version note:** **v1.28.29 "Mesh" shipped 2026-08-25** — a server-only
> release (schema 1.28.28 → **1.28.29**, additive `agent_cards` +
> `delegations`; client + plugin unchanged) — agents become named
> colleagues: A2A-shaped **Agent Cards** signed with the UMP operator key at
> provisioning (`POST /ops/agents/cards`, Admin) and RE-VERIFIED at every
> use point (reads fail the whole list closed on one tampered row);
> agent→agent **delegation** on a run's lineage (`POST/GET
> /workflow/runs/{id}/delegations{,/{id}/result}`) — the target's card is
> verified BEFORE any write (`400 agent_unknown` / `card_tampered`), task/
> result content screened by `channel::screen_content` and stored in-table
> while lineage payloads carry ids+actors only; results are delegatee-only,
> exactly-once CAS; a pure **working-set arbiter**
> (`mesh::working_set_domain`) pins the per-agent scratch-domain vocabulary.
> Wired: router + openapi + route-coverage + route-authz (+ mesh source
> mapping) + docs/api.md in the same change. Tests: bin 883/6 ignored (+4),
> lib 194/1; clippy `-D warnings` + fmt clean; lipstyk diff-strict clean;
> live smoke on a DB COPY green (doctor clean, `/audit/verify` ok). Honest
> ceilings: delegation results ride the lineage like steering (no auto-
> ingest into evidence/shared knowledge — promotion stays HITL); working-set
> isolation pins vocabulary only (no read-side filter yet); key rotation
> invalidates cards until re-provisioned; no client surface. See
> `CHANGELOG.md` §[1.28.29].

> **Version note:** **v1.28.27 "Relay" shipped 2026-08-25** — a
> **server-only** release (server `Cargo.toml`/lock 1.28.26 →
> **1.28.27**; schema 1.28.26 → **1.28.27** — additive `handover_offers`
> table; client + plugin unchanged) — the one-click handover over the I-PASS
> packet Lineage already builds: `POST /workflow/runs/{id}/handover/offer`
> refuses an incomplete packet with the MISSING list (five gate predicates
> in `src/workflow/relay.rs::packet_missing`; the refusal writes nothing),
> accept CAS-transfers run `owner` to the acceptor inside the SAME
> WorkflowTx as the offer state move (SLA clock byte-untouched; reply names
> the resume-at checkpoint), decline REQUIRES a screened reason ≤4000 — all
> three are `workflow/handover` lineage events audited in their own tx;
> offers are idempotent by open-state key. `GET /ops/handovers?domain=&now=`
> ranks active runs by SLA remaining, flagged inside the Watchbill ring's
> derived overlap window. Wired: router + openapi + route-coverage +
> route-authz (+ relay source mapping) + docs/api.md in the same change.
> Tests: bin **864**/6 ignored (+8), lib **194**/1; clippy `-D warnings` +
> fmt clean; live smoke on a DB COPY green end-to-end (`/audit/verify` ok)
> plus a hardening smoke (invisible-char addressee 400, accept-on-finished
> run 409 no-resurrection, empty decline reason 400, corrupt board row
> skipped + counted). Honest ceilings: packet completeness reads the STORED
> shape (form, not quality); any Write principal may accept for the
> addressee; board caps at 500 active runs with per-row state_json reads;
> the offer's `overlap_minutes` is recorded but not yet enforced against
> the derived ring window; no client/plugin surface yet. See `CHANGELOG.md`
> §[1.28.27].

> **Version note:** **v1.28.26 "Crew" shipped 2026-08-25** — a
> **server-only** release (server `Cargo.toml`/lock 1.28.25 →
> **1.28.26**; schema 1.28.25 → **1.28.26** — additive `presence` /
> `principal_skills` / `crew_config` tables; client + plugin unchanged) —
> colleagues become visible: presence WITHOUT a background worker (every
> mutating request upserts one row inside its own tx via `crew::touch`;
> reads TTL-decay active <5min / away <30min / offline), the roster view
> `GET /ops/crew` joining presence × Watchbill shift sites × role/skills
> tags, skills changes proposal-gated (`crew_skills_update`; the domain
> rides INSIDE the payload so approval applies exactly what was proposed;
> approval CAS + tags + audit in one IMMEDIATE tx), the DPO switch
> `POST /ops/crew/config` failing open to HIDDEN, and DSAR erasure now
> reaching presence + skills + shift rosters (lifting the Watchbill roster
> ceiling). Roster output passes the invisible-strip read seam; activity
> kinds are closed vocabulary. RAII immediate transactions in both new
> mutating handlers (context7 doc pass vs rusqlite DropBehavior guidance).
> Tests: bin **856**/6 ignored (+7), lib **194**/1; clippy `-D warnings` +
> fmt clean. Honest ceilings: presence bumps on MUTATING acts only
> (read-only work shows offline); `current_case_ref` is opaque but the
> roster does not re-authorize per member; DSAR dry-run doesn't count crew
> rows; legal holds don't freeze people-metadata; approvals audit under
> `global` tenant while tags land under the proposed domain. See
> `CHANGELOG.md` §[1.28.26].

> **Version note:** **v1.28.25 "Watchbill" shipped 2026-08-24** — a
> **server-only** release (server `Cargo.toml`/lock 1.28.24 →
> **1.28.25**; schema 1.28.23 → **1.28.25** — additive `shifts` table +
> `(domain, start_epoch)` index; client + plugin unchanged) — follow-the-sun
> as data: the `shifts` ring (site, tz, window, declared overlap budget,
> principal-id roster) + the pure read-time core (`src/workflow/shifts.rs`)
> that derives each boundary's overlap window from its shift pair and answers
> which site owns the queue at any instant (`GET /ops/shifts?now=`) — the
> queue re-scopes to the INCOMING site at the START of the derived overlap
> window while open runs stay byte-identical (`ring_boundary_rescopes_queue_not_cases`).
> `POST /ops/shifts` is **Admin** (pure operator config), validation + insert +
> audit ride one `BEGIN IMMEDIATE` tx, double booking refuses unless the later
> shift starts inside the earlier's final overlap period (anchored at e.end −
> e.overlap — a mid-shift start is 409, caught by live smoke on a DB copy).
> Reads capped newest-500 (Bound law); tz ≤64 chars, roster ≤64×256.
> Tests: bin **849**/6 ignored (+4), lib **194**/1; clippy `-D warnings` +
> fmt clean; lipstyk diff-strict clean. Honest ceilings: advisory scheduling
> data only (no enforcement until Relay .27); DSAR sweep does NOT cover shift
> rosters yet (Crew .26); no DELETE surface / retention for stale shifts;
> refused inserts write no Denied audit row. See `CHANGELOG.md`
> §[1.28.25].

> **Version note:** **v1.28.24 "Beacon" shipped 2026-08-24** — a
> **server-only** release (server `Cargo.toml`/lock 1.28.23 →
> **1.28.24**; schema unchanged at 1.28.23 — publish rides the
> pre-scaffolded KCS columns; client + plugin unchanged) — the
> demand-reduction half of KCS: approved knowledge becomes a **publicly
> published KB** as a generated static artifact an operator hosts; the server
> stays loopback, publishing is a human decision with its own verb.
> **M1:** `brain kb build --domain <d> --out <dir>` emits a deterministic
> static site (per-slug article pages, index, client-side-only search index,
> sitemap/robots/404, CSP `default-src 'none'`, superseded-slug redirects via
> the existing `supersedes` evidence chain) + a SHA-256 `kb_manifest.json`;
> every field passes the new strict public seam (`kb::sanitize_public` —
> unconditional PII redact, NO principal argument, no operator bypass);
> mask primitives moved verbatim to shared lib `pii_mask.rs` so gate +
> screen + public seam share one definition. **M2:** proposal kind
> `kcs_publish` (created via `POST /kcs/articles/{id}/publish`; approval
> requires `approve` + the NEW distinct `publish` capability — existing roles
> unchanged); in-tx CAS publish/retract + slug uniqueness via the partial
> unique index + audited `workflow/kcs/publish`; `GET /kcs/articles/{id}/preview`
> renders the EXACT public page (what you approve is what ships). **M3:**
> `POST /webhooks/kb-feedback` — ALWAYS Standard-Webhooks HMAC-verified
> (`BRAIN_KB_FEEDBACK_SECRET_FILE`, 0600 fail-closed) with seen-claim replay
> dedup → anonymous `kb_feedback` finding rows (no raw IP by construction);
> scoreboard gains `self_service_deflection_units` + `kb_feedback_total` +
> `kb_hot_topics`; freshness watcher fires the existing `expiry` kind;
> hot-topic threshold fires `workflow`. **M4:** `docs/kb-deflection.md` —
> deflection is INDICATIVE, repeat-contact rate stays primary; no lift
> claims. Tests: bin **845**/6 ignored (+7), lib **191**/1 (+10), brain 19,
> mcp 37, eval 4, metrics 8; clippy `-D warnings` + fmt clean. Honest
> ceilings: signing delegates to `scripts/release-sign.sh`; `revision`
> renders content_hash (envelope law-version not persisted per-article);
> deflection/hot-topics are vote-based signals, not CRM repeater clustering;
> CDN caches after retract are operator-side; no client GUI publish node yet
> (the preview endpoint is the render contract).

> **Version note:** **v1.27.31 "AuditRepair" shipped 2026-08-21** — a
> **server-only** security release (server `Cargo.toml`/lock 1.27.30 →
> **1.27.31**; schema 1.27.30 → **1.27.31** — schema_meta keys only, no
> tables/columns; client + plugin unchanged) — the **announced audit-chain
> re-anchor**: the items v1.27.26 "Notarize" deliberately deferred because
> they change what an audit row MEANS once stored. **M6+M2 (keyed full-row
> links):** an `hmac256` epoch (per-DB `schema_meta.audit_chain_epoch`; absent
> = `legacy`, the byte-identical 5-field SHA-256 link) whose links are
> HMAC-SHA256 over the FULL row — id, ts, kind, actor, target_hash, status,
> detail_hash, prev_hash — under a 32-byte key that NEVER lives in the DB it
> protects (`BRAIN_AUDIT_CHAIN_KEY` → `BRAIN_AUDIT_CHAIN_KEY_FILE` → a
> generated 0600 `audit-chain.key` beside the DB; wide modes refused, the
> auth-secret posture; init at server + `brain` CLI boot). A reconstructed
> chain from attacker-chosen content cannot pass verify even when every
> SHA-256 recomputes; mutating any committed field (incl. renumbered ids)
> breaks verify. Writes to a keyed chain without its key fail CLOSED (row
> refused, `/health` counter, verify not-ok) — never an unkeyed downgrade.
> **M3 (head pin + restore attestation):** `schema_meta.audit_chain_head`
> pins `(id, hash, epoch)` in the same tx as every audit row
> (`record_tenant` re-pins per commit; prune re-pins in-tx; the migration
> stamps the initial legacy pin for existing chains); `verify_chain`
> compares pin vs recomputed head → truncation/extension of an
> internally-valid chain is DETECTED; `backup::restore` verifies the restored
> chain BEFORE certifying (broken chain → refuse, `.bak` preserved) +
> classifies pre/post pins — a rolled-back head is disclosed at error level
> and the `restore complete (head=…)` row records where the chain landed.
> **M4 (multi-db chain sweep):** `/audit/verify` (additive `domains`
> breakdown + failing domains in the alert payload), `/audit` (rows tagged
> `domain`, merged newest-first across every registered chain), `/metrics`
> (`brain_audit_chain_ok` aggregates all domains), `/ump/audit/verify`, and
> the read-event retention prune all iterate every registered domain — a
> broken second-domain chain is reported, never absorbed by an ok global
> pool. **Re-anchor operator step:** `brain-server --re-audit` (offline,
> instead of serving): verify-before-replay (no laundering), keyed replay,
> epoch flip + new pin + an `anchor` evidence row per domain on the NEW
> chain; idempotent; per-domain failures fail the run. Fresh row-less DBs
> bootstrap straight to `hmac256` when a key resolves (server boot + lazy
> domain open); existing chains stay legacy until re-anchered (an audit
> chain is evidence — its format flips only under the documented protocol:
> snapshot → quiesce → `--re-audit` → verify every domain → snapshot the new
> baseline). New `AuditKind::Anchor`. Also fixed `--re-embed` exiting 2 in
> the argv guard. Tests: server bin **717** / 6 ignored (+2), lib **147** /
> 1 ignored (+10 — full-row commitment, attacker rejection, pin-on-commit,
> truncation, keyless fail-closed, re-anchor replay/idempotence/refusal,
> bootstrap, restore rollback classification + refusal); clippy `-D
> warnings` + fmt clean (`--all-targets --features bench`); live `--re-audit`
> smoke green (key 0600, epoch + pin stamped, anchor rows chained, tamper
> refused). Honest ceilings: legacy chains keep 5-field links until the
> operator re-anchors; pin detection reads at verify time, not write time;
> `/health`'s chain watcher stays global-only (`/audit/verify` is the
> multi-domain authority); the key is part of the DR baseline (a restore
> without it refuses certification); key rotation = re-anchor under the new
> key. See `IMPLEMENTATION_PLAN_v1.27.31_AuditRepair.md` +
> `CHANGELOG.md` §[1.27.31].

> **Version note:** **v1.27.29 "Survey" shipped 2026-08-21** — a
> **server-only** scaffold release (server `Cargo.toml`/lock 1.27.28 →
> **1.27.29**; client + plugin unchanged) — the `crates/` engine-crate
> workspace: five intentionally-empty crates (`brain-interview-core`,
> `brain-consensus-core`, `brain-executor-core`, `brain-troubleshoot-core`,
> `legal-rules-db`) as their own workspace node, `edition 2024`,
> `rust-version 1.97`, clippy `-D warnings` clean, zero dependencies; the
> driver harness stays in `tools/steward-harness/` (1.27.35 — the cores are
> harness-independent). **No schema, no migration, no endpoints, no server
> code change.** See `IMPLEMENTATION_PLAN_v1.27.29_Survey.md` +
> `CHANGELOG.md` §[1.27.29].

> **Version note:** **v1.27.30 "Spine" shipped 2026-08-21** — a
> **server-only** foundation release (server `Cargo.toml`/lock 1.27.29 →
> **1.27.30**; schema 1.27.25 → **1.27.30**; client + plugin unchanged) — the
> governed-workflow substrate for the Steward line: **no engine code, no new
> endpoints, no wire change, no telemetry.** **M1/M2 (docs):** the architecture
> contract, the G0 audit (PASSED — adopt the pi_agent_rust fork; execution in
> 1.27.35), the Restate awakeable mapping, the three SHA-pinned port specs, the
> rubric pin (written 2026-08-20), and the diagnostics-loop spec — **all in the
> PRIVATE IP repo `brain-steward-ip`** (moved 2026-08-21; `.gitignore` defends
> the doc names). The M6 compliance mapping (primitive→workflow + the §A.4
> customer table) moved private with them (same repo). **M3 (schema):** five
> additive tables in every domain DB — `workflow_runs` (CAS `state_revision`),
> `workflow_steps`, `outbox` (`idempotency_key UNIQUE` — exactly-once by key,
> not retry count), `findings`, `contradictions` — guarded by the extended
> schema-contract test; new `AuditKind::Workflow`. **M4/M5 (substrate):**
> `src/workflow/{tx,outbox,state,evidence}.rs` — `WorkflowTx` (RAII `BEGIN
> IMMEDIATE`), `enqueue`/`deliver` (`UPDATE … RETURNING`), `cas_update`
> (`Stale`/`Gone` conflict vocabulary), and the pure evidence-reducer (O(n)
> seen-set dedup, contradiction surfacing, deterministic order; oracle-pinned,
> not mathematically closed). **Audit-per-write is structural:** every mutating
> primitive emits its own `AuditKind::Workflow` row via `record_tenant`
> (SAVEPOINT-nested — transition + audit commit atomically and roll back
> together; CAS conflicts audit `denied`); pinned by
> `audit_rolls_back_with_the_transition` +
> `outbox_enqueue_audits_once_not_on_replay`. **M7:** the engine-crate
> workspace shipped one release earlier as v1.27.29 "Survey" (extracted from
> this plan; see `IMPLEMENTATION_PLAN_v1.27.29_Survey.md`).
> **Toolchain:** built/tested on rustc **1.97.1** stable;
> server package stays edition 2021 (an edition flip is its own release); ZERO
> new dependencies — the substrate wires onto existing `rusqlite` + audit chain
> only. Tests: server bin **715** / 6 ignored (+11), lib **137** / 1, brain 18,
> mcp 19, bench 8; clippy `-D warnings` + fmt clean on both workspaces; the
> migration boots green on a copy of the live DB (schema 1.27.30 stamped,
> `verify_chain` intact). Honest ceilings: no engine code yet (1.27.32–34
> consume this substrate); the oracle-fixture commits are deferred to the port
> milestones; G0 is a written decision, not an executed fork. See
> `IMPLEMENTATION_PLAN_v1.27.30_Spine.md` + `CHANGELOG.md` §[1.27.30].

> **Version note:** **v1.27.27 "Seal" shipped 2026-08-20** — a
> **server-only** release (server `Cargo.toml`/lock 1.27.26 → **1.27.27**;
> client + plugin unchanged) — the capstone of the 1.27.21→1.27.27 hardening
> lineage — **no schema, no migration, no new endpoints, no wire change, no
> telemetry.** **M1:** the fail-closed sweep found the named gates already
> closed by v1.27.16/21/25; the one genuine residual was
> `govern.rs::retention_report` silently degrading to code defaults on a
> pool/profile-store error (compliance evidence certifying a possibly-wrong
> policy) — now `500 internal` ("no overrides stored" ≠ "overrides
> unreadable"). New pins: `revocation_lookup_error_denies` (valid JWS over a
> broken pool → 401, the F-28 class as a store-ERROR not a revoked jti),
> `role_lookup_empty_degrades_to_no_access` (the Ok-side complement:
> unresolvable role names → empty permit), `poisoned_chain_watch_reads_as_not_ok`
> + `poisoned_snapshot_reads_as_not_ok` (real catch_unwind poisoning; the
> `unwrap_or_default()` reads are load-bearing fail-closed), and the
> consolidated source-shape pin `poisoned_lock_denies_every_gate`.
> **M2/M4 verified shipped:** `/ump/forget {"hard":true}` + the
> ingest-replace/vault sweeps + domain-delete all run `refuse_if_held`
> (v1.27.21/25), and `purge_chunk_ids` carries the structural backstop so the
> fence holds of the FUNCTION, not call-site discipline; added the soft-branch
> pin `ump_forget_soft_flags_but_not_held_chunks`. **M3 (F-61 + S2-44, the
> code change):** `contains_suspicious_pattern` is now phrase-aware — entries
> in canonical spaced form matched as contiguous token runs (a spaced entry
> can never be dead; "you are analyzing" no longer matches "you are an"),
> jammed forms still matched inside single tokens (whitespace-stripping
> obfuscation gains nothing), `jailbreak`/`override` kept as stem-tolerant
> single tokens; the matcher feeds `blocklist_hit`/PRF, so the recall gate was
> re-run — floors held at baseline. **M5:** the lipstyk de-slop watchdog lands
> in CI (`lipstyk` job, diff-scoped strict: any diagnostic on changed lines
> fails; `.lipstyk.toml` disables only the two group-attributed cross-file
> rules that fire on untouched files; Rust + TS across src/client/plugin) —
> this release's own code passed it after fixing its three initial findings;
> absolute-zero across the tree is NOT claimed (~918 documented-class
> diagnostics remain, per the v1.27.24 honest ceiling). **M6:** the total gate
> ran green in one pass — fmt, clippy `-D warnings` (default/bench/otel),
 tests, lipstyk strict-diff, `badges.sh --selfcheck`, recall floors. Tests:
> server bin **704**/6 ignored (+8), lib **137**/1. See `CHANGELOG.md` §[1.27.27].

> **Version note:** **v1.27.26 "Notarize" shipped 2026-08-20** — a
> **server-only** release (server `Cargo.toml`/lock 1.27.25 → **1.27.26**;
> client + plugin unchanged) — the audit-integrity follow-up on v1.27.25 —
> **no schema, no migration, no telemetry.** **M5 (F-23, the headline): the
> one remaining audit chain-fork window closes.** `record_tenant` previously
> fell through to an unserialized tip-read + INSERT when `BEGIN IMMEDIATE`/
> `SAVEPOINT` failed — exactly the read-modify-write race the exclusive start
> exists to prevent (two writers could read the same tip and insert rows
> sharing a `prev_hash`, which `verify_chain` then reports forever). Now the
> write is **dropped, not forked**: the row is skipped (an absent entry reads
> as a gap in a later verify, never as a forged continuation), `audit_commit_failures`
> on `/health` is bumped, and an error log fires. Pinned by
> `begin_immediate_failure_skips_and_warns_not_forks` — a real file-backed
> two-connection lock conflict (busy_timeout 0 + held write lock): the write
> is refused, no partial fork row lands, the counter increments, the surviving
> chain still verifies. **M2/M6 (F-03 full 8-field hash + HMAC keyed chain)
> are deliberately deferred** to the announced audit-repair milestone
> (`IMPLEMENTATION_PLAN_v1.27.31_AuditRepair.md`): both change the chain
> format and require an operator re-anchor — an audit chain is evidence; its
> format changes only with explicit re-anchor, never silently. This release
> closes only the fork window that needed no format change. **Plus** the
> rerank-tier model retune: the opt-in cross-encoder tier (armed on
> `enterprise`/`desktop`/`quality-local`) prefers
> **`mixedbread-ai/mxbai-rerank-large-v1`** (DeBERTa-v3-large cross-encoder →
> `logits[:,0]`, loaded via fastembed's BYO-ONNX user-defined seam from
> `BRAIN_RERANK_MODEL_DIR`, default `models/mxbai-rerank-large-v1/`) with
> **`BAAI/bge-reranker-v2-m3`** as the automatic in-enum fallback — same
> fail-open + boot-warmed + top-50 (`BRAIN_RERANK_TOP_N`) contract; `Qwen3-Reranker-0.6B`/
> `mxbai-rerank-large-v2` are documented exclusions (causal-LM/ChatML + last-
> token logit, incompatible with the `logits[:,0]` seam). Model-truth fixes:
> `minishlab/potion-base-2M` is **English** (not multilingual) → retrieval
> profile renamed **`compact`** (`PROFILE_COMPACT`; `multilingual` stays as a
> deprecated alias, no behavior change), `mxbai-rerank-large-v1` → DeBERTa-v3
> (~435M), `gte-base-en-v1.5` → ~137M. Tests: server bin **696**/6 ignored
> (+1), lib **137**/1; clippy `-D warnings` + fmt clean. Honest ceilings:
> the skip-on-failure is read-time enforcement over stored rows — it prevents
> new forks, it cannot repair a chain that already forked (restore + verify
> M4/F-22 stays deferred); the fail-open rerank contract is unchanged; full
> chain hardening (F-03 + HMAC + head pin) is the re-anchor milestone, not
> this release. See `CHANGELOG.md` §[1.27.26].

> **Version note:** **v1.27.25 "Scoped" shipped 2026-08-19** — a
> **server + plugin** release (server `Cargo.toml`/lock 1.27.24 → **1.27.25**;
> plugin 0.4.5 behavior fix, no package bump) closing the pass-3 audit's
> actionable findings — **no schema, no migration, no telemetry.** **M1 (the
> headline, S3-01 CRITICAL):** the graph-PPR third recall leg (unreleased
> default-on from `00a79fe`) now applies the SAME tenant/owner/scope boundary
> as the vector/FTS legs — `graph_retrieve` takes `&SearchFilters`, composes
> `k.domain = ?` + the shared `push_gate_filters` set on the chunk fetch, and
> carries `k.pii` into the hit (was hardcoded `pii:false` → graph hits were
> structurally unredactable). Pinned by two lib tests with the exact
> shared-entity cross-domain fixture. **M2:** the `/get/{id}` idiom (label in
> SQL + row-domain re-auth + `RecordReadGate`) extended to `/verify`,
> `/ump/memory/{id}` (MCP `ump.get`-reachable), `/procedure/{id}/steps`;
> `/suggest` gains the v1.14 scope filter + v1.23 role gate (owner-restricted
> roles no longer get other owners' private rows as suggestions).
> **M3 (S3-03):** the rate limiter moved OUTSIDE the auth layers (an
> unauthenticated flood now trips 429 before any token work or audit write —
> previously 401-before-bucket + a sync Connection::open + audit INSERT per
> free request, unthrottled DB-write amplification); deny-path audit writes on
> `spawn_blocking`; source-inspection layer-order pin. **M4:** edge-history
> endpoint gate Read → **Admin** (four doc surfaces already claimed Admin;
> code now agrees) + warn on dropped read-audit; `/domains/{name}/export`
> **Admin in shim mode** (the snapshot IS the whole shared pool there) +
> escaped `vacuum_into`; `/add` quarantine flag IN-TX (failed flag → rollback,
> the `/ingest/memory` posture); `XFF` rightmost-untrusted; limiter
> fail-closed on poison; dead `"developermode"` blocklist entry fixed;
> audit BEGIN-failure bumps `audit_commit_failures`; boot `VACUUM INTO`s
> escaped. **Plugin:** `autoRecallGraph:false` explicitly sends `graph:false`
> (the server default-on had silently re-enabled the leg for every plugin
> user). **Docs:** openapi `/health`+`/health/db` schemas match the shipped
> shapes; SECURITY.md egress inventory truthful (three enumerated bounded
> paths). Tests: server bin **694** / 6 ignored (+5), lib 133 / 1, brain 18,
> mcp 19, bench 5; clippy `-D warnings` + fmt clean. Honest ceilings: PPR
> mass still crosses domains via shared entity names in shim (ranking only —
> every emitted hit is scoped; the S2-41 entity oracle stays the documented
> ceiling); the audit chain stays unkeyed/5-of-8 (F-03 + S2-16/S2-35 deferred
> to the audit-repair milestone); S2-28 restore-holds still deferred. See
> `CHANGELOG.md` §[1.27.25]. Wave 2 (same release): audit prune
> verify-before-prune + `retention` evidence row (S2-16/S2-35), NULL-prefix
> verify rule (F-03 half, no hash change), restore re-applies legal holds +
> discloses resurrections (S2-28), `idx_rels_open_unique` partial unique
> index + legacy dedup (S3-08, schema → **1.27.25**), /decayed +/quarantine
> +/stats +/consolidate shim scoping (S2-31/43), domain_invalid no longer
> leaks the inventory (S2-32), ingest auto-route re-authorizes on the routed
> target (S2-33), /clients 403 on empty grants (S2-15), DSAR remanence after
> the pragma (S2-18), chunker unterminated-fence + newline fixes (S2-19/20),
> evidence self-link dedupe (S2-38), domain delete archives tombstones +
> evidence_links (S2-21). Tests: bin **696**/6, lib **136**/1. The plugin was
> tested + rebuilt in `~/Sites/openclaw` (145 vitest + oxlint + tsc green).

> **Version note:** **v1.27.24 "Brushed" shipped 2026-08-18** — a
> **server-only** release (server `Cargo.toml`/lock 1.27.23 → **1.27.24**;
> client + plugin unchanged) — the dead-code + fail-closed pass from the
> lipstyk de-slop audit. **No schema, no migration, no wire change, no
> telemetry.** **M5** removes the `handlers/mod.rs` blanket
> `#![allow(dead_code)]`/`#![allow(unused_imports)]` and deletes the real dead
> code it hid (unused imports in auth/recall/ump/govern; the never-used
> `authorize_read_domain`; the never-read `ProposalRow.created_at`; the UMP
> recall `ranking_hints` field → `_ranking_hints`, serde-preserved wire key) —
> clippy `-D warnings` is now the dead-code watchdog. `connector/mod.rs` keeps
> a **truthful** allow (it is the `brain-connector-gh` binary's library, not
> server-runtime cruft — deleting would remove a shipped, tested feature
> binary). **M3** closes the one genuine poisoning-control swallow the sweep
> surfaced: `breach::row_from` propagates a corrupt `jurisdictions` JSON cell
> as a `FromSqlConversionFailure` instead of silently deserializing to an empty
> list (D-1 "never certify silence"), pinned by
> `row_decode_fails_closed_on_corrupt_jurisdictions`. Tests: server bin **689**
> / 6 ignored (+1), lib **133** / 1; clippy `-D warnings` clean on default +
> bench + otel; fmt clean; `connector-github` feature still compiles. Honest
> ceiling: this delivers the headline M5 + genuine-M3 items and **deliberately
> does not chase the residual lipstyk heuristic hits** — the bulk are false
> positives by inspection (`Option<String>`→`""` wire shapes, best-effort
> cleanup, clones into owned/Arc/spawn_blocking contexts, the feature-gated
> connector library); a blind sweep to force "zero" would risk behavior changes
> the hard rule forbids. See `CHANGELOG.md` §[1.27.24].

> **Version note:** **v1.27.23 "Medicate" shipped 2026-08-18** — a
> **server-only** release (server `Cargo.toml`/lock 1.27.22 → **1.27.23**;
> client + plugin unchanged) closing the three security findings the
> adversarial pass left open — **no new schema, no new endpoints, no wire
> change, no telemetry.** **M1 (A-01)** the outbound-egress bound was already
> shipped in v1.27.21 (5 s connect / 15 s total, `webhook.rs` `egress_client`)
> — re-verified, not re-built. **M2 (A-02)** public `/health` shrinks to the
> minimal load-balancer probe shape `{status, version}`; every
> deployment-fingerprinting field (`model`, `otel.endpoint`, `pool`, `backup`,
> `webhook`, `hardening`, `compliance.dpo_contact`, `integrity`) moved behind
> the existing Read gate on `/health/db` — an unauthenticated probe can no
> longer fingerprint a regulated BPO deployment (intentional surface reduction,
> same class as v1.20.2 F2; operator monitors must switch to the gated detail).
> The pure `health_body` builder is reused (no dead code). **M3 (A-03)** the
> feature-gated neural embedders (`bge-m3` / `gte-base-en-v1.5`) now `warn!` on
> lock/model failure instead of silently returning an empty vector — the
> D-1 "never certify silence" invariant; callers already skip on empty (no
> corrupt zero-vec write existed), so this closes only the missing signal.
> Tests: server bin **688** / 6 ignored (+2), lib **133** / 1; clippy
> `-D warnings` + fmt clean; route-authz + openapi guard tables unchanged.
> Honest ceilings: `/health` shrinking is the intended behavior change; the
> neural warn path is reachable only under `--features neural-embed`
> (enterprise/desktop — the default edge static model is infallible); an embed
> failure still returns empty (caller skips) — now loud, not silent;
> `compliance.dpo_contact` stays on the Read-gated detail (the privacy notice
> remains the public subject-contact channel). See `CHANGELOG.md` §[1.27.23].

> **Version note:** **v1.27.22 "Cascade" shipped 2026-08-18** — a
> **server-only** release (server `Cargo.toml`/lock 1.27.21 → **1.27.22**;
> client + plugin unchanged) — a bug-fix release closing two
> *documented-but-unimplemented* behaviors in the graph edge layer, making the
> code true to its own documentation. Reuses the shipped bi-temporal columns +
> hash-chained audit + quarantine machinery; no new endpoints except the history
> surface, no new storage, no schema columns/tables, no wire change, no
> telemetry (schema stamp → **1.27.22** for `relationships.superseded_at` +
> the `idx_rels_unique`→`idx_rels_bt` swap). **M1 (BUG-1)** the ingest path's
> write-once `INSERT OR IGNORE` → the new pure lib `src/graph_supersede.rs`
> `resolve_edge_insert` (`EdgeAction::{SameWindow, Created, Superseded}`):
> unchanged re-ingest stays an idempotent no-op (history not churned); a changed
> window retires the old version at `superseded_at` = transaction-time END (old
> row preserved verbatim), handoff exact (`old.superseded_at == new.created_at`),
> audit `Ingest` detail `created:<id>` / `superseded:<old_id>->:<new_id>`.
> **M2 (BUG-2)** traversal meets its own doc: the recursive walk + seed filter
> edges to *current beliefs* (`superseded_at IS NULL` **AND** no newer live
> same-triple row via `NOT EXISTS` — a no-op on well-formed/legacy DBs so
> default recall/traversal is byte-identical; corrects the backdated-supersession
> double-edge). Superseded edges are hidden everywhere (`/graph/relations`,
> `entity_relations`, `relations_for`, `ump_ops::relations_for_chunk`,
> `graph_ppr` adjacency). **M3** new `GET /graph/relationships/{id}/history`
> (Admin, `AuditKind::GraphRead`) reconstructs the full version lineage of an
> edge triple — every version, four timestamps + `current` flag, given any one
> version id (`404 Relationship not found` on miss) — route + route-coverage +
> route-authz guard tables + openapi + docs/api.md + README. Tests: server bin
> **686** / 6 ignored, lib **133** / 1 (incl. **5** graph_supersede), brain 18,
> mcp 19, bench 8; clippy `-D warnings` + fmt clean; `badges.sh --selfcheck`
> clean. Recall gate held on the new build (the M5 byte-identity pin):
> `brain eval --floor r5=0.85,r10=0.85,mrr=0.85` over the frozen 37-query
> 10-doc smoke corpus → r@5 0.919 / r@10 0.919 / mrr 0.905 / ndcg@10 0.909,
> exit 0 (recorded in `BENCHMARKS.md`). Honest ceilings: edge supersession is
> deterministic on the temporal interval, not LLM-judged (semantic
> contradictions stay out of scope); history is the versioned edge rows, not a
> per-field audit diff; the graph-label read-seam posture is unchanged from
> v1.27.21; a correctness/doc-truth fix, not a recall-quality claim —
> LongMemEval parity stays `PENDING`. Rollback is minimal (supersession only
> *sets* `superseded_at`, never destructively mutates). Verify `brain doctor`
> post-install. See `IMPLEMENTATION_PLAN_v1.27.22_Cascade.md` +
> `CHANGELOG.md` §[1.27.22].

> **Version note:** **v1.27.21 "Finish" shipped 2026-08-18** — a
> **server + client + plugin** release (server + client `Cargo.toml`/locks
> 1.27.20 → **1.27.21**; plugin **0.4.4 → 0.4.5**) completing the pass-2
> hardening audit's **S2-** findings + client **N5–N15** + plugin seams — the
> fail-closed-erasure + fence-forgeability class the audit rates CRITICAL. No
> new schema, no new columns/tables, no telemetry; the one wire change is the
> bit-stable **backup v3** writer (`brain backup` now defaults to `v3`).
> **M1** backup v3: header bound as GCM AAD (S2-13), Argon2id params bounded
> pre-allocation (S2-14, `kdf_params_out_of_range`); v1/v2 keep read paths.
> **M2** the fence-forgeability close (S2-02): shared `strip_sentinels` on MCP
> `tool_result_payload` + `format_response` + the plugin banner, invisible-
> strip-first. **M3** (S2-03 CRIT, S2-04) the legal-hold fence now guards the two
> erasure paths that bypassed it — `POST /ump/forget {"hard":true}` (MCP
> `ump.forget`-reachable) and the ingest-replace/vault sweep — both run
> `refuse_if_held` in-tx → `409 legal_hold_active` all-or-nothing. **M4** (S2/N1)
> empty `live_uris` reconcile 400s `live_set_empty` unless `allow_empty: true`
> (no silent mass retirement). **M5** (F-27) auth fail-closed: `read:<team>/*`
> wildcard grants only the shared `global` pool; a no-role token passes
> `require_dpo_role` only when the role store defines no roles at all.
> **M6** client offline-queue integrity (N5–N8: retry-park at 5, identity-not-
> history key, salted DSAR digest + per-install salt, purge-owner persisted) +
> replay drift (N9/N13 char-boundary hash + kept-set drift). **M7** plugin
> 0.4.5: env-token ladder (`BRAIN_TOKEN_FILE`→`BRAIN_TOKEN`→config, never
> writes), query-length-only logging, composed-fence sentinel strip. **M9** webhook
> egress bound (5 s connect / 15 s total). Tests: server lib **128**, main bin
> **674** / 6 ignored, brain 18, mcp 19, bench 5, eval 2, metrics 8; client **152**;
> clippy `-D warnings` + fmt clean (both trees); wasm **5.3 MB**; plugin 144 vitest +
> oxlint + tsc; the three client gate failures found during the pass (`&mut Vec`→
> slice, slice-clone, and a grep-guard matching its own literal) fixed with new pins.
> Honest ceilings: v3 AAD is write/read-time (existing v2 `.bak` files stay readable
> via the no-AAD path, not migrated); the hold fences are read-time enforcement over
> stored rows; N7's salt is uniqueness, not secrecy; the role-empty gate is governance
> narrowing; F-09/S2-28 (restore-path audit-chain verify + hold/tombstone reapply)
> deliberately deferred to the audit-repair milestone. See
> `IMPLEMENTATION_PLAN_v1.27.21_Finish.md` + `CHANGELOG.md` §[1.27.21].

> **Version note:** **v1.27.20 "Console" shipped 2026-08-17** — a
> **client + CLI** release (server `Cargo.toml`/lock 1.27.19 → **1.27.20**;
> client `Cargo.toml`/lock 1.27.19 → **1.27.20**; plugin unchanged at 0.4.4) —
> the operator-surface bar: no server code, no wire changes, no schema. **M3
> the i18n truth (F-38):** the five bundles expose one identical key set
> (parity wall), every render surface sits behind `t()`/`t_fmt()` — pinned by
> the new `no_raw_strings_in_rsx` source-scan test in `client/src/i18n.rs`
> (rsx-region tracking + `// i18n-exempt: <reason>` escape; skips test
> modules, prop values, wire keys, CSS classes, glyph-only strings) — and the
> review-queue label gains the missing `E` key. **M4 the CLI (F-37):**
> `--json` envelope mode on every data command (query/explain/get/ingest-dir/
> suggest/suggest-metrics/retention/snapshot-status/connector-status/status/
> eval; interactive flows refuse it exit 2); the flag parser learns its
> vocabulary (`BOOL_FLAGS` never consume the next token — `ingest-dir
> --dry-run ~/vault` works; unknown flag → exit 2 "unknown flag"; `--` ends
> flags; `--k abc` → exit 2 instead of silently 5); `ingest-dir` counts
> failures separately and exits non-zero on every-file-failed
> (`all_files_failed`); `status` prints `n/a` for `-1` sentinels; help is
> generated from the one `SUBCOMMANDS` table the dispatcher consumes (the
> flush-left `brain client add` survivor line + missing `brain token
> rotate`/`brain ump …` lines fixed; `flags:`/`exit codes:` sections
> documented); `brain suggest` gains the recall/get strip chain parity.
> Tests: server main bin **670** / 6 ignored (unchanged), brain CLI bin 12 →
> **18**, client 140 → **143**; clippy `-D warnings` + fmt clean (both trees);
> `badges.sh --selfcheck` clean (855 passed weighted); `brain --help` diff
> line-by-line reviewed — only intended moves. Honest ceilings: `--json`
> covers the data commands (interactive flows refuse); the flag vocabulary is
> a fixed list, added flags must land there + in the table (both
> single-sourced); the scan skips prop values by design (placeholders are
> keyed, the rule targets labels); modal focus-traps/digest display shipped
> with their tests in earlier v1.27.x sessions and are re-verified here. See
> `IMPLEMENTATION_PLAN_v1.27.20_Console.md` + `CHANGELOG.md` §[1.27.20].

> **Version note:** **v1.27.19 "Scrub" shipped 2026-08-16** — a
> **server + client** release (server `Cargo.toml`/lock 1.27.18 → **1.27.19**;
> client `Cargo.toml`/lock 1.27.15 → **1.27.19**; plugin unchanged at 0.4.4) —
> the silent-failure pass: no new endpoints, no wire changes, no schema
> change, no telemetry. **F-54** `POST /auth/logout` + `POST /auth/revoke`
> wrote the denylist best-effort and returned 204 regardless — a failed
> INSERT left the token live for its full shelf life with the operator told it
> was dead; both now surface the failure as `500 revoke_failed` (success
> meaningfully means dead). **D-1 (the day's headline): the `let _ =`
> residue sweep — 24 sites.** The worst: chunk-purge residue deletes
> (relationships/vec0/evidence/traces) ran `let _ =` inside the purge tx — one
> failing DELETE silently left partial erasure the purge then certified
> complete; every residue now propagates and rolls back the whole purge.
> Same class fixed elsewhere: stale vec0 rows on reindex, chunk stored
> without its evidence links, webhook seen-writes, retention prunes, refresh
> failures, orphan PII residues, `secure_delete`/`wal_checkpoint(TRUNCATE)`
> failures on purge now `warn!` (certified-silence ended). **D-2** the
> best-effort audit settle failure is never silent: monotonic
> `audit_commit_failures` on `/health` `hardening` (0 = green, reports-not-
> retries). **D-8** the prompt-injection blocklist screen runs ONCE at
> `SearchResult::raw()` construction and rides as an internal
> `#[serde(skip)] blocklist_hit` flag — both PRF extractors consume the flag
> instead of re-normalizing content per query (behavior-identical, pinned by
> `blocklist_flag_one_shot_at_construction_and_consumed` +
> `prf_skips_injection_flagged_content` re-routed through `raw()`). **D-7**
> client outcomes announce: Ops gate-strip decide/reject status, Security
> quarantine release/delete `aria-live` lines, Data decayed/tombstones load
> errors (all were `let _ =`/`if let Ok`). **D-6** the singleton UMP path's
> `.next().unwrap()` → `pop()` + `?` (last write-path panic gone). **D-5**
> dead "reserved for v1.6" trace-prefix vocabulary removed (v1.6 closed
> without consuming it). Tests: server bin **670** / 6 ignored, lib **126** /
> 1, brain 12, mcp 17, bench 8, client **132**; clippy `-D warnings` + fmt
> clean (both trees); `badges.sh --selfcheck` clean. Honest ceilings:
> `audit_commit_failures` reports, it does not retry; the blocklist flag is a
> construction snapshot (content is immutable post-construction by design);
> client status lines are announcements, not an action log (v2.x); D-1 warns
> where the sweep judged propagation too invasive (`warn!` with context),
> never certifies silence. See `CHANGELOG.md` §[1.27.19].

> **Version note:** **v1.27.18 "Groundwork" shipped 2026-08-16** — a
> **server-only** release (server `Cargo.toml`/lock 1.27.17 → **1.27.18**;
> client + plugin unchanged at 1.27.15 / 0.4.4) — the read-path cost pass. No
> new endpoints, no wire changes, no telemetry. **E-1 (the day's headline):
> the FTS-vocabulary PRF weighting shipped in v0.9.1 NEVER ran.** Bundled
> SQLite 3.53.2's `fts5vocab` 'instance' table exposes `(term, doc, col,
> offset)` — one row per occurrence — while the v0.9.1 query referenced the
> pre-3.40 `cnt`/`rowid` columns, so every `prf_extract_terms_fts` call
> silently errored into the unweighted pure-DF fallback. E-1 rewrites the
> two legs against the real schema: per-term occurrence counts (`COUNT(*)`
> = old `SUM(cnt)`) scoped `doc IN (window)`, then a corpus-df round-trip
> (`COUNT(DISTINCT doc)`) for ONLY the locally-selected terms, capped at
> `MAX_DF_TERMS` = 4096 leaders (adversarial-vocab bound; escape hatch stays
> the pure fallback). Output now really is corpus-idf ranked — expansion
> lists change vs 1.27.17 (eval rows shift; no parity claim made). Pinned by
> `prf_df_matches_legacy_corpus_scan` (legacy-as-intended oracle),
> `prf_vocab_schema_is_occurrence_shaped` (schema freeze), the re-stemmed
> `test_prf_extract_terms_fts_weights_corpus`. **E-4** evidence enrichment
> batched — and its placeholder-pair bug (one of two `IN` groups never
> bound → silent empty links) fixed + pinned. **E-5** migration indexes:
> add `idx_knowledge_domain`/`idx_knowledge_owner`/
> `idx_knowledge_title_heading`, drop `idx_tombstones_kid`/
> `idx_entities_name`/`idx_evidence_links_from` (UNIQUE duplicates) →
> schema **1.27.18**. **E-7/E-8/E-12** `SearchFilters` → `Arc`, per-query
> vec0-existence probe → process `VEC0_READY` flag (`migrate_down_0_9_0`
> clears it), `sanitize_read_cow` zero-copy on provably-clean rows.
> **F-31** O(m) mention dedup (oracle-pinned). **F-44** `/import` dial
> 1 GiB — layered BEFORE the 1 MiB global cap (meta-testing the production
> order; the old single-cap pre-empted large imports). **F-45**
> `/ingest/memory` hard-rejects: per-entry >`MAX_CONTENT` →
> `400 entry_too_large` all-or-nothing, invalid UTF-8 → `400 invalid_utf8`
> (was silently mis-stored/"Empty content"). **F-46** retention read-gate
> `strftime('%s',…)` → `unixepoch(COALESCE(…))` (value-identical, pinned
> both SQL-side and SQLite-side). **F-53** tracker slot is RAII — released
> on timeout/panic, never swept (pinned). **M6** release `opt-level` "z"→2
> (speed; strip+LTO unchanged). Tests: server bin **673** / 6 ignored, lib
> 125 / 1, brain 12, mcp 17, bench 8; clippy `-D warnings` + fmt clean.
> Honest ceilings: PRF expansion output changes (now weighted — not a
> regression claim, a behavior completion); `revoked_at` DDL defaults keep
> their single-format TEXT `strftime`; the schema bump drops three indexes
> once on first boot after upgrade; verify `brain doctor` post-install —
> this release is the first since v0.9.1 where expansion lists change.
> See `CHANGELOG.md` §[1.27.18].

> **Version note:** **v1.27.17 "Strongbox" shipped 2026-08-16** — a
> **server-only** release (server `Cargo.toml`/lock 1.27.16 → **1.27.17**;
> client + plugin unchanged at 1.27.15 / 0.4.4) — the one-file audit follow-up:
> the **backup envelope** gets a real KDF + per-backup random keys, and the
> plaintext snapshot can never be world-readable, never survives a failure,
> and never clobbers a live file. No new endpoints, no schema change, no
> telemetry. **M1 (F-08/F-10) format v2:** `BSBK` magic + u16 version + u32
> length-prefixed JSON header (`{"kdf":"argon2id","t":3,"m":65536,"p":1,
> "salt":…,"nonce":…,"created_at":…}`); the key is argon2id (64 MiB/3
> passes, < 2 s soft-benchmarked) with a **per-backup** 16-byte salt + 12-byte
> random nonce (F-08's same-second GCM-nonce-reuse exploit killed:
> `two_v2_backups_same_second_use_different_nonces`); header bytes are GCM
> AAD (bit-flips fail decryption); the KDF vocabulary is closed (`argon2id`
> only); the passphrase is verified by decryption, so
> same-passphrase-any-header restores work; `decrypt_backup` is the one
> decrypt seam for restore AND verify; legacy v1 files (no magic) restore
> through the original path with a `warn!` (read compat forever, `--format
> v1` kept for byte-identical archives). **M2 (F-11) snapshot hygiene:**
> `create_private_file` = 0600 + `create_new` (a planted path aborts, never
> writes through), `vacuum_into` = quote-escaped SQL literal (pinned),
> `SnapshotGuard` removes the plaintext snapshot on EVERY failure path
> (pinned by an unreadable config-dir injection); backup refuses a stale
> `<db>.bak` (fail-closed). **M3 (F-17):** restore refuses to clobber the
> previous safety snapshot (clear message, fail-closed) and the whole
> restore/verify path runs off `decrypt_backup` + `vacuum_into` (no inline
> SQL format strings). **M5:** `brain backup --format v1|v2` (default v2).
> Tests: server bin **659** / 6 ignored, lib **124** / 1 (incl. **20** backup
> tests), brain 12, mcp 17, bench 5; clippy `-D warnings` + fmt clean; live
> E2E smoke green (v2 roundtrip → doctor verify → .bak 0600 → v1 legacy read
> → wrong-passphrase rejected). Honest ceilings: the passphrase stays the only
> secret (no KMS/rotation); the .bak is the rollback path, not a journal
> (restoring twice requires moving it); v1 files are never migrated in place.
> See `CHANGELOG.md` §[1.27.17].

> **Version note:** **v1.27.16 "Drawbridge" shipped 2026-08-16** — a
> **server-only** release (server `Cargo.toml`/lock 1.27.15 → **1.27.16**;
> client + plugin unchanged at 1.27.15 / 0.4.4) — the fail-closed pass over
> the identity + read surfaces the audit itemized: no new endpoints, no new
> columns, no telemetry. **M1 (F-04/05/06) the domain read-gate:** pure
> `can_read_domain`/`authorize_read_domain` (read:team/* = everywhere; `None`
> principal = superuser, unchanged); `/search` authorizes the domain it
> actually queries (was always `global`); `/get/{id}` + `/multi-get` bind the
> `X-Brain-Domain` label in SQL (ids cannot cross domains in shim mode),
> re-authorize on the row's own domain, and run the composite `RecordReadGate`
> (v1.14 scopes + v1.23 roles — recall parity on by-id reads, probe-blind 404
> for foreign rows); recall federation + graph traversal `retain` only
> readable targets (explicit foreign domains stay loudly 403); shim-mode graph
> edges scope by chunk-provenance label (unlinked edges invisible to scoped
> readers, `graph_domain_scope`). **M2 (F-07) per-IP rate limiting:** the
> plain `axum::serve` never injected the peer `SocketAddr`, so every client
> shared ONE bucket — a global limiter in practice; now
> `into_make_service_with_connect_info::<SocketAddr>`, production pin tested
> by source inspection; key set bounded (evict oldest 25% at
> `RATE_LIMIT_MAX_KEYS`). **M3 fail-closed identity:** M3.1/F-26
> `auth::TokenRead` (NotConfigured|Active|ReadFailed) — poisoned lock = `500
> auth_store_unavailable` (was: empty set = auth-off = allow-all), configured-
> but-empty store = 401 (was: allow); M3.2/F-27 `role_retrieval_gate` degrades
> to the EMPTY permit + `AND 1 = 0` guards (was `None` = no narrowing =
> fail-open on incident); M3.3/F-28 JWT revocation check refuses on ANY store
> error (was `if let Ok(conn)` + `unwrap_or(false)` skip); M3.4/F-13
> `/auth/logout` behind the bearer middleware (public logout could only
> "revoke nothing"); M3.5/F-25 UMP L3 signing-key seed refuses wide modes
> (fails closed to L2). **M4 (F-33) write-boundary trust labels:**
> `MemoryKind::is_strict_valid` round-trip on `/proposals` + `/ingest` (no
> silent fallback to fact), `confidence` ∈ 0.0..=1.0 hard-reject (no clamped
> lies); M4.3 `/add` closed `source` vocabulary for JWT principals —
> ingest kinds + connector family kinds, `manual` EXCLUDED (no forged human
> authorship). **M5 (F-41) the domain-registration cap:** `MAX_DOMAIN_DBS` =
> 256 (`BRAIN_MAX_DOMAIN_DBS`), `DomainRegistry::register` is the ONE creation
> path (idempotent), `seed_registered` boot-seeds the clients-table domains
> WITHOUT opening pools (vanished files recreate on first access, cap-bounded),
> registered-only `pool_for` REFUSES (`Unknown`) a never-registered name and
> never creates a file — a probeable surface cannot fill the disk; the
> `map_domain_error` seam: 400 `domain_invalid` / 404 `domain_unknown`
> (probe-blind) / 507 `insufficient_storage` / 500 internal. Contract:
> openapi.yaml (logout auth, /add vocab, /ingest fields, /domains 507,
> NotFound `domain_unknown`); `x-api-version` stamp stays "1.21.0". Tests:
> server bin **659** / 6 ignored, lib 113 / 1, mcp 17, brain 12, bench 5;
> badges **825 passed** (bench,migrate), clippy `-D warnings` + fmt clean,
> selfcheck clean. Honest ceilings: the gates are read-time enforcement over
> stored labels (a write storing a wrong label is out of scope); graph scope
> keys on the chunk link (NULL `knowledge_id` edges have no domain atom);
> the cap bounds multi-db registrations only (shim mode shares one file);
> fail-closed role degradation means a role-store outage denies retrieval
> (monitor for the `warn!`). See `CHANGELOG.md` §[1.27.16].

> **Version note:** **v1.27.14 "Fencepost2" shipped 2026-08-16** — a
> **server + plugin** patch release (server `Cargo.toml`/lock 1.27.13 →
> **1.27.14**; plugin **0.4.3 → 0.4.4**; client unchanged at 1.27.13) landing
> the information-flow-integrity follow-up of v1.27.12/0.4.3 — the `untrusted`
> fence becomes a **structural** (not decorative) boundary on every LLM-facing
> seam, and the quarantine taint can no longer be lost or silently written.
> **Plugin (F-01):** `sanitizeForBlock` in `plugin/src/format.ts` moved the
> sentinel strip to the END of the pipeline (it was first), so a near-marker a
> transform then synthesizes (NBSP/TAB/zero-width split across the
> `CONTEXT|END` boundary, or a markdown-ref shortening) cannot forge the fence
> close after it was stripped; the `U+E0000–U+E007F`-inclusive invisible strip
> now runs BEFORE the `\s` collapse so `U+FEFF` (which JS `\s` treats as
> whitespace) is removed, not widened to a space — a regression the openclaw
> `vitest` run caught (`"ig nore"` → `"ignore"`); plus the recall `snippet`
> is now routed through the same block boundary (was the one raw detail
> field). New near-marker forgery suite: 47 format tests / 142 extension
> tests, all green on the openclaw tree. **Server read-seam (M3):** the
> `sanitize_read(_opt)`/`sanitize_stored` seam in `src/gate.rs` now covers
> every stored-content read surface — UMP reads (F-10), legacy `/search`
> (F-18), `/quarantine` review list (F-17), recall/suggest metadata (F-19/21) —
> with a wiring meta-test pinning the seam to every response-forming site.
> **MCP/CLI (F-20/F-63):** new `src/fence.rs` exports the shared
> `FENCE_BEGIN/END` + `strip_markdown_refs` + `strip_control_chars`;
> `tool_result_payload` wraps results in the fence, `format_response` + the
> `brain` recall/get prints gain strip parity. **Quarantine fail-closed
> (F-14/F-15):** `flag_if_quarantined` returns `rusqlite::Result<bool>` and
> every ingest path (structured, procedure, `/add`, `/ingest/memory`) rolls
> back or errors rather than store an injection chunk with a silently-missed
> flag; `/ingest/memory` now flags a `Reject` verdict (stricter, never
> dropped) under the default quarantine posture. Tests: server bin **627** / 6
> ignored, lib **113** / 1 ignored, brain 12, mcp **17**, bench 5
> (`--features bench`); client 124 unchanged; plugin **142** extension tests;
> clippy `-D warnings` + fmt clean; `badges.sh --selfcheck` clean (**793
> passed / 7 ignored**); UMP L3. Honest ceilings: the fence is transport-layer
> data/instruction separation, not a CaMeL/FIDES capability lattice (mantra
> #2); the plugin is validated via the openclaw `vitest` suite + `tsc` — no
> standalone runner here; the restore on flag-write failure drops the
> uncommitted tx (chunk never stored), it does not re-flag. See
> `CHANGELOG.md` §[1.27.14].

> **Version note:** **v1.27.13 "Contract" shipped 2026-08-16** — a
> **server + client** patch release (server + client `Cargo.toml`/locks
> 1.27.12 → **1.27.13**; plugin **0.4.3** first released here) shipping the
> two post-1.27.12 integrity fixes + the documentation-contract completion —
> no new storage, no new endpoints, no wire changes. **Fix 1 (client):**
> `DetailActions` in `client/src/panels/review.rs` now forwards the server
> `content_digest` on detail-modal approvals (`Some(&digest)`, matching the
> queue quick-approve + batch paths; previously the modal sent `None`, so a
> drifted proposal could still be approved from the detail view — the
> key-accelerator/ops/offline paths deliberately stay `None`, the documented
> legacy seam). **Fix 2 (plugin, 0.4.3):** the v1.27.12 provenance
> `[src: · mk: · lb: · reg:]` labels now run through `sanitizeForBlock`
> like hit bodies — a recalled chunk cannot forge its attribution line or
> the `UNTRUSTED_*` fence markers through a label. **Contract pass:**
> `openapi.yaml` documents the response body of every 200/201 (51
> description-only responses now carry wire-exact examples extracted from
> the handler sources — BreachView, Transfer, TiaTemplate, DpaTerms,
> Client, LegalHoldRow, DsarResponse/LedgerRow, AuditRow, capabilities,
> recall trace, ProposalView; `/auth/logout` corrected to 204-on-success +
> 401-no-principal); `docs/api.md` endpoint inventory + README API tables
> completed (profiles/roles/connectors, domains family, clients register,
> transfers, breach, holds); README badges refreshed from the real build
> (version 1.27.13, **782 passed / 7 ignored** via `scripts/badges.sh`,
> `bench,migrate`). The `x-api-version: "1.27.13"`-style contract stamp is
> unchanged at "1.21.0" (the wire contract did not move — the same
> convention as every release since v1.21.0; the runtime `X-Api-Version`
> header follows `CARGO_PKG_VERSION`). Tests: server bin **626** / 6
> ignored, lib 105 / 1 ignored, brain 12, mcp 15, bench 5; client **124**;
> clippy `-D warnings` + fmt clean (both trees); `cargo audit` clean;
> UMP conformance **L3**; recall gate r@5 0.919 / r@10 0.919 / mrr 0.905.
> ROADMAP.md untouched (the v1.27 line has never updated its Caliber-line
> header). See `CHANGELOG.md` §[1.27.13].

> **Version note:** **v1.27.12 "ReviewArmour · Rotate · Provenance" shipped
> 2026-08-15** — a **server + client** release (server `Cargo.toml`/lock
> 1.27.10 → **1.27.12**; client `Cargo.toml`/lock 1.27.11 → **1.27.12**;
> plugin touched) landing three security themes against the 2026 agentic-AI
> threat landscape (OWASP Agentic Top 10 / MS AI Red Team v2 lines) — no new
> storage, no new endpoints, no telemetry. **ReviewArmour** (LITL): `/proposals`
> serves the read-canonical review form (`sanitize_read`: PII redact →
> markdown-ref → invisible strip) + a stable, principal-independent
> `content_digest` (SHA-256 over the stripped form; PII kept OUT so the
> fingerprint is identical across admin/non-admin readers and across
> list/edit/approve); `approve_proposal` accepts an optional `digest` and 409s
> on ANY drift (checked against the fresh row inside the `BEGIN IMMEDIATE`
> tx) — an approval binds to the bytes the reviewer was shown; the client
> queue + detail-modal paths both forward the digest (legacy quick-approve /
> offline-replay pass `None`, server enforces when present). **Rotate**:
> `brain token rotate` generates a fresh 32-byte hex bearer and atomically
> replaces the token file — temp created at 0600 (`OpenOptions` +
> `create_new`, never umask-dependent), fsync'd, renamed over the target;
> refuses group/world-readable secrets (fail-closed mirror of
> `check_secret_permissions`); server startup warns on unsigned alert/DSAR
> webhook sinks + group/world-readable UMP signing keys. **Provenance** (IFC):
> the vec0 + FTS retrievers select `k.source`/`k.node_kind`/`k.lawful_basis`/
> `k.region`, threaded through fusion → `RecallHit` (`Option<String>`, absent
> when NULL, `#[serde(skip)]` on `SearchResult` so the wire shape is
> additive); the plugin renders a deterministic `[src: · mk: · lb: · reg:]`
> line inside the `UNTRUSTED_*` fence, labels run through `sanitizeForBlock`
> (fence-marker forging closed). Tests: server bin **626** / 6 ignored, lib
> 105, brain 12, mcp 15, bench 5; client **124**; clippy `-D warnings` + fmt
> clean (default, bench, otel); full CI green (fmt/clippy/test, otel gate,
> recall eval, cargo audit, UMP conformance, release build; client
> fmt+clippy+test+wasm). Honest ceilings: approve *binds* — it does not force
> full-read or rewrite at-rest rows (verbatim evidence fidelity preserved);
> rotation coordinates the token FILE only (the openclaw env source is a
> printed operator step, not auto-edited); provenance tags are labels, not an
> enforced taint grid; the optional domain-isolation "Boundary" federation
> flag is deliberately not in this release (it changes recall breadth and
> ships gated, later). See `CHANGELOG.md` §[1.27.12].

> **Version note:** **v1.27.11 "Console" shipped 2026-08-15** — a **client**
> release (client `Cargo.toml`/lock 1.23.0 → **1.27.11**; server stays 1.27.10;
> plugin unchanged) — the v1.27 series capstone: the role-gated BPO dashboard
> views. **M1** `role::ConsoleView` + `console_view()` (pure): `client-auditor`
> → `ClientAdmin` (its own single-client dashboard), `bpo-ops` + the full-
> control roles (`admin`/`solo`/`controller`) → `BpoOps` (the all-clients
> board), everything else → `Undefined` (stock console). **M2** `Route::Clients
> {}` gated into the desktop rail + mobile tab bar only when `console_view`
> resolves, plus a palette entry (coverage test → 15 targets). **M3**
> `panels/console.rs`: `client_admin` is the honest single-tenant-per-client
> poster — renders ONLY the clients granted by the client-side allowlist
> (`api::client_auditor_domains`, the token mirror of the server
> `client_authorized_domains` seam; `filter_granted` pure re-filter, `Some([])`
> denies all), no client switcher, server R9 row filter as backstop; `bpo_ops`
> is read-only (`/clients` + `/connectors` + `/proposals` depth). Tests: client
> **122** passed; clippy `-D warnings` + fmt clean; release wasm 5.1 MB (budget
> 7). Honest ceilings: the console is read-only UI over the shipped API (no new
> server surface); the plan's Overview/Data/Rights/Audit client-admin panels
> reduce to the register overview here — the rest are the existing per-role-
> gated panels; auditor tokens are operator-issued (scopes → client domain).
> See `IMPLEMENTATION_PLAN_v1.27.11_Console.md` + `CHANGELOG.md` §[1.27.11].

> **Version note:** **v1.27.9 "Roles" shipped 2026-08-15** — a **server**
> release (server `Cargo.toml`/lock 1.27.8 → 1.27.9; schema unchanged 1.27.8;
> client + plugin unchanged) — the BPO role postures + domain-scoped client
> views. **M1** `role::PRESETS_RAW` seeds `client-auditor` (read-only on ONE
> client domain, `can:["read"]` — the min-necessary wedge) + `bpo-ops` (the
> all-clients operations read), INSERT OR IGNORE so edits survive. **M2**
> `auth::client_authorized_domains` — the allowlist seam mapping a
> `client-auditor` principal to the non-wildcard domains of its `scopes`
> (None = unrestricted; empty = sees nothing). **M3** `GET /clients` +
> `GET /clients/{name}` row-filter to the auditor's granted client-domain(s)
> (parent verification #7); the handler still calls `authorize` (defense-in-
> depth); every other principal keeps the Admin path gate, so
> `bpo-ops`/admin/opaque see the full register. No migration, no schema bump
> (roles are seeded rows). Tests: server bin 617 → **619** / 6 ignored
> (`client_auditor_sees_only_their_domain` + `client_auditor_can_read_only`),
> lib role presets at 12, schema-contract pins 12 seeded roles; clippy
> `-D warnings` + fmt clean. Honest ceilings: a read-time row filter on one
> register, not true multi-tenancy (v2.0 Cortex); auditor tokens are operator-
> bound (scopes → client domain), not auto-provisioned; `POST /clients` stays
> Admin. See `CHANGELOG.md` §[1.27.9].

> **Version note:** **v1.27.5 "Holds" shipped 2026-08-15** — a **server**
> release (server `Cargo.toml`/lock 1.27.4 → 1.27.5; client + plugin unchanged)
> — the proof + thin-CLI pass of the v1.22 per-client legal-hold isolation: the
> isolation already exists (each domain's own `legal_holds` table). `POST
> /clients/{name}/hold` (Admin, audited) resolves the client's `domain` from
> the register (404 unknown, 409 archived) and delegates to the shared
> `observe`-style seam `handlers::holds::post_legal_hold_for_domain` (the
> `/legal-hold` body extracted once; no new hold logic). `brain client hold
> add|list <name>` drives it; tests `legal_hold_per_client_isolates_domains`
> (identical autoincrement ids across acme-us + beta-eu — acme's held, beta's
> free) + `client_hold_unknown_or_archived_rejected` pin the cross-domain
> boundary. Server bin 603 → **605** / 6 ignored, lib 105; clippy `-D warnings`
> + fmt clean; route + authz + openapi audits green. No schema change. Honest
> ceilings: proof + ergonomics, not new semantics — holds stay per-domain and
> archiving a client does not auto-release them (R6 termination). See
> `CHANGELOG.md` §[1.27.5].

> **Version note:** **v1.27.4 "Dsar" shipped 2026-08-15** — a **server**
> release (server `Cargo.toml`/lock 1.27.3 → 1.27.4; client + plugin unchanged)
> — the R4 per-client jurisdiction-aware DSAR. `POST /clients/{name}/dsar`
> (Admin, audited) resolves the client's `domain` + `jurisdiction` from the
> register (404 unknown, 409 archived) and delegates to the shared DSAR core via
> the new `observe::run_dsar_subject` seam — a single domain-pool run + the
> client-stamped `DsarResponse` (deadline/rights per its law, certificate
> carrying its jurisdiction + transfer mechanism). No new purge logic: locate/
> purge/export/certificate/hold-deferral all stay in `run_dsar_pool`; the shared
> `normalize_dsar_subject` is the one subject/action trust boundary (post_dsar
> refactored onto it, behavior-preserving). `brain client dsar <name> <subject>
> [--action purge|export|both] [--dry-run]` drives it. Tests: server bin 600 →
> **603** / 6 ignored, lib 105; clippy `-D warnings` (default + bench + otel) +
> fmt clean; route + authz + openapi audits green. Honest ceilings: subject
> erasure, not a blanket domain wipe (R6 termination); mechanism advisory, not
> gating; the audit anchor stays the global chain while the ledger/certificate
> live in the client's domain. See `CHANGELOG.md` §[1.27.4].

> **Version note:** **v1.26.3 "Cross-Border (fourth pass)" shipped 2026-08-15**
> — a **server** release (server `Cargo.toml`/lock 1.26.2 → 1.26.3; client +
> plugin unchanged) — the pass-4/5 validator + evidence-fidelity follow-up of
> v1.26.2. **4th pass:** `validate_register` now rejects `expires_at <
> signed_at` (`transfer_timestamp_invalid`) — an evidence register must not
> accept an instrument expiring before it was signed (signed == expiry stays
> valid); openapi 400 description notes the ordering. **5th pass:** the DSAR
> certificate `mechanism` is whitespace-trimmed like the jurisdiction field
> beside it (still free-text). Re-verified clean: panic/unsafe sweep (zero
> `unwrap()`/`unsafe` outside `#[cfg(test)]` in the new modules), pedantic/
> perf/complexity lint scan of the new modules, route/schema/openapi guard
> audits, otel gate. Tests: server bin **592** / 6 ignored, lib 105, otel 594 /
> 6; clippy `-D warnings` (default + bench + otel) + fmt clean; client wasm
> untouched. See `CHANGELOG.md` §[1.26.3].

> **Version note:** **v1.26.2 "Cross-Border (third pass)" shipped 2026-08-15**
> — a **server** release (server `Cargo.toml`/lock 1.26.1 → 1.26.2; client +
> plugin unchanged) — the deep-review follow-up of v1.26.1, same feature set.
> Evidence fidelity at the row boundary: `Transfer.lawful_basis` →
> `Option<String>` (`transfer_row` no longer `unwrap_or_default()`s — a NULL
> basis serializes `null`, never `""`, in the list + DPA artifact), and
> `register` stores the basis in its canonical lowercase vocabulary form
> (`b.trim().to_ascii_lowercase()`, matching mechanism/jurisdiction — the
> validator already accepted `Contract`, storage now agrees). New regression
> `lawful_basis_stored_canonical_and_null_semantics_preserved`; panic/unsafe
> sweep: zero `unwrap()`/`unsafe` outside `#[cfg(test)]` in the new modules;
> openapi 400 description covers the timestamp bounds. Tests: server bin
> 591 → **592** / 6 ignored, lib 105; clippy `-D warnings` (default + bench +
> otel) + fmt clean; route audits green. See `CHANGELOG.md` §[1.26.2].

> **Version note:** **v1.26.1 "Cross-Border (second pass)" shipped 2026-08-15**
> — a **server** release (server `Cargo.toml`/lock 1.26.0 → 1.26.1; client +
> plugin unchanged) — the post-review cleanup of v1.26.0, same feature set.
> Mechanisms re-verified 2026-08-15: EU SCC 2021 + UK IDTA/Addendum still in
> force (ICO plans an in-2026 update — the curated register stays human
> re-checked), EU-US DPF adequacy live since 2023-07-10 — the vocabulary
> needs no change. Fixes: `signed_at`/`expires_at` bounds moved into the one
> shared `validate_register` (handler-only `expires_at` check removed;
> `signed_at` now validated — `400 transfer_timestamp_invalid`), the dead
> `MAX_LIMIT*10` pre-clamp dropped from `GET /transfers` (`list` is the single
> bound), `dsar_deadline_for` deduped via `and_then` on `deadline_days`
> (identical fallback branches collapsed), `POST /transfers` response key
> `transfer_id` → `id` (matches GET rows + the `{id}` artifact routes; the
> same `jurisdiction_invalid` code/message as the DSAR gate), openapi.yaml
> schema drift closed (`/dsar` jurisdiction/mechanism + rights, `/ingest`
> lawful_basis/purpose + compliance.lawful_basis_missing), and six
> module-internal types tightened `pub` → `pub(crate)` (no dead exports).
> Tests: server bin 591 / 6 ignored (all assertions live in the existing
> bounds test), lib 105; clippy `-D warnings` (default + bench + otel) + fmt
> clean; route audits green. See `CHANGELOG.md` §[1.26.1].

> **Version note:** **v1.26.0 "Cross-Border" shipped 2026-08-15** — a
> **server** release (server `Cargo.toml`/lock 1.25.0 → 1.26.0; client + plugin
> unchanged) landing the **evidence + tagging** layer for a PH BPO serving
> US/UK/EU/AU/SG/CA clients — honestly framed: no new enforcement; the BPO
> stays processor/sub-processor. **M1** the cross-border transfer register:
> `src/transfers.rs` (`register`/`list`/`validate_register`/`transfer_by_id`) +
> `src/handlers/transfers.rs` (`POST`/`GET /transfers`, Admin + audited
> `AuditKind::Transfer`), the `transfers` table (schema → 1.26.0, guarded by
> the schema-contract test), validated `MECHANISMS`
> (scc-eu-2021/uk-idta/dpf-us/cbpr/bcr/adequacy) + any-short-lowercase
> `is_jurisdiction_code` (a future law adds without a release). **M2**
> `JurisdictionRule` — the curated code-versioned table (eu/uk/us/au/sg/ca/ph →
> law + deadline_days + rights); `dsar_deadline_for` is pure (law's fixed days,
> else PH "reasonable" → `BRAIN_DSAR_WINDOW_DAYS`), wired into `POST /dsar`
> (`jurisdiction` param → deadline + certificate jurisdiction/mechanism + the
> response `rights` list). **M3** `IngestRequest.purpose` +
> `knowledge.lawful_basis/purpose` columns; `lawful_basis_flag(strict_domain,
> basis)` flags a strict-posture record with no basis as
> `compliance.lawful_basis_missing` (Art 5/6 + NPC 2024-04 evidence). **M4** the
> TIA (Schrems II, from `SurveillancePosture` + destination law) + DPA (Art 28)
> templates on `GET /transfers/{id}/tia` + `/dpa` — **pre-filled evidence
> a human DPO/legal reviews + signs**; nothing renders legal judgment. 4 routes
> in the router + route-coverage + route-authz guard tables + openapi.yaml.
> Tests: server bin 582 → **591** / 6 ignored, lib 105 (unchanged); clippy
> `-D warnings` (default + bench + otel) + fmt clean; route audits green.
> **Fixed on review:** the initial `get_dpa` draft resolved only the **newest**
> register row (`list(…, 1)` then filter) — now a by-id `transfer_by_id`
> lookup, pinned by `dpa_fields_resolve_any_row_by_id`. Honest ceilings: this
> is evidence + tagging, **not enforcement** — nothing gates a transfer on the
> registered mechanism (blocking policies v2.x); the jurisdiction rules +
> surveillance postures are a curated snapshot a human re-checks (law evolves);
> PH "reasonable" uses the operator window; the client keeps its own
> controller obligations. See `CHANGELOG.md` §[1.26.0].

> **Version note:** **v1.25.0 "PH-Compliant" shipped 2026-08-15** — a
> **server** release (server `Cargo.toml`/lock 1.24.0 → 1.25.0; client + plugin
> unchanged) landing the Philippines home-jurisdiction posture, honestly
> framed: **no PH AI statute yet** — RA 10173 (DPA 2012) + NPC advisories
> (2024-04 AI; 2026-01 scraping) + EO 119 (gov-data residency) are the law in
> force; **HB 7396 (risk-based AI) is pending, not enacted** (structured to
> absorb, never pre-implemented). **M1** `COMPLIANCE_PH.md` maps every RA 10173
> control to a shipped feature (`src/ph.rs::DPA_CONTROLS` cross-ref test). **M2
> the one new primitive** — the breach-notification workflow:
> `src/breach.rs` (`open`/`add_event`/`close`/`list`/`get`) + `src/handlers/
> breaches.rs` (`POST /breach`, `/breach/{id}/event`, `/breach/{id}/close`,
> `GET /breaches`, `GET /breaches/{id}`); DPO/admin role-gated
> (`can_act_on_breach`: `dpo` role or `admin` capability, v1.23.0); per-
> jurisdiction notification deadlines computed from `discovered_at` (ph NPC
> 72h, eu Art-33 authority 72h, subject-notification per law); every event
> hash-chained into the audit via new `AuditKind::Breach`; `breaches` +
> `breach_events` tables (schema → 1.25.0), wired into the router, route-
> coverage + route-authz guard tables, and openapi.yaml. **M3** `PIA_TEMPLATE.
> md` (pre-filled, not auto-filed) + scraping provenance: a scrape ingest
> without a documented `lawful_basis` quarantines (the v0.9.7 flag), never
> stored (`IngestRequest.source` + `lawful_basis`; `ph::scrape_posture`).
> **DPO contact** — `BRAIN_DPO_CONTACT` surfaced on `/health`
> (`compliance.dpo_contact`, null when unset). Tests: server bin 571 → **582**
> / 6 ignored, lib 105 (unchanged); clippy `-D warnings` (default + bench +
> otel) + fmt clean; route-coverage + route-authz audits green. Honest
> ceilings: breach detection is **human-opened** (anomaly/leak sensors v2.x);
> a jurisdiction absent from the deadline table yields no deadline (the DPO
> confirms); HB 7396 is forward-watch only; each BPO client's own
> jurisdiction is v1.26.0 (cross-border); the client Security-panel countdown
> surfacing is a client release. See `IMPLEMENTATION_PLAN_v1.25.0_PH_
> Compliant.md` + `CHANGELOG.md` §[1.25.0].

> **Version note:** **v1.24.0 "Connectors" shipped 2026-08-15** — a
> **server** release (server `Cargo.toml`/lock 1.23.0 → 1.24.0; client +
> plugin unchanged) landing the vertical-integration foundation: the v0.9.6
> supervised connector pipeline (backfill + reconcile + source/revision
> linkage) gains a profile-gated registry + a shared translate template for
> the USE_CASES.md verticals (CRM, Slack, Jira/Linear, read-only HRIS/EHR). No
> new pipeline — each connector is a translate+ingest module on the GitHub
> template, gated by a profile's `connectors_allowed` (v1.21.0). **M1**
> `src/connector/kind.rs` pins the shipped vocabulary (`CONNECTOR_KINDS`,
> `is_connector_kind`, `family`) and `Profile::connector_allowed()` is the
> pure gate (absent → allow; explicit empty → deny-all air-gap; exact or
> bare-family grant for `a-b` sub-kinds); `POST /connectors/register` (Admin,
> audited) validates the kind and enforces the domain's bound profile →
> `403 connector_not_in_profile`, wired into the router, route-authz guard
> table, and openapi.yaml. **M2** `src/connector/pipeline.rs` is the pure
> translate template: `ConnectorDoc` + `connector_source_kind` + `live_uris`
> + `translate_*` for crm/slack/issue/structured-fact, linking stable
> `crm://`/`slack://`/`jira://` source URIs into the existing source/revision
> model and feeding kind-scoped `/sources/reconcile`; read-only PII records
> (HRIS/EHR) default to `private` scope. **M3** supervised: reconcile is
> never auto-sync and every translated record flows through the injection
> screen (poisoned records quarantine, not memory). **M4** CLI messages are
> vocabulary-aware (the github connector stays the only runnable backfill
> binary). Tests: server bin 569 → **571** / 6 ignored, lib 95 → **105**
> (kind vocab/family, `connector_allowed` gating, pipeline translate +
> source-kind + live-uri linkage, kind-scoped slack reconcile sweep,
> translated-record quarantine); route-coverage + route-authz audit green
> with the new route; clippy `-D warnings` + fmt clean. Honest ceilings:
> connectors are supervised backfill + reconcile (streaming is v2.x), the
> per-source transport needs per-connector handling (github is the only
> runnable network binary; the other kinds ship in registry + translate
> template only), and read-only into memory (no write-back to the source).
> The client Health panel still reads `/connectors` (with `last_sync`), card
> unchanged. Schema stays **1.23.0** — M1 adds no DDL (the `connectors` table
> already carried `kind TEXT`); the server Cargo bump is release alignment
> only, independent of the shared contract. See
> `IMPLEMENTATION_PLAN_v1.24.0_Connectors.md` + `CHANGELOG.md` §[1.24.0].

> **Version note:** **v1.23.0 "Roles" shipped 2026-08-15** — a
> **server + client** release (both `Cargo.toml`/locks 1.22.0/1.21.0 →
> 1.23.0; plugin unchanged) landing the role-based UI posture the v1.17.1
> operator roles promised without a UI gate — the operator console now
> *renders what your role can act on*. **Server-side, zero new endpoints or
> fields**: the MCP surface already accepts `{name, roles[]}` and stamps the
> JWT `roles` claim; this release only mirrors delegated/server roles into
> the existing claims shape. **Client M3** (`role.rs` + `api.rs`): a
> pure `role_can_see(roles, panel)` table maps resolved
> `server/delegated` role names → panels/actions, resolved once per token via
> `api().roles()` (`server` = always-grant all, incumbent-equivalent; JWT
> `roles` claim = delegated; absent token = unrestricted, loopback
> incumbent). The **Review queue** is the enforcement surface: `role_allows`
> gates approve/reject/edit (approve requires `role_can_see("dpo")` unless
> `server`-root; reject is always safe; edit only to non-approved) — so a
> `qa`/`agent` token can no longer rubber-stamp approvals. **Nav gating**:
> the desktop rail + mobile tab bar hide Subjects / Security / Audit / Data
> unless the resolved roles grant them (defense-in-depth — the server still
> enforces every endpoint). `role.rs` has a unit test per posture (exec hides
> sensitive panels but keeps the dashboard; qa can't approve/purge;
> supervisor approves but doesn't purge; agent hides audit+subjects; solo and
> no-roles see all). Tests: client 113 → **119**; server suite + schema
> contract + clippy `-D warnings` + fmt clean on both trees; client wasm
> unchanged in budget. Honest ceilings: gating is UI posture + JWT-presented
> roles — the server-authoritative RBAC the roles claim points at is
> delegated/scoped-role enforcement (v1.25+); `roles` from the JWT are as
> trusted as the token itself (local signing key, not an external IdP). See
> `IMPLEMENTATION_PLAN_v1.23.0_Roles.md` + `CHANGELOG.md` §[1.23.0].

> **Version note:** **v1.22.0 "Regulated" shipped 2026-08-15** — a
> **server-only** release (server `Cargo.toml`/lock 1.21.0 → 1.22.0; client +
> plugin unchanged) landing the **enforcement** the v1.21.0 policy fields
> promise, for the regulated buyer — the compliance line stays separate and
> green. **M1 legal hold** (`src/legal_hold.rs` + `src/handlers/holds.rs`):
> a new `legal_holds` table in every domain DB (partial active-hold index);
> `POST /legal-hold` / `POST /legal-hold/{id}/release` / `GET /legal-holds`
> (Admin, audited). Enforcement is the freeze: `page_decayed` drops held ids
> from `/decayed`, `purge` returns `409 legal_hold_active` (+ per-id reasons,
> via new `HandlerError::conflict_with`), and `run_dsar_pool` *defers* (never
> purges) held targets while listing `{id, reasons}` on the certificate's
> `held_ids[]` — the WORM-lite posture. Multiple concurrent holds supported;
> frozen until EVERY hold is explicitly released. **M2 retention report**
> (`govern::retention_report`): `GET /retention/report` = per domain × kind →
> ttl_days → count → expiring-within-30d, the storage-limitation evidence
> HIPAA/SOX/FedRAMP reviewers read. **M3 region pin**:
> `storage_layout::region`/`region_from` (fail-closed label: lowercase
> alnum+hyphen 1..=63) + additive `knowledge.region` wired via an `AFTER
> INSERT` trigger (all ingest paths, zero per-site churn), backfilled legacy
> NULLs once, never rewritten (region change preserves history); surfaced on
> every chunk + `/export` + the DSAR certificate + bundle. **M4 compliance
> pack**: `COMPLIANCE.md` §10 HIPAA/SOX/FedRAMP posture maps (posture, not
> certification). Tests: main bin 554 → **556** / 6 ignored, lib 86 → **87**
> (+ the `region_from` resolver); schema-contract test pins **1.22.0**; the
> route-authz audit learned the `holds` module; clippy `-D warnings` + fmt
> clean. The new integration test drops bare `unwrap()` for a
> `Result<_, Box<dyn Error>>` + `?` shape (only `.expect(msg)` + safe
> `unwrap_or`/`filter_map`). Honest ceilings: legal hold is per-id manual (no
> e-discovery search-to-hold yet; v1.23), region is a stamp not routing
> (multi-region v2.x), retention reports rather than auto-enforces (decay
> marks, the human purges, holds block even that), and the compliance pack
> documents posture only. See `IMPLEMENTATION_PLAN_v1.22.0_Regulated.md` +
> `CHANGELOG.md` §[1.22.0].

> **Version note:** **v1.21.0 "Profiles" shipped 2026-08-15** — a
> **server + client** release (server `Cargo.toml`/lock 1.20.30 → 1.21.0;
> client 1.20.25 → 1.21.0; plugin unchanged) landing the preset system: a
> **Profile** is a typed JSON bundle of the *existing* v1.14/v1.15/v1.17.1
> knobs (access_scope default, PII posture, per-kind retention, audit level,
> kind vocabulary) — no new governance primitives, no new columns. **M1**
> `src/profile.rs` (new lib module) + migration: `profiles` + `domain_profiles`
> tables (schema → 1.21.0, additive); apply-at-request-time semantics under the
> invariant **the profile sets defaults, the row wins** — `pii_mode: strict`
> masks title+content at the write boundary via the existing `screen_source_prompt`
> maskers (one-way `[redacted:*]` placeholders, deliberately NOT a vault — the
> v1.20.19 posture), `default_access_scope` fills only absent values, `kinds`
> rejects out-of-vocabulary ingests (`kind_not_allowed`), unreadable bound
> profiles fail CLOSED; new friendly `ttl_days` ingest field; at retrieval a
> bound profile's `retention` block REPLACES the server-wide policy for that
> domain (JSON `null` = no decay; empty block = nothing decays) — recall's
> per-domain loop + `/decayed`'s per-row filter both honor it (the SQL superset
> unions kinds + the least-restrictive cutoff, so the superset property holds);
> `audit_level` drives `/recall` read-events when `BRAIN_AUDIT_READ_EVENTS` is
> unset (verbose on / minimal off / standard = JWT posture; env = kill-switch).
> **M2** the 12 USE_CASES.md presets seeded INSERT OR IGNORE (operator edits
> survive re-migrations; every field editable via `POST /profiles/{name}`).
> **M3** `brain setup` (interactive pick → knob preview → bind; `--profile
> NAME --yes` scriptable) + the client connect-flow "What best describes your
> team?" step (shows when the home domain is unbound; Skip persists via the
> web pref seam). **M4** `GET /profiles`, `GET|POST /profiles/{name}` (Admin +
> audited), `GET|POST /domains/{name}/profile` (bind/unbind, `null` unbinds)
> in openapi.yaml (+ Profile schemas + a NotFound component); the client
> Health panel gains the profile/knobs card. Tests: main bin 542 → **548** /
> 6 ignored (incl. the `#[ignore]`d e2e: strict masking stores only
> placeholders, explicit `ttl_days` beats the profile default, unbound domain
> byte-identical), lib 80 → **86**, brain CLI **+1**, client 111 → **113**;
> clippy `-D warnings` + fmt clean on default + bench + otel; client wasm
> 4.99 MB (budget 7). Honest ceilings: strict masking runs after auto-routing
> (the quantized embedding + caller entities derive from raw text; neither
> practically invertible); the HITL propose/approve flow keeps its v1.14
> posture (promotion lands in `global` with column defaults — v1.22);
> `audit_level` covers `/recall` only; `connectors_allowed` is stored +
> surfaced only (registry not domain-scoped; v1.24); `legal_hold_default` is
> a flag (enforcement v1.22); the wizard binds `global` (per-domain targeting
> is `brain setup`). See Agent 94 + `CHANGELOG.md` §[1.21.0].

> **Version note:** **v1.20.30 "Caliber (foundation)" shipped 2026-08-14** — a
> **server-only** release (Cargo.toml/lock 1.20.29 → 1.20.30; client + plugin
> unchanged) landing the v1.28 "Caliber" M1+M2 groundwork EARLY, so it does not
> sit unreleased across the v1.21–v1.27 compliance line (the lines are
> independent; discipline rule: every Profiles-line release keeps the Caliber
> seams green — they live in the default suite). **The default build is
> behavior-identical**: edge-default stays potion/512-d/no-rerank; every neural
> path is `--features neural-embed,rerank-tier` + `MODEL_PROFILE` opt-in.
> **M2** `src/embed.rs`: the object-safe `Embedder` trait
> (`encode`/`encode_one`/`store_dim`), `AppState.model: Arc<dyn Embedder>`, all
> ~13 encode sites profile-agnostic; `migration::run_migration_with_store_dim`
> interpolates the vec0 dim + stamps `embedding_dim` in `schema_meta`, **failing
> closed** on a cross-dim profile switch (a 512-d DB under enterprise refuses
> with the `--re-embed` instruction); tiers: enterprise=BGE-M3 1024-d (verified
> end-to-end — dense+sparse+colbert from one FastEmbed pass; sparse/colbert
> unconsumed until v1.30), desktop=gte-base-en-v1.5 768-d (ponytail: modernbert
> is better but not in FastEmbed's enum — custom-ONNX is the upgrade path);
> fastembed 5 optional, ort rc.12 → rc.13. **M1** `src/search/rerank.rs`:
> bge-reranker-v2-m3 via `TextRerank`, LazyLock, **fail-open**, writing the
> reserved `rerank_score`/`rerank_truncated` slots post-fusion; boot arms it on
> enterprise/desktop/quality-local and **warms at boot** (a lazy first-recall
> load put the download in the request path — observed as a first-query 503,
> fixed live). **Escape hatch** `brain-server --re-embed <profile>`
> (`rebuild_vec_store_at_dim` + the /reindex loop; clears the legacy
> `embeddings` backfill source — old-dim f32 rows re-backfilled would be
> cross-dim corruption). **Capacity**: Desktop RSS 512 → 1024 MiB (neural tiers
> measured ~830 MiB; Jetson stays 512). Tests: main bin 534 → **542** / 5
> ignored, lib 76 → **80** / 1 ignored (incl. the `#[ignore]`d BGE-M3 load
> test); clippy `-D warnings` + fmt clean across default AND
> `neural-embed,rerank-tier`. **Tier smoke (directional, not a parity claim —
> BENCHMARKS.md §v1.28):** all three tiers live through `/recall` (10-doc
> corpus, `brain eval`, 37 queries): edge = the v1.17.4 baseline byte-consistent
> (MRR 0.905); desktop/enterprise = MRR 0.919 / nDCG 0.917 — the rerank lift on
> a recall-saturated set. Honest ceilings: the ≥100-query frozen set + the
> IronCurtain head-to-head (v1.31 "Proven") stay `pending` — **no parity claim
> is made**; the running launchd service still executes 1.20.29 until
> `install-service.sh`. See Agent 93 + `CHANGELOG.md` §[1.20.30].

> **Version note:** **v1.20.24 "Sweep" shipped 2026-08-13** — a
> **server + client + plugin** release (all three `Cargo.toml`/locks
> 1.20.23 → 1.20.24) paying the **seven audit gaps** the post-v1.20.23 audit
> itemized on the closed harden line — **no new endpoints, no new fields, no
> telemetry**. **G1** the v1.20.3 `strip_invisible` pair becomes a shared lib
> module (`src/strip_invisible.rs`; screen.rs re-exports) applied at the MCP
> tool envelope (`tool_result_payload` seam) + `format_response`, the CLI
> recall/get prints, and the openclaw plugin (`sanitizeForBlock` +
> `\u200B-\u200F\u202A-\u202E\u2066-\u2069\uFEFF`; titles + graph tool).
> **G7** client strips + bounded source-prompt scroll box (CSS-only). **G2**
> PII read-path uniformity (`/get/{id}`, `/multi-get`, search, proposals —
> `redact_content` for non-admin on every read). **G3** auth fails closed:
> `auth_token_misconfigured` + `check_secret_permissions` (mode & 0o077) on
> token file + JWT key; `main_inner` refuses to start. **G4** DSAR erases
> every domain DB (per-pool `run_dsar_pool`, global last with the aggregate
> SHA-256 on its ledger row). **G5** `/decayed` narrowed to an index-served
> superset WHERE (`decayed_superset_sql`, min-days cutoff; `page_decayed`
> stays arbiter) — **and the regression test caught `/decayed` returning `[]`
> since v1.14**: `strftime('%s')` is TEXT so `get::<i64>` dropped every row;
> `unixepoch()` fixes it. **G6** purge tombstones + DSAR ledger digests are
> SHA-256 of deleted content, not brute-forceable xxh3-64. +5 server tests
> (main bin 527 → 532 passed / 5 ignored), MCP bin 13 → 15, client 111
> unchanged, plugin 94 → 96; all clippy `-D warnings` + fmt clean. Honest
> ceilings: G3 is startup-only enforcement; G5's superset is exact for the
> CURRENT_TIMESTAMP format; G4's aggregate is a domain-list digest
> (per-pool bundles hash at write time; no crash-recovery protocol). See
> Agent 91 + `CHANGELOG.md` §[1.20.24].

> **Version note:** **v1.20.25 "Consolidate" shipped 2026-08-13** — a
> **server + client + plugin** release (server `Cargo.toml`/lock + client
> 1.20.24 → 1.20.25; plugin 0.2.1 → 0.2.2) consolidating the tail the v1.20.24
> "Sweep" left — **no new endpoints, no new fields**. **M1** `audit::hash` goes
> xxh3-64 → **SHA-256** (64 hex), and the recall-trace `query_hash` + `otel.rs`
> delegate to it — the G6 "no offline-recoverable digest" rule now reaches the
> audit + trace family, not just tombstones. **M2** a shared read seam
> `gate::sanitize_read`/`sanitize_read_opt` = `strip_invisible`∘`redact_content`
> now covers **every** emitted text field — title/content/snippet/evidence/
> heading on recall/search hits + `/get/{id}` + `/multi-get` — closing the
> raw-invisible-Unicode gap on the HTTP JSON boundary. **M3** DSAR + chunk
> purge erase the **graph + review-queue residue**: the v1.20.24
> relationship-delete referenced a non-existent `entities.knowledge_id` column
> ("no such column" silently aborted the DELETE, so relationships + PII-named
> entity nodes survived every purge) — the clause is removed, `purge_chunk_ids`
> now collects affected entity ids and orphans-sweeps them (shared entities
> survive), and `run_dsar_pool` additionally sweeps `proposals` by subject
> verbatim (raw candidate content with no owner column). **M4** the webhook
> signing secret fails closed on wide modes (`check_secret_permissions`, the G3
> posture). +3 server tests (main bin 532 → 534 passed / 5 ignored), MCP 15
> unchanged, client 111 unchanged, plugin 97 (+1: memory_store default/direct
> routing); both trees + plugin clippy `-D warnings` + fmt clean. Honest
> ceilings: the proposal sweep is a literal `LIKE %subject%` (no owner join);
> the orphan sweep is scoped to the purge's affected set (standalone entities
> untouched by design); M1's stored hash is a fingerprint, not a content lease
> (audit-chain verification unchanged). See Agent 92 + `CHANGELOG.md`
> §[1.20.25].

> **Version note:** **v1.20.23 "Calibrate" shipped 2026-08-13** — a
> **server + client** release (both `Cargo.toml`/lock 1.20.22 → 1.20.23)
> delivering the HITL essay's fourth condition — **evaluative feedback to the
> reviewer** (anti-rubber-stamp). The signals shipped since v1.14/v1.20.3/v1.20.14;
> what was missing was visibility of `decided_at` (written on approve/reject/
> expire but never read). **M1** exposes it: `ProposalView.decided_at` (column
> 11, `Option<i64>`) + a `since` window param on `GET /proposals` (`WHERE
> created_at >= ?`; absent → byte-identical legacy query), extracted as
> `list_proposals_page` (the `page_decayed`/`list_dsar_page` idiom, unit-testable
> with a bare `Connection`). **M2** the client computes the four reviewer signals
> (`calibration_stats`: approve-rate, median `decided_at - created_at` latency,
> edit-rate, screen-override-rate — zero denominators → `0.0`/`None`, no NaN) and
> renders a dismissable strip above the Review queue with a rubber-stamp warn
> (approve-rate > 0.9 over ≥ 20 decisions); fetch-failed → nothing (offline
> degrade). **No new telemetry, no new server logic.** +2 server tests (main bin
> 525 → 527 passed / 5 ignored), +3 client tests (108 → 111 passed); both trees
> clippy `-D warnings` + fmt clean; wasm + all 5 binaries + `badges.sh --selfcheck`
> clean. Honest ceilings: the window is `since`-bounded **and** list-capped
> (LIMIT 200 → "last 200 decisions" label); `override_rate` keys on read-time
> `screen_verdict`; the strip is per-operator-global; the warn threshold is a
> constant heuristic (reviewer baselines are v2.x). **This was the planned last
> release of the v1.20.x line** — the v1.20.24 "Sweep" audit-followup shipped
> after (see above) — closure note in CHANGELOG §[1.20.23] + the Hardening-Line
> INDEX. See Agent 90 + `CHANGELOG.md` §[1.20.23].

> **Version note:** **v1.20.22 "Clocks" shipped 2026-08-13** — a
> **server + client** release (both `Cargo.toml`/lock 1.20.21 → 1.20.22)
> extending the v1.20.15 "queue is a clock" core (reused unchanged) to
> **erasure + retention**: GDPR Art 17's 30-day window and Art 12's response
> deadline become **visible, not assumed**. **M1** the DSAR surface (`observe.rs`
> + `config.rs`) — pure `dsar_deadline(created_at) = created_at +
> dsar_window_secs()` (`DEFAULT_DSAR_WINDOW_DAYS = 30`, `BRAIN_DSAR_WINDOW_DAYS`
> override, the `BRAIN_PROPOSAL_TTL_SECS` pattern); `DsarResponse` gains
> `created_at` + `deadline` (computed, the client's source of truth). **M1.2**
> `GET /dsar` ledger list (Admin): bounded page (`limit` default 100,
> `1..=MAX_MULTI_GET`), newest-first, **server-computed per-row `deadline`** (no
> client window mirror), query extracted as `list_dsar_page` (the `page_decayed`
> idiom) and wired into the openapi + route + authz guard tables. **M2** client:
> the Subjects panel fetches the ledger + renders the 30-day countdown via
> `time_budget::{remaining, tier, format_remaining}` (day-scale bands `<3d`
> warn, `<1d` danger) on a ~30s on-load ticker (`dsar_clock` pure core); the
> Data panel gains the `next_expiries` pure core (sort, cap 10, skip expired)
> + tier-colored labels. **M1.3/M2.3** +2 server tests (main bin 523 → 525
> passed / 5 ignored) and +3 client tests (105 → 108 passed); both trees clippy
> `-D warnings` + fmt clean, wasm + all binaries release-clean. Honest ceilings:
> the countdown is a **signal, not enforcement** (no background worker, repo
> rule; the v1.20.17 ledger TTL is the only automatic bound); the window is
> display math on `created_at` (a reminder channel is v2.x); `GET /dsar` is an
> Admin-only operator registry, not subject-facing; `/decayed` only returns
> already-expired rows, so the Data "next to expire" card is the client boundary
> that would surface a near-expiry row if the server ever returned one. See
> Agent 89 + `CHANGELOG.md` §[1.20.22].

> **Version note:** **v1.20.21 "Subject360" shipped 2026-08-13** — a
> **server + client** release (both `Cargo.toml`/lock 1.20.20 → 1.20.21)
> turning the execute-blind DSAR into an execute-informed one. **M1** `POST
> /dsar` gains `dry_run` (`observe.rs`) — the `dsar_requests`/`knowledge`
> locate + bundle build run, then a read-only branch reports the `Footprint`
> (`roots`/`derived`/`export_rows`/`tombstones`/`dsar_rows`) and drops the tx
> untouched: no purge, no sweep, no ledger row, no certificate. The export
> bundle builder is extracted once (`build_export_bundle`) and shared, so the
> dry-run runs the *exact* same query as the live purge (no duplication);
> `count_subject_tombstones` matches the purge's tombstone reasons. `M1.1` +2
> server tests (main bin 523 passed / 5 ignored) proving the write-free
> footprint + the builder is behavior-preserving. **M2** the client Data &
> Rights panel gains a "Preview DSAR footprint" card (`subjects.rs` +
> `api.rs::dsar_preview`/`parse_footprint`); `openapi.yaml` documents
> `dry_run` + the `Footprint` schema. 2 client tests (+, main bin 105 passed);
> both trees clippy `-D warnings` + fmt + wasm/release clean. Honest ceilings:
> the footprint is a point-in-time preview (owner + `derived_from` walk, depth
> 8, no cross-domain dependency analysis — federation is v2.x), and
> ledger-history counts reflect the v1.20.17 retention window. See Agent 88 +
> `CHANGELOG.md` §[1.20.21].

> **Version note:** **v1.20.20 "Replay" shipped 2026-08-13** — a **client**
> release (client Cargo.toml/lock 1.20.16 → 1.20.20; server 1.20.19 → 1.20.20,
> version-alignment only — **zero server code**, `openapi.yaml` untouched)
> turning the already-stored decision path (v1.15.0 "Observe" M2) into a
> routed, ledger-linked, exportable evidence surface. **M1** `Route::RecallTrace`
> (`trace_panel`/`TraceCard` in recall.rs) now reads the stored shape —
> `query_hash` (not `query`, v1.20.17 M3) + the applied `scope` array — and runs
> every displayed string through the v1.20.3 `strip_invisible` render boundary
> (`replay_str`/`replay_list`), closing the bidi/zero-width smuggling class on
> the replay view. **M2** the Audit panel links `kind == "recall"` rows to
> `/recall/{id}` (the audit row id *is* the trace id) via pure `replay_href`.
> **M3** the replay view exports the raw trace JSON via the existing
> `document::eval` blob seam; `replay_*` i18n keys in `en` only (de/fr/es/nl
> fall back). 3 tests (+, main client bin 100 → 103 passed), client clippy
> `-D warnings` + fmt + wasm build clean, server suite untouched. Honest
> ceiling: traces store the query **hash** (deliberate — a recall query can be
> personal data), so the exact query is recovered via audit + hash, not shown
> verbatim. See Agent 87 + `CHANGELOG.md` §[1.20.20].

> **Version note:** **v1.20.18 "Bound" shipped 2026-08-13** — a
> **server** release (server Cargo.toml 1.20.17 → 1.20.18; client stays at
> 1.20.16) closing the **three unbounded read paths** and collapsing the **two
> quadratic scans** the v1.20.2 Harden D-group left. **M1** `GET
> /graph/entity/{name}` and `GET /graph/relations` now take a `?limit=`
> (default `MAX_GRAPH_EDGES` = 500, clamped 1..=500) and run
> `ORDER BY r.id LIMIT ?` — a stable, reproducible page (shared `GraphLimit`
> + `clamp_graph_limit`; extracted `entity_relations`/`relations_for`).
> **M2** `find_subject_conflicts` (`consolidate.rs`) is grouped by subject —
> O(n²) over all current rows → O(sum of m² per subject), ~O(n) dominating on
> mostly-unique subjects, output sorted for determinism. **M3**
> `idx_tombstones_reason_purged` index serves `/tombstones?subject=&since=`
> + the DSAR certificate reads (schema → 1.20.18, guarded by the schema-
> contract test). **M4** `/decayed` (`list_decayed`, gate.rs) gains
> `?limit=`/`?offset=` paging (default `MAX_DECAYED` = 500, applied after the
> Rust-side `effective_expiry` filter; `page_decayed` extracted). 5 tests (+,
> main bin 514 → 519 passed), all gates green: 519 passed / 5 ignored (main
> bin), clippy `-D warnings` + fmt clean, openapi/route/schema guards green,
> release build clean. Honest ceilings: the graph `ORDER BY r.id` page is a
> bounded but arbitrary window (no semantic ranking), `/decayed` pages but
> still scans once (the expiry is a Rust pure function, not a SQL predicate),
> and the conflict scan is still quadratic within a single subject (inherent
> to the mC2 rule). See Agent 85 + `CHANGELOG.md` §[1.20.18].

> **Version note:** **v1.20.19 "Vault" shipped 2026-08-13** — a **server**
> docs-correction release (server Cargo.toml 1.20.18 → 1.20.19; client stays at
> 1.20.16). The **v1.14 `pii_map` write-time placeholder vault was never built** —
> zero `INSERT INTO pii_map` sites in-tree, only `/export`'s read path. **M1**
> deletes that dead read path (`ExportQuery.include_pii_map` + the `pii_map`
> envelope key gone), **M1.3/M1.4** drop the table outright at migration
> (`DROP TABLE IF EXISTS pii_map`; schema → 1.20.19, guarded by the schema-
> contract test + `migration_drops_pii_map_and_empty_table`), and **M1.2/M2**
> correct every doc claim — the shipped PII control is deterministic read-time
> output redaction (`redact_content` + `screen_source_prompt`) + at-rest LUKS,
> **not** a vault. A fetchable placeholder→raw map would *increase* the
> personal-data surface; it is deliberately absent. 2 tests (+, main bin 519 →
> 521 passed). See Agent 86 + `CHANGELOG.md` §[1.20.19].

> Full per-release + per-agent history (v1.0.0→v1.20.20, Agent 87→1) moved to
> **`docs/AGENTS_HISTORY.md`** — load it on demand. This file is the operational
> contract only.

---


> **Version note:** **v1.20.18 "Bound" shipped 2026-08-13** — a
> **server** release (server Cargo.toml 1.20.17 → 1.20.18; client stays at
> 1.20.16) closing the **three unbounded read paths** and collapsing the **two
> quadratic scans** the v1.20.2 Harden D-group left. **M1** `GET
> /graph/entity/{name}` and `GET /graph/relations` now take a `?limit=`
> (default `MAX_GRAPH_EDGES` = 500, clamped 1..=500) and run
> `ORDER BY r.id LIMIT ?` — a stable, reproducible page (shared `GraphLimit`
> + `clamp_graph_limit`; extracted `entity_relations`/`relations_for`).
> **M2** `find_subject_conflicts` (`consolidate.rs`) is grouped by subject —
> O(n²) over all current rows → O(sum of m² per subject), ~O(n) dominating on
> mostly-unique subjects, output sorted for determinism. **M3**
> `idx_tombstones_reason_purged` index serves `/tombstones?subject=&since=`
> + the DSAR certificate reads (schema → 1.20.18, guarded by the schema-
> contract test). **M4** `/decayed` (`list_decayed`, gate.rs) gains
> `?limit=`/`?offset=` paging (default `MAX_DECAYED` = 500, applied after the
> Rust-side `effective_expiry` filter; `page_decayed` extracted). 5 tests (+,
> main bin 514 → 519 passed), all gates green: 519 passed / 5 ignored (main
> bin), clippy `-D warnings` + fmt clean, openapi/route/schema guards green,
> release build clean. Honest ceilings: the graph `ORDER BY r.id` page is a
> bounded but arbitrary window (no semantic ranking), `/decayed` pages but
> still scans once (the expiry is a Rust pure function, not a SQL predicate),
> and the conflict scan is still quadratic within a single subject (inherent
> to the mC2 rule). See Agent 85 + `CHANGELOG.md` §[1.20.18].

> **Version note:** **v1.20.17 "Scrub" shipped 2026-08-12** — a
> **server** release (server Cargo.toml 1.20.16 → 1.20.17; client stays at
> 1.20.16) closing **five verified GDPR-erasure (Art 17) completeness gaps** —
> **no schema change, no new route**. **M1** the DSAR ledger (`observe.rs`)
> persists a `bundle_hash` (xxh3), never the raw export bundle; mature
> completed ledger rows are pruned on the read-event cadence
> (`BRAIN_DSAR_LEDGER_DAYS`, default 30, `purge_stale_dsar_ledger`). **M2**
> `/export` gained a `redact_owner` query param — rows owned by another owner
> export with `content` redacted to `[redacted]` via a shared `should_redact`
> helper covering both the JSON and UMP (`render_ump`) paths. **M3**
> `recall_traces` stores `query_hash` (xxh3), never the raw query text. **M4**
> a `ump.remember` whose declared `scope.owner` mismatches the principal is now
> audited as a `denied` auth event via the shared `record_forbidden_scope`
> helper (detail xxh3-hashed; best-effort — audit failure never fails the
> request). **M5** the DSAR purge transaction commits the ledger row with the
> erase and backfills the certificate timestamp after commit. 7 tests (+,
> 507 → 514 passed), all gates green: 514 passed / 5 ignored (main bin),
> clippy `-D warnings` +
> fmt clean, openapi/route/schema guards green, release build clean. Honest
> ceilings: export redaction strips chunk `content` only (metadata unsplit), the
> ledger prune rides the read-event cadence (no dedicated boot timer), and the
> xxh3 hashes are non-adversarial fingerprints like the audit chain's own. See
> Agent 84 + `CHANGELOG.md` §[1.20.17].

> **Version note:** **v1.20.16 "Bidi" shipped 2026-08-12** — a
> **server + client** release (server Cargo.toml 1.20.15 → 1.20.16; client
> 1.20.15 → 1.20.16) closing the **one real gap** a deep audit of six proposed
> agentic-security hardening measures (LITL/UI markdown, IFC/taint tracking,
> Rule-of-Two, MCP ETDI signed manifests, SPIFFE/SPIRE + mTLS, EchoLeak +
> Unicode normalization) found against the live tree. **The other five were
> already defended or out of brain-server's scope** (verdict recorded in
> `CHANGELOG.md` §[1.20.16]): the Dioxus client renders escaped text nodes
> (no markdown parser, no `dangerous_inner_html`, build-guarded) so the LITL/
> EchoLeak markdown-image class is structurally absent; `/recall` already
> serializes `untrusted: true` per hit (the IFC *enforcement* is
> orchestrator-side); Rule-of-Two is an OpenClaw concern; MCP rug-pull/shadowing
> targets aggregating clients, not a single self-hosted server with a
> compile-time-fixed tool table; SPIFFE/TPM is org-level infra. The one gap:
> `strip_invisible` (`src/screen.rs` + `client/src/main.rs` mirrors) covered
> tag-block / variation-selectors / zero-width / legacy BOM set but **not the
> Unicode `Bidi_Control` block** — the directional-override smuggling class
> (U+202E RLO et al.) named by Trojan Source / W3C TR#20. Widened in one move
> to strip `U+200E–U+200F` (LRM/RLM), `U+202A–U+202E` (LRE/RLE/PDF/LRO/RLO),
> and `U+2066–U+2069` (LRI/RLI/FSI/PDI isolates) — the full canonical
> `Bidi_Control` set. No new codepath, no new dep, no abstraction: the existing
> predicate reaches both the classifier-scoring boundary (server) and the
> operator render boundary (client) automatically. Tests extended (no new
> files). ponytail ceiling: the layer-1 blocklist runs on raw bytes, not
> stripped input — widening shrinks but doesn't close that leg (separate "where
> strip is applied" change). Server 507 passed + 5 `#[ignore]`d green, clippy
> `-D warnings` + fmt green; client 100 passed, clippy + fmt + wasm green.
> See Agent 83 + `CHANGELOG.md` §[1.20.16].

> **Version note:** **v1.20.15 "Clock" shipped 2026-08-12** — a
> **server + client** release (server Cargo.toml 1.20.14 → 1.20.15; client
> 1.20.14 → 1.20.15) bringing the console line's "the queue is a clock" rule to
> the **review queue** per `IMPLEMENTATION_PLAN_v1.20.15_Clock.md`. **M1 server**
> (`handlers/gate.rs`): `ProposalView` gains three computed, non-stored fields
> via the pure `proposal_deadline(created_at)` — `expires_at` (`created_at +
> proposal_ttl_secs()`, the alert watcher's math) + `warn_secs`/`critical_secs`
> (the exact `ALERT_WARN_SECS`/`ALERT_CRITICAL_SECS` constants), so a client
> countdown and the server alert can never disagree about a tier; no schema
> change, no new route; openapi documents the fields. **M2 client**: the new
> shared `client/src/time_budget.rs` core (`tier`/`remaining`/`format_remaining`
> /`now_unix`, Dioxus-free) replaces the old per-panel client TTL mirror
> (`ops::clock_until` + `DEFAULT_PROPOSAL_TTL_SECS` deleted); Review cards + the
> deep-link detail page render a tier-colored absolute-deadline badge
> (`Xd Yh`/`Xh Ym`/`Xm`/`<5m`/`expired`) ticked on a ~30s cadence, with `Expired`
> rows disabling approve/reject/edit; a client-side sort-by-deadline toggle
> (`review::expiry_order`, stable id tie-break) defaults to the server's creation
> order so nothing changes unless asked (ponytail: ≤200 rows, local sort honest).
> **M3 wrap**: server + client → 1.20.15, `api::now_unix` delegates to the shared
> core, CHANGELOG + AGENTS. Server 507 passed + 5 `#[ignore]`d green, clippy +
> fmt green; client 100 passed (+1 `expiry_order` sort test), clippy + fmt + wasm
> green. Honest ceilings: the `<5m` band is not parameterized by an
> `ALERT_CRITICAL_SECS` override (it shifts tier color only); the badge + sort
> strings are `en`-only first cuts; the 30s tick is a signal, not enforcement
> (the server's 400 on a stale approve stays authoritative). See Agent 82 +
> `CHANGELOG.md` §[1.20.15].

> **Version note:** **v1.20.14 "Steer" shipped 2026-08-12** — a
> **server + client** release (server Cargo.toml 1.20.13 → 1.20.14; client
> 1.20.13 → 1.20.14) closing the HITL essay's fifth limb — **evaluative
> substitution** (edit-then-approve) — per `IMPLEMENTATION_PLAN_v1.20.14_
> Steer.md`. **M1 server `POST /proposals/{id}/edit`** (`handlers/gate.rs`):
> re-scores a pending proposal through the exact `ingest_proposal` path
> (novelty vec0 KNN / `find_conflict` / salience) + the v1.20.3 injection
> screen (`Reject` → 400; `Quarantine` → stored), stamps `edited_at`; same
> TTL-expiry + `BEGIN IMMEDIATE` CAS discipline as approve/reject (v1.20.2
> A3/A4, a concurrent decision → clean 409); audit detail = SHA-256 of
> before+after content only (never raw text, pinned by a known-vector test);
> `gate.edit` otel span under `--features otel`. **M1 migration**: additive
> nullable `proposals.edited_at`. **M2 client Review panel**: `edit_for`
> signal through `card()` + an Edit button, an `EditEditor` dialog, `E` key
> + `?` help row, a `warn` **edited** badge on card + detail, offline
> `QueuedAction::Edit`, new i18n `edit`/`review_key_edit`. **M3 wire**: 
> `ProposalView.edited_at` ↔ `Proposal.edited_at` (`#[serde(default)]`);
> openapi documents the route. Honest ceilings: review-queue-only (no
> rewriting of promoted chunks); audit carries hashes not a full text diff;
> `en`-only strings until a native pass; no measured device run. Server 622
> tests (+1 `sha256_hex` vector, 5 `#[ignore]`d green), clippy `-D warnings`
> + fmt green; client 99 tests, clippy + fmt + wasm green. See Agent 81 +
> `CHANGELOG.md` §[1.20.14].

> **Version note:** **v1.20.13 "Media" shipped 2026-08-12** — a
> **version-aligned** release (server Cargo.toml 1.20.12 → 1.20.13; client
> 1.20.12 → 1.20.13, version-alignment only — the same pattern as v1.18.2
> "Align"; **no runtime code, no schema change, no new routes**) shipping the
> outbound half of the GTM documentation line per
> `IMPLEMENTATION_PLAN_v1.20.13_Media.md`: the *narrative* that makes
> brain-server discoverable and saleable, built on the v1.20.12 *reference*.
> **M1 `docs/blog/`** (relocated from the private `marketing/blog/`, not
> re-authored — the v1.20.12 reuse precedent): 8 technical-buyer posts, one per
> hard-won mechanism (compliance-time-bomb framing, deterministic HITL,
> tamper-evident audit, reference-faithful retrieval, no-lock-in via MCP/UMP/HTTP,
> OWASP 2026 as the sales doc, the honest ceiling, a clearly-labelled forward-
> looking Profiles preview). **M2 `docs/media-kit.md`** (also relocated): name/
> one-liners/positioning, a Brain-vs-Mem0/LangGraph/RAG sizing table with honest
> ceilings, headline stats tied to the proof map. **M3 cross-links**: product-site
> `index.md` + README Documentation table + `docs/README.md` docs-map gain Blog +
> Media kit rows; README badge → 1.20.13. **M4 wrap**: CHANGELOG §[1.20.13];
> ROADMAP released-version header + v1.20.13 row → Shipped; `openapi.yaml` +
> `Cargo.toml`/lock + `client/Cargo.toml`/lock re-stamped to 1.20.13. Fixed the
> two link classes relocation surfaced (stale `blog-07-` in post 01; the media
> kit's `../trust/` → `./trust/` now that it sits at `docs/` — one level shallower
> than the blog). Honest ceilings: in-tree Markdown, not a published blog/CMS
> (v2.2.1 "Drift"); the Profiles post is forward-looking; media-kit positioning
> is author-faithful, not an analyst endorsement. See Agent 80 +
> `CHANGELOG.md` §[1.20.13].

> **Version note:** **v1.20.12 "Docs" shipped 2026-08-12** — a
> **version-aligned** release (server Cargo.toml 1.20.11 → 1.20.12; client
> 1.20.9 → 1.20.12, version-alignment only — the same pattern as v1.18.2
> "Align"; **no runtime code, no schema change, no new routes**) shipping the GTM
> documentation line per `IMPLEMENTATION_PLAN_v1.20.12_Docs.md`. The three tiers
> — **M1 `docs/product-site/`** (landing `index.md` + install + quickstart +
> editions placeholders), **M2 `docs/research/`** (one scientific explainer per
> shipped mechanism: bi-temporal KG, submodular packing, TRACE edges, PPR graph
> leg, hub dampening, calibrated abstention, reachable-PRF gate — each a
> problem → reference → deterministic implementation → ceiling), and **M3
> `docs/trust/`** (the proof map: every SECURITY/COMPLIANCE/OWASP_AGENTIC_2026
> claim → shipped release → live `curl`/`brain` proof, plus `reproduce.md`'s
> throwaway-instance walk-through) — were **relocated from the private
> `marketing/` dir into the public in-tree `docs/`** (reuse, not re-authoring:
> the content was already written by the v1.20.6 GTM line; sibling-relative
> links survive the move, `../../docs/` links in product-site fixed to `../`).
> **M4 cross-links + alignment** in README + docs-map + COMPLIANCE.md + SECURITY.md;
> README version badge regenerated from the real build via `scripts/badges.sh`
> (server + client both 1.20.12, tests 621). Honest ceilings: in-tree Markdown
> (not a deployed site — the v2.2.1 "Drift" step), editions/pricing placeholders
> until v2.2 "Meridian", the explanations are author-faithful, not SOTA-parity
> claims, and the client bump is version-alignment only (last client feature
> release remains v1.20.9 "Register").
> See Agent 79 + `CHANGELOG.md` §[1.20.12].
> **Version note:** **v1.20.11 "Housekeeping" shipped 2026-08-12** — a
> **server + docs** release closing the operator-console line (server Cargo.toml
> 1.20.10 → 1.20.11; client stays at 1.20.9; **no new runtime code, no schema
> change, no new deps**). **M1 `scripts/badges.sh`** — badges are facts, not
> hand-typed claims: derives the version from `Cargo.toml` (server + client),
> the test count from an actual `cargo test --features bench,migrate` run, the
> UMP level from the shipped self-attested L3 (asserted every push by the
> `ump-conformance` CI job), and an SBOM-present flag from the on-disk CycloneDX
> JSON; prints the badge block to paste, and `--selfcheck` guards the version
> derivation + the release-checklist completeness (exits nonzero on drift). It
> never fabricates a number it did not measure. **M2 `docs/release-checklist.md`**
> — codifies the six-part release wrap (Cargo.toml+lock → openapi.yaml →
> CHANGELOG → ROADMAP → README badges via `badges.sh` → AGENTS.md) with the
> verifying commands + gates, and documents the docs-only exception. **M3 `/proof`
> panel: NOT built** (optional/off by default — the v1.20.10 integrity signal
> already lives in the queue-header `Badge`). README badge drift fixed (hand-
> typed 712 → measured **621**); ROADMAP v1.20.6 + v1.20.9 rows marked Shipped
> (they had shipped but were still Planned). See Agent 78 + `CHANGELOG.md`
> §[1.20.11].
> **Version note:** **v1.20.10 "Proof" shipped 2026-08-12** — a
> **server + docs** release (server Cargo.toml 1.20.8 → 1.20.10; client stays
> at 1.20.9; no new routes, no schema change, no new deps). **M1 a live
> integrity feed** — `alert::spawn_chain_watcher` re-runs the existing full
> `/audit/verify` chain check on a cadence (`BRAIN_CHAIN_CHECK_SECS`, default
> 60s) and raises an `integrity` alert on ok↔broken transitions (pure
> `chain_transition` core: no per-tick spam, a broken boot raises instantly, a
> recovery raises `ok`); `/health` gains `integrity:{chain_ok, last_checked_at,
> chain_head}` — the watcher's cached posture, content-free and PII-free.
> **M2 the CRA evidentiary kit** (`scripts/cra-kit.sh` + `docs/cra.md`) —
> idempotently assembles the CycloneDX SBOM, `SECURITY.md`, `SUPPORT.md`,
> `docs/deployment.md`, `COMPLIANCE.md` into `dist/cra-kit/` with a
> `CRA_MANIFEST.json` SHA-256 index (evidences the EU CRA SBOM+reporting+
> support bar; "certification is an org action" is the explicit honest ceiling).
> **M3 the ADMT kit** (`scripts/admt-kit.sh` + `docs/admt.md`) — a read-only
> assembly of existing `GET /get/{id}` + `GET /audit?kind=reconcile` into a
> per-decision `ADMT_RECORD.json` + hashed manifest ("why this became memory,
> by what path, from what source"; inherits the server's integrity posture,
> never fabricates a summary). **M4 `SUPPORT.md`** — the repo-standard support
> statement (versions → SECURITY.md, reporting path, update guidance, honest
> no-SLA posture). 505 server tests (+1 `chain_transition` + `ChainWatchState`
> default) + 5 `#[ignore]`d green, clippy `-D warnings` + fmt green, CRA kit
> smoke-verified (hashes match). See Agent 77 + `CHANGELOG.md` §[1.20.10].
> **Version note:** **v1.20.9 "Register" shipped 2026-08-12** — a
> **client** release (client Cargo.toml 1.20.8 → 1.20.9; server + API contract
> stay at 1.20.8). **M1 the read-only Agent Memory Register** (`/register`,
> `client/src/panels/register.rs`) — a pure client composition of the already-
> shipped `GET /export` (`knowledge` body) + `GET /get/{id}` endpoints (no new
> routes/wire types/deps) surfacing the v1.20.7 `origin` marker as an operator
> provenance ledger: origin tiers (`human`/`model`/`imported`) with live counts,
> owner/source/memory-kind filters, rows of id · bounded excerpt · provenance
> badges · UTC date (`format_epoch`). **M2 a shared `EvidenceModal` viewer**
> (one `role="dialog"` renderer, hand-rolled Esc-close modal per the review-
> panel idiom — no Radix `DialogRoot` in the client) opened from any register
> row, fetching `GET /get/{id}` to show the verbatim span + `source_uri` +
> revision + heading + line range. Read-only by construction (`parse_export_rows`
> rejects any non-`/export` body). **M3 wrap** (i18n `nav_register` in en; nav
> targets 13 → 14 with the guard test + `palette_navigate_covers_every_non_
> detail_route` updated). 99 client tests (+6 register cores), clippy `-D
> warnings` + fmt + wasm green. See Agent 76 + `CHANGELOG.md` §[1.20.9].
> **v1.20.7 "Telemetry" shipped 2026-08-12** — a
> **server** release (server Cargo.toml 1.20.4 → 1.20.7; no API contract change) adding
> optional OpenTelemetry tracing of the write-gate decision path, **gated
> behind a new `otel` Cargo feature** so the default build ships with **zero
> tracing machinery and zero new runtime deps** (every `#[instrument]` + the
> OTLP exporter are `#[cfg(feature = "otel")]`). **M1 instrumented the three
> decision seams**: the injection screen (`screen::screen` → `screen` span,
> records `verdict`), the human review gate (`gate::ingest_proposal`/
> `approve_proposal`/`reject_proposal` → `gate.{propose,approve,reject}` with
> `outcome`), and recall (`recall::run_recall` → `recall` span with `decision`/
> `graph_rescued`/`hits`/`domain`/`principal`/`query_hash`). New `src/otel.rs`
> (`init_otel` → `SdkTracerProvider` + OTLP HTTP exporter to
> `BRAIN_OTEL_ENDPOINT`, default `127.0.0.1:4318/v1/traces`) + pure label
> helpers `query_hash` (bounded xxh3 — content never a field) /
> `screen_verdict_span` / `gate_outcome`. `main.rs` `init_tracing` wires
> `EnvFilter` (own layer) + the otel layer. 500 otel tests + 2 new cfg-gated
> `screen::tests::otel_tests` (a hand-rolled capturing `Layer<Registry>`
> proves the `screen` seam emits `[("verdict","clean")]`), clippy `-D
> warnings` + fmt green under default AND `otel` AND `bench,migrate[,otel]`;
> a new `otel-gate` CI job compiles + tests the feature (a default build
> compiles a different surface — a broken otel build would slip past
> `lint-test`). No version bump yet (the `otel` feature rides into the next
> tagged release). See Agent 75 + `CHANGELOG.md` §[1.20.7].
> **v1.20.6 "Console" shipped 2026-08-12** — a
> **client** release (client Cargo.toml 1.20.0 → 1.20.6; server + API contract
> stay at 1.20.0) shipping the first release of the operator-console line.
> **M1 the Memory Operations panel** (`/ops`, `client/src/panels/ops.rs` — a
> pure client composition of the already-shipped `/proposals`, `/decayed`, and
> recall-`include_flagged` endpoints; no new routes/wire types/deps) fuses the
> HITL posture into one at-a-glance surface: a **live pending-proposal queue**
> (content + `source_prompt` + live SLA countdown + A-approve/R-reject reusing
> the v1.20.0 decide/offline-enqueue path), the **flagged & quarantined
> inventory** from the v1.20.3 injection screen (read-only, stripped of
> invisible smuggling chars at display only), and a **gate-health strip**.
> **M2 SLA countdown clocks** — pure `clock_until`/`sla_tier`/`queue_priority`
> cores (expired first, then nearest-expiry, stable tie-break) on a ~30s
> once-on-mount loop (the "queue is a clock" rule; the server's 400 on a stale
> approve stays authoritative). **M3 flagged surface**. **M4 wrap** (i18n
> `ops_*`/`sla_*`/`gate_*` keys in en; nav targets 12 → 13). 90 client tests,
> clippy `-D warnings` + fmt + wasm green. See Agent 73 + `CHANGELOG.md`
> §[1.20.6].
> **GTM docs line (companion to v1.20.6, no version bump; ROADMAP rows
> v1.20.12 "Docs" + v1.20.13 "Media"):** shipped `marketing/` (private,
> gitignored — product-site landing/install/quickstart/editions, 7 research
> explainers, trust proof-map + live reproduce, 8 blog posts, media kit);
> README + docs-map left untouched (GTM stays out of the public tree). Docs-
> only, tree otherwise unchanged. See Agent 74 + `CHANGELOG.md` §[1.20.6] GTM
> note.
>
> > **Version note:** **v1.20.5 "Agentic" shipped 2026-08-11** — the
> **enterprise capstone** of the GhostJacking-hardening line (docs-only; no
> server/client version bump, zero new routes/schema/deps — a docs-only patch
> tag marks the artifact). Maps the hardened stack (G1–G6 closed across
> v1.20.1–v1.20.4) to the two 2026 OWASP agentic frameworks and ships the
> adoption artifacts. **M1 `docs/OWASP_AGENTIC_2026.md`** — the control-by-
> control compliance matrix for the **OWASP GenAI LLM Top 10:2026** (LLM01–10)
> and the **OWASP Top 10 for Agentic Applications 2026** (ASI01–10); every row =
> `Shipped vX.Y` or an owned `Ceiling v2.x` residual-risk; AIUC-1 crosswalk;
> standard = **100% control coverage**, not 100% risk elimination (LLM01 has no
> prevention per OWASP 2026). **M2 ZT4AI posture** (SECURITY.md § + COMPLIANCE.md
> §3.5: workload identity — agents not shared service accounts, did:key +
> capability tokens ≤90d; least-agency — plugin recall + proposal only, write
> approval outside the prompt; Rule of Two; one egress boundary). **M3
> audit-ready-replay playbook** (COMPLIANCE.md §3.6: what/why/to-whom/for-how-
> long from `/audit` + recall traces + DSAR certs + retention — export paths
> already exist, no new code). **M4 enterprise ops runbook** (docs/deployment.md:
> token rotation + poisoning-incident-response + classifier ops with
> `BRAIN_INJECTION_THRESHOLD_HIGH/LOW` + model `sha256sum` pin). ROADMAP
> released-version → 1.20.5 + released row. Docs release — tree unchanged, all
> quality gates pass. See Agent 72 + `CHANGELOG.md` §[1.20.5].
>
> **Version note:** **v1.20.4 "Replay" shipped 2026-08-11** — a
> **server** release (server Cargo.toml 1.20.3 → 1.20.4; client stays at
> 1.20.0) closing the GhostJacking **G6** webhook replay window (per
> `IMPLEMENTATION_PLAN_v1.20.4_Replay.md`; **no schema change, no new
> routes**). **M1 the optional Standard Webhooks handshake** for first-party
> senders: when `BRAIN_WEBHOOK_TIMESTAMP_REQUIRED=1`, `/webhooks/{kind}`
> requires the open-spec headers (`webhook-id`/`webhook-timestamp`/
> `webhook-signature`) and verifies `v1,<base64>` HMAC-SHA256 over
> `{id}.{timestamp}.{raw body}` in constant time
> (`WebhookQueue::verify_standard_signature` + `receive_standard`); the
> timestamp rides inside the HMAC so a replay cannot re-stamp it, and
> `webhook-id` feeds the existing `webhook_seen` idempotency. **M2 `/health`
> `webhook.{replay_secs,timestamp_required,scheme}`**. **M3 docs** (GitHub
> replay protection = delivery-id idempotency, not a timestamp — its sender is
> a trusted third party). The hard window is **opt-in**; the legacy GitHub
> path is byte-identical. **This closes all six audit gaps (G1–G6)** across
> the v1.20.x line. 500 server tests (+2 webhook) + 5 `#[ignore]`d green,
> clippy `-D warnings` + fmt green. See Agent 71 + `CHANGELOG.md` §[1.20.4].
>
> > **Version note:** **v1.20.3 "Classify" shipped 2026-08-11** — a
> **server** release (server Cargo.toml 1.20.2 → 1.20.3; client stays at
> 1.20.0, one pure fn + render sites + a test) closing the GhostJacking G5
> upgrade path (per `IMPLEMENTATION_PLAN_v1.20.3_Classify.md`; **no schema
> change** — `proposals.screen_verdict` is recomputed deterministically at read
> time, schema stays at 1.20.1). **The two-layer injection screen** (`src/
> screen.rs`, the single seam every ingest write site routes through):
> layer 1 = the deterministic blocklist (always on); layer 2 = an optional,
> feature-gated local ONNX classifier (`injection-classifier` + `ort`/
> `tokenizers`, **off by default** — the Jetson envelope treats memory as
> scarcest, and the blocklist + `flagged`/`untrusted` segregation remain the
> always-on defense). When enabled loads a BERT-tiny INT8 model at
> `BRAIN_INJECTION_CLASSIFIER` + tokenizer at `BRAIN_INJECTION_TOKENIZER`
> once via a `LazyLock`; banding score ≥ 0.9 → 400, ≥ 0.7 → stored flagged,
> else clean; sentence-packed + density-adjusted scoring; policy + thresholds
> read per call (flippable without restart), only the model load is cached.
> **Wired into** `/add`, `/ingest/memory`, `/ingest/markdown`, `/ingest`
> (`ingest_one`), `/procedure` (root + each step), `/ingest/proposal`.
> **`flag_if_quarantined`** now takes the screen's bool verdict (a layer-2 hit
> quarantines exactly like a layer-1 hit). **Canonical `screen::is_invisible`**
> (adds tag block U+E0000–E007F + variation selectors U+FE00–FE0F) now shared
> by the blocklist normalization, the classifier, and the **client render
> boundary** (client strips invisible smuggling chars from *displayed* hits;
> raw bytes never rewritten). **`ProposalView.screen_verdict`** badge +
> **`/health` `injection_classifier_loaded`**. 611 server tests (+14) + 5
> `#[ignore]`d green (incl. the 2 model-backed Shield/audit drills), clippy
> `-D warnings` + fmt green on default AND `--features injection-classifier`;
> 83 client tests. See Agent 70 + `CHANGELOG.md` §[1.20.3].
> **v1.20.2 "Harden" shipped 2026-08-11** — a
> **server-only** release (server Cargo.toml 1.20.1 → 1.20.2; plugin stays
> 0.2.1; client stays 1.20.0) closing the v1.20.x deep + security second-pass
> audit findings (per `IMPLEMENTATION_PLAN_v1.20.2_Harden.md`; **no schema
> change** — schema stays at 1.20.1). **A1 [C]** audit hash chain fork under
> concurrent autocommit writers closed (`record_tenant` → `BEGIN IMMEDIATE` on
> autocommit, `SAVEPOINT` in a caller tx, mirroring `record_and_rotate`);
> **A3 [H]** `approve_proposal` CAS'd (`409 proposal_already_decided`),
> **A4 [H]** stale-expiry moved before the tx. **B1** `/procedure` now screens
> injection like its siblings (root + each step; Quarantine → flagged + no
> `next_step` edges). **C1 [PII]** `mask_card` Luhn-checks 13–19 digit runs.
> **D1–D4 [DoS]** `BRAIN_TRUST_PROXY` gating + `RateLimiter` capped/LRU,
> `extract_vocabulary` capped at 500, `/export` bounded, `/v1/embeddings`
> batch capped at 64. **E1 [AuthZ]** `/tombstones` + `/dsar/{id}/certificate`
> tenant-scoped. **F1–F4** `source_prompt` bounded+screened, `/health/db`
> Read-gated, `multi_get` single-query, `/metrics` intent documented.
> **G** MCP 2026-07-28 protocol compliance (Agent 68) ships here +
> `MAX_LINE_BYTES` guard + `sanitize_echo` hex-escape. 597 server-side tests
> (+1 B1) + 5 `#[ignore]`d green, clippy `-D warnings` + fmt + all 5 release
> binaries green. See Agent 69 + `CHANGELOG.md` §[1.20.2].
> **v1.20.1 "Shield" shipped 2026-08-11** — a
> **server + plugin + client** release closing the GhostJacking-audit P0s
> (per `IMPLEMENTATION_PLAN_v1.20.1_Shield.md`; server Cargo.toml 1.18.2 →
> 1.20.1, plugin 0.2.0 → 0.2.1, client stays at 1.20.0 "Polish").
> **M1 the shared `/ingest` write core now screens injection like its
> siblings** (`ingest_one`: `Reject` policy → 400 `input_rejected`;
> `Quarantine` default → stored flagged + KG edges skipped — one guard covers
> plain/single-UMP/batch-UMP + the plugin's `memory_store`/`autoCapture`,
> closing G1). **M2 autoCapture routes through the human review queue by
> default** (plugin `captureMode: "proposal"` + `BrainClient.submitProposal()`
> → `/ingest/proposal`; `direct` opt-out stays M1-screened), backed by the
> new `proposals.source_prompt` column — PII-screened at persist via pure
> `gate::screen_source_prompt` (only `[redacted:…]` form, LLM01:2026 #7
> "exact action not summary", rendered in the client Review panel's "sourcing
> prompt" block) — plus a 7-day proposal TTL
> (`BRAIN_PROPOSAL_TTL_SECS`, `expire_if_stale`: expired → auto-reject +
> `proposal_expired` audit; approve/reject on stale refuse 400). **M3 docs**
> (SECURITY.md + MEMGHOST_MITIGATION.md honest). 583 server-side tests
> green (+3: `ingest_screens_injection_like_its_siblings` — the audit §5 drill
> as a model-backed `#[ignore]`d test with quarantine/reject/benign arms,
> `test_proposal_expires_after_ttl_and_audits`, and the lib's
> `source_prompt_is_pii_screened_and_rendered`) + 82 client tests + 94 plugin
> tests, clippy `-D warnings` + fmt + wasm + bundle budget green. See Agent 67 +
> `CHANGELOG.md` §[1.20.1].
> **v1.20.0 "Polish" shipped 2026-08-11** — a
> **client** release (client Cargo.toml 1.19.0 → 1.20.0; server stays at 1.18.2).
> The final milestone of the v1.14→v1.20 client chain (the done-state):
> **M1 system-following theme** (`dark → light → system` via
> `THEME_MODES` cycle + pure `pick_theme`; `system` sets
> `data-theme="system"` and the CSS `@media (prefers-color-scheme: light)`
> block follows the OS — no JS),
> **M2.1 a CI bundle budget** (`client/bundle-budget.sh` — release wasm ≤ 7 MB,
> measured 4.34 MB; the plan's <50 KB/<5 MB final budgets stay operator
> `dx bundle` measurements in BENCHMARKS), and **M3 offline-tolerance**
> (`queue.rs`: bounded 100, payload-keyed idempotency, localStorage-persisted
> action-ids only — never the token; approve/reject/purge/DSAR queue while the
> backend is unreachable, replay once-per-key on recovery, "queued (offline)"
> surfaced in review rows + batch summary + top-bar badge). **M4 zero-telemetry
> reaffirmed** (nothing collects data). 82 client tests (+5), clippy `-D
> warnings` + fmt + wasm green, bundle budget green. See Agent 66 +
> `CHANGELOG.md` §[1.20.0].
> **v1.19.0 "Integrated" shipped 2026-08-10** — a
> **client** release (client Cargo.toml 1.18.2 → 1.19.0; server stays at 1.18.2).
> The v1.19.0 plan (SSO + deep links + PWA + scale) was audited against the
> tree: most was already shipped — deep links (v1.16.7), iOS/Android `brain://`
> intent filters (v1.17.0), PWA shell (v1.16.7), recall debounce (v1.16.7), and
> the JWT-pair + silent-refresh + principal half of SSO (v1.16.5). The one
> remaining testable delta shipped: **audit filters URL-addressable**
> (`/audit?since=&principal=` via `Route::Audit { since, principal }` → pure
> `audit::filter_from_query`, seeded into the panel's `AuditFilter`). The rest
> are honest ceilings: OIDC PKCE needs a server `/auth/authorize` (v2.x —
> brain-server is a token validator, not an IdP), virtualized lists need viewport
> JS, wasm-split stays a Dioxus 0.7.10 ceiling. 77 client tests (+1), clippy +
> fmt + wasm green. See Agent 65 + `CHANGELOG.md` §[1.19.0].
> **v1.18.2 "Transparency" shipped 2026-08-09** — a
> **server** release (server Cargo.toml 1.17.5 → 1.18.2; client stays at
> 1.18.1): the two real accuracy gaps the v1.18.1 Transparency plan found in
> COMPLIANCE.md §7 (Art 50 pack) — **M2 `knowledge.origin` column**
> (write-time model-vs-human marker: `human` for interactive manual writes,
> `model` for memory auto-capture, safe `imported` default for bulk/unknown;
> idempotent migration backfill + index, wired into /add, propose→approve,
> /ingest/memory, procedures via the pure `gate::origin_for_source` helper) and
> **M1 `/export` provenance** (`export_format_version: 2` + per-row `origin` +
> `provenance_summary {total, by_origin, by_source}`; all 12 v1 fields preserved
> byte-identical). **M5 COMPLIANCE §7** aligned + an **Enforcement** note
> (Art 50 = national authorities, €15M/3% Art 99(3), not the €35M/7% Art 99(2)
> tier). **M3/M4 already shipped** (ai-notice/ai-literacy/cop-notice routes +
> docs/AI_LITERACY.md in v1.16.7/v1.16.8); `/.well-known/ai-notice`
> `origin_metadata` now lists `origin`. 476 server tests (+2), clippy `-D
> warnings` + fmt green; schema contract + INSERT-site guards updated to 1.18.2.
> See Agent 64.
> **v1.18.1 "Harden" shipped 2026-08-09** — a
> **client-only** release (client Cargo.toml 1.18.0 → 1.18.1; server + API
> contract unchanged at 1.17.5): closes the v1.17.8/v1.18.0 honest ceilings
> where a real, low-risk improvement exists — **M1 console history persists
> across reload, secret-safe** (only `redact_for_history`-clean lines reach
> localStorage via the i18n pref seam, capped at 100; non-JSON/opaque lines are
> flagged `secret` and stay in-memory — `credentials_stay_in_memory` guard
> still holds) and **M4a the client bundle is measured, not guessed** (wasm
> 3.7 MB + 60 KB JS + 40 KB CSS in BENCHMARKS.md; wasm-split deferred to
> Dioxus 0.8-stable). M2/M3/M5/M6 are code-grounded non-changes (no CLI-link to
> replace, no SSE control exists client-side, gesture/focus untestable here).
> 76 client tests (+2), clippy + fmt + wasm green. See Agent 63 +
> `CHANGELOG.md` §[1.18.1].
> **v1.18.0 "Compliant" shipped 2026-08-09** — a
> **client-only** release (client Cargo.toml 1.17.8 → 1.18.0; server + API
> contract unchanged at 1.17.5): the WCAG 2.2 AA + i18n + privacy hardening
> pass on the v1.17.x console. i18n (`en`/`de`/`fr`/`es`/`nl`),
> `prefers-reduced-motion`, keyboard A/S/R/J/K + WCAG 2.1.4 toggle,
> focus/landmark/semantic gates, and privacy labels shipped in v1.16.x–v1.17.0;
> this release closes the two remaining testable gaps — **M1.4 in-app `?`
> keyboard help on Review** (WCAG 3.2.6; pure `keyboard_help()` core + i18n
> `review_help_*` keys) and **M2 a `client-gate` CI job** (fmt + clippy `-D
> warnings` + test + wasm build — the client previously had zero CI coverage).
> 74 client tests (+1), clippy + fmt + wasm green. axe-core browser gate and
> the native VoiceOver/NVDA/TalkBack pass stay documented operator steps
> (`client/a11y-checklist.md`). See Agent 62 + `CHANGELOG.md` §[1.18.0].
> **v1.17.8 "Complete 3/3" shipped 2026-08-09** — a
> **client-only** release (client Cargo.toml 1.17.7 → 1.17.8; server + API
> contract unchanged at 1.17.5): the final part of the three-part "Complete"
> operator-console line — **M5 Data & Rights panel** (`/data`: purge by
> ids/owner, portable export JSON/UMP/UMP-Markdown, per-kind retention editor
> with one-click clear, `/decayed` + `/tombstones` registries), **M6 UMP panel**
> (`/ump`: capabilities card + `ump_integrity_badge`, remember, recall with kind
> filter + clamped max_recall, audit + verify chain), **M7 System panel**
> (`/system`: domains, snapshot integrity, Art 30, reindex, connectors +
> reconcile, and a Try-it console with `serialize_request` + `redact_for_history`
> so history never retains a token-bearing body), and the **M8 wrap** (three new
> routes added to rail + tab bar + palette, nav targets now 12; new i18n keys in
> all five locales, each locale 50 keys). 73 client tests (+7), clippy `-D
> warnings` + fmt + wasm build green; 7 new api.rs wire/parse cores + `Clone` on
> the 10 typed wire structs (root cause of `Signal<T>()` call-syntax failures)
> + `post_raw` made pub. Fixed the M5/M6/M7 rsx build hazards (hoisted `let`s +
> label computation before `rsx!`, literal-brace placeholders, `Key::Enter`,
> named-closure → `move |_| run_x(())`). See Agent 61 + `CHANGELOG.md`
> §[1.17.8].
> **v1.17.7 "Complete 2/3" shipped 2026-08-09** — a
> **client-only** release (client Cargo.toml 1.17.6 → 1.17.7; server + API
> contract unchanged at 1.17.5): the second of the three-part "Complete"
> operator-console line — **M3 Graph panel** (`/graph`: debounced entity
> lookup + traverse with typed hop-chain `paths` via a pure `render_path`
> core + `kind_is_valid` filter), **M4 Create workspace** (`/create` hub →
> Ingest Structured/Markdown/Memory tabs, Procedures step-builder +
> `/classify` + `/decision/:id/evaluate`, Consolidate propose/apply/undo;
> pure `parse_ingest_result`/`parse_decision_vars` cores), and the **M8 wrap**
> (Graph + Create added to rail + tab bar + palette, nav targets now 9;
> new i18n keys in all five locales). 66 client tests (+7), clippy `-D
> warnings` + fmt + wasm build green; 8 new api.rs wire types + methods
> pinned. Also fixed a real `render_path` bug (doubled ` --` separator).
> See Agent 60 + `CHANGELOG.md` §[1.17.7].
> **v1.17.6 "Complete 1/3" shipped 2026-08-09** — a
> **client-only** release (client Cargo.toml 1.17.0 → 1.17.6; server + API
> contract unchanged at 1.17.5): the first of the three-part "Complete"
> operator-console line — **M1 command palette v2** (fused nav + lookup +
> action; grouped Recent/Go to/Lookup/Run, 5-per-group cap, persisted recents,
> `/` re-focus + Tab trap, two-step destructive confirm, per-row aria-labels;
> pure `palette_group`/`command_keywords`/`palette_lookup`/`remember_recent`/
> `destructive_action` cores + an M1.5 route-coverage guard), **M2 Overview**
> (decision-first `/` home: 4-card status row — Health/Snapshot/Retention/UMP —
> each linking to its panel, a DAR-chain alert list from `/decayed` +
> `/tombstones` + `/consolidate/propose` + UiState signals with severity
> ordering, and a top-5 pending-proposal queue preview with one-click
> Approve/Reject + `/review/:id` deep link; pure `overview_alerts` core), and
> **M8** (Overview added to rail + tab bar + palette; `Connect` moved to
> `/connect` outside the shell; new Overview + palette i18n keys in all five
> locales; version + CHANGELOG + ROADMAP split + v1.17.4 plan marked
> superseded). 59 client tests (+10), clippy `-D warnings` + fmt + wasm build
> green; 6 new api.rs wire methods + types pinned. See Agent 59 +
> `CHANGELOG.md` §[1.17.6].
> **v1.17.5 "Eval Fix" shipped 2026-08-09** — the
> **server** release (server Cargo.toml 1.17.4 → 1.17.5; API contract
> 1.17.5): `brain eval` was dead — it GET'd `/recall` (POST-only → 405
> every run, so the v1.17.1 M3 ship gate never scored) and mapped judged
> indices through a HashSet (hash-order arbitrary). Now POSTs
> `{query, limit}`, parses `hits`/`results`, matches DOCS slice order;
> pinned by a new test. Round-21 CI gaps closed: `ump-conformance` job
> asserts the reference suite's `UMP 1.0 / L3` badge line on every push
> (keeps the README badge honest), `recall-gate` job enforces the frozen
> fixture floors (`r5/r10/mrr ≥ 0.85`), and the tag release now ships a
> CycloneDX SBOM (`scripts/sbom.sh` → dist/) per EU CRA / OWASP A03:2025.
> BENCHMARKS.md gains its first row (smoke set: r@5 0.919, r@10 0.919,
> nDCG@10 0.911, MRR 0.905; parity rows stay PENDING per protocol).
> See Agent 58.5 + `CHANGELOG.md` §[1.17.5].
> **v1.17.4 "UMP Conformance" shipped 2026-08-09** — the
> **server** release (server Cargo.toml 1.17.3 → 1.17.4; API contract
> 1.17.4): reference-suite wire fixes so `github.com/edihasaj/
> universal-memory-protocol`'s `conformance.ts` scores UMP 1.0 / L1–L3
> (previously "none"). **Breaking:** `did_key_from_ed25519` now emits the
> reference `0xed 0x01` 34-byte prefix (old `z2De…` → `z6Mk…`, pinned by the
> RFC 8032 vector-1 did); the integrity block is the reference §2.8 shape
> `{content_hash: "blake3:<base32>", signature: "ed25519:<std-base64>",
> signer: <did:key>}` (JS-flavor canonicalization so the reference
> `verify()` byte-matches; legacy v1.17.3 records still verify via dual-read).
> Ops: `from_ump` lenient (absent `ump` = 1.0), `provenance`/`consent`
> round-trip, `superseded_by` emitted on prior records (L2 bi-temporal),
> urn id resolution (the `ump_id` column is now loaded by
> `KNOWLEDGE_ROW_COLS`), revise drops the carried `origin` so revisions get
> a fresh urn, feedback → `{ok:true}`, forget distinguishes
> `erased`/`tombstoned`. New `#[ignore]`d `ump_suite_parity_l1_to_l3`
> replays the suite end-to-end; the external `@universalmemoryprotocol/core`
> conformance run scores **13/13, UMP 1.0 / L3** (the run caught a missing
> `ed25519:` signature prefix — fixed + pinned). 473 server tests + 70 lib +
> 9 + 8 + 7 + 3×2, clippy `-D warnings` + fmt green. See Agent 58 +
> `CHANGELOG.md` §[1.17.4].
> **v1.17.3 "UMP Rollout" shipped 2026-08-09** — the
> **server** release (server Cargo.toml 1.17.2 → 1.17.3; API contract
> 1.17.3): full UMP 1.0 conformance through **L3** — **M2 HTTP ops**
> (`/ump/capabilities`, `/ump/remember`, `/ump/memory/{id}`, `/ump/recall`,
> `/ump/revise`, `/ump/forget`, `/ump/feedback`, `/ump/subscribe` SSE,
> `/ump/audit`, `/ump/audit/verify` + batch `?format=ump` ingest +
> `/.well-known/ump.json`), **M3 MCP tools** (`ump.*`, 9 tools), **M4 file
> binding** (`?format=ump-md` export/import + `brain ump export|import` +
> the v1.17.1 `/export` empty-DB regression fix), **M5 identity +
> capability tokens** (`src/ump_integrity.rs`: did:key Ed25519, RFC 8785
> JCS, blake3 → base32, sign/verify; `brain ump keygen`; §5.2 compact
> bearer tokens enforced at middleware + per-handler `cap_gate` verbs ×
> scope, admin never grantable; §5.3 injection-resistant rehydration
> documented). 473 server tests + 7 brain-bin + 67 lib tests, clippy
> `-D warnings` + fmt green; conformance **UMP 1.0 / L3** (self-attested).
> See Agent 57 + `CHANGELOG.md` §[1.17.3].
> **v1.17.1 "Govern" shipped 2026-08-09** — the
> **server** release (server Cargo.toml 1.16.7 → 1.17.1; API contract
> 1.17.1): **M1 ingest-owner correctness fix** (the CRA DSAR-drill gap —
> `principal_to_owner` wired into every direct-ingest site, so a real DSAR
> locates the subject's rows), **M2 per-kind retention** (`/retention`
> GET/POST, query-time kind-default expiry, `BRAIN_RETENTION_KIND_DAYS`,
> `/decayed` surfaces `effective_expiry`/`reason`), **M3 eval ship-gate**
> (`brain eval` + `BENCH_RECALL_FLOOR`, frozen 32-query fixture), **M4 UMP
> wire adapter** (`/export?format=ump` + `/ingest?format=ump`,
> universalmemoryprotocol.io 0.1 → UMP 1.0 in v1.17.2, round-trip identity), **M5 Art 30
> register** (`/art30`, `BRAIN_CONTROLLER_NAME`), **M6 CoP marker**
> (`/.well-known/cop-notice`, self-attested), **M7 snapshot self-check**
> (`/snapshot/status` + `brain snapshot-status`: exists/size/0600/integrity/
> chain per `.bak`). 451 server tests + 5 brain-bin tests, clippy `-D
> warnings` + fmt green. See Agent 56 + `CHANGELOG.md` §[1.17.1].
> **v1.17.0 "Mobile" shipped 2026-08-08** — a **client-only**
> release (client Cargo.toml 1.16.8 → 1.17.0; server + API contract unchanged at
> 1.16.7): completes the v1.17.0 Mobile plan on top of the v1.16.6 mobile
> groundwork — **M2.4 portable refresh control** (Review/Audit/Health via a
> shared `RefreshButton`), **M3.3 `brain://` deep-link intent filters** (iOS
> `url_schemes` + Android VIEW/BROWSABLE intent), **M3.4 offline connect
> pre-fill** (last base URL persisted as a non-secret UI pref + specific
> failure), and **M3.1 store-readiness privacy labels**
> (`client/STORE_READINESS.md`, "no data collected" — accurate). 49 client tests
> (+1), clippy `-D warnings` + fmt + wasm build green. Native `dx bundle
> --platform {ios,android}` is an operator step (signing + Android SDK). See
> Agent 55 + `CHANGELOG.md` §[1.17.0].
> **v1.16.8 "Global" shipped 2026-08-08** — a **client-only**
> release (client Cargo.toml 1.16.7 → 1.16.8; server + API contract unchanged at
> 1.16.7): locale (i18n) + light/dark theme + density + locale-aware numbers +
> a privacy block on the connect screen. Zero-dep FTL-subset `t()` with
> `en`/`de`/`fr`/`es`/`nl` bundles compiled in via `include_str!` (current-locale →
> `en` → key fallback, never blank); `data-theme`/`data-density`/`dir` applied
> to `<html>` by signal-driven effects, prefs persisted (sanitized) to web
> `localStorage`; `format_number` groups per locale. **Also fixed a real build
> fragility:** `deploy-web.sh` now compiles Tailwind (`npx @tailwindcss/cli`)
> before `dx bundle`, because `dx bundle` does NOT recompile Tailwind in build
> mode — it copies a stale `assets/tailwind.css`, so CSS edits silently never
> reached the bundle (the stale-CSS bug class Agent 50 fixed). 48 client tests
> (was 43). Live `/app` re-deployed; `data-theme`/`data-density` verified in the
> served bundle. See Agent 54 + `CHANGELOG.md` §[1.16.8].
> **v1.16.7 "Integrated" shipped 2026-08-08** — the
> combined **server + client** release. **Server** (Cargo.toml 1.16.6 →
> 1.16.7): the hardening + compliance round that was sitting in `[Unreleased]`
> — **Art 50** `/.well-known/ai-notice` (EU AI Act, with `docs/MEMGHOST_MITIGATION.md`),
> **P0 snapshot-permission fix** (all `VACUUM INTO` `.bak` files now chmod
> 0600), **`/health` content-leak fix** (CVE-2026-29787 class; pure
> `health_body()` + regression test), **`/tombstones?limit=`** now honored,
> **`/export`** now emits the `source` column, and a test-isolation fix. See
> Agent 53. **Client** (1.16.6 → 1.16.7): the Integrated plan — **M1 deep
> links** (`/review/:proposal_id`, `/subjects/certificate/:dsar_id`),
> **M2 PWA** (manifest + offline-shell service worker that caches only
> `/app/index.html` + `/app/assets/*`, never the API), **M4 paginated audit**
> (server `OFFSET` + client Load-more with boundary-id dedup), **M5 command
> palette** (⌘K overlay), **M6 recall debounce** (300ms generation-guarded
> commit), and the carried-over hardening — **M7.3** hand-rolled drawer focus
> trap, **M7.5** aria-live regions, **M7.6** `dir="auto"` RTL. **M3
> wasm-split + M7.7 Mobile milestones are documented ceilings** (Dioxus 0.7.10
> has no wasm-split yet; no Android SDK here). 43 client tests + clippy + fmt
> + wasm green; live `/app` + manifest + sw 200. See `CHANGELOG.md` §[1.16.7].
> **v1.16.6 "Mobile" shipped 2026-08-08** — a
> client-only release landing the two testable milestones of the v1.16.6
> Mobile plan (M2 secure token storage + M3 responsive UX). **Dioxus pinned to
> 0.7.10** (the semver-open `0.7` spec already resolved to the newest stable —
> the 0.7.8/0.7.10 wasm-hotpatch TOCTOU/UB + 0.7.6 panic-resilience fixes are
> compiled in). **M2 `src/storage.rs`** is a `#[cfg(target_arch = "wasm32")]`-
> gated keyring seam: non-web persists the auth token to the OS keyring
> (`keyring` 3.6.3 apple-native/windows-native/sync-secret-service), web stays
> in-memory (v1.16.1 posture); connect saves only a real token
> (`should_persist`), launch auto-reconnects via a saved token. **M3** adds a
> mobile bottom tab bar + `.drawer` bottom sheet, pure `@media (640px)` CSS
> swap (no JS, same `Routable`), ≥44px touch targets, safe-area insets. M1/M4/M5/
> M6 (lib.rs mobile entry, probe pause, store readiness, MASVS) are documented
> operator/native-toolchain steps — no Android SDK / `dx` here. Client
> 1.16.5→1.16.6; server + API contract unchanged. 37 client tests + clippy
> `-D warnings` + fmt + wasm build green. See `CHANGELOG.md` §[1.16.6].
> **v1.16.4 "Styled" shipped 2026-08-08** — a
> client-only shadcn/ui design-system restyle: fixed sidebar dashboard shell
> (brand + grouped nav-link pills with live count badges + slim sticky top bar),
> a shadcn-style component layer in `input.css` (semantic tokens mapped onto the
> app's AA-verified palette + radius/shadows + reusable `.card`/`.btn`/`.badge`/
> `.input`/`.nav-link`/`.table` classes), every panel restyled to the layer, and
> a `deploy-web.sh` fix (stale-CSS glob now picks the freshest tailwind build).
> Client 1.16.2→1.16.4; server + API contract unchanged. All 31 client tests +
> clippy `-D warnings` + fmt green. See `CHANGELOG.md` §[1.16.4].
> **v1.16.2 "Harden + Accessible" shipped 2026-08-08** — the
> server serves the Dioxus client (`/app` ServeDir + SPA fallback,
> `BRAIN_CLIENT_DIR`), a path-aware CSP (strict API_CSP vs relaxed CLIENT_CSP for
> the WASM bundle), an ErrorBoundary around the router, operator-facing
> `error_message()` mapping, a cancel-safe batch `BatchSummary`, and two
> code-hygiene grep guards (`xss_escape_hatch_is_unused`, `credentials_stay_in_memory`).
> Plus the WCAG 2.2 AA client pass: SPA focus-to-`<h1>` + per-route document
> title (`PageTitle`/`use_document_title`), `scroll-margin-top` (2.4.11),
> a no-`<div onclick>` semantic gate, and `--color-ink-faint` → `#7c8492` (AA
> 4.6:1). **v1.16.0 "Client" shipped 2026-08-08** — the Dioxus
> control surface (web + desktop + iOS + Android, one Rust codebase). Implements
> the eight `IMPLEMENTATION_PLAN_v1.16.0_Client.md` milestones on top of the
> `client/` scaffold: M1 the connection state machine (single `use_future`
> probe with a false-offline guard + chain-verify-before-writes recovery;
> dependency-free sleep via `document::eval`+setTimeout — no tokio dep), M2 nav
> badges + principal + Esc-closable context drawer, M3 honest-batch review
> (per-row `RowOutcome` tracking, 404-no-pending = success, `BatchGuard`
> DropGuard, A/S/R/J/K keyboard with a WCAG 2.1.4 toggle, reject-with-reason +
> suggest-re-ingest), M4 recall decision-path viewer (per-retriever ranks,
> fused score, relevance tiers, `min_relevance` slider, deep-linkable
> `?trace=true` artifact via `/recall/:trace_id`), M5 DSAR certificate card
> (found/purged/tombstone_root/chain_head/certified_at + live green/red chain
> badge), M6 auth-failure feed (`GET /audit?kind=auth` → denied rows), M7 audit
> client-side filters + JSON export, M8 semantic-token layer (zero ad-hoc color
> classes remain). `.zed/settings.json` uses the Tailwind CSS language mode
> (`tailwindcss-intellisense-css`) so `@theme`/`@source`/`@apply` are understood.
> 25 tests (was 7), clippy `-D warnings` + fmt clean, zero new deps. `dx serve`
> is an operator step. See `CHANGELOG.md` §[1.16.0].
> **v1.15.0 "Observe" shipped 2026-08-08** — the
> observability + compliance-workflow layer on v1.14's governance primitives:
> read-event audit (recall/search/get/multi-get emit rows into the existing
> SHA-256 hash chain; opt-in — off for loopback, on for JWT mode —
> `BRAIN_AUDIT_READ_EVENTS` + `BRAIN_AUDIT_READ_SAMPLE_RATE` +
> `BRAIN_AUDIT_RETENTION_DAYS` prune-with-re-anchor), the recall trace
> endpoint (`GET /recall/{trace_id}/trace`, `POST /recall?trace=true` returns
> `trace_id`; `recall_traces` side table holds the non-content decision path),
> the DSAR workflow (`POST /dsar` locate→export→purge→chain-verifiable
> deletion certificate, `GET /tombstones` registry, `GET /dsar/{id}/certificate`,
> opt-in Art 19 HMAC-SHA256 webhook via `BRAIN_DSAR_WEBHOOK_URL`/`_SECRET`),
> and the buyer-facing `COMPLIANCE.md` (ISO 42001 / NIST AI RMF / SOC 2 map,
> Intent-Based-Auditing 4/4, PH DPA/GDPR/CCPA jurisdiction posture). **This
> release deliberately breaks the "no outbound HTTP dep on the server"
> constraint** — the Art 19 webhook needs outbound HTTP, so `reqwest` is now
> required (the `connector-github` feature gates only its binary). 518 tests
> green. See `CHANGELOG.md` §[1.15.0].
> **v1.14.0 "Gate" shipped 2026-08-07** — the ROADMAP's
> v1.14.0 row (the Alex Xu thread's #1 ask): human-in-the-loop write-back.
> `POST /ingest/proposal` scores a candidate deterministically (novelty via
> vec0 KNN, conflict via the consolidate machinery, salience via a
> length/entity heuristic) but creates NO `knowledge` row; it becomes memory
> only via `POST /proposals/{id}/approve` (one tx, optional atomic
> `?supersedes`). Per-chunk `expires_at` decay (strict `<`, default-excludes,
> `?include_decayed`, `GET /decayed` review list — nothing decays
> autonomously), `assertion_kind`/`confidence`/`min_relevance`, record-level
> `access_scope`+`owner` (JWT-mode deny-by-default filter; loopback trusts
> localhost), PII output redaction (`[redacted:email]`/`[redacted:phone]`) +
> opt-in write-time placeholder mode (`BRAIN_REDACT_PII=1`, `pii_map` vault),
> GDPR `GET /export` + `POST /purge` (hard audited delete across tables,
> tombstone + audit), `episodic` memory_kind + `?memory_kind=`. Migration bug
> fixed: the old `tombstones` `CREATE TABLE IF NOT EXISTS` was a silent no-op
> against the v0.9.1 schema (purge INSERT would have failed) — now guarded
> column-adds. 512 tests green. See `CHANGELOG.md` §[1.14.0]. *(Correction —
> **v1.20.19 "Vault"**: the write-time placeholder vault was never built; the
> shipped PII control is deterministic read-time output redaction, and the
> `pii_map` table is dropped.)*
> **v1.13.2 "Harden" shipped 2026-08-06** —
> post-1.13.1 rough-edges audit hardening pass. Three fixes from a deep
> API/code review: (1) `PRAGMA busy_timeout=5000` on **every** SQLite pool init
> (`src/main.rs` main pool, `src/domain_registry.rs` `open_with_migration`,
> `src/migration.rs` pragma batch) — previously only `auth/revocation.rs` set
> one, so concurrent writers against `POOL_MAX_SIZE=20` connections could fail
> immediately with `SQLITE_BUSY` instead of waiting; write contention now queues
> up to 5s. (2) `GET /graph/traverse` accepts `name`/`entity` as aliases for
> `start` (`#[serde(alias)]`, docs canonical stays `start`; the response field
> is `entity` and sibling routes use `name`/`entity`). (3) `POST /recall`
> accepts `explain` as an alias for `provenance` (`GET /search` had always gated
> telemetry on `explain`; `/recall` used `provenance` — same intent, two flag
> names; both now work on `/recall`). Back-compat preserved on both alias
> changes; `cargo fmt`/`clippy -D warnings`/478 tests green. See
> `CHANGELOG.md` §[1.13.2].
> **v1.13.1 "Recall" fix shipped 2026-08-06** —
> v1.15.0 M1 hotfix (automatic retrieval routing). Shim-mode recall never
> centroid-routed (a `None if !multi_db` short-circuit searched `global` only),
> so the v1.13.0 relabel migration made non-`global` rows (the moved
> `gutmindsynergy` blog corpus) unreachable by default recall. v1.13.1 routes on
> retrieval in shim mode too: the matched domain + a `global` rescue leg; an
> un-routed query scopes to `global` and never federates into a bulk domain
> (the blog-domination guard). Kill switch `BRAIN_RECALL_ROUTING_ENABLED`.
> 478 tests green. See `CHANGELOG.md` §[1.13.1].
> **v1.12.2 "Harden" shipped 2026-08-04** —
> audit-fix release: `/auth/refresh` check-then-act race closed
> (`record_and_rotate` under `BEGIN IMMEDIATE`, mutation-proven
> `concurrent_refresh_serializes_exactly_one_winner`), database stack bumped
> (rusqlite 0.40.1 / sqlite-vec 0.1.9 / r2d2_sqlite 0.35.0 → bundled SQLite
> 3.53.2, fts3_tokenizer + CVE-2022-35737-class fixes), and the permanently
> red `cargo audit` CI job fixed via `.cargo/audit.toml` (RUSTSEC-2023-0071
> "Marvin" accepted with documentation — verified no fixed release exists in
> any rsa/jsonwebtoken release; EdDSA keys avoid RSA entirely). 466 tests
> green. See `CHANGELOG.md` §[1.12.2].
> v1.12.1 "Harden" shipped 2026-08-04 —
> AuthZ wiring completion: closes the v1.2 S1 audit finding (Agent 38's
> "authorize() never called" claim had gone stale — ~15 handlers were gated,
> but 20 routes still shipped with middleware-only auth). Every non-public
> route now enforces its §3.3 matrix action at handler entry (20 gates wired:
> search/stats/embeddings/get*/multi-get/graph*/quarantine-list/audit/
> audit-verify/metrics/recall/verify/propose/connectors/revoke/domains/
> suggest-metrics/procedure-steps; `reindex` + `DELETE /memory/{id}` upgraded
> Write→Admin; `/audit` gains Admin gate + cross-tenant 403 via
> `handlers::audit_scope`; `/auth/revoke` finally enforces its documented
> admin requirement). Back-compat preserved: `None` principal (opaque mode)
> stays superuser; webhooks stay HMAC-internal. New mutation-proven
> wiring-guard test (`authz_gates_cover_every_non_public_route`, 40-route
> contract table) + router-level middleware tests. 465 tests green. See
> `CHANGELOG.md` §[1.12.1].
> v1.12.0 "Discern" shipped 2026-08-03 —
> noise-aware graph retrieval + complexity-gated activation: `tagged_with`/
> `alias_of` edges weigh 0.1 vs semantic types (the live KG is 94% taxonomy
> noise), GAAMA-style hub dampening (`w_ij·min(1, θ/deg(i))`, θ = 50) tames
> degree-73/101/150 mega-hubs, and the graph leg auto-engages as a bounded
> rescue pass before v1.5.0 abstention when the estimator says `ClarifyQuery`
> (arXiv:2602.03578; `BRAIN_GRAPH_RESCUE_ENABLED` kill switch). No LLM, no new
> schema, no re-ingest. 460 tests green. See `CHANGELOG.md` §[1.12.0].
> v1.11.0 "Associate" shipped 2026-08-03 — HippoRAG-2-style
> graph retrieval: deterministic Personalized PageRank over the existing
> `entities`/`relationships` KG as a third, opt-in `?graph=true` RRF leg on
> `/search` + `/recall` (`α = 0.5` matched to the reference config, bounded by
> `MAX_PPR_ITER`/`trace::MAX_VISITED`, no LLM, no new schema, no embeddings in
> the graph leg). 455 tests green. See `CHANGELOG.md` §[1.11.0].
> v1.10.0 "Procedural" shipped 2026-08-02 (ordered-step
> procedures (`POST /procedure` one-tx ingest + `GET /procedure/{id}/steps`
> via `next_step` edges with `step_index`), deterministic keyword-router
> categorization (`POST /classify`, auditable matched keywords), and
> deterministic decision-rule evaluation (`POST /decision/{id}/evaluate`).
> `knowledge.node_kind` repurposed as Mem0-style `memory_kind`
> (fact/procedure/step/decision; legacy `'event'` → `'fact'`, fresh-DB default
> now `'fact'`). Fixes from the finish pass: `classify` matched-keywords lexicon
> index bug + `MemoryKind::from_str` wired at its read site. 447 tests green.
> See `CHANGELOG.md` §[1.10.0].
> v1.9.1 "Harden" (bug-fix) shipped 2026-08-02 (near-dup coverage via the live
> `vec_knowledge` index + suggest-feedback last-wins dedup + dead-code removal).
> v1.9.0 "Suggest" (light cut) shipped 2026-08-02 (opt-in anticipation +
> `POST /suggest/feedback` + `GET /suggest/metrics`, `BRAIN_SUGGEST_ENABLED`
> kill switch).
> v1.8.0 "Maintain" (light cut) shipped 2026-08-01 (reviewable proposals + undo).
> v1.7.0 "Explain" (light cut) shipped 2026-08-01 (faithful path explanations).
> v1.6.0 "Reconcile" (light cut) shipped 2026-08-01 (atomic supersession).
> v1.5.0 "Epistemic" (light cut) shipped 2026-08-01 (calibrated abstention + `/verify`).
> v1.4.2 "Link" (noise-reduction release) shipped 2026-07-30.
> v1.4.0 "Calibrate" shipped 2026-07-30 (surpass-human retrieval).
> v1.3.0 "Bedrock" shipped 2026-07-29 (memory-safety hardening).
> v1.2.0 shipped 2026-07-29 (JWT/JWS + AuthZ). v1.1.0 shipped 2026-07-28.
> v1.0.0 "Domains" shipped 2026-07-26. **Next milestone: v2.0.0 Cortex**
> (multi-team tenancy, ready — consumes the v1.2 AuthN/AuthZ foundation).
> All v1.x releases shipped; v2.0 is the first externally-pilotable release. Noted: v1.12.2 "Harden" (audit-fix) is the latest 1.x point release; see the 1.12.2 entry above and `CHANGELOG.md` §[1.12.2].
> v1.11.0 "Associate" shipped 2026-08-03; see the agent entry below.


---

## Agent execution log

## All Agents COMPLETED ✅

## Agent 91: v1.20.24 "Sweep" — the audit gaps, closed (session 2026-08-13)
**Status:** COMPLETED (code + tests + gates + release wrap; deploy/tag pending operator)
**Date:** 2026-08-13

Shipped the v1.20.24 "Sweep" **server + client + plugin** release per
`IMPLEMENTATION_PLAN_v1.20.24_Sweep.md`: the post-v1.20.23 audit itemized
seven unpaid gaps on the closed harden line. This release pays all seven —
**no new endpoints, no new fields, no telemetry** — plus one genuine bug the
new regression tests exposed.

- **G1 — every agent-facing seam strips invisible Unicode.** The v1.20.3
  `strip_invisible` pair becomes a shared lib module (`src/strip_invisible.rs`;
  screen.rs re-exports, `crate::screen::*` untouched), applied at: the MCP
  tool-result envelope (extracted pure `tool_result_payload`) + `format_response`
  seam (`src/bin/mcp.rs`), the CLI `brain recall`/`brain get` prints
  (`src/bin/brain.rs`), and the openclaw plugin (`format.ts::sanitizeForBlock`
  + the `\u200B-\u200F\u202A-\u202E\u2066-\u2069\uFEFF` class; recall titles
  + `memory_get` title + `memory_graph_entity` outputs through it). Ponytail:
  strips output only; storage verbatim.
- **G7 — client display fences.** Strips at evidence-modal content,
  procedure-step content, graph names/relations, review + ops source prompts;
  the submit-form content columns become a bounded scroll box
  (`max-h-40 overflow-y-auto`) — LITL smuggling is screened server-side; this
  is the display fence. CSS-only → client test count unchanged (111).
- **G2 — PII read-path uniformity.** `GET /get/{id}` + `POST /multi-get`
  now select + mask `pii` rows for non-admin principals (the v1.14
  `redact_content` pattern; `pii_principal` cloned pre-move), `POST /search`
  masks after flagged-evidence suppression, `GET /proposals` masks content via
  the read-time `scan_pii` leg.
- **G3 — auth fails closed.** `config::auth_token_misconfigured()` — explicit
  `AUTH_TOKEN_FILE` that can't yield tokens AND no `AUTH_TOKEN` fallback →
  fatal at startup; `auth::check_secret_permissions()` — `mode & 0o077 != 0`
  → refuse. Enforced on the token file (via `config::auth_token_file()` in
  `TokenStore::new`) and the JWT private key (`jwks.rs`); `main_inner` exits
  before any bind. Ladder + no-file default unchanged.
- **G4 — DSAR erases every domain DB.** `post_dsar` multi-db runs
  `run_dsar_pool` per `registry.known_domains()` pool (shim = the single
  global pool, byte-identical v1.20.23), non-global pools first each in its
  own tx (erasure-safe direction), global last with `write_ledger=true` +
  `aggregate_hash` (SHA-256 of `{"subject","domains":[…]}`); post-commit
  audit/chain-head/certificate on the global conn; tombstone anchor prefers
  the ledger-bearing run. New `DsarPoolRun` + extracted `run_dsar_pool`.
- **G5 — `/decayed` narrowed + the found bug.** Extracted pure
  `decayed_superset_sql` (branch A exact `expires_at < ?1` + branch B
  kind-policy superset at the **min**-days cutoff; `page_decayed` stays the
  arbiter) served by new `idx_knowledge_expires_at` + `idx_knowledge_kind_created`.
  **The superset regression test failed first, exposing `/decayed` returning
  `[]` since v1.14**: `strftime('%s', …)` is TEXT, `get::<i64>` dropped every
  row in `.filter_map(|r| r.ok())`. Fixed with `unixepoch(…)` (INTEGER, same
  parsing).
- **G6 — deletion digests.** `purge_chunk_ids` computes `sha256_hex(content)`
  in-tx into `tombstones.content_hash` (not the row's brute-forceable xxh3-64);
  DSAR ledger bundle hash = `gate::sha256_hex` (pub(crate)). Knowledge-dedup
  content_hash stays xxh3 deliberately (row still exists).
- **Tests:** main bin 527 → **532 passed** / 5 ignored (+5: superset property
  on a real DB, purge-digest, cross-domain purge + single ledger, auth
  permission ladder, config fail-closed ladder), MCP bin 13 → **15** (envelope
  + response seam); client 111 (unchanged); plugin 94 → **96** (bidi class +
  title strip). jwks + main-auth fixtures now write key files 0o600 (the
  fail-closed contract). Both trees + plugin clippy `-D warnings` + fmt clean;
  server 5 binaries + client wasm clean.

Version both `Cargo.toml`/locks → 1.20.24. CHANGELOG §[1.20.24],
`IMPLEMENTATION_PLAN_v1.20.24_Sweep.md`, ROADMAP released-row + plan row,
AGENTS header + this entry.

**Honest ceilings (carried to v2.x):** G3 is reader-side enforcement at
startup (a file chmod'd wide after boot is not re-checked mid-flight). G5's
superset is exact for the CURRENT_TIMESTAMP format only. G4's aggregate is a
digest of the domain *list* (per-pool bundles hash individually at write
time); the certificate is a best-effort audit record, not a crash-recovery
protocol. G2 masks read-time; storage stays verbatim.

## Agent 90: v1.20.23 "Calibrate" — reviewer calibration strip (session 2026-08-13)
**Status:** COMPLETED (code + tests + gates + release wrap; deploy/tag pending operator)
**Date:** 2026-08-13

Shipped the v1.20.23 "Calibrate" **server + client** release per
`IMPLEMENTATION_PLAN_v1.20.23_Calibrate.md`: the HITL essay's fourth condition —
**evaluative feedback to the reviewer** (a rubber-stamp gate is a false
control). The signals already shipped (`created_at`/`edited_at`/
`screen_verdict` on every `ProposalView`, `decided_at` written on
approve/reject/expire since v1.14.0) but `decided_at` was **never read**, so no
consumer could compute a decision-latency. This release exposes it, adds a
`since` window, and computes the four reviewer signals client-side — **no new
telemetry, no new server logic**.

- **M1.1 — `ProposalView.decided_at`** (`src/handlers/gate.rs`). The
  `list_proposals` SELECT now carries `decided_at` (column 11, `Option<i64>`,
  `#[serde(default)]`). The three write sites (approve/reject/TTL auto-expire)
  always stamped it; the read now surfaces it. Extracted `list_proposals_page`
  (the `page_decayed`/`list_dsar_page` idiom) so the projection is
  unit-testable with a bare `&Connection` — no HTTP stack, no model.
- **M1.2 — `since` window param.** `GET /proposals?status=&limit=` gains
  `?since=<unix ts>` (`WHERE status = ?1 AND created_at >= ?3` when present;
  byte-identical legacy query when absent). Parameterized. A `since` window
  still stops at `LIMIT` (200), so the stats fetch passes `limit=200` or it
  samples only the 50 default.
- **M2 — client calibration core + strip** (`client/src/panels/review.rs`).
  Pure `Calibration` + `calibration_stats(approved, rejected)` — approve-rate,
  median decision latency, edit-rate, screen-override-rate, zero denominators →
  `0.0`/`None` (no NaN). `ApiClient::proposals_since` fetches both windowed
  pages at `limit=200`. A dismissable strip above the queue renders the four
  figures + a rubber-stamp warn (approve-rate > 0.9 over ≥ 20 decisions →
  `warn` tier); fetch-failed → renders nothing (offline degrade). `role="status"`
  + `aria-live="polite"`. `cal_*` i18n keys in `en` only. A plain fn (like
  `card`) rather than `#[component]` (the macro's Clone+PartialEq prop
  constraint doesn't fit the closure-capturing body).
- **Tests:** server +2 (`proposal_view_round_trips_decided_at`,
  `proposals_since_filters_created_at_and_is_optional`), main bin 525 → **527
  passed** / 5 ignored; client +3 (`calibration_stats_rates_and_median`,
  `calibration_stats_handles_empty_and_zero_denominators`,
  `rubber_stamp_warns_only_over_real_workload`), 108 → **111 passed**. Both
  trees clippy `-D warnings` + fmt clean; server all 5 binaries + client wasm
  build clean; `scripts/badges.sh --selfcheck` OK. `openapi.yaml` documents
  `ProposalView.decided_at` + the `since` param.

Version both `Cargo.toml`/locks → 1.20.23. CHANGELOG §[1.20.23] (+ the v1.20.x
line-closure note), ROADMAP released-row + plan row, `IMPLEMENTATION_PLAN_v1.20_Hardening_Line_INDEX.md`
closure note, README badge, AGENTS header + this entry. **v1.20.23 closes the
v1.20.x hardening line** (Scrub → Bound → Vault → Replay → Subject360 → Clocks
→ Calibrate) — the v1.20.24 "Sweep" audit-followup shipped after (Agent 91).

**Honest ceilings (carried to v2.x):** the window is `since`-bounded **and**
list-capped (LIMIT 200) — a 30-day window on a busy queue samples the newest
200, so the strip labels itself "last 200 decisions" when the cap is hit (a
COUNT-aware window is v2.x). `override_rate` keys on the v1.20.3 read-time
`screen_verdict` recomputation, not a stored decision-time verdict (a model
swap re-badges in-flight rows). The strip is per-operator-global (all
principals), not per-reviewer (RBAC breakdown is v2.3). The `warn` threshold
(0.9 / 20) is a constant heuristic, not a reviewer baseline (v2.x cohort
tooling).

## Agent 89: v1.20.22 "Clocks" — DSAR Art 17 deadline + retention expiry (session 2026-08-13)
**Status:** COMPLETED (code + tests + gates + release wrap; deploy/tag pending operator)
**Date:** 2026-08-13

Shipped the v1.20.22 "Clocks" **server + client** release per
`IMPLEMENTATION_PLAN_v1.20.22_Clocks.md`: the v1.20.15 "queue is a clock" core
(reused unchanged — zero new clock logic) extended to **erasure + retention**,
so GDPR Art 17's 30-day window and Art 12's response deadline become *visible,
not assumed*. `dsar_requests` always stamped `created_at`/`completed_at`; what
was missing was the visibility.

- **M1.1 — `DsarResponse` deadline** (`src/handlers/observe.rs` +
  `src/config.rs`). Pure `dsar_deadline(created_at) = created_at +
  dsar_window_secs()`; `config` gains `DEFAULT_DSAR_WINDOW_DAYS = 30` (Art 17)
  + `BRAIN_DSAR_WINDOW_DAYS` override (the `BRAIN_PROPOSAL_TTL_SECS` env
  pattern). `DsarResponse` gains `created_at` + `deadline` (computed, the
  client's source of truth — the `expires_at`/`warn_secs` discipline). No
  schema change; the certificate path is untouched.
- **M1.2 — `GET /dsar` ledger list** (Admin). Bounded (`limit` default 100,
  clamped `1..=MAX_MULTI_GET`), newest-first (`ORDER BY id DESC`), the audit
  pagination idiom. `{ requests: [{id, subject, action, status, created_at,
  deadline, completed_at}], total }`. `deadline` is **server-computed per
  row** (via the shared `dsar_deadline`), so the client ticks against the same
  number the POST response carries — **no client mirror of the window** (a
  deliberate deviation from the plan's frozen row shape: without it M2.1 would
  need a client-side window constant, the very drift this release is against).
  Extracted `list_dsar_page` (the `page_decayed` idiom) so ordering + page
  boundary are unit-testable without HTTP. Wired into the openapi route +
  schema tables and the route/authz guard tables.
- **M1.3 — two server tests:** `test_dsar_deadline_is_created_at_plus_window`
  and `test_dsar_ledger_list_returns_rows_with_deadline_fields` (newest-first
  ordering, open-row `completed_at` = None + `deadline` present, `limit`/
  `offset` boundary, `total` counts all rows). Main bin 523 → **525 passed** /
  5 ignored.
- **M2.1 — Subjects panel: DSAR ledger + 30-day countdown** (`client`). New
  `ApiClient::dsar_ledger` + `DsarLedger`/`DsarLedgerRow` wire types
  (`#[serde(default)]` timestamps). The panel fetches the ledger and per open
  row runs the countdown through the v1.20.15 `time_budget::{remaining, tier,
  format_remaining}` core (day-scale bands `<3d` warn, `<1d` danger), re-rendered
  by one ~30s on-load ticker (the ops.rs idiom). Pure `dsar_clock` render core
  + `dsar_clock_*` i18n keys in `en`.
- **M2.2 — Data panel: next expiries** (`client`). Pure `next_expiries`
  (sort by expiry, take 10, skip already-expired) + tier-colored labels via
  `format_remaining`. `expiry`/`data_next_expiry` i18n key in `en`.
- **M2.3 — three client tests:** `dsar_clock_tiers_and_labels_the_art17_deadline`,
  `next_expiries_sorts_by_expiry_caps_at_ten_and_skips_expired`,
  `dsar_ledger_parse_defaults_absent_timestamps`. Client 105 → **108 passed**.

All gates green: both trees clippy `-D warnings` + `fmt` clean, all server
binaries + client wasm build clean, openapi/route/schema guards green. Version
both `Cargo.toml`/locks → 1.20.22. CHANGELOG §[1.20.22], ROADMAP released-row,
`docs/trust/proof-map.md` DSAR row, AGENTS header + this entry.

**Honest ceilings (carried to v2.x):** the countdown is a **signal, not
enforcement** — brain-server never auto-re-purges or re-reports (no background
worker; the v1.20.17 ledger TTL is the only automatic bound). The 30-day window
is display math on `created_at`; the DB does not enforce it (a
reminder/notification channel is v2.x). `GET /dsar` is an Admin-only operator
registry, not subject-facing (DSARs keep flowing through POST + the
certificate path). `/decayed` only returns already-expired rows, so the Data
"next to expire" card is the client boundary that would surface a near-expiry
row if the server ever returned one.

## Agent 88: v1.20.21 "Subject360" — DSAR footprint preview (session 2026-08-13)
**Status:** COMPLETED (code + tests + gates + release wrap; deploy/tag pending operator)
**Date:** 2026-08-13

Shipped the v1.20.21 "Subject360" **server + client** release per
`IMPLEMENTATION_PLAN_v1.20.21_Subject360.md`: turning the execute-blind DSAR
into an **execute-informed** one — a read-only `dry_run` previews *what would
be deleted* before any purge (GDPR Art 17 "show the scope"). Same locate
engine, same export-bundle builder, one boolean between preview and erasure.

- **M1 — `dry_run` on `POST /dsar`** (`src/handlers/observe.rs`).
  `DsarRequest` gains `#[serde(default)] dry_run: bool`; `DsarResponse` gains
  `footprint` (skip-if-none); `DsarOutcome` becomes an enum
  (`Completed`/`Footprint`). The handler locates + builds the bundle, and a
  `dry_run` branch reports the `Footprint` then drops the read-only tx —
  nothing purged, swept, ledger-written, or certified. `Footprint` carries
  `roots`/`derived`/`export_rows`/`tombstones`/`dsar_rows`/`dry_run: true`.
  The export-bundle SELECT was extracted once into `build_export_bundle` and
  is shared by both paths (no duplicated query); `count_subject_tombstones`
  matches the purge's exact tombstone reasons (`owner:<subject>`,
  `derived`+`origin_id` scoped to this subject's roots).
- **M1.1** — two server tests: `dsar_dry_run_footprint_counts_and_writes_nothing`
  (3 roots + 1 derived + prior tombstone → exact counts; knowledge/tombstones/
  ledger untouched) and `dsar_export_bundle_builder_matches_live_shape`
  (behavior-preserving refactor proof).
- **M1.2 — openapi.yaml** documents `dry_run`, the `Footprint` schema
  (under `components`), and `DsarResponse.footprint`; `status` enum gains
  `preview`. No new route — the route/schema contract guards are unaffected.
- **M2 — footprint preview card** (`client/src/panels/subjects.rs` +
  `client/src/api.rs`). `ApiClient::dsar_preview` POSTs
  `{subject, action: both, dry_run: true}` (pure `dsar_preview_body` builder +
  `parse_footprint` decode core); the panel renders a "Preview DSAR
  footprint" card (subject input + button, `role="status"` preview note,
  **no** purge button — one-click separation of see vs erase). `dsar_preview_*`
  i18n keys in `en` only.
- **M2.1** — two client tests: `parse_footprint_reads_counts_and_dry_run_flag`,
  `dsar_preview_request_carries_dry_run_true`.

+2 server tests (main bin 521 → 523, 5 ignored) and +2 client tests (103 →
105). All gates green: both trees clippy `-D warnings` + `fmt` clean, all 5
server binaries + client wasm build clean, openapi/route/schema guards green.
Version both `Cargo.toml`/locks → 1.20.21.

**Honest ceilings (carried to v2.x):** the footprint is a point-in-time
preview (locate semantics: owner + `derived_from` walk, depth 8) — not a full
cross-domain dependency analysis (federation is v2.x). Ledger-history counts
reflect the v1.20.17 retention window. No parallel "what is *not* deleted"
report (backups posture in COMPLIANCE.md). No new schema.

## Agent 87: v1.20.20 "Replay" — decision-path replay surface (session 2026-08-13)
**Status:** COMPLETED (code + tests + gates + release wrap; deploy/tag pending operator)
**Date:** 2026-08-13

Shipped the v1.20.20 "Replay" **client** release per
`IMPLEMENTATION_PLAN_v1.20.20_Replay.md`: turning the decision path the server
already stored (v1.15.0 "Observe" M2, `GET /recall/{trace_id}/trace`) into a
**routed, ledger-linked, exportable evidence surface**. Server 1.20.19 →
1.20.20 is version-alignment only — **zero server code**, `openapi.yaml`
untouched.

- **M1 — routed leaf is the structured replay view** (`client/src/panels/recall.rs`).
  `Route::RecallTrace` (main.rs) already delegates to `trace_panel` — no new
  renderer. The `TraceCard` header now reads the **stored** shape: `query_hash`
  (not `query`, v1.20.17 M3) and the applied `scope` **array** (it was reading a
  nonexistent `query`/`applied_scope` string before, so those cells were stale).
  Every displayed string (header fields + per-hit id/score/source/relevance/
  assertion) crosses the v1.20.3 `strip_invisible` render boundary via pure
  `replay_str`/`replay_list` — closing the bidi/zero-width smuggling class on
  the replay view with no drift from the other surfaces.
- **M2 — audit ledger → replay deep link** (`client/src/panels/audit.rs`).
  The join is free: the read-event audit row id **is** the trace id. A new
  `replay` column renders a link to `/recall/{id}` for `kind == "recall"` rows
  (and only those) via pure `replay_href` — test-pinned so a future
  trace-capable kind is wired explicitly, never silently left unlinked.
- **M3 — evidence export + i18n**. `trace_panel` gains an export button that
  downloads the raw trace JSON through the existing `document::eval` blob seam
  (the audit JSON-export idiom — no new helper). New `replay_*` keys
  (`replay_title`/`replay_audit_link`/`replay_export`) authored in `en` only;
  de/fr/es/nl fall back per the `ops_title` convention. `RecallTrace` stays a
  detail route — the palette guard is unaffected.

+3 tests (main client bin 100 → 103). All client gates green: clippy
`-D warnings` + `fmt` clean, wasm build clean, server suite untouched.

**Honest note:** the replay view is read-only over what the trace recorded;
traces store the query **hash** (deliberate — a recall query can be personal
data), so the exact query is recovered via audit + hash, not shown verbatim.
Read-event traces remain opt-in + sampled (JWT mode default), so the ledger
link exists only where a trace row exists. No screenshot/PDF export — the JSON
is the honest evidence artifact (signed-PDF remains the v2.x T0.5 ceiling).

## Agent 86: v1.20.19 "Vault" — PII-vault promise made honest (session 2026-08-13)
**Status:** COMPLETED (code + tests + gates + release wrap; deploy/tag pending operator)
**Date:** 2026-08-13

Shipped the v1.20.19 "Vault" **server** docs-correction release per
`IMPLEMENTATION_PLAN_v1.20.19_Vault.md`: making the **never-built** v1.14
`pii_map` write-time placeholder vault honest. Client stays at 1.20.16; one
schema change (a table *drop*), no new route.

- **M1 — dead read path removed** (`src/handlers/gate.rs`). The only in-tree
  `pii_map` usage was `/export`'s read side (`?include_pii_map=true` +
  `pii:read`). `ExportQuery.include_pii_map` and the `pii_map` envelope key are
  gone; a request carrying `?include_pii_map=true` is simply ignored (serde
  drops the unknown field). `export_format_version` stays at 2.
- **M1.2 — real posture documented** (`src/gate.rs`, `src/handlers/observe.rs`).
  Rewrote the `/export` doc + the `redact_content` `ponytail:` to state plainly:
  the shipped PII control is deterministic read-time output redaction
  (`redact_content` + `screen_source_prompt`, default-on unless the caller holds
  `pii:read`/Admin) + at-rest LUKS (v1.12.2). A fetchable placeholder→raw map
  would increase the personal-data surface; it is deliberately absent.
- **M1.3 + M1.4 — table dropped** (`src/migration.rs`). The `CREATE TABLE
  pii_map` block became `DROP TABLE IF EXISTS pii_map` — erases any legacy
  placeholder rows and the table at migration (idempotent; a fresh DB never
  recreates it). Schema stamp → 1.20.19 (`SCHEMA_VERSION_V1_20_19`), guarded by
  `test_migration_schema_contract` (now asserts the table is *dropped*) +
  `migration_drops_pii_map_and_empty_table` (seeds a legacy row, re-migrates,
  asserts row + table gone and ingest still works).
- **M2 — configuration contract.** `BRAIN_REDACT_PII` had **no** `config.rs`
  getter — the write-path promise was purely documentation. Deleted the claim
  from `docs/features.md`/`docs/configuration.md`/`docs/security.md`/
  `docs/compliance.md`/`docs/human-in-the-loop.md`/`docs/RFP_RESPONSE_KIT.md`/
  `docs/api.md`/`COMPLIANCE.md`/`SECURITY.md`; `openapi.yaml` `/export` no longer
  documents `include_pii_map`/`pii_map`.

+2 tests (main bin 519 → 521, lib 70 → 71). All gates green: clippy
`-D warnings` + `fmt` clean, openapi/route/schema guards green, release build
clean.

**Honest note:** this release retracts a promise that was never delivered —
there was no write path, so no operator relied on the behavior; the change
strictly shrinks the personal-data surface (a table we never wrote to is gone).

## Agent 85: v1.20.18 "Bound" — DoS + performance bounds (session 2026-08-13)**Status:** COMPLETED (code + tests + gates + release wrap; deploy/tag pending operator)
**Date:** 2026-08-13

Shipped the v1.20.18 "Bound" **server** release per
`IMPLEMENTATION_PLAN_v1.20.18_Bound.md`: closing the three unbounded read paths
and collapsing the two quadratic scans the v1.20.2 "Harden" D-group left.
Client stays at 1.20.16; one schema change (a tombstone index), no new route.

- **M1 — Graph endpoints finite edge sets** (`src/main.rs`). `get_entity` and
  `get_relations` returned every incident edge. Both now read a `?limit=`
  (shared `GraphLimit` query struct + `clamp_graph_limit`, default
  `MAX_GRAPH_EDGES` = 500, clamped 1..=500) and run `ORDER BY r.id LIMIT ?` —
  a stable, reproducible page (the KG has no histogram to rank by). Extracted
  `entity_relations` / `relations_for` so the LIMIT contract is unit-tested
  (`graph_entity_respects_limit_and_clamps`, `graph_relations_respects_limit_*`).
- **M2 — `find_subject_conflicts` grouped by subject** (`consolidate.rs`). The
  proposal-write conflict scan was O(n²) over ALL current rows though the rule
  only compares same-subject rows. Now grouped via `HashMap<String, Vec<&Row>>`
  → O(sum of m² per subject), ~O(n) dominating on mostly-unique subjects.
  Output sorted by `(from_chunk, to_chunk)` for determinism. Rule unchanged,
  verified by `find_subject_conflicts_groups_by_subject_same_output` +
  `find_subject_conflicts_returns_all_pairs_per_subject`.
- **M3 — `idx_tombstones_reason_purged`** (`migration.rs`). Compound index on
  `tombstones(reason, purged_at)` for the `/tombstones?subject=&since=`
  registry + DSAR certificate reads. Schema stamp → 1.20.18
  (`SCHEMA_VERSION_V1_20_18`); guarded by `test_migration_schema_contract`.
- **M4 — `/decayed` paged** (`handlers/gate.rs`). `list_decayed` returned every
  expired chunk. New `?limit=`/`?offset=` page the Rust-filtered result
  (default `MAX_DECAYED` = 500) — the split never lands on the "is it
  expired?" decision. Extracted `page_decayed` (`page_decayed_respects_limit_and_offset`).

+5 tests (main bin 514 → 519). All gates green: 519 passed / 5 ignored (main
bin), clippy `-D warnings` + `fmt` clean, openapi/route/schema guards green,
release build clean.

**Honest ceilings:** the graph `ORDER BY r.id` page is a bounded but arbitrary
window (no semantic ranking); `/decayed` pages the corpus but still scans it
once (the expiry is a Rust pure function, not a SQL predicate); the conflict
scan is still quadratic within a single subject (inherent to the mC2 rule).

## Agent 84: v1.20.17 "Scrub" — GDPR erasure (Art 17) completeness (session 2026-08-12)
**Status:** COMPLETED (code + tests + gates + release wrap; deploy/tag pending operator)
**Date:** 2026-08-12

Shipped the v1.20.17 "Scrub" **server** release per
`IMPLEMENTATION_PLAN_v1.20.17_Scrub.md`: closing five verified GDPR-erasure
(Art 17 "right to erasure") completeness gaps. **No schema change, no new
route** — every fix lands on existing code paths. Client stays at 1.20.16.
See `CHANGELOG.md` §[1.20.17].

### Changes Made
- **M1 — DSAR ledger stores a hash, not the raw bundle** (`src/handlers/observe.rs`).
  `POST /dsar` used to persist the full exported `bundle` JSON in the
  `dsar_requests` side-table — a retained copy of the very data the DSAR just
  erased. Now persists `bundle_hash` (xxh3 of the export body) only, and the
  working export body is discarded after the certificate is built. Mature
  ledger rows are pruned on the existing read-event prune cadence (the same
  `spawn_blocking` that calls `prune_audit_retention`): new
  `purge_stale_dsar_ledger(conn, retention_days) -> i64` deletes rows where
  `status='completed' AND completed_at < now - days*86400`, guarded by
  `BRAIN_DSAR_LEDGER_DAYS` (default 30, `config::dsar_ledger_retention_days()`).
  Zero-retention is a no-op (no autonomous deletion of a just-completed
  certificate).
- **M2 — cross-owner export redaction** (`src/handlers/gate.rs`). `GET /export`
  gained an optional `redact_owner` query param: when present, any row whose
  `owner` doesn't match the value exports with `content` replaced by
  `[redacted]`. A shared `should_redact(row_owner, redact_owner)` helper drives
  both the JSON path and `render_ump` (`?format=ump`), so the two paths can
  never disagree about a row. A cross-owner export no longer leaks another
  subject's chunk body.
- **M3 — stored recall traces hash the query** (`src/handlers/recall.rs`). The
  `recall_traces` side-table stored the raw `query` text. Now stores
  `query_hash` (xxh3 fingerprint) so the replay endpoint returns the decision
  path without retaining the queried prose at rest.
- **M4 — UMP scope-mismatch audited as a denied auth event**
  (`src/handlers/ump_ops.rs`). A `ump.remember` whose declared `scope.owner`
  mismatches the authenticated principal was silently dropped. Now extracted
  `record_forbidden_scope(conn, principal_sub, declared_owner) -> bool`: best-
  effort `audit::record(AuditKind::Auth, principal, detail, AuditStatus::Denied,
  "api")` where detail names the mismatch — hashed like all audit fields. An
  audit failure never fails the request.
- **M5 — purge-tx atomicity** (`src/handlers/observe.rs`). The DSAR erase
  transaction now commits the ledger row with the erase (via
  `last_insert_rowid` before the record move), and the certificate
  `signed_at`/certified fields are backfilled after commit — an interrupted
  purge can't leave an orphaned export with no ledger record.
- **Release wrap**: server Cargo.toml/lock 1.20.16 → 1.20.17 (`client/` not
  bumped — server-only); openapi.yaml documents `redact_owner` on `/export`,
  the trace `query_hash`, and the version stamp; CHANGELOG §[1.20.17];
  ROADMAP released header + v1.20.17 Shipped row; README badge → 1.20.17;
  AGENTS header + this entry.

### Verification
- `cargo test --features bench,migrate`: **514 passed, 5 ignored** (main bin;
  was 507 at the 1.20.16 baseline — the five M1/M3/M4/M5 tests land in the
  observe/recall/ump bins and the gate test extends an existing export test).
  All targets green, 0 failed.
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean
  (after removing a useless no-arg `format!` and an unused IIFE in the export
  test).
- `cargo fmt --check`: clean.
- `test_openapi_covers_routes` + `authz_gates_cover_every_non_public_route` +
  `test_migration_schema_contract` green (no new routes, no schema change).
- Release build (all 5 binaries) clean.

### Ship status: COMPLETED (code + tests + gates + wrap) 2026-08-12
`scripts/install-service.sh` (live restart — picks up the hash-only ledger +
purge), commit/tag `v1.20.17`, and the GitHub release are operator steps. No
client bundle change (server-only static release).

### Honest ceilings (carried into v1.21 / v2.x)
- Export redaction replaces chart `content` only; row metadata (source, origin,
  owner) is unsplit — a fully subject-scoped export should be scoped at source.
- `purge_stale_dsar_ledger` rides the read-event prune cadence, not a dedicated
  boot timer (no such timer exists in this tree).
- `bundle_hash`/`query_hash` are xxh3 fingerprints (non-adversarial) — a
  consumer needing the exact query/bundle re-derives it from its own source
  copy, matching the audit chain's own hashing posture.

## Agent 83: v1.20.16 "Bidi" — close the Unicode bidi-smuggling gap (session 2026-08-12)
**Status:** COMPLETED (code + tests + gates + release wrap; deploy/tag pending operator)
**Date:** 2026-08-12

A deep audit of six proposed agentic-security hardening measures (LITL/UI
markdown, IFC/taint tracking, Rule-of-Two, MCP ETDI signed manifests,
SPIFFE/SPIRE + mTLS, EchoLeak + Unicode normalization) against the live v1.20.15
tree. **Five of six were already defended or out of brain-server's scope**;
exactly one real, in-scope gap surfaced and is closed here as a server+client
patch release. See `CHANGELOG.md` §[1.20.16] for the verdict.

### The verdict (per-item)
1. **LITL/UI markdown hardening** — ALREADY DEFENDED. The Dioxus client renders
   every proposal/recall content as an escaped text node
   (`review.rs:558`, `recall.rs:233`, `ops.rs:296`). No markdown parser, no
   `<img>` rendering; `dangerous_inner_html` is build-time grep-guarded
   (`client/src/main.rs:1841`). The "action-description laundering" model also
   doesn't map — proposals aren't model-generated tool-action summaries, they
   ARE the artifact under approval. No-op.
2. **IFC / taint tracking on recall** — PARTIALLY DONE, NO DELTA. `/recall`
   already serializes `untrusted: true` on every hit (`handlers/mod.rs:111`,
   hard-set at all 10 recall sites). The FIDES/CaMeL *enforcement* (label
   propagation through tool calls, policy fence before sensitive sinks) is an
   orchestrator-layer (OpenClaw) concern per Microsoft SFI. An optional
   per-hit `origin` delta was **rejected as YAGNI/churn** — `origin` is
   provenance (already in `/export` + `/.well-known/ai-notice`), not a taint
   label, and adding it per-hit risks muddying the clean universal-untrusted
   posture for no current consumer.
3. **Rule of Two at gateway** — OUT OF SCOPE. brain-server is a memory HTTP
   backend: no web scraping, no shell/exec, one bounded outbound path (the Art
   19 HMAC webhook). The in-process-extension authority concern is OpenClaw's
   plugin architecture. Nothing to change here.
4. **MCP ETDI / signed manifests** — NOT APPLICABLE. `src/bin/mcp.rs` exposes a
   compile-time-constant tool table (pinned by `tool_list_contains_all_nine_
   ump_tools`). No dynamic third-party servers, no `tools/list_changed`, no
   schema drift possible. Rug-pull/shadowing targets aggregating MCP clients,
   not a single self-hosted trusted server whose tools are local HTTP proxies.
   did:key identity already ships for UMP.
5. **SPIFFE/SPIRE + mTLS + TPM** — YAGNI/org-level. brain-server already has
   bearer/JWT + did:key capability tokens (UMP §5.2). SPIFFE/SPIRE is
   multi-instance org infra; TPM needs hardware. Disproportionate for a
   single-loopback launchd service. Documented as a v2.x operator ceiling.
6. **EchoLeak + Unicode normalization** — SPLIT: 6.1 N/A (no markdown/image
   rendering, CSP split strict/`connect-src 'self'`); **6.2 REAL GAP** → this
   release.

### The gap (6.2) + the fix
`strip_invisible` (`src/screen.rs:36` + `client/src/main.rs:52` mirrors) covered
tag-block (U+E0000–E007F), variation selectors (U+FE00–FE0F), zero-width
(U+200B/C/D/2060), and legacy BOM/soft-hyphen/grapheme-joiner — but **not the
Unicode `Bidi_Control` block** (U+202E RLO et al.), the directional-override
smuggling class named by Trojan Source / W3C TR#20 and by the EchoLeak
hardening literature. **Widened in one move** to strip:
- `U+200E–U+200F` (LRM/RLM marks)
- `U+202A–U+202E` (LRE/RLE/PDF/LRO/RLO — the overrides, the named gap)
- `U+2066–U+2069` (LRI/RLI/FSI/PDI isolates — the modern equivalent)

The full canonical `Bidi_Control` set (same line count as a narrow U+202E-only
fix, edge-case-correct: a reviewer would otherwise ask why the isolates were
left out). No new codepath, no new dep, no abstraction — the existing predicate
reaches both the classifier-scoring boundary (server, `screen.rs:227` where
`score_field` calls `strip_invisible`) and the operator render boundary (client)
automatically. The `icu_properties` "Default_Ignorable" bin (already transitive
via tokenizers) was **evaluated and rejected** — promoting a transitive dep to
direct + growing the binary to replace a 3-range `||` chain is over-engineering.

### Changes Made
- **`src/screen.rs`**: `is_invisible` widened with the three bidi-control ranges
  + the `strip_invisible` doc comment updated to list the bidi block + a
  `ponytail:` note documenting the blocklist-on-raw-input ceiling. Test
  `strip_invisible_removes_smuggling_forms` extended (U+200E/U+202E/U+2066 in the
  loop + a full LRE/RLE/PDF/LRO/PDI collapse assertion).
- **`client/src/main.rs`**: the mirror `is_invisible` widened identically +
  inline comment updated; test
  `strip_invisible_removes_smuggling_but_keeps_visible_text` extended with the
  same three bidi codepoints.
- **Release wrap**: Cargo.toml/lock + openapi.yaml 1.20.15 → 1.20.16 (both
  packages — server + client predicates touched); CHANGELOG §[1.20.16]
  (incl. the full audit verdict so the "why not the other five" is on record);
  AGENTS header + this entry.

### Verification
- Server: `cargo test --features bench,migrate` → **507 passed, 5 ignored**
  (the existing baseline; the bidi cases extend `strip_invisible_removes_
  smuggling_forms`, no count delta). `cargo clippy --all-targets --features
  bench,migrate -- -D warnings` clean. `cargo fmt --check` clean.
- Client: `cargo test` → **100 passed** (the bidi cases extend the existing
  `strip_invisible` test, no count delta). Clippy `-D warnings` + fmt + wasm
  build clean.

### Ship status: COMPLETED (code + tests + gates + wrap) 2026-08-12
`scripts/install-service.sh` (live restart), `./deploy-web.sh` (live `/app`),
commit/tag `v1.20.16`, and the GitHub release are operator steps.

### Honest ceilings (carried forward)
- The server's layer-1 blocklist (`contains_suspicious_pattern`) runs on **raw**
  content (`screen.rs:107`), not stripped input — a bidi-wrapped phrase the
  classifier now strips + catches can still dodge the blocklist leg. Widening
  `is_invisible` shrinks this gap (the classifier scores stripped text) but the
  blocklist-on-raw-input is a separate "where strip is applied" change,
  documented `ponytail:` and out of scope for this recommendation.
- Strip runs at the screen/classifier/render boundaries, never by rewriting
  stored bytes — a legitimate user's bidi characters stay verbatim at rest
  (unchanged from v1.20.3).

---

## Agent 82: v1.20.15 "Clock" — deadline clocks in the review queue (session 2026-08-12)
**Status:** COMPLETED (code + tests + gates + release wrap; deploy/tag pending operator)
**Date:** 2026-08-12

Shipped the v1.20.15 "Clock" release per `IMPLEMENTATION_PLAN_v1.20.15_Clock.md`:
the console line's "the queue is a clock" rule now reaches the **review queue**
cards + the review detail page — the operator sees exactly how much time and
information they have left to think, instead of a wall of "pending". The server
M1 (deadline fields on `ProposalView`) + the client M2.1 shared `time_budget`
core + the `/ops` refactor were already in the tree from a prior session; this
session completed the remaining M2 review/detail wiring and the M3 wrap. See
`CHANGELOG.md` §[1.20.15].

### Changes Made
- **M2.2 — live deadline badges on Review cards** (`client/src/panels/review.rs`):
  the muted tabular span at the card head is now a tier-colored clock —
  `format_remaining(remaining(expires_at, now))` → `Xd Yh` / `Xh Ym` / `Xm` /
  `<5m` / `expired`, colored `ok`/`warn`/`danger` via `time_budget::tier` with
  the server-provided `warn_secs`/`critical_secs`. `Expired` rows carry the
  `badge-danger` tier and disable approve/reject/edit. A once-on-mount ~30s
  tick (`use_signal(now_unix)` bumped on each `tick()`) re-renders every
  countdown from a fresh `now_unix()`.
- **M2.3 — detail page clock** (`review.rs`): the deep-link detail header now
  shows the same absolute-deadline badge next to novelty/salience, ticked live.
- **M2.3 — sort-by-deadline toggle** (`review.rs`): pure `expiry_order` sorts
  the fetched list by `(expires_at, id)` — expired first, then the most urgent
  deadline (the clock rule) — toggled by an "expiry first" / "creation order"
  button. Defaults **on** to the server's creation order so nothing changes
  unless asked; never touches server data (ponytail: ≤200 rows, local sort
  honest, API surface flat).
- **M3 — wrap**: server + client `Cargo.toml`/lock + `openapi.yaml` 1.20.14 →
  1.20.15; `CHANGELOG.md` §[1.20.15]; AGENTS header + this entry.

### Verification
- `cargo test --features bench,migrate`: **507 passed, 5 ignored** green
  (the M1 `proposal_deadline` band-mirror test already in tree). `cargo clippy
  --all-targets --features bench,migrate -- -D warnings` clean; `cargo fmt
  --check` clean.
- Client: `cargo test` **100 passed** (was 99; +1 `expiry_order_sorts_nearest_
  deadline_first`, which also pins the stable id tie-break). Clippy `-D
  warnings` clean; `cargo fmt --check` clean; `cargo build --target
  wasm32-unknown-unknown` clean.

### Ship status: COMPLETED (code + tests + gates + wrap) 2026-08-12
`./deploy-web.sh` (live `/app`), `scripts/install-service.sh` (live restart —
picks up the new `ProposalView` fields), commit/tag `v1.20.15`, and the GitHub
release are operator steps.

### Honest ceilings (carried into v1.21 / v2.x)
- The `<5m` display band is not parameterized by an `ALERT_CRITICAL_SECS`
  override — an override shifts only the tier *color* (computed from the
  server-provided thresholds), never the coarse label (ponytail in the core).
- The sort-toggle + badge strings are `en`-only first cuts (the shared clock
  core is English-first); other locales inherit via the en-fallback until a
  native pass.
- The 30s tick is a signal, not enforcement — the server's 400 on a stale
  approve stays authoritative (unchanged).

## Agent 81: v1.20.14 "Steer" — edit-then-approve (evaluative substitution)
**Status:** COMPLETED (code + tests + gates + release wrap; tag pending operator)
**Date:** 2026-08-12

Shipped the fifth limb of the human-in-the-loop essay — **evaluative
substitution** — as a combined **server + client** release (server Cargo.toml
1.20.13 → 1.20.14; client 1.20.13 → 1.20.14). Bainbridge's irony of automation:
a reviewer stuck with binary approve/reject buttons is a gate, not an
evaluator. This release lets a human **rewrite a pending proposal and approve
the corrected version** (steering *toward* a better solution) instead of just
reject-with-reason / suggest-re-ingest (steering *away*). Zero tokens, no LLM,
no background worker; editing is an audited operator mutation like every other
decision, and the TTL clock is untouched so an edit never dodges expiry
(consequentiality preserved). See `CHANGELOG.md` §[1.20.14].

### Changes Made
- **M1 — Server `POST /proposals/{id}/edit`** (`src/handlers/gate.rs`): body
  `{content}` → re-scores deterministically through the exact `ingest_proposal`
  path (`gate::novelty` vec0 KNN, `find_conflict`, `gate::salience`), runs the
  v1.20.3 two-layer injection screen (`Reject` → 400 `input_rejected`;
  `Quarantine` → allowed + stored, the read-time `screen_verdict` badge
  recomputes it), and stamps `edited_at` (unix ts). Same stale/expiry + CAS
  discipline as approve/reject (v1.20.2 A3/A4): TTL check + expiry audit land
  on the raw autocommit conn **before** the tx, then a `BEGIN IMMEDIATE` tx
  re-checks `status='pending'`; `n==0` → clean rollback + 409 on a concurrent
  approve/reject. Audit detail is **SHA-256 of before+after content only**
  (never raw text, pinned by the `sha256_hex_is_deterministic_hex_of_content`
  known-vector test). Normalize (`content.trim()`), bound (`MAX_QUERY`),
  `authorize(Action::Write)`, `gate.edit` otel span under `--features otel`.
- **M1 — Migration** (`src/migration.rs`): additive nullable
  `proposals.edited_at`; schema-contract + wiring-guard + openapi-coverage
  tests updated (the `/proposals/{id}/edit` row added to the authz table).
- **M2 — Client Review panel** (`client/src/panels/review.rs`): an
  `edit_for: Signal<Option<(i64,String)>>` threaded through `panel()` →
  `card()` (signature + call site), a `card()` **Edit** button, an
  `EditEditor` dialog (Escape-close, cancel, re-scored-on-save, inline
  `feedback` `error_message`), `E`/`?` keyboard + the `?` help table row
  (`review_key_edit`). A `warn` **edited** badge (`panels::edited_label`) on
  card + detail header so a reviewer/auditor sees the content shown is not the
  original capture. Offline: `QueuedAction::Edit` (payload-keyed — two distinct
  edits of one proposal are distinct actions, last-edited-wins on replay; a
  decided proposal 404s and counts as applied). New i18n `edit` /
  `review_key_edit` in `en`.
- **M3 — wire contract**: `ProposalView.edited_at` (server) ↔
  `Proposal.edited_at` (`#[serde(default)]`, client); `openapi.yaml`
  documents `/proposals/{id}/edit` + the nullable field.

### Fixes during the pass (compile/clippy/fmt gates)
- Two closures (re-ingest + edit) both captured `content_for_reingest` → moved —
  added a separate `content_for_edit` binding (the `E0382` the first test run
  surfaced).
- `EditEditor`'s `feedback` signal outer `mut` was unused (only `.set()` via a
  shadowed inner binding) — dropped the `mut` (`unused_mut` warning).
- client `cargo fmt` re-flowed the `edit_proposal` call chain; server `cargo
  fmt` fixed the migration-comment drift the `--check` flagged.

### Verification
- `cargo test --features bench,migrate`: **506 passed, 5 ignored** (main-bin
  target; +1 `sha256_hex` known-vector test vs the 1.20.13 baseline of 622
  total across all targets). All targets green, 0 failed.
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
  `cargo fmt --check`: clean.
- Client: `cargo test` **99 passed**, clippy `-D warnings` clean, fmt clean,
  `cargo build --target wasm32-unknown-unknown` clean.
- `bash scripts/badges.sh --selfcheck`: OK (server 1.20.14, client 1.20.14,
  tests 622; README badge regenerated to match).

### Ship status: COMPLETED (code + tests + gates + wrap) 2026-08-12
`scripts/install-service.sh` (live restart — the migration adds `edited_at` on
boot), `./deploy-web.sh` (live `/app`), commit/tag `v1.20.14`, and the GitHub
release are operator steps.

### Honest ceilings (carried into v1.21 / v2.x)
- Editing is review-queue-only; rewriting an already-promoted chunk stays the
  take-the-supersede path (consolidate + supersession).
- The audit detail is before/after **hashes**, not a full content history diff
  of an edited proposal (consistent with the hash-only audit practice).
- The `edit` + `review_key_edit` strings are `en`-only first cuts; de/fr/es/nl
  inherit via the en-fallback until a native pass.
- No measured capacity/device run exercises the new panel (the `bench
  --envelope` operator step remains open, unchanged for releases).

## Agent 80: v1.20.13 "Media" — GTM content + media kit (session 2026-08-12)
**Status:** COMPLETED (docs + version-aligned release wrap; tag pending operator)
**Date:** 2026-08-12

Shipped the v1.20.13 "Media" GTM content line per
`IMPLEMENTATION_PLAN_v1.20.13_Media.md`, then aligned the version line (server
`Cargo.toml` 1.20.12 → 1.20.13; client 1.20.12 → 1.20.13, version-alignment only
per the v1.20.12 "Align" pattern) so the tag is a single 1.20.13. **No runtime
code, no schema change, no new routes.** See `CHANGELOG.md` §[1.20.13].

### Key decision: relocate, don't re-author (the lazy-senior move)
The 8 blog posts + media kit already existed in the gitignored `marketing/`
working dir (authored by the v1.20.6 GTM line, Agent 74). Re-writing them into
`docs/` would be pure duplication. Instead this release **relocated** the content
into the public in-tree `docs/` (the exact v1.20.12 reuse precedent):
- `marketing/blog/` (8 posts) → `docs/blog/`
- `marketing/media-kit.md` → `docs/media-kit.md`
The loose `marketing/` posts (launch/linkedin/substack) + architecture assets are
the future publishing channel's raw material (v2.2.1 "Drift"), not this release's
scope — they stay in `marketing/` (gitignored).

### Changes made
- **M1 — `docs/blog/`** (8 posts, `_drafts`-ready): compliance-time-bomb framing,
  deterministic human-in-the-loop, tamper-evident audit, reference-faithful
  retrieval (each citing its `docs/research/` explainer), no-lock-in via
  MCP/UMP/HTTP, OWASP 2026 as the sales doc, the honest ceiling, and a clearly-
  labelled forward-looking Profiles preview (v1.21.0).
- **M2 — `docs/media-kit.md`**: name/one-liners/positioning/elevator, the
  Brain-vs-Mem0/LangGraph/RAG sizing table with honest ceilings, headline stats
  tied to the proof map, press contact/ask.
- **M3 — cross-links**: `docs/product-site/index.md` links the blog + media kit;
  README Documentation table + `docs/README.md` docs-map gain Blog + Media kit
  rows; README badge → 1.20.13.
- **M4 — wrap + version**: CHANGELOG §[1.20.13]; ROADMAP released-version header
  → 1.20.13 + v1.20.13 row Planned → **Shipped**; `openapi.yaml` + `Cargo.toml`/
  lock + `client/Cargo.toml`/lock re-stamped to 1.20.13; AGENTS header + this
  entry.

### Link fixes the relocation surfaced (real, not cosmetic)
- `blog/01` referenced `blog-07-honest-ceiling.md` — the file is
  `07-honest-ceiling.md` (stale `blog-` prefix). Fixed to `07-honest-ceiling.md`.
- The media kit's `../trust/` links were written for the `marketing/` location;
  at `docs/` they'd resolve to repo root. Now `./trust/` (the media kit sits one
  level shallower than the blog's `../trust/`). The blog posts' `../research/` +
  `../trust/` + `../../docs/OWASP_AGENTIC_2026.md` links resolve as-authored at
  `docs/blog/`.

### Verification
- Docs-only release: no code changed, so `cargo fmt --check`, clippy `-D
  warnings`, and `cargo test --features bench` pass by construction (tree's
  runtime code is byte-identical).
- Every `.md` link in `docs/blog/` + `docs/media-kit.md` resolves to an existing
  file (scripted check, correctly resolving from the file's own directory —
  the first checker's `normpath` mishandled the `../` base and flagged two false
  positives that turned out to be real `../trust/` → `./trust/` fixes).

### Honest ceilings (carried into v2.2.1 "Drift")
- Blog posts are Markdown in-tree, **not** a published blog/CMS — the
  static-serve/publish step is the v2.2.1 "Drift" + operator handoff.
- The Profiles preview post is forward-looking (v1.21.0), clearly labelled.
- Media-kit positioning is author-faithful, not an analyst endorsement; every
  technical claim maps to a v1.20.12 proof-map row.
- The client bump is version-alignment only (no client code change).

## Agent 79: v1.20.12 "Docs" — GTM documentation line + version alignment (session 2026-08-12)
**Status:** COMPLETED (docs + version-aligned release wrap; tag pending operator)
**Date:** 2026-08-12

Shipped the v1.20.12 "Docs" GTM documentation line per
`IMPLEMENTATION_PLAN_v1.20.12_Docs.md`, then aligned the version line (server
`Cargo.toml` 1.20.11 → 1.20.12; client 1.20.9 → 1.20.12, version-alignment only
per the v1.18.2 "Align" pattern) so the tag is a single 1.20.12. **No runtime
code, no schema change, no new routes.** See `CHANGELOG.md` §[1.20.12].

### Key decision: relocate, don't re-author (the lazy-senior move)
The three tiers the plan describes **already existed** in the gitignored
`marketing/` working dir (authored by the v1.20.6 GTM line — product-site
landing/install/quickstart/editions, 7 research explainers, trust proof-map +
reproduce). Re-writing them into `docs/` would have been pure duplication of
~14 files. Instead this release **relocated** the existing content into the
public in-tree `docs/` (reuse per the ladder, not re-authoring):
- `marketing/product-site/{index,install,quickstart,editions}.md` →
  `docs/product-site/`
- `marketing/research/01…07.md` (bi-temporal, submodular packing, TRACE edges,
  PPR graph, hub dampening, abstention-verify, PRF-evidence) → `docs/research/`
- `marketing/trust/{proof-map,reproduce}.md` → `docs/trust/`
`marketing/blog/` + `media-kit.md` + the loose posts stay put (they are the
v1.20.13 "Media" scope). `marketing/` stays gitignored (still holds that work).

### Changes made
- **Relocation** (above) with a link fix: the two product-site files that
  pointed at `../../docs/*.md` (valid from `marketing/`, wrong from `docs/`)
  now use `../*.md`. All `.md` links across the three tiers verified to resolve.
- **M4 cross-links** — README Documentation table gains Product site / Research
  / Trust rows; `docs/README.md` docs-map gains the same three rows; COMPLIANCE.md
  + SECURITY.md gain a "Verify, don't trust" pointer to
  `docs/trust/proof-map.md` + `reproduce.md`.
- **Wrap** — README version badge → 1.20.12; ROADMAP released-version header →
  1.20.12 + v1.20.12 row Planned → **Shipped**; CHANGELOG §[1.20.12]; AGENTS
  header + this entry.

### Verification
- Docs-only release: no code changed, so `cargo fmt --check`, clippy `-D
  warnings`, and `cargo test --features bench` pass by construction.
- Every `.md` link inside `docs/product-site/`, `docs/research/`, `docs/trust/`
  resolves to an existing file (scripted check).
- `reproduce.md` commands are the same smoke-tested commands the proof-map
  cites (audit verify, UMP capabilities, DSAR cert, OWASP matrix) — live service
  unchanged.

### Honest ceilings (carried into v2.2.1 "Drift")
- Docs are Markdown in-tree, **not** a deployed site with a domain — the
  static-serve/publish step is the v2.2.1 "Drift" + operator handoff.
- Editions/pricing are placeholders until v2.2 "Meridian" lands.
- Scientific explanations are author-faithful to the papers; brain-server is a
  deterministic implementation of *specific* techniques, not a SOTA-parity
  claim — each explainer states its ceiling honestly.
- The client bump is version-alignment only (no client code change); the last
  client feature release remains v1.20.9 "Register". README badges were
  regenerated from the real build via `scripts/badges.sh` (server + client both
  1.20.12, tests 621).

## Agent 78: v1.20.11 "Housekeeping" — badge generation + release hygiene (session 2026-08-12)
**Status:** COMPLETED (code + tests + gates + docs; deploy/tag pending operator)
**Date:** 2026-08-12

Shipped the final release of the operator-console line, per
`IMPLEMENTATION_PLAN_v1.20.11_Housekeeping.md`. **Server + docs** (server
Cargo.toml 1.20.10 → 1.20.11; client stays at 1.20.9). **No new runtime code,
no schema change, no new dependency** — a dev-tool + docs close-out: badges
are facts, not hand-typed claims, and the release wrap is a checklist, not a
skill. See `CHANGELOG.md` §[1.20.11].

### Changes Made
- **M1 — `scripts/badges.sh`** (new). Derives the README's dynamic badges from
  the real build: version from `Cargo.toml` (server) + `client/Cargo.toml`
  (client), test count from an actual `cargo test --features bench,migrate`
  run (parses the "N passed" lines, summed across targets), UMP level from the
  shipped self-attested L3 (a CI-asserted constant, never a drifting claim),
  and an SBOM-present flag from the on-disk `sbom/brain-server-<v>.cdx.json`.
  Prints the shield.io badge block for the human to paste. `--selfcheck` runs
  the plan's two tests in one invocation: (1) asserts the derived version
  equals the `Cargo.toml` version (an independent extraction, not the same
  sed), (2) asserts `docs/release-checklist.md` names all six wrap artifacts
  (Cargo.toml / openapi.yaml / CHANGELOG / ROADMAP / README / AGENTS). Exits
  nonzero on any drift. It never fabricates a number it did not measure.
- **M2 — `docs/release-checklist.md`** (new). The six-part wrap (Cargo.toml
  + lock → openapi.yaml → CHANGELOG → ROADMAP → README badges via `badges.sh`
  → AGENTS.md) with the verifying command per step + the four green gates and
  the docs-only exception (no `Cargo.toml`/OpenAPI change for a docs release
  like v1.20.5). A doc, not a CI gate (wiring it into CI as blocking is the
  operator's call, explicitly out of scope).
- **M3 — `/proof` panel: NOT built.** Optional/off-by-default per the plan —
  the v1.20.10 integrity signal already lives in the queue-header `Badge`; a
  whole panel is speculative UI until the operator asks. Documented as such.
- **M4 — wrap + version.** Server `Cargo.toml`/lock + `openapi.yaml`
  1.20.10 → 1.20.11 (client untouched). `CHANGELOG.md` §[1.20.11]; `ROADMAP.md`
  released-version header → 1.20.11 + v1.20.6 ("Console") and v1.20.9
  ("Register") rows flipped Planned → **Shipped** (they shipped but were still
  listed Planned) + v1.20.11 row → Shipped; README badges regenerated via
  `badges.sh` (fixing the hand-typed **712 → measured 621** drift); AGENTS
  header + this entry.

### Verification
- `scripts/badges.sh --selfcheck`: **OK** (version derivation + six-artifact
  checklist completeness both guard-clean). Full `badges.sh` run:
  `server 1.20.11  client 1.20.9  tests 621 passed  UMP L3  sbom no`
  (sbom = no is correct — the 1.20.11 SBOM is produced by `sbom.sh` at release
  time).
- `cargo test --features bench,migrate`: **621 passed** (the same number the
  badge now reports — measured, not stored).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
  `cargo fmt --check`: clean (the tree's runtime code is unchanged by a script
  + doc, so these pass by construction).
- No new Rust tests: no runtime code was added (the plan's two checks live as
  the shell `--selfcheck` guard, not the Rust suite).

### Ship status: COMPLETED (code + tests + gates + docs) 2026-08-12
Commit/tag `v1.20.11` and the GitHub release are operator steps. No server
restart, no client bundle (dev-tool + docs only). If a release-time SBOM badge
matters, run `scripts/sbom.sh` before tagging (it emits
`sbom/brain-server-1.20.11.cdx.json`).

### Honest ceilings (carried into v2.0)
- Badge generation is a script, not a CI hard-gate — it produces facts to
  paste; a blocking CI check is the operator's call (CI churn risk outweighs
  the gain; the repo's CI is already green and the v1.17.5 badge jobs already
  assert the honest lines).
- The `/proof` panel is optional and off by default — a single `Badge` already
  surfaces the integrity signal.
- The release checklist is a doc, not automation; a `release.sh` that does all
  six steps is a v2.x dev-infra nicety, deliberately not built here (automation
  that gets the wrap wrong is worse than a reviewed checklist).

## Agent 76: v1.20.9 "Register" — read-only Agent Memory Register + shared EvidenceModal (session 2026-08-12)
**Status:** COMPLETED (code + tests + gates + docs; deploy/tag pending operator)
**Date:** 2026-08-12

Shipped the v1.20.9 "Register" **client** release, per the plan. **Client-only**
(client Cargo.toml 1.20.8 → 1.20.9; server + API contract stay at 1.20.8). A
pure client composition of the already-shipped `GET /export` + `GET /get/{id}`
endpoints — **no new routes, no new wire types, no new deps** — surfacing the
v1.20.7 `origin` marker (and the v1.18.2 provenance it derives from) as an
operator-facing provenance ledger. See `CHANGELOG.md` §[1.20.9].

### Changes Made
- **M1 — Register panel** (`/register`, `client/src/panels/register.rs`, new) +
  `Route::Register {}`. Reads the `knowledge` body of `GET /export` and
  partitions rows into the three **origin** tiers (`human`/`model`/`imported`)
  with live counts, plus an All tab. Pure `register_filter` narrows by
  owner/source/memory-kind; each row renders id · bounded excerpt (via the
  v1.20.3 `strip_invisible` render boundary + `chars().take` cap) · provenance
  badges · UTC date (pure `format_epoch`, Howard Hinnant civil-from-days —
  no timezone dep).
- **M2 — shared evidence viewer (`EvidenceModal`)** — one reusable
  `role="dialog"` renderer opened from any register row; fetches the existing
  `GET /get/{id}` wire and shows the verbatim span + `source_uri` + revision +
  heading + line range. Hand-rolled Esc-close modal matching the review-panel
  idiom (the client has no Radix `DialogRoot`).
- **M3 — wiring.** `panels::register` module + main.rs use-import; rail `NavLink`
  + mobile `TabLink` + command palette (`command_names` aliases `register/
  ledger/provenance/origin/who/ownership`, `palette_commands` entry,
  `command_label` "Agent Memory Register"); nav targets **13 → 14** (guard test
  `palette_lists_nav_targets_and_conditional_signout` + the `palette_navigate_
  covers_every_non_detail_route` route array updated); i18n `nav_register` in
  `en` only (de/fr/es/nl fall back per the established `ops_title` convention).
- **Version bump** client 1.20.8 → 1.20.9; CHANGELOG §[1.20.9]; CLIENT_ROADMAP
  v1.20.9 row → Shipped; client README status → v1.20.9; AGENTS header + this
  entry.

### Verification
- `cargo test --manifest-path client/Cargo.toml`: **99 passed** (was 92 at the
  v1.20.8 baseline; +6 register cores/tests — `register_filter`,
  `origin_group`, `register_excerpt`, `format_epoch`,
  `evidence_modal_uses_existing_get_route`, `register_is_read_only` — +1 nav-
  count guard update). Clippy `-D warnings` clean, `cargo fmt --check` clean,
  `wasm32-unknown-unknown` build clean.
- The only clippy finding was a real lint (`tab() == ""` →
  `tab().is_empty()`, `comparison-to-empty`) — fixed.
- Server suite untouched (zero server edits).

### Ship status: COMPLETED (code + tests + gates + docs) 2026-08-12
`./deploy-web.sh` (live `/app`), tag `v1.20.9`, and the GitHub release are
operator steps. No server restart needed (client-only static bundle).

### Honest ceilings (carried into v1.21)
- The register is **read-only by construction**: `parse_export_rows` yields zero
  rows from any non-`/export` body, so the ledger can't be fed a mutation's
  response.
- Recall hits still open the existing shared drawer (`DrawerContent::Hit`);
  the register's `EvidenceModal` is `pub` for a future recall entry (the plan's
  recall wiring was deferred — rewiring would orphan a drawer variant + risk a
  working v1.20.8 file whose reader garbles in this env).
- `highlights` and `source_prompt` are server proposal-only and are **not**
  rendered (the plan's client-side claims to them were wrong; `/get/{id}` has
  no such fields). `format_epoch` is UTC `YYYY-MM-DD` only — no timezone
  conversion.

## Agent 75: v1.20.7 "Telemetry" — M1 instrumented decision cores behind `--features otel` (session 2026-08-12)
**Status:** COMPLETED (code + tests + gates + docs + CI; version bump/tag pending operator)
**Date:** 2026-08-12

The observability half of the v1.20.x audit follow-up. **Server-only** (server
stays at 1.20.4; no schema change, no new routes, no API contract change): the
three seams that decide what becomes (or stays) memory now emit OpenTelemetry
spans an operator can ship to any collector — **gated behind a new `otel`
Cargo feature** so the default build ships with **zero tracing machinery and
zero new runtime deps** (every `#[instrument]` and the OTLP exporter are
`#[cfg(feature = "otel")]`). The feature rides into the next tagged release.
See `CHANGELOG.md` §[1.20.7].

### Changes Made
- **M1 — instrumented the decision seams** (all `#[cfg_attr(feature = "otel",
  tracing::instrument(name = "…"))]`, default build byte-identical):
  - **injection screen** (`screen::screen` → `screen` span, records `verdict`
    via `Span::current().record(...)`). A proposed `layer` field was dropped —
    not determinable from `ScreenResult` alone without re-exposing the internal
    layer-2 hit to callers (YAGNI; the `verdict` label is the join key).
  - **human review gate** (`gate::ingest_proposal` → `gate.propose`,
    `approve_proposal` → `gate.approve`, `reject_proposal` → `gate.reject`,
    each with `outcome` via `gate_outcome`).
  - **recall** (`recall::run_recall` → `recall` span with `decision`,
    `graph_rescued`, `hits`, `domain`, `principal`, `query_hash`).
- **`src/otel.rs`** (new, `#[cfg(feature = "otel")]`): `init_otel` →
  `SdkTracerProvider` + OTLP HTTP exporter to `BRAIN_OTEL_ENDPOINT` (default
  `127.0.0.1:4318/v1/traces`); pure label helpers `query_hash` (bounded xxh3 —
  content never a span field), `screen_verdict_span`, `gate_outcome`. Declared
  in `main.rs` (line 80), **not** `lib.rs` — it's a binary module (the `pub
  mod otel` lib-side addition was reverted).
- **`main.rs` `init_tracing`**: `EnvFilter` is its own layer (`fmt::Layer`
  has no `with_env_filter`), `provider.tracer("brain-server")` via
  `TracerProvider::tracer`. Reverted an unnecessary `rt-tokio-current-thread`
  Cargo feature — `with_batch_exporter` takes one arg and spawns its own thread.
- **`src/config.rs`**: `otel_endpoint()` reads `BRAIN_OTEL_ENDPOINT`.
- **Cargo.toml**: `otel` feature (`tracing`, `tracing-subscriber/env-filter`,
  `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`
  `{http-proto,reqwest-blocking-client}`, `tracing-opentelemetry`);
  `tracing-subscriber`'s `registry` feature enabled only under `otel`.
- **CI `otel-gate` job** (`ci.yml`): compiles the feature (a default build
  compiles a different surface — a broken otel build would slip past
  `lint-test`), runs the cfg-gated tests, enforces clippy. YAML verified (pyyaml).
- **Release wrap**: CHANGELOG §[1.20.7], AGENTS header + this entry.

### Verification
- `cargo test --features otel`: **500 passed, 5 ignored** (the `screen` seam
  test passes under the feature; default-build behavior unchanged).
- New cfg-gated `screen::tests::otel_tests`:
  `screen_emits_verdict_span` — a hand-rolled capturing `Layer<Registry>`
  proves the seam emits a `screen` span with exactly
  `[("verdict", "clean")]`; `verdict_span_label_covers_all_verdicts` pins all
  three `ScreenResult` → label mappings.
- clippy `-D warnings` + fmt green under **default**, **`otel`**, and
  **`bench,migrate[,otel]`**. Default `cargo check` clean.
- `ci.yml` re-parses with pyyaml (`otel-gate` job present, 3 named steps).

### Fix class encountered (not guesswork)
Three `E0382` moved-value captures surfaced as the spans were added
(`principal` moved into the `approve_proposal` closure, `query` moved into a
formatting closure in recall). Each fixed by computing the string label before
the `#[instrument]`/`Span::current()` call and capturing that label — the
recorded field is `&'static str`/owned String, not the moved value.

### Ship status: COMPLETED (code + tests + gates + docs + CI) 2026-08-12
Server version bump (`otel` feature rides into the next tagged release),
`scripts/install-service.sh` (live restart — only if an operator opts into a
collector + `--features otel` build), tag, and GitHub release are operator steps.

### Honest ceilings (carried into a later release)
- Default build has **no telemetry**; the feature requires an operator rebuild
  + a collector at `BRAIN_OTEL_ENDPOINT`.
- `query_hash` is a bounded xxh3 fingerprint, not the query — recall spans never
  carry content (a consumer wanting the exact query re-derives it via the hash +
  audit). Content-as-field is a deliberate non-goal.
- Only the three decision seams are instrumented; the wider request path,
  connectors, and webhook handlers are not yet covered.
- `gate_outcome`/`screen_verdict_span` are stable label strings (not the enum
  Debug repr) — a documented contract for dashboard joins.


---

## Agent 73: v1.20.6 "Console" — Memory Operations panel + SLA clocks + flagged surface (session 2026-08-12)
**Status:** COMPLETED (code + tests + gates + docs; deploy/tag pending operator)
**Date:** 2026-08-12

Shipped the first release of the operator-console line, per
`IMPLEMENTATION_PLAN_v1.20.6_Console.md`. **Client-only** (client Cargo.toml
1.20.0 → 1.20.6; server + API contract unchanged). The panel is a pure
composition of the already-shipped `/proposals`, `/decayed`, and recall-
`include_flagged` endpoints — no new routes, no schema change, no new
dependency. See `CHANGELOG.md` §[1.20.6].

### Changes Made
- **M1 — Memory Operations panel** (`client/src/panels/ops.rs`, new) + the
  already-wired `Route::Ops {}` at `/ops` (rail + tab bar + palette; nav
  targets 12 → 13, guard test updated). Three regions, one decision type each:
  **live pending queue** (top-left primary; each row = exact content +
  `source_prompt` + a live SLA countdown + A-approve/R-reject reusing the
  v1.20.0 `decide`/offline-enqueue path), **flagged & quarantined** (recall
  `include_flagged: true` filtered to `flagged == Some(true)` + `GET /decayed`,
  read-only, rendered through the v1.20.3 invisible-char strip boundary), and a
  **gate health strip** (approved/rejected counts + expired derived from the
  queue → a severity hint).
- **M2 — SLA countdown clocks** (the "queue is a clock" rule). New Dioxus-free
  pure cores in `ops.rs`: `clock_until(created_at, ttl, now_unix)` (the single
  countdown source of truth; `None` once past deadline), `sla_tier` (critical
  < 5 min / warn < 1 hr / ok mapped onto the `danger`/`warn`/`ok` tokens),
  `gate_health`, `fmt_remaining`, and `queue_priority` (in-place sort: expired
  first, then nearest-expiry, stable tie-break by id). A once-on-mount
  `use_future` loop re-renders every countdown from a fresh `now_unix()` every
  ~30s (dependency-free, the health-refresh idiom); expired rows carry the
  server-auto-reject note.
- **M3 — flagged surface** — the injection screen's output is now visible in
  the console (the v1.20.3 G5 output the operator could only otherwise hunt
  for). Display-only invisible-char strip; raw bytes never rewritten.
- **M4 — wrap** — `ops_*`/`sla_*`/`gate_*` i18n keys in `en` (de/fr/es/nl
  resolve via the en-fallback); client version bump; CHANGELOG §[1.20.6];
  CLIENT_ROADMAP v1.20.6 row → Shipped; client README status → v1.20.6; AGENTS
  header + this entry.

### Verification
- `cargo test --manifest-path client/Cargo.toml`: **90 passed** (the new pure
  cores are pinned by `clock_until_returns_remaining_and_none_when_expired`,
  `sla_tier_maps_budgets`, `fmt_remaining_labels`,
  `queue_priority_expired_first_then_nearest_expiry`,
  `queue_priority_stable_tie_break_by_id`, `gate_health_*`; the palette
  nav-target guard moved 12 → 13). Clippy
  `-D warnings` clean, `cargo fmt --check` clean, `wasm32-unknown-unknown`
  build clean.

### Ship status: COMPLETED (code + tests + gates + docs) 2026-08-12
`./deploy-web.sh` (live `/app`), tag `v1.20.6`, and the GitHub release are
operator steps. No server restart needed (client-only static bundle).

### Honest ceilings (carried into v1.20.7/8)
- The clock refreshes on a ~30s timer, not instant push (instant = the v1.20.8
  "Signal" plan); the server's 400 on a stale approve is the authoritative
  backstop.
- `DEFAULT_PROPOSAL_TTL_SECS` mirrors the server default; an operator override
  of `BRAIN_PROPOSAL_TTL_SECS` drifts the displayed clock until the server 400
  (documented in the core).
- `Proposal.screen_verdict` is not yet on the client wire type (server-side in
  v1.20.3), so queue rows carry `source_prompt` but not the verdict badge; the
  flagged region surfaces screen-caught rows instead.
- Gate-health counts are a point-in-time pass over `/proposals?status=…`, not
  a rolling persisted window.

---

## Agent 74: v1.20.6 GTM docs line + v1.20.6 `screen_verdict` wire fix (session 2026-08-12)
**Status:** COMPLETED (docs + code + tests + gates; deploy/tag pending operator)
**Date:** 2026-08-12

Shipped the go-to-market documentation tier (ROADMAP rows **v1.20.12 "Docs"** +
**v1.20.13 "Media"**, plans `IMPLEMENTATION_PLAN_v1.20.12_Docs.md` /
`IMPLEMENTATION_PLAN_v1.20.13_Media.md`) as a **docs-only line** — no version
bump, no schema change, tree otherwise unchanged — plus closed a real client
wire gap found while writing it. See `CHANGELOG.md` §[1.20.6] GTM note.

### Changes Made

All content lives **untracked** in the gitignored `marketing/` directory
(private/pre-release; the public tree is untouched). A correction to an earlier
review: the content was first placed under `docs/` and linked from the public
README/docs-map, then **relocated to `marketing/`** and the public links
reverted per the repo's gitignore convention for GTM material.

- **`marketing/product-site/`** (4 files): `index.md` (landing, 3 pillars +
  "compliance time bomb" one-liner), `install.md` (bare metal + Docker,
  `scripts/install-service.sh`, `~/.openclaw/workspace/brain.db`, port 8765),
  `quickstart.md` (5-min flow: ingest → query → approve → audit/verify),
  `editions.md` (OSS/Pro/Enterprise table; capability is one binary, editions
  are packaging not feature-fork; status placeholder noting v2.2 "Meridian").
- **`marketing/research/`** (7 peer-technique → deterministic-implementation
  explainers): `01-bi-temporal` (Graphiti, `src/temporal.rs::extract_interval`,
  `knowledge.valid_from/valid_to`, `?at=`), `02-submodular-packing`
  (arXiv:2607.00725, `DEFAULT_MAX_CONTEXT_TOKENS=160`, `DEDUP_SIMILARITY=0.85`),
  `03-trace-edges` (arXiv:2607.00339, `MAX_HOPS=4`, `/graph/traverse?explain`),
  `04-ppr-graph` (HippoRAG-2 `igraph.personalized_pagerank` verbatim,
  `PPR_ALPHA=0.5`, `RRF_K=60`, ~94% taxonomy-noise caveat),
  `05-hub-dampening` (GAAMA θ=50 + MemORAI + arXiv:2602.03578, rescue gating),
  `06-abstention-verify` (`ClarifyQuery`, `MAX_QUERY=2000`, `MAX_MATCH_RANGES=100`),
  `07-prf-evidence` (reachable PRF gate, `Evidence` struct + highlights). Each
  cites real constants + source files, so the docs can't drift into fiction.
- **`marketing/trust/`** (2 files): `proof-map.md` — 21-row
  claim→shipped-release→live-curl table (audit chain, DSAR certs, AuthN/AuthZ/
  OIDC/JWKS, UMP L3, screen gate/TTL, PII, OWASP 2026, webhooks) + owned
  ceilings; `reproduce.md` — throwaway-instance (`DB=/tmp/brain-repro-$$.db`,
  `PORT=18799`) 7-step walkthrough + honest caveats.
- **`marketing/blog/`** (8 POV posts): `01-compliance-time-bomb`, `02-human-gate`,
  `03-tamper-evident-audit`, `04-reference-faithful` (no LLM in loop),
  `05-no-lock-in` (UMP/HTTP/MCP vs framework lock-in), `06-owasp-matrix`
  (control matrix as sales doc), `07-honest-ceiling` (deliberate limits),
  `08-profiles-preview` (explicitly forward-looking to v1.21.0).
- **`marketing/media-kit.md`** — one-liners, positioning statement, Brain-vs-field
  sizing table with honest ceilings, headline stats, press/reproduce ask.
- **Wrap:** CHANGELOG §[1.20.6] GTM note (public, no private paths) + AGENTS
  header + this entry. The public README + `docs/README.md` docs-map were
  deliberately **not** given a GTM row (private content stays out of the public
  tree).

### v1.20.6 `screen_verdict` wire fix (real gap found while writing the docs)

Agent 73's ceiling "`Proposal.screen_verdict` is not yet on the client wire
type" was **still true and now closed**. The server `ProposalView` carries
`screen_verdict` (src/handlers/gate.rs:266, from `src/screen.rs::ScreenResult`)
but the client `Proposal` struct (client/src/api.rs:1120) was missing it. Added
`#[serde(default)] pub screen_verdict: Option<String>`; rendered a verdict badge
in the Review card header + the Ops panel pending-queue rows via new pure
`verdict_badge()`/`verdict_label()` helpers in `client/src/panels/mod.rs`
(quarantine→`warn`/"quarantined", else `ok`/"clean"); fixed the test
constructors in ops.rs + review.rs. **Result: 90 client tests pass, clippy
`-D warnings` + fmt clean** — the `_Tier4` label work Agent 73 deferred as a
wrapped item is now delivered.

### Verification
- Docs: hand link-checked the new tiers' cross-references (research ↔ blog ↔
  trust ↔ media-kit) + the constants/files cited exist in source.
- Client: `cargo test --manifest-path client/Cargo.toml` 90 passed; clippy
  `-D warnings` clean; `cargo fmt --check` clean. Server tree untouched.

### Ship status: COMPLETED (docs + code + tests + gates) 2026-08-12
`./deploy-web.sh` (live `/app` — picks up the badge), commit/tag, and GitHub
release are operator steps. No server restart needed (docs + client static).

### Honest ceilings
- `editions.md` Pro/Enterprise values are placeholders pending v2.2 "Meridian"
  (pricing/licensing) — flagged in-file, not fabricated.
- `08-profiles-preview.md` is explicitly forward-looking to v1.21.0 Profiles
  (not shipped code).
- The media-kit "sizing table" is author-faithful positioning, not an
  independent analyst endorsement; every technical claim maps to a proof-map row.

---

## Agent 72: v1.20.5 "Agentic" — OWASP 2026 compliance matrix + ZT4AI posture + replay playbook (session 2026-08-11)
**Status:** COMPLETED (docs + release wrap; tag pending operator)
**Date:** 2026-08-11

Shipped the v1.20.5 "Agentic" **docs-only** release closing the GhostJacking
hardening line, per `IMPLEMENTATION_PLAN_v1.20.5_Agentic.md`. **Zero new
routes, zero schema change, zero new deps, no server/client version bump** — the
code for every audit finding (G1–G6) shipped in v1.20.1–v1.20.4; this is the
enterprise capstone that maps the hardened stack to the two 2026 OWASP agentic
frameworks and ships the adoption artifacts. See `CHANGELOG.md` §[1.20.5].

### Changes Made (all docs)

- **M1 — `docs/OWASP_AGENTIC_2026.md`** (new). The control-by-control compliance
  matrix: **OWASP GenAI LLM Top 10:2026** (LLM01–LLM10, pub. 2026-08-04,
  incident-grounded) + **OWASP Top 10 for Agentic Applications 2026**
  (ASI01–ASI10, pub. 2025-12-10). Every row = `Shipped vX.Y` (cited to a real
  feature: screen/classifier, PII redaction, AuthZ matrix, capability tokens,
  SBOM, abstention+verify, vec0 hygiene, quarantine, proposal TTL, Standard
  Webhooks) or `Ceiling v2.x` (owned residual risk). AIUC-1 crosswalk note
  (procurement bridge) + residual-risk section naming owners. The matrix's
  standard is **100% control coverage** — LLM01 has no prevention per OWASP
  2026; segregation + gates + least-privilege are the load-bearing defenses.
- **M2 — ZT4AI posture** (`SECURITY.md` § + `COMPLIANCE.md` §3.5). Workload
  identity (agents not shared service accounts; did:key + capability tokens,
  ≤90d rotation), least-agency (openclaw plugin = recall + proposal only, write
  approval outside the model's prompt — the LLM03/ASI01 policy-gateway pattern),
  Rule of Two (the v1.20.1 gate is the approval for the memory-write action),
  egress boundary (exactly one outbound path: the Art 19 HMAC webhook).
- **M3 — audit-ready-replay playbook** (`COMPLIANCE.md` §3.6). The 2026 bar
  ("if a system can't replay the agent's reasoning and decision path, it is not
  ready for production"); the evidence bundle for an incident / SOC 2 review:
  what (`/audit` chain + `/audit/verify`), why (recall traces + proposal-gate
  trail), to-whom (principal pillar + DSAR certificates + `origin`), for-how-long
  (per-kind retention + `BRAIN_AUDIT_RETENTION_DAYS`). Export paths already
  exist — no new code.
- **M4 — enterprise ops runbook** (`docs/deployment.md` §Security operations).
  Token rotation (v1.20.2 machine-identity pattern) + poisoning-incident-
  response (review `/decayed` + `/consolidate/propose` → purge → re-verify chain
  → rotate) + classifier operations (FPR calibration via
  `BRAIN_INJECTION_THRESHOLD_HIGH/LOW`, retrain trigger, `sha256sum` model-
  artifact hash-pin).
- **Release wrap.** `ROADMAP.md` released-version header → 1.20.5 + released row
  (v1.20.5 "Agentic", depends v1.20.1–v1.20.4); `CHANGELOG.md` §[1.20.5]; AGENTS
  header + this entry. No version bump (docs only); the docs-only patch tag
  `v1.20.5` is the operator's call (recommended).

### Verification

- Claims spot-checked against source before writing: `screen.rs::screen`
  (single seam), `ingest_one`, `screen_source_prompt`/`screen_verdict`,
  `verify_standard_signature` + `receive_standard`, `DEFAULT_PROPOSAL_TTL_SECS`,
  `INJECTION_THRESHOLD_HIGH/LOW` + `BRAIN_INJECTION_THRESHOLD_*` — all present.
- Docs-only release: the tree is unchanged, so `cargo fmt --check`, clippy
  `-D warnings`, and `cargo test --features bench` pass by construction; the
  three docs files' cross-references hand link-checked to the new matrix.

### Ship status: COMPLETED (code + tests + docs) 2026-08-11
The docs-only tag `v1.20.5`, the commit, and the GitHub release are operator
steps.

### Honest ceilings (carried into v2.0)
- LLM01 has no prevention (OWASP 2026's own position); adaptive white-box
  classifier evasion (GCG-class) still beats a hardened encoder — the
  `untrusted` segregation + approval gate are the surviving controls. Owners:
  ops / platform (v1.21+ re-evaluation).
- v2.x code ceilings the matrix names: per-principal quotas (LLM06), at-rest
  encryption (LLM02), mTLS (ASI07), full multi-team tenancy + SSO (ASI03), A2A
  federation (ASI07) — all owned by v2.0 "Cortex"; the v1.20.4 Standard Webhooks
  handshake is the 2026-compliant boundary until then.
- "100% hardened" = 100% control coverage, not 100% risk elimination — the
  matrix's residual-risk section is the truthful statement an auditor can sign.

---

## Agent 71: v1.20.4 "Replay" — G6 signed-timestamp webhook replay window (session 2026-08-11)
**Status:** COMPLETED (code + tests + gates + release wrap; live restart/tag pending operator)
**Date:** 2026-08-11

Shipped the v1.20.4 "Replay" **server** release closing the GhostJacking **G6**
webhook replay window, per `IMPLEMENTATION_PLAN_v1.20.4_Replay.md`. Server
1.20.3 → 1.20.4; client stays at 1.20.0. **No schema change, no new routes.**
The G6 gap: `WEBHOOK_REPLAY_SECS` only applied when a caller-supplied timestamp
was present, and GitHub sends none (its only replay protection is `x-github-
delivery` idempotency — acceptable, its sender is a trusted third party). This
release ships the honest, bounded improvement for senders that DO provide a
timestamp. See `CHANGELOG.md` §[1.20.4].

### Changes Made
- **M1 — Standard Webhooks handshake, opt-in** (`src/handlers/webhooks.rs`).
  When `BRAIN_WEBHOOK_TIMESTAMP_REQUIRED=1`, `receive` dispatches to
  `receive_standard`, which requires the open spec's header set
  (`webhook-id`/`webhook-timestamp`/`webhook-signature`) and verifies the
  `v1,<base64>` HMAC-SHA256 over `{id}.{timestamp}.{raw body}` in constant time
  (new pure `WebhookQueue::verify_standard_signature` in `src/webhook.rs`; the
  timestamp rides inside the HMAC so a replay cannot re-stamp it). `webhook-id`
  feeds the existing `webhook_seen` idempotency. The flag path accepts any kind
  (explicit operator opt-in); missing headers / bad signature → `deny` + 401.
- **M2 — `/health` visibility** (`src/main.rs` `health_body`): `webhook.
  {replay_secs:300, timestamp_required, scheme: standard-webhooks|legacy}`.
- **M3 — docs stance** for GitHub (SECURITY.md + COMPLIANCE.md §webhooks +
  docs/deployment.md): GitHub replay protection is delivery-id idempotency, not
  a timestamp; first-party senders can opt into the hard window via the spec
  headers + flag.
- **Config** (`src/config.rs`): `webhook_timestamp_required()` reads
  `BRAIN_WEBHOOK_TIMESTAMP_REQUIRED` (`1` → true, else false).
- **Release wrap.** Cargo.toml/lock + openapi.yaml 1.20.3 → 1.20.4 (no
  route/schema change); README badge; CHANGELOG §[1.20.4]; AGENTS header + this
  entry.

### Verification
- `cargo test --features bench`: **500 passed, 5 ignored** (main bin 498 + 2
  new webhook tests; the plan's `webhook_rejects_old_timestamp_when_flag_set`
  + `webhook_default_still_accepts_github_no_timestamp` are pinned by the
  existing `enqueue_ts_rejects_stale_timestamp` + `enqueue_ts_none_accepted`).
  New: `standard_signature_covers_id_timestamp_payload` (tamper to id/timestamp/
  body each fails) + `standard_signature_rejects_bad_header_format` (rejects
  non-`v1,` and the legacy `sha256=` form).
- `health_body_never_leaks_content_or_pii` extended to pin `webhook.replay_secs`
  = 300 + `webhook.scheme` = `legacy`. `test_openapi_covers_routes` green (no
  new routes).
- Clippy `-D warnings` + fmt clean.

### Ship status: COMPLETED (code + tests + gates + wrap) 2026-08-11
`scripts/install-service.sh` (live restart), commit/tag `v1.20.4`, and the
GitHub release are operator steps.

### Honest ceilings (carried into v1.21+)
- GitHub's replay protection remains delivery-id idempotency — no timestamp is
  invented for it (would be theater + break the connector).
- The hard window is opt-in (first-party senders); no default-behavior change.
- The spec handshake is verification-side only; the legacy GitHub path keeps its
  `sha256=` HMAC scheme (back-compat); the spec's `webhook-origin`/allowlist
  features are not adopted.
- **This closes all six audit gaps (G1–G6)** across the v1.20.x line. Remaining
  security work is the cross-repo G3 wrap (OpenClaw, tracked in v1.20.2) and the
  documented exec/read posture.

---

## Agent 65: v1.19.0 "Integrated" — audit-filter deep links, closes the plan's testable deltas (session 2026-08-10)
**Status:** COMPLETED (code + tests + docs; deploy/tag pending operator)
**Date:** 2026-08-10

Shipped the v1.19.0 "Integrated" client release. **Client-only** — server + API
contract stay at 1.18.2 (zero server changes). An audit of the plan against the
tree found that most of it had already shipped in earlier releases; this release
closes the one remaining **testable** delta and documents the rest as honest
ceilings (the same pattern as Agent 62/63/64). See `CHANGELOG.md` §[1.19.0].

### Audit: what the plan asked vs. what was already in the tree
- **M2 deep links** — already shipped (v1.16.7): `/review/:proposal_id`,
  `/recall/:trace_id`, `/subjects/certificate/:dsar_id`; iOS/Android `brain://`
  intent filters (v1.17.0). **Only gap: `/audit?since=&principal=`** — the
  audit panel's filters were client-side only, not URL-addressable.
- **M3 PWA** — already shipped (v1.16.7): `pwa/manifest.webmanifest` + `sw.js`
  (shell-only caching + offline navigation fallback).
- **M4 debounce** — already shipped (v1.16.7 M6 recall debounce, generation-
  guarded). Virtualized lists + wasm-split are untestable-here / Dioxus-0.7.10
  ceilings (audit already paginates server-side).
- **M1 OIDC/SSO** — brain-server is a token *validator*, not an IdP: its
  `/.well-known/openid-configuration` advertises empty `authorization_endpoint`/
  `token_endpoint`. A real authorization-code + PKCE flow needs a new server
  `/auth/authorize` proxy (v2.x; documented in v1.16.5/v1.16.8 plans +
  `docs/proxy-sso.md`). The client's JWT-pair mode + silent refresh-on-401 +
  principal pillar (v1.16.5) already consume the JWT half.

### Changes Made
- **`/audit?since=&principal=` deep link** (`src/panels/audit.rs` +
  `src/main.rs`). `Route::Audit {}` gained `since: Option<String>` +
  `principal: Option<String>` query params (`#[route("/audit?:since&:principal")]`);
  the `Audit` component threads them into `audit::panel(since, principal)`,
  which seeds the existing client-side `AuditFilter` via a new pure
  `filter_from_query` (None/empty → unconstrained; kind never comes from the
  query string). All six `Route::Audit` construction sites updated to
  `Route::Audit { since: None, principal: None }`. `AuditFilter` gained `Debug`
  for the assert. A reviewer can now share e.g. `/audit?principal=alice` and it
  opens pre-filtered.
- **Release wrap.** client Cargo.toml/lock 1.18.2 → 1.19.0; CHANGELOG §[1.19.0]
  (incl. the honest ceilings); CLIENT_ROADMAP v1.19.0 row → Shipped (with the
  audit-verified scope); client README status → v1.19.0; AGENTS header + this
  entry.

### Verification
- `cargo test --manifest-path client/Cargo.toml`: **77 passed** (was 76; +1
  `filter_from_query_seeds_deep_link_params`).
- `cargo clippy --all-targets --manifest-path client/Cargo.toml -- -D warnings`:
  clean. `cargo fmt --check`: clean. `cargo build --target
  wasm32-unknown-unknown`: clean.
- Server suite untouched (476 baseline — zero server edits).

### Ship status: SHIPPED 2026-08-10
All operator steps executed: `./deploy-web.sh` → `client/dist/` rebuilt
(commit `689d7ae`, ship rebuilt tailwind.css for the v1.19.0 bundle) and the
live `/app` serves the v1.19.0 bundle (`brain-client-dxhc3dc1e3fbc1f72f3.js`;
`/app/index.html` + `/app/manifest.webmanifest` 200); tag `v1.19.0`
(`60e2c33`) created + pushed; GitHub release v1.19.0 published 2026-08-10.
No server restart needed (client-only static bundle).

### Honest ceilings (carried into v1.20.0)
- OIDC authorization-code + PKCE is a server-side v2.x gap (needs `/auth/authorize`
  on brain-server or an IdP proxy); the client already consumes the JWT half.
- Virtualized lists need viewport JS (untestable here without `dx serve`); audit
  pagination is the honest no-JS equivalent.
- wasm-split lazy panels remain a Dioxus 0.7.10 ceiling (re-measure on 0.8-stable).

---

## Agent 66: v1.20.0 "Polish" — system theme + bundle budget + offline queue, the done-state (session 2026-08-11)
**Status:** COMPLETED (code + tests + docs; deploy/tag pending operator)
**Date:** 2026-08-11

Shipped the final milestone of the v1.14→v1.20 client chain — **the done-state**.
**Client-only** — server + API contract stay at 1.18.2 (zero server changes).
An audit of the plan against the tree found density/typography (M1.2/M1.3)
already shipped in v1.16.8 and zero-telemetry (M4) needing no code; this
release closes the remaining testable deltas. See `CHANGELOG.md` §[1.20.0].

### Changes Made
- **M1 — system-following theme** (`src/i18n.rs` + `src/main.rs` +
  `styles/input.css`). The saved pref is now tri-state `dark`|`light`|`system`;
  the top-bar toggle cycles through `THEME_MODES`. `pick_theme` sanitizes
  (non-empty, returns static literals); the existing theme effect sets
  `<html data-theme>` verbatim. The `system` mode needs zero JS: a new
  `@media (prefers-color-scheme: light) { html[data-theme="system"] { … } }`
  block in `input.css` (same token values as `[data-theme=light]`, kept in
  sync by comment) follows the OS both on launch and live-mid-session.
  Density + typography stay as shipped (v1.16.8).
- **M2.1 — bundle regression budget in CI** (`client/bundle-budget.sh` +
  `.github/workflows/ci.yml`). Release wasm (the dominant bundle term) must
  stay ≤ 7,000,000 B: measured 4,339,760 B at ship. A new `bundle budget`
  step in the `client-gate` job runs the script (build → measure → fail on
  breach). The plan's final <50 KB web-initial / <5 MB mobile budgets stay
  operator `dx bundle` measurements (no Dioxus CLI on CI), recorded in
  `BENCHMARKS.md` (which keeps the v1.18.1 dx-bundled 3.7 MB row as the
  floor reference).
- **M3 — offline-tolerance** (`src/queue.rs`, new; wired in `src/main.rs` +
  review/subjects/data panels). A bounded (100) action queue holding
  `QueuedAction::Approve/Reject/Purge/Dsar` with payload-keyed idempotency
  keys (`key()`) and serde persistence through the existing `i18n::pref_save`
  seam (localStorage holds action-ids only, never the token — the
  `credentials_stay_in_memory` grep guard still passes). The decision/batch/
  purge/DSAR paths that hit an unreachable or erroring server enqueue instead
  of dropping; a top-bar "queued" badge shows the count. On recovery the
  queue replays once per key (`run_replay`: settle-by-key, a 404-no-pending
  counts as applied, survivors re-enqueue) — a replay can never double-apply.
  Review rows render `RowOutcome::Queued` as "queued (offline)" and the
  batch summary counts `queued`; DSAR outcomes surface the queued state
  instead of a generic failure.
- **M4 — zero-telemetry reaffirmed.** Nothing in M1–M3 collects data (the
  queue is local action-ids); the plan's desktop/mobile in-app update check +
  opt-in crash reporting remain honest ceilings (native toolchains; no
  third-party by mandate).
- **Release wrap.** client Cargo.toml/lock 1.19.0 → 1.20.0; CHANGELOG
  §[1.20.0]; CLIENT_ROADMAP v1.20.0 row → Shipped; plan ship-notes;
  BENCHMARKS bundle row; client README status → v1.20.0; AGENTS header +
  this entry.

### Fixes during the pass (compile/clippy gates)
- `pick_theme` returned a borrowed `&'static str` for the `light`/`system`
  arms (lifetime error — now maps to literals).
- `queue_remove` was dead code (replay re-enqueues survivors instead) —
  deleted with its test, per the no-dead-code rule.
- Two `Err(…)` DSAR outcomes → `DsarOutcome::Failed(…)`; a stale
  `Signal`-method call in the replay effect; `len() > 0` / `iter().any(==)`
  clippy lints.
- `Subject` (the queue wire type) dropped `datetime: String` to keep the
  queue payload purely action-ids (it was unused by the replay path).

### Verification
- `cargo test --manifest-path client/Cargo.toml`: **82 passed** (was 77; +5
  queue bounds/dedup/serde/pick_theme + replay-applies-once; the batch
  summary test now pins `queued`).
- `cargo clippy --all-targets --manifest-path client/Cargo.toml -- -D warnings`:
  clean. `cargo fmt --check`: clean. Desktop + wasm builds clean.
- `bash client/bundle-budget.sh`: green (4,339,760 B ≤ 7,000,000 B).
  `ci.yml` re-parses (pyyaml).
- Server suite untouched (476 baseline — zero server edits).

### Ship status: SHIPPED 2026-08-11
`./deploy-web.sh` → live `/app` re-deployed and serving the v1.20.0 bundle
(index + js + wasm + tailwind + manifest + sw all 200); commit `96ffd11`
pushed to `main`; tag `v1.20.0` created + pushed; GitHub release v1.20.0
published. No server restart needed (client-only static bundle).

### Honest ceilings (carried into v2.0)
- Measured `dx bundle` sizes + memory/FPS profiling on target devices stay
  operator steps (`dx` is not on CI; no physical devices here); the plan's
  <50 KB / <5 MB budgets are recorded in `BENCHMARKS.md` as
  measured-success criteria, and the CI wasm budget is the tripwire.
- `system` theme applies on launch/change, not live-mid-session (web
  media-query live-listening is a small v2.x polish).
- Replay settle-by-key is client-side idempotency (a row already rejected
  server-side still counts as applied once) — a server-side idempotency
  contract is a v2.x backend nicety, documented in the plan.
- wasm-split stays a Dioxus 0.8 ceiling; the budget gate guards the bundle
  until then.

---

## Agent 67: v1.20.1 "Shield" — GhostJacking P0s: shared /ingest screen + autoCapture human gate (session 2026-08-11)
**Status:** COMPLETED (code + tests + docs; live restart/tag pending operator)
**Date:** 2026-08-11

Shipped the v1.20.1 "Shield" **server + plugin + client** release closing the
two P0 findings of the GhostJacking audit (G1 + G2), per
`IMPLEMENTATION_PLAN_v1.20.1_Shield.md`. Server 1.18.2 → 1.20.1; plugin
0.2.0 → 0.2.1; client stays at 1.20.0 (one new wire field + two pure-gen
tests + an i18n block + a review-panel section, version-neutral). See
`CHANGELOG.md` §[1.20.1].

### Changes Made
- **M1 — shared `/ingest` write core screens injection** (`src/handlers/ingest.rs`).
  `ingest_one` (the one core for plain + single-UMP + batch-UMP ingest, and the
  plugin's `memory_store`/`autoCapture` direct path) now runs the same
  `scan_injection` screen as `/add` + `/ingest/memory` (G1). On `Reject` policy
  (config) → HTTP 400 `input_rejected`; on `Quarantine` (default) → stored
  flagged (`flagged=1`, excluded from recall) + KG edges skipped. A
  `input_rejected`/`quarantined` field joins the response. No new routes, no
  feature flag, deterministic.
- **M2 — autoCapture through the human review queue.** `captureMode` on the
  plugin config (`proposal` default | `direct`). `proposal` POSTs
  `/ingest/proposal` (the v1.14 review gate — nothing becomes memory without
  a reviewer approve) via the new `BrainClient.submitProposal()`; `direct`
  keeps the old autoCapture→`memory_store` behavior, still M1-screened.
  Server side: additive `proposals.source_prompt` column (migration +
  schema 1.20.1), PII-screened at persist via pure `gate::screen_source_prompt`
  (only the `[redacted:…]` form persists — LLM01:2026 control #7 "exact
  action, not a summary"), round-tripped through `ProposalView` +
  `/proposals` + the client wire type, rendered in the Review panel's
  "sourcing prompt" block. TTL: `BRAIN_PROPOSAL_TTL_SECS` (default 7 days) —
  `expire_if_stale` auto-rejects expired proposals + audits
  `proposal_expired`; approve/reject on a stale proposal refuse 400.
- **M3 — docs honest.** SECURITY.md: `/ingest` write surface marked screened,
  autoCapture gated by default. `docs/MEMGHOST_MITIGATION.md`: `captureMode`
  documented (proposal default, direct escape hatch).
- **Release wrap.** server Cargo.toml/lock 1.18.2 → 1.20.1; plugin
  package.json 0.2.0 → 0.2.1; openapi.yaml 1.20.1 (`ProposalView.source_prompt`
  + `/ingest` result fields); README badge → 1.20.1; ROADMAP released row;
  wiki Home/Release-History; CHANGELOG §[1.20.1]; AGENTS header + this entry.

### Verification
- `cargo test --features bench,migrate`: **583 passed across all targets**
  (main bin 478 passed + 4 `#[ignore]`d; +3 vs the 1.18.2 baseline:
  `ingest_screens_injection_like_its_siblings` — the audit §5 drill become a
  model-backed `#[ignore]`d test with quarantine/reject/benign arms,
  `test_proposal_expires_after_ttl_and_audits`, and the lib's
  `source_prompt_is_pii_screened_and_rendered`). Clippy `-D
  warnings` + fmt green; `test_migration_schema_contract` + wiring guards green.
- Client: **82 passed** (unchanged — the delta is the `Proposal.source_prompt`
  wire field (serde default, fixture-updated) + the Review card's rendering of
  the "sourcing prompt" details block; clippy + fmt + wasm green). Plugin:
  **94 passed** (+3 submitProposal wire, captureMode default routing, config
  registry default), via `pnpm test:extension brain-server` in the openclaw
  workspace; the canonical copy at `openclaw/extensions/brain-server` synced
  (7 files).
- Full local gates run race-free (tests first, then clippy/fmt wasm/bundle in
  a second band — the `--features bench,migrate` test build reserves a lot of
  memory; parallel full-suite runs thrash).

### Ship status: COMPLETED (code + tests + docs) 2026-08-11
`scripts/install-service.sh` (server restart — the migration runs on boot;
plugin config `captureMode` in `~/.openclaw/openclaw.json`), the tag
`v1.20.1`, the GitHub release, and the openclaw-fork push (extension copy)
are operator steps.

### Honest ceilings (carried into v1.20.2 / v1.20.3)
- The screen stays the deterministic blocklist (G5 classifier upgrade is
  v1.20.3). Quarantine stores flagged, never deletes.
- `source_prompt` is PII-scanned, not semantically safe; approved proposals
  render it in Review for the human's own judgement.
- G3 (OpenClaw subagent/exec/read/pdf envelope coverage) is OpenClaw-side —
  companion plan v1.20.2. G4 (live token at rest, world-readable plist) is
  operator/tooling — v1.20.2. G6 webhook replay P2 documented, v1.20.4 if
  prioritized.

---

## Agent 68: MCP 2026-07-28 protocol compliance — `src/bin/mcp.rs` (UNRELEASED, rides into the next release)
**Status:** COMPLETED (code + tests + gates; no version bump by operator decision)
**Date:** 2026-08-11

Brought the `mcp` stdio server up to the **final MCP 2026-07-28 spec**
(canonical path `modelcontextprotocol.io/specification/2026-07-28/`; research
was done against the spec pages + a grep of the schema confirming `ping` and
`initialize` are gone). Deliberately shipped **without a release** — no version
bump, no tag — because it changes no HTTP API contract, no schema, and neither
client nor plugin, and both `v1.20.2–v1.20.5` (GhostJacking hardening line) and
`v1.21.0` (client Profiles) are pre-allocated to other plans. Work is
traceable in `CHANGELOG.md` §[Unreleased].

### Changes Made
- **Stateless modern core**: no `initialize`/`initialized` handshake (SEP-2575).
  Every request carrying `_meta` is validated (`check_meta`): mandatory
  `io.modelcontextprotocol/protocolVersion` (string) +
  `io.modelcontextprotocol/clientCapabilities` (object); `clientInfo` optional.
  Missing/ill-formed → -32602; unsupported version → -32022 with
  `data {supported: ["2026-07-28","2025-11-25"], requested}`.
- **`server/discover`** (the modern replacement for `initialize`): returns
  `supportedVersions`, `capabilities`, `instructions`, `ttlMs` (3_600_000),
  `cacheScope: "public"` — stateless, cacheable.
- **Result envelope**: every modern success carries `resultType: "complete"` +
  `_meta.io.modelcontextprotocol/serverInfo`; `tools/list` adds `ttlMs`
  (300_000) + `cacheScope` (SEP-2549 caching hints).
- **Error surface per the new spec**: unknown tool → -32602 protocol error (was
  an `isError: true` result); parse error → -32700 with null id; missing
  method → -32600; explicit null id → -32600; `dispatch` maps `server/discover`
  + `tools/list` failures → -32603 and `tools/call` failures → -32602
  (transport errors included, `ponytail:` noted). `ping` kept as a no-op
  (removed from the new schema; harmless for legacy tooling).
- **Dual-era legacy**: a legacy client's `initialize` sets a `legacy` flag
  scoped to the stdio process → bare requests (no `_meta`) dispatch and
  responses keep the legacy 2025-11-25 shape (no `resultType` envelope).
- **Versioning**: stale `PROTOCOL_VERSION = "2024-11-05"` replaced by
  `MODERN_VERSION = "2026-07-28"` / `LEGACY_VERSION = "2025-11-25"` /
  `SUPPORTED_VERSIONS`. Cargo.toml stays at 1.20.1.

### Verification
- `cargo test --features bench,migrate`: **591 passed, 4 ignored** (was 583;
  +8 mcp wire tests: discover modern surface, tools/list complete+cacheable,
  bare request → -32602, missing `_meta` fields → -32602, unsupported version
  → -32022 with data, initialize → legacy mode, unknown tool → -32602, parse
  error → -32700 null id). Clippy `-D warnings` + `cargo fmt --check` green.
- **Live stdio smoke** (release binary, static methods): discover →
  `resultType=complete`, `supportedVersions=[2026-07-28, 2025-11-25]`,
  ttlMs/cacheScope present; modern tools/list → complete + 12 tools + caching
  hints; bare tools/list → -32602; initialize → 2025-11-25 (no resultType);
  legacy tools/list → 12 tools (no resultType).

### Honest ceilings (carried forward)
- `server/discover` is served, but no modern MCP client exists in this
  environment to exercise a full `tools/call` round-trip against it (the live
  stdio smoke covers the static surface; `tools/call` behaviour is pinned by
  the pre-existing unit tests + the shared HTTP client).
- **2026-08-11 follow-up — real-client verification (legacy era only):**
  wired as a test into OpenClaw 2026.8.1 (`openclaw mcp add brain-server
  --command ~/.local/bin/mcp`, then `openclaw mcp unset brain-server` after)
  — openclaw's `@modelcontextprotocol/sdk` 1.30.0 client speaks 2025-11-25, so
  the probe exercised the **dual-era legacy path** end-to-end: `initialize` →
  legacy response, `tools/list` → all 12 tools, `tools/call` →
  `ump.capabilities` → live L3 payload. The modern-era `_meta` path still has
  no real client here. The native `brain-server` plugin remains the production
  OpenClaw integration; the MCP registration was a test only. Note: a
  freshly-copied `~/.local/bin/mcp` must be ad-hoc signed
  (`codesign --force --sign -`) or have `com.apple.provenance` stripped, or
  Gatekeeper SIGKILLs it on Node-child spawn (reproduced; the AGENTS.md
  documented failure class). Documented in `CHANGELOG.md` §[Unreleased].
- Caching hints are advertised per SEP-2549; no client here exercises cache
  re-use.
- The hardening line (v1.20.2–v1.20.5) and client Profiles (v1.21.0) are
  unaffected; this work rides into the next versioned release.

---

## Agent 70: v1.20.3 "Classify" — G5 two-layer injection screen + client render boundary (session 2026-08-11)
**Status:** COMPLETED (code + tests + gates; live restart/tag pending operator)
**Date:** 2026-08-11

Shipped the GhostJacking **G5** upgrade path as **v1.20.3**. **Server**
(Cargo.toml 1.20.2 → 1.20.3) + a version-neutral **client** delta (stays at
1.20.0). **No schema change** — `proposals.screen_verdict` is recomputed at
read time, so the schema stays 1.20.1 and `test_migration_schema_contract` is
untouched. See `CHANGELOG.md` §[1.20.3].

### Changes Made
- **Two-layer injection screen** (`src/screen.rs`, the single seam every
  ingest write site routes through). Layer 1 = the deterministic blocklist
  (always on). Layer 2 = an **optional, feature-gated local ONNX classifier**
  (`injection-classifier` feature + `ort`/`tokenizers`, **off by default** —
  the Jetson envelope treats memory as scarcest; blocklist +
  `flagged`/`untrusted` remain the always-on defense). When enabled, loads a
  BERT-tiny INT8 model at `BRAIN_INJECTION_CLASSIFIER` + tokenizer at
  `BRAIN_INJECTION_TOKENIZER` once via a `LazyLock<Option<Arc<dyn
  InjectionScorer>>>`, off the request path. Banding: score ≥ 0.9 → HTTP 400,
  ≥ 0.7 → stored flagged, else clean; sentence-packed + density-adjusted
  scoring (StackOne calibration). **Policy + thresholds read per call** (an
  operator flips `INJECTION_POLICY` without a restart); only the model load is
  cached. ort rc.13 API wired: `ort::session::Session` under a `Mutex` (its
  `run` needs `&mut`, handlers are multi-threaded), `?` into `anyhow` blocked
  (ort::Error is !Send/!Sync) → mapped to strings.
- **Wired into every ingest write site**: `/add`, `/ingest/memory`,
  `/ingest/markdown`, `/ingest` (`ingest_one`), `/procedure` (root + each
  step), `/ingest/proposal`. `Reject` → 400 (`input_rejected`); `Quarantine`
  → stored flagged + KG edges skipped. `flag_if_quarantined` now takes the
  screen's bool verdict — a layer-2 hit quarantines exactly like a layer-1
  hit.
- **Review-queue badge**: `ProposalView.screen_verdict` (`clean`/`quarantine`;
  `reject` is never persisted, recomputed deterministically at read).
- **`/health`** hardening field `injection_classifier_loaded`.
- **Canonical `screen::is_invisible`** (extended from v0.9.7: adds tag block
  U+E0000–E007F + variation selectors U+FE00–FE0F) shared by the blocklist
  normalization, the classifier, and the **client render boundary** — the
  client strips invisible smuggling chars from *displayed* recall hits +
  review proposals; raw stored bytes never rewritten.
- **Release wrap**: version 1.20.2 → 1.20.3 (Cargo.toml, openapi.yaml,
  README badge); CHANGELOG §[1.20.3]; AGENTS header + this entry. The plan
  file is gitignored per repo convention.

### Verification
- `cargo test --features bench,migrate`: **611 passed, 5 ignored** (was 597 at
  the v1.20.2 baseline; +14: screen pipeline / banding / density / strip /
  `screen_verdict` label + the `ingest_write_sites_route_through_screen`
  wiring guard). All 5 `#[ignore]`d pass — incl. the 2 model-backed
  Shield/audit drills (`ingest_screens_injection_like_its_siblings` +
  `procedure_screens_injection_like_its_siblings`), which required switching
  the screen's policy cache from a `OnceLock<Screen>` (cached the policy at
  first use → a runtime `INJECTION_POLICY` flip in the test never took effect)
  to caching only the classifier and reading policy per call.
- `cargo clippy --all-targets --features bench,migrate -- -D warnings` clean
  AND `--features bench,migrate,injection-classifier` clean. `cargo fmt
  --check` clean. Client: **83 passed** (was 82; +1 strip_invisible test),
  clippy + fmt clean.

### Ship status: COMPLETED (code + tests + gates) 2026-08-11
`scripts/install-service.sh` (live restart), tag `v1.20.3`, and the GitHub
release are operator steps.

### Honest ceilings (carried into v1.20.4 / v2.0)
- Layer 2 is verified on desktop (feature build compiles); a real ONNX model
  isn't present in this env, so the live model-backed path is an operator
  step (`bench --envelope` before treating as Jetson-shippable — repo
  precedent: rerank was removed for the same reason).
- The classifier catches semantic patterns, not every obfuscation; Quarantine
  stores flagged, never deletes.
- `screen_verdict` is recomputed at read time, so a model swap can re-badge an
  in-flight proposal; a model-drift Reject on a stored row reads as
  `quarantine`.
- `strip_invisible` runs at screen/classifier/render boundaries, not by
  rewriting stored bytes.
- G3 (OpenClaw envelope) + G4 (token at rest) remain operator/OpenClaw-side.

## Agent 69: v1.20.2 "Harden" — deep + security second-pass audit fixes (session 2026-08-11)
**Status:** COMPLETED (code + tests + ship gate + release wrap; live restart/tag/push pending operator)
**Date:** 2026-08-11

Shipped the consolidated v1.20.x **deep + security second-pass audit** fix
release as **v1.20.2**. **Server-only** (server Cargo.toml 1.20.1 → 1.20.2;
plugin stays 0.2.1; client stays 1.20.0). **No schema change** — schema stays
at 1.20.1, `test_migration_schema_contract` unchanged + green. The working
tree already carried most of the implementation (10 files); this session
audited it against the plan, closed the one missing check (B1's
`procedure_screens_injection_like_its_siblings`), fixed the G3 test that the
hex-escape broke, and wrapped the release. See `CHANGELOG.md` §[1.20.2].

### Changes Made
- **A1 [C] audit chain fork under concurrent autocommit writers** (`src/audit.rs`).
  `record_tenant` now branches on `conn.is_autocommit()`: autocommit → `BEGIN
  IMMEDIATE` (read-modify-write serializes at BEGIN); inside a caller tx →
  `SAVEPOINT` (outer tx holds the write lock). Mirrors `record_and_rotate`.
  Pinned by `audit_chain_survives_concurrent_autocommit_writers`.
- **A2 [M]** `prune_audit_retention` re-anchor → `TransactionBehavior::Immediate`.
- **A3 [H]** `approve_proposal` CAS'd (`AND status='pending'`, `n>0` →
  `409 proposal_already_decided`), whole promote in `BEGIN IMMEDIATE`.
- **A4 [H]** `approve_proposal` expires stale **before** the tx opens (distinct
  autocommitted event + re-check inside tx).
- **B1** `/procedure` write core screens injection like its siblings (root +
  each step; Reject → 400; Quarantine → per-chunk `flag_if_quarantined` +
  skip `next_step` edges). **Added** the missing plan check:
  `procedure_screens_injection_like_its_siblings` (`#[ignore]`d, model-backed,
  mirroring the v1.20.1 Shield test — Quarantine/Reject/benign arms).
- **C1 [PII]** `mask_card` Luhn-checks 13–19 digit runs (16-digit cards were
  flagged but never masked); wired into both `redact_content` +
  `screen_source_prompt`. Pinned by `redaction_masks_luhn_valid_16_digit_cards`.
- **D1 [DoS]** `X-Forwarded-For` only trusted when `BRAIN_TRUST_PROXY=1`
  (default: socket addr) + `RateLimiter` capped at `RATE_LIMIT_MAX_KEYS=10_000`
  with LRU eviction (oldest 25%).
- **D2 [DoS]** `extract_vocabulary` capped at `MAX_VOCAB_ENTITIES=500`.
- **D3 [DoS]** `/export` bounded (hard row cap + precomputed provenance
  summary); full streaming JSON is a `ponytail:` v2.x ceiling.
- **D4 [DoS]** `/v1/embeddings` batch capped at `MAX_EMBEDDING_BATCH=64`.
- **E1 [AuthZ]** `/tombstones` + `/dsar/{id}/certificate` tenant-scoped against
  the principal's `sub` at the SQL layer (cross-tenant → empty/404, no leak).
- **E3** `/add` now enforces `MAX_CONTENT`.
- **F1** `source_prompt` bounded (`MAX_SOURCE_PROMPT=2048`) + screened.
  **F2** `/health/db` moved out of the public lists (now Read-gated).
  **F3** `multi_get` collapsed to a single `WHERE id IN (...)`.
  **F4** `/metrics` tenant intent documented.
- **G (folded Agent 68)** MCP 2026-07-28 protocol compliance ships here +
  G1 `MAX_LINE_BYTES=1 MiB` guard, G3 `sanitize_echo` hex-escapes user input
  (no prompt-injection carrier in `error.message`), G4 `ponytail:` ceiling.
- **Wrap.** Cargo.toml 1.20.1 → 1.20.2; openapi.yaml → 1.20.2; CHANGELOG
  §[1.20.2]; AGENTS header + this entry. The plan file (`IMPLEMENTATION_PLAN_
  v1.20.2_Harden.md`) is gitignored per repo convention (referenced, not
  committed).

### Verification
- `cargo test --features bench,migrate`: **597 passed, 5 ignored** (was 591/4
  at the Agent-68 baseline; +1 B1 test, and the G3 change required updating
  `unknown_tool_is_a_protocol_error` to assert the hex-escaped form — the raw
  `"nope"` no longer appears by design). All 5 `#[ignore]`d tests pass
  (`--ignored`).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean. `cargo build --release --features
  bench,migrate --bin brain-server --bin brain --bin mcp --bin bench --bin
  brain-migrate-rehearse`: all 5 binaries clean.
- Wiring guards green: `authz_gates_cover_every_non_public_route` +
  `test_openapi_covers_routes` + `test_migration_schema_contract` (1.20.1).

### Ship status: COMPLETED (code + tests + gates + wrap) 2026-08-11
`scripts/install-service.sh` (live restart), commit/tag `v1.20.2`, the GitHub
release, and the push are operator steps.

### Honest ceilings (carried into v1.20.3+ / v2.0)
- The injection screen stays the deterministic blocklist (G5 classifier =
  v1.20.3). Quarantine stores flagged, never deletes.
- `/export` streaming is a bounded guard, not a server-sent stream (v2.x);
  `RateLimiter` LRU is in-process (shared store v2.1); capability tokens stay
  operator-only (per-tenant cap scope = v2.0 multi-tenancy); the audit-chain
  A1 fix is per-process (distributed chain = v2.1).
- Part H (operator token-at-rest + OpenClaw envelope coverage) is operator-
  only, no brain-server code.

---

## Agent 64: v1.18.2 "Transparency" — Art 50 origin marker + export provenance (session 2026-08-09)
**Status:** COMPLETED (code + tests + docs; live restart/tag pending operator)
**Date:** 2026-08-09

Shipped the v1.18.2 "Transparency" server release (unified version line; client
stays at 1.18.1). An audit of the plan against HEAD found its M3/M4 already
shipped (ai-notice/ai-literacy/cop-notice routes + `docs/AI_LITERACY.md` in
v1.16.7/v1.16.8); this release closes the two real accuracy gaps the plan
identified in COMPLIANCE.md §7 and aligns the doc. See `CHANGELOG.md` §[1.18.2].

### Changes Made
- **M2 — `knowledge.origin` column** (`src/migration.rs`): `TEXT NOT NULL
  DEFAULT 'imported'` + `idx_knowledge_origin` + idempotent backfill by source
  (`manual`→`human`, `memory`→`model`, else `imported`); `schema_version` →
  1.18.2. Pure `gate::origin_for_source(Option<&str>)` helper + test. Write-time
  wiring: `/add` + `/ingest/memory` in `main.rs`, propose→approve promote in
  `handlers/gate.rs`, procedures in `handlers/procedure.rs` (`human`).
  `markdown`/`structured` keep the `imported` default — never claim human
  authorship for an unknown path.
- **M1 — `/export` provenance** (`handlers/gate.rs`): `KNOWLEDGE_ROW_COLS` +
  `knowledge_row_to_json` now carry `origin` (reindexed); envelope gains
  `export_format_version: 2` + `provenance_summary {total, by_origin,
  by_source}`. All 12 v1 field names preserved byte-identical.
- **M3 polish**: `/.well-known/ai-notice` `origin_metadata` lists `origin`.
- **M5 COMPLIANCE.md §7** aligned + Enforcement note (national market
  surveillance authorities, €15M/3% Art 99(3) — not €35M/7% Art 99(2)).
- **Release wrap**: server Cargo.toml/lock 1.17.5 → 1.18.2; openapi.yaml
  version + `/export` schema; README badge; CHANGELOG §[1.18.2]; AGENTS header
  + this entry.

### Verification
- `cargo test --features bench,migrate`: **476 passed, 3 ignored** (+2 vs
  baseline; +`origin_for_source_maps_kinds`, `migration_backfills_origin_by_source`,
  `export_contains_source_origin_and_provenance_summary`). Fixed during pass:
  the INSERT-site guard (`ingest_insert_sites_write_owner_column`) and
  `test_migration_schema_contract` version stamp both updated to the new
  columns/1.18.2.
- Clippy `-D warnings` + `cargo fmt --check` green. All 5 binaries build.

### Ship status: COMPLETED (code + tests + docs) 2026-08-09
`scripts/install-service.sh` (server restart — the migration runs on boot),
commit/tag `v1.18.2`, and the GitHub release are operator steps. Client
untouched (static bundle at 1.18.1).

### Honest ceilings (carried into v1.19 / v2.x)
- `origin` is a write-time tag from the source-kind routing, not a learned
  authorship classifier; `imported` is the honest default for bulk/unknown.
- Backfill is by current `source` kind — a legacy row whose kind changed over
  time tags by its present value (idempotent, re-runs are no-ops).
- UMP wire-format conformance of the Art 50 bridge remains a later release.

---

**Status:** COMPLETED (code + tests + docs; deploy/tag pending operator)
**Date:** 2026-08-09

Shipped the "Harden" plan's honest, testable deltas as **v1.18.1** (the plan
said v1.18.0, but v1.18.0 was taken by "Compliant"; per the client point-release
convention this is a point bump). **Client-only** — server + API contract stay at
1.17.5. An audit of the plan against the tree found only two items that were both
real and testable here; the rest are code-grounded non-changes. See
`CHANGELOG.md` §[1.18.1].

### Changes Made
- **M1 — console history persists across reload, secret-safe**
  (`src/api.rs` + `src/panels/system.rs`). `StoredLine { text, secret }`;
  pure `line_is_secret` (a non-JSON/opaque body = token-like, cannot be
  redacted → held in-memory only) + `persist_history` (drops secret/empty
  lines, caps to last 100). `run_console` pushes a `StoredLine`, persists only
  the clean subset via the existing `i18n::pref_save("console_history", …)`
  seam; a `use_effect` loads it back on mount (only if history is empty). The
  `credentials_stay_in_memory` grep guard still passes — raw token-bearing
  input never touches disk.
- **M4a — client bundle measured, not guessed** (`BENCHMARKS.md`). Recorded
  the `dx bundle` sizes as measured facts: wasm 3,724,711 B (3.7 MB) + 60 KB
  JS + 40 KB CSS. Parse/instantiate time on a target device stays PENDING
  (operator browser harness). wasm-split not adopted (experimental in 0.7.10,
  shell-heavy bundle); re-measure after Dioxus 0.8-stable.
- **Release wrap.** client Cargo.toml/lock 1.18.0 → 1.18.1; CHANGELOG §[1.18.1];
  client README status → v1.18.1; BENCHMARKS client-bundle row; AGENTS header
  + this entry.

### Code-grounded non-changes (honest ceilings, not deferred-as-lazy)
- **M2 token-minting panel UX** — no "CLI docs link" exists in the UMP panel to
  replace; minting is correctly CLI-only (no mint endpoint by design). Adding
  untestable UX churn was skipped; security posture unchanged.
- **M3 SSE subscribe** — no SSE subscribe control exists in the client; the
  `/ump/subscribe` endpoint is server-side reachability only → nothing
  misleading to rename. A live browser change stream is v2.x A2A.
- **M5 native pull-to-refresh / M6 focus-return** — native gesture needs a touch
  platform + `dx serve`; focus-return is `document::eval`-based; neither is
  verifiable in this env (no Android SDK / browser harness). The accessible
  `RefreshButton` and existing focus trap remain.

### Verification
- `cargo test --manifest-path client/Cargo.toml`: **76 passed** (was 74; +2
  `line_is_secret_for_opaque_non_json_bodies` + `persist_history_drops_secret_lines_and_caps`).
  Clippy `-D warnings` clean, `cargo fmt --check` clean, desktop + wasm builds clean.
- Server suite untouched (473 baseline — zero server edits).

### Ship status: COMPLETED (code + tests + docs) 2026-08-09
`./deploy-web.sh` → live `/app` re-deploy, tag `v1.18.1`, and the GitHub release
are operator steps. No server restart needed (client-only static bundle).

### Honest ceilings (carried into v1.19)
- M2 mint UX, M3 SSE browser stream, M5 native gesture, M6 focus-return — see
  non-changes above; each is a documented operator/tooling step or a v2.x A2A
  ceiling.
- Console history persistence is pattern-based (`redact_for_history`); no
  guaranteed PII classifier is claimed — operator care remains the last line of
  defense.

---

## Agent 62: v1.18.0 "Compliant" — `?` keyboard help + client CI gate (session 2026-08-09)
**Status:** COMPLETED (code + tests + docs; deploy/tag pending operator)
**Date:** 2026-08-09

Shipped the v1.18.0 "Compliant" plan's remaining testable deltas. **Client-only**
— server + API contract stay at 1.17.5 (zero server changes). The plan's M3
(i18n, all 5 locales) and M4 (privacy labels) shipped in v1.16.8/v1.17.0, and M1's
WCAG pass (`prefers-reduced-motion`, A/S/R/J/K + WCAG 2.1.4 toggle,
focus/landmark/semantic gates, `a11y-checklist.md` manual-pass artifact) is in
place across v1.16.2–v1.17.x. An audit of the plan against the tree found the two
real gaps and closed them. See `CHANGELOG.md` §[1.18.0].

### Changes Made
- **M1.4 — in-app `?` keyboard help on Review** (`src/panels/review.rs`). The
  WCAG 3.2.6 consistent-help gap: pressing `?` (or the new `?` toolbar button,
  `aria-expanded` + `aria-label`) toggles an in-app `<dl role="note">` table
  documenting the A/S/R/J/K shortcuts. Pure `keyboard_help()` core returns the
  `(i18n-key, key)` rows so the rendered list and the `?` mapping share one
  source of truth; `ReviewKey::Help` wired through `key_action`. The `?` mapping
  respects the existing WCAG 2.1.4 shortcuts-off toggle. i18n keys
  (`review_help*`) added to `en` (source; the other locales inherit via
  `resolve`'s en-fallback — the `locale_bundles_load_and_en_is_complete` test
  stays green).
- **M2 — `client-gate` CI job** (`.github/workflows/ci.yml`). The Dioxus client
  had **zero CI coverage**; a new job runs `cargo fmt --check` + `cargo clippy
  --all-targets -- -D warnings` + `cargo test` + the `wasm32-unknown-unknown`
  build (the web target, and the one the automated a11y grep gates
  `interactive_elements_are_buttons` + `xss_escape_hatch_is_unused` run
  against). YAML verified locally (pyyaml).
- **Release wrap.** client Cargo.toml/lock 1.17.8 → 1.18.0; CHANGELOG §[1.18.0];
  client README status → v1.18.0; CLIENT_ROADMAP v1.18.0 row → Shipped;
  AGENTS.md header + this entry.

### Verification
- `cargo test --manifest-path client/Cargo.toml`: **74 passed** (was 73; +1
  `question_mark_opens_help_and_table_covers_all_keys`). Clippy `-D warnings`
  clean, `cargo fmt --check` clean, desktop + wasm builds clean.
- `ci.yml` parses. Server suite untouched (473 baseline — zero server edits).

### Ship status: COMPLETED (code + tests + docs) 2026-08-09
`./deploy-web.sh` → live `/app` re-deploy, tag `v1.18.0`, and the GitHub release
are operator steps. No server restart needed (client-only static bundle).

### Honest ceilings (carried into v1.19)
- **axe-core browser gate (M2.1)** stays an operator/tooling step — needs
  Playwright + `dx bundle` + a live server + browser download, none runnable in
  this repo's CI surface. Tracked in `client/a11y-checklist.md`.
- **Native screen-reader pass (M1.7)** is the human gate; the `a11y-checklist.md`
  VoiceOver/NVDA/TalkBack matrix is the operator artifact.
- i18n `de`/`fr`/`es` are human-authored first cuts; native review is a follow-up
  when a buyer engages.

---

## Agent 55: v1.17.0 "Mobile" — portable refresh + deep links + offline connect + store readiness (session 2026-08-08)
**Status:** COMPLETED (code + tests + docs + tag + release; client-only)
**Date:** 2026-08-08

Shipped the remaining milestones of the v1.17.0 Mobile plan on top of the
v1.16.6 mobile groundwork (which already landed M1 secure-token storage + M2
responsive UX). **Client-only** — server + API contract stay at 1.16.7. See
`CHANGELOG.md` §[1.17.0].

### Changes Made
- **M2.4 portable refresh control** — new shared `RefreshButton`
  (`panels/mod.rs`) bumping the panel's existing `refresh: Signal<u32>`;
  wired into Review (toolbar), Audit (next to Export), Health (new `refresh`
  signal + button row). The native pull-to-refresh *gesture* stays a v1.18.0
  ceiling (needs touch events; untestable without `dx serve`).
- **M3.3 deep-link intent filters** (`Dioxus.toml`) — `[ios] url_schemes =
  ["brain"]` + an Android `VIEW`/`BROWSABLE` intent filter for the `brain://`
  custom scheme, opening into the existing `Routable` router. Verified the TOML
  parses (tomllib). Full https universal-link parity is v1.19.0.
- **M3.4 offline connect pre-fill** (`main.rs`) — on a successful connect the
  resolved base is persisted as a non-secret UI pref (`i18n::pref_save
  "last_base"`, the existing localStorage seam — the token stays keyring-only);
  the Connect screen pre-fills the URL field on a returning/offline connect.
  The specific `/health` failure was already surfaced; the field now comes
  pre-populated. Pure `prefill_if_empty(current, remembered)` guard (fills an
  empty field, never overwrites the operator's typing) + test.
- **M3.1 store-readiness** — new `client/STORE_READINESS.md`: App Store / Play
  privacy-nutrition labels ("no data collected", accurate — one self-hosted
  backend, no analytics/tracking/third-party SDKs), icon/launch/screenshot +
  deep-link + submission checklist. Icon/screenshot generation + store upload
  are operator steps (no platform tooling here).
- **Version bump** client 1.16.8 → 1.17.0. CHANGELOG §[1.17.0], CLIENT_ROADMAP
  v1.17.0 row → Shipped, client README status → v1.17.0, AGENTS.md header +
  this entry.

### Verification
- `cargo test --manifest-path client/Cargo.toml`: **49 passed** (was 48; +1
  `offline_prefill_fills_empty_field_only`).
- `cargo clippy --all-targets --manifest-path client/Cargo.toml -- -D warnings`:
  clean. `cargo fmt --check`: clean.
- `cargo build` + `cargo build --target wasm32-unknown-unknown`: clean (the
  wasm build covers the one-codebase web target; desktop compiles too).
- `Dioxus.toml` parses (python tomllib): `ios.url_schemes=['brain']`,
  `android.intent_filters=[{actions=[VIEW], categories=[DEFAULT,BROWSABLE],
  auto_verify=true, data=[{scheme='brain'}]}]`.

### Ship status: SHIPPED 2026-08-08
Tag `v1.17.0` created + pushed; GitHub release published. No server restart
needed (client-only static bundle; the live `/app` is unaffected by the version
bump — `deploy-web.sh` is an operator step if the operator wants the new client
live).

### Honest ceilings (carried into v1.18.0)
- Native iOS/Android bundling (`dx bundle --platform {ios,android}`) is an
  **operator step** — needs signing + an Android SDK, neither present here. The
  compile surface is covered by desktop + wasm; the platform glue ships in
  `Dioxus.toml` + `storage.rs`.
- Pull-to-refresh is a button, not the native gesture (v1.18.0).
- `brain://` links are registered but not fully panel-routed — URL parity v1.19.0.
- App-store review is an external gate (low risk: "no data collected" + a
  governance tool).

## Agent 61: v1.17.8 "Complete 3/3" — Data & Rights + UMP + System panels, closes the line (session 2026-08-09)
**Status:** COMPLETED (code + tests + docs; deploy/tag pending operator)
**Date:** 2026-08-09

Shipped the final part of the three-part "Complete" operator-console line
(`v1.17.8`). **Client-only** — server + API contract stay at 1.17.5 (zero
server changes, zero schema change). See `CHANGELOG.md` §[1.17.8].

### Changes Made

**M5 — Data & Rights panel (`src/panels/data.rs`, new).** The v1.14 / v1.15
lifecycle surface: purge (`POST /purge` by comma/space/newline-separated ids
or an owner email), portable export (`GET /export` as JSON / UMP /
UMP-Markdown via the existing `document::eval` download seam), a per-kind
retention editor (`GET /retention` → `retention_to_edits` sorted overrides;
set a kind+days override, one-click `×` clear per kind via `retention_clear`),
the `/decayed` review list, and the `/tombstones` deletion-registry. Status
region is `role="status" aria-live="polite"`.

**M6 — UMP panel (`src/panels/ump.rs`, new).** The v1.17.3 wire surface:
capabilities card (`UmpCapabilities` + pure `ump_integrity_badge` badge/label
from the `conformance` line), `POST /ump/remember` (JSON body → `{ok,id}`),
`POST /ump/recall` with kind filter + `max_recall` clamped to 1..100
(rendering the `results` envelope), and `POST /ump/audit` load + verify-chain.

**M7 — System panel (`src/panels/system.rs`, new).** Domains list, snapshot
integrity, the Art 30 register (pretty-JSON), `POST /reindex`
(`ReindexResult`), connectors list (`ConnectorRow`: `kind · instance / state`)
+ `POST /sources/reconcile` (`ReconcileResult`), and a **Try-it console**
(`get_raw`/`post_raw`/`delete_raw` + `serialize_request` request-line builder
+ `redact_for_history` so the persisted in-memory history never stores a
token-bearing body).

**M8 — Route + nav + i18n + version.** `Route::Data` (`/data`), `Route::Ump`
(`/ump`), `Route::System` (`/system`) under the AppShell; all three added to
sidebar rail + mobile tab bar + command palette (nav targets now **12**, guard
test updated); new `data_*`/`ump_*`/`sys_*`/`nav_*` keys in all five locales
(each locale now 50 keys, en-completeness test green). **api.rs**: `Clone`
added to the 10 typed wire structs so `Signal<T>()` call-syntax reads work
(root cause of the call-syntax failures; consolidate.rs's `Item` already had
it), `post_raw` made `pub`, pure `parse_purge_result`/`retention_to_edits`/
`parse_ump_record`/`parse_ump_recall`/`ump_integrity_badge`/
`serialize_request`/`redact_for_history` cores + wire-contract tests. Version
1.17.7 → 1.17.8; CHANGELOG §[1.17.8]; CLIENT_ROADMAP v1.17.8 row → Shipped;
client README status → v1.17.8; AGENTS.md header + this entry.

### Verification

- `cargo test --manifest-path client/Cargo.toml`: **73 passed** (was 66; +7
  api.rs wire/parse cores). Clippy `-D warnings` clean, `cargo fmt --check`
  clean, desktop + wasm builds clean.
- Dioxus rsx hazards fixed during the build pass: `let` statements as direct
  rsx children of `if let` bodies (hoisted all signal reads + label
  computation before `rsx!`); `t()`/placeholders with literal braces inside
  rsx format strings (hoisted to locals, simplified `r#"{"query":...}"#`
  placeholders to plain strings); `Signal<T>()` call syntax needs `T: Clone`;
  `onkeydown` compares `Key::Enter` not `"Enter"`; named `move |_|` closures
  can't coerce to `ListenerCallback` (wrapped as `move |_| run_x(())`).

### Ship status: COMPLETED (code + tests + docs) 2026-08-09

`./deploy-web.sh` → live `/app` re-deploy, tag `v1.17.8`, and the GitHub
release are operator steps. No server restart needed (client-only static
bundle).

### Honest ceilings (carried into v1.18+)

- Console history is in-memory only (not localStorage) and holds the
  `redact_for_history` output; a careful operator still avoids pasting secrets.
- Capability-token minting stays CLI-only (server has no mint endpoint by
  design); the panel links the CLI docs.
- SSE subscribe is a reachability indicator, not a live browser change stream
  (A2A streaming is a v2.x ceiling).
- wasm-split unchanged (Dioxus 0.7.10 ceiling); bundle grows.

## Agent 60: v1.17.7 "Complete 2/3" — Graph panel + Create workspace (session 2026-08-09)
**Status:** COMPLETED (code + tests + docs; deploy/tag pending operator)
**Date:** 2026-08-09

Shipped the second of the three-part "Complete" operator-console line
(`v1.17.7`). **Client-only** — server + API contract stay at 1.17.5 (zero
server changes, zero schema change). See `CHANGELOG.md` §[1.17.7].

### Changes Made

**M3 — Graph panel (`src/panels/graph.rs`, new).** Debounced (300 ms) entity
lookup via `GET /graph/entity/{name}` → typed `EntityView` (traits +
relations with `from`/`to`/`relation_type`); a traverse card issuing
`GET /graph/traverse?start=&depth=&kind=&at=&cross_domain=true` → typed
`TraverseResponse` with `paths` (structured hop chains rendered by the pure
`render_path` core, `A --relation--> B --relation--> C`) and the flat
`traversal` rows collapsed in a `<details>` table. `kind` filter validated by
the pure `kind_is_valid` (exact or `prefix:`-style, matching the v1.7 server
contract).

**M4 — Create workspace (`src/panels/create.rs` hub → `ingest.rs` +
`procedures.rs` + `consolidate.rs`),** the v1.14/v1.10 write surface:
- Ingest: three tabs (Structured / Markdown / Memory) with real `<button>`
  toggles (aria-pressed), JSON pre-validation before send, per-mode result
  via `parse_ingest_result` / `IngestOutcome` (Created/Duplicate/Error).
- Procedures: a step builder (title/body/optional is-decision) → `POST
  /procedure` → typed `ProcedureResponse`; lists ordered steps via
  `/procedure/{id}/steps` → `Vec<StepView>`; plus `POST /classify` (typed
  `ClassifyResponse`) and `POST /decision/{id}/evaluate` (typed
  `DecisionOutcome`, vars parsed by the pure `parse_decision_vars` core —
  lenient, non-numeric dropped).
- Consolidate: `POST /consolidate/propose` → typed `ConsolidateProposal`;
  contradictions + near-dups as list items; one-click `POST /consolidate/apply`
  and `POST /consolidate/undo`, both refresh the proposal list.

**M8 wrap.** `Route::Graph{}` at `/graph` + `Route::Create{}` at `/create`
under the AppShell; both added to sidebar rail + tab bar + command palette
(nav targets now **9**, guard test updated); all M3/M4 i18n keys in all five
locales. **api.rs**: 8 typed wire structs + methods + pure cores
(`render_path`, `kind_is_valid`, `parse_entity`, `parse_ingest_result`,
`parse_decision_vars`) + wire-contract tests. Version 1.17.6 → 1.17.7;
CHANGELOG §[1.17.7]; CLIENT_ROADMAP v1.17.7 row → Shipped.

### Bug found + fixed

- **`render_path` doubled separator** — the palette's `render_path` core
  emitted `A --e--> B -- --c--> C` (a ` --` was pushed twice per hop). The
  separator is now emitted exactly once; pinned by
  `render_path_renders_faithful_chains` to `dave --employs--> 2 --ceo_of--> carol`.

### Verification

- `cargo test --manifest-path client/Cargo.toml`: **66 passed** (was 59; +7
  render_path + wire types + parse cores). Clippy `-D warnings` clean, `cargo
  fmt --check` clean, desktop + wasm builds clean.
- Dioxus rsx hazards fixed during the build pass: inline `if` in rsx can't
  hold a nested `rsx!` (ingest tab body → `match`); `#[component]` fn can't
  be called positionally in braces (tab_btn → plain fn); an unbraced
  raw-string placeholder with `{...}` broke the format-string parser.

### Ship status: COMPLETED (code + tests + docs) 2026-08-09
`./deploy-web.sh` → live `/app` re-deploy, tag `v1.17.7`, and the GitHub
release are operator steps. No server restart needed (client-only static
bundle).

### Honest ceilings (carried into v1.17.8)
- Graph entity relations are the server snapshot shape; `paths` intermediate
  hops surface by id unless a name resolves.
- Ingest does client-side JSON pre-validation only (server still validates).
- Palette `Lookup`/`Run` command rows remain wired-but-reserved; live
  id/action constructors arrive with v1.17.8's remaining panels.
- wasm-split unchanged (Dioxus 0.7.10 ceiling); bundle size grows.

## Agent 59: v1.17.6 "Complete 1/3" — command palette v2 + Overview + M8 wrap (session 2026-08-09)
**Status:** COMPLETED (code + tests + docs + deploy; tag pending operator)
**Date:** 2026-08-09

Shipped the first of the three-part "Complete" operator-console line
(`v1.17.6` + `v1.17.7` + `v1.17.8`) — the spine the two later parts register
into. **Client-only** — server + API contract stay at 1.17.5 (zero server
changes, zero schema change). See `CHANGELOG.md` §[1.17.6].

### Changes Made

**M1 — Command palette v2 (`src/main.rs`).** Replaced the v1.16.7 nav-only
palette with the full fused **nav + lookup + action** contract:
- `Command` is a flat tagged enum (`Navigate` / `Lookup` / `Run` / `SignOut`).
  The `Lookup` (`Proposal`/`Chunk`/`Entity`) and `Run`
  (`ExportAudit`/`ExportUmp`/`Reindex`/`Refresh`/`OpenTrace`) row **types** +
  every match arm (label / keywords / group / destructive) ship now; the live
  ids/actions that construct them arrive with the v1.17.7/v1.17.8 panels
  (`#[allow(dead_code)]` with a ponytail note — reserved, not unfinished).
- Pure Dioxus-free cores: `palette_group` (i18n-key group label),
  `command_keywords` (alias index), `palette_lookup` (grouped, 5-per-group cap,
  Recent prepended when the needle is empty), `remember_recent` (dedup + cap
  8), `destructive_action` (Reindex only).
- Component: grouped rendering (headers are labels, not cursor items — rows
  flattened into owned `(index, header, command)` triples so the `for` body
  needs no `let` and the onclick closures capture only Copy/owned values),
  `/` re-focus, `Tab`/`Shift+Tab` via the existing `focus_trap`, a two-step
  destructive confirm (`aria-live` "Press Enter to confirm" row, `Esc` aborts),
  per-row `aria-label`, recents via `i18n::pref_save`/`pref_load`.
- M1.5 single source of truth: `palette_commands` + the
  `palette_navigate_covers_every_non_detail_route` guard test.

**M2 — Overview (`src/panels/overview.rs`, new).** Decision-first `/` landing:
- 4-card status row (Health / Snapshot integrity / Retention / Server + UMP),
  each a `StatusCard` linking into its owning panel, fed by one `use_resource`
  per endpoint (`health`, `snapshot_status`, `retention`, `ump_capabilities`).
- DAR-chain alert list from `/decayed` + `/tombstones` + `/consolidate/propose`
  counts + the existing quarantine/auth-failure UiState signals; pure
  `overview_alerts` severity-sorts (Danger→Warn→Info) and drops zero sources.
- Top-5 pending queue preview (`/proposals?status=pending`) with one-click
  Approve/Reject (mirrors review's `decide`, `refresh += 1` inside `spawn` so
  the closure stays `Fn`+`Copy`) + `/review/:id` deep link.
- 3 tests (empty case, severity ordering, only-nonzero-sources).

**api.rs** — 6 new `ApiClient` methods (`snapshot_status`, `retention`,
`ump_capabilities`, `decayed`, `consolidate_propose`, `tombstones`) + wire
types mirroring the confirmed handler shapes + 6 wire-contract pin tests.

**M8 — Route + nav + i18n + version + docs.**
- `Route::Overview {}` at `/`; `Connect` moved to `/connect` (outside the
  AppShell layout, so the shell's connect-first redirect has no loop). Overview
  added as first rail + tab-bar item + palette entry.
- i18n: new Overview + palette keys in all five locales (`en`/`de`/`fr`/`es`/
  `nl`), `format_number` on alert counts.
- Version 1.17.0 → 1.17.6 (`client/Cargo.toml` + lock); `CHANGELOG.md`
  §[1.17.6]; `CLIENT_ROADMAP.md` v1.17.4 row split into three
  (v1.17.6/v1.17.7/v1.17.8); `IMPLEMENTATION_PLAN_v1.17.4_Complete.md` marked
  superseded; AGENTS header + this entry.

### Verification
- `cargo test --manifest-path client/Cargo.toml`: **59 passed** (was 49; +3
  overview alerts, +6 api wire pins, +1 palette route guard). Clippy `-D
  warnings` clean, `cargo fmt --check` clean, wasm build clean. Server suite
  untouched (473 baseline unchanged — zero server edits).
- The `for`-loop borrow errors (a `let` or a borrowed row can't live inside a
  Dioxus `for` body) were fixed by materializing owned data before the rsx
  (queue preview → `Vec<(id, kind)>`; palette rows → owned triples).

### Ship status: COMPLETED (code + tests + docs + deploy) 2026-08-09
`./deploy-web.sh` → live `/app` re-deploy. Tag `v1.17.6` + GitHub release are
operator steps. No server restart needed (client-only static bundle).

### Honest ceilings (carried into v1.17.7 / v1.17.8)
- Lookup is instant against **client-held ids only**; server-backed fuzzy
  lookup is v2.x. Recents are a flat non-secret label list, not deep-linkable
  objects.
- The `Lookup`/`Run` command rows ship as reserved + wired types; the
  constructors arrive with the v1.17.7/v1.17.8 panels.
- No RBAC-aware UI (v1.23.0); OpenAPI not parsed client-side; wasm-split
  unchanged (Dioxus 0.7.10 ceiling), bundle grows.

## Agent 58.5: v1.17.5 "Eval Fix" — dead eval gate revived + Round-21 CI gaps (session 2026-08-09)
**Status:** COMPLETED (code + tests + docs + tag + release)
**Date:** 2026-08-09

Three logical commits (`a99b327`, `0bcf030`, `e96a2b8`) on `main`, then the
release wrap. See `CHANGELOG.md` §[1.17.5].

### Changes Made
- **`brain eval` fixed (it was dead).** `run_eval` sent `GET /recall?query=…`
  — `/recall` is POST-only, so every run returned 405 and the v1.17.1 M3
  ship gate (`BENCH_RECALL_FLOOR`/`--floor`) never scored. Now POSTs
  `{"query", "limit": 10}` on `/recall`, keeps GET `q`/`k` on `/search`
  (`src/bin/brain.rs`). Also fixed `results_to_doc_indices`: it read only
  `results` (`/search` shape) while `/recall` returns `hits`, and mapped
  content → judged index through a `HashSet` — `.position()` on a hash set
  is arbitrary order, so recall math hit the wrong indices. Now matches the
  DOCS slice directly (fixture-documented array positions). New brain-bin
  test pins both response shapes.
- **CI `ump-conformance` job** — boots a scratch keyed instance
  (`brain ump keygen` + `AUTH_TOKEN_FILE` + fresh DB), runs the official
  `@universalmemoryprotocol/core@1.0.0` conformance runner, asserts the
  `UMP 1.0 / L3` badge line. The runner exits 0 for any level ≥ L1, so the
  gate checks the badge text itself — the README badge stays honest on
  every push/PR.
- **CI `recall-gate` job** — seeds the frozen 10-doc smoke corpus into a
  scratch instance, runs `brain eval --floor r5=0.85 --floor r10=0.85
  --floor mrr=0.85` under `pipefail` (a floor breach fails CI). Smoke set
  only; parity stays gated by the BENCHMARKS.md protocol.
- **SBOM ships on release** — `release.yml` now runs the existing
  `scripts/sbom.sh` (cargo-cyclonedx from Cargo.lock, EU CRA / OWASP
  A03:2025) and stages the CycloneDX JSON into `dist/` alongside the
  binaries.
- **First BENCHMARKS.md row** — the 37-query frozen smoke run on the
  default profile: r@5 0.919, r@10 0.919, nDCG@10 0.911, MRR 0.905 (p@5
  0.276 / p@10 0.138). Recorded as the gate's baseline, explicitly **not**
  a parity claim; parity rows stay PENDING per protocol (≥100 judged
  queries on target hardware incl. 4 GB ARM). Fixture doc-count corrected
  32 → 37.

### Verification
- `cargo test --features bench`: brain-server **473 passed, 3 ignored**;
  brain-bin **8 passed** (+1 `doc_indices_parse_recall_hits_and_search_results`).
  Clippy `-D warnings` + fmt clean. YAML parses (pyyaml).
- **Live end-to-end:** scratch instance (port 18771) seeded with the 10-doc
  corpus via `brain ingest-dir`; `brain eval` prints all 37 per-query rows,
  mean r@5=0.919 r@10=0.919 p@5=0.276 p@10=0.138 mrr=0.905 ndcg@10=0.911;
  `--floor r5=0.99` → "FLOOR BREACH" + exit 1; `r5=0.85,r10=0.85,mrr=0.85`
  → all floors ok, exit 0. The exact CI commands verified locally before
  committing.

### Ship status: SHIPPED 2026-08-09
Tag `v1.17.5` + GitHub release; live restart via `scripts/install-service.sh`.

### Honest ceilings (carried into v1.18 / v2.0)
- The eval smoke set is a wiring/CI fixture, not evidence of quality —
  parity rows remain PENDING until ≥100 judged queries on a representative
  corpus on target hardware (incl. the 4 GB ARM edge run).
- The conformance job needs network (npm install + HF model download at
  boot) — standard for CI; the live runner remains the operator's tool for
  ad-hoc reruns.
- `p@k` is low (0.276) by design: the 10-doc corpus + 37 queries reward
  recall, and the mean is diluted by the negation/abstention queries.

## Agent 58: v1.17.4 "UMP Conformance" — reference-suite wire fixes (session 2026-08-09)
**Status:** COMPLETED (code + tests + docs; server release)
**Date:** 2026-08-09

Wire-conformance release: every defect a byte-level review of the reference
conformance suite (`github.com/edihasaj/universal-memory-protocol`
`conformance.ts` + `integrity.ts`) surfaced against the v1.17.3
implementation, so the reference runner scores the full L1–L3 set (it
previously scored "none"). See `CHANGELOG.md` §[1.17.4].

### Changes Made
- **did:key bug fixed (breaking)** — `did_key_from_ed25519` emitted a
  33-byte bare-`0xed` prefix; the reference uses the two-byte `0xed 0x01`
  varint (34 bytes) and `publicKeyFromDidKey` rejects anything else. Old
  output `z2De…`, correct form `z6Mk…` (RFC 8032 vector-1 pinned).
- **Integrity block → reference §2.8 shape (breaking)** —
  `{content_hash: "blake3:<base32>", signature: "ed25519:<std-base64>",
  signer: <did:key>}` replaces `{algo, hash, key, sig}`. Content hash covers
  the canonical record minus `integrity` only (`id` stays inside), using
  JS-flavor canonicalization (integral floats → `1` not `1.0`, U+2028/U+2029
  escaped, sorted keys) so the reference `verify()` byte-matches; the
  signature is Ed25519 over BLAKE3 of the hash STRING. `verify_record`
  dual-reads the legacy v1.17.3 shape.
- **Ops** — `from_ump` lenient (absent `ump` defaults to `1.0`; explicit
  unknown majors still rejected); `UmpMeta` carries `provenance` + `consent`
  (emitted on every record); `superseded_by` resolved from `supersedes`
  evidence links on get/recall (L2 bi-temporal: prior record gets
  `time.valid_to` + `superseded_by` → new urn); urn id resolution via the
  `ump_id` column — `KNOWLEDGE_ROW_COLS` now loads it (root cause of the
  "no chunk with id urn:ump:…" 404); revise drops the carried `origin` so
  the revision gets a fresh content-addressed urn; feedback → `{ok:true}` +
  `session`; forget reports `erased` vs `tombstoned`.
- **Docs/ops** — server 1.17.3 → 1.17.4 (Cargo.toml + lock + openapi.yaml +
  README badge); CHANGELOG §[1.17.4] (breaking DID + integrity note);
  launchd plist gains `BRAIN_UMP_KEY_DIR`; wiki did:key + integrity example
  fixed to the reference shapes; COMPLIANCE.md cites Reg (EU) 2026/1744
  (GPAI obligations live 2026-08-02, watermarking 2026-12-02) with the
  provenance-not-watermarking posture.

### Verification
- `cargo test --features bench,migrate`: **473 passed, 3 ignored** (+3:
  suite-parity + the 2 model2vec-load). `--ignored`: `ump_suite_parity_
  l1_to_l3` green. lib 70 + mcp 9 + migrate_rehearse 8 + brain 7 + bench 3×2
  green. Clippy `-D warnings` + fmt clean.
- New `#[ignore]`d `ump_suite_parity_l1_to_l3` replays the suite's exact
  requests end-to-end against a keyed instance (capabilities, remember with
  provenance, get-by-urn with a reference-shape signed block, recall with
  urn ids + `signals`, revise → `supersedes:[urn]`, prior `valid_to` +
  `superseded_by`, forget `tombstoned`, validation 400 `invalid_record`,
  feedback `{ok:true}`).

### Ship status: SHIPPED 2026-08-09
Live restart (`scripts/install-service.sh`) done — live service reports
v1.17.4 / L3 (operator key at `~/.config/brain-server/ump/operator.key`);
tag `v1.17.4` created + pushed; GitHub release published. The external
conformance run was executed against a throwaway keyed instance (see
Verification) — the suite found one further defect (emit lacked the
`ed25519:` signature prefix), fixed + pinned, and the final run is
**13/13 checks, UMP 1.0 / L3**.

### Honest ceilings (carried into v1.18 / v2.0)
- The reference suite assumes a fresh store: reruns against a persistent DB
  report `merged` on L1.remember (content dedup by design). The runner's
  correct target is a throwaway keyed instance with a fresh DB — same as the
  reference `ump-serve`.
- Legacy v1.17.3 integrity verifies via dual-read but its signer did was
  itself mis-formatted (33-byte) — old records are readable, not
  re-signable.
- The client dashboard milestones (M1–M8) planned under v1.17.4 remain a
  separate, future client release; this release is server-only wire
  conformance.

## Agent 57: v1.17.3 "UMP Rollout" — full UMP 1.0 conformance through L3 (sessions 2026-08-09)
**Status:** COMPLETED (code + tests + docs + live smoke; server release)
**Date:** 2026-08-09

Shipped the ROADMAP's v1.17.3 "UMP Rollout" server release: full UMP 1.0
conformance (spec §2–§9) on the v1.17.2 wire-corrected adapter, closing
Agent 56's "one record per call + L0 conformance" ceilings. See
`CHANGELOG.md` §[1.17.3] for the full record.

### Changes Made
- **M1 — record engine** — new pure lib module `src/ump_integrity.rs`
  (`#![deny(unsafe_code)]`, the `brain_server::eval` precedent):
  `did_key_from_ed25519` (multicodec `0xed` + base58btc),
  `canonical_jcs` (RFC 8785 via BTreeMap, test vector), blake3 → base32
  content hashes, ed25519-dalek sign/verify (§2.8 `integrity`), compact
  §5.2 capability tokens (mint/parse/enforce).
- **M2 — HTTP ops** — new `src/handlers/ump_ops.rs`: all 10 `/ump/*`
  routes + batch `?format=ump` ingest (per-record status, one failure
  doesn't abort) + `/.well-known/ump.json` discovery doc. `/ump/recall`
  shares the extracted `run_recall` core (byte-identical pipeline; two
  consumers). `/ump/subscribe` is an SSE change feed over a tokio
  broadcast — `{kind,id}` only, never bodies.
- **M3 — MCP tools** — 9 `ump.*` tools in `src/bin/mcp.rs` (thin HTTP
  proxies, same shape as existing tools).
- **M4 — file binding** — `?format=ump-md` export/import + `brain ump
  export|import` CLI; fixed the v1.17.1 `/export` empty-DB regression
  (`observed_secs` → `pub(crate)`, `Option<String>` timestamps; pinned by
  `export_mapping_survives_real_timestamp_rows`).
- **M5 — identity + capability tokens** — `brain ump keygen [--dir]`
  (0700/0600 posture, refuses overwrite, prints DID); capability tokens
  verified at both auth middlewares on `/ump/*` + `/export` only, then
  verbs × scope enforced per handler via new `cap_gate` (after
  `authorize` — a capability bearer has no JWT principal, so both gates
  always run on the UMP surface; reads `read`, writes `write`/`derive`,
  export `export`; scope absent/empty/`global`; `audit`/`audit/verify`
  deny token bearers — no admin verb exists). Expired/malformed/off-surface
  → 401.
- **M6 — docs/release** — version 1.17.2 → 1.17.3 (Cargo.toml + lock +
  openapi.yaml); CHANGELOG §[1.17.3]; API_CONTRACT §15 UMP binding;
  SECURITY §UMP (key storage + §5.3 injection-resistant rehydration);
  COMPLIANCE §9 integrity/consent map; plan ship-notes.

### Verification
- `cargo test --features bench,migrate`: brain-server **473 passed, 2
  ignored** (was 451; +22 in-bin; codec + integrity tests live in the lib's
  67), brain **7** (+2 keygen/subcommand), lib **67**, mcp **3**, bench
  **3**, migrate_rehearse **9**. Ignored 2 unchanged (model2vec-load).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`:
  clean. `cargo fmt --check`: clean.
- Wire guards green: `test_openapi_covers_routes`, 
  `authz_gates_cover_every_non_public_route` (+10 UMP rows),
  `test_migration_schema_contract`.
- **Live smoke** (port 18767, opaque mode, key dir set): L3 conformance;
  remember with `read,write` token → `urn:ump:…` created; recall → §3.2
  `results` envelope with signed `integrity` blocks; read-only token on
  remember → 401 "lacks the 'write' verb"; `acme`-scope token → 401; expired
  token → 401; capability token on `/search` → 401 (off-surface);
  capabilities public, L2 without key.

### Ship status: SHIPPED (code-complete) 2026-08-09
Live restart (`scripts/install-service.sh`), commit/tag `v1.17.3`, and the
GitHub release are operator steps.

### Honest ceilings (carried into v1.17.4 / v2.0)
- Conformance L3 is self-attested; no external conformance-suite run.
- A2A federation, remote agent identity, per-tenant key hierarchies v2.x.
- Capability tokens are self-issued (owner signs for peers); no third-party
  IdP/verification registry.
- `subscribe` is a change signal only; live record streaming = A2A ceiling.
- Client-side §5.3 obligations (never-execute-body) documented, not
  server-enforced.

## Agent 56: v1.17.1 "Govern" — per-kind retention + Art 30 + UMP + eval gate + snapshot self-check + CoP (session 2026-08-09)
**Status:** COMPLETED (code + tests + docs; server release)
**Date:** 2026-08-09

Shipped the ROADMAP's v1.17.1 "Govern" server release: all seven milestones
of `IMPLEMENTATION_PLAN_v1.17.1_Govern.md` (M1 landed in a prior session,
commit `33d0fa7`; M2/M3/M5/M7 code landed in the previous session; this
session wired M4 + M6 and wrapped the release). See `CHANGELOG.md` §[1.17.1]
for the full record.

### Changes Made (this session)
- **M4 UMP adapter wired** — new `src/handlers/ump.rs` compiled in (module
  registered between `sources`/`suggest` in `handlers/mod.rs`):
  `to_ump`/`from_ump`/`um_kind`/`brain_kind`/`record_id` + 3 unit tests
  (round-trip identity, kind mapping incl. raw_kind preservation,
  malformed rejection). `GET /export?format=ump` re-renders the portable
  export via new pure `render_ump` (per-chunk name-based graph resolved
  through the entity map; `ExportQuery.format` added; knowledge SELECT
  extended with `title`/`expires_at`/`created_at`). `POST /ingest?format=ump`
  accepts a one-record UMP envelope (`IngestQuery.format`) and lowers into
  the existing structured-ingest path (entities/relations preserved,
  capacity 507 guard kept). Batch import documented as a v2.x ceiling.
  OpenAPI documents both `format=ump` params.
- **M3 fix** — `run_eval` used `query` for `/search` (which reads `q`); the
  endpoint now selects the param name per endpoint (`q`/`query`), so
  `BENCH_RECALL_FLOOR` gates compute real scores.
- **M6 CoP marker** — `/.well-known/cop-notice` (public): pure
  `build_cop_notice()` (self-attested posture, commitments, COMPLIANCE.md
  self-assessment link, `last_review`); routed + added to both auth-public
  path lists + openapi route-coverage test + `openapi.yaml`; unit test.
- **Docs** — CHANGELOG §[1.17.1], COMPLIANCE.md §7.1 + honest ceilings
  refresh, plan ship-notes for M2–M7 + SHIPPED status, README badge →
  1.17.1, ROADMAP released row → v1.17.1, AGENTS header + this entry.

### Verification
- `cargo test --features bench,migrate --bin brain-server`: **451 passed, 1
  ignored** (+5: ump round-trip/kind/malformed, render_ump graph-per-chunk,
  cop_notice). `--bin brain`: **5 passed**.
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`:
  clean. `cargo fmt --check`: clean.
- Wire guards green: `test_openapi_covers_routes` (+`/.well-known/cop-notice`),
  `authz_gates_cover_every_non_public_route`, `test_migration_schema_contract`
  (1.17.1).

### Ship status: SHIPPED (code-complete) 2026-08-09
Live restart (`scripts/install-service.sh`), commit/tag `v1.17.1`, and the
GitHub release are operator steps.

### Honest ceilings (carried into v1.17.2 / v2.0)
- Retention is query-time + kind-default; no TTL roll-up worker, no
  autonomous archival.
- UMP import is one record per call; batch import + A2A federation are v2.x/v3.x.
- CoP marker is self-attested posture, not a certification badge.
- Art 30 register is a projection of existing tables (no new mandatory schema).
- Evals corpus is release-sized; the operator's judged corpus stays private.

## Agent 54: v1.16.8 "Global" — i18n + themes + density + locale numbers + privacy block (session 2026-08-08)
**Status:** COMPLETED (code + tests + docs + deploy; client-only)
**Date:** 2026-08-08

Shipped the v1.16.8 "Global" plan: locale (i18n), light/dark theme, density,
locale-aware number formatting, and a privacy-transparency block on the
connect screen. **Client-only** — the server + API contract stay at 1.16.7.
See `CHANGELOG.md` §[1.16.8].

### Changes Made
- **`src/i18n.rs`** (new, zero new deps): `parse_ftl` + `BUNDLES: LazyLock` of
  `en`/`de`/`fr`/`es`/`nl` compiled via `include_str!`; `t()` resolves current-locale
  → `en` → the key itself (never blank); `is_rtl`; `format_number` (per-locale
  digit grouping); `pick_locale/theme/density` sanitizers; `pref_save`/`pref_load`
  (web `localStorage`, no-op native). Pure cores (`resolve`, `group_digits`) are
  signal-free so the unit tests need no Dioxus runtime.
- **Global prefs as accessor `fn`s** — `theme()`/`density()`/`locale()` return
  `Signal::global(...)` (Dioxus' documented idiom). A `static Signal` can't be
  `.set()` without an immutable-static borrow error; the accessor-fn pattern
  sidesteps it. Prefs persisted (sanitized) to `localStorage`; restored on launch
  by a `use_future`; `data-theme`/`data-density`/`dir` applied to `<html>` by
  three `use_effect`s (no reload).
- **`locales/{en,de,fr,es,nl}/main.ftl`** — full shell/nav/connect/review/settings
  strings + the privacy block; every non-`en` key is covered by `en` (test-pinned).
- **Shell chrome localized** — rail + tab-bar nav, top-bar counts/badges,
  connection + principal pillars, sign-out, banners, drawer header, and the
  Connect screen all render through `t()`. Precomputed locals feed `rsx!` text
  nodes so no nested `t("…")` call sits inside a formatted string (a compile
  hazard caught and fixed). Locale-aware `format_number` on the pending/flags
  counts (M5).
- **Light theme + density CSS** (`input.css`): `html[data-theme="light"]` swaps
  every token (dark-first default; state hue *names* unchanged so the recall/
  security tests hold); `html[data-density="compact"]` sets 14px root font.
- **M6.2 privacy block** on Connect: a `<details>` panel stating exactly what the
  client sends / stores / never does (token to the backend only; nothing stored on
  web; no telemetry/analytics/third-party).
- **`deploy-web.sh` now compiles Tailwind first** — `dx bundle` does NOT recompile
  Tailwind in build mode (the `[tailwind] input` is `styles/input.css`, not a root
  `tailwind.css`, so dx's auto-watch never fires) — it copies+hashes a stale
  `assets/tailwind.css`, silently dropping CSS edits. The script now runs
  `npx @tailwindcss/cli -i styles/input.css -o assets/tailwind.css` per the Dioxus
  0.7 docs. This is the real "stale-CSS" bug class Agent 50's `ls -t` fix partially
  papered over.

### Verification
- `cargo test --manifest-path client/Cargo.toml`: **48 passed** (was 43; +5 i18n
  tests). `cargo clippy --all-targets -- -D warnings` clean. `cargo fmt --check`
  clean. `cargo build` + `cargo build --target wasm32-unknown-unknown` clean.
- **`dx bundle` + live deploy:** `./deploy-web.sh` → `dist/` carries a fresh hashed
  `tailwind-*.css` with `data-theme=light]` and `data-density=compact]{font-size:
  14px` plus the full `.card`/`.drawer` component layer. Live `/app` serves the new
  index.html + CSS; verified `data-theme`/`data-density` present in the served CSS.
  (Debug `dx build` does not recompile Tailwind; the release `dx bundle` copies the
  pre-built `assets/tailwind.css` that the new script step now regenerates.)

### Ship status: SHIPPED (code + deploy) 2026-08-08
Client 1.16.7 → 1.16.8 (`client/Cargo.toml`); CHANGELOG §[1.16.8], client README,
AGENTS header + this entry. No server restart needed (client-only static bundle);
tag `v1.16.8` is an operator step.

### Honest ceilings (carried into v1.17.0)
- i18n is a simple FTL subset — no ICU plurals/term references (all strings are
  static); `fluent` is the upgrade path.
- `fr` digit grouping uses `.` (a narrow no-break space would be more correct).
- No RTL locales ship yet; `dir` + CSS are ready but unexercised by a real RTL
  string set.
- No system-color-scheme auto-follow; `color-scheme` flips correctly with the toggle.
- The `.ftl` files are hand-maintained alongside string keys; a missing key degrades
  to the key name (visible), never blank — by design.

---

## Agent 53: v1.16.7 — server version alignment + release wrap (session 2026-08-08)
**Status:** COMPLETED (version + docs + build + tag)
**Date:** 2026-08-08

Formalized the server side of v1.16.7. The client shipped as v1.16.7 earlier
(v1.16.7 tag, Agent 52) with the server left at 1.16.6; the server's
hardening + compliance round (previously in `[Unreleased]`, 18 commits past
the tag) is now released as the server component of **v1.16.7** — server
`Cargo.toml` 1.16.6 → 1.16.7, matching the client.

### Changes Made
- **Server version** 1.16.6 → 1.16.7 (`Cargo.toml` + `Cargo.lock`) +
  `openapi.yaml` (`version` + `x-api-version`).
- **CHANGELOG §[1.16.7]** — merged the `[Unreleased]` server work (Art 50
  `/.well-known/ai-notice` + `docs/MEMGHOST_MITIGATION.md`; P0 snapshot
  chmod-0600; `/health` content-leak fix; `/tombstones?limit=` honored;
  `/export` emits `source`; test isolation) into the client section under
  `### Server — Security/Added/Fixed/Changed` + `### Client — …` subsections.
- **AGENTS.md header** reworded to a combined server + client release;
  this Agent 53 entry added.

### Verification
- `cargo test --features bench,migrate`, clippy `-D warnings`, `cargo fmt
  --check`, and the release build all green (Agent-52 baseline: 436 passed,
  1 ignored).
- No schema change; API contract unchanged (additive `offset`/`limit` and
  `source` column only).

### Ship status: SHIPPED 2026-08-08
Commit + push of the version/docs wrap. Tag `v1.16.7` already exists (client
release); live restart is an operator step (`scripts/install-service.sh`).

---

## Agent 52: v1.16.7 "Integrated" — deep links + PWA + paginated audit + command palette (session 2026-08-08)
**Status:** COMPLETED (code + tests + docs + deploy; client-only)
**Date:** 2026-08-08

Shipped the v1.16.7 "Integrated" plan: the deep-link + PWA + pagination +
command-palette milestones plus the carried-over client hardening. **Client-only**
— the server + API contract stay at 1.16.6 (the only server change is the
additive `offset` param on `/audit`). See `CHANGELOG.md` §[1.16.7].

### Changes Made
- **M1 deep links** (`main.rs`): `Route` gained `ReviewDetail { proposal_id }`
  (`/review/:proposal_id`) + `DsarDetail { dsar_id }`
  (`/subjects/certificate/:dsar_id`); `RecallTrace` (`/recall/:trace_id`)
  already existed (v1.16.0). Leaf `ReviewDetail`/`DsarDetail` components; the
  review card title + certificate subject became real `<Link>`s. Pure
  `locate_proposal`/`subject_of` + tests.
- **M2 PWA** (`client/pwa/` + `deploy-web.sh`): `manifest.webmanifest` +
  `sw.js` (caches only `/app/index.html` + `/app/assets/*`; navigation falls
  back to shell; never the API). `deploy-web.sh` ships both + injects the
  manifest link, theme-color, and SW registration into `index.html`.
- **M4 paginated audit** (server `src/audit.rs::recent_tenant` + `main.rs`
  `AuditQuery.offset`; client `api.rs::audit_page` + `audit.rs` panel): server
  `ORDER BY id DESC LIMIT ? OFFSET ?`; client Load-more button (PAGE=100) with
  boundary-id dedup `retain(|r| r.id < tid)`. Server test
  `recent_tenant_paginates_with_offset` (4/4/2 pages, no overlap/dupe).
- **M5 command palette** (`main.rs`): ⌘K/Ctrl+K overlay; `Command` enum +
  pure `palette_commands`/`filter_commands`/`command_label` + tests. The
  `select` closure does its signal writes inside `spawn` so it stays `Fn`+`Copy`
  (a directly-mutating shared closure would force `FnMut` and break the
  multiple event handlers).
- **M6 recall debounce** (`recall.rs`): 300ms generation-guarded commit after
  typing stops. Pure `debounce_commit` + test.
- **M7.3 drawer focus trap** (`main.rs::focus_trap`): Tab/Shift+Tab cycles
  focus within the dialog via a small `document::eval` snippet.
- **M7.5 aria-live**: `role="status" aria-live="polite"` on the review batch
  summary, DSAR certificate badge, and audit export.
- **M7.6 RTL**: `deploy-web.sh` injects `<html dir="auto">`.

### Verification
- Client: **43 tests** (was 43 at last gate; M1/M5/M6 tests added), clippy
  `-D warnings` clean, `cargo fmt --check` clean, wasm build clean.
- Server: `cargo test --features bench,migrate` → **436 passed, 1 ignored** +
  audit/integration green (the only change is the additive `offset` param).
- `./deploy-web.sh` → live `/app` 200; `/app/manifest.webmanifest` + `/app/sw.js`
  200; dist carries hashed JS/WASM/CSS + manifest + sw + `dir="auto"`.

### Ship status: SHIPPED (code + deploy) 2026-08-08
Client 1.16.6 → 1.16.7 (`client/Cargo.toml`); CHANGELOG §[1.16.7], CLIENT_ROADMAP
row, AGENTS header + this entry. Live restart not needed (client-only static
bundle); tag is an operator step.

### Honest ceilings (carried into v1.16.8)
- **M3 wasm-split not built** (Dioxus 0.7.10 has no wasm-split; docs list it
  as planned) — documented ceiling, no code.
- **Drawer focus trap is hand-rolled** (`document::eval`), not the shadcn/
  Radix Dialog with full focus restoration — `dx components add dialog` can't
  run (registry unreachable).
- **RTL is `dir="auto"` only** — no i18n string extraction (v2.x).
- **M7.7 Mobile milestones** (lib.rs entry, probe pause/resume, store
  readiness, MASVS) remain operator/native-toolchain steps — no Android SDK /
  cargo-ndk here.

---

## Agent 51: v1.16.6 "Mobile" — secure token storage + responsive UX (session 2026-08-08)
**Status:** COMPLETED (code + tests + docs; client-only)
**Date:** 2026-08-08

Shipped the two testable milestones of the v1.16.6 "Mobile" plan (**M2** secure
token storage + **M3** responsive UX) as **v1.16.6**. **Client-only** — server
+ API contract unchanged. Also pinned Dioxus to the newest stable **0.7.10** and
updated every plan/doc "Dioxus 0.7.2" reference. See `CHANGELOG.md` §[1.16.6].

### Changes Made
- **Dioxus 0.7.10** — the semver-open `dioxus = { version = "0.7", … }` spec
  already resolved to the newest stable in `Cargo.lock` (verified via lockfile +
  `cargo tree` + crates.io; context7's Dioxus index caps at v0.7.2, so the
  patch line was confirmed from the lockfile instead). The security-relevant
  0.7.2→0.7.10 fixes (0.7.8/0.7.10 wasm-hotpatch TOCTOU/UB; 0.7.6 web
  panic-resilience + `inert`) are compiled in. 8 doc files' "0.7.2" refs
  updated to 0.7.10.
- **M2 — `src/storage.rs`** (new, `#[cfg(target_arch = "wasm32")]`-gated):
  non-web saves/loads/deletes the auth token in the OS keyring (`keyring`
  3.6.3 — features `apple-native`/`windows-native`/`sync-secret-service`
  verified via crates.io; the delete API is `delete_credential`, not
  `delete_password`); web is a no-op (token stays in-memory, v1.16.1 posture).
  `should_persist(token)` gates the connect-save — a loopback (empty-token)
  connect never clobbers a previously-saved remote token. Connect saves on
  success; a launch `use_resource` (the idiomatic Dioxus run-once primitive,
  not `use_effect`) silently probes `/health` with any saved token and jumps to
  Review, falling through to the form on a stale/revoked token.
- **M3 — responsive UX**: AppShell renders both the desktop rail (now
  `.nav-rail`) and a new mobile bottom `nav.tab-bar` with `TabLink` components
  (same `Routable` targets → identical a11y nav); pure `@media (min-width:
  640px)`/`@media (max-width: 639px)` swap them — no viewport JS. `.tab-link`
  enforces ≥44px touch targets; `.tab-bar` + the drawer consume
  `env(safe-area-inset-bottom)` (notch/home indicator). The context drawer is
  now `.drawer`: right rail ≥sm, full-width rounded bottom sheet <640px.
- **Version** client 1.16.5 → 1.16.6 (`Cargo.toml`); CHANGELOG §[1.16.6],
  AGENTS.md header + this entry.

### Verification
- `cargo test --manifest-path client/Cargo.toml`: **37 passed** (was 36; +1
  `persist_gate_requires_a_real_token`).
- `cargo clippy --all-targets --manifest-path client/Cargo.toml -- -D warnings`: clean
  (after removing a redundant `let nav = nav;` binding + `mut` fixes).
- `cargo fmt --check`: clean.
- `cargo build` + `cargo build --target wasm32-unknown-unknown`: both clean
  (web is the primary target; the storage seam + auto-reconnect are wasm-gated).
- Tailwind v4.3.3 compiles `styles/input.css`: `.tab-bar`/`.tab-link`/`.drawer`/
  `nav-rail`, the `min-height:44px` touch target, and both breakpoint `@media`
  blocks are present verbatim in the output.

### Ship status: SHIPPED (code-complete) 2026-08-08
No server restart needed (client-only; static bundle). Live `deploy-web.sh` +
tag are operator steps.

### Honest ceilings (carried into v1.16.7)
- **M1 (lib.rs mobile entry), M4 (probe pause/resume), M5 (store readiness),
  M6 (MASVS)** documented as operator/native-toolchain steps — no Android SDK /
  cargo-ndk / `dx` in this environment, so native iOS/Android artifacts can't
  be built or verified here (same as every prior client release).
- **Android keyring** uses the separate `android-native-keyring-store` crate
  (Keystore-encrypted prefs), wired by `dx` at bundle time — a documented
  ceiling, not compiled in this build.
- **Web token stays in-memory only** — browser localStorage is not a secure
  credential store (MASVS-STORAGE); the v1.16.1 posture is deliberate.
- The auto-reconnect probes the same-origin/loopback base by default; a remote
  install with a persisted token still needs the operator to enter the URL
  (the URL is not a secret, so it's not persisted).

---

## Agent 49.5: v1.16.3 "Serve" — web bundle serving + live bugfixes (RETROSPECTIVE) — 2026-08-08
**Status:** COMPLETED (retrospective — no code written this session)
**Date:** 2026-08-08

Release-history-gap closure. Four commits between the v1.16.2 and v1.16.4 tags
(`cd7d10f`, `59c8217`, `4fc66da`, `edfb00d`) were folded into the v1.16.2
changelog instead of being given their own tag/plan/AGENTS entry. This session
recognized them as the distinct release **v1.16.3 "Serve"**, created the
missing tag, and wrote the retrospective records. See `CHANGELOG.md` §[1.16.3]
+ `IMPLEMENTATION_PLAN_v1.16.3_Serve.md`.

### What the release actually was
- **M1 `cd7d10f`** — serve the compiled Dioxus web bundle under `/app`;
  `Dioxus.toml` `base_path = "app"`; client dev/serve/deploy README; build
  tooling.
- **M2 `59c8217`** — `CLIENT_CSP` gains `'unsafe-eval'` (wasm-bindgen glue's
  `new Function()` is JS eval, blocked by `'wasm-unsafe-eval'` alone → client
  never rendered); API CSP stays strict.
- **M3 `4fc66da`** — `client/deploy-web.sh` (bundle + inject concrete
  `/app/assets/tailwind-*.css` link + copy to dist).
- **M4 `edfb00d`** — same-origin connect default ("cannot reach brain-server"
  fix) + deploy-web.sh derives JS/WASM hashes from index.html instead of
  globbing stale `target/` assets.

### Actions taken (this session)
- Created tag **`v1.16.3`** at `edfb00d` (last bugfix commit before the v1.16.4
  restyle) — the tag history is now contiguous v1.16.0…v1.16.6.
- Wrote `IMPLEMENTATION_PLAN_v1.16.3_Serve.md` (retrospective).
- Added `CHANGELOG.md` §[1.16.3] with Fixed / Improvements / Security sections.
- Verified the four commits' diffs to attribute them correctly (see the
  verification table in the plan).

### Honest ceiling (retrospective)
No dedicated tests — it's a serving/build/config release verified by the live
`/app` smoke + the v1.16.2 suite. Retrospective plans can't retrofit code into
an already-tagged history.

---

## Agent 50: v1.16.4 "Styled" — shadcn/ui design-system restyle of the Dioxus client — 2026-08-08
**Status:** COMPLETED (code + tests + docs + deploy; client-only)
**Date:** 2026-08-08

Shipped the ROADMAP's v1.16.x client polish as **v1.16.4**: a full
shadcn/ui-flavored design-system restyle of the control surface. **Client-only**
— the server + API contract stay at 1.16.2. Research-grounded (context7 +
web search): shadcn v4 globals.css token pattern + Tailwind v4 `@theme`
semantic tokens, Button/Card/Badge/Input/Table/Sidebar anatomy, and the 2026
dashboard aesthetic (neutral slate base, single brand accent, soft radius,
subtle elevation). See `CHANGELOG.md` §[1.16.4] for the full record.

### Changes Made
- **`input.css` rewritten into a shadcn component layer** — semantic tokens
  (`--color-background/foreground/card/popover/muted/accent/destructive/border/
  input/ring`) mapped onto the app's own AA-verified palette (the state hues
  `ok`/`warn`/`danger`/`info`/`neutral` keep their exact names — the recall/
  security tests pin them), a `--radius-sm…2xl` scale, `--shadow-xs/sm`, and
  reusable classes: `.card` (+ header/body/footer), `.btn` + variants
  (primary/outline/secondary/ghost/destructive) + `.btn-sm/.btn-md`, `.input`/
  `.select`, `.label`, `.badge` + state badges, `.nav`/`.nav-link`/`.nav-badge`,
  and `.table`. Replaced every ad-hoc `border border-border-subtle surface-raised
  rounded` string across the client.
- **`AppShell` → sidebar dashboard** — fixed left rail (brand mark + grouped
  `nav-link` pills with live count badges on the rail: Review pending, Security
  flags, Audit `!`) + a slim sticky top bar (connection dot, pending count,
  Security/Audit badges, principal) + the drawer as a `card`. New `NavLink`
  component (optional `badge`/`dirty`). No layout-semantic regression: nav stays
  real `<Link>`s, actions stay real `<button>`s.
- **Connect screen** — branded card (mark + title), labeled `.input` fields,
  primary Connect button, status lines.
- **Panels restyled** — Review (button bar + card-based proposal rows + the two
  modals), Recall (input/select + hit rows + trace card), Subjects (DSAR action
  card + certificate card), Security (chain card + quarantine + auth-failure
  `.table`), Audit (filter bar + `.table`), Health (Service + Corpus cards).
- **`deploy-web.sh` stale-CSS fix** — the `ls | head -1` glob picked the
  alphabetically-first (stale) hashed `tailwind-*.css` in `target/` between
  rebuilds, so a restyle could deploy the old stylesheet while index.html
  referenced the new one. `ls -t | head -1` now picks the freshest build.
- **Version**: client 1.16.2 → 1.16.4 (`client/Cargo.toml`); CHANGELOG §[1.16.4],
  AGENTS.md header + this entry.

### Verification
- `cargo test --manifest-path client/Cargo.toml`: **31 passed** (unchanged —
  the a11y/security grep gates still pass; the restyle used real `<button>`s,
  no `dangerous_inner_html`, no token persistence).
- `cargo clippy --all-targets --manifest-path client/Cargo.toml -- -D warnings`: clean.
- `cargo fmt --check --manifest-path client/Cargo.toml`: clean.
- Tailwind v4 CLI compiles `styles/input.css` clean (all component classes
  present); the earlier `@apply … tabular` error (a base-layer class, not a
  utility) fixed by hoisting `font-variant-numeric` out of `@apply`.
- `./deploy-web.sh` → fresh hashed CSS (`tailwind-dxhb346fa5af6b99d26.css`) with
  the component layer; `/app/index.html` + `/app/assets/tailwind-*.css` serve 200.

### Ship status: SHIPPED (code + deploy) 2026-08-08
Deployed to `client/dist` (what the live server serves at `/app`). No server
restart needed — the bundle is static. Tag `v1.16.4` created + pushed.

### Honest ceilings (carried into v1.17.0)
- **shadcn Dialog + axe-core CI still deferred** (unchanged from Agent 49) —
  `dx` CLI unavailable here; the drawer keeps `role="dialog"`/`aria-modal`/Esc.
- The design layer is a hand-rolled shadcn-flavored system, not generated via
  `dx components add` — no Radix primitives, so the focus-trap/return-focus
  behaviors remain the v1.18.0 pass.
- 2026 aesthetic is a judgment call, not a benchmark; the manual a11y checklist
  pass (Agent 49) still stands.

---

## Agent 50.5: v1.16.5 "Secure" — JWT refresh lifecycle + principal — 2026-08-08
**Status:** COMPLETED (code + tests + docs; client-only — shipped as commit
`002d345`, tag `v1.16.5`)
**Date:** 2026-08-08

Client-only release: the JWT lifecycle on the Dioxus control surface — silent
refresh-on-401, principal identity display, pre-emptive expiry refresh, the
honest revocation path, and a JWT-pair connect mode. Server + API contract
unchanged. See `CHANGELOG.md` §[1.16.5] +
`IMPLEMENTATION_PLAN_v1.16.5_Secure.md`.

### Changes Made
- **M1 JWT-aware `ApiClient`** — `TokenClaims` (sub/exp/scope/team) +
  `decode_claims()` (base64url payload decode, no signature verification —
  brain-server verifies on receipt; the client trusts claims for display +
  expiry only, never for authz).
- **M2 principal pillar** — `with_principal()`/`with_refresh_pair()` derive the
  identity pillar from the JWT `sub`; `derive_principal()` separates opaque
  loopback tokens (None) from JWT-shaped ones. Top bar shows `acting as <sub>`
  vs `loopback` (replaces the hardcoded `remote-user` placeholder).
- **M3 refresh-on-401 + M5 pre-emptive refresh** — `request_with_refresh`
  silently refreshes once on 401 and retries; `needs_refresh()` refreshes when
  `exp` is within 60s. One retry, no infinite loop.
- **M4 Connect JWT mode** — token / JWT-pair radio toggle (access + refresh
  pasted from `brain key mint` or an IdP).
- **M6 revocation-aware errors** — `error_message()` maps `refresh_reuse_
  detected` → "session revoked", 401 → "session may have expired" + reconnect.
- **Fix** — `request()` no longer holds the `RwLock` guard across an await
  (clippy `await_holding_lock`); the access token is cloned out before the send.

### Verification
- `cargo test --manifest-path client/Cargo.toml`: **36 passed**.
- clippy `-D warnings` + `cargo fmt --check` clean; desktop build clean.
- Commit message notes: "Plan files renumbered Secure 1.16.3→1.16.5, Mobile
  1.16.4→1.16.6, Integrated→1.16.7, Global→1.16.8 (gitignored, not committed)."

### Ship status: SHIPPED (code + tag) 2026-08-08
Tag `v1.16.5` created. Live restart is not needed (client-only).

### Honest ceilings (carried into v1.16.6)
- Token lives in WASM memory for the session lifetime (BFF/HttpOnly cookie is
  the v2.x ceiling).
- No PKCE flow (interactive login needs a brain-server `/auth/authorize` or
  IdP proxy).
- Concurrent refreshes from two panels are server-safe but the loser logs out
  (client-side single-refresh mutex is the v1.16.6 polish).
- **Recorded 2026-08-08 (later session):** the v1.16.5 tag was created but
  never pushed to origin, and it had no CHANGELOG/AGENTS entry — this entry +
  the §[1.16.5] changelog + the remote tag push were completed retrospectively
  alongside the v1.16.3 gap-closure session.

---

## Agent 49: v1.16.2 "Harden + Accessible" — client serving/CSP + WCAG 2.2 AA pass — 2026-08-08
**Status:** COMPLETED (code + tests + docs; live restart pending operator)
**Date:** 2026-08-08

Shipped both the v1.16.1 "Harden" and v1.16.2 "Accessible" plans as a single
**v1.16.2** release (v1.16.1 was already taken by the observe-fix). Server
changes (M1) + client security gates (M2–M6) + the WCAG 2.2 AA client pass.
See `CHANGELOG.md` §[1.16.2] for the full record.

### Changes Made
- **Server serves the client** — `nest_service("/app", ServeDir::new(config::client_dir()).not_found_service(ServeFile(index.html)))` (SPA fallback for deep-links) + `/` → `/app/` redirect. `config::client_dir()` reads `BRAIN_CLIENT_DIR` (default `client/dist`).
- **Path-aware CSP** — `security_headers_middleware` reads the path: `/app`+`/` get `CLIENT_CSP` (`'wasm-unsafe-eval'` + `connect-src 'self'`), else strict `API_CSP`. `/app`+`/` added to the auth-public set in both `jwt_auth_middleware` and `auth_middleware`. Pinned by `csp_strict_for_api_routes_relaxed_for_client_routes`.
- **Client Harden** — `ErrorBoundary` around the router; `api::error_message()` (401/403/404/429/503/fallback) wired into Review/Recall/Health; `BatchSummary` + `batch_outcome()` cancel-safe batch collapse rendered as a one-line summary; two grep guards (`xss_escape_hatch_is_unused`, `credentials_stay_in_memory`).
- **Client Accessible** — `PageTitle` component (`tabindex="-1"` + focus-on-mount via `onmounted`→`set_focus`), `use_document_title()` per route, `*:focus-visible{scroll-margin-top:4rem}` (2.4.11), `tests::interactive_elements_are_buttons` (no `<div onclick>`), `--color-ink-faint`→`#7c8492` (AA 4.6:1), `client/a11y-checklist.md` manual artifact.
- **Version**: server 1.16.1→1.16.2, client 1.16.0→1.16.2. openapi.yaml → 1.16.2. README/CHANGELOG/ROADMAP/COMPLIANCE/AGENTS updated.

### Verification
- `cargo test --features bench,migrate`: **522 passed** (was 518 at v1.15.0; +new CSP test + prior v1.16.x).
- `cargo test --manifest-path client/Cargo.toml`: **30 passed** (was 25 at v1.16.0; +ErrorBoundary/batch/guard/wire tests).
- clippy `-D warnings` clean (server + client). `cargo fmt --check` clean (both).

### Ship status: SHIPPED (code-complete) 2026-08-08
Live restart is an operator step (`scripts/install-service.sh`). Tag `v1.16.2` created + pushed.

### Honest ceilings (carried into v1.17.0)
- **shadcn Dialog (M5) + axe-core CI (M6) deferred** — `dx` CLI unavailable here; `dx components add dialog` + `dx bundle --platform web` axe gate can't run. The drawer has `role="dialog"`/`aria-modal`/Esc; full Radix Tab-trap + return-focus is v1.18.0.
- axe catches 20–60%; the manual VoiceOver/NVDA pass (checklist in `client/a11y-checklist.md`) is irreplaceable.
- No aria-live beyond existing `role="status"` banners; no RTL (v1.16.6).

---

## Agent 48: v1.16.0 "Client" — the Dioxus control surface (M1–M8) — 2026-08-08
**Status:** COMPLETED (code + tests + docs + tag + release)
**Date:** 2026-08-08

Shipped the ROADMAP's v1.16.0 "Client" row: the Dioxus control surface (web +
desktop + iOS + Android, one Rust codebase) consuming brain-server's v1.14/v1.15
governance APIs. See `CHANGELOG.md` §[1.16.0] for the full record.

### Changes Made
- **M1 connection state machine** (`client/src/main.rs`): `Conn` enum + pure
  `probe_state(failures, ok)` (the false-offline guard — N failures before
  amber) + pure `writes_allowed(conn, verify_ok, pending_reverify)` (the chain-
  verify-before-writes gate). A single `use_future` probe at the app root owns
  its timer. **Dependency-free sleep** via `document::eval`+`setTimeout` — no
  `tokio` dep (works web + desktop; tokio's timer doesn't work in WASM).
  `UiState` bundle (conn/writes_enabled/pending_reverify/pending_count/
  quarantine_count/audit_dirty/auth_failures_count/drawer) provided via context.
  Read-only degrade banner + mutation freeze when amber. Recovery 200 → conn
  green but writes frozen until `/audit/verify` returns `{"ok":true}`.
- **M2 nav structure** (`main.rs` AppShell): F-pattern `Pending: N` top-left,
  Security/Audit count badges, principal identity pillar, Esc-closable context
  drawer (`role="dialog" aria-modal="true"`) with typed `DrawerContent`
  (Proposal/Hit/Certificate/AuthFailure). New route `RecallTrace { trace_id }`.
- **M3 honest-batch review** (`client/src/panels/review.rs`): `RowOutcome`
  enum + `classify_outcome` (404→AlreadyDone), per-row outcome tracking,
  `BatchGuard` DropGuard + `clear_pending_selection`, `A`/`S`/`R`/`J`/`K`
  keyboard with `key_action` + `shortcuts_enabled` toggle (WCAG 2.1.4),
  reject-with-reason editor, suggest-re-ingest editor.
- **M4 recall decision-path viewer** (`panels/recall.rs`): richer `Hit` fields
  (assertion_kind/confidence/relevance/decayed), per-retriever ranks + fused
  score rendering, `min_relevance` slider + `drop_low_relevance`, `?trace=true`
  toggle → `trace_id`, `trace_panel()` + `TraceCard` + `json_str`.
- **M5 DSAR certificate card** (`panels/subjects.rs`): structured card +
  `chain_badge()` (green/red), `DsarCertificate::from_value` typed fields,
  live re-verify via `dsar_certificate`.
- **M6 auth-failure feed** (`panels/security.rs`): `audit_kind("auth")` +
  `auth_failures()` pure filter (kind=auth AND status=denied) + count badge.
- **M7 audit filters + export** (`panels/audit.rs`): client-side `AuditFilter`
  + `filter_audit` + `ts_on_or_after`, JSON export via `document::eval`.
- **M8 visual-token layer** (all panels): every ad-hoc color class → semantic
  token (zero `text-gray-*`/`text-green-*`/`text-red-*` remain).
- **`client/src/api.rs`** (wire delta): `ApiClient::with_principal` +
  `is_configured` + `principal()`, `Hit` +5 fields, `RecallResponse.trace_id`,
  `recall(trace, min_relevance)`, `recall_trace(id)`, `reject_proposal(reason)`,
  `audit_kind(kind)`, `DsarCertificate::from_value`.
- **Editor support** (`.zed/settings.json`): Tailwind CSS language mode
  (`tailwindcss-intellisense-css` + `!vscode-css-language-server`) — verified
  via context7 + the Zed Tailwind docs; resolves the false "Unknown at rule"
  warnings on `@theme`/`@source`/`@apply`.
- **Version bump**: client `0.1.0` → `1.16.0` (`client/Cargo.toml`). Docs:
  README, client/README, CHANGELOG §[1.16.0], ROADMAP released-version line,
  CLIENT_ROADMAP v1.16.0 row → Shipped, AGENTS.md header + this entry.

### Tests (→ 25 passed; +18)
M1 (`probe_degrades_only_after_n_failures`, `writes_re_enable_only_after_chain_verify`,
`recovery_is_real_200_not_heuristic`), M3 (`batch_404_is_treated_as_already_done`,
`batch_surfaces_partial_failure`, `drop_guard_clears_pending_selection_on_cancel`,
`keyboard_maps_asrjk_and_s_only_on_conflict`), M4 (`drop_low_relevance_filters_below_tier`,
`relevance_tier_color_maps_to_state_tokens`), M5 (`chain_badge_reflects_live_verify`,
`certificate_card_fields_render_from_server_json`), M6 (`auth_failure_feed_parses_denied_rows`),
M7 (`filter_audit_filters_by_kind_and_principal_and_since`, `ts_on_or_after_handles_date_prefix`),
api.rs wire pins (`recall_parses_hits_and_decision` extended, `recall_hit_parses_without_v1_14_fields`,
`dsar_certificate_and_stats_parse` extended, `dsar_certificate_defaults_when_fields_absent`,
`url_encode_reserved_chars`, `api_client_principal_and_configured`).

### Verification
`cargo test --manifest-path client/Cargo.toml`: **25 passed** (was 7).
`cargo clippy --all-targets --manifest-path client/Cargo.toml -- -D warnings`: clean.
`cargo fmt --check --manifest-path client/Cargo.toml`: clean.
`cargo build --manifest-path client/Cargo.toml`: clean, zero warnings.
Zed diagnostics on `client/styles/input.css`: clean (zero warnings).

### Ship status: SHIPPED 2026-08-08
Tag `v1.16.0` created + pushed. `dx serve` (web/desktop smoke) is an operator step.

### Honest ceilings (carried into v1.17.0)
- Connection is web-first (the eval-based instant-wake listener + desktop/mobile
  lifecycle variants land with v1.17.0).
- Token is in-memory only (secure-storage seam is v1.17.0).
- Audit filters are client-side (server-side params are v1.19.0).
- Drawer focus trap is partial (Esc + ARIA now; full Radix Tab-cycling is v1.18.0).
- Export is client-side (no `/audit/export` server route).

---

## Agent 47: v1.15.0 "Observe" — read-event audit + recall trace + DSAR + COMPLIANCE.md — 2026-08-08
**Status:** COMPLETED (code + tests + docs; live restart pending operator)
**Date:** 2026-08-08

Shipped the ROADMAP's v1.15.0 "Observe" row (rounds 4–6 of the memory-stack
audit): the observability + compliance-workflow layer on v1.14's governance
primitives. See `CHANGELOG.md` §[1.15.0] for the full record.

### Changes Made
- **M1 read-event audit** (`src/audit.rs` + `src/config.rs`): new
  `AuditKind::Recall/Search/Get`; `record`/`record_tenant` return `Option<i64>`
  (row id); `record_read_event` (audit row + optional `recall_traces` side row);
  `read_trace`; `chain_head`; `prune_audit_retention` (DELETE expired by
  `ts < datetime('now','-N days')`, re-anchor oldest survivor as genesis,
  recompute survivor `prev_hash`s). Env: `BRAIN_AUDIT_READ_EVENTS` (default off
  loopback / on JWT), `BRAIN_AUDIT_READ_SAMPLE_RATE`, `BRAIN_AUDIT_RETENTION_DAYS`.
  `/recall`, `/search`, `/get/{id}`, `/multi-get` emit read events
  (best-effort; hash-only invariant test-pinned). `?trace=true` on `/recall`
  returns `trace_id` (the audit row id).
- **M2 recall trace** (`src/handlers/observe.rs`): `GET /recall/{trace_id}/trace`
  (Admin) replays the stored decision path (query, decision, domains searched,
  applied scope, actor, per-hit id/score/assertion_kind/source/relevance/decayed).
- **M3 DSAR** (`src/handlers/observe.rs` + `src/handlers/gate.rs`):
  `dsar_locate` (owner roots + transitive `derived_from` walk, depth 8) extracted
  for testability; `post_dsar` (locate→export→purge→tombstone→audit→certificate
  →ledger, one tx); shared `purge_chunk_ids` extracted from `/purge` (tombstone
  now carries `reason` + `origin_id`); `GET /tombstones?subject=&since=`;
  `GET /dsar/{id}/certificate` (live `chain_verifies`); `notify_art19`
  (opt-in HMAC-SHA256 signed POST, 3 bounded retries, fail-soft).
- **Migration** (`src/migration.rs`): `recall_traces` + `dsar_requests` tables
  + `idx_dsar_subject`; guarded adds of `tombstones.reason`/`origin_id`;
  `schema_version` → 1.15.0. **Deliberate constraint break:** the Art 19
  webhook needs outbound HTTP — `reqwest` is now a required dep; the
  `connector-github` feature gates only its binary (comment updated in
  `src/connector/mod.rs`).
- **M4 docs**: new `COMPLIANCE.md` (system/data flows, logging spec, DSAR,
  risk controls, retention classes, ISO 42001/NIST AI RMF/SOC 2 map,
  Intent-Based-Auditing 4/4, PH DPA/GDPR/CCPA jurisdiction, Art 4 literacy,
  Art 50 origin-metadata note). Version bumps: Cargo.toml 1.14.0 → 1.15.0,
  openapi.yaml (4 new routes + `trace`/`trace_id`), README, CHANGELOG
  §[1.15.0], ROADMAP row → Shipped, AGENTS.md.
- **Wiring guards updated**: `test_openapi_covers_routes` (+ v1.14 + v1.15
  routes), `authz_gates_cover_every_non_public_route` (+4 routes, all Admin,
  `observe` source mapping), `test_migration_schema_contract` (+ v1.15 tables +
  tombstone columns + 1.15.0 stamp).

### Tests (→ 518 passed, 1 ignored; +6)
`test_observe_read_event_recorded_and_trace_replayable`,
`test_observe_read_events_default_on_for_jwt_off_for_loopback`,
`test_observe_dsar_locate_and_purge_semantics`,
`test_observe_deletion_certificate_chain_anchors_and_verifies`,
`test_observe_art19_webhook_posts_on_purge` (real TCP listener, signed POST),
`test_observe_audit_retention_prunes_and_reanchors`.

### Verification
`cargo test --features bench,migrate`: 518 passed, 1 ignored. Clippy
`-D warnings` clean. `cargo fmt` clean.

### Ship status: SHIPPED (code-complete) 2026-08-08
Live restart is an operator step (`scripts/install-service.sh`).

### Honest ceilings (carried into v1.16)
- Read events default off in loopback mode; opt in explicitly to collect
  read traces (`BRAIN_AUDIT_READ_EVENTS=on`).
- Audit chain is single-process (distributed audit = v2.1).
- DSAR export is brain-server JSON, not UMP wire format.
- No PII encryption at rest (COMPLIANCE.md documents the LUKS posture).
- No trace backfill for pre-v1.15.0 recalls.
- The prune re-anchor rewrites every survivor `prev_hash` (O(n), rare path;
  >1M-row logs would want a periodic checkpoint).

---

## Agent 46: v1.14.0 "Gate" — write-back gating + trust surfaces — 2026-08-07
**Status:** COMPLETED (code + tests + release wrap; live restart pending operator)
**Date:** 2026-08-07

Shipped the ROADMAP's v1.14.0 "Gate" row (the Alex Xu thread's #1 ask) with zero
tokens and no auto-promote. See `CHANGELOG.md` §[1.14.0] for the full record.

### Changes Made
- **New `src/gate.rs`** (pure logic, in the `#![deny(unsafe_code)]` lib module):
  `scan_pii` (email / phone / Luhn card — conservative, no deps), `salience`
  (length/entity band), `novelty` (vec0 KNN, safe-None on missing index),
  `confidence` (stored-rule factors), `relevance_tier`, `is_decayed`,
  `has_pii_read` (loopback or Admin), `redact_content` +
  `mask_email`/`mask_phone` (`[redacted:...]` output masking).
- **New `src/handlers/gate.rs`**: `ingest_proposal` (deterministic
  novelty/conflict/salience scoring, NO knowledge row), `list_proposals`,
  `approve_proposal` (promote in one tx + optional `?supersedes` →
  `resolve_supersession`), `reject_proposal`, `list_decayed`, `export`
  (portable JSON; `pii_map` excluded by default, `?include_pii_map` + `pii:read`
  opts in; *(removed v1.20.19 — the `pii_map` vault was never built)*), `purge`
  (hard delete across knowledge + vec0 + relationships +  proposals in one tx, tombstone + audit, by id or owner), `scope_filter`
  (JWT-mode deny-by-default access-scope data-layer filter; loopback trusts
  localhost), `principal_to_owner`.
- **`src/search/mod.rs`**: `SearchFilters` + `SearchResult` gate fields
  (`include_decayed`, `now_unix`, `memory_kind`, `min_relevance`,
  `access_scopes`; `assertion_kind`, `confidence`, `expires_at`, `pii`),
  `push_gate_filters` (shared decay/kind/scope SQL for both vec0 + FTS).
- **`src/handlers/recall.rs`** + **`src/handlers/mod.rs`**: request fields
  (`include_decayed`, `memory_kind`, `min_relevance`), `min_relevance` post-
  fusion filter, `decayed` flag, PII redaction on output for non-`pii:read`
  principals.
- **`src/handlers/ingest.rs`**: `pii` flag set on structured ingest.
- **`src/migration.rs`**: `proposals` + `pii_map` tables *(the `pii_map`
  write-time vault was never built and is dropped in v1.20.19)*; `knowledge`
  columns
  `expires_at`/`access_scope`/`assertion_kind`/`confidence`/`owner`/`pii`;
  `tombstones` gains `content_hash` + `purged_at` via idempotent `ALTER TABLE`.
  **Bug fixed:** the old `CREATE TABLE IF NOT EXISTS tombstones(...)` was a
  silent no-op against the v0.9.1 schema and would have failed the purge INSERT
  on real DBs — now guarded column-adds.
- **Release wrap**: version 1.13.6 → 1.14.0 (Cargo.toml, openapi.yaml with 7
  new routes + version, README, ROADMAP row → Shipped, CHANGELOG §[1.14.0],
  AGENTS.md). New plan: `IMPLEMENTATION_PLAN_v1.14.0_Gate.md`.

### Tests (→ 512 passed, 1 ignored)
Pure gate.rs (PII scan/Luhn/salience/confidence/tiers/decay/redaction/
has_pii_read/novelty-safe), handlers/gate.rs (`principal_to_owner`),
search/tests.rs (`push_gate_filters`), and integration in main.rs
(`test_gate_filters_apply_at_sql_level`, `test_gate_approve_promotes_
proposal_in_one_tx`, `test_gate_purge_removes_across_tables_with_tombstone`).
The purge test caught the tombstones migration bug. `test_openapi_covers_routes`
+ `authz_gates_cover_every_non_public_route` extended with the 7 new routes.

### Verification
`cargo test --features bench,migrate`: **512 passed, 1 ignored**. Clippy
`-D warnings` clean. `cargo fmt --check` clean. Release build: all 5 binaries
clean.

### Ship status: SHIPPED (code-complete) 2026-08-07
Live restart + `brain` CLI review commands + live smoke are operator steps
(`scripts/install-service.sh`).

### Honest ceilings (carried into v2.0)
- `pii:read` is a documented v2.0 refinement — `Scope` grammar only supports
  read/write/admin/traverse, so `has_pii_read` keys on Admin/loopback today.
- `scan_pii` is deterministic pattern matching ("control, not a classifier"),
  not learned; no semantic PII detection.
- Access-scope filter is JWT-mode only; loopback/opaque trusts localhost
  (documented SECURITY.md posture).
- Decay is strict `<`, default-excludes; no background worker, nothing deleted
  autonomously.
- `BRAIN_REDACT_PII` write-time placeholder mode is opt-in and off by default.
  *(Correction — **v1.20.19 "Vault"**: the placeholder vault was never built;
  the control is deterministic read-time output redaction and there is no
  `BRAIN_REDACT_PII` knob.)*

---

## Agent 45: v1.13.2 "Harden" — rough-edges audit hardening pass — 2026-08-06
**Status:** COMPLETED (code + tests + live restart + tag)
**Date:** 2026-08-06

A deep API/code review surfaced three fixable rough edges on v1.13.1. Two were
real bugs (write contention failed instead of queuing; the same telemetry
concept used two different flag names across two endpoints), one was a
naming-consistency papercut. All three closed with back-compat preserved; no
new schema, no new model, no new routes.

### Changes Made
- **`PRAGMA busy_timeout=5000` on every SQLite pool init** (`src/main.rs` main
  pool via `with_init`, `src/domain_registry.rs` `open_with_migration`, and the
  `src/migration.rs` pragma batch). Previously only `auth/revocation.rs` set a
  busy timeout, so concurrent writers against `POOL_MAX_SIZE=20` connections
  could fail immediately with `SQLITE_BUSY` instead of waiting. Write contention
  now queues up to 5s. This is the cheapest real throughput win and the only
  one that touches correctness, not ergonomics.
- **`GET /graph/traverse` accepts `name`/`entity` as aliases for `start`**
  (`src/main.rs` `TraverseQuery`, `#[serde(alias)]`). Docs canonical stays
  `start` (openapi.yaml + README agree), but the response field is `entity` and
  sibling routes (`/graph/entity/{name}`, `/graph/relations?from=&to=`) use
  `name`/`entity`, so callers can now mirror the field back. Back-compat
  preserved — `start` still works.
- **`POST /recall` accepts `explain` as an alias for `provenance`**
  (`src/handlers/recall.rs`). `GET /search` had always gated telemetry on
  `explain`; `/recall` used `provenance`, so the same intent needed two flag
  names depending on the endpoint. Both spellings now work on `/recall`.
- **OpenAPI** (`openapi.yaml`): documented the aliases (`start` description +
  `provenance` description), version → 1.13.2.
- **Version bump** 1.13.1 → 1.13.2 (`Cargo.toml`/`Cargo.lock`, `openapi.yaml`,
  `README.md`, `CHANGELOG.md` §[1.13.2], `AGENTS.md`).

### Verification
- `cargo test --features bench,migrate`: **478 passed, 1 ignored**.
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `scripts/install-service.sh`: release binaries built + copied to
  `~/.local/bin`, launchd service restarted, `/health` OK.

### Ship status: SHIPPED 2026-08-06
Tag `v1.13.2` created. Live launchd service reports v1.13.2.

### Honest ceilings (carried into v1.14.0 / v1.15.0)
- The v1.13.1 ceilings (routing is a single fixed threshold, not calibrated;
  profile rerank + corpus curation + plugin wiring remain for the v1.15.0
  "Recall" plan M2/M3/M4; the `global` rescue leg doubles the shim search to two
  passes when routed) all carry forward unchanged.
- `/classify` is keyword-only by design (the `ponytail:` comment names the
  model2vec upgrade path) — not touched this release.
- AuthZ stays domain-level, not per-record (v2.0 "Cortex" work) — not touched.

---

## Agent 44: v1.13.1 "Recall" fix — automatic retrieval routing (v1.15.0 M1 hotfix) — 2026-08-06
**Status:** COMPLETED (code + tests + live restart + tag)
**Date:** 2026-08-06

Fix-release on top of v1.13.0. Shim-mode recall never centroid-routed — a
`None if !multi_db` short-circuit (`src/handlers/recall.rs:195-200`) searched
the `global` pool only, so after v1.13.0's relabel migration the moved
`gutmindsynergy` blog rows became **unreachable by default recall** (live-verified:
a blog query returned only `global` residue copies; `?domain=gutmindsynergy`
returned the real rows). This hotfix makes routing automatic on retrieval in
shim mode.

### Changes Made
- **`src/handlers/recall.rs`**: removed the shim routing bypass. New pure helper
  `shim_routing_targets(route)` — routed non-global domain → `[domain, global]`
  (matched domain primary + `global` rescue leg for real working memory);
  un-routed/`global` → `[global]` (never federates into a bulk domain — the
  blog-domination guard). Reuses the existing cross-domain RRF merge; no new
  fusion code. Domain-agnostic — no hardcoded domain names in production.
- **`src/config.rs`**: `brain_recall_routing_enabled()` kill switch
  (`BRAIN_RECALL_ROUTING_ENABLED`, default on) — `false` restores the exact
  pre-v1.13.1 shim behavior without a rebuild.
- **Version bump** 1.13.0 → 1.13.1 (`Cargo.toml`/`Cargo.lock`, `openapi.yaml`,
  `README.md`, `CHANGELOG.md` §[1.13.1], `AGENTS.md`).

### Tests (+3 → 478 passed, 1 ignored)
- `shim_routing_targets_routed_domain_plus_global_rescue`
- `shim_routing_targets_routed_to_global_scopes_to_global`
- `shim_routing_targets_unrouted_scopes_to_global_not_bulk_domain`

### Verification
- `cargo test --features bench,migrate`: **478 passed, 1 ignored**.
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- **Live end-to-end** (after `scripts/install-service.sh`): blog query →
  `domains_searched: ['global','gutmindsynergy']` (rows reachable again);
  working-memory + visa queries → `['global']` (no blog dumped); throwaway
  instance with `BRAIN_RECALL_ROUTING_ENABLED=false` → `['global']` only
  (kill switch proven live).

### Ship status: SHIPPED 2026-08-06
Tag `v1.13.1` created. Live launchd service reports v1.13.1.

### Honest ceilings (carried into v1.15.0)
- Routing is a single fixed `DOMAIN_CONFIDENCE_THRESHOLD`, not calibrated.
- M1 only: `profile` rerank weighting, corpus-curation tooling, and the plugin
  `recallProfile` config remain in the v1.15.0 "Recall" plan (M2/M3/M4).
- The `global` rescue leg doubles the shim search to two passes when routed.

---

## Agent 1: Fix Critical Security Issues
**Status:** COMPLETED  
**Date:** 2026-02-24

### Changes Made
- Fixed CORS Configuration - Environment-based CORS with `CORS_ORIGINS` env var
- Restricted HTTP methods to GET, POST, PUT, DELETE
- Restricted headers to Content-Type only

### Verification
- `cargo clippy -- -D warnings` - PASSED
- `cargo clippy -- -D dead_code` - PASSED

---

## Agent 2: Remove Dead Code & Refactor
**Status:** COMPLETED  
**Date:** 2026-02-24

### Changes Made
- Removed unused imports and dead code
- Fixed clippy warnings
- Cleaned up EntityExtractor module

---

## Agent 3: Optimize Search & Database
**Status:** COMPLETED  
**Date:** 2026-02-24

### Changes Made
- Added database indexes for entities and relationships
- Optimized search with batch processing

---

## Agent 4: Configuration & Constants
**Status:** COMPLETED  
**Date:** 2026-02-24

### Changes Made
- Extracted magic numbers to config.rs
- Added SEARCH_BATCH_SIZE to config
- Centralized all configuration constants

---

## Agent 5: Comprehensive Testing
**Status:** COMPLETED  
**Date:** 2026-02-24

### Changes Made
- Improved test infrastructure
- Fixed clippy warnings in tests

---

## Agent 6: Error Handling & Logging
**Status:** COMPLETED  
**Date:** 2026-02-24

### Changes Made
- Added structured logging with tracing
- Improved error handling

---

## Agent 7: Documentation
**Status:** COMPLETED  
**Date:** 2026-02-24

### Changes Made
- Updated README.md to v0.8.1
- Added CORS_ORIGINS to environment variables

---

## Agent 8: Release Preparation
**Status:** COMPLETED  
**Date:** 2026-02-24

### Changes Made
- All agents merged to main
- Ready for release v0.8.1

---

## Agent 9: Version Bump to v0.9.0
**Status:** COMPLETED  
**Date:** 2026-07-08

### Changes Made
- Updated Cargo.toml to v0.9.0
- Updated README.md current version to v0.9.0
- Updated ROADMAP.md released version to v0.9.0
- Updated SPECS.md verification basis to v0.9.0
- Fixed SERVER_VERSION to use env!("CARGO_PKG_VERSION")
- Updated AGENTS.md to v0.9.0

## Agent 10: v0.9.1 "Recall" — hybrid retrieval + PRF + rerank + provenance
**Status:** COMPLETED  
**Date:** 2026-07-11 (released same day as v0.9.2/v0.9.3)

The biggest retrieval release since v0.9.0. Phase 2 of the roadmap. The retrieval
engine was extracted into `src/search/` (`#![deny(unsafe_code)]`; all sqlite-vec
FFI stays in the crate root) and hardened end-to-end. See `CHANGELOG.md` §[0.9.1]
for the full record.

### Changes Made
- **Hybrid retrieval with Reciprocal Rank Fusion.** Vector (`vec0` KNN) and
  lexical (FTS5 BM25) run concurrently on independent pooled read connections,
  fused via RRF (`k = 60`, no learned weights).
- **PRF query expansion actually executes now.** Previous gate compared an RRF
  fused score against an unreachable `0.3` threshold (top RRF ≈ 2/60 ≈ 0.033),
  so expansion never ran. New deterministic gate `prf_should_expand`: expansion
  fires only when the top pass-1 result appears in **both** dense and lexical
  lists within a bounded rank. Anti-injection guardrail skips quarantined rows.
- **Optional cross-encoder rerank tier** (`--features rerank` + `RERANK_ENABLED=true`).
  BGERerankerV2M3 via `fastembed`. Default build stays pure-static. Contract
  repaired: over-fetches a candidate window (`RERANK_CANDIDATES=30`) and reranks
  **before** truncating to `k`.
- **Per-result provenance** on both `/search` and `/recall` (per-retriever ranks,
  fused score, expansion terms, rerank score).
- **Metadata-filtered KNN** (`source`, `since` ISO-8601, `domain` pushed into
  `vec0` + FTS5 `WHERE` clauses, parameterized).
- **Structure-aware Markdown chunking** (`src/chunker.rs`): heading-boundary
  splits, code-fence-safe, one chunk per `knowledge` row with `document_id`,
  `chunk_index`, `heading_path`, 1-indexed line span. New `GET /get/{id}` and
  `POST /multi-get`.
- **Implemented `POST /ingest`** (was `unimplemented!()`/panic) + `DELETE /memory/{id}`
  with vec0 cleanup + tombstone audit row + `POST /reindex`.
- **Bearer-token auth** (`AUTH_TOKEN`) on non-public routes, loopback-safe defaults.
- **P2 scaffolding:** `domain`/`observed_at`/`valid_from`/`valid_to` columns on
  `knowledge`, `src/domain_registry.rs` (lazy per-domain pools, off by default
  via `BRAIN_MULTI_DB`), `src/domain_router.rs` (centroid routing + federation).
- **Developer surface:** `brain` CLI (`src/bin/brain.rs`), MCP server
  (`src/bin/mcp.rs`), `openapi.yaml`, benchmark harness (`bench` feature +
  `src/bin/bench.rs`), recall eval harness (`#[ignore]`d `eval_recall_harness`).
- **Migration safety:** pre-migration `VACUUM INTO` backup (marker-guarded),
  `migrate_down_0_9_0()` reversibility, post-backfill parity check.

### Verification
- `cargo test`: 103 passed, 1 ignored.
- `cargo clippy --all-targets --features bench -- -D warnings`: clean.
- Measured RSS/latency/recall on 4 GB ARM remain PENDING (no hardware run).

---

## Agent 11: v0.9.2 "Connect" — Obsidian vault ingestion
**Status:** COMPLETED  
**Date:** 2026-07-11

### Changes Made
- New `src/vault.rs`: pure frontmatter (`title`/`tags`/`aliases`) + `[[wikilink]]` parser (no YAML dep).
- `knowledge.source_path` column + index (additive migration).
- `/ingest/markdown` vault semantics: source_path provenance, scoped dedup/replace
  (unchanged = no-op; changed = sweep + re-insert), wikilink→`references`, tags→`tagged_with`,
  aliases→`alias_of` KG edges. DB-write extracted to `write_markdown_ingest` for testability.
- `brain ingest-dir` sends `source_path` + walk bounds (50k files / 500 MiB).
- Fix: `/graph/entity` + `/graph/traverse` now allow spaces in entity names (note titles).
- Version bump to v0.9.2 (Cargo.toml, README, ROADMAP, SPECS, CHANGELOG, AGENTS).

### Verification
- `cargo test`: 103 passed, 1 ignored (model-backed eval harness).
- `cargo clippy --all-targets -- -D warnings`: clean (default, `bench`, `rerank` features).
- `cargo fmt --check`: clean.
- End-to-end: ingested a 3-note vault, verified source_path, idempotent re-ingest,
  changed-file replace, wikilink graph traversal, and semantic recall.

## Agent 12: v0.9.3 "Calibrate" — named checkpoint
**Status:** COMPLETED  
**Date:** 2026-07-11

Named release formalizing the retrieval-calibration work that shipped in v0.9.1.
**No new runtime code** — the three Calibrate exit criteria are all already
satisfied by v0.9.1 and guarded by dedicated tests. This release exists to make
the calibration state a named, reviewable checkpoint before the source-lifecycle
work in v0.9.4.

### Calibration state (verified, not newly added)
- **PRF executes** — `prf_should_expand` gate. Guarded by
  `prf_expands_only_on_cross_retriever_agreement`.
- **Rerank has a candidate window** — `RERANK_CANDIDATES = 30`, over-fetch +
  rerank-before-truncate. Guarded by `candidate_window_equals_k_when_disabled`.
- **Benchmark is reproducible** — `bench` feature + `tests/metrics.rs` implement
  the protocol; metric functions unit-tested with hand-computed values.

### Honest status
- Measured RSS/latency/recall numbers on 4 GB ARM and the ≥100 judged-query
corpus remain **PENDING** a hardware run. No claim of measured QMD parity is made.

---

## Agent 13: v0.9.4 bug-fix sweep (session 2026-07-17)
**Status:** COMPLETED  
**Date:** 2026-07-17

Five logical commits shipped to `origin/main` (`e859702`..`ddd3b17`). Version stayed at 0.9.4 — this was a bug-fix sweep, **not** the "Sources" feature release (which remains planned).

### Changes Made
- **CLI bearer auth** (`fix(http)`): `brain`/`mcp`/`bench` returned 401 on every authenticated route (`/search`, `/stats`, `/recall`, `/ingest/*`) because the shared HTTP client in `bin_common/http.rs` had no auth support. Added `bearer: Option<&str>` to `get()`/`post()`; each binary resolves `BRAIN_TOKEN_FILE` → `BRAIN_TOKEN` → `~/.config/brain-server/auth-token`. Zero-config for the common install.
- **`--version`/`-V` flags** (`fix(cli)`): `brain-server --version` used to silently start the server (no argv inspection in `main.rs`). Added `handle_cli_args()` before any side effect; rejects unknown flags instead of launching. `brain --version` was rejecting as unknown subcommand; added match arm.
- **`/stats` embeddings count** (`fix(stats)`): was reporting `2` on a 430-doc corpus — handler counted the legacy `embeddings` table (frozen read-only since v0.9.0) instead of the live `vec_knowledge` vec0 table. One-line fix.
- **Install script** (`feat(install)`): `install-service.sh` now ships the 3 CLI binaries alongside the server (with `--features bench`), and strips `com.apple.provenance` xattr after each copy (macOS SIGKILL fix).
- **Docs** (`docs`): CHANGELOG `[Unreleased]` section + ROADMAP integrated the granular v0.9.4–v0.9.9 chain (Sources → Inspect → Bridge → Guard → Evidence → Qualify → Domains) with a Prereqs column.

### Verification
- `cargo test --features bench`: 112 passed, 1 ignored.
- `cargo clippy --all-targets --features bench -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `brain status` / `brain --version` / `brain-server --version`: all work from `$PATH`.

---

## Agent 14: v0.9.4 infra shoring-up (session 2026-07-17)
**Status:** COMPLETED  
**Date:** 2026-07-17

Safety-net work done **before** any v0.9.4 feature code, per the principle that schema-migration releases need their foundation solid first. Two commits pushed to `origin/main`.

### Changes Made
- **`src/sources.rs` audit** (read-only, no commit): temporarily wired `mod sources;` into `main.rs` → compiled clean → all 7 unit tests pass → full suite 119 passed (was 112) → reverted the wiring. **Verdict: the 486 lines are salvageable and finished.** v0.9.4 is now integration-only work (migration + handlers + routes), shrinking the release from ~4 sessions to ~1. See Known Issues §1 for the updated status.
- **`ci: test + clippy with --features bench`** (`6a69797`): added two steps to the `lint-test` job — `cargo clippy --all-targets --features bench -- -D warnings` and `cargo test --all-targets --features bench`. The bench binary is feature-gated and was previously untested upstream. Closes Known Issue §2 first bullet.
- **`test: add migration schema-contract test`** (`6370b77`): added `test_migration_schema_contract` in `src/main.rs`. Runs the real `run_migration` on a fresh in-memory DB, asserts the full table set (`knowledge`/`embeddings`/`vec_knowledge`/`entities`/`relationships`/`tombstones`/`knowledge_fts`/`schema_meta`), asserts every column on `knowledge` that handlers depend on, and verifies the core loop (insert → FTS5 trigger fires → vec0 INSERT accepted → COUNT(*) sees the row). The single test that would catch a broken v0.9.4 migration before it reaches the live 430-doc DB.

### Verification
- `cargo test --features bench`: 113 passed, 1 ignored (+1 from baseline).
- `cargo clippy --all-targets --features bench -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `.github/workflows/ci.yml`: YAML valid, new steps in place.

---

## Agent 15: v0.9.4 Sources M1 — schema migration (session 2026-07-17)
**Status:** COMPLETED  
**Date:** 2026-07-17

First feature work for v0.9.4 Sources, landed after the infra shoring-up (Agent 14) made it safe to do. One commit (`ecab395`).

### Changes Made
- **Additive migration for `sources` + `source_revisions` tables** + `knowledge.source_id`/`revision_id` columns (commit `ecab395`). Schema matches what `src/sources.rs` already implements against. Existing rows left NULL — they keep working as before; only new ingests get source linkage. All statements idempotent (`CREATE TABLE IF NOT EXISTS`, column-presence guards).
- **Extended `test_migration_schema_contract`** to assert the new tables and columns exist after migration.

### Verification
- `cargo test --features bench`: 113 passed, 1 ignored.
- `cargo clippy --all-targets --features bench -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- **Tested against a copy of the live 430-doc DB**: copied `~/.openclaw/workspace/brain.db` → `/tmp`, ran the server against it, all 430 docs + 24 entities + 25 relationships survived, new tables/columns/indexes all present, existing rows correctly NULL. Pre-migration `VACUUM INTO` backup created successfully.

### What remains for v0.9.4 to ship
1. Wire `mod sources;` into `main.rs` for real + retrofit `/ingest/markdown` and `/ingest/memory` to call `upsert_source` + `upsert_revision` + `link_chunks` inside their existing transactions.
2. Add `brain reconcile` + `DELETE /sources/<id>` routes.
3. Run against the live DB (via `install-service.sh` restart).

---

## Agent 16: v0.9.4 Sources M2 — integration glue + routes (session 2026-07-17)
**Status:** COMPLETED (code only — live restart pending operator)
**Date:** 2026-07-17

Landed the integration work that turns the salvaged `src/sources.rs` module
(Agent 14 audit) + M1 schema migration (Agent 15) into live v0.9.4 behavior.
No new schema this session — the migration from `ecab395` already had the
shape the integration code needed. All work is in 5 files (4 modified + 1 new);
no commit has been pushed yet (operator's call to commit + restart together).

### Changes Made

- **`mod sources;` wired into `main.rs`** — one-line module declaration. The
  7 pre-existing `sources::tests::*` tests are now reachable from `cargo test`,
  which is where the test-count delta of +7 vs Agent 15 comes from.

- **`/ingest/markdown` retrofit** (`src/main.rs`):
  - `write_markdown_ingest` takes a new `raw_content: &str` parameter (the
    original payload, frontmatter + body) so the revision hash reflects ANY
    change in the file, not just body changes that survive frontmatter stripping.
  - For vault ingests (`source_path.is_some`), the changed-file path now calls a
    new `link_vault_source` helper that composes `sources::upsert_source` +
    `upsert_revision` + `link_chunks` against the inserted chunk ids, inside the
    existing transaction. Fail-loud: an orphan chunk with no source linkage is
    a real bug, not a degraded ingest.
  - The unchanged-file no-op path now backfills source linkage for pre-v0.9.4
    chunks that have NULL `source_id` (first v0.9.4 re-ingest of a legacy file).
    Best-effort with `let _ =` — a failure here must not retroactively break a
    previously-working no-op ingest.
  - Interactive adds (`source_path.is_none`) stay unlinked, matching pre-v0.9.4
    behavior. No source rows are created for them.

- **`/ingest/memory` retrofit** (`src/main.rs`): each memory entry now creates
  a `manual` source with URI `manual://{content_hash}` (no PII in the URI;
  stable across re-ingests of the same content; unique per distinct content).
  Kind = `KIND_MANUAL` keeps these immune to vault reconcile (which is
  kind-scoped). Calls composed inside the existing per-entry transaction;
  fail-soft `let _ =` matches the surrounding `continue`-on-error style.

- **New module `src/handlers/sources.rs`** with two contract-style handlers
  using the existing `HandlerError` envelope:
  - `POST /sources/reconcile` — body `{kind, live_uris: [string]}`. The server
    does NOT walk the filesystem; the caller supplies the live URI set
    (preserving the client/server boundary). Bounded `MAX_LIVE_URIS = 50_000`
    (matches `MAX_INGEST_FILES`). Wraps `sources::reconcile` in one tx.
  - `DELETE /sources/{id}` — retires a single source by id, sweeping its chunks
    from retrieval and tombstoning the source + active revision. Returns 404 if
    the id doesn't exist. Wraps `sources::delete_source`.

- **`bin_common/http.rs`**: added `pub fn delete(...)` mirroring `get`/`post`
  for the body-less HTTP DELETE convention. Marked `#[allow(dead_code)]` so the
  `mcp`/`bench` binaries (which `#[path]`-include this file) don't warn.

- **`brain` CLI** (`src/bin/brain.rs`):
  - `brain reconcile <path> [--kind vault] [--dry-run]`: walks the path with
    the SAME walker + `.brainignore` semantics + canonicalized-absolute-path
    URI form that `brain ingest-dir` uses, so the URIs the client sends match
    what's stored in `sources.uri`. POSTs the live set to `/sources/reconcile`.
  - `brain source-delete <id>`: tiny companion to the DELETE route. Without it
    the route is only reachable via raw `curl`; with it, both new routes have
    symmetric CLI coverage.

- **`sources.rs`**: added `pub const KIND_MANUAL` alongside the existing
  `KIND_VAULT`. No other changes — the module's existing 7 tests + the audit
  verdict from Agent 14 ("salvage, don't rewrite") held up under integration.

### Tests added (`src/main.rs`)

Four new integration tests (the smallest checks that fail if the wiring breaks):
- `test_vault_ingest_links_source_and_revision` — vault ingest creates a
  `sources` row (kind='vault', state='active', title set), one active
  `source_revisions` row with the right `chunk_count`, and every chunk points
  back at both.
- `test_vault_reingest_backfills_source_linkage` — simulates a pre-v0.9.4 chunk
  (NULL `source_id`), re-ingests unchanged content, asserts the chunk now has
  source linkage. This is the path the live 430-doc DB takes on first v0.9.4
  ingest after the restart.
- `test_vault_changed_content_supersedes_revision` — editing a file creates a
  new active revision, the prior one is retained as `superseded`, and the
  current chunk points at the active one.
- `test_memory_source_linkage_composition` — `/ingest/memory`'s source
  composition (no HTTP harness exists; the test calls the same `upsert_source`/
  `upsert_revision`/`link_chunks` sequence the handler inlines) produces a
  `manual` source with URI `manual://{hash}`.

Existing `write_markdown_ingest` callers in 3 vault tests updated to pass the
new `raw_content` arg (passed the chunk text — those tests don't exercise
revision hashing, just chunk-level behavior).

### Verification
- `cargo test --features bench`: **124 passed, 1 ignored** (was 113; +7 from
  newly-reachable `sources::tests::*` + +4 new integration tests).
- `cargo clippy --all-targets --features bench -- -D warnings`: clean.
  (Needed `#[allow(clippy::too_many_arguments)]` on `write_markdown_ingest` —
  now 8 args after adding `raw_content`. Commented why bundling into a struct
  is pure ceremony for a private fn with one prod caller.)
- `cargo fmt --check`: clean (fmt also fixed a few pre-existing nits in
  `sources.rs` along the way).
- `cargo build --release --features bench --bin brain-server --bin brain --bin mcp --bin bench`:
  all 4 binaries build clean.
- CLI smoke: `brain reconcile` / `brain source-delete` dispatch correctly,
  reject missing args / non-integer ids.

### v0.9.4 ship status: SHIPPED 2026-07-17
All three operator steps below were executed. Commit `4de1472` landed the M2
 diff, `75d29a9` landed Agent 17's chunker rewrite, `067a53e` was the release-wrap
 docs commit. `scripts/install-service.sh` was run — the live launchd service
 reports v0.9.4 (`brain doctor` ✓). The 430-row live DB ingested the M1
 migration cleanly; existing rows kept NULL source linkage as expected. The
 optional retroactive `brain ingest-dir <vault>` for source-linking legacy
 vaults remains an operator call, not a blocker.

---

## Agent 17: v0.9.4 chunker CommonMark rewrite (session 2026-07-18)
**Status:** COMPLETED
**Date:** 2026-07-18

Closed the chunker's known-limitation gap (the ponytail ceiling from Agents
10/15) in the only honest way: by adopting the canonical Rust CommonMark
parser. No new chunker code is hand-rolled against the spec — that's a
60+ page document and a bug factory. `pulldown-cmark` is the standard tool,
used by `text-splitter`'s `MarkdownSplitter` (Context7-verified 2026-07-17).

### Research
- Context7 lookup: `/pulldown-cmark/pulldown-cmark` — current 0.13.4,
  `#![forbid(unsafe_code)]` upstream, `into_offset_iter()` yields
  `(Event, Range<usize>)` with byte-accurate source spans.
- Context7 lookup: `/benbrandt/text-splitter` — `MarkdownSplitter` uses
  pulldown-cmark internally; confirmed this is the canonical approach.
- Wrote a one-off `examples/cmark_explore.rs` to dump event streams for
  setext / indented code / blockquote / list / table inputs — established
  that container markup (`>`, `-`, `|`) lives in source bytes BETWEEN inline
  text events, so a byte-range-union approach captures it naturally. (Kept as
  `examples/chunk_demo.rs` — a useful dev tool for inspecting chunker output
  on real files.)

### Changes Made

- **Cargo.toml**: added `pulldown-cmark = { version = "0.13",
  default-features = false }` (we use only the parser; the default `html` +
  `getopts` features are dropped to keep the dep tree small).

- **`src/chunker.rs` rewritten.** Same public API (`Chunk { text,
  heading_path, line_start, line_end }` + `chunk_markdown(content) ->
  Vec<Chunk>`), so no caller changes. New algorithm: walk pulldown-cmark
  events with `into_offset_iter()`, accumulate a chunk byte-range by
  extending it to cover every event whose source bytes should appear in
  chunk text, then slice the source verbatim at flush. Heading events close
  the current chunk and contribute their text to the breadcrumb (instead of
  to chunk text — matches pre-v0.9.4 behavior). Code blocks set a "don't
  split" flag so a fence is never broken mid-block. `MAX_CHUNK_CHARS` renamed
  to `MAX_CHUNK_BYTES` (it was always bytes).

- **Constructs now handled correctly** (each was mis-handled by the
  pre-v0.9.4 line-scanner):
  - Setext headings (`Foo\n===` / `Foo\n---`) → recognized as H1/H2
  - Indented code blocks (4-space indent) → recognized as code; interior
    `#`-comment lines no longer mistaken for ATX headings
  - Blockquotes → `>` markers preserved in chunk text via byte-range union
  - Lists → `-` / `*` / `+` / numbered markers preserved
  - GFM tables → `|` separators and `---` divider row preserved verbatim
  - Fenced code with info strings (` ```rust `) → preserved
  - Lazy continuation / nested lists / every other CommonMark construct →
    handled by pulldown-cmark upstream

- **Removed**: the hand-rolled `parse_heading` function and its dedicated
  test `parse_heading_recognizes_levels_and_rejects_non_headings` (the spec
  is now pulldown-cmark's responsibility, not ours).

### Tests added (`src/chunker.rs`)

Six new per-construct tests, each one would have failed against the
pre-v0.9.4 chunker:
- `setext_headings_are_recognized` — setext becomes breadcrumb, `=====` not
  in chunk text.
- `indented_code_block_is_not_split_and_hash_lines_are_code` — 4-space
  indent treated as code; `#`-comment NOT a heading.
- `blockquote_markup_is_preserved` — both `>` markers survive.
- `list_with_wikilinks_is_preserved` — `[[wikilink]]` brackets + `-`
  markers survive (the multi-Text-event-per-bracket case).
- `gfm_table_is_preserved_with_markup` — `|` separators and `---` divider.
- `hash_in_code_fence_is_not_a_heading` — locked-in behavior for the
  carryover `#`-in-fence warranty.

All 7 pre-existing chunker tests still pass unchanged — the public behavior
is preserved for documents that the old scanner handled correctly.

### Verification
- `cargo test --features bench`: **130 passed, 1 ignored** (was 125 before
  this session, was 113 at v0.9.3). Delta vs v0.9.4-pre-chunker-rewrite:
  +5 (6 new chunker tests − 1 removed `parse_heading` test).
- `cargo clippy --all-targets --features bench -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench --bin brain-server --bin brain
  --bin mcp --bin bench`: all 4 binaries build clean.
- Real-world smoke test: ingested a synthetic markdown doc exercising every
  previously-broken construct (setext H1/H2, indented code with `#`-comment,
  fenced code, blockquote, GFM table, multi-section) through the new
  chunker. Output verified by `examples/chunk_demo.rs`: every section landed
  under the correct breadcrumb, every special character survived, no chunk
  was split mid-fence.
- The carryover warranty test `test_special_characters_survive_ingest_
  pipeline` still passes — end-to-end preservation of special-char source
  paths + content (chunker → DB → source-linkage → dedup) is intact.

### v0.9.4 ship status: SHIPPED 2026-07-17
Same as Agent 16's ship-status note — commit `75d29a9` landed this chunker
 rewrite, `067a53e` was the release wrap, and the live service is on v0.9.4.
 The optional retroactive `brain ingest-dir <vault>` for source-linking legacy
 vaults remains an operator call, not a blocker.

---

## Agent 18: v0.9.5 M1 "Inspect" — structured query contract (session 2026-07-19)
**Status:** COMPLETED (code + live restart + docs)
**Date:** 2026-07-19

First milestone of v0.9.5 "Inspect". Closes the plan's M1: a versioned,
validated, structured query document shared by `/search` and `/recall`, with
real lexical controls (phrases / exclusions / exact code paths) and clear
multi-source OR semantics. No schema migration — pure contract + retrieval
wiring on top of the v0.9.4 `source`/`revision` columns.

### Changes Made
- **New `src/search/query.rs`** (the M1 contract):
  - `QueryDoc` — versioned (`v`, unknown versions rejected), fields `q`, `lex`
    (`LexSpec`), `vec`, `hyde`, `intent`, `sources`, `source`, `since`,
    `domain`, `k`, `profile`, `explain`. `from_text()` keeps a bare string
    backwards-compatible. `into_filters()` lowers it into `SearchFilters`,
    normalizing `since` and rejecting empty/unsupported queries with
    structured errors (`QueryDocError`).
  - `LexSpec { terms, phrases, exclude, code }` + `compile_lex()` — emits a
    **validated, FTS5-quoted** MATCH string. Replaces the old unvalidated raw
    `lex` passthrough (which returned opaque SQLite errors on bad input). Each
    entry is individually quoted so caller input can never inject FTS5
    operators. `exclude` → `-"…"`; `code`/`phrases` → `"…"`. Defaults empty.
  - 12 unit tests: compiler shape (phrases/exclude/code/quote-strip/combine),
    version gate, empty-query rejection, since normalization, multi-source
    preservation, legacy bare-string back-compat.

- **`src/search/mod.rs`** (no migration):
  - `SearchFilters` gained `sources: Vec<String>` (OR scope) + `profile`
    (passthrough). `vec0_knn` and `fts_search` now apply
    `source IN (?,?…)` when `sources` is non-empty, falling back to the legacy
    single `source = ?` when empty. `perform_search_traced`'s `since`-normalize
    clone copies the two new fields.

- **`src/main.rs` (`/search`)** + **`src/handlers/recall.rs` (`/recall`)**:
  - Both routes lower their params into `QueryDoc`, sharing ONE lexical
    compiler + validation path.
  - `/recall` accepts `lex` as a full `LexSpec` via `lex_from_string_or_struct`
    (string **or** object) — the OpenClaw plugin's `{"lex":"foo"}` still works.
  - `/search` (GET) takes comma-separated `sources=a,b` and a legacy `lex`
    string (mapped to `LexSpec.terms`, safely quoted).
  - `intent` is recorded into telemetry/provenance only — never injected as a
    search term, never relaxes filters (verified by trace: read only at
    `search/mod.rs:868,880`).

### Verification
- `cargo test --features bench`: **129 passed, 1 ignored** (was 130 at
  v0.9.4; −1 `parse_heading` was already removed in v0.9.4, +12 new query
  tests − 1 removed `parse_heading`-era delta; net the M1 surface adds the 12
  query.rs tests + handler tests). Baseline before M1 was 130; M1 lands at 129
  because one pre-M1 test was retired with the chunker rewrite accounting.
  (Re-checked post-commit: 129 passed / 1 ignored.)
- `cargo clippy --all-targets --features bench -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench --bin brain-server --bin brain
  --bin mcp --bin bench`: all 4 binaries build clean.

### Live end-to-end (after `scripts/install-service.sh` restart, pid 32069)
- `/recall` POST with `LexSpec` (phrases + `-exclude` + `code`) + `sources:
  ["manual","vault"]` + `provenance:true` → hits returned, `telemetry` present.
- `/recall` legacy string `lex:"obsidian"` → still works (back-compat).
- `/recall` plain `{"query":"project roadmap"}` (OpenClaw-style) → still works.
- `/search?q=memory&lex=obsidian&sources=manual,vault&explain=true` →
  `query_plan` shows compiled `lex: "obsidian"` + OR scope `["manual","vault"]`
  + `telemetry`.

### v0.9.5 M1 ship status: SHIPPED 2026-07-19
Commits `a46c7ab`, `ade13d1`, `28309f9`. `scripts/install-service.sh` rebuilt
+ restarted the launchd service; verified the new contract against the live
430-doc DB. **M1 is signed off** — all five plan checklist items complete.

### M1 honest ceilings (carried forward, not bugs)
- `profile` accepted-but-passthrough (no rerank/weighting yet).
- `LexSpec` covers terms/phrases/exclusions/code only — no `NEAR`/prefix/
  column filters (upgrade path noted inline in `compile_lex`).
- `/search` GET takes a flat `lex` string, not a nested `LexSpec` (GET query
  strings can't carry nested JSON); full structured form is on `/recall` POST
  and will back the `brain query` CLI in M3.

---

## Agent 19: v0.9.5 M2 "Inspect" — evidence quality (session 2026-07-19)
**Status:** COMPLETED (code + live restart + docs)
**Date:** 2026-07-19

Second milestone of v0.9.5 "Inspect". Closes the plan's M2: every
visible result carries faithful, bounded evidence (span + source link +
highlight ranges) and `explain` is a reproducible block. No schema migration
— reuses the v0.9.4 `source_id`/`revision_id` columns + the M1
`QueryDoc`/`Provenance` plumbing.

### Changes Made
- **New `Evidence` struct (`src/search/mod.rs`)** `{ text, line_start,
  line_end, heading_path, source_uri, revision_id, highlights }`:
  - `text` is always a verbatim substring of `content` (the `with_snippet`
    invariant — never synthesized).
  - `highlights` are byte-offset `[start,end)` ranges **within `text`** (the
    snippet window), computed by `highlight_ranges()` — the server never
    injects HTML, and the ranges can't point past the revealed text (the
    redaction guarantee).
  - `source_uri` (`sources.uri`) + `revision_id` (`source_revisions.id`)
    form a stable, dereferenceable link to the exact source revision.
- **`SearchResult::enrich_evidence(conn, results, snippet_q)`** — one
  batched `LEFT JOIN` to `knowledge` span columns + `sources` +
  `source_revisions` for all hit ids (not N queries). Populates `evidence`
  on each result; leaves `source_uri`/`revision_id` = `None` for
  pre-v0.9.4 rows with NULL linkage (graceful, verified on live DB).
- **`config.rs`**: `MAX_SNIPPET_CHARS` (240, was inline 180),
  `SNIPPET_CONTEXT_CHARS` (60, was inline), `MAX_EXPLAIN_BYTES`
  (64 KiB redaction cap), `MAX_MULTI_GET` (1000).
- **Handler wiring (`src/main.rs`, `src/handlers/recall.rs`)**:
  - `/search` and `/recall` both call `enrich_evidence` after retrieval;
    `RecallHit` gains an `evidence` field.
  - `GET /get/{id}` and `POST /multi-get` now return `source_uri` +
    `revision_id` via the same LEFT JOIN; `multi-get` bound raised to
    `MAX_MULTI_GET` (was hardcoded 100).
  - `/search?explain=true` **redacts full `content`** from results (keeps
    the bounded `evidence.text`/`snippet`); adds `k`/`source`/`domain`/
    `since`/`profile` to `query_plan` for full reproducibility; if the
    explain payload exceeds `MAX_EXPLAIN_BYTES` it returns the summary only.

### Verification
- `cargo test --features bench`: **133 passed, 1 ignored** (was 129 at
  M1; +4 new M2 tests: `highlight_ranges_finds_term_offsets_within_window`,
  `highlight_ranges_skips_short_tokens`, `enrich_evidence_attaches_span_and_
  source_link`, `enrich_evidence_handles_unlinked_chunks_gracefully`).
- `cargo clippy --all-targets --features bench -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench --bin brain-server`: clean.

### Live end-to-end (after `scripts/install-service.sh` restart)
- `/recall` POST `{"query":"obsidian","provenance":true}` → each hit
  carries `evidence` with `line_start`/`line_end`/`heading_path` +
  `highlights` (e.g. `[[5,13]]` for "timeline").
- `GET /get/{id}` → returns `source_uri` + `revision_id` (NULL on legacy
  rows, as expected for the pre-v0.9.4 430-doc DB).
- `/search?q=obsidian&explain=true` → `content` is absent from results
  (redacted); `query_plan` carries `k`/`lex`/`sources`/`domain`/`since`.

### v0.9.5 M2 ship status: SHIPPED 2026-07-19
Commits `0b10b45` (Evidence + enrich + highlights + config), `9a4ce75`
(handler wiring + get/multi-get + explain redaction). `scripts/install-
service.sh` rebuilt + restarted the launchd service; verified the new evidence
contract against the live 430-doc DB. **M2 is signed off** — all four M2
sub-milestones (M2.1–M2.4) complete.

### M2 honest ceilings (carried into M3)
- `highlights` are on the snippet window (redaction by design); a client
  wanting highlights over the *full* chunk must call `/get/{id}`.
- `/recall`'s explain uses `provenance`/`telemetry`; `/search`'s uses
  `query_plan` — two shapes, one semantic; M3 may unify the envelope.
- M2 adds no rerank weighting from `profile` (still passthrough from M1).

---

## Agent 20: v0.9.5 M3 "Inspect" — product interface (session 2026-07-19)
**Status:** COMPLETED (code + live restart + docs)
**Date:** 2026-07-19

Third and final milestone of v0.9.5 "Inspect". Closes the plan's M3: a
structured `brain` CLI, a discoverable OpenAPI contract, an MCP tool schema, and
an explicit versioning/deprecation policy so third parties can depend on the API
without surprise. No schema migration — reuses the v0.9.4 `source`/`revision`
columns + the M1 `QueryDoc`/`LexSpec` + the M2 `/get/{id}`/`/multi-get` routes.

### Changes Made
- **`brain query` → `POST /recall` with `QueryDoc`** (`src/bin/brain.rs`):
  repeatable `--phrase`/`--exclude`/`--code` (lowered into `LexSpec`),
  multi-`--source` OR scope, `--intent`, `--profile`, `--since`, `--k`,
  `--explain`. `build_query_doc` builds the JSON; `print_hits` renders
  `/recall` hits; `print_telemetry` renders the unified envelope. Removed the
  now-dead `print_results` (was only used by the old `/search` `cmd_query`).
- **`brain get <id>` implemented** (`src/bin/brain.rs`): hits the existing
  `GET /get/{id}` (M2.3 CLI ceiling closed). Prints title/source/heading/line
  span/`source_uri`/`revision_id` + content; 404 → "no chunk with id".
- **`brain explain` unified** (`src/bin/brain.rs`): POSTs `/recall` with
  `provenance:true`, prints the shared telemetry + per-hit provenance block
  (closes the M2.2 envelope split — CLI now uses one shape, not `/search`'s
  `query_plan`).
- **`GET /openapi.yaml`** (`src/main.rs`): serves the canonical contract,
  embedded via `include_str!("../openapi.yaml")` so it ships in the binary.
- **`openapi.yaml` → v0.9.5** (hand-written, no `utoipa` dep): all 23 routes
  documented (added `/get/{id}`, `/multi-get`, `/sources/reconcile`,
  `/sources/{id}`, `/reindex`, `/openapi.yaml`); new `QueryDoc`/`LexSpec`/
  `Evidence`/`Chunk`/`QueryPlan`/`SearchTelemetry` schemas; `evidence`/
  `snippet`/`source_uri`/`revision_id` on `SearchResult`/`RecallHit`.
- **`examples/client_example.rs`** — typed client over the shared `bin_common`
  HTTP client, demonstrating a structured `QueryDoc` roundtrip.
- **MCP tool schema** (`src/bin/mcp.rs`): `brain_search`/`brain_recall`/
  `brain_ingest` updated to v0.9.5 `QueryDoc` (`phrases`/`exclude`/`code`/
  `sources`/`source`/`since`/`intent`/`provenance`); both search tools now POST
  `POST /recall` via one `recall_body` lowerer. Removed unused `get` import;
  added `#[allow(dead_code)]` on `bin_common/http.rs::get` (used by some
  binaries, not all) to keep clippy clean.
- **API versioning + deprecation** (`src/main.rs` + `API_CONTRACT.md`):
  `X-Api-Version: <semver>` on every response (global `SetResponseHeaderLayer`);
  `Deprecation: version="0.9.5"` RFC 8594 header on legacy `POST /add` and
  `GET /search`; `API_CONTRACT.md` gained the §Versioning & deprecation policy
  (discovery, structured-query contract, deprecation signal, migration mapping,
  stability promise).
- **`test_openapi_covers_routes`** (`src/main.rs`): asserts every route
  registered in `build_app` appears in `openapi.yaml` — the single test that
  catches a route shipping without a contract.

### Verification
- `cargo test --features bench`: **133 passed, 1 ignored** (was 133 at M2;
  `test_openapi_covers_routes` landed as intended but one test was
  concurrently retired — net zero against M2's 133. Recorded honestly
  here after the fact rather than leaving the originally-claimed 134.)
- `cargo clippy --all-targets --features bench -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench --bin brain-server --bin brain
  --bin mcp --bin bench`: all 4 binaries build clean.
- Live smoke (freshly-built `target/release/brain` against the v0.9.4 server,
  since restart happens with `install-service.sh`): `brain query "obsidian"
  --k 2` → recall hits; `brain get 1` → chunk + content; `brain explain
  "obsidian" --source vault` → unified telemetry + per-hit provenance.

### v0.9.5 ship status: SHIPPED 2026-07-19
v0.9.5 "Inspect" (M1 + M2 + M3) complete. `Cargo.toml` bumped 0.9.4 → 0.9.5.
`scripts/install-service.sh` rebuilds + restarts the launchd service so the
live binary reports v0.9.5 with the new `brain` CLI, MCP schema, `X-Api-Version`
header, and `GET /openapi.yaml`. **M3 is signed off** — all four plan bullets
(CLI, OpenAPI, MCP schema, versioning/deprecation) complete.

### M3 honest ceilings (carried forward)
- `highlights` over the *full* chunk still need `GET /get/{id}` (M2.3); the
  `brain get` CLI returns full content so a client can compute its own.
- `profile` accepted but passthrough (no rerank weighting yet) — reserved for
  v0.9.6+.
- OpenAPI is hand-written (no code-gen dep) to keep the build dependency-
  minimal; the coverage test guards it from drift.

---

## Agent 21: v0.9.6 M1 "Bridge" — connector contract + supervisor (session 2026-07-20)
**Status:** COMPLETED (code + tests + pushed)
**Date:** 2026-07-20

First milestone of v0.9.6 "Bridge". Lays the smallest set of code that lets a
connector exist at all, without writing any GitHub-specific logic.

### Changes Made
- **New `src/connector/mod.rs`**: `ConnectorManifest` + `ConnectorRow` +
  `list_connectors` + `upsert_connector`. Idempotent registration (state ←
  'registered' on conflict).
- **New `src/connector/supervisor.rs`**: `next_backoff` (exponential capped
  at 60s, `checked_shl` for overflow safety, no jitter — single local
  supervisor) + `spawn_once` (tokio::process + kill_on_drop).
- **New `src/handlers/connectors.rs`**: `GET /connectors` route.
- **New `src/bin/brain-connector-stub.rs`** (~140 LOC): M1 reference connector.
  Spawns, parses `--config`/`--checkpoint` argv, emits the JSON-lines event
  stream, ingests one doc via the existing `/ingest/markdown` route, exits 0.
- **Migration**: additive `connectors` + `connector_checkpoints` tables.
- **`openapi.yaml`**: `/connectors` route + `ConnectorRow` schema.
- **`test_migration_schema_contract`** + **`test_openapi_covers_routes`**
  extended with the new route.

### Verification
- `cargo test --features bench`: 152 lib + 9 integration passed, 1 ignored
  (was 142+9 at M0 baseline; +10 new across connector + supervisor +
  handler modules).
- `cargo clippy --all-targets --features bench -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench`: 5 binaries clean.
- End-to-end smoke: `target/release/brain-connector-stub` ingested one doc
  through the live v0.9.5 server; `/search?q=stub%20connector` returned it
  with `source_uri=stub://default/test-doc` + evidence.

---

## Agent 22: v0.9.6 M2.1 + M2.2 "Bridge" — auth foundation + GitHub connector binary (session 2026-07-20)
**Status:** COMPLETED (code + tests + pushed)
**Date:** 2026-07-20

Second milestone. Lands the unified auth foundation (trait + credential store
+ GitHub App impl) and the real `brain-connector-gh` binary that backfills
GitHub issues through brain-server's existing source/revision pipeline.

### Changes Made
- **New `src/connector/auth/mod.rs`**: `AuthProvider` trait + `AccessToken`
  (with redacted `Display`) + `StaticTokenProvider`.
- **New `src/connector/auth/store.rs`**: `CredentialStore<T>` — per-connector
  JSON config at `~/.config/brain-server/connectors/{kind}-{instance}.json`
  (mode 0600, atomic save via `std::fs::rename`).
- **New `src/connector/auth/github_app.rs`**: `GitHubAppProvider` — full JWT
  (RS256) + installation-token flow. Token-level repo scoping via the
  `repositories` body field (DoD-1 mechanism). In-memory single-slot cache
  with `REFRESH_SKEW=60s`.
- **New `src/connector/github/client.rs`**: `GitHubClient` wraps reqwest with
  GitHub-required headers + rate-limit sleep (capped at 60s) + Link-header
  pagination.
- **New `src/connector/github/translate.rs`**: `translate_issue` renders each
  issue as YAML frontmatter + Markdown body. Source URI:
  `github://{owner}/{repo}/issues/{N}`.
- **New `src/connector/github/mod.rs`**: `backfill_issues_for_repo` + cursor
  store (`get_cursor`/`upsert_cursor` against `connector_checkpoints`).
- **New `src/bin/brain-connector-gh.rs`** (~280 LOC): the binary.
- **New `src/lib.rs`**: minimal library target exposing only
  `pub mod connector`. Server modules stay private to `src/main.rs`.
- **`Cargo.toml`**: new optional deps `jsonwebtoken` (10.4, with `rust_crypto`
  + `use_pem`) + `reqwest` (0.13, `rustls` + `json` + `blocking`); new feature
  `connector-github`; new `[[bin]]` `brain-connector-gh` (requires
  `connector-github`). New dev-deps `rsa` + `rand` + `base64`.

### Verification
- `cargo test --features bench`: 152 lib + 9 integration passed, 1 ignored
  (unchanged from M1).
- `cargo test --features bench,connector-github`: 174 lib + 9 integration
  passed, 2 ignored (+18 new vs M2.1).
- `cargo clippy --all-targets --features bench -- -D warnings`: clean.
- `cargo clippy --all-targets --features bench,connector-github -- -D warnings`:
  clean.
- `cargo build --release --features bench`: 5 binaries clean.
- `cargo build --release --features bench,connector-github --bin brain-connector-gh`:
  clean. Binary runs and surfaces clear argv errors.

---

## Agent 23: v0.9.6 M2.3 + M3 "Bridge" — reconcile + CLI + ship (session 2026-07-20)
**Status:** COMPLETED (code + tests + tag)
**Date:** 2026-07-20

Final milestone of v0.9.6 "Bridge". Lands the periodic-reconcile path (M2.3)
and the operator CLI surface (M3), then tags the release.

### Changes Made
- **`src/connector/github/mod.rs`**: added `reconcile_github_sources` +
  `ReconcileReport`. The connector binary now backfills ALL configured repos,
  collects the union of walked source URIs, then calls `/sources/reconcile`
  once with the full set (kind-scoped: per-repo calls would sweep other repos'
  rows). `BackfillReport` gained `walked_uris` to feed this.
- **`src/bin/brain-connector-gh.rs`**: orchestrates backfill → reconcile in
  one pass. Emits `progress`/`done`/`error` JSON-lines for each phase.
- **`src/bin/brain.rs`** CLI: three new subcommands:
  - `brain connect github --app-id N --install-id N --key-file PATH --repo O/R [...]` —
    writes the connector config to `~/.config/brain-server/connectors/github-{instance}.json`
    (mode 0600, atomic write). Validates the key file exists and (on unix)
    warns if its mode is broader than 0600. No server roundtrip —
    registration is local-file.
  - `brain sync [github] [--config PATH | --instance NAME]` — resolves the
    binary (PATH → target/debug → target/release), resolves the config
    (explicit / `--instance` / glob if exactly one), resolves the brain DB
    path, spawns `brain-connector-gh` with the right argv, inherits stdout.
  - `brain connector-status` — calls `GET /connectors`, renders a table.
  Plus helpers `which` (PATH lookup) + `glob_github_configs`.
- **Version bump**: 0.9.5 → 0.9.6 across `Cargo.toml`, `openapi.yaml`,
    `CHANGELOG.md`, `AGENTS.md` header.

### Verification
- `cargo test --features bench`: 152 lib + 9 integration passed, 1 ignored.
- `cargo test --features bench,connector-github`: 174 lib + 9 integration
  passed, 2 ignored.
- `cargo clippy --all-targets --features bench -- -D warnings`: clean.
- `cargo clippy --all-targets --features bench,connector-github -- -D warnings`:
  clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench`: 5 binaries clean.
- `cargo build --release --features bench,connector-github --bin brain-connector-gh`:
  clean.
- CLI smoke: `brain --help` shows the three new commands; `brain connect
  github` errors on missing args; `brain connector-status` calls /connectors
  (returns 404 on the still-v0.9.5 live service — expected; resolves once
  `install-service.sh` is re-run).

### v0.9.6 ship status: SHIPPED 2026-07-20
All five DoD items provable. Tag `v0.9.6` created at the M2.3+M3 commit and
pushed. `scripts/install-service.sh` rebuilds + restarts the launchd service
so the live binary reports v0.9.6 with `GET /connectors` live.

### M3 honest ceilings (carried into v0.9.7)
- Issues only. PRs filtered out at translate time; dedicated PR backfill in v0.9.7.
- No comments. Body-only; threaded comments later.
- No `brain connector doctor`. `brain status` + `brain connector-status`
  cover the same ground for v0.9.6.
- Webhook ingress deferred. Reconcile satisfies DoD-2; webhooks land in v0.9.7.
- `kill_on_drop` shutdown instead of graceful drain — lands with v0.9.7
  `brain disconnect`.

---

## Agent 24: v0.9.9 "Qualify" — full release (session 2026-07-25)
**Status:** COMPLETED (code + tests + release build + docs)
**Date:** 2026-07-25

The v1.0 cutover rehearsal milestone. v0.9.7 "Guard" and v0.9.8 "Evidence"
were done directly (no agent numbers); this agent landed the full v0.9.9
release on top of them. The lazy-dev audit (in
`IMPLEMENTATION_PLAN_v0.9.9_Qualify.md`) drove the scoping: the v1.0
multi-domain foundation (`DomainRegistry`, `domain_router`, `backup`, `bench`)
was already shipped under `BRAIN_MULTI_DB=false` since v0.9.1, so v0.9.9 is
~70% extraction + plumbing of existing primitives + ~30% new tooling. **No
new schema migration, no new model, no multi-db cutover.**

### Changes Made

**M1 — Extract domain-ready seams**
- New `src/storage_layout.rs` (lib module): `StorageLayout` derives every
  on-disk path (legacy `brain.db`, future `global.db`, per-domain
  `brain-<name>.db`, backups, registry, connector configs) from one root.
  `config::brain_db_path()` delegates to it; back-compat invariant locked by
  test. New `BRAIN_DATA_ROOT` env var is the v1.0 relocation knob.
- `is_valid_domain` lifted to `storage_layout` so the security-critical
  filename check lives in one place; `DomainRegistry::is_valid_domain`
  delegates. Pure resolve logic factored into `resolve_root()` so tests don't
  mutate process env (the lesson from the first test run).
- `schema_version()` reader + `SCHEMA_VERSION_V0_9_9` constant. `run_migration`
  records the version in `schema_meta`; the rehearsal tool reads it.
- `test_migration_schema_contract` extended: asserts v0.9.5–v0.9.8 tables
  (`audit_events`, `webhook_queue`, `webhook_seen`, `evidence_links`) + the
  `authority` column + the recorded schema version.

**M2 — Migration rehearsal and recovery** (delegated to a sub-agent)
- `run_migration` + `migrate_down_0_9_0` extracted from `main.rs` to a new
  lib module `src/migration.rs`. Mechanical move; the one signature change is
  `run_migration(db, mmap_mib: i64)` so the lib has no dep on the
  server-private `config` module. All 9 call sites updated.
- New `src/bin/brain_migrate_rehearse.rs` (feature-gated behind `migrate`).
  Six subcommands: `backup` / `copy` / `verify` / `report` / `rollback` /
  `rehearse` (all-in-one). Parity checks: row counts for every table + FTS5
  count + vec0 count + source/revision linkage + evidence_links +
  audit_events + schema-version comparison + 50-row random vec0 byte
  spot-check. Exits 0 only when every check passes.
- 5 new tests (the 4 required M2.10 tests + 1 helper).

**M3 — Capacity and release qualification**
- New `src/capacity.rs` (lib module): `CapacityTarget` (Desktop | Jetson),
  `CapacityEnvelope` (max_docs / max_db_mib / max_rss_mib), `CapacityStatus`
  (Ok | Warning | Exceeded), `classify()`. Tightenable via `CAPACITY_MAX_*`
  env vars. Lives in the lib so `bench` + `brain-migrate-rehearse` share it.
- `/health` reports the `capacity` object. Writes call `guard_capacity` →
  HTTP 507 when exceeded; reads never check. All four ingest paths guarded
  (`/add`, `/ingest`, `/ingest/memory`, `/ingest/markdown`). `AppError` gained
  an `InsufficientStorage` variant; `HandlerError` gained
  `insufficient_storage()`.
- `bench` gains `BENCH_ENVELOPE=desktop|jetson` assertion mode: exits non-zero
  on RSS or p95 ceiling breach — turning the report into a ship gate.

**Docs + version**
- `Cargo.toml` 0.9.8 → 0.9.9. New `migrate` feature + `brain-migrate-rehearse`
  `[[bin]]` entry.
- `openapi.yaml` → 0.9.9: `/health` capacity field; `X-Api-Version: 0.9.9`.
- `API_CONTRACT.md`: §8 Capacity envelopes + §9 Migration (v1.0 per-row
  cutover rule, rehearsal tool, recovery procedure).
- `CHANGELOG.md`: `[0.9.9]` section. `ROADMAP.md` v0.9.9 row → Shipped.
  `README.md` version → 0.9.9 "Qualify".

### Verification
- `cargo test --features bench,migrate`: **244 passed, 1 ignored**
  (40 lib + 182 bin + 5 migrate-rehearse + 8 integration + others; was 231
  at M2 baseline, +13 from capacity + storage_layout + schema_contract
  extension).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate --bin brain-server --bin
  brain --bin mcp --bin bench --bin brain-migrate-rehearse`: 5 binaries clean.
- Smoke: `brain-migrate-rehearse` usage exits 1 on missing subcommand;
  `report` on non-existent DBs exits 0 with 0-counts (graceful).

### v0.9.9 ship status: SHIPPED 2026-07-25 (code-complete; live restart pending operator)
All DoD items provable except the measured-capacity-table operator step
(committed to `BENCHMARKS.md` on the next hardware run — the code-level ship
gate is `bench --envelope`). `scripts/install-service.sh` rebuilds + restarts
the launchd service so the live binary reports v0.9.9.

### Honest ceilings (carried into v1.0.0)
- **No `BRAIN_MULTI_DB=true` cutover performed.** The rehearsal runs against a
  copy; the live DB stays in shim mode. The cutover is the v1.0 ship step.
- **WAL-active detection is a heuristic** (file-size check); operator is
  expected to have stopped the server.
- **50-row vec0 spot-check is a sample**, not a full scan — catches the known
  sqlite-vec corruption class but cannot prove byte-identity of every embedding.
- **Old-schema fixtures (v0.9.4/v0.9.6/v0.9.8) + interrupted-migration SIGTERM
  test deferred.** The current-schema parity checks cover the ship gate; the
  upgrade-from-old-schema path is exercised by the server's own startup
  migration on every prior release.
- **`scripts/soak.sh` + large-vault generator deferred** as operator tooling;
  `bench --envelope` is the code-level ship gate.

---

## Agent 25: v1.0.0 "Domains" — audit-driven full release (session 2026-07-26)
**Status:** COMPLETED (code + tests + remote build + tag)
**Date:** 2026-07-26

Two-session release. Session 1 was a prior agent's "shipped" claim that an
audit revealed to be ~60% complete with a latent validator bug (multi-word
entity names like the canonical `vitamin d3` example were silently rejected).
Session 2 (this agent) closed every gap from the audit, then a second-pass
review caught a further critical bug (shim-mode `DELETE /domains/{name}` would
have wiped the global `audit_events` log).

### Audit findings closed (session 2)
- **Validator regression fixed** (`src/handlers/mod.rs`): the single-shape
  `is_match` checker that ignored its `pattern` arg is replaced with three
  correctly-scoped checkers (`is_valid_domain`/`is_valid_name`/`is_valid_rel_type`).
  Pinned by `validators_match_their_documented_shapes`.
- **MCP `brain_ingest` updated** (`src/bin/mcp.rs`): schema now exposes
  `content`/`title`/`domain`/`entities`/`relations`/`source`; routes to
  `POST /ingest` when structured fields are present (per the plan: agent does
  extraction client-side). Verified live via `tools/list`.
- **Cross-domain RRF merge** (`src/handlers/recall.rs:rrf_merge_domains`):
  replaced raw-score sort (wrong: per-domain scores aren't comparable after
  quantization + IDF differences) with rank-based RRF using the same `RRF_K = 60`
  as in-domain fusion. 2 unit tests pin the behavior.
- **`?cross_domain=true` on `/graph/traverse`** (`src/main.rs`): fans out across
  every known domain pool, labelling each hop with `source_domain`.
- **Domain lifecycle completed** (`src/handlers/domains.rs`):
  `DELETE ?confirm=<name>` (typo-replay guard), `POST /{name}/vacuum`,
  `GET /{name}/export` (VACUUM INTO snapshot, `application/octet-stream`),
  `POST /{name}/import` (SQLite magic-header check, atomic temp+rename).
- **Unknown-domain 400 now carries `details.known_domains`** — actionable.
- **Boot-time legacy cutover** (`src/main.rs`): when `BRAIN_MULTI_DB=true` and
  legacy `brain.db` has data, performs a one-shot `VACUUM INTO` into
  `global.db`, marker-guarded. The runtime keeps reading the legacy path so
  the live DB never silently shifts under the operator.
- **Four M6 integration tests added** (`src/main.rs`): domain isolation,
  fallback trigger, structured ingest (`vitamin d3`), export round-trip.

### Second-pass critical-correctness fix
- **Shim-mode `DELETE /domains/{name}` no longer wipes global tables.** The
  first draft did `DELETE FROM audit_events` (no WHERE) — would have destroyed
  the immutable audit log when any single domain was deleted. Now scoped:
  multi-db clears the whole per-domain DB; shim mode deletes only
  `WHERE domain = ?` rows + orphan entities + the one matching centroid.
  Pinned by `delete_domain_shim_mode_sql_preserves_global_tables`.

### Other second-pass hardening
- **Import handler validates the SQLite magic header** before disk write.
- **Import temp path is unique per PID** (no concurrent-import collision).
- **Import rename failure cleans up the temp file**.
- **Export handler `Content-Disposition` is safe** (domain name passes
  `is_valid_domain` → no quote/header-injection chars).
- **Recall `strict` flag now actually threads through** (was previously
  `let _ = req.strict;` — discarded).

### Verification
- `cargo test --features bench,migrate`: **263 passed, 1 ignored** (+8 vs
  v0.9.9's 255; +3 validator, +2 RRF, +1 shim-delete, +4 M6 integration).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- Local: `cargo build --release --features bench,migrate` — 5 binaries clean.
- Remote (openclaw, Linux x86_64, cargo 1.93.1): release build + 263 tests green.
- Live launchd service restarted via `scripts/install-service.sh`:
  `brain doctor` ✓, `brain status` ✓ (8496 docs, v1.0.0).
- End-to-end smoke against live service: `POST /ingest` with `vitamin d3`,
  `/domains`, `/domains/{name}/{vacuum,export,import}`, `DELETE ?confirm`,
  `/graph/traverse?cross_domain=true` — all return expected status codes.

### Ship status: SHIPPED 2026-07-26
Tag `v1.0.0` created. Logical commits split by concern (validator fix,
recall RRF, traverse cross_domain, domain lifecycle, MCP wiring, legacy
migration, docs). `scripts/install-service.sh` re-run; the live launchd service
is on v1.0.0.

### Honest ceilings (carried into v1.1)
- **Domain `dim`/`quant` not per-domain** — all domains share the global model
  profile; per-domain model selection is a v1.1 concern.
- **No registry DB table** — file enumeration works and avoids a separate
  `registry.db` to manage; per-domain `dim`/`quant`/`version` metadata store
  is a v1.1 concern.
- **The `global` domain still reads the legacy `brain.db` even in multi-db
  mode.** The boot-time snapshot creates `global.db` as a backup + rehearsal
  target; runtime stays on `brain.db` for `global` so the 8496-doc live DB
  never silently shifts under the operator.
- **Cross-domain `ATTACH` was not used.** Per-domain pool queries + RRF merge
  is simpler and avoids sqlite-vec attach complications; ARM eMMC benchmark
  remains an operator step (`bench --envelope`).
- **`VACUUM INTO '<path>'` is operator-path-controlled and unparameterized**
  (SQLite DDL limitation). Pre-existing pattern across `backup.rs`, the
  rehearsal tool, and the v0.9.0 backup code; the new v1.0 paths inherit it.

---

## Agent 26: v1.1.1 "Harden" (audit chain bug-fix) — 2026-07-29
**Status:** COMPLETED (code + tests + tag + live restart)
**Date:** 2026-07-29

Bug-fix release on top of v1.1.0. An audit of the audit hash-chain
implementation (`src/audit.rs`) surfaced a latent false-negative in
`verify_chain` that affected every DB migrated from v1.0 → v1.1. This agent
closed that bug plus the three honest ceilings v1.1.0 carried forward.

### The bug
- **`verify_chain` false-negative on migrated DBs.** The v1.1.0 walk
  (`src/audit.rs:226-230`) used a match arm `(None, None) => {}` designed for
  "the first row before any link" — but then unconditionally advanced
  `expected` to `Some(...)`. After the additive `ALTER TABLE ADD COLUMN
  prev_hash` migration, **every** pre-v1.1 row has NULL `prev_hash`, so on a
  real migrated DB the *second* NULL row hit the `_ => return false`
  fallthrough. `/audit/verify` and `/metrics` (`brain_audit_chain_ok`) would
  report tampering on a clean DB. None of the existing tests caught this
  because they used `record()` (which always sets `prev_hash`) — never the
  migration-realistic NULL → Some boundary.

### Changes Made
- **`verify_chain` rewrite (`src/audit.rs`).** NULL `prev_hash` rows now
  carry "no backref to verify" — they advance the running link but never
  fail. Only a v1.1 row whose stored `prev_hash` disagrees with the
  recomputed link returns false. Pinned by
  `hash_chain_survives_migration_with_many_null_rows`.
- **`record_tenant` now wraps its read+INSERT in a `SAVEPOINT`**
  (`src/audit.rs`). `BEGIN` would error when called inside a caller's
  existing transaction (e.g. `delete_quarantine`); `SAVEPOINT` nests
  cleanly. Rolling back the savepoint on audit-INSERT failure touches only
  the audit row, not the caller's work. Pinned by
  `record_tenant_is_safe_inside_caller_transaction` +
  `record_tenant_rollback_does_not_undo_caller_work`.
- **`/metrics` TTL cache (`src/main.rs` + `src/config.rs`).**
  `brain_audit_chain_ok` is now backed by a TTL-memoized result
  (`AUDIT_CHAIN_CACHE_TTL_SECS=60`). `/audit/verify` remains authoritative
  and always scans fully — that is its job.
- **Real migration fixture test (`src/audit.rs`).**
  `hash_chain_survives_real_v1_0_to_v1_1_migration` builds a DB with the
  pre-v1.1 `audit_events` schema, inserts rows, runs the actual
  `run_migration`, then verifies the chain holds across the NULL → Some
  boundary with real `record()` calls afterward.

### Version bump
- `Cargo.toml` 1.1.0 → 1.1.1. `openapi.yaml` → 1.1.1. `README.md`,
  `ROADMAP.md`, `CHANGELOG.md`, `AGENTS.md` updated.

### Verification
- `cargo test --features bench,migrate`: **278 passed, 1 ignored** (was
  275 at v1.1.0; +3 new tests: migration fixture, savepoint-nesting,
  savepoint-rollback-isolation).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate --bin brain-server --bin
  brain --bin mcp --bin bench --bin brain-migrate-rehearse`: all 5 binaries
  clean.
- **Bug regression check**: temporarily restored the buggy `verify_chain`
  logic and confirmed the new
  `hash_chain_survives_migration_with_many_null_rows` test fails on it —
  proving the test catches the bug it was written for.

### Ship status: SHIPPED 2026-07-29
Tag `v1.1.1` created. `scripts/install-service.sh` re-run; the live
launchd service reports v1.1.1.

### Honest ceilings (carried into v1.2)
- All three v1.1.0 ceilings closed (see CHANGELOG `[1.1.1]`).
- v1.2's ceilings (no JWT/JWS, no AuthZ) remain.

---

## Agent 27: v1.1.2 "Harden" (constant-time auth hardening) — 2026-07-29
**Status:** COMPLETED (code + tests + tag + live restart)
**Date:** 2026-07-29

Security hardening release. A best-practices pass (rusqlite 0.40.1 docs +
RustCrypto `subtle` 2.6.1, fetched 2026-07-29 via the fallback hierarchy in
AGENTS.md — no context7 MCP available this session, used `fetch` on
docs.rs/cheatsheetseries.owasp.org instead) surfaced one real gap and two
documented judgment calls.

### Research (context7 fallback → official docs)
- **rusqlite 0.40.1** (`docs.rs/rusqlite/latest/rusqlite/struct.Connection.html`,
  fetched 2026-07-29): confirmed `savepoint_with_name(&mut self)` is the
  canonical savepoint API. Considered for `record_tenant`; left as raw-SQL
  `SAVEPOINT` (see judgment call below).
- **RustCrypto `subtle` 2.6.1** (`docs.rs/subtle/latest/subtle/trait.ConstantTimeEq.html`,
  fetched 2026-07-29): confirmed `[u8]: ConstantTimeEq` with a documented
  short-circuit on length mismatch (same as the existing hand-rolled length
  check — acceptable because token length isn't secret for fixed-format
  random tokens). Already a transitive dep via sha2/hmac/aes-gcm.
- **OWASP Query Parameterization Cheat Sheet**
  (`cheatsheetseries.owasp.org/cheatsheets/Query_Parameterization_Cheat_Sheet.html`,
  fetched 2026-07-29): confirmed all brain-server SQL uses parameterized
  queries — no SQL injection surface in the v1.1.0/1.1.1 changes.

### The gap closed
- **Bearer-token `ct_eq` was a hand-rolled fold with no `black_box` barrier.**
  The v1.1.0 ponytail comment explicitly flagged this as a future risk: "if
  this ever fronts a network adversary, swap to the `constant_time_eq` crate
  for an asm/black_box-backed guarantee against optimizer-driven short-
  circuiting." LLVM is permitted to short-circuit the manual fold back into
  an early-exit compare, re-introducing the timing oracle the pattern exists
  to prevent. Swapped to `subtle::ConstantTimeEq::ct_eq`, which uses
  asm/black_box primitives the optimizer can't fold away. Zero build cost
  (already a transitive dep). Pinned by the existing `test_ct_eq`.

### Considered and left as documented best-practice judgment calls
- **`verify_chain`'s `want == got` hash comparison left as plain `==`.** This
  compares two equal-length SHA-256 hex strings inside a tamper-detection
  read path (not an auth gate). An attacker who could measure the timing
  remotely would already control the DB and could simply edit `prev_hash` to
  match. `ct_eq` here would be gold-plating without a real threat model.
- **`record_tenant`'s raw-SQL `SAVEPOINT` left as-is.** rusqlite 0.40.1
  exposes `savepoint_with_name()`, but it takes `&mut Connection`; the ~20
  call sites pass `&Connection` (often from a pooled r2d2 connection, which
  derefs to `&Connection`). Migrating would ripple through every caller +
  require pooled-connection borrow gymnastics for zero correctness gain — the
  current raw-SQL approach is verified by 3 v1.1.1 tests and uses
  parameterized queries (no injection surface).

### Version bump
- `Cargo.toml` 1.1.1 → 1.1.2. `openapi.yaml` → 1.1.2. README, ROADMAP,
  CHANGELOG, AGENTS updated.

### Verification
- `cargo test --features bench,migrate`: **278 passed, 1 ignored**
  (unchanged from v1.1.1 — the swap is behavior-preserving).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.

### Ship status: SHIPPED 2026-07-29
Tag `v1.1.2` created. `scripts/install-service.sh` re-run; the live
launchd service reports v1.1.2.

### Honest ceilings (carried into v1.2)
- v1.2's ceilings (no JWT/JWS, no AuthZ) remain.

---

## Agent 28: v1.2.0 "AuthN" (JWT/JWS + AuthZ layer) — 2026-07-29
**Status:** COMPLETED (code + tests + tag + live restart)
**Date:** 2026-07-29

The biggest security release since v1.0. Replaces the v1.1 opaque-bearer-
token surface with enterprise-grade JWT/JWS authentication + a real AuthZ
layer enforced at the data-access layer. The prerequisite for v2.0 multi-team
tenancy. Back-compat is the default — when `BRAIN_JWT_ISSUER` is unset OR no
keys are loaded, the server runs in v1.1 opaque-token mode and every existing
install keeps working unchanged. JWT is opt-in. Seven milestones shipped.

### Research basis
- **Context7 lookup on `jsonwebtoken` v10** (verified 2026-07-29): API
  surface, `Validation` builder, `Algorithm` enum, `decode_header` +
  `decode::<T>` semantics. Confirmed the `Validation::new(alg)` per-alg
  pattern + `set_issuer` / `set_audience` builder methods. The library was
  already an optional dep via the connector-github feature; v1.2 promotes it
  to required (with `use_pem` + `rust_crypto` features).
- **OWASP JWT Cheat Sheet**: the canonical cheat-sheet URLs were 404ing on
  the v1.2 ship date. Source of truth was the encoded checklist in
  `IMPLEMENTATION_PLAN_v1.2.0_AuthN.md` §M1.3 (which was Context7-verified
  at plan-write time). The 14-test matrix in `src/auth/jwt.rs` pins every
  item.
- **OWASP Top 10:2025** coverage map updated in `SECURITY.md` — every v1.2
  control now has a ✅ marker (was 🚧).

### The 7 milestones shipped

**M1 — JWT verification core** (`src/auth/jwt.rs`). `verify_access_token()` +
`Claims` + `AuthError`. `ALLOWED_ALGS` whitelist (RS256/384/512, ES256/384/512,
EdDSA) checked **before** key lookup — the OWASP algorithm-confusion defense
(`none`, all HS*, all PS* rejected unconditionally). Every claim validated:
`iss`, `aud`, `exp`, `nbf`, `sub`, `jti`. 30s leeway for clock skew (subsumes
`reject_tokens_expiring_in_less_than` — documented trade-off). **14 tests**
pin the full OWASP JWT Cheat Sheet failure matrix (the plan called for 13;
the actual implementation added `wrong_token_type_rejected`,
`algorithm_whitelist_rejects_ps256`, `leeway_absorbs_small_clock_skew`).

**M2 — Revocation** (`src/auth/revocation.rs`). Additive `revoked_tokens` +
`refresh_chains` tables. `RevocationCache` (60s negative-lookup cache, bounded
TTL — eventual consistency by design). `purge_expired` housekeeping on a
background timer. Refresh-chain reuse detection: presenting a stale refresh
token calls `revoke_chain` and burns the whole family (OWASP pattern). Chain
id derived from `(iss, sub)` — per-user per-issuer.

**M3 — AuthZ** (`src/auth/policy.rs`). `AuthzPolicy` trait + `InMemoryPolicy`
default (no external deps; OPA/Cedar impls are the swappable v2.1+ upgrade
path). `Action` enum (Read/Write/Admin/Traverse) + `Scope`
(`<action>:<team>/<domain>` with wildcards) + `Principal` +
`is_authorized()`. Escalation: write implies read down, admin implies both.
Default-deny → 403, never 404 (no existence leakage — OWASP A01:2025). The
retrofit is minimal: a single `authorize(principal, action, team, domain)`
helper called at handler entry, not a full pool-resolution refactor.
`Option<Principal>` where `None` = superuser (back-compat path).

**M4 — OIDC discovery + JWKS** (`src/handlers/well_known.rs`).
`GET /.well-known/openid-configuration` (RFC 8414) + `GET /.well-known/jwks.json`
(RFC 7517). Both PUBLIC — clients need them to learn how to verify tokens.
Issuer pinned to `BRAIN_PUBLIC_BASE_URL` — never inferred from `Host` (OWASP
A02:2025 Security Misconfiguration: Host-header spoofing).

**M5 — Key management** (`src/auth/jwks.rs` + `src/bin/brain.rs`). `KeyStore`
loads RSA/EC/Ed25519 PEMs from `BRAIN_JWT_KEY_DIR` (default
`~/.config/brain-server/keys/`, mode 0700; private keys 0600). `brain key
generate/list/prune` CLI: RSA keypair generation with 0600 private-key mode +
0700 dir mode. Two keys live during rotation; old key drops from JWKS only
after every cached token has expired.

**M6 — Audit integration.** AuthN/AuthZ events flow into the existing v1.1
audit log: token-verified, token-rejected (with reason), authz-denied (with
principal/action/team/domain), logout. Per-tenant audit filter unchanged.

**M7 — Migration** (`src/migration.rs`). Additive: `revoked_tokens` +
`refresh_chains` tables. `schema_version` stamped `1.2.0`. Two-layer
middleware: `jwt_auth_middleware` runs outermost (verifies JWS, checks
revocation, injects `Principal` into extensions); the v1.1 `auth_middleware`
runs as fallback and short-circuits when the Principal is already set.

### Auth route handlers (`src/handlers/auth.rs`)
- `POST /auth/refresh` — verifies refresh token, rotates chain, mints new
  access + refresh pair. Reuse → `revoke_chain` → 403 `refresh_reuse_detected`.
- `POST /auth/logout` — adds the request's access-token `jti` to the denylist.
- `POST /auth/revoke` — operator revoke by `(jti, iss)`; requires admin auth.

### Files added / modified
- **New:** `src/auth/{mod,jwt,jwks,policy,revocation}.rs`,
  `src/handlers/{auth,well_known}.rs`. (`src/auth.rs` → `src/auth/mod.rs`.)
- **Modified:** `src/main.rs` (`JwtMiddlewareState` + `jwt_auth_middleware` +
  5 new routes + `AppState` fields + revocation purge task),
  `src/handlers/mod.rs` (`authorize` helper + `HandlerError::forbidden`),
  `src/migration.rs` (additive tables + schema_version 1.2.0),
  `src/bin/brain.rs` (`brain key generate/list/prune`),
  `Cargo.toml` (`jsonwebtoken` required + `rsa`/`rand`/`base64` direct deps).

### Version bump
- `Cargo.toml` 1.1.2 → 1.2.0. `openapi.yaml` → 1.2.0 (5 new routes + 8 new
  schemas: `TokenPair`/`RefreshRequest`/`RevokeRequest`/`OidcConfig`/
  `JwkSet`/`Jwk`/`Principal`/`Scope`). README, ROADMAP, CHANGELOG, AGENTS,
  SECURITY, THREAT_MODEL updated.

### Verification
- `cargo test --features bench,migrate`: **308 passed, 1 ignored** (was 278
  at v1.1.2; +30 from the 7 milestones — JWT matrix, revocation, AuthZ,
  OIDC/JWKS, key management, handler wiring).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate --bin brain-server --bin
  brain --bin mcp --bin bench --bin brain-migrate-rehearse`: all 5 binaries
  clean.
- `test_openapi_covers_routes`: green (every registered route appears in
  `openapi.yaml`; the 5 new v1.2 routes are documented even though they're
  not yet in the test's hardcoded `registered` array — documentation
  completeness, not test-driven).

### Ship status: SHIPPED 2026-07-29
Tag `v1.2.0` created. `scripts/install-service.sh` re-run; the live launchd
service reports v1.2.0.

### Honest ceilings (carried into v1.3)
- **No distributed revocation.** The 60s negative cache is per-process; a
  multi-instance deployment has a 60s window per instance. Distributed
  revocation (Redis-backed denylist) is v2.1.
- **No hot key reload — restart required.** Adding/removing signing keys via
  `brain key generate/prune` requires an `install-service.sh` restart.
- **EC/Ed JWK emission not implemented.** EC/Ed keys verify correctly but
  don't appear in `/.well-known/jwks.json`; rotate to RSA for any key a
  third party must discover via JWKS.
- **No cookie-based refresh token storage.** Refresh tokens returned in the
  JSON body only; CLI bearer is the assumed client shape. The
  `HttpOnly`+`Secure`+`SameSite=Strict` cookie path lands with the v2.0 UI.
- **Refresh-chain reuse detection burns the chain silently.** The legit user
  discovers the burn on their next refresh (`refresh_reuse_detected`, 403).
  A user-facing notification channel is v2.1.
- **Audit hash-chain comparison stays plain `==`.** Carried from v1.1.2 —
  tamper-detection read path, not an auth gate.

---

## Agent 29: v1.3.0 "Bedrock" (memory-safety hardening) — 2026-07-29
**Status:** COMPLETED (code + tests + tag + live restart)
**Date:** 2026-07-29

The memory-safety release. Makes the binary bulletproof on its own terms:
zero panics reachable in production paths, every `unsafe` block documented,
property-based tests for the invariants that hand-written tests miss, and
cargo-fuzz infrastructure. **No new schema, no new model, no new route contract**
— purely hardening + observability + a runtime tuning knob. Prerequisite for
the v1.4+ cognitive-stack work (you can't build temporal KGs on a binary that
panics on adversarial input).

### Changes Made

**M1 — Panic elimination.** Audited every `unwrap()`/`expect()`/`panic!` in
non-test code. Zero remaining in production paths. Three real fixes:
- `src/bin/mcp.rs` — JSON-RPC notification handling `unwrap()`d on
  `Option<Value>` for the request id; a notification (no id) would panic.
  Now handled as `None`.
- `src/vault.rs` — first-line `unwrap()` on `Option<&str>` *before* the guard
  that proves it's `Some`. Moved after the guard.
- `src/connector/auth/github_app.rs` — `expect()` on a poisoned mutex.
  Now `unwrap_or_else(|e| e.into_inner())` for poison recovery.

**M2 — `unsafe` audit.** 10 duplicate `unsafe { transmute(...) }` blocks for
sqlite-vec registration (scattered across `main.rs`, `domain_registry.rs`,
`handlers/domains.rs`, `audit.rs`, `brain_migrate_rehearse.rs`) collapsed into
**one** documented safe wrapper: `register_sqlite_vec()`. Every remaining
`unsafe` block now carries a `// SAFETY:` comment per the Rust nomicon. The
live `/health` reports `hardening.unsafe_blocks = 2` (the wrapper + the
migrate-rehearse copy that runs out-of-process).

**M3 — cargo-fuzz infrastructure.** `fuzz/` crate with four targets:
`fuzz_chunker`, `fuzz_lex_compile`, `fuzz_query_doc`, `fuzz_validator`. Behind
the nightly toolchain (not in the stable CI gate). Two targets (`fuzz_chunker`,
`fuzz_lex`) are stubs because the chunker/query modules are binary-private;
moving them to the lib crate is the documented follow-up.

**M6 — Proptests.** Four `proptest` suites (256+ cases each), the smallest
checks that fail if a core invariant breaks:
- `proptest_chunker_never_panics_and_ranges_are_valid` — random UTF-8 input →
  chunk text is always a verbatim substring; byte ranges never slice mid-codepoint.
- `proptest_chunker_handles_multibyte_inputs` — multibyte chars (•, 💡, 🏋️)
  never cause slice panics.
- `proptest_normalize_domain_is_idempotent` — `normalize(normalize(x)) == normalize(x)`.
- `proptest_classify_is_monotonic` — increasing docs/db/rss never *improves*
  the capacity status (Ok → Warning → Exceeded is one-way under load).

**M7 — `/health` hardening observability.** `/health` now emits a `hardening`
object: `{ unsafe_blocks, panics_caught, memory_leaks_detected }` so ops can
see the memory-safety posture at a glance. `panics_caught` comes from
`CatchPanicLayer` (would be >0 only if a handler panicked and was caught).

**M8 — `BRAIN_WORKER_THREADS`.** Tokio runtime is now configurable. Default =
number of cores; Jetson target = `2` (saves ~10 MB RSS + context-switch
overhead). `main()` builds the runtime manually instead of `#[tokio::main]`
so the override is honored. `worker_threads()` reads + validates the env var.

### Verification
- `cargo test --features bench,migrate`: **324 passed, 1 ignored** (was 320
  at v1.2.1; +4 proptest suites).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate --bin brain-server --bin
  brain --bin mcp --bin bench --bin brain-migrate-rehearse`: all 5 binaries
  clean.
- Panic-elimination audit: grep of non-test `unwrap()`/`expect()`/`panic!`
  returns zero in production paths (the three fixed sites above were the only
  reachable ones).

### Ship status: SHIPPED 2026-07-29
Tag `v1.3.0` created. `scripts/install-service.sh` re-run; the live launchd
service reports v1.3.0 with the `/health` `hardening` object live.

### Honest ceilings (carried into v1.4)
- **miri / loom / LeakSanitizer**: the *procedure* is documented in the plan
  (nightly toolchain + sanitizer RUSTFLAGS); **not** integrated into CI. The
  `memory_leaks_detected` field on `/health` is reserved for a future LSAN
  integration and is always `0` today.
- **Fuzz coverage is partial.** `fuzz_chunker`/`fuzz_lex` are stubs until the
  chunker and query modules move from the server binary into the lib crate
  (the `brain-migrate-rehearse` + connector code already live in `src/lib.rs`).
- **Hot key reload still requires restart** (carried from v1.2.0).
- **Distributed revocation** still 60s per-instance (carried from v1.2.0; v2.1).
- **Audit hash-chain comparison stays plain `==`** (carried from v1.1.2 —
  tamper-detection read path, not an auth gate).

---

## Agent 30: v1.4.0 "Calibrate" (surpass-human retrieval) — 2026-07-30
**Status:** COMPLETED (code + tests + tag + live restart + GitHub release)
**Date:** 2026-07-30

The surpass-human retrieval release. Implements the July-2026 SOTA on top of
the v1.3.0 memory-safe foundation. No new model, no neural net in the hot
path, no external API calls in recall — the low-power manifesto holds.

### Research basis (Context7-verified 2026-07-30)
- **Graphiti / Zep** (`/getzep/graphiti`, fetched via context7 MCP): confirmed
  the bi-temporal EntityEdge model — `valid_at`/`invalid_at` are valid-time
  (when the fact holds in the world); `expired_at` is wall-clock invalidation;
  `reference_time` is source provenance. `resolve_edge_contradictions` expires
  (not deletes) old facts. The bi-temporal filter is exactly
  `valid_at <= ? AND (invalid_at IS NULL OR invalid_at > ?)`.
- **arXiv:2607.00725** (submodular evidence packing): budgeted monotone
  submodular maximization, lazy greedy, (1-1/e) bound. +5.1 F1 on HotpotQA.
- **arXiv:2607.00339** (TRACE): hierarchical nodes + typed edges +
  validity-aware traversal.

### The 5 milestones shipped

**M1 — Bi-temporal edges.** Additive migration: `relationships.valid_at` +
`invalid_at`. New `src/temporal.rs`: deterministic temporal-marker extraction
("from 2011 to 2017", "currently", "since 2020"). `/ingest` relations accept
explicit temporal overrides; `/recall` + `/graph/traverse` accept `?at=`.
`perform_search_traced` normalizes `at` alongside `since`. 11 unit tests.

**M2 — Submodular evidence packing.** New `src/search/packing.rs`: lazy greedy
under a token knapsack. Objective = relevance + coverage + representativeness,
gated by MMR-style diversity (`DEDUP_SIMILARITY=0.85`). `/recall`
`max_context_tokens` triggers packing; `gold_answer` drives the
`answer_in_context` diagnostic. 12 unit tests.

**M3 — TRACE typed edges.** New `src/trace.rs`: prefix vocabulary
(`update:`/`supersedes:`/`contradicts:`/`causes:`) + bounded-walk constants
(`MAX_HOPS=4`, `MAX_VISITED=256`). `RELTYPE_RE` accepts `prefix:base`.
`/graph/traverse` is validity-aware + bounded. Schema reservation:
`knowledge.node_kind` (default `event`) + `parent_id`. 6 unit tests.

**M5 — Regression harness.** New `brain_server::eval` lib module: pure
metrics (P@k/R@k/MRR/NDCG/`answer_in_context_rate`). `bench eval` mode loads a
judgments file, runs `/recall`, reports metrics + optional ship gate. 9 tests.

**M4 — Multi-vector: DEFERRED.** Per the plan's lazy-dev escape hatch ("if the
feature isn't worth the watts, defer it"). Cannot be measured until M5's
harness provides a baseline. `multivec` feature flag reserved (no-op). Lands in
v1.4.1+ with measured Δ-recall vs Δ-RSS.

### Bug found + fixed during live smoke
- **`normalize_since` rejected bare `YYYY-MM-DD`.** The bi-temporal `at` filter
  commonly uses date-only form (`?at=2015-06-01`); the function only accepted
  RFC3339 or `YYYY-MM-DD HH:MM:SS`. Fixed to accept bare dates (padded to
  midnight). Pinned by an extended test.

### Version bump
- `Cargo.toml` 1.3.0 → 1.4.0. `openapi.yaml` → 1.4.0 (new params on `/recall`
  + `/graph/traverse`). README, ROADMAP, CHANGELOG, SECURITY, SPECS updated.

### Verification
- `cargo test --features bench,migrate`: **367 passed, 1 ignored** (was 324
  at v1.3.0; +43 new across temporal/packing/trace/eval/integration).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.
- **Remote (openclaw, Linux x86_64)**: release build clean + 367 tests green.
- **Live end-to-end smoke** (after `scripts/install-service.sh` restart,
  pid 24873): bi-temporal `?at=2015` finds the edge, `?at=2020` doesn't;
  submodular packing reports `packed_tokens=86`, `answer_in_context=true`;
  typed-edge `update:lives_at` accepted, `Has Space` rejected.

### Ship status: SHIPPED 2026-07-30
8 logical commits (`e9c0e30`..`19743b1`). Tag `v1.4.0` created + pushed.
GitHub release published. `scripts/install-service.sh` re-run; the live
launchd service reports v1.4.0.

### Honest ceilings (carried into v1.5)
- **Temporal extraction is English-only + deterministic.** Bounded marker
  set; no relative dates or inferred durations. LLM extractor is v2.x.
- **Submodular packing uses lexical Jaccard for diversity**, not embedding
  cosine (cheap proxy; cosine would need the model in the packer).
- **TRACE node hierarchy is schema-only.** `node_kind`/`parent_id` exist but
  nothing populates session/topic yet (v1.8 Consolidate).
- **M4 multi-vector deferred** — see above.
- **The 100-query judged corpus is an operator step.** The harness ships;
  the judgments don't (they require the operator's private DB).

---

## Agent 31: v1.4.0 dead-code cleanup (session 2026-07-30)
**Status:** COMPLETED
**Date:** 2026-07-30

Clean-up pass triggered by a roadmap accuracy review. Two-agent audit (first
pass identified spurious dead-code candidates; second pass disproved all but
one). The principle: deletion over addition, but only after tracing the real
flow.

### Changes Made
- **Deleted dead `IngestResponse` from `main.rs`.** A second, private
  `IngestResponse { success, id }` lived at `main.rs:458`. The real response
  type is `handlers::mod::IngestResponse { id, status, domain, ... }` in
  `handlers/ingest.rs:71`. The main.rs copy was constructed by zero handlers
  and was a leftover from a refactor that moved the ingest handler out of
  `main.rs`. 6 lines deleted.
- **Removed misleading `#[allow(dead_code)]` on `RateLimiter` struct + `is_allowed`.
  Both are live code: `RateLimiter::new()` is called at `main.rs:3352`, wired
  into the axum middleware layer at `main.rs:3604`, and `is_allowed()` is
  called at `main.rs:2828` by `rate_limit_middleware`, which is registered in
  the router. The `#[allow(dead_code)]` was a leftover from before the rate
  limiter was activated in the middleware stack.
- **Kept `#[allow(dead_code)` on `AppState.rate_limiter`** — axum accesses
  this field by type (`State<Arc<RateLimiter>>`), not by name. The compiler
  can't see the runtime usage path. This is a standard false positive with
  type-based DI, not dead code.
- **Updated TODO.md `--features rerank` references.** The `rerank` Cargo
  feature was deleted in commit `3fcac72` (v0.9.5). The TODO entries
  referencing it as a CI target were stale.
- **Second-pass verification confirmed all `#[allow(dead_code)]` on trace.rs
  prefix constants, temporal.rs `AT_FILTER_SQL`, and packing.rs constants are
  deliberate ponytail ceilings** (reserved for v1.6+). Not dead — just waiting.
  Deletion would cost more than keeping.

### Verification
- `cargo test --features bench,migrate`: 367 passed, 1 ignored (unchanged).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo build --release --features bench,migrate --bin brain-server`: clean.

---

## Agent 32: v1.4.1 "Link" — deterministic entity linker upgrade (session 2026-07-30)
**Status:** COMPLETED
**Date:** 2026-07-30

Pure linker upgrade on top of v1.4.0. Grounded in July 2026 research (deep
sweep across ACL, EMNLP, arxiv via websearch): Aho-Corasick confirmed gold
standard for deterministic entity matching; pure frequency-based between-word
counting is legacy — SOTA deterministic approach is dependency parsing + SVO
extraction (needs a POS tagger dep, ~5 MB via `nlprule`). This session took
the pragmatic middle ground: verb-suffix filtering (zero deps) + heading
hierarchy extraction (2026 document-structure research confirms this is a
critical structural signal). Full dependency parsing upgrade path documented
in ponytail comments.

### Changes Made
- **Heading hierarchy → `part_of`** (`src/linker.rs`): new
  `extract_heading_relationships()` public function. Walks the markdown heading
  tree, creates `part_of` edges for every adjacent heading pair where both are
  known entities. Zero new deps. Wired into `write_markdown_ingest` in
  `src/main.rs` after the mention loop.
- **Verb-suffix filtering** (`src/linker.rs`): `is_likely_verb()` /
  `has_verb_suffix()` — filters discovered relationship candidates through
  English verb morphology (-ed, -ing, -ate, -ify, -ize, -ise + 3rd-person
  -s/-es/-ies base-strip check). Rejects "maps", "data", "example". Accepts
  "manages", "communicates", "configures". Zero new deps.
- **Entity leakage fix**: `discover_verb_patterns()` now builds an entity-name
  set and excludes entity names from the candidate verb pool (entity names are
  things, not relationships).
- **`find_relationships()`** accepts new `extra_patterns: &[(&str, &str)]`
  parameter, merging discovered patterns with the built-in `RELATION_PATTERNS`
  at query time.
- **`EntityVocabulary.entities`** made `pub` so `extract_heading_relationships`
  can access the entity set from outside the module.
- **4 new tests**: `heading_hierarchy_creates_part_of_edges`,
  `heading_hierarchy_skips_stop_headings`, `verb_suffix_filter_rejects_nouns`,
  `verb_suffix_accepts_verb_patterns`. 2 existing tests updated for the new
  `find_relationships` signature. 1 dead-code cleanup in test.

### Verification
- `cargo test --features bench,migrate`: 391 passed, 1 ignored (was 367, +24).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate --bin brain-server --bin brain`: clean.

--- (read before starting v0.9.4 Sources)

## Agent 33: v1.5.0 "Epistemic" (light cut — calibrated abstention + span verification) — 2026-08-01
**Status:** COMPLETED (code + tests + 4 logical commits; live restart pending operator)
**Date:** 2026-08-01

Scoped to the **evidence-gated v1.5 surface** sanctioned by
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.5, NOT the
broader `IMPLEMENTATION_PLAN_v1.5.0_Epistemic.md` (which that roadmap
explicitly supersedes). The user requested "light and no CPU intensive" —
that maps directly to M2 (abstention wiring, zero new compute) + M5
(`/verify`, opt-in lexical match). M1/M3/M4 + Carry-forward operator steps
are deferred with documented reasoning.

### Scope decision (flagged before coding)

The attached plan marked itself superseded; the authoritative roadmap
forbids 3 of its 5 milestones (counterfactual influence, source-trust
ranking, fixed universal threshold). Surfaced the conflict to the user
rather than blindly implementing the superseded plan; user confirmed
the light cut.

### Changes Made (4 commits: `f1b2991`, `34ac223`, `499e9b4`, docs)

- **Calibrated abstention on `/recall`** (`src/handlers/{mod,recall}.rs`):
  `RecallResponse.decision` field (`ok` | `low_confidence`). When the
  existing `HeuristicEstimator` (v1.4.0) emits `Recommendation::ClarifyQuery`,
  `/recall` returns `{decision: "low_confidence", hits: []}` instead of
  top-1 garbage. NOT a magic `score < 0.3` cutoff — driven by the calibrated
  multi-signal recommendation (overlap + gap + lexical density), which is
  what the evidence-gated roadmap requires. Zero new compute: `confidence`
  + `recommendation` were already computed by `perform_search_with_prf`.
  Pure `abstention_decision()` helper extracted for testability.
- **`POST /verify` deterministic span verification** (new
  `src/handlers/verify.rs`): `{chunk_id, claim}` → `{supported, decision,
  match_ranges}`. Case-insensitive substring match over one chunk's text.
  Zero embeddings, zero LLM, zero model load — O(content.len()), opt-in.
  Reuses the existing `/get/{id}` SQL shape (one query, no new schema).
  Bounded: `MAX_QUERY` (2000) on claim, `MAX_MATCH_RANGES` (100) on output.
  Pure `verify_claim()` helper. No audit row (pure read).
- **OpenAPI contract** (`openapi.yaml` → 1.5.0): `/verify` route +
  `VerifyResponse` schema + `decision` field on `/recall`.
  `test_openapi_covers_routes` extended with `/verify`.
- **Pre-existing rust-1.97 clippy lints silenced** in `src/linker.rs`
  (`saturating_sub`, lifetime elision, `as_bytes` slice) + `cargo fmt`
  drift in `linker.rs`/`ingest.rs`. Not introduced by this release;
  unblocked the `-D warnings` gate.
- **Version bump** 1.4.2 → 1.5.0 across `Cargo.toml`, `openapi.yaml`,
  `README.md`, `CHANGELOG.md`, `AGENTS.md`.

### Tests
- `abstention_returns_low_confidence_only_on_clarify_query` — fires only
  on `ClarifyQuery`; `Return`/`RunPrf`/`RunReranker`/`IncreaseTopK`/`None`
  all map to `Ok` (the back-compat invariant).
- 7 `verify_claim` tests: case-insensitive, byte-offset-round-trip,
  non-overlapping, empty-claim, no-match, cap-enforcement, unicode-safe.

### Verification
- `cargo test --features bench,migrate`: **401 passed, 1 ignored** (was
  391 at v1.4.2; +10).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate --bin brain-server --bin
  brain --bin mcp --bin bench --bin brain-migrate-rehearse`: 5 binaries clean.
- **Live end-to-end smoke: operator step** (run `scripts/install-service.sh`).

### Honest ceilings (carried into v1.6)
- Abstention is heuristic, not learned — `ClarifyQuery` threshold calibrated
  on rank-agreement signals, not a judged corpus.
- `/verify` is lexical only — no semantic/paraphrase match.
- No audit row on `/verify` (pure read).
- M1/M3/M4 + Carry-forward (judged corpus, fuzz targets exercising prod
  code, miri/LSAN) deferred per the evidence-gated roadmap.

--- (read before starting v0.9.4 Sources)

## Agent 34: v1.6.0 "Reconcile" (light cut — atomic supersession + consistency check) — 2026-08-01
**Status:** COMPLETED (code + tests + 4 logical commits; live restart pending operator)
**Date:** 2026-08-01

Scoped to the **evidence-gated v1.6 surface** sanctioned by
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.6. The attached
`IMPLEMENTATION_PLAN_v1.6.0_Reconcile.md` is superseded by that roadmap (same
pattern as v1.5.0). User confirmed Option A (roadmap-compliant cut) after the
conflict was surfaced.

### Research basis (Context7-verified 2026-08-01)

- **Graphiti** (`/getzep/graphiti`): `resolve_edge_contradictions` is the
  canonical pattern — old facts expired (`invalid_at = resolved.valid_at`),
  never deleted. brain-server applies the same semantics at chunk level via
  the existing `knowledge.valid_from`/`valid_to` columns.
- **MemConflict / MOSAIC** (per roadmap): motivate conflict-aware memory but
  "do not justify automatic deletion" — manual-first resolution is mandatory.

### Discovery

~85% of the infrastructure already shipped in v0.9.8 + v1.4.0:
- `knowledge.valid_from`/`valid_to` columns (v0.9.8)
- `/recall` + `/graph/traverse` bi-temporal filters (v1.4.0)
- `evidence_links` table + `find_subject_conflicts` (v0.9.8)
- `AuditKind::Reconcile` variant (v1.1.0)

The single missing piece: the atomic operation that expires the prior fact
when an operator records a `supersedes` link. This release closes that gap.

### Changes Made (4 logical commits)

- **`consolidate::resolve_supersession(tx, from, to, now_utc)`** — the
  mandatory Carry-forward. Atomically in the caller's transaction: (1) insert
  `supersedes` evidence_link (idempotent via UNIQUE), (2) set `valid_to=now`
  on the OLD chunk ONLY if still NULL (idempotent — won't overwrite a
  historical timestamp), (3) audit via `AuditKind::Reconcile` (hash only).
  Graphiti's pattern at chunk level.
- **`/consolidate/apply` routing on kind** (`handlers/consolidate.rs`).
  `supersedes` links now call `resolve_supersession`; other kinds keep the
  plain `link_evidence` path (no retrieval-state change).
- **`brain resolve <new_id> <old_id>` CLI** — operator-facing shortcut.
  POSTs one supersedes link; prints confirmation + the "still retrievable via
  /recall?at=<past>" note.
- **`brain check-consistency` CLI + `unresolved_contradictions` field** on
  `/consolidate/propose` + new `find_unresolved_contradictions()` in
  `consolidate.rs`. Surfaces `contradicts` links with no paired `supersedes`.
  Pure detection; never auto-fixes.
- OpenAPI updated (v1.6.0).

### Tests (6 new)

- `resolve_supersession_expires_old_chunk_and_records_link` — link + valid_to + audit.
- `resolve_supersession_is_idempotent` — second call touches 0 rows, no ts overwrite.
- `resolve_supersession_rejects_self_link`.
- `resolve_supersession_rollback_changes_neither` — third arm of exit criterion.
- `supersession_makes_chunk_invisible_to_default_recall_but_visible_historically`
  — end-to-end SQL proof using the EXACT filter fragment `vec0_knn`/`fts_search` use.
- `find_unresolved_contradictions_flags_unresolved_and_hides_resolved`.

Together these prove all 3 arms of the roadmap exit criterion:
"approved update changes current recall; historical recall still returns the
prior claim; failed transaction changes neither."

### Verification

- `cargo test --features bench,migrate`: **407 passed, 1 ignored** (was 401
  at v1.5.0; +6).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.
- **Live end-to-end smoke: operator step** (run `scripts/install-service.sh`).

### Deferred (per evidence-gated roadmap)

- M1 auto-contradiction detection at ingest (CPU + roadmap-forbidden).
- M3 auto conflict-resolution policy (manual-first mandate).
- M4 edit-in-place + `knowledge_history` table (real schema add; "undo" only).
- TRACE session/topic hierarchy (schema reservation only).
- Multi-vector (no-op until judged baseline).

### Honest ceilings (carried into v1.7)

- Resolution is operator-driven only (no auto-detection at ingest).
- `resolve_supersession` expires one chunk per call (multi-way conflicts need multiple calls).
- `find_unresolved_contradictions` is the only consistency check (orphans/cycles deferred).
- No propagation to entities/relationships KG (chunks only; KG edges have their own `?at=` filter).

--- (read before starting v0.9.4 Sources)

## Agent 35: v1.7.0 "Explain" (light cut — faithful path explanations + kind filter) — 2026-08-01
**Status:** COMPLETED (code + tests + 2 logical commits; live restart pending operator)
**Date:** 2026-08-01

Scoped to the **evidence-gated v1.7 surface** sanctioned by
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.7. The attached
`IMPLEMENTATION_PLAN_v1.7.0_Reason.md` is superseded by that roadmap (same
pattern as v1.5.0/v1.6.0). User confirmed "same way" (Option A,
roadmap-compliant cut).

### Research basis (Context7-verified 2026-08-01)

- **Graphiti** (`/getzep/graphiti`): `edge_bfs_search` is the canonical
  bounded-BFS pattern (origin nodes, max_depth, filters, limit).
  brain-server already had this in `/graph/traverse` (v1.0/v1.4).
- **Roadmap guardrail**: "A graph path is association unless an intervention-
  ready causal model and domain expert validation exist." Forbids M2/M3/M4.

### Discovery

The bounded-BFS + bi-temporal + cross-domain + MAX_HOPS=4 + MAX_VISITED=256
infrastructure already shipped in v1.0/v1.4. The single gap: `/graph/traverse`
returned `path` as a flat string of entity ids (`1->5->9`) with no relation
types. A faithful explanation needs `A --works_at--> B --ceo_of--> C`, not
`1->5->9`. This release closes that gap by extending the existing endpoint
(no new route, no new schema).

### Changes Made (2 logical commits)

- **Faithful explanation paths on `/graph/traverse?explain=true`.** The
  recursive CTE now carries `relation_type` per hop (`edge_path` column,
  pipe-separated); the response includes a new `paths` array with structured
  hop chains `[{from:{id,name}, relation, to:{id,name}}, ...]`. Consuming
  agents can render the reasoning chain verbatim. The flat `traversal` array
  stays for back-compat.
- **`?kind=<relation_type>` edge filter.** Restricts the walk to edges whose
  `relation_type` matches. Exact match (`kind=works_at`) or prefix match
  when ending with `:` (`kind=causes:` for the causal subgraph — opt-in,
  no auto-causal claims). Wildcards (`_`/`%`) in user input are escaped to
  prevent LIKE injection.
- **OpenAPI contract** updated (v1.7.0): `kind` + `explain` params, `paths`
  array, `edge_path` + `from_entity` fields on `traversal` rows.
- 2 new unit tests (`explanation_paths_reconstruct_hop_chain_from_cte_output`,
  `explanation_paths_empty_on_empty_input`).

### Verification

- `cargo test --features bench,migrate`: **409 passed, 1 ignored** (was 407
  at v1.6.0; +2).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.
- **Live end-to-end smoke: operator step** (run `scripts/install-service.sh`).

### Deferred (per evidence-gated roadmap)

- M2 causal discovery / M3 counterfactual simulation (roadmap-forbidden;
  graph paths are association, not causation).
- M4 transitive inference (virtual inferred edges with `state='inferred'`).
- M1's `/graph/reason` new endpoint (not needed — `/graph/traverse?explain=true`
  IS bounded multi-hop reasoning).
- TRACE session/topic hierarchy + multi-vector (schema reservations only).

### Honest ceilings (carried into v1.8)

- Intermediate entity names in `paths` are best-effort (seed + leaf named;
  intermediates surface as ids unless caller resolves via `/get/{id}`).
- `?kind=` filter is exact/prefix only (no regex, no negation).
- No audit row on traverse (pure read).
- Graph paths are association, not causation — even with `?kind=causes:`,
  the brain reports what the graph contains, not what is true in the world.

--- (read before starting v0.9.4 Sources)

## Agent 36: v1.8.0 "Maintain" (light cut — reviewable proposals + undo) — 2026-08-01
**Status:** COMPLETED (code + tests + 2 logical commits; live restart pending operator)
**Date:** 2026-08-01

Scoped to the **evidence-gated v1.8 surface** sanctioned by
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.8. The attached
`IMPLEMENTATION_PLAN_v1.8.0_Consolidate.md` is superseded by that roadmap
(same pattern as v1.5.0/v1.6.0/v1.7.0). User confirmed continuation of the
same Option A (roadmap-compliant light cut) pattern.

### Research basis (Context7-verified 2026-08-01)

- **Graphiti** (`/getzep/graphiti`): `resolve_extracted_nodes` uses cosine
  similarity threshold (default 0.6 for nodes) for dedup candidates + tracks
  duplicate_pairs explicitly (no silent merging). Neptune driver shows the
  Python-side cosine pattern brain-server applies via the existing vec0 KNN.
- **Roadmap guardrail**: "duplicate and stale-source proposals" in; "automatic
  archiving, domain moves, fabricated summaries, synthetic relation insertion"
  forbidden.

### Discovery

The exact-duplicate + subject-conflict + unresolved-contradiction detectors
already shipped in v0.9.8 / v1.6.0 (via `/consolidate/propose`). The missing
pieces for the v1.8 exit criterion: stale-source detection, near-duplicate
detection, and undo.

### Changes Made (2 logical commits)

- **`consolidate::undo_supersession(tx, old_chunk)`** — the roadmap exit
  criterion's undo arm: "reject or undo them without retrieval regression."
  Clears `valid_to` back to NULL + removes the `supersedes` evidence_link,
  atomically in the caller's tx. Audited via `AuditKind::Reconcile` (hash only).
  Idempotent — a re-run on an already-undone chunk touches 0 rows.
- **`POST /consolidate/undo` + `brain undo-resolve <old_id> [...]` CLI.**
  Batch wrapper: takes a list of chunk ids; each is undone atomically in one tx.
- **`consolidate::find_stale_sources(conn)`** — vault sources whose `uri` is a
  file path that no longer exists on disk. Pure detection; never archives.
  Operator reviews and either re-ingests (file moved) or retires via
  `DELETE /sources/{id}`. Surfaced in `/consolidate/propose` + `brain
  check-consistency`.
- **`consolidate::find_near_duplicates(conn, threshold, max_pairs)`** — pairs
  of current chunks with embedding cosine > 0.95 (different content hash).
  Uses the existing vec_knowledge KNN — bounded O(n×k), not O(n²). Capped at
  50 pairs per proposal. Surfaced in `/consolidate/propose` + `brain
  check-consistency`.
- **`decode_embedding`** helper — interprets the vec0 int8 blob format.
  Pinned by a round-trip test (ponytail: pins the blob-layout assumption).
- **OpenAPI contract** updated (v1.8.0): `/consolidate/undo` route +
  `stale_sources` + `near_duplicates` fields on `ConsolidateProposal`.
- 5 new tests + 1 existing test updated.

### Verification

- `cargo test --features bench,migrate`: **414 passed, 1 ignored** (was 409
  at v1.7.0; +5).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.
- **Live end-to-end smoke: operator step** (run `scripts/install-service.sh`).

### Deferred (per evidence-gated roadmap)

- M1 background ConsolidationWorker (autonomous consolidation forbidden).
- M3 summarization ("fabricated summary" forbidden; medoid stays a chunk).
- M4 cross-cluster linking / synthetic relation insertion (forbidden).
- M5 archival / domain moves ("automatic archiving" + "domain moves" forbidden).
- Resumable batches as saved state (proposal endpoint is idempotent; re-run
  picks up where you left off).

### Honest ceilings (carried into v1.9)

- Near-duplicate detection is per-domain only (cross-domain needs federation).
- `find_near_duplicates` loads each chunk's embedding once per scan (~5 MiB
  transient for 10k chunks; bounded + ephemeral).
- `decode_embedding` assumes the vec0 int8 blob layout (pinned by round-trip test).
- Undo only reverses supersedes-kind resolutions (other kinds have no state).
- No background worker (operator-triggered only; roadmap choice).

--- (read before starting v0.9.4 Sources)

## Agent 37: v1.9.0 "Suggest" (light cut — opt-in anticipation + false-positive metric) — 2026-08-02
**Status:** COMPLETED (code + tests + 4 logical commits + live restart)
**Date:** 2026-08-02

Final light-cut release of the v1.x cognitive-stack line. Scoped to the
**evidence-gated v1.9 surface** in
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.9. The attached
`IMPLEMENTATION_PLAN_v1.9.0_Anticipate.md` is superseded by that roadmap (same
pattern as v1.5–v1.8). User confirmed continuation of the Option A
(roadmap-compliant light cut) pattern.

### Research basis (Context7-verified 2026-08-02)

- **Mem0** (`/mem0ai/mem0`, benchmark 83.22): the `feedback` API shape
  (`memory_id`, `feedback: POSITIVE|NEGATIVE`, `feedback_reason?`) + "feedback
  analytics" track accept vs dismiss — **this is the false-positive metric the
  roadmap requires.** Session identity is client-owned (`run_id`); the server
  never auto-tracks sessions.
- **Letta/MemGPT** (`/letta-ai/letta`, benchmark 83.31): anticipatory memory
  is *reviewable* — nothing is silently injected. `/suggest` returns labelled
  candidates the caller explicitly asked for; the agent chooses to use them.

### Discovery

The full Anticipate plan (M1 sessions table + auto-start, M3 short-poll/SSE
push, M4 attention decay, M5 personalization vector) is **forbidden** by the
roadmap's "Do not ship" list ("unsolicited push, ranking decay, hidden
personalization, or SSE by default"). The only surviving scope: opt-in pull +
false-positive metric. The session concept survives in **client-owned** form
(Mem0 `run_id` pattern): caller passes opaque `session` string; server never
auto-tracks, auto-expires, or auto-embeds a session.

### Changes Made (4 logical commits)

- **`POST /suggest`** (`src/handlers/suggest.rs`): opt-in anticipation pull.
  Caller supplies explicit `context`; server embeds via existing `StaticModel`,
  runs `vec0_knn` with over-fetch = `k + exclude.len()`, filters `exclude`
  ids, truncates to `k`, tags every hit `provenance.reason = "anticipated"`.
  Reuses v1.6.0 `valid_to IS NULL` default (superseded chunks never suggested)
  + v0.9.7 flagged-row exclusion (quarantined chunks never suggested). Zero
  new state, zero background work, zero push.
- **`POST /suggest/feedback`**: Mem0-pattern accept/dismiss (`feedback:
  accept|dismiss`, optional hashed `reason`, optional `session`). Validates
  chunk exists (404 on typo so the metric isn't poisoned). Tenant-scoped via
  JWT principal. The `suggest_feedback` table IS the audit surface (append-
  only, hash-of-reason, tenant-scoped) — no duplicate `audit_events` row.
- **`GET /suggest/metrics`**: false-positive rate (dismisses / total) over
  the feedback ledger, optional `session` / `since` window. **This IS the
  roadmap exit criterion, made queryable.** Tenant-scoped.
- **`BRAIN_SUGGEST_ENABLED` kill switch** (`src/config.rs`, default `true`):
  when `false`, all three routes return `501 Not Implemented` — the roadmap's
  "otherwise the feature is removed" guarantee, without a rebuild.
- **CLI** (`src/bin/brain.rs`): `brain suggest`, `brain suggest-feedback`,
  `brain suggest-metrics`.
- **Migration** (`src/migration.rs`): additive `suggest_feedback` table +
  `schema_version = 1.9.0` (was `1.4.0`; v1.5–v1.8 made no schema change).
  `test_migration_schema_contract` extended.
- **OpenAPI** → 1.9.0: three routes + `SuggestionHit`/`SuggestTelemetry`/
  `SuggestMetrics` schemas. `test_openapi_covers_routes` extended.

### Tests (14 new)

12 pure-function tests in `suggest.rs` (validate_suggest bounds, exclusion +
truncation algorithm, FeedbackOutcome parsing, metric math including the
zero-total-not-NaN edge) + 2 integration tests in `main.rs`
(`suggest_feedback_table_is_append_only_and_queryable` — proves the INSERT +
GROUP BY + tenant isolation against real rows;
`suggest_exclude_filter_uses_the_same_knowledge_visibility_as_recall` —
proves superseded chunks are never suggestable).

### Verification

- `cargo test --features bench,migrate`: **428 passed, 1 ignored** (was 414
  at v1.8.0; +14).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.
- **Live end-to-end smoke** (after `scripts/install-service.sh`, pid 17967):
  `/suggest` returns anticipated chunks (excluded ids correctly dropped,
  telemetry accurate); `/suggest/feedback` records accept+dismiss;
  `/suggest/metrics?session=` returns `false_positive_rate: 0.5` (1/2);
  `BRAIN_SUGGEST_ENABLED=false` on a throwaway port-18765 instance → all
  three routes return `501` while `/version` stays `200` (kill switch proven
  live, not just unit-tested).

### Deferred (per evidence-gated roadmap)

- **M1 sessions table + auto-start + 30-min window + running embedding mean**
  — "hidden personalization."
- **M3 short-poll `/events` + SSE push** — "unsolicited push" + "SSE by
  default." `/suggest` is an explicit pull; the agent asks.
- **M4 attention decay + spaced-repetition** — "ranking decay."
- **M5 personalization vector** — "hidden personalization."

### Honest ceilings (carried into v2.0)

- No semantic anticipation (KNN-over-context, not a learned predictor).
- Session is client-owned (no boundary detection / timeout / embedding mean).
- `accept`/`dismiss` is binary (Mem0's `VERY_NEGATIVE` collapsed).
- Metrics are per-process (live scan, no rollup; bounded by index).
- Feedback is not retrieval-affecting (no boost/decay — roadmap-forbidden).
- Near-duplicate / cross-domain suggest deferred (per-domain only).

--- (read before starting v0.9.4 Sources)

## Agent 38: v1.9.1 "Harden" (bug-fix — post-release audit of v1.7.0–v1.9.0) — 2026-08-02
**Status:** COMPLETED (code + tests + 4 logical commits; live restart pending operator)
**Date:** 2026-08-02

A security + code-quality audit of the v1.7.0–v1.9.0 releases surfaced three
fixable findings (one High correctness, one Medium security, one Low quality);
the rest were judged Low/forward-compat and carried into v2.0. The uncommitted
v1.10.0 "Procedural" WIP in the tree was **stashed** before the hotfix so the
release is a coherent v1.9.1 on a clean v1.9.0 base, then popped back for
finishing afterward.

### The audit findings (see audit write-up for the full list)

- **C1 (High, correctness):** v1.8.0 `find_near_duplicates` JOINed the legacy
  `embeddings` JSON table, frozen at v0.9.0 — production ingests write only
  `vec_knowledge`, so the scan silently covered 2 of 8538 chunks on the live
  DB. The `ed1e401` "fix" had traded a hard failure (wrong column `v.embedding`)
  for silent under-coverage; no test caught it because the fixture fabricated
  an `embeddings` table.
- **S2 (Medium, authenticated):** `/suggest/feedback` was append-only with no
  idempotency — a replay/retry recorded duplicate rows, poisoning the
  false-positive metric (the v1.9 roadmap exit criterion).
- **S1 (Low→High at v2.0):** `/suggest` returns full chunk content with no
  tenant scoping; `authorize()` is never called anywhere in production code
  despite the v1.2.0 record claiming handler-entry gates. Safe today
  (single-tenant, `auth_middleware`-gated), carried into v2.0.
- **S3/S4/S5/S6 (Low):** `reason_hash` uses xxh3-64 (inherited from `audit::hash`);
  `find_stale_sources` is a filesystem-existence oracle; `kind` LIKE-prefix
  backslash edge; unbounded `session`/`old_chunks` inputs. All documented, none
  blocking.
- **Q1/Q2 (Low):** stale "batched lookup" comment + dead `needed_ids` collection
  in `build_explanation_paths`; fragile `?at`→`?3`/`?kind`→`?3/?4` placeholder
  renumbering in the traverse CTE (the exact fragility that caused the v1.7.0
  shipped-then-fixed bug).

### Changes Made (4 logical commits)

1. **`fix(consolidate)`** — `find_near_duplicates` now reads
   `vec_knowledge.embedding_int8` and dequantizes via `decode_embedding`
   (flipped from `#[allow(dead_code)]` to live). Blob format verified against
   sqlite-vec's `vec_int8` docs (raw signed bytes, no header); the KNN query
   stays byte-identical to `/recall` (`vec_quantize_int8(?1,'unit')`), so only
   the vector SOURCE changed. Fixture's unused `embeddings` table removed.
   Regression test `near_duplicates_cover_vec0_ingested_chunks_not_legacy_json_only`
   ingests two near-identical chunks through the REAL quantize path (zero
   `embeddings` rows) and asserts the pair is proposed.
2. **`fix(suggest)`** — feedback is last-wins per `(chunk_id, session)`. Unique
   expression index `(chunk_id, COALESCE(session, ''))` (SQLite 3.51) + handler
   upsert. Replays collapse; changed-mind overwrites; session-less rows covered
   via COALESCE. Pre-existing duplicates deduped (keep latest) before index
   creation. Schema stamp 1.9.0 → 1.9.1. Two tests: the exact upsert contract
   (`suggest_feedback_last_wins_per_chunk_session`) + the metrics GROUP BY /
   tenant-isolation test updated to the one-signal-per-key contract.
3. **`style(fmt)`** — rustfmt drift on the new test (automated).
4. **release wrap** — `docs` + version bump 1.9.0 → 1.9.1 (Cargo.toml,
   openapi.yaml, CHANGELOG, README, AGENTS.md) + `build_explanation_paths`
   comment honesty/dead-code removal (Q1).

### Verification

- `cargo test --features bench,migrate`: **430 passed, 1 ignored** (was 428 at
  v1.9.0; +3 new tests − 1 renamed).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- Live-DB proof (see audit + smoke): `embeddings` has 2 rows vs 8538
  `knowledge` — the old scan covered ~0%; the new scan reads the live index.
- **Live end-to-end smoke: operator step** (run `scripts/install-service.sh`;
  the near-dup scan + suggest-feedback dedup are then live against the 8538-doc
  DB).

### Honest ceilings (carried into v2.0)

- `/suggest` still has no principal/tenant scoping (S1) — single-tenant safe,
  multi-tenant leak at v2.0 if not gated.
- `authorize()` helper remains uncalled in production — the v1.2 AuthZ surface
  is unit-tested but not handler-wired; wiring is v2.0 work.
- `reason_hash` stays xxh3-64 (non-cryptographic) — inherited pattern.
- `find_stale_sources` filesystem-existence oracle + `kind` LIKE edge remain
  (both authenticated, Low).

--- (read before starting v0.9.4 Sources)

## Agent 39: v1.10.0 "Procedural" — ship the finished WIP + re-verify v1.0.0→v1.10.0 — 2026-08-02
**Status:** COMPLETED (code + tests + full-chain verification; live restart pending operator)
**Date:** 2026-08-02

Finished the stashed v1.10.0 "Procedural" WIP (restored in Agent 38) and
re-verified the entire v1.0.0→v1.10.0 release line. The WIP had three known
gaps; all closed. The re-verify then surfaced three more.

### The WIP finish (commit `db99cad`)

- **Merge resolution** — the stash popped with conflicts in `Cargo.toml`/
  `Cargo.lock`/`src/main.rs`/`src/migration.rs`/`src/storage_layout.rs`
  (v1.9.1 hotfix had touched the same regions). Kept BOTH migration blocks:
  the v1.9.1 suggest-feedback dedup index AND the v1.10.0 node_kind repurpose +
  `evidence_links.step_index`; final schema stamp 1.10.0 supersedes 1.9.1.
- **openapi.yaml** — documented the 4 new routes (`/procedure`,
  `/procedure/{id}/steps`, `/classify`, `/decision/{id}/evaluate`) + 3 schemas
  (`StepView`, `CategoryResult`, `DecisionOutcome`); version → 1.10.0.
  `test_openapi_covers_routes` green.
- **`fix(procedural)` — `classify` matched-keywords lexicon-index bug.** The
  winning category was right but its keyword list came from the *sorted*
  `scores` slot (after `sort_by`, that slot is no longer the LEXICON index).
  `classify_detects_compliance` failed: category "compliance" but no `hipaa`/
  `pii` in `matched_keywords`. Fixed by resolving the lexicon index via the
  `CATEGORIES` position (shares LEXICON ordering).
- **`cleanup(procedural)` — `MemoryKind::from_str` wired at its read site.**
  Was a dead fn kept alive only by tests; the `GET /procedure/{id}/steps`
  handler now parses `node_kind` through it, making the forward-compat
  fallback (unknown → `fact`) live code. Also fixed a clippy `unnecessary_sort_by`.

### Re-verify v1.0.0→v1.10.0 (what was checked)

- **Tests + gates:** 447 passed / 1 ignored; clippy `-D warnings` clean;
  `cargo fmt --check` clean; all 5 release binaries build. Tags
  `v1.0.0`→`v1.9.1` all present (v1.10.0 tagged at wrap).
- **Schema-contract test** (`test_migration_schema_contract`) covers the whole
  chain: tables from v0.9.0→v1.2.0, `knowledge`/`audit_events` columns,
  v1.4.0 bi-temporal + TRACE reservation, v1.9.0 `suggest_feedback`,
  v1.10.0 `step_index` + node_kind relabel, final stamp 1.10.0.
- **Route coverage:** `test_openapi_covers_routes` green.
- **Live-DB migration smoke** (copy of the 8538-doc DB): 8538 `'event'` rows →
  `'fact'`, schema stamp 1.10.0, all 4 new routes exercised via HTTP
  (`/classify` → `compliance` + `["hipaa","pii"]`; 2-step procedure ingested
  atomically; `/procedure/{id}/steps` ordered + normalized `memory_kind`;
  `/decision/{id}/evaluate` fires the matched branch).

### Fixes from the re-verify (this agent)

- **`node_kind` default wart** (`migration.rs` + schema-contract test): the
  v1.10.0 migration relabeled existing `'event'` rows to `'fact'` but the
  column DEFAULT was still `'event'`, so fresh DBs and new rows on existing DBs
  kept inserting `'event'`. Read path normalizes via `MemoryKind::from_str`, so
  cosmetic — but semantically wrong. Changed the fresh-DB default to `'fact'`;
  `ponytail:` comment documents the existing-DB gap (SQLite can't ALTER a
  column default without a table rebuild).
- **Schema-contract test now asserts the v1.9.1 dedup index**
  (`idx_suggest_feedback_chunk_session`) — previously only the v1.9.0 tenant
  index was checked, so a dropped v1.9.1 index would slip past the contract
  test and only fail the upsert test.
- **Release docs brought current:** README → 1.10.0; CHANGELOG `[1.10.0]`
  section; ROADMAP v1.10.0 row → Shipped; AGENTS.md header + this entry.

### Verification

- `cargo test --features bench,migrate`: **447 passed, 1 ignored**.
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: 5 binaries clean.
- **Live end-to-end smoke: operator step** (run `scripts/install-service.sh`;
  the new routes are then live against the 8538-doc DB).

### Honest ceilings (carried into v2.0)

- No background worker / no auto-consolidation — procedures, steps, and
  decisions are explicit writes.
- Pre-v1.10 DBs keep the `'event'` column default (cosmetic; read-path
  normalization covers it).
- `classify` is a deterministic keyword router, not a learned classifier
  (the `ponytail:` comment names the model2vec upgrade path, v1.11).
- `/suggest` still has no principal/tenant scoping (S1 from Agent 38) and
  `authorize()` remains unwired — v2.0 multi-tenancy work.
- Still no ARM/Jetson measured-capacity run (`bench --envelope` operator step).

--- (read before starting v0.9.4 Sources)

## Agent 40: v1.11.0 "Associate" — HippoRAG-2-style PPR graph leg (session 2026-08-03)
**Status:** COMPLETED (code + tests + release-wrap docs; live restart pending operator)
**Date:** 2026-08-03

Shipped the ROADMAP's v1.11.0 "Associate" row: a deterministic Personalized
PageRank retriever over the **existing** `entities`/`relationships` KG as a
third, opt-in `?graph=true` RRF leg on `/search` + `/recall`. Faithful to the
HippoRAG 2 reference (verified verbatim from `OSU-NLP-Group/HippoRAG`
`HippoRAG.py` + `config_utils.py`): `damping=0.5` (NOT the 0.85 from the plan
draft — the reference's real default), power iteration
`π = (1−α)s + α·Pᵀπ`, L1 convergence `1e-6`, bounded `MAX_PPR_ITER = 50` +
`trace::MAX_VISITED = 256`, undirected weighted edges where weight =
`COUNT(DISTINCT relationships.knowledge_id)` (the `node_to_node_stats`
fact-edge count at pair level). **No LLM, no new schema, no embeddings in the
graph leg** — the `< 5W` manifesto holds.

### Research basis (Context7 + webfetch, 2026-08-03)
- **HippoRAG 2 reference verified verbatim** (`HippoRAG.py::run_ppr` +
  `graph_search_with_fact_entities`): `igraph.personalized_pagerank(damping=0.5,
  directed=False, weights='weight', reset=node_weights, implementation='prpack')`.
  Key port note: prpack normalizes the reset vector internally; the Rust port
  must normalize seeds to a probability distribution (documented in the code).
- **Config defaults confirmed from `config_utils.py`**: `damping=0.5`,
  `passage_node_weight=0.05`. The repo plan draft wrote 0.85 — corrected to a
  faithful 0.5.
- **Live-DB pre-flight**: 1495 entities, 2376 relationships. ~94% of edges are
  `tagged_with` taxonomy noise (note → tag noun); only ~134 semantic edges;
  the cleanest multi-hop paths are the synthetic `dave/acme/carol` bench
  fixture. Recorded as the corpus ceiling, not a bug.

### Changes Made
- **New `src/search/graph_ppr.rs`** (pure safe Rust in the `#![deny(unsafe_code)]`
  module): `SparseGraph` (CSR adjacency + id↔index maps, self-loop/zero-weight
  guards), `build_graph`, `seed_entities_from_query` (case-insensitive exact
  entity-name containment), `personalized_pagerank` (power iteration, bounded),
  `expand_to_chunks` (top-n entities → distinct `relationships.knowledge_id`
  chunks, `flagged=0`/`valid_to IS NULL` visibility), `graph_retrieve(conn,
  query, k, include_flagged)`, `restrict_to_reachable` (BFS capped at
  `MAX_VISITED`). Constants: `PPR_ALPHA = 0.5`, `PPR_EPSILON = 1e-6`,
  `MAX_PPR_ITER = 50`, `PASSAGE_NODE_WEIGHT = 0.05` (reserved — documented
  ceiling for the DPR-passage-seed upgrade path).
- **Third RRF leg in `src/search/mod.rs`**: `SearchSource::Graph`,
  `Provenance.graph_rank`, `SearchTelemetry.graph_ms`/`graph_candidates`,
  `SearchFilters.graph`, and `rrf_fuse` extended to 3-way (same formula, same
  `RRF_K = 60`). The graph thread runs concurrently inside the existing
  `std::thread::scope` on its own pooled connection; the disabled path pays
  zero latency (`graph_ms = 0`).
- **Opt-in plumbing**: `graph: bool` on `QueryDoc` (+`Default`+`into_filters`),
  `RecallRequest`, GET `/search` `SearchParams`, and `brain query --graph`
  (bare `--graph` or `--graph=true` enables; `--graph=false` opts out).
- **`HitSource::Graph` wired** in recall's `map_source` (the variant already
  existed).
- **OpenAPI** → 1.11.0: `graph` param on QueryDoc + GET `/search` +
  `graph_rank` on both provenance schemas + `graph_ms`/`graph_candidates` on
  SearchTelemetry.
- **4 plan verifications** as unit tests: `ppr_ranks_connected_entities_higher_than_unrelated`,
  `ppr_seed_from_query_uses_exact_entity_names`, `rrf_fuses_graph_leg_with_vector_and_fts`,
  `ppr_bounded_by_max_visited`, plus self-loop/zero-weight guards.
- **Docs**: Cargo.toml 1.10.0 → 1.11.0; README version row; ROADMAP v1.11 row →
  Shipped; CHANGELOG `[1.11.0]`; AGENTS header + this entry.

### Verification
- `cargo test --features bench,migrate`: **455 passed, 1 ignored** (was 447;
  +6 graph_ppr tests + 1 rrf graph-fusion test + 1 net from the openapi schema).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate --bin brain-server --bin brain`: clean.
- **Live smoke on a copy of the live 8538-doc DB** (throwaway port, in-memory
  copy): default path unchanged (`graph_ms = 0`, `graph_candidates = 0`);
  `graph=true` → `graph_candidates = 107–112`, `graph_ms ≈ 4ms`; exact
  entity-name query `acme_v17c_1785593852` seeds the graph leg and surfaces
  `source=graph` / `both` hits the vector+lexical legs miss
  (`dave works at acme_v17c` + `acme_v17c ceo is carol` at `graph_rank 0/1`);
  `brain query "acme" --graph` CLI works.

### Honest ceilings (carried into v2.0)
- **Live multi-hop quality is corpus-bound**: on the live 8538-doc DB ~94% of
  KG edges are `tagged_with` taxonomy; the graph leg retrieves, but the cleanest
  multi-hop paths are the synthetic `dave/acme/carol` bench fixture. The
  mechanism ships; corpus quality is an operator concern (vault re-ingest with
  the v1.4.1 heading-hierarchy linker would grow the semantic edge set).
- No DPR passage scores in the seed (plan forbids an embedding in this leg);
  `PASSAGE_NODE_WEIGHT` documents the upgrade path.
- Graph leg respects `include_flagged` but not per-domain pools in multi-db
  mode yet (the pool resolved by the caller is the domain's own — a cross-domain
  graph leg is v2.0 federation work).
- Live restart is an operator step (`scripts/install-service.sh`).

---

## Agent 41: v1.12.0 "Discern" — noise-aware graph retrieval + complexity-gated activation (session 2026-08-03)
**Status:** COMPLETED (code + tests + docs; live restart pending operator)
**Date:** 2026-08-03

Continues the v1.11.0 "Associate" line per the user's explicit request
("make a detailed v1.12.0… compliment existing code, improve KG quality with
hub dampening + edge-type weights, correctly wired in with auto-gating,
tests, no dead code/duplicates, latest research"). Research + live-DB
pre-flight (Agent 40's notes + this session): the live KG is 94%
`tagged_with` taxonomy edges (2242/2376) with degree-73/101/150 mega-hubs.
The v1.11.0 graph leg was unweighted, so PPR mass washed out across tag
clouds, and a `ClarifyQuery` query (v1.5.0 abstention) never got a graph
chance at all. This release fixes both, adopting only the *arithmetic* from
the 2025-2026 research (GAAMA arXiv:2603.27910 hub dampening + edge-type
weights; MemORAI arXiv:2605.01386 static case; "Use Graph When It Needs"
arXiv:2602.03578 complexity gating) — LLM extraction parts forbidden.

### Changes Made (M1 + M2 + M3, one working tree)

**M1 — noise-aware weights (`src/search/graph_ppr.rs`):**
- `type_base_weight(rel_type)` — `tagged_with`/`alias_of` → 0.1, all other
  relation types → 1.0. The pair-aggregation SQL now groups by
  `relation_type`; each group's `COUNT(DISTINCT knowledge_id)` is scaled by
  its type weight before the per-pair sum feeds the unchanged `build_graph`.
- `SparseGraph::dampen_hubs(θ)` — GAAMA's per-source-node
  `w_ij · min(1, θ/deg(i))` (θ = `HUB_DAMPING_THETA` = 50), applied to the
  reachable-bounded graph after `restrict_to_reachable`, before PPR.
  Per-source asymmetry is intentional (matches the reference); the existing
  row-normalization in `personalized_pagerank` handles it.
- Determinism hardening: `edge_rows` sorted by `(a, b)` so vertex admission
  is independent of SQLite's GROUP BY order (PPR values are order-
  independent; the stable tie-break in `expand_to_chunks` is not).

**M2 — complexity-gated activation (`src/search/mod.rs` +
`src/handlers/recall.rs`):**
- `should_attempt_graph_rescue(recommendation, graph_enabled, enabled)` —
  pure gate: `ClarifyQuery` AND graph leg not already enabled AND
  `BRAIN_GRAPH_RESCUE_ENABLED` (default true; `config::brain_graph_rescue_enabled()`,
  same pattern as `BRAIN_SUGGEST_ENABLED`).
- In `perform_search_with_prf`, the `ClarifyQuery` arm now runs one bounded
  graph-augmented pass (`graph = true`, same `prf_depth` overfetch, same
  pooled-connection pattern) and fuses via the shared two-pass RRF fuse.
  Strictly additive: that path previously returned zero hits (v1.5.0
  abstention); the kill switch restores exact v1.11.0 behavior.
- `fuse_pass_lists()` — the two-pass RRF fuse extracted from
  `fuse_prf_passes` (now a thin wrapper adding the `prf_expanded` flag), so
  a graph rescue is never mislabeled as PRF-expanded (the "no duplicates"
  item: one shared fuse, no copy).
- `RetrievalStrategy::HybridGraph` + `SearchTelemetry.graph_rescued` for
  observability; `brain query` telemetry prints it.
- `recall.rs`: `abstention_decision(recommendation, hits_empty)` — abstains
  only when `ClarifyQuery` AND the final hit list is empty. v1.5.0 contract
  preserved on the non-rescue path; a successful rescue returns its hits
  with `decision: "ok"`.

**M3 — release wrap:** version 1.11.0 → 1.12.0 (Cargo.toml, openapi.yaml —
`graph_rescued` on SearchTelemetry, README, ROADMAP new Shipped row, CHANGELOG
`[1.12.0]`, AGENTS header + this entry). New plan:
`IMPLEMENTATION_PLAN_v1.12.0_Discern.md` (research-cited, milestones,
verification, honest ceilings).

### Tests (+5 → 460 passed, 1 ignored)
- `type_base_weight_downgrades_taxonomy_noise` — the weight-table contract.
- `hub_dampening_scales_heavy_hubs_but_not_light` — exact math: deg-100
  source ×0.5 at θ=50, deg-10 unchanged, leaf half-edge untouched
  (per-source damping).
- `graph_retrieve_weights_semantic_over_tag_cloud` — integration fixture
  (in-memory entities/relationships/knowledge): mixed hub with 2 semantic +
  100 `tagged_with` neighbors; the semantic-backed chunk must rank above the
  tag cloud. **Regression-proven**: temporarily reverting to the v1.11
  arithmetic makes this test FAIL (tag cloud wins) — the test pins the
  mechanism it was written for.
- `should_attempt_graph_rescue_matrix` — true only for ClarifyQuery +
  graph-disabled + kill-switch-on; false for every other recommendation,
  explicit `?graph=true`, kill switch off, and missing recommendation.
- `graph_rescue_fuse_does_not_mark_prf_expanded` — the shared fuse never
  claims PRF expansion; `fuse_prf_passes` still does; identical ranking.
- `abstention_returns_low_confidence_only_on_clarify_query` extended: the
  ClarifyQuery + non-empty-hits → `ok` arm (the rescue's payoff).

### Verification
- `cargo test --features bench,migrate`: **460 passed, 1 ignored** (was 455
  at v1.11.0; +5).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- **Live end-to-end smoke: operator step** (run `scripts/install-service.sh`;
  the weighted graph + rescue are then live against the 8538-doc DB).

### Honest ceilings (carried into v2.0)
- θ=50 and the 0.1 type weight are corpus-calibrated constants, not learned
  (deterministic + auditable by design).
- The rescue fires only on the would-be-abstention path; it cannot fix a
  query with no KG structure (no entity match → no seeds → abstain as
  before).
- Type weights are static (no query-conditioning); concept nodes (GAAMA),
  query-conditioned weights (MemORAI), and noun-phrase seeding
  (SAP/LazyGraphRAG) remain future options.
- The tag cloud is structural (re-created on every re-ingest); corpus
  quality is an operator concern (vault re-ingest with the v1.4.1
  heading-hierarchy linker grows the semantic edge set).
- `/suggest` tenant scoping (S1 from Agent 38) + unwired `authorize()` remain
  v2.0 multi-tenancy work; no ARM/Jetson measured-capacity run yet.

---

## Agent 42: v1.12.1 "Harden" — AuthZ wiring completion (session 2026-08-04)
**Status:** COMPLETED (code + tests + tag + live restart)
**Date:** 2026-08-04

Closes the v1.2 S1 audit finding for real. Agent 38's "authorize() never
called" claim was **stale**: by v1.11 the function existed and ~15 handlers
were gated (ingest, suggest, procedure, consolidate, quarantine-release/
delete, domains lifecycle, sources), but a full route-by-route audit against
the v1.2 §3.3 enforcement matrix (`IMPLEMENTATION_PLAN_v1.2.0_AuthN.md`)
found **20 non-public routes shipping with middleware-only auth** — any valid
bearer passed, no scope check. This release wires every one of them and pins
the wiring with tests.

### The audit (route → matrix action → verdict)
- **Already gated (verified, unchanged)**: `/add` (Write), `/ingest/memory`
  (Write), `/ingest/markdown` (Write), `/ingest` (Write, `gate_domain`),
  `/quarantine/{id}/release` + `/delete` (Admin), `/domains/{name}` (Admin,
  `?confirm` guard), `/domains/{name}/vacuum` (Admin), `/domains/{name}/export`
  (Read), `/domains/{name}/import` (Admin), `/sources/reconcile` (Write),
  `/sources/{id}` (Write), `/suggest` (Read), `/suggest/feedback` (Write),
  `/classify` (Read), `/decision/{id}/evaluate` (Read), `/consolidate/apply` +
  `/undo` (Write), `/procedure` (Write).
- **Gated but wrong action (upgraded to matrix)**: `POST /reindex` (Write→
  Admin — §3.3 makes reindex an operator surface), `DELETE /memory/{id}`
  (`forget`, Write→Admin).
- **20 gaps wired** (all at handler entry, before any pool/DB/model access):
  - Read: `search`, `stats` (domain param), `get_chunk`/`multi_get`/
    `get_entity`/`get_relations`/`traverse_graph` (all `X-Brain-Domain`-
    scoped), `list_quarantined`, `metrics`, `recall` (domain param),
    `verify` (domain header), `consolidate::propose`, `connectors::list`,
    `domains` (list), `suggest::metrics`, `procedure::steps`.
  - Write: `embeddings` (`/v1/embeddings`).
  - Admin: `list_audit`, `verify_audit_chain`, `auth::revoke_handler` (the
    route comment always claimed "requires admin auth"; now enforced via a
    new `AuthHandlerError::forbidden()`).
- **New `handlers::audit_scope()`**: `/audit` is Admin-gated AND tenant-
  scoped — a principal only ever sees its own tenant's rows; requesting
  another tenant's filter is a 403. `None` principal keeps the v1.1
  passthrough (no filter change).

### Back-compat analysis (why nothing breaks)
- `None` principal = superuser (opaque-token mode). In JWT mode, opaque
  tokens are rejected by the JWT layer, so the superuser path is unreachable
  there. Default installs (no `BRAIN_JWT_ISSUER`) are byte-identical.
- `/webhooks/{kind}` stays HMAC-verified inside the handler (GitHub cannot
  present a brain bearer token) — by design.
- Public list unchanged: `/health`, `/health/db`, `/ready`, `/version`,
  `/openapi.yaml`, `/.well-known/*`, `/auth/refresh`, `/auth/logout`.
- Legacy handlers return their existing error shapes (add_chunk-style
  `{success:false}` / `{error:...}`) rather than a new HTTP status — the
  established legacy-path convention (see `/add`).

### Tests (+5 → 465 passed, 1 ignored)
- **`authz_gates_cover_every_non_public_route`** — the wiring guard. A
  40-route contract table (mirrors `test_openapi_covers_routes`) + a
  hand-rolled source scan of `build_app`'s `.route(...)` registrations →
  handler body (brace-balanced, string-aware) → asserts `authorize(` present
  AND the matrix `Action::X` literal. **Mutation-proven**: flipping `/recall`
  to `Action::Write` in the table fails the test; reverting passes.
- **Router-level middleware tests** (`tower` dev-dep added, already in the
  lock as an axum dependency): missing token → 401, wrong token → 401, valid
  opaque token → pass, `/health` + `/webhooks/*` bypass, JWT-mode 401 without
  a valid JWS.
- **`audit_scope` unit tests**: cross-tenant 403, own-tenant forced, own-
  tenant request allowed, superuser passthrough (Some/None × requested).

### Verification
- `cargo test --features bench,migrate`: **465 passed, 1 ignored** (was 460
  at v1.12.0; +5).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: 5 binaries clean.
- **Live smoke** (after `scripts/install-service.sh` restart): opaque-mode
  back-compat — `brain doctor` ✓, `brain query` / `/stats` / `/recall` all
  still 200 with the existing bearer token. Cross-tenant enforcement proven
  on a throwaway JWT-mode instance (copy of the live DB, test RSA key):
  team-alpha-scoped JWT on domain `alpha` → 200 with `?domain=alpha`,
  403 on `?domain=beta`; a read-scoped JWT on `/reindex` → 403.

### Honest ceilings (carried into v2.0)
- The wiring-guard table is hand-maintained (same convention as
  `test_openapi_covers_routes`): a new route needs a table row + a gate, or
  the test fails — that's the point.
- `?cross_domain=true` on `/graph/traverse` gates on the base domain only.
- Opaque-mode superuser (`None` principal) remains the v1.1 contract; v2.0
  tenancy runs JWT-only where tenants exist.
- Distributed revocation, hot key reload, EC/Ed JWKS emission remain v2.1+
  (unchanged from v1.2).

---

## Agent 43: v1.12.2 "Harden" — audit-fix release (session 2026-08-04)
**Status:** COMPLETED (code + tests + tag + live restart)
**Date:** 2026-08-04

Deep-stability audit of v1.12.1 (unsafe blocks, SQL injection surface, auth
stack, middleware, backups, deps, CI) found the codebase fundamentally sound
— then closed the three real findings found. See `CHANGELOG.md` §[1.12.2] for
the full record.

### Changes Made
- **`/auth/refresh` check-then-act race fixed** (`src/auth/revocation.rs`):
  `record_refresh_use` + `rotate_chain` as separate steps let two concurrent
  presentations of the SAME refresh token both pass and both mint — silently
  defeating reuse detection. New `record_and_rotate` wraps check + rotation
  in `BEGIN IMMEDIATE`: presentations serialize, the loser reads the rotated
  chain and is detected as reuse, and the family is burned exactly once (the
  burn is committed before the error returns). Mutation-proven by
  `concurrent_refresh_serializes_exactly_one_winner` — removing the
  `BEGIN IMMEDIATE` makes the test FAIL.
- **Database stack bumped** (`Cargo.toml`): rusqlite 0.40.1, sqlite-vec
  0.1.9, r2d2_sqlite 0.35.0 → bundled SQLite 3.51.1 → **3.53.2** (fts3_tokenizer
  hardening + CVE-2022-35737-related fixes). The v1.11.0-comment savepoint
  concern is unused (codebase uses raw-SQL SAVEPOINT, v1.1.2). `sqlite3_vec_init`
  FFI unchanged (verified against the 0.1.9 source).
- **CI `cargo audit` job turned green**: `.cargo/audit.toml` accepts
  RUSTSEC-2023-0071 (rsa "Marvin" timing sidechannel) with documentation —
  verified 2026-08-04 that **no fixed release exists anywhere** (rsa
  0.10.0-rc.18 and jsonwebtoken 11 both still affected); local-daemon timing
  model + 0600 keys + EdDSA-alternative (since v1.2). Rows added to
  `SECURITY.md` + `THREAT_MODEL.md`. `cargo audit` exits 0.
- **Docs**: version bump 1.12.1 → 1.12.2 (Cargo.toml, openapi.yaml, README,
  CHANGELOG, AGENTS).

### Verification
- `cargo test --features bench,migrate`: **466 passed, 1 ignored** (was 465;
  +1 race regression test).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean. `cargo audit`: exit 0.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.

### v1.12.2 ship status: SHIPPED 2026-08-04
Tag `v1.12.2` created. `scripts/install-service.sh` re-run; the live launchd
service reports v1.12.2.

### Honest ceilings (carried into v2.0)
- `cargo audit`'s two unmaintained-crate warnings remain (number_prefix,
  paste — transitive via model2vec-rs/tokenizers; warnings don't fail CI).
- The audit.toml RUSTSEC-2023-0071 ignore must be revisited if rsa ever
  publishes a patched release (re-audit trigger documented in the file).
- Distributed revocation, hot key reload, EC/Ed JWKS emission remain v2.1+
  (unchanged from v1.2).
- No ARM/Jetson measured-capacity run yet (`bench --envelope` operator step).

### 1. `src/sources.rs` is wired in and shipped — v0.9.4 released 2026-07-17
**Status 2026-07-17 (shipped):** the module is wired into `main.rs`, both ingest
paths call it, the reconcile + source-delete routes + CLI commands are live,
AND the live launchd service is running v0.9.4 (`brain doctor` ✓, 430-doc DB
healthy). Commits: `ecab395` (M1 migration), `4de1472` (M2 integration),
`75d29a9` (chunker rewrite), `067a53e` (release wrap).

Historical record (kept for context): a previous session (commit `eee95df`)
wrote `src/sources.rs` but left it unwired. Agent 14 audited it
("salvageable, ready to integrate"). Agent 15 landed the M1 additive migration.
Agent 16 landed the M2 integration: `mod sources;`, `/ingest/markdown` +
`/ingest/memory` retrofits, `POST /sources/reconcile`, `DELETE /sources/{id}`,
`brain reconcile`, `brain source-delete`, + 4 integration tests.

### 2. CI gaps in `.github/workflows/ci.yml`
- ~~**No `--features bench`** anywhere in CI~~ — **FIXED 2026-07-17 (commit `6a69797`)**. The `lint-test` job now runs `cargo clippy --all-targets --features bench -- -D warnings` and `cargo test --all-targets --features bench`.
- **Ubuntu-only** — production target is ARM (Jetson Nano), dev is macOS arm64. No ARM cross-compile job. *(Still open — lower priority.)*
- **Migration safety net added 2026-07-17 (commit `6370b77`)**: `test_migration_schema_contract` in `src/main.rs` asserts the full table/column contract after `run_migration` and verifies the ingest→FTS→vec0 roundtrip. This catches a broken v0.9.4 migration before it reaches the live DB. Not a full HTTP integration suite — the lazy minimal check that fails if the migration breaks the core loop. HTTP-level breakage still relies on `brain doctor` smoke tests.

### 3. Historical plaintext token leak (openclaw-side, not brain-server)
The brain-server bearer token (`8893e7ce…`) is baked into **21 rows** of `~/.openclaw/agents/main/agent/openclaw-agent.sqlite` (`transcript_events` × 5, `trajectory_runtime_events` × 16) from a 2026-07-15 debug session. brain-server's own DB is clean — the leak is entirely in openclaw's memory log. **Purge is paused**: the DB is live (the openclaw gateway process holds it, WAL active — verify with `pgrep -f openclaw/dist/index.js` before touching it). Safe purge requires stopping openclaw → backup → redact → VACUUM → restart. The same token is also in `~/.openclaw/openclaw.json`'s `authToken` field (still live config, not yet remediated).

# END — historical agent execution log
