# Roadmap & Release History

Brain Server ships on a strict linear release chain. This page is the roadmap summary and the release history. The authoritative version of both lives in `ROADMAP.md` and `CHANGELOG.md` in the repository.

## Current status

- **Latest server version:** 1.28.35 "Outreach" (2026-08-26) — proactive care, consent-first: a hashed-subject consent registry (revocation-wins, fail-closed verdicts), campaigns as HITL proposals gated per recipient BEFORE filing with consent proof riding every included recipient, approved campaigns export for CRM-side execution only (no send engine exists anywhere), the consent-gated Order-of-Care post-close follow-up, DSAR sweep erasing consent rows by re-hashing the subject, and ISO 10004 VoC fields on the scoreboard. Its predecessor 1.28.34 "Goodwill" shipped the full ISO
  10002/10003 complaint lifecycle: the closed state chain as lineage events on
  the audit chain, the remedy matrix as HITL proposals citing legal basis +
  published conduct clause with role-tier approval caps that escalate exactly
  one level over cap, a goodwill ledger aggregated only from audited remedies,
  and the national-body ADR packet per Reg. 2024/3228 (the EU ODR platform is
  discontinued). Financial execution never happens here — every remedy is a
  decision record. Earlier in the line, v1.28 FirstLight → Settle
  (1.28.15–17) shipped the real governed workflow loop itself: role-gated
  run/state/events/answer/steering routes, AskHuman answers digest-bound to
  the live question, all four mediated hostcall kinds
  (`exec`/`http`/`events`/`ui`), and the repaired fail-closed
  `/workflow/scoreboard`.
- **Latest client version:** 1.28.23 — ships alongside the server.
- **Latest plugin version:** 0.4.7 (2026-08-23) — drift reconciliation + hardening; rides brain-server `v1.28.14` and later.
- **Next milestone:** v1.29 "Acuity", then v2.0.0 "Cortex".
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

## The 1.28 harness → conformance lines (v1.28.15 → v1.28.34)

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
