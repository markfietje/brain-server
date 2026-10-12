# Agent write posture (who may write memory)

The invariant behind the whole write path is one sentence: **the human
decides what becomes memory.** This page is the operational contract that
makes the sentence true for every caller class, on every write surface.

## The caller classes

| Class | Who | Writes |
|---|---|---|
| Operator (opaque bearer) | The human running the server; `None` principal | Direct inserts under `BRAIN_WRITE_POSTURE=open` (the default); proposals under `review` |
| Scoped JWT (human) | Whatever its scopes/roles grant | Direct inserts where the scope admits, per the posture knob |
| Scoped JWT carrying the `agent` role | A modeled agent | **Proposal-only, under both postures** |
| `AgentLoopback` (the two-lane token file's line 2) | The gateway's standing agent identity | **Proposal-only, under both postures** |
| Capability bearer | UMP-scoped tokens (`ump.recall`/`get`/`feedback`) | Read + feedback only; no admin verb exists in the vocabulary |

The class boundary is one seam — `effective_write_posture(principal)` — and
its own doc comment is the law: the knob keeps its name, its `open` default
and its meaning for everyone else, but for a recognized agent class the
answer is `review` regardless of the environment, because a default launch
(`cargo run`, a hand-started binary) is `open`, and an env-dependent
invariant is not an invariant.

A role-less JWT with write scope is a **deliberate ceiling**, not an
oversight: the server cannot tell an agent from a human holding the same
token, so a deployment minting agent JWTs without the `agent` role relies
on the posture knob — which still works, and is still the operator's call.

## The write surfaces

Every agent-callable surface that can change what current memory IS reads
the class seam:

| Surface | Agent class | Everyone else |
|---|---|---|
| `POST /ingest` (and `/add`, `/ingest/markdown`, capture) | proposal (202) | posture knob |
| `POST /ingest/proposal` | proposal | proposal (it IS the proposal door) |
| `POST /ump/remember` | proposal | posture knob |
| `POST /ump/revise` | proposal (supersession deferred to approval) | posture knob |
| `POST /procedure` | **refused (`403`)** — procedures are operator-authored, later EXECUTED by decision evaluation, and their root + ordered steps cannot ride the proposal wire | posture knob |
| `POST /consolidate/apply` · `/undo` | **refused (`403`)** — a supersession decides what current memory IS, and the lane has no approver; a proposal cannot express the link | posture knob |
| `/proposals/{id}/approve` · `/reject` | **refused** — approval and disposal are role acts (`approve`/`reject`), and the agent preset holds neither | roles that declare them |

The matrix (`tests/authz_matrix.rs`) drives every route × the agent class
through the composed app, so a new surface that "forgets the class" fails
CI, not an audit.

## Two decisions worth knowing why

- **The agent preset holds `read` + `write` but neither `approve` nor
  `reject`.** Approving promotes; rejecting prunes the human review queue —
  including of a human's queued capture. Both ends of the review decision
  are the reviewer's, not the reviewed's.
- **Legal hold outranks consolidation.** `resolve_supersession` refuses a
  held chunk before any mutation, exactly like every erasure path: held
  evidence must not be destroyed by routine supersession either.

## Erasure stays human too

`/purge` requires Admin scope + the `purge` role; capability bearers are
always refused (no admin verb exists in their vocabulary); legal holds
fence every path. The kill-switch (`/ops/agents/revoke`) refuses the opaque
operator bearer loudly (`operator_bearer_unrevocable` — a static token is
unkillable from inside; rotation + restart is the remedy) while always
writing revocation rows for the classes the switch can reach.
