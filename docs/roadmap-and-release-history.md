# Roadmap & Release History

Brain Server ships on a strict linear release chain. This page is the roadmap summary and the release history. The authoritative version of both lives in `ROADMAP.md` and `CHANGELOG.md` in the repository.

## Current status

- **Latest server version:** 1.28.65 "Meridian" (2026-09-07) — content hygiene
  across the model seam, shipped across three trees the same day: `/suggest`
  joins the untrusted-evidence contract (`untrusted: true` on every hit,
  recall/search parity); the openclaw plugin's invisible-Unicode strip is
  pinned to the server's canonical set by a cross-tree drift fixture; the
  openclaw host strips smuggled Unicode + neutralizes forged host markers at
  the one plugin-merge seam; MCP tool results ride the untrusted-content
  envelope. The line's first live end-to-end proof (poisoned memory → real
  recall → host merge → composed prompt, forgeries absent) is retained in
  `docs/MERIDIAN_PROOF_20260907.md`.
- **Latest client version:** 1.28.23 — ships alongside the server.
- **Latest plugin version:** 0.5.1 (2026-09-07) — the Meridian strip-set
  parity sync; rides brain-server v1.28.14 and later.
- **Active line:** the SEAM LINE (v1.28.63 → v1.28.69, one theme per release,
  closing every code-closeable finding of the 2026-09-06 joint
  brain-server × openclaw security audit) followed by the REGISTER LINE
  (v1.28.70 → v1.28.75). Next releases: .66 Truthglass (the approver sees
  the truth), .67 Pin (tool + signer identity pinned), .68 Shutter (image +
  beacon egress), .69 Deadbolt (egress + process boundary).
- **v2.0.0 "Cortex"** (multi-team tenancy) remains the first externally-pilotable release — it consumes the v1.2 AuthN/AuthZ foundation.

## The release line (v0.9 → v1.17)

| Release | Name | What shipped |
|---|---|---|
| v0.9.1 | Recall | Hybrid retrieval (vector + FTS + RRF), PRF expansion, provenance |
| v0.9.2 | Connect | Obsidian vault ingestion |
| v0.9.4 | Sources | Source lifecycle + reconcile |
| v0.9.5 | Inspect | Structured query contract + evidence |
| v0.9.6 | Bridge | Connectors + GitHub backfill |
| v0.9.9 | Qualify | Capacity envelopes + migration rehearsal |
| v1.0.0 | Domains | Multi-domain foundation |
| v1.1.x | Harden | Audit chain fixes + constant-time hardening |
| v1.2.0 | AuthN | JWT/JWS + OIDC/JWKS + AuthZ |
| v1.3.0 | Bedrock | Memory-safety hardening |
| v1.4.0 | Calibrate | Bi-temporal edges + submodular packing + TRACE + eval harness |
| v1.4.1 | Link | Deterministic entity linker upgrade |
| v1.5.0 | Epistemic | Calibrated abstention + span verification |
| v1.6.0 | Reconcile | Atomic supersession + consistency check |
| v1.7.0 | Explain | Faithful path explanations |
| v1.8.0 | Maintain | Reviewable proposals + undo |
| v1.9.0 | Suggest | Opt-in anticipation + false-positive metric |
| v1.9.1 | Harden | Bug-fix audit |
| v1.10.0 | Procedural | Ordered procedures + classification + decision rules |
| v1.11.0 | Associate | HippoRAG-2-style PPR graph leg |
| v1.12.x | Discern / Harden | Noise-aware graph retrieval + AuthZ wiring |
| v1.13.x | Route / Recall-fix | Domain routing + routing hotfix |
| v1.14.0 | Gate | Human-in-the-loop write-back + trust surfaces |
| v1.15.0 | Observe | Read-event audit + recall trace + DSAR + COMPLIANCE.md |
| v1.16.0 | Client | The Dioxus control surface (web + desktop + mobile) |
| v1.16.1–1.16.8 | Serve / Styled / Secure / Mobile / Integrated / Global | Serving + CSP, design-system restyle, JWT lifecycle, responsive UX, deep links + PWA, i18n + themes |
| v1.17.0 | Mobile | Portable refresh + deep links + offline connect + store readiness |
| v1.17.1 | Govern | Per-kind retention + Art 30 + UMP wire adapter + eval ship-gate |
| v1.17.3 | UMP Rollout | Full UMP 1.0 conformance through L3 (HTTP ops + MCP tools + file binding + identity/capability tokens) |
| v1.17.4 | UMP Conformance | Reference-suite wire fixes (did:key + integrity block) → L3 |
| v1.17.5 | Eval Fix | `brain eval` revived + Round-21 CI gates + SBOM |
| v1.17.6 | Complete 1/3 | Command palette v2 + Overview home |
| v1.17.7 | Complete 2/3 | Graph panel + Create workspace |
| v1.17.8 | Complete 3/3 | Data & Rights + UMP + System panels + Try-it console |
| v1.18.0 | Compliant | `?` keyboard help on Review (WCAG 3.2.6) + a `client-gate` CI job |
| v1.18.1 | Harden | Console history persists (secret-safe) + measured client bundle |
| v1.18.2 | Transparency | Art 50 `knowledge.origin` marker + `/export` provenance |
| v1.19.0 | Integrated | Audit filters URL-addressable; deep links, PWA, JWT-pair SSO-half |
| v1.20.x | Polish → Vault | Client polish + offline queue; the v1.14→v1.20 client chain closes; `pii_map` vault removed (read-time redaction is the control) |
| v1.21.0 | Profiles | Preset knob bundles + `brain setup` + profile-bound retention/PII |
| v1.22.0 | Regulated | Legal hold + retention report + region pin + compliance pack |
| v1.23.0 | Roles | Role-based UI posture + role presets (`client-auditor`, `bpo-ops`) |
| v1.24.0 | Connectors | Profile-gated connector registry + translate template |
| v1.25.0 | PH-Compliant | Breach-notification workflow + PIA + scraping provenance |
| v1.26.x | Cross-Border | Transfer register + jurisdiction rules + TIA/DPA templates |
| v1.27.x | Harden/Console/Review | Fail-closed erasure + fence forgeability, backup v3, console `--json`, i18n truth, client reviewer calibration, silent-failure sweep, recall-cost + PRF weights, client console dashboard, edge supersession + history (1.27.22 "Cascade") |

