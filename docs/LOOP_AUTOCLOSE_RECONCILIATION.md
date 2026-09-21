# The auto-close reconciliation — registered decision record

Date: 2026-09-20. A decision record, not an evaluation result, release, or compliance claim.

## The statement being reconciled

The loop-line's law book says "the machine never auto-closes". The GDL case machine
nevertheless has a `Resolved` terminal, and the checkpoint persists a case row with the
`resolved` status when that terminal is reached. Read literally side by side, the two look
contradictory. They are not, and this record fixes the reconciliation so neither statement
can be quoted against the other.

## The reconciliation

"The machine never auto-closes" is defined as:

> **No terminal path may reach `Resolved` without (a) a passing, law-clean Verify artifact
> (L6 re-run match + A6 stability floor + A6 negative check), (b) a present handoff
> capture (A7), and — since 1.32.7 Diagnostic Closure — (c) a law-clean ClosureArtifact
> (NAM 2015 step 6: the decision communicated to the customer, warning signs, follow-up;
> absence is the exact A8 refusal, reflexive re-closure without a communication record
> is A9).** Knowledge publication remains proposal-only. A failed or missing Verify is
> the terminal `VerifyFailed` hand-back to a human, never a silent retry.

Under this definition, the machine settling a case as `Resolved` IS permitted — it is the
**evidence-gated closure**: every ingredient of the closure is a law-gated artifact the
machine was forced to earn (the verify artifact must re-run the plan's exact failing
scenario under the A6 stability floor with a clean negative check; the capture must exist
and be retrievable; the closure record proves the customer was actually closed with —
communicated_to, shared decision, warning signs, and follow-up, telehealth-gated). What
never happens is closure without verify-passed evidence, closure over open contradictions,
closure without a communication record, or silent knowledge publication — captures land as
proposals on the human review queue or not at all.

## Where the law is enforced

- `verify_gate` (L6/A6) — the re-run IS the planned failing scenario, the stability window
  is at least the floor, the negative check ran. Only `pass: true` can sit under a closure.
- `handoff_gate` (A7) — the capture artifact is present and non-empty.
- The closure duty (A8/A9, 1.32.7) — the Handoff pass post-gate seam: no `Resolved`
  without a law-clean `ClosureArtifact`; the open-red-flag lock (T10) refuses closure
  entirely until a verify-class rule-out closes the flag; a non-urgent referral-type
  handoff refuses (B1) without its return contract.
- The driver's terminal construction — `Resolved` is built only at the Handoff phase, only
  on a gate pass, and only with both artifacts present; the open-contradiction refusal
  (A4) blocks Handoff closure entirely until every pair is dispositioned.
- The exhaustive terminal-path test
  (`machine_never_auto_closes_resolved_requires_law_clean_verify_and_capture`) drives every
  terminal path and asserts the property; a new `GdlOutcome` variant fails that test at
  compile time until its terminal path is reconciled too.

## Scope limits

Live closure remains disabled pending the operator's compute gates; the machine's
evidence-gated settlement of a case row is internal bookkeeping, not live customer
communication. This record is engineering law for the loop-line; it makes no compliance
determination and no claim beyond the recorded runs.
