# The Governed Diagnostic Loop: law-cited phases, clinical process shape, local calibrated judgment

**File:** `src/workflow/gdl.rs` (case machine, 9,871 lines) ·
`src/workflow/gdl_checkpoint.rs` (journal contract, 771) ·
`src/workflow/gdl_eval.rs` (A/B/C runner, 1,191) ·
`src/workflow/decide/{lang,router,sequence,calibration,presets}.rs` (System-1 pure port) ·
`src/workflow/reflection.rs` (retrospective corpus) ·
`src/workflow/redflags_domains.json` (must-miss catalog)

## The problem

Autonomous troubleshooting fails in four repeatable shapes: skipped triage
(work starts before the case is classified), unspoken worst cases (nobody
names what kills), dropped handoffs (context evaporates between owners), and
premature closure (the case ends because effort ran out, not because evidence
ran in). Post-hoc incident labels cannot fix these — they describe the
failure after the patient, customer, or outage already paid for it. The 2026
RCA literature converges on the alternative posture this module implements:
active reasoning, where the loop drives evidence through a hypothesis
structure instead of labeling an incident post-hoc. The open question the
code answers is how to make that structure *enforceable* — gates a model
cannot argue with, in deterministic Rust, with every refusal citing its law.

## The references

- **Phased diagnosis as a process.** National Academies of Sciences,
  Engineering, and Medicine, *Improving Diagnosis in Health Care* (2015):
  diagnosis as a multi-step process with named failure points, step 6
  carrying the closure discipline this loop gates as A8/A9 (no resolution
  without a law-clean closure artifact, reflexive closure refused). Cited
  as process shape, not as a diagnostic instrument — the code enforces
  that closure *happens with evidence*, never what the diagnosis *is*.
- **Structured handoff.** Starmer et al., *Changes in Medical Errors after
  Implementation of a Handoff Program*, NEJM 2014 (the I-PASS study):
  sender-owned illness-severity / patient-summary / action-list /
  situation-awareness / synthesis sections, assembled — never synthesized —
  by the sender. The loop's `ipass_facts` renders sender-owned sections
  only; the C3 escalated case lands exactly one pre-filled offer draft,
  HITL-gated.
- **Triage acuity.** Gilboy et al., *Emergency Severity Index, v4* (AHRQ),
  and Mackway-Jones et al., *Emergency Triage* (the Manchester system):
  banded acuity with wait windows. The loop ports the *shape* — MTS-style
  bands (RED/ORANGE/YELLOW/GREEN/BLUE) plus ESI 1–5, at least one required
  at triage exit (T4), closed sets (T15/T16) — while keeping acuity a
  MONITOR beside the authoritative P-class SLA (`advertised_sla` takes the
  tighter of the two, never the looser).
- **Calibrated confidence.** Guo, Pleiss, Sun & Weinberger, *On Calibration
  of Modern Neural Networks*, ICML 2017: predicted probabilities need
  temperature fitting against held-out data (ECE) before anyone acts on
  them. The System-1 port implements exactly this — entropy confidence,
  temp buckets, hand-computable ECE with a NaN-means-no-measure law —
  with the rollout consequence the paper implies: conservative 0.85
  thresholds (escalate-heavy) until the fit exists, auto-act only behind a
  fine-tuned checkpoint with a pinned SHA plus ECE evidence.
- **Reciprocal structure, not cited as one paper because it isn't one:**
  the loop's per-phase JSON artifact + pure-arbiter (`parse_and_gate`) +
  bounded-then-routed retry (`MAX_PHASE_ATTEMPTS = 3`) is the
  propose-verify-route pattern the agentic literature re-derives
  independently; the repo's contribution is making the verifier
  deterministic, total (never panics — the fuzz seams drive it), and
  law-citing.

## The deterministic way brain-server implements it

One case is one governed experiment through seven forward-only phases
(`GdlPhase::ALL` — `Intake → Triage → Hypothesize → Plan → Act → Verify →
Handoff`; a case that cannot satisfy a phase routes or escalates, never
skips). Per phase-pass, ONE `WorkflowTx` carries the `workflow_steps` row
(Act adds one sub-row per test-log row), the CAS run-state advance with its
own audit row, and one audit row per step — all-or-nothing, hash-chained;
the session narrative rides append-only `agent_session_events`. Nine
binding laws (L1 evidence-before-action through L9 no-fix-from-memory) are
enforced where mechanically checkable, and every gate failure cites its law
via `err(law, detail)` — a rejection is an auditable process fact. The
clinical layer (1.32.7) adds the T/A/B/C gate families: acuity duty,
red-flag forcing function with monotonic escalate-first lock, the
per-domain must-miss catalog (fail-closed on parse), NAM-gated closure at
the single resolution seam, back-referral contracts with an overdue HITL
sweep that never auto-resolves, and the red-flag-handoff escalation
exception. The System-1 layer (1.32.8, Phase 0 landed) adds the pure
decision modules under hard invariants: closed choice/score/noul
vocabularies, a 20-option ceiling with no bypass, `f32` confined to
`calibration.rs` by compile-time scan, integer score units downstream.
Learning closes the loop retrospectively: the closing transaction derives a
reflection record ONLY from audited gate rows (never agent free text —
`input_digest`, never raw case text) plus hard-negative disagreement
tuples, proven byte-identical with capture on versus off, exported
de-identified under a dual gate with frozen train/holdout partitions.

## Measured ceiling

- **Analogy, not instrument.** ESI/MTS/ATA are `-style` labels; the
  clinical content is keyword data in one `health` catalog domain, not
  SNOMED/ICD/LOINC; `resource_estimate` never binds. The loop enforces
  process, never practices medicine — no diagnostic claims, no
  certification claims.
- **Acuity is advisory by construction.** Monitor-only beside P-class; a
  deployment that wants acuity to bind resourcing must say so explicitly
  (no such knob exists today).
- **Local judgment is ungated potential until 1.32.8 stamps.** Phase 0 is
  pure math with 134 tests and no callers; base checkpoints are weak
  zero-shot (36% business cited vs 73% hosted); the `score` primitive is
  quarantined on strict scaling; inference, preload, pilots, and the
  temperature fit are all ahead — the lane stamps on operator-labeled
  proof, not before.
- **The corpus is retrospective-only by proof, useful-only by future
  work.** Capture cannot perturb resolution (pinned), but no training run
  on the corpus has happened in-tree; train/holdout bleed is checkable
  (frozen partitions ride the rows), not yet checked by a training loop.
