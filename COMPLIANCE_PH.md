# COMPLIANCE_PH — RA 10173 / NPC posture (v1.25.0)

> **Honest framing first:** the Philippines has **no dedicated AI statute yet**.
> AI is governed by **RA 10173 (Data Privacy Act 2012)** + NPC advisories
> (2024-04 AI; 2026-01 data scraping) + **EO 119 (July 2026, gov-data
> residency)**. **HB 7396** (a risk-based AI bill) is *pending*, not enacted.
> This annex documents the posture for the law **actually in force** (DPA +
> NPC advisories + EO 119) and is **structured to absorb HB 7396** when it
> passes — it is risk-based + high-risk registration, which maps onto the
> existing profile/role/retention primitives.
>
> **Not legal advice.** This is a documented control posture, not a legal
> opinion or a certification. Product the operator deploys to meet the DPA
> obligations; NPC registration / ISO 27001 / APEC CBPR certification remain
> the operator's external audits.

## Control map (RA 10173 + NPC advisories → shipped feature)

Each control names the feature it leans on; the cross-reference test pins that
`COMPLIANCE_PH.md` names every control in `ph::DPA_CONTROLS`.

### `pic_pip_duties`
**PIC/PIP duties** (controller/processor). The owner + scope model (v1.14.2)
stamps who is accountable for a record (`owner`), the read scope is enforced
at retrieval, and every write/erase is an append-only **audit-chain** event
(v1.15.0). `Feature:` access_scope/owner + audit-chain provenance.

### `privacy_by_design`
**Privacy by design (NPC 2024-04)**. Default `access_scope` is `private`
(deny-by-default); PII is write-time placeholder-redacted (`[redacted:*]`,
v1.14.2); retrieval is **data minimization** by construction (harmless
abstention + top-K, never a memory dump). `Feature:` deny-by-default scope +
placeholder redaction.

### `lawful_basis`
**Lawful basis + purpose.** Scraped data carries a documented **lawful basis**
or it is **quarantined, not stored** (the NPC 2026-01 scraping advisory
posture — v1.25.0 M3). A basis is a bounded free-text tag bound to a scrape
consent/purpose; a record with no basis never reaches memory. A principled
basis/purpose schema atop this tag lands in v1.26.0. `Feature:` scraped-data
provenance (this release).

### `subject_rights`
**Data subject rights** (access / correction / erasure). v1.15.0's DSAR
surface (locate → export → purge → certificate) satisfies RA 10173 rights
requests; the Art 17-style 30-day window + ledger deadline (v1.20.22) is the
Art 12-response-clock equivalent. `Feature:` privacy notice + v1.15.0 DSAR
surface.

### `npc_registration`
**NPC registration template.** An operator processing **500+ data subjects**
must register with the NPC (online portal with particular fields). Those fields
(controller name, contact, purposes, recipients, retention) map to the existing
Art 30 register (`GET /art30`) + `brain setup` profile wizard — an operator
checklist, not a certification. `Feature:` operator checklist on the 500+
subject trigger.

### `dpo_role`
**Data Protection Officer.** The `dpo` role (v1.23.0) runs DSARs and — new in
v1.25.0 — is the **only** actor who opens/closes a breach (the `dpo` role or
`admin` capability; role-gated). Contact surfaced on `/health` + the public
privacy notice via the `BRAIN_DPO_CONTACT` config. `Feature:` v1.23.0 dpo role
+ dpo_contact on /health.

### `eo119_residency`
**EO 119 gov-data residency.** When `BRAIN_REGION=PH` and the dataset is
gov-classified, the v1.22.0 region stamp pins every row + the DSAR
certificate/bundle to PH; the storage-in-PH requirement is documented here for
the operator to bind (`BRAIN_REGION=ph`). `Feature:` v1.22.0 region stamp
(BRAIN_REGION=PH).

### `hb7396_forward`
**HB 7396 forward-watch.** The risk-based bill is a dated note, not a law:
if enacted, its high-risk registration + risk-assessment duties map onto the
existing profile + retention-class + audit primitives. It ships **unimplemented
by design** (no pre-implementation of an unenacted requirement); revisit on
enactment. `Feature:` profile/retention/roles absorb the risk-based bill.

## Breach-notification workflow (v1.25.0 M2)

PH DPA + most client laws require breach notification within a bounded window
(PH: **72 hours** to the NPC + affected). The breach workflow is
**human-opened by the DPO role** (automated detection is a v2.x monitoring
concern):

- `POST /breach` opens an incident `{scope, description, severity,
  discovered_at, affected_estimate, jurisdictions?}`.
- `POST /breach/{id}/event` appends `notification | assessment | note` lines
  (append-only; the notification log records the per-jurisdiction deadline).
- `POST /breach/{id}/close` closes it; `GET /breaches` / `GET /breaches/{id}`
  are the DPO + auditor ledger.
- Deadlines are computed by jurisdiction from `discovered_at` (PH NPC 72h,
  EU Art 33 authority 72h, subject-notification per law) and surfaced as
  countdowns in the client Security panel.
- Every breach event is **hash-chained into the existing audit**
  (`kind: "breach"`), the tamper-evident record.

**Honest ceilings:** human-opened (no anomaly/leak sensor — v2.x); a
jurisdiction absent from the deadline table yields no deadline (the DPO
confirms per the DPA); each BPO client's jurisdiction is the v1.26.0
cross-border follow-up.

## Scraping provenance (v1.25.0 M3)

Per the NPC 2026-01 advisory, scraped data without a documented lawful basis is
**quarantined** (the v0.9.7 quarantine flag — excluded from recall, KG, and
export), never silently stored. A scrape ingest with a documented, bounded
`lawful_basis` stores normally.

## PIA (v1.25.0 M3)

NPC 2024-04 expects a **Privacy Impact Assessment** for cross-border + AI
processing. `PIA_TEMPLATE.md` ships a pre-filled template (what data, lawful
basis, retention, recipients, transfers) drawn from the live config — the DPO
completes the narrative; it is pre-filled, not auto-filed.

---

**Status:** conformance posture documented for the law in force. Certification
(NPC registration, ISO 27001, APEC CBPR) is the operator's external audit.
Revisit **HB 7396** on enactment.