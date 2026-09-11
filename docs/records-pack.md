# Records of processing, breaches, and transfers

This pack answers three regulator questions from live data instead of
spreadsheets: what processing exists, what broke, and what crossed a
border. Everything here is HTTP-only except RoPA, which also has a CLI.

## Article 30 register (`GET /art30`, Admin)

A read-only projection over the running system: processing categories
with counts by memory kind, purposes, retention posture, recipients
(webhook and connector sinks), transfer legal bases, DSAR history,
lifecycle split (live / superseded / tombstoned), and which provenance
fields are populated. It reflects the database, not a form someone
filled in last quarter.

## RoPA records (`GET /ropa`, `POST /ropa`, `POST /ropa/{id}`)

One row per processing activity: activity, controller, processor,
lawful basis, data categories, recipients, retention days, security
measures, transfers. Creation is Admin-only and audited; incomplete
submissions get `400 ropa_incomplete`. CLI: `brain ropa list`,
`brain ropa add` with the matching flags.

## Breach ledger (`POST /breach`, `/breach/{id}/event`,
`/breach/{id}/close`, `GET /breaches`, `GET /breaches/{id}`)

Recording a breach returns the notification deadlines computed from the
discovery time and the declared jurisdictions, so each clock is visible
from the first minute. Follow-up is an append-only event chain
(notifications, assessments, notes), hash-chained like everything else;
closing is explicit and idempotent. Recording is manual. The server does
not detect breaches on its own, and the page says so.

## Transfer register (`POST /transfers`, `GET /transfers`,
`GET /transfers/{id}/tia`, `GET /transfers/{id}/dpa`)

Each cross-border flow records dataset, origin and destination
jurisdictions, mechanism (SCCs, UK IDTA, DPF, CBPR, BCR, or adequacy),
counterparty, lawful basis, and purpose. The TIA endpoint pre-fills a
Schrems-II-shaped assessment from the row and public posture data; the
DPA endpoint pre-fills Article-28 sub-processor fields. Both are evidence
artifacts for a lawyer to review, not legal judgments by software.
Client-level DPAs ride `brain client dpa get|set`.
