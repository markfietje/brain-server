# Metrics dictionary — the normative definitions

Every scoreboard field the API serves (`GET /workflow/scoreboard`) is defined
here exactly once: formula, source (data lineage), window semantics, and the
industry citation it follows. This file is pinned by the meta-test
`scoreboard_fields_have_dictionary_entries` — a scoreboard field cannot ship
without its dictionary entry. All rates are **integer ten-thousandths**
(10000 = 100%).

Posture: *documented measurement*, not certification. Fields whose data
source does not exist in this system are **not emitted** (no invented
telephony/CRM numbers) — see "Deliberately absent" at the end.

## Scoreboard fields

| Field | Definition / formula | Source (lineage) | Window | Citation |
|---|---|---|---|---|
| `fcr_units` | share of scored runs with no repeat contact: `runs_without_repeat / runs_scored`. A recurrence recorded inside the FCR window marks its predecessor as not-first-contact-resolved. | `workflow_runs.state_json` (`repeat_contact`, `prev_contact_age_secs`) + fail-closed audit linkage (`audit_events`) | `BRAIN_FCR_WINDOW_DAYS` repeat-attribution window, default 7 days | SQM-class FCR repeat-window methodology; COPC R8.0 FCR discipline |
| `repeat_contact_rate_units` | complement of FCR: `runs_with_repeat / runs_scored`. The primary demand metric — deflection never trades against it. | same as `fcr_units` | same FCR window | COPC R8.0; docs/kb-deflection.md |
| `correctness_units` | share of runs whose recorded findings contain no contradiction/incorrect marker. | `workflow_runs.state_json.findings` | last 1000 runs (scored cohort) | ISO 18295-1 process-clause posture; AI Act Art.12 traceability |
| `override_rate_units` | share of workflow steps where human guidance overrode the engine's step output. | `workflow_steps` rows derived into `StepRow`s | last 1000 runs | HITL law (COMPLIANCE.md); NIST AI RMF |
| `gap_rate_units` | knowledge-gap rate. Currently pinned to 0 in the run-derived scorer — gaps derive from proposals, not runs alone; non-zero emission rides the flywheel release. | reserved (proposals tables) | — | KCS v6 Solve-loop gap capture |
| `abstention_rate_units` | share of steps where the engine abstained rather than guessed. Higher is honest, not worse. | `workflow_steps` (`abstained`) | last 1000 runs | AI Act Art.14 human-oversight posture |
| `guidance_acceptance_units` | accepted guidance over offered guidance: `accepted / (accepted + rejected)`; SCALE when none offered. | `workflow_steps` (`guidance_accepted`) | last 1000 runs | COPC R8.0 QA calibration discipline |
| `handoff_completeness_units` | share of runs reaching `completed` status with an I-PASS-complete handover record. | `workflow_runs.status` + handover packet predicates (`src/workflow/relay.rs::packet_missing`) | last 1000 runs | I-PASS handover research; COPC service-level management |
| `audit_green` | boolean: every scored run references at least one workflow audit row (fail-closed — absence never counts green). | `audit_events` linkage per run | last 1000 runs | AI Act Art.12 logging; SOC 2 readiness |
| `escalation_honored_units` | share of runs where recorded escalation requests were honored (default true only when nothing was requested). | `workflow_runs.state_json.escalation_honored` | last 1000 runs | ISO 18295-1 customer-handling clauses |
| `runs_scored` | count of runs in the scored cohort (most recent 1000 by id). | `workflow_runs` | last 1000 runs | — |
| `return_rate_units` | share of runs that are return/RMA runs: `return_runs / runs_scored`. | `workflow_runs.kind = 'return'` | last 1000 runs | returns/warranty KPI set (ClaimLane canon) |
| `warranty_claim_rate_units` | share of runs that are warranty-claim runs: `warranty_runs / runs_scored`. | `workflow_runs.kind = 'warranty_claim'` | last 1000 runs | returns/warranty KPI set |
| `ftfr_units` | **First-time-fix rate** for repair-field work: repair-field runs with no repeat inside the FCR window over all repair-field runs — FCR's repeat-window method applied to first-VISIT resolution (`BRAIN_FCR_WINDOW_DAYS`, default 7). The headline field-service economics metric (~1.6 extra dispatches per missed first visit). Empty cohort scores 0 — absence is never dressed up as perfection. | `workflow_runs.kind='repair_field'` + `state_json.repeat_contact` / `prev_contact_age_secs` | same FCR window | SQM-class repeat-window methodology; FSM FTFR benchmarks |
| `refund_cycle_time_median_secs` | median seconds from run creation to terminal resolution over resolved return/warranty runs; 0 when none resolved. | `workflow_runs.created_at`/`updated_at` + terminal `status` | last 1000 runs | refund cycle-time KPI set |
| `returnless_share_units` | share of RETURN runs disposed returnless-refund, over all return runs; 0 when the cohort is empty. Returnless refunds pair with fraud review (disposition ranking gates it). | `workflow_runs.state_json.returnless` over `kind='return'` | last 1000 runs | returnless-refund/fraud-detection pairing (2026 practice) |
| `aftersales_fraud_flag_rate_units` | share of RETURN runs whose deterministic fraud signals flagged them, over all return runs; 0 when the cohort is empty. Signals inform HITL — they never auto-deny. | `workflow_runs.state_json.fraud_flagged` over `kind='return'` | last 1000 runs | fraud-signals-feed-HITL posture |
| `goodwill_total_cents_30d` | sum of amount_cents over APPROVED remedy proposals in the trailing 30 days whose approval audit row verifies (fail-closed — an unaudited remedy never aggregates). | `proposals kind='complaint_remedy' status='approved'` × `audit_events` target/detail hash linkage | trailing 30 days | ISO 10002 remedy discipline; goodwill-ledger-visible posture |
| `goodwill_entries_30d` | count of audited approved remedies in the window. | same as `goodwill_total_cents_30d` | trailing 30 days | — |
| `goodwill_unaudited_excluded_30d` | approved remedies EXCLUDED for missing audit linkage — surfaced, never folded away. | same scan | trailing 30 days | fail-closed evidence law |
| `voc_contacts_total` | count of workflow runs — the contact volume the VoC ratio denominates. | `workflow_runs` | rolling (all runs) | ISO 10004 satisfaction monitoring as data |
| `voc_complaints_total` | count of runs with `kind = 'complaint'`. | `workflow_runs.kind` | rolling (all runs) | ISO 10002 register posture |
| `voc_complaints_per_thousand_contacts_units` | `complaints * 100_000 / max(contacts, 1)` — complaints per thousand contacts in hundredths (per-mille × 100). Zero contacts score 0. The CSAT/DSAT instruments stay CRM-side; this is the lineage-derived complaint-density twin. | `workflow_runs.kind` counts | rolling (all runs) | ISO 10004 §complaint-per-thousand-contacts KPI canon |

