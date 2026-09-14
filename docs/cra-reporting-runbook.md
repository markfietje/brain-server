# CRA Art 14 Incident & Vulnerability Reporting Runbook

> **The clock:** Regulation (EU) 2024/2847 (CRA) Art 14 reporting obligations
> apply from **2026-09-11** (Art 71(2)). This runbook is the operator's drill
> card for the three statutory clocks. It is deliberately short: in the window
> you have no time to read a manual — you need the template, the channel, and
> the checklist. Pinned by `reg_watch_cra_pin_is_green` +
> `reg_watch_runbook_clock_anchor` in `src/reg_watch.rs` (the calendar as code
> — if this file loses its anchors, CI goes red).
>
> Rehearsal: `scripts/cra-report-drill.sh` (timed tabletop; baseline record at
> the bottom of this file).

## When this runbook fires (trigger taxonomy)

Two distinct triggers, two clocks, one channel pair:

| Trigger | Definition | First clock |
|---|---|---|
| **Actively exploited vulnerability** | A vulnerability in a shipped brain-server version (or a pinned dependency in its SBOM) that is being exploited in the wild — a public exploit exists, or compromise is observed/inferred | **24 h** early warning |
| **Severe incident** | An incident having an impact on the security of a supported deployment: confirmed compromise, supply-chain compromise of a release artifact, or a breach of the audit-evidence chain that a customer relies on | **24 h** early warning |

Not reportable under Art 14 (fix normally, document normally): vulnerabilities
not exploited in the wild and without an incident; internal near-misses caught
by the gates; experimental-branch issues in unshipped code.

## The three clocks

The first two run from **awareness** (the moment the operator/manufacturer
becomes aware of the vulnerability/incident — log the timestamp, everything
else hangs off it). The final report does NOT: its clock anchors on the
trigger (vulnerability → the fix/mitigation becoming available; severe
incident → the 72 h notification). That split is the L7-01 correction —
one month was never the vulnerability trigger's final-report clock.

## 24-hour early warning

- **What:** the short-form early warning — "we are aware, here is the shape."
- **Contains:** affected product + versions (from the release matrix below),
  a one-paragraph description, the suspected impact, and whether exploitation
  is observed. Unknown fields are filled with `unknown (under assessment)` —
  the early warning is not blocked by incomplete facts.
- **To:** ONE submission via the CRA **single reporting platform** (Art 14(1):
  the platform's electronic notification end-point of the CSIRT designated as
  coordinator, simultaneously accessible to ENISA). See channel table.
- **Template:** `scripts/cra-report-drill.sh` emits a filled sample from this
  section; keep the shape stable so downstream automation can parse it.

## 72-hour notification

- **What:** the updated notification — the early warning refined with the
  initial assessment: severity (CVSS or documented equivalent), root cause,
  indicators of compromise (if any), and the mitigation/containment already
  shipped or advised.
- **To:** the same coordinator-CSIRT + ENISA pair, referencing the early
  warning's submission receipt so the clocks visibly chain.

## Final report

The final report's clock depends on the trigger (final-OJ numbering,
re-verified 2026-09-14 vs the EUR-Lex full text + the Commission reporting
page):

- **Actively exploited vulnerability** (Art 14(2)(c)): due **no later than
  14 days after a corrective or mitigating measure is available** — the clock
  anchors on the FIX, not the notification. Log the fix-availability moment
  the way you log awareness.
- **Severe incident** (Art 14(4)(c)): due **within one month after the
  submission of the incident notification** (the 72 h notification under
  point (b) of that paragraph).

- **What:** the closure report: root cause, full timeline (awareness →
  containment → fix → release), the remediation shipped (version + signed
  release), lessons applied to the secure-development process, and evidence
  cross-references (SBOM version, audit-drill records). The coordinator CSIRT
  may also request an intermediate status report at any point (Art 14(6)).
- **To:** the same channel pair.

## Channels

| Channel | When | How |
|---|---|---|
| **Single reporting platform** → CSIRT designated as coordinator + ENISA | every Art 14 report (all three clocks) | the ENISA-operated platform (live from 2026-09-11): ONE submission reaches the CSIRT designated as coordinator for the manufacturer's **main establishment in the Union** — NOT the deployment's member state — and ENISA simultaneously (Art 14(1), 14(7)). Non-EU manufacturers fall back through the authorised-representative → importer → distributor chain (Art 14(7)); the operator submits under the manufacturer identity registered in SUPPORT.md |
| **GitHub Security Advisory** (private) | inbound vulnerability intake (pre-Art 14) | SECURITY.md §"Report a vulnerability" — the intake that STARTS the clock |
| **Downstream deployers** (release notes + SECURITY feed) | fix availability | signed release + advisory; never the only channel for a live incident. NOTE: for the vulnerability trigger this moment ALSO starts the 14-day final-report clock |

**Operator blank — fill at deploy time:** coordinator CSIRT for this
manufacturer (main establishment in the Union; if the platform's end-point
list has not been consulted recently, re-check it):
`________________________________` (endpoint/contact), verified on: `________`.

## Artifact checklist (what you assemble before sending)

Everything Art 14 asks for already exists in this repo's machinery — the drill
proves you can assemble it inside the clock:

- [ ] **SBOM** for the affected release: `scripts/sbom.sh` (CycloneDX JSON) or
      `scripts/cra-kit.sh` for the whole bundle
- [ ] **Affected-version matrix**: `CHANGELOG.md` release list — which shipped
      versions contain the vulnerable code, which contain the fix
- [ ] **Containment statement**: the workaround/mitigation paragraph
      (config-level mitigations from `docs/deployment.md` where applicable)
- [ ] **Signed release or advisory reference**: the fix release tag + its
      signed-artifact verification path (`scripts/release.sh` output)
- [ ] **Evidence integrity proof**: `GET /audit/verify` → `{"ok":true}` from
      the affected deployment (or the explicit statement that the chain is
      part of the incident)
- [ ] **Awareness timestamp** and the per-clock submission receipts

## Role call (operator roles, honestly named)

brain-server is operator-deployed; the "manufacturer roles" below are the
operator's hats, not a staffed org chart. Name them per deployment:

| Role | Who (fill in) | Does |
|---|---|---|
| Clock keeper | ____________ | stamps awareness, owns the 24 h/72 h/final deadlines, files the submissions |
| Technical writer | ____________ | drafts the three reports from this runbook + the artifact checklist |
| Approver / signer | ____________ | signs the submission (and the final report) — MUST be a human (HITL law; a report is an irreversible external act) |
| Dispatcher | ____________ | submits to ENISA + CSIRT, records receipts, informs affected deployers |

## Drill record (baseline)

`scripts/cra-report-drill.sh` runs the tabletop end-to-end against a fabricated
actively-exploited-vulnerability notice: it stamps wall-clock at every step,
fills the 24 h template, and prints a timing report. Run it once per release
train (and after any runbook edit); paste the timing output below so the next
incident starts from a measured baseline, not an estimate.

Baseline drill of record: see `docs/THROUGHPUT_PROOF_20260905.md` §CRA drill
(the v1.28.58 "Throughput" release drill, 2026-09-05).
