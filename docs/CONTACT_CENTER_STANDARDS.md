# Contact Center Standards Alignment — the 1.28.x program, second pass

**Date:** 2026-08-23 · **Posture:** self-assessed conformance mapping (the house rule from COMPLIANCE.md applies: *documented posture, not a certification*). Standards move — this file cites the versions verified this date and names the watch items.

**Standards inventory (verified 2026-08):**

| Standard | Version status | Why it matters here |
|---|---|---|
| **ISO 18295-1:2017** (contact centres — requirements for the centre) / **-2** (client orgs) | revision in progress (ISO/AWI 18295-1) — watch item | THE international standard; explicitly covers in-house *and* outsourced centres (= on-house + BPO) |
| **COPC CX Standard, Release 8.0** (Feb 2026) | current | The performance-management framework global centers buy against: forecasting, scheduling, capacity, service level, QA calibration |
| **KCS v6** (Consortium for Service Innovation) | current | The knowledge-centered-service backbone of the planned v1.28.x knowledge loop |
| **WCAG 2.2 AA / EN 301 549 (→ V4.1.1 incorporates WCAG 2.2 AA) / Section 508 + VPAT ACR** | EAA enforcement live since June 2025 | Procurement gate for the console in EU and US-federal contexts; EN 301 549 adds **non-web software clauses** that cover the desktop client |
| **ISO 10002:2018** (complaints handling) | current | Complaints are a distinct class from incidents — the diagnostic loop models them as a first-class case class with its own ack/response clocks and the full lifecycle (v1.28.34) |
| Industry KPI definitions (SQM-class FCR methodology; consensus AHT/abandonment/service-level) | — | Small and global centers must report the same words meaning the same things |
| **EU AI Act Art. 12/14/15, NIST AI RMF, ISO/IEC 42001** | mapped | DecisionRecords, human-in-the-loop, monthly signed calibration already land here (COMPLIANCE.md) |
| **GDPR / PH RA 10173, ISO 27001-mapped controls, SOC 2 readiness** | mapped | SECURITY.md / COMPLIANCE.md lineage |

---

## Conformance matrix — capability → standard → where it ships

**Status legend:** ✅ shipped (in a released version) · 🟡 planned (v1.28.x conformance releases) · ❌ open gap.

| Capability (release) | Standard anchor | Status |
|---|---|---|
| *Every gap below (G1–G8) is planned to close in the v1.28.x conformance releases.* | | |
| Governed diagnostic loop, evidence per step, audit chain | ISO 18295-1 process/performance clauses; AI Act Art.12 traceability | 🟡 planned (v1.28.x) |
| HITL on every memory/publication decision; erasure; DSAR | GDPR Art.15/17/19/22; ISO 18295-1 data protection | ✅ shipped (v1.27.x) |
| QA: 100% justified scoring, gold calibration, κ gate, monthly human sign-off | COPC R8.0 QA + calibration discipline | 🟡 planned (v1.28.x); **explicit COPC mapping rows to be added to COMPLIANCE.md** (gap G6) |
| KCS double loop + public KB + deflection | KCS v6; demand reduction | 🟡 planned (v1.28.x) |
| SLA envelopes P1–P4 + follow-the-sun handover | COPC service-level management; handover research | 🟡 planned (v1.28.x candidate) |
| **Complaints as a first-class case class** | ISO 10002 | ✅ shipped — distinct intake class with its own acknowledgment/response SLA (`complaint_class_gets_acknowledgment_sla`, `src/workflow/frontdoor.rs`); full ISO 10002 lifecycle as lineage events (v1.28.34) |
| **Normative metric dictionary with formulas + data lineage** | COPC/KPI consensus; SQM FCR method | ✅ shipped — `docs/metrics.md` normative dictionary; `BRAIN_FCR_WINDOW_DAYS` default 7 (`src/config.rs`) |
| **WCAG 2.2 AA as a hard gate; VPAT/ACR artifact; desktop EN 301 549 software clauses** | EN 301 549 V4.1.1 / Section 508 / EAA | ✅ shipped — `docs/trust/acr-vpat.md` + `docs/trust/wcag22-aa-checklist.md`; release-blocking client gate (`wcag_22_aa_gate_blocks_release`) |
| **RTL + pseudolocale readiness** (global locales) | global usability | ⚠️ **gap G4** — recon: no RTL locales, FTL subset |
| **WFM seam (shift/skills feed in/out)** | COPC forecasting/scheduling/capacity | 🟡 partially shipped — `GET/POST /ops/shifts` + `GET /ops/skills` routed (`src/main.rs`); documented interop-boundary contract still open |
| People clauses: competence, workload visibility | ISO 18295-1 people/performance | 🟡 partially (presence + skills shipped; workload visibility planned v1.28.x); **workload = measured visibility, never enforcement — documented ceiling** (G7) |
| Deployment tiers small → global | ISO 18295-1 applicability (any size) | ✅ shipped — `docs/deployment.md` "Deployment tiers" section defines T1–T4 |
| Payment data handling | PCI DSS | ✅ **explicit non-scope**: payment data never ingested; THREAT_MODEL §6 gains the boundary row (G9, one line) |
| ISO/AWI 18295-1 revision | — | watch item: re-map clause refs on publication (G10) |