## Report-cadence fields (same read, weekly report ride)

| Field | Definition / formula | Source (lineage) | Window | Citation |
|---|---|---|---|---|
| `calibration_report_emitted` | true when THIS read crossed the weekly boundary and landed a machine-generated CalibrationRecord on the audit chain. | `src/workflow/calibration.rs` | weekly cadence | monthly signed-recalibration posture (COMPLIANCE.md) |
| `kcs_linkage_rate_units` | share of published knowledge linked from closed-run evidence. | `src/workflow/kcs.rs::kcs_measures` | rolling (all governed articles) | KCS v6 Evolve loop |
| `searched_found_rate_units` | share of recall/search events that ended in a found article (SIR proxy). | `kcs_measures` | rolling | KCS v6 Solve loop (search-and-solve) |
| `article_freshness_median_age_secs` | median age in seconds since last review across governed articles. | `kcs_measures` | rolling | KCS v6 article-health |
| `self_service_deflection_units` | INDICATIVE deflection from on-page KB feedback (solved-proofs over total feedback). Never traded against `repeat_contact_rate_units`. | `kcs::kb_feedback_measures` | rolling | docs/kb-deflection.md governs; KCS v6 self-service |
| `kb_feedback_total` | total on-page feedback events counted. | `kcs::kb_feedback_measures` | rolling | — |
| `kb_hot_topics` | top slugs by feedback count above `KB_HOT_TOPIC_THRESHOLD`. | `kcs::kb_hot_topics` | rolling | KCS v6 Evolve (content-defect queue) |
| `reask_rate` | re-ask events (`case/reask`) ÷ closed cases, in hundredths. Three deterministic sources emit the event: `crm_merge` (Zendesk/Salesforce merges, Genesys reopens via the Bridges sync), `marked` (operator `reask` note / `brain workflow note --reask`), `derived` (duplicate-open heuristic — exact hashed-subject match within `BRAIN_REASK_WINDOW_DAYS`, default 3 days, HITL-gated as `case_merge_suggested`; the approved merge emits). No fuzzy matching; no surveys. | `outbox topic='case/reask'`, `workflow_runs.status` | rolling; window semantics per `BRAIN_REASK_WINDOW_DAYS` | CXC customer-effort canon (effort-proxy dimension); Keystone v1.28.36 |

## Derived proxy (planned scorer integration)

**`customer_effort_events`** — a deterministic CES *proxy* per case computed
from the lineage: repeat contacts × channel switches × handovers ×
re-asks (`case/reask`, weighted like a repeat — see `frontdesk::effort_proxy`:
score = repeats×2 + switches×1 + handovers×3 + re_asks×2). No survey
instrument exists here (VoC surveys stay CRM-side per ISO 10004); this is the
lineage-derived twin. It lands as a scored dimension in the next scorer
version with gold-set families extended; until then it is defined here so the
formula is fixed before any code emits it.

## Deliberately absent (scope guards)

- **AHT decomposition** (talk + hold + ACW): appears only when CRM data
  provides the components; no telephony feed exists today.
- **Abandonment rate**: requires a telephony feed; absent until one exists.
- **CSAT/VoC**: instruments stay CRM-side (ISO 10004); only lineage-derived
  proxies live here.
- No forecasting/scheduling/capacity metrics: WFM alignment is interop
  (`GET/POST /ops/shifts`, `GET /ops/skills`), not reimplementation.
