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

The workspace ships focused engine crates that build on the SDK's pattern.
The classification below is **machine-checked** by
`engine_sdk_crate_map_is_accurate` in `src/docs_truth.rs`, which fails when a
named crate does not exist on disk, when the SDK itself is missing from the
list, or when a crate the server actually calls is still called a scaffold.

**Filled** — carries a decision core and is called:

- `brain-engine-sdk` — the SDK this document describes (`pure`/`policy`/`host`;
  13k+ lines, ~190 tests). Listed here because the crate list that omitted it
  was the doc's own subject.
- `brain-delivery-core` — autonomy tiers, phase machine, promotion gate,
  attestation predicate, budget ledger, replay comparator, release-status
  machine. Pure, no I/O. **Called by `src/workflow/delivery.rs` since the
  delivery-persistence round** — an earlier revision of this line said "ungated:
  no callers yet", which that round made false.
- `brain-consensus-core` — `Artifact` (the typed artifact the delivery seam
  reuses), `Review`/`Verdict`, the capped `advance` state machine,
  `review_join_gate`, `approval_gate`, and `stage_writer`. Pure, no I/O.
  **Called by the delivery phase pass.**
- `brain-executor-core` — `Goal`/`parse_brief`, the `CheckpointGate` JSON
  validator, `RunState` with the named critic ceiling, `requires_delegation`,
  and `artifact_hash`. Pure, no I/O. **Called by the delivery phase pass.**
  Two honest ceilings, both pinned: `apply_steering` is a **declared no-op**
  (all six `SteeringKind` values are reserved vocabulary with no defined
  semantics against a two-field `Aggregate`, and the signature is infallible
  so it cannot report a failure it cannot have), and the `Goal`/`parse_brief`
  pair is the scope engine the design owner assigns to D3 rather than to the
  interview crate.
- `brain-aftersales-core` (dispositions/evidence/gates), `brain-interview-core`,
  `brain-care-core`, `brain-fuzz` (corpus replay).

**Filled, with a disclosed gap** — carries a decision core but has **no**
tests: `brain-troubleshoot-core` (advisor/evidence/gates/kernel/subagents).
Listed as Filled because it is called, not because it is covered.

**Scaffolds** (lib-only by design, no callers): `legal-rules-db`. It is the
largest remaining scaffold by line count, so the earlier grouping of it
alongside the two engine cores above was the clearest symptom of this
classification rotting.

The harness reference implementation lives in `tools/steward-harness`
(see [API reference — workflow](api.md)).
