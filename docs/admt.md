# ADMT — Automated Decision-Making Transparency

> v1.20.10 "Proof" — a read-only assembly for the question "why did this become
> memory, by what path, from what source?" Each decision that turns a proposal
> into memory is human-approved (v1.14 Gate); this kit surfaces the decision's
> own recorded trail.

## The record

```
scripts/admt-kit.sh <chunk-id> [--out DIR]
```

requires a running server + a read-token (default `~/.config/brain-server/auth-token`,
override `BRAIN_TOKEN_FILE`). It calls **existing, already-audited endpoints**
and assembles them verbatim — it fabricates nothing:

| Field | Source | Meaning |
|-------|--------|---------|
| `decision_evidence` | `GET /get/{id}` | the chunk's `origin` (v1.18.2 provenance), `owner`, title, evidence span |
| `decision_path` | `GET /audit?kind=reconcile` | the proposal-gate trail — `proposal:{id}` approve/reject rows |

The audit rows come from the tamper-evident **hash chain** (verified by
`/audit/verify`); the `/health` `integrity.chain_ok` posture (v1.20.10) says
whether that chain currently verifies. Together: *who approved it, from what
source, against an unbroken chain.*

## Why this is trustworthy (not a re-derivation)

- **No new computation.** Every field is copied from an already-served JSON
  response; the record can be diffed against the live endpoints at any time.
- **No new authority.** It inherits the server's existing integrity posture —
  it cannot vouch for a chain the server itself reports as broken.
- **PII-safe.** Proposals were PII-redacted at write time (v1.20.1); `/get/{id}`
  reveals `owner` only through the operator's own read token. The record carries
  provenance + gate rows, never secret content.

## Honest ceiling

- The audit rows are **records of the decision**, not a causal/score model of
  *why* the reviewer approved. Explainability beyond the gate trail (e.g. the
  exact scoring signals that ranked a proposal) is a separate, future surface.
- `chain_ok` reflects the integrity watcher's *last* full verify (default 60s),
  not a live per-request scan.
