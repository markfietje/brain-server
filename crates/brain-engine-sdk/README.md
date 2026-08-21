# brain-engine-sdk

The stable engine ABI for the governed-workflow harness: engine cores compile
against this crate — never against a server binary.

## Surface

- **`pure`** — deterministic, dependency-free cores: `evidence` (the
  claim-grouping reducer) and `qa_score` (integer ten-thousandths scoring,
  cause split, flywheel proposals). Oracle-pinned; output order deterministic.
- **`policy`** — law/compliance vocabulary as pure data: the P-class SLA TTL
  table (`stamp_envelope`) and the default per-kind retention days. Single
  owner; hosts facade it verbatim and layer env overrides on top.
- **`host`** — the storage seam engines write through:

```rust
use brain_engine_sdk::host::{AuditKind, AuditStatus, WorkflowHost, WorkflowHost as _};

fn drive(host: std::sync::Arc<dyn WorkflowHost>) -> Result<(), brain_engine_sdk::host::HostError> {
    let unit = host.tx()?;                                  // opens a unit of work
    host.enqueue(1, "intake", r#"{"a":1}"#, "k-1")?;        // idempotent by key
    host.cas(1, 0, r#"{"a":1,"ok":true}"#)?;                // compare-and-swap
    host.audit(AuditKind::Workflow, "engine", "run:1", AuditStatus::Ok, "step");
    unit.commit()                                           // dropping rolls back
}
```

## Guarantees

- Every mutating call is atomic in itself and emits its audit row inside the
  same transaction; a dropped unit leaves no transition and no audit row.
- All signatures are value-typed — any transactional backend can implement
  `WorkflowHost` without an ABI change. The SDK never opens a database and
  has zero dependencies.
- `unsafe` is forbidden crate-wide.

## Versioning

Minor bumps add items; removals or reshapes are breaking releases.
`sdk::VERSION` + `sdk::requires_host(min)` gate host compatibility at wiring
time. Engines pin the minor line they compile against.
