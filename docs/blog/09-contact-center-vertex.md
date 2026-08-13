# Agent memory for a contact center: what has to be true before you trust it

*A buyer's-eye look at why a support/contact-center deployment can't use "just
any" agent memory — and the controls that have to be real. Grounded in shipped
code; the tenancy ceiling is stated honestly, not hidden.*

A contact center runs on its memory of past resolutions. A customer calls about
a billing issue; the agent who last fixed it is gone; the knowledge base holds
the policy but the *resolution path* lives in transcripts and ticket history.
Agent-assist memory is the obvious answer: give every agent an AI that recalls
"How did we resolve this exact case before?" But a support center is not a
hobbyist's chatbot. Before that memory earns a seat in the operation, four
things have to be true — and a lot of memory products quietly fail one of them.

## 1. It has to recall without fabricating

In a support center, a wrong memory is not a curiosity — it's a compliance
incident or a lost customer. Recall that "confidently returns top-1 garbage" is
worse than no recall at all. So the retrieval has to be **deterministic** and
**reference-faithful**: the agent should get the actual span of what was
recorded, cite it, and be told when the answer is *not* confidently in memory.

Brain Server does this with **calibrated abstention** — when retrieval quality
is too low, it returns "I don't know" (`low_confidence`, no hits) instead of a
fabricated top-1 — and **span verification**, a deterministic check that a claim
is literally present in the stored text before an agent acts on it. There's no
LLM deciding what to recall, so there's no "the model made it up" failure mode
at the memory layer.

## 2. Client data has to stay where your client contract says it stays

A BPO serves many clients. Client A's account data and Client B's must not
mingle — in the data, in the answers, or in the egress. That means memory that
**stays on-prem** and is **scoped** per domain/account, with per-agent opt-in and
chat-type gating so private memory never surfaces in a shared queue.

Brain Server is loopback-first and offline-capable: memory lives on the
operator's own host, there is **no telemetry and no data egress by default**,
and per-domain isolation with centroid auto-routing keeps one account's memory
from leaking into another's answers. The per-query cost is zero because there's
no embedding API — embeddings are a local static model.

## 3. Nothing enters memory without a human signing it

Support memory that an agent can silently write is memory a hostile prompt can
poison. Every capture should be **proposed, scored, and admitted only on human
approval** — and an injection screen should quarantine adversarial input before
it ever reaches a reviewer.

Brain Server's write path is a **gate, not a path**: a captured fact is scored
(novelty / conflict / salience) and *proposed*; it becomes memory only when an
operator approves it. The injection screen flags suspicious content before the
human gate. And the erase side is human-only — an agent can read and propose,
but cannot delete memory.

## 4. It has to survive the auditor

A support deployment eventually faces the question "what did the system know,
when, and why?" That requires a **tamper-evident audit chain**, **replayable
recall traces** (what exactly was injected into a given turn), and a **DSAR**
path that can locate, export, purge, and issue a deletion certificate.

Brain Server writes every decision to a **SHA-256 hash chain** that `/audit/verify`
proves end-to-end. DSARs produce chain-verifiable deletion certificates. PII is
redacted deterministically at read time. Those are the same controls a finance,
healthcare, or public-sector buyer asks for — because they're the same controls.

## The honest ceiling

What Brain Server ships today is a **single-node** memory server: the *controls*
above (isolation, audit, DSAR, PII, human gate) are real and shipped. What is
**not** shipped yet is **multi-client tenancy on one shared backend** — running
Client A and Client B as isolated tenants in a single multi-tenant service.
That is the roadmap's **v2.0 "Cortex"** milestone (call-center intelligence:
multi-team tenancy, ticket-pattern resolution, cross-domain skill seeding). So:

- **If you need a single trusted node per client** — ship today's binary per
  tenant, and you get full isolation, audit, DSAR, and PII containment.
- **If you need one shared, multi-tenant platform across many clients** — that
  packaging is v2.0, not today. We say so plainly because a support-center
  buyer should never discover a hard ceiling after the contract.

## Why we're telling you this

A contact center is exactly the deployment where the four controls above stop
being "nice to have" and become load-bearing. We built them into the OSS line —
not behind a paywall — because a memory store that only becomes auditable and
human-gated after you license it is not a memory store a support center should
trust with client data. The product tells you its limits; that's the point.

*See **[Who it's for — target audiences](../audiences.md)** for the full segment
map, **[Human in the loop §7 — the erasure procedure](../human-in-the-loop.md#7-the-erasure-procedure)**
for the exact, audited path an operator/QA/Admin follows to delete memory (and why the
friction is by design), and `COMPLIANCE.md` / `SECURITY.md` for the controls behind each
claim. The tenancy ceiling is tracked on the roadmap's v2.0 "Cortex" row.*