## The 1.28 harness → conformance lines (v1.28.15 → v1.28.35)

| Release | Name | What shipped |
|---|---|---|
| 1.28.15 | FirstLight | The governed loop runs for real — the steward-harness stub becomes the engine; the AskHuman gate closes |
| 1.28.16 | Anvil | Every engine tool-effect crosses one mediated, countable, auditable hostcall door (`exec`/`http`/`events`/`ui`) |
| 1.28.17 | Settle | Settlement guarantees as contract tests: budget fails closed before any handler runs, cancel settles between steps, resumed runs keep exactly-once event keys |
| 1.28.18 | Lineage | Events remember where they came from: `parent_id` ancestry, checkpoints become events, rewind branches instead of deleting, the I-PASS handoff packet endpoint |
| 1.28.19 | Witness | Client attestation: per-plugin mount evidence with the Anchor-signed boot manifest; persistent reconnecting SSE; MCP Streamable HTTP/SSE transport |
| 1.28.20 | Cockpit | Desktop + mobile become cargo features of one client codebase; transcript renderers, evidence view, lineage timeline, scoreboard panel |
| 1.28.21 | Fathom | Virtual unlimited context: one run per case end-to-end, deterministic context-window derivation, keyset transcript windowing, resumable event stream |
| 1.28.22 | Bridges | CRM intake: Zendesk/Salesforce/Genesys Cloud case bodies flow through the HITL gate and open governed `support-case` runs (`crm_cases` linkage) |
| 1.28.23 | Evolve | The KCS loop closes: article lifecycle states on knowledge rows, case↔article linkage, capture fires when a case closes solved |
| 1.28.24 | Beacon | Approved articles publish as a generated static public KB (`brain kb build`) behind the strict public seam; KB deflection feedback |
| 1.28.25 | Watchbill | Follow-the-sun shifts: pure time-table ring arithmetic — which site owns the queue, derived handover overlap windows |
| 1.28.26 | Crew | Presence roster without a background worker: TTL decay at read time, shift/role/skills badges, proposal-gated skills tags |
| 1.28.27 | Relay | The one-click handover: offer/accept/decline over the I-PASS packet; incomplete packets refuse loudly naming what's missing |
| 1.28.28 | Channel | The case gets a room: screened, case-scoped human notes on the same lineage; `@skill:`/`@principal` mentions become swarm invites |
| 1.28.29 | Mesh | Agents as named colleagues: signed Agent Cards re-verified at use, agent→agent delegation as lineage events, working-set arbiter |
| 1.28.30 | Parcels | Signed site-to-site knowledge parcels: export approved-only rows, verify-before-write import landing as proposals, ledger chained into audit |
| 1.28.31 | Charter | The conformance pack (G1–G10): complaint ack/response clocks as policy stamps, normative metrics dictionary, WCAG 2.2 AA CI gate |
| 1.28.32 | Frontdesk | One intake for every post-sale worktype: 13 intent classes, worktype policy rows, entitlement vocabulary |
| 1.28.33 | Returns | Aftersales dispositions: deterministic return/RMA ranker citing its basis, GPSR recall mode, returnless/fraud KPIs |
| 1.28.34 | Goodwill | The full ISO 10002/10003 complaint lifecycle: lineage-event state machine, HITL remedy matrix with escalating approval caps, national-body ADR packet, goodwill ledger |
| 1.28.35 | Outreach | Consent-first proactive care: hashed-subject consent registry, per-recipient-gated campaign proposals (export-only), Order-of-Care follow-up, ISO 10004 VoC scoreboard fields |

