# CRA Art 14 Incident & Vulnerability Reporting Runbook

> **The clock:** Regulation (EU) 2024/2847 (CRA) Art 14 reporting obligations
> apply from **2026-09-11**. This runbook is the operator's drill card for the
> three statutory clocks. It is deliberately short: in the window you have no
> time to read a manual — you need the template, the channel, and the
> checklist. Pinned by `reg_watch_cra_pin_is_green` in `src/reg_watch.rs`
> (the calendar as code — if this file loses its anchors, CI goes red).
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

All three run from **awareness** (the moment the operator/manufacturer becomes
aware of the vulnerability/incident — log the timestamp, everything else hangs
off it):

## 24-hour early warning

- **What:** the short-form early warning — "we are aware, here is the shape."
- **Contains:** affected product + versions (from the release matrix below),
  a one-paragraph description, the suspected impact, and whether exploitation
  is observed. Unknown fields are filled with `unknown (under assessment)` —
  the early warning is not blocked by incomplete facts.
- **To:** ENISA via the EU reporting portal, AND the national CSIRT/PSIRT of
  the deployment's member state (see channel table).
- **Template:** `scripts/cra-report-drill.sh` emits a filled sample from this
  section; keep the shape stable so downstream automation can parse it.

## 72-hour notification

- **What:** the updated notification — the early warning refined with the
  initial assessment: severity (CVSS or documented equivalent), root cause,
  indicators of compromise (if any), and the mitigation/containment already
  shipped or advised.
- **To:** the same ENISA + CSIRT pair, referencing the early warning's
  submission receipt so the clocks visibly chain.

## Final report

- **What:** the closure report, due **no later than one month** after the 72 h
  notification (and on request of ENISA/CSIRT): root cause, full timeline
  (awareness → containment → fix → release), the remediation shipped
  (version + signed release), lessons applied to the secure-development
  process, and evidence cross-references (SBOM version, audit-drill records).
- **To:** the same channel pair.

## Channels

| Channel | When | How |
|---|---|---|
| **ENISA** (EU agency portal) | every Art 14 report (all three clocks) | the EU reporting portal entry point published under CRA Art 14(2); operator submits under the manufacturer identity registered in SUPPORT.md |
| **National CSIRT / PSIRT** | every Art 14 report (all three clocks) | the member state where the affected deployment is operated. The live install's CSIRT is the operator's own jurisdiction — record it HERE in the blank below at deploy time, not during an incident |
| **GitHub Security Advisory** (private) | inbound vulnerability intake (pre-Art 14) | SECURITY.md §"Report a vulnerability" — the intake that STARTS the clock |
| **Downstream deployers** (release notes + SECURITY feed) | fix availability | signed release + advisory; never the only channel for a live incident |

**Operator blank — fill at deploy time:** national CSIRT for this deployment:
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
