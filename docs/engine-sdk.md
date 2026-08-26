# Engine SDK

`crates/brain-engine-sdk` is the **stable engine ABI** for the governed
workflow: engine cores compile against this crate — never against a
brain-server binary. The server is the first host adapter; the same contract
lets any transactional backend drive a workflow core.

Authoritative detail lives in the crate's own
[`README.md`](../crates/brain-engine-sdk/README.md); this page is the map.

## Surface

| Module | What it is |
|---|---|
| `pure` | Deterministic, dependency-free cores — `evidence` (claim-grouping reducer), `qa_score`, `complaint` (role-tier remedy approval caps, v1.28.34), `consent` (v1.28.35). Oracle-pinned, deterministic output order. |
| `policy` | Law/compliance vocabulary as pure data: P-class SLA TTL table (`stamp_envelope`) + default per-kind retention days (fact 365 / episodic 30 / procedure·step·decision 730 / entitlement 1825). Hosts facade it verbatim and layer env overrides. |
| `host` | The storage seam engines write through: `tx()` unit of work, idempotent `enqueue`, CAS state advance, in-tx audit rows. Dropping a unit rolls back everything. |

## Guarantees

- Every mutating call emits its audit row **inside the same transaction** —
  no transition without evidence.
- Value-typed signatures; the SDK never opens a database and has zero
  dependencies; `unsafe` is forbidden crate-wide.
- Versioning: minor bumps add items; removals/reshapes are breaking releases.
  `sdk::VERSION` + `requires_host(min)` gate compatibility at wiring time;
  engines pin the minor line they compile against.

## The engine crates

The workspace ships focused engine crates that build on the SDK's pattern —
some filled, some deliberate scaffolds so the graph stays green:

- Filled: `brain-aftersales-core` (dispositions/evidence/gates),
  `brain-interview-core`, `brain-troubleshoot-core`, `brain-care-core`,
  `brain-fuzz` (corpus replay).
- Scaffolds (lib-only by design): `brain-consensus-core`,
  `brain-executor-core`, `legal-rules-db`.

The harness reference implementation lives in `tools/steward-harness`
(see [API reference — workflow](api.md)).