## The post-sale → seam lines (v1.28.36 → v1.28.65)

| Release | Name | What shipped |
|---|---|---|
| 1.28.36 | Keystone | Public case-status pages (unguessable refs, fixed vocabulary), governed multilingual KB, the counted re-ask |
| 1.28.37 | Advocate | The whole ISO 10002 complaint lifecycle on shipped machinery — the register IS the audit chain; public `how-to-complain` page; audited ack SLA |
| 1.28.38 | Lexicon | The normative metric dictionary (G2) — metrics defined once, cited everywhere |
| 1.28.39 | Access | WCAG 2.2 AA as hard release gates over the console (the six 2.2-new criteria), logical-property RTL mirroring, pseudolocale budgets |
| 1.28.40 | Handshake | The versioned WFM seam (`wfm/1`, additive-only, `brain wfm-import`) + workload/coverage views (alert, never reassign) |
| 1.28.41 | Terrain | Tested tier profiles (t1–t4 checked in, CI-booted) + the T1–T4 deployment guide — the Conformance Line closes |
| 1.28.42 | Valet | The personal AI assistant, dogfooded: consent-gated, metadata-only reminders riding the governed loop |
| 1.28.43 | Switchboard | The channel bridge framework (`/webhooks/channel/{kind}` + `/drain`, Standard-Webhooks HMAC); `channel_threads`; Signal promoted first-class |
| 1.28.44 | Caravel | WhatsApp for Business as a governed edge: template + consent + approved proposal ALL THREE for business-initiated contact; the 24h window binds kernel-side |
| 1.28.45 | Herald | Slack + Teams as operator annexes: proposals render as Blocks/Cards with digest-bound approve actions (bridge refuses, kernel re-verifies) |
| 1.28.46 | Plumb | The Foundation Line opens: the service layer (`src/service/`) + the SQL debt lock; zero product surface by design |
| 1.28.47 | Quarry | Rights-surface service cores extracted (DSAR/legal-hold/UMP ops) |
| 1.28.48 | Masonry | Lifecycle-surface service cores (kcs articles, procedures, consolidate) |
| 1.28.49 | Terrace | Register-surface service cores (clients, profiles, connectors) |
| 1.28.50 | Aqueduct | Retrieval-surface service cores (recall/search/suggest) |
| 1.28.51 | Confluence | The long tail: sixteen handler files drained to zero embedded SQL |
| 1.28.52 | Cornerstone | The Foundation Line closes: zero SQL in handlers MACHINE-ENFORCED (no allowlist) |
| 1.28.53 | Triage | The review queue is domain-scoped for real — rows carry domains, CAS re-checks the row's domain |
| 1.28.54 | Scaffold | The Spire Line opens: the thin-binary ledger (ceilings frozen over main.rs), guard tables as data |
| 1.28.55 | Buttress | Pre-main library code promoted with its pins (bootstrap/helpers come home) |
| 1.28.56 | Vaulting | The lib flip: bootstrap + router decomposition; route registrations live only under `server/router/**` |
| 1.28.57 | Capstone | main.rs pinned ≤ 300 lines of wiring, machine-checked — the Spire Line closes |
| 1.28.58 | Throughput | Concurrent truth (BENCH_CLIENTS fan-out, same-seed determinism), contention gauges, the compliance calendar as code — the Enterprise Line opens |
| 1.28.59 | Headroom | Durability policy explicit + echoed, lock-wait telemetry, the write-discipline ratchet |
| 1.28.60 | Loom | Opt-in CPU parallelism (rayon), determinism-proven: byte-identical vec index across loom/serial postures |
| 1.28.61 | Standby | Warm standby (encrypted follower, signed manifest, rehearsed promote with measured RTO/RPO) + the seven CodeQL alerts closed |
| 1.28.62 | Attestation | Claim-bound provenance marks (AI Act Art 50 posture), the principal kill-switch, approval-fatigue telemetry, the crypto inventory — the Enterprise Line closes |
| 1.28.63 | Wardline | Reserved vocabulary at the workflow input seam: kernel-only outbox topics, closed run statuses, the valet fence — the SEAM LINE opens (the only code-false security law in repo history, made true) |
| 1.28.64 | Blackout | Revocation at the authentication seam (`401 identity_revoked` everywhere), denylist real-exp, per-kid alg pinning, one public-path list + the reverse-direction guard |
| 1.28.65 | Meridian | Content hygiene across the model seam, three trees: `/suggest` untrusted labels, plugin strip-set parity fixture, the openclaw merge-seam strip/neutralize, MCP results in the untrusted envelope; the line's first live end-to-end proof |
| 1.28.66 | Truthglass | Approvals carry effective tool-call args; head-and-tail truncation with exact counts; DSAR/restore prompts |
| 1.28.67 | Pin | MCP catalog sha256 pins + per-run reconcile; `BRAIN_MCP_SCOPE`; parcel `expected_signer` required |
| 1.28.68 | Shutter | Image + beacon egress closed (fork gates; docs posture here) |
| 1.28.69 | Deadbolt | Egress resolve-validate-pin; private-sink opt-out; absolute harness path; children die on drop — the SEAM LINE closes |
| 1.28.70 | Twokeys | Token-file line 2 becomes a scoped agent principal; single-token keeps legacy posture with a warn |
| 1.28.71 | Pores | Screen runs on stripped text; translation/typoglycemia/encoding tiers; optional ONNX classifier |
| 1.28.72 | Scrim | Read-seam hostile-element strip; Write-gated suggestion evidence; pre-stream 403 on denied event subscribers |
| 1.28.73 | Keyring | Deterministic operator key + one-deep rotation; chain-less restores refuse; bounded replay eviction |
| 1.28.74 | Origin | Owner/channel origin context; channel-capture labels ride recall with exclude option |
| 1.28.75 | Preflight | argv0 + allowlist canonicalization; installer review-posture default; SBOM selfcheck gate |
| 1.28.76 | Selfheal | Bounded fixed-point strips; budgeted scorer input; kill-switch reach; gated live SSE |
| 1.28.77 | Erasure | Session-arm erasure; export cap; restore-before-overwrite; valet crank/brief seams |
| 1.28.78 | Unconditional | Quarantine on every leg; at-least-once channel delivery |
| 1.28.79 | Parity | Multiline-token refusal; redirect re-pin; chat-gated mirrors; quarantine-closed reindex |
| 1.28.80 | Lockdown | Manual-redirect transport; system-prompt merge sanitize; single-block tool envelope; signed pin acks; auth/wildcard admissions; optional approval quorum; `included_global`, `authn`, tripwire echoes |

