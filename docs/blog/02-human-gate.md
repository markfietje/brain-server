# Human-in-the-loop, not "ask the model nicely"

*2026. The write gate, and why autonomy without a gate is how memory goes wrong.*

Every agent-memory product needs a write path. There are two ways to build it.

**The easy way:** the model stores what it thinks is worth remembering. This is
convenient and it is precisely how an agent's memory fills with noise, with
hallucinations, and with the output of a prompt-injection attack. There is no
gate because the model *is* the gate — and a model cannot reliably tell true
from false, important from trivia, or its own output from an attacker's.

**The hard way, and the one we chose:** a candidate is **proposed**, scored
deterministically, and promoted to memory **only when a human approves it**.
Autonomy stops at the proposal. Nothing becomes long-term memory without a
person saying yes.

## How it works

`POST /ingest/proposal` scores a candidate deterministically — no LLM:

- **Novelty** — how far is this from what's already known? (1 − max cosine over
  current chunks.)
- **Conflict** — does it contradict something on record?
- **Salience** — is it long enough to matter and rich in entities?

It creates **no memory row**. It sits in a review queue. It becomes memory only
via `POST /proposals/{id}/approve` (one transaction, optionally atomically
superseding an old fact), or it is rejected, or it **expires** — the proposal
TTL (`BRAIN_PROPOSAL_TTL_SECS`, default 7 days) auto-rejects stale candidates
so the queue can't rot.

For memory that's captured automatically (e.g. an agent plugin's autoCapture),
the default routes it *through* the same proposal gate rather than writing
directly — the escape hatch to `direct` is explicit, not the default.

## Why this is the right posture for 2026

The OWASP 2026 agentic frameworks (LLM03, ASI01) and every HITL (human-in-the-
loop) review-queue guide arrive at the same design rule: **write approval must
live outside the model's prompt.** An agent that can approve its own memory
writes is an agent whose memory is whatever an attacker convinced it to
remember. The gate pattern — propose, human-approve, promote in one transaction
— is the load-bearing control, and it's in the OWASP 2026 control matrix
(`docs/OWASP_AGENTIC_2026.md`).

## The honest trade

A human gate means memory updates are not instant. That's the point: it makes
memory **reviewable**, which is what turns a store into something you can
defend in a review. The operator console (`/ops`) shows the pending queue as a
clock — what's waiting, its SLA countdown, and the injection screen's verdict on
each item — so the gate is a workflow, not a black hole.

**The takeaway:** if your agent's memory can be written by the agent, then your
agent's memory is already untrusted. Gate the write, keep the human, and you
can actually answer "who decided this memory was true?" — because the answer
is a named human, recorded in the audit chain.

*See [`docs/research/06-abstention-verify.md`](../research/06-abstention-verify.md)
for how the read side is grounded too, and the proof map for the gate's live
repro.*