## The planned conformance pack (v1.28.x "Charter")

One planned release closes the gaps; nothing else in the line changes scope.

1. **G1 Complaints (ISO 10002):** the case intake classifier gains a `Complaint` intent class; complaints get their own envelope policy (acknowledgment deadline ≤ policy, response deadline, distinct priority map); the complaints register is the existing audit chain + a `case kind='complaint'`; escalation-to-dispute is a documented handover with reason `dispute`. Zero new tables — the class rides existing machinery. *Tests: `complaint_class_gets_acknowledgment_sla`, `complaint_escalation_is_audited_as_dispute`.*
2. **G2 Metrics dictionary:** `docs/metrics.md` — every scoreboard field (every current scoreboard field + the conformance-pack additions) gets: formula, source table/column (data lineage into the audit-derivable property), window semantics, and the industry citation (FCR per SQM-style repeat-window, configurable `BRAIN_FCR_WINDOW_DAYS` default 7; AHT decomposition talk+hold+ACW where CRM data provides it; abandonment only where telephony feeds exist). *Tests: `scoreboard_fields_have_dictionary_entries` (meta-test greps docs↔code parity), `fcr_window_is_configurable_and_deterministic`.*
3. **G3 Accessibility as a gate:** the client DoD gains **WCAG 2.2 AA conformance as a release-blocking gate** (focus-visible, target-size, dragging alternatives, consistent help — the 2.2-specific criteria; the existing automated tests extend); ship `docs/trust/acr-vpat.md` — the Accessibility Conformance Report (VPAT format) for web + desktop, honestly marking the known ceilings (axe browser gate, focus restoration); desktop additionally maps the EN 301 549 non-web software clauses. *Tests: `wcag_22_aa_gate_blocks_release` (checklist-driven), `acr_lists_known_ceilings_honestly`.*
4. **G4 Global locales:** add `ar` (or `he`) RTL locale + `en-XA` pseudolocale to the i18n test set; the transcript/panels pass a mirroring smoke (dir=rtl attribute plumbing exists via the theme/density eval bridge — extend it). Parity test covers the new locales. *Test: `rtl_locale_renders_mirrored_without_layout_breakage`.*
5. **G5 WFM seam:** `GET/POST /ops/shifts` + `GET /ops/skills` become the documented interop boundary (JSON, stable schema, Read/Write gated) — centers keep their WFM tool, brain keeps the governed truth. No forecasting engine is built (COPC alignment = interop, not reimplementation). *Test: `wfm_feed_round_trips_shifts_and_skills`.*
6. **G6–G10:** COPC R8.0 + ISO 18295-1 clause map rows in COMPLIANCE.md; workload-visibility ceiling note (measured, never enforced — the centre manages its people, the tool makes it visible); deployment tier guide (`docs/deployment.md` section): **T1 solo** (loopback, single domain, posture=open) → **T2 team** (roles + proposals, posture=review) → **T3 site** (multi-domain/multi-db, calibration, public KB feedback) → **T4 global** (sites + parcels + regional residency stamps); PCI non-scope row in THREAT_MODEL §6; ISO/AWI 18295-1 watch item registered in the docs-truth meta-test so the revision can't land silently.

**Scope guards (deliberate non-goals):** the conformance pack does NOT pursue certification of anything (self-assessment only); does NOT build forecasting/scheduling/capacity engines (COPC alignment = interop only); does NOT add survey tooling (CSAT instruments stay CRM-side); does NOT add telephony/queue metrics the data source can't ground (abandonment appears only when a telephony feed exists).

## The Order of Care (doctrine — 2026-08-23 research pass)

