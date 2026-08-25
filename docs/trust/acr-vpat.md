# Accessibility Conformance Report

Based on VPAT® Version 2.5 · **Report date:** 2026-08-25 · **Product:** brain-server web console + desktop client
**Evaluation method:** automated axe-core scans on the served console build + keyboard-only manual walkthroughs of every panel. Posture per house rule: *documented conformance claim backed by evidence*, not a certification.

## Standards applied

| Standard | Scope of this report |
|---|---|
| **WCAG 2.2 AA** (W3C Recommendation) | web console |
| **EN 301 549 V4.1.1** (clauses 9 + 10 + 11) | clause 11 (non-web software) for the desktop client; clauses 9–10 inherit the WCAG result |
| **Section 508** (refreshed) | inherits EN 301 549 mapping |

## Conformance level claimed

**Partially supports** WCAG 2.2 AA — every Success Criterion is either met
(evidence below) or listed under Known Ceilings with its remediation owner.
No criterion is "does not support" without an entry there.

## WCAG 2.2 criteria — evidence summary

The machine-checkable list lives in
[wcag22-aa-checklist.md](wcag22-aa-checklist.md); the release gate
(`wcag_22_aa_gate_blocks_release`) fails when any criterion loses its pass or
its documented ceiling. Highlights:

- **Perceivable:** text alternatives on icon-only buttons (`aria-label` from the locale bundle — the same `t()` surface, so translations carry accessibility labels too); contrast verified against both shipped themes (dark/light) at AA ratios; no information conveyed by color alone in verdict/status chips (text label always present).
- **Operable:** full keyboard operation (the review flow is keyboard-first: A/S/R/E/J/K shortcuts with visible focus); **2.4.7 focus-visible** styling ships in both themes; **2.5.8 target size** ≥ 24×24 CSS px on all interactive controls; **2.5.7 dragging alternatives** — every drag affordance has a button equivalent; reflow to 320 px / 400% zoom.
- **Understandable:** page language follows the active locale (`ar` sets `dir="rtl"`, mirrored layout smoke-pinned); error messages are text, tied to their input.
- **Robust:** semantic HTML controls (native button/input), labels bound via `for`/`aria-label`; status changes announced through live regions on the review queue.

## EN 301 549 clause 11 (desktop client, non-web software)

| Clause area | Posture |
|---|---|
| 11.1 general / 11.2 legacy | n/a — current platform APIs only |
| 11.3 keyboard + focus (11.1.1.2 style equivalents of WCAG operability) | supported: the desktop shell renders the same semantic controls; full keyboard traversal, visible focus ring |
| 11.5 visual contrast / font scaling | supported: OS font-scale respected up to 200%; theme contrast shared with web |
| 11.8 speech / 11.9 automation | partial — see Known Ceilings |

## Known ceilings (honest)

- **axe browser gate covers the web console only.** The axe scan runs against
  the served console build; the desktop shell is covered by the manual
  keyboard walkthrough + clause-11 self-assessment above, not by axe.
- **Focus restoration after modal close is not yet guaranteed everywhere.**
  Drawers restore focus to their invoker; the command palette and the confirm
  dialog do not yet — tracked as an open a11y defect, remediation planned
  before the next ACR revision.
- RTL mirroring is attribute-level (`dir="rtl"`); deep bidirectional text in
  mixed-content transcripts relies on browser bidi algorithms — no dedicated
  Unicode bidi audit has been run.
- The report reflects the build dated above; each release re-runs the gate,
  but manual walkthrough evidence refreshes only when UI panels change.
