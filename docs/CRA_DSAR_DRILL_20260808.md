# CRA 30-Minute Drill — DSAR Evidence (2026-08-08)

**Status:** COMPLETED · **Playbook:** `docs/AI_LITERACY.md` §"How a deployer
demonstrates literacy" step 3 (the report's CRA 30-minute drill deadline is the
same muscle). · **Server:** brain-server 1.16.7.

The drill executes the deletion workflow a data subject would exercise —
**locate → export → purge → certificate** — against test subject data, so the
operator is literate in the deletion path before a real request arrives. This
file is the retained audit evidence.

## Environment

The drill ran against a **throwaway JWT-mode instance** so no live data was
touched:

- Port `18765` (`BIND_HOST=127.0.0.1`, `BIND_PORT=18765`), temp DB
  (`/tmp` scratch), temp RSA key (`BRAIN_JWT_KEY_DIR`, `drill-kid`).
- JWT mode: `BRAIN_JWT_ISSUER=https://drill.test`, audience `brain-server`.
- Test subject (owner = JWT `sub`):
  `cra-dsar-drill-20260808@example.test`
- Token: RS256 access token, scopes `["admin:*/*"]` (DSAR is Admin-gated).

## Workflow executed (end to end)

| Step | Action | Result |
|---|---|---|
| 1 | `POST /ingest` test memory as subject | HTTP 200, `knowledge` id 1 |
| 2 | `POST /dsar {subject, action:"both"}` | HTTP 200, `status: completed` |
| 3 | `GET /dsar/2/certificate` | HTTP 200, `chain_verifies: true` |
| 4 | `GET /get/1` after purge | HTTP 404 (`chunk not found`) — row gone |
| 5 | `GET /tombstones?subject=` | 1 row, reason `owner:<subject>` |
| 6 | `GET /audit/verify` | `{"ok":true}` — chain intact |

## Deletion certificate (recorded)

```json
{
  "certificate": {
    "action": "both",
    "certified_at": "2026-08-08T14:51:35.033880+00:00",
    "chain_head": "11245d78da10e85d61f32fd1c972754285bed4db760e00f01feb6bf47e35f383",
    "found_count": 1,
    "purged_ids": [1],
    "subject": "cra-dsar-drill-20260808@example.test",
    "tombstone_root": 1
  },
  "chain_verifies": true
}
```

`tombstone_root: 1` anchors the deletion into the SHA-256 audit chain; the
subsequent `/audit/verify` returns `ok:true`, so the purge did not break the
chain.

## Honest finding (surfaced during the drill)

The drill initially ran with `found_count: 0`. Root cause: **no ingest path
persists `knowledge.owner`.** `dsar_locate` locates rows by `owner = <subject>`,
but `/ingest` (and the other ingest routes) never write the `owner` column;
`principal_to_owner` is only wired into the `/purge` handler, not ingest. On a
normal DB, a real DSAR therefore locates **nothing** — the locate leg is
effectively non-functional in the current build. The drill only located the row
after the operator seeded `owner` directly on the test row in the throwaway DB.

**Impact:** this is a correctness/compliance gap in the v1.15.0 DSAR workflow,
not a drill artifact. Recommend wiring `principal_to_owner` into the ingest
write path (and the connector / markdown / memory ingests) as a v1.17+
correctness item — it is a prerequisite for `per-kind retention` and for any
real DSAR locating records by subject.

## Drill verdict

The deletion **workflow** (locate → export → purge → tombstone → certificate →
chain-verify) works end to end and is evidenced above. The **locate-by-owner**
data dependency is broken in the current build and is tracked as the finding
above. Operator is literate in the path; a real drill rerun is recommended once
the ingest-owner wiring lands.
