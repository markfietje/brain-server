# The loop learned clinical discipline (without practicing medicine)

*2026-09-22, v1.28.92 / 1.32.7 "Diagnostic Closure". What the governed case loop borrowed from healthcare safety regimes — and the lines it refuses to cross.*

The 1.32.7 stamp added something unusual to a troubleshooting engine: triage acuity bands, a red-flag forcing function, a must-miss catalog with sepsis and stroke in it, a closure gate named after a National Academy of Medicine step, and handoffs shaped like I-PASS. This post explains what that layer is, what it buys three different readers, and — carefully — what it is not. It is not a medical device. It makes no diagnostic claims. It speaks no SNOMED, ICD, or LOINC. What it does is enforce *process safety* in the shape clinicians and auditors already recognize.

## What shipped

The Triage → Handoff span of the GDL case machine now carries six enforced mechanisms (all gate code in `src/workflow/gdl.rs`, all refusals citing their law):

- **Triage acuity duty (T4/T15/T16/T18).** Every case classifies acuity before anything else: an MTS-style band (RED/ORANGE/YELLOW/GREEN/BLUE) or an ESI level 1–5, or both. No bypass, no default. Disposition is a closed six (`self_care`, `virtual_primary`, `in_person_primary`, `refer`, `facility`, `ed`) — an `ed` disposition without an open red-flag is refused, and a virtual encounter must carry modality-adequacy or it converts.
- **Acuity as monitor, P-class as law.** The acuity windows (RED 0s through BLUE 14,400s) never override the authoritative P-class SLA (P1 3,600s through P4 604,800s). The advertised target is the tighter of the two, never the looser. Clinical urgency informs; operational contract governs.
- **Red-flag forcing function.** Every case names its worst case, whether it is ruled out, on what basis, and what a miss would cost first. The lock is monotonic escalate-first: once a flag is open, the case can only move toward more care, never less, without recorded justification. Behind it sits a must-miss catalog (`redflags_domains.json`): a `default` domain (irreversible data loss, active breach) and a `health` domain — sepsis, chest pain, anaphylaxis, abuse/self-harm in minors, stroke, decompensation — as keyword data. Parse failure closes the gate, not the case.
- **NAM-step-6 closure gate.** No case resolves without a law-clean closure artifact (A8); reflexive closure without the artifact is refused (A9), at the single resolution seam. The loop cannot end a case by getting tired of it.
- **Back-referral contract.** A referral handoff without a return contract is refused (B1); the receiver's release needs the report complete (B3 names what's missing); overdue contracts land a human task and never auto-resolve. Except one: a red-flag handoff never blocks on paperwork — escalation outranks the contract by law.
- **I-PASS discipline.** Escalations land exactly one pre-filled offer draft, built from sender-owned sections only — the machine never synthesizes handoff content — and gated on human approval.

## What it buys the operator

The night-shift version: triage can never be skipped, the thing that kills people gets named before anything else, the handoff you receive actually contains the case, and the case you close was actually finished. Every one of those used to depend on individual diligence. Now it depends on gates that refuse. Diligence still matters — the keyword catalog is heuristic, acuity is an analogy — but the floor moved from "whoever is on shift" to "the machine will not let this shape of failure through."

## What it buys the buyer

Healthcare-adjacent buyers already get sovereignty (on-prem, no egress), erasure (DSAR with certificates), and explainability (fenced, provenance-labeled recall) from this system. The 1.32.7 layer adds something procurement actually asks for: a workflow shaped like the safety regimes the buyer's clinicians and auditors already answer to. ESI and MTS are the languages of their triage nurses. I-PASS is the language of their handoff audits. NAM is the language of their diagnostic-safety reviews. When the auditor asks "how do you ensure deteriorating cases escalate," the answer is a gate ID and a test, not a policy paragraph. Nothing here is a certification claim — there is none — but the evidence is shaped to fit the frame the buyer's world already uses.

## What it buys the engineer (in any domain)

The design pattern travels without the clinical content. Strip out the health keywords and what remains is: mandatory classification before work, a forcing function for worst-case thinking with a monotonic lock, a must-miss list for your own domain's catastrophes, closure that requires evidence of completion, referrals that carry return contracts, and handoffs the machine assembles but never invents. A BPO triage queue, an SRE incident process, and a clinical intake desk all fail in the same shapes — skipped triage, unspoken worst cases, dropped handoffs, premature closure. The gates are domain-shaped data over domain-free laws.

## The lines it refuses to cross

Stated plainly, because the temptation to overread this is real: acuity is monitor-only and never binds resourcing; ESI/MTS/ATA are `-style` labels, and the clinical content is keyword lists, not coded terminology — the loop cannot practice medicine, only enforce process; there are no diagnostic claims and no certification claims; the non-clinical neighbors (session-tree handoff infrastructure, the local-decision port, the ungated classifier lane) are explicitly not healthcare evidence. The layer lives in code, tests, and the release record today; the compliance-surface write-up follows. Process safety first, paperwork second — but the paperwork is owed, and this post is part of paying it.