## Milestone themes

- **v1.16.x "Integrated"** — client polish: PWA, deep links, command palette, responsive mobile, paginated audit.
- **v1.17.x "Govern" → "Complete"** — governance server releases (retention, Art 30, UMP conformance) then the full operator console that surfaces them (12 panels).
- **v1.18.x "Compliant" → "Transparency"** — WCAG 2.2 AA + i18n + privacy hardening, secret-safe console history, and the Art 50 origin marker + export provenance.
- **v2.0.0 "Cortex"** — multi-team tenancy, ready, consuming the v1.2 AuthN/AuthZ foundation.
- **v2.1+ "Limits" / "Regions"** — distributed revocation, scaling.
- **v3.x "Survive" / "Sovereign"** — federated, sovereign deployments.
- **v4.0 "Standard"** — standards conformance.

## How releases are governed

Since v1.5, feature releases are scoped to an **evidence-gated roadmap** (`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md`). The rule: ship only what is evidenced and low-risk; forbid autonomous consolidation, unsolicited push, hidden personalization, and synthetic content. Light cuts are preferred over ambitious-but-unverifiable features.

## Next steps

- **[Features](./features.md)** — everything current releases can do.
- **[Governance & Compliance](./compliance.md)** — the standards work ahead.
- The full history: `CHANGELOG.md` and `ROADMAP.md` in the repository.
