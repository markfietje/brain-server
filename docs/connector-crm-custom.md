# connector-crm-custom — the "any other CRM" escape hatch

v1.28.22 "Bridges" ships Zendesk, Salesforce, and Genesys Cloud as code.
Vertical CRMs with a REST surface (Freshdesk, ServiceNow, Jira Service
Management, …) are **configuration, not code** — bounded to the same
`CrmCase` contract every built-in source speaks.

## The contract (fixed)

Every CRM source, built-in or custom, normalizes to the `CrmCase` shape
(`src/connector/crm/mod.rs`):

| Field | Meaning |
|---|---|
| `source` | `"zendesk"` \| `"salesforce"` \| `"genesys"` \| your custom label |
| `org_id` / `case_id` | the CRM organization/instance (tenant key) + the vendor's stable case id |
| `case_ref` | stable `crm:{source}:{org}:{id}` — the run-linkage key |
| `title`, `status` (`open`/`closed_solved`/`merged_away`), `priority` (optional, verbatim vendor string) | envelope. `merged_away` is the merged-ref state a vendor surfaces on ticket/case/workitem merges (see `merged_into`) |
| `subject_ref` | salted SHA-256 of customer identity — never raw PII |
| `updated_rev` | vendor revision marker (idempotency key input) |
| `updated_at` | vendor last-update timestamp (ISO-8601), verbatim |
| `body_markdown` | case description (untrusted; enters via proposals) |
| `is_seed` / `is_not_seed` | optional structured symptom seeds |
| `merged_into` | the SURVIVING case's vendor id when this ref was merged into another (`None` unless merged) |
| `reopened` | `true` when a previously-closed workitem reopened (the re-ask source; Zendesk/Salesforce merges ride `merged_into` instead) |

## What ships today

The custom path ships as **this document + the pure mapping tests only**.
There is deliberately **no generic JSONPath runtime** in brain-server or the
connector binary — a config-driven field-extraction engine is an injection
hole, not a feature.

## Wiring a vertical CRM (operator recipe)

Until a per-vendor module exists, drive `brain-connector-crm` against any
REST CRM by writing a thin shim script that:

1. Polls the CRM's list endpoint (respect its rate limits — 300s cadence
   floor like the built-ins).
2. Emits one JSON object per case matching the contract table above.
3. Pipes it to `brain ingest` (the CLI) or `POST /ingest?format=ump` — under
   `BRAIN_WRITE_POSTURE=review` the body lands as a proposal, exactly like
   the built-in connectors.
4. Opens/reuses a run via `POST /workflow/runs` with
   `state_json = {"case_ref": "crm:yourcrm:{org}:{id}", "origin": "crm-connector"}`
   and posts `crm/case/updated` / `crm/case/closed` events on it.

Config lives beside the built-ins as `custom-*.json`
(`{base_url, auth_type, case_list_path, case_detail_path}`), mode-checked
0600 by the same secret-file posture. Secrets ride in separate `*_file`s.

## Honest ceiling

A future release may promote the most-requested shapes (Freshdesk,
ServiceNow) to tested vendor modules following the three shipped ones — each
is ~150 lines of pure mapper + URL builders over `VendorTransport`. The
generic field-mapping runtime stays out permanently.
