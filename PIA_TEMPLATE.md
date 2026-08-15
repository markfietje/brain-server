# Privacy Impact Assessment (PIA) Template — pre-filled (v1.25.0 M3)

> NPC 2024-04 expects a PIA for cross-border + AI processing. This template is
> **pre-filled from the live config** so the operator's PIA is not a blank page;
> the DPO completes the narrative sections (marked **[narrative]**). Pre-filled,
> not auto-filed — legal review required.

## 1. What data does brain-server process?

- **Kinds:** fact, episodic, procedure, step, decision (the `memory_kind`
  vocabulary, populated by ingest + connectors).
- **PII posture:** deterministic read-time output redaction + at-rest LUKS; a
  write-time placeholder vault is deliberately **not** a vault (see COMPLIANCE.md
  §vault). No raw PII is copied into pii_map.
- **Scope:** deny-by-default `private`; broader scopes are operator-assigned.

## 2. Lawful basis

- **[narrative]** Identify the lawful basis per processing purpose.
- Scraped data: must carry a documented `lawful_basis` tag or it is
  **quarantined** (v1.25.0 M3, NPC 2026-01 posture) — never stored undocumented.
- A principled basis/purpose schema atop the tag is v1.26.0.

## 3. Retention (storage limitation)

- Per-kind retention defaults: fact 365d, episodic 30d, procedure/step/decision
  730d (`src/config.rs`). Squashed at recall, purged only by explicit operator
  `/purge` or a DSAR (blocks on legal hold). Nothing is deleted autonomously
  beyond the audited decay flag.

## 4. Recipients / transfers

- **[narrative]** Local-first: storage is on the deployment host. A DSAR /
  recorder webhook (opt-in, signed) may reach a configured recipient.
- EO 119 gov-data residency: with `BRAIN_REGION=ph` + gov-classified data, the
  region stamp pins rows + the DSAR bundle to PH (`Feature:` eo119_residency).

## 5. Purpose limitation + minimization

- Retrieval is top-K semantic search (harmless abstention on low confidence) —
  data minimization by construction, not a wholesale dump.

## 6. Breach readiness

- v1.25.0 M2 breach workflow (DPO-opened, per-jurisdiction 72h deadlines,
  audit-chained). **[narrative]** Name the DPO + incident flow.

## 7. AI processing note

- HB 7396 is pending; the high-risk registration + risk-assessment duties map
  onto the existing profile + retention-class + audit primitives if enacted.
  **[narrative]** Revisit on enactment.