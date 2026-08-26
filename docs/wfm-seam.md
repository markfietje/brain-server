# WFM Interop Seam (v1.28.40 "Handshake")

The first-party, versioned boundary between brain-server and any
workforce-management (WFM) tool. No interchange standard exists to adopt in
this space — so the seam is the standard: a documented, additive-only JSON
contract over the shift ring (Watchbill) and the HITL-maintained skills
registry. Vendor-specific Verint/NICE connectors are explicitly later work;
the generic CSV/JSON adapters (`brain wfm-import`) are what any WFM maps
through today.

## Endpoints

- `GET /ops/shifts?domain=&now=` — the shift ring view plus every stored
  shift for the domain (Read on the domain; capped at newest 500).
- `GET /ops/skills?domain=` — the skills registry grouped by principal
  (Read on the domain). Skills are HITL-maintained: this feed only READS.

## Change policy

**Additive-only.** Fields are added, never removed or renamed. A field may
be deprecated (kept emitted, documented as such) before removal in a NEW
schema version. Any breaking need means a new `wfm/<n>` constant, a change
log entry below, and a major consumer migration path. The
`wfm_schema_is_versioned_and_additive_only` test enforces the two-way pin:
server-emitted keys must match the declaration below exactly, and the
declared version must equal the shipped constant.

<!-- wfm-schema
GET /ops/shifts: schema_version,now,domain,queue_scope_site,incoming_site,in_overlap,next_boundary_epoch,shifts
shift object: id,domain,site,tz,start_epoch,end_epoch,overlap_minutes,roster
GET /ops/skills: schema_version,domain,skills
skills object: principal,skills
-->

## Import

```
brain wfm-import <file.csv|file.json> [--domain D] [--dry-run]
```

Shift rows land through `POST /ops/shifts` semantics (validation,
double-booking refusal, audit row in the same transaction). Skill rows NEVER
write the registry directly — each becomes one `crew_skills_update`
proposal a human approves (the only write path to `principal_skills`).

CSV grammar (deliberately tiny: no quoting, no embedded commas — use JSON
for anything richer):

    domain,site,tz,start_epoch,end_epoch,overlap_minutes,roster
    acme,manila,+08:00,1700000000,1700028800,60,op-a;op-b

    principal,skill
    op-a,billing

JSON adapters accept arrays of objects with the same fields (`tz`,
`overlap_minutes`, `roster` optional).

## Change log

### `wfm/1` — v1.28.40 "Handshake"

Initial version. Shift feed: ring view + stored shifts. Skills feed:
grouped registry read. Both stamped `schema_version: "wfm/1"`.

## Honest ceilings

- Gate-backlog attribution in `/ops/workload` rides only onto principals the
  domain's own lineage already surfaced (`proposals` has no domain column);
  no cross-tenant inference is performed.
- Fatigue signals are visibility for the scheduling human — nothing ever
  reassigns work automatically (G7's own posture, per ISO 18295-1).
- No forecasting, no adherence monitoring, no automatic queue reassignment.
- Vendor-specific connector parsing (Verint/NICE) is later work; these
  generic adapters are the 100%.