The sequence below is what the standards families converge on independently (ISO 10002's lifecycle, the KCS Solve loop, ITIL's incident→problem flow, COPC's service-level discipline), with the *ordering rationale* supplied by the research that explains which step buys which outcome:

**Prevent → Self-serve-verified → Acknowledge fast, once → Understand with evidence → Resolve first-contact-first-time-right → Remedy with fairness → Confirm with the customer → Capture → Follow up → Learn.**

| # | Step | Why this position (research) | Enforced by (product) |
|---|---|---|---|
| 1 | **Prevent** | proactive intervention saves 20–40% of at-risk customers; a contact that never happens has effort = 0, the best possible CES | proactive outreach cohorts; IoT/CRM signals via connectors (planned v1.28.x). **Keystone closes the last gap: the public case-status page** — a customer who can *see* the case doesn't call about it (static artifact, unguessable ref, fixed seven-word vocabulary, zero PII) |
| 2 | **Self-serve, verified** | deflection only counts when it *solves* — failed self-service adds effort | public KB feedback loop with solved-proof; suggestion self-instruction rate (planned v1.28.x); **Keystone: verified multilingual self-service** — governed human translations (`kcs_translate` HITL), staleness first-class on the content-health worklist, hreflang alternates, never a silent fallback |
| 3 | **Acknowledge fast, once** | the perception clock: acknowledgment speed shapes satisfaction more than resolution speed; repetition of context is the #1 effort driver | complaint acknowledgment SLA (planned); context continuity — customer never repeats (planned v1.28.x). **Keystone: the re-ask is now COUNTED, not just avoided** — `case/reask` events from three deterministic sources (CRM merge, operator mark, derived duplicate proposal) feed `reask_rate` and the effort proxy |
| 4 | **Understand with evidence** | guessing adds contacts; evidence-first is the diagnostic loop's core | IS/IS-NOT intake gate; the evidence law |
| 5 | **Resolve first-contact, first-time-right** | FCR = the strongest single driver: +12–15% retention; speed-over-resolution *harms* retention | verify-gated close (2nd verification); FTFR for field work (planned) |
| 6 | **Remedy with fairness** | the service-recovery paradox: a well-recovered failure beats no failure — *iff* minor, fast, genuine, never repeated; severe/repeated failures forfeit it | role-capped remedies + code-clause citation (planned v1.28.x); repeater detection |
| 7 | **Confirm with the customer** | peak-end rule: the *ending* of the journey is disproportionately remembered | **Planned: confirm-gate** — a case cannot reach `closed` without a customer-confirmation event (or the documented consent-absent exception after 3 attempts) |
| 8 | **Capture** | knowledge captured in-workflow, not after (KCS) | capture-at-close (planned v1.28.x) |
| 9 | **Follow up** | the proactive ending that compounds the peak-end effect into brand trust | **Planned: follow-up event** — post-close check as a consent-gated proposal at policy interval |
| 10 | **Learn** | RCA prevents the next contact — the loop that feeds step 1 | knowledge flywheel; complaint clusters ranked first (planned v1.28.x) |

**Queue priority when steps collide:** Safety/legal > at-risk retention > SLA-clock > FIFO-with-context. Never AHT over resolution (the research is unambiguous that this trades retention for a metric).

**NEW derived metric (amendment):** `customer_effort_events` — a deterministic CES proxy computed per case from the lineage: repeat contacts × channel switches × handovers × re-asks (`case/reask`, emitted since v1.28.36 from three deterministic sources; score = repeats×2 + switches×1 + handovers×3 + re_asks×2). No survey instrument (VoC surveys stay CRM-side per ISO 10004); this is the lineage-derived twin, documented in the metrics dictionary as a *proxy* with its formula. Scorer: the confirm-gate and effort-proxy land as scored dimensions in the next scorer version (gold-set families extend accordingly).



| Standard / law | Anchor in the product |
|---|---|
| **ISO 10001:2018** codes of conduct (promises incl. returns/warranties) | published code clauses live in the public KB; remedy proposals cite them (planned v1.28.x) |
| **ISO 10002:2018** complaint handling | complaint lifecycle: acknowledge→investigate→remedy→close, register = audit chain (planned v1.28.x) |
| **ISO 10003:2018** external dispute resolution | ADR handoff packet → **national ADR body** per member state — the EU ODR platform was discontinued 20 Jul 2025 (Reg. 2024/3228); do not reference it |
| **ISO 10004:2018** satisfaction monitoring | VoC store: CRM-side CSAT ingested via connectors + public feedback; scoreboard dictionary formulas (planned v1.28.x) |
| **ISO 23592:2021** service excellence | the service-excellence model maps to the tier guide + calibration discipline; principles cited, not certified |
| **GPSR (EU) 2023/988** (since 13 Dec 2024) | safety-recall worktype: blast proposals, Safety Gate reference fields, serial/batch traceability via the entitlement registry (planned v1.28.x) |
| **Directive (EU) 2019/771** | 2-year conformity guarantee + limitation extension computed in the entitlement window arithmetic (planned v1.28.x) |
| Consumer Rights Directive (14-day withdrawal) | withdrawal disposition with legal-basis citation (planned v1.28.x) |
| Consent regimes (ePrivacy / TCPA-class) | consent registry: per-subject/channel/purpose, proposal-gated, DSAR-erasable; no-consent-no-send as a gate (planned v1.28.x) |
| Aftersales KPI canon (FTFR 68%→82% benchmarks; returns/warranty KPI set) | FTFR + return rate + refund cycle time + warranty claim rate in the metrics dictionary (planned v1.28.x) |

## Verdict of the second pass

The architecture was already the strong part — the standards pass changed no load-bearing design. What it changed: **accessibility is a gate, not residue; complaints are a class, not an escalation flavor; metrics are a dictionary, not a scoreboard accident; WFM is a boundary, not a build; tiers are documented, not implied.** With the conformance pack landed (planned v1.28.x), the program is honestly presentable to a small center (T1), a global BPO (T4), and a procurement office holding ISO 18295 / COPC R8.0 / EN 301 549 checklists — as *mapped posture*, which is the only claim this repo has ever been allowed to make.
