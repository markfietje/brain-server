# Opt-in Anticipation (the Suggest surface)

**File:** `src/handlers/suggest.rs` (`suggest`, `feedback`, `metrics`) ·
`src/handlers/mod.rs` (`MAX_QUERY`)

## The problem

Passive recall answers only what you ask. Real productivity comes from the
store *surfacing* what is relevant to what you are working on *now* — before
you finish phrasing the question. But unsolicited, unprompted injection of
memory into an agent's context is dangerous (prompt-injection) and annoying
(false positives). The design tension is: **how do you get anticipation without
giving the store a push channel?**

## The reference

- **Generative Agents** — Park, O'Brien, Cai, Morris, Liang, & Bernstein
  (2023), *Generative Agents: Interactive Simulacra of Human Behavior*, UIST
  2023. Agent memory scored by recency / importance / relevance, with *reflective
  memory* synthesizing higher-level abstractions — the canonical "memory as a
  first-class agent component" architecture.
- **MemGPT / Letta** — Packer, Wooders, Lin, et al. (2023), *MemGPT: Towards
  LLMs as Operating Systems*, arXiv:2310.08560 (**preprint**, cite honestly).
  OS-style virtual-context paging between main and external context. The
  relevant lesson (cited in `src/handlers/suggest.rs`): *anticipatory memory
  must be reviewable* — nothing is silently injected.
- **Mem0** — the `feedback` API shape (`memory_id`, `feedback`,
  `feedback_reason?`) and feedback analytics that track accept vs. dismiss —
  the false-positive metric Brain Server mirrors.

## The implementation

The roadmap explicitly forbids unsolicited push, ranking decay, hidden
personalization, and SSE-by-default. What ships (v1.9.0) is deliberately
narrow and honest:

- **`POST /suggest`** — an **opt-in pull**. The caller supplies explicit
  context; the server returns *related-but-not-already-surfaced* chunks, each
  tagged `reason: "anticipated"`. Nothing is pushed; the agent decides whether
  to use a candidate.
- **`POST /suggest/feedback`** — Mem0-style `accept` / `dismiss` per surfaced
  chunk, recording which anticipations were useful.
- **`GET /suggest/metrics`** — the **false-positive rate** (the roadmap exit
  criterion): feedback analytics that measure how often `suggest` is wrong.

Session identity is **client-owned** (a caller-supplied opaque `run_id`); the
server does no session-boundary detection, no timeout, no embedding mean. No new
state machine, no background worker, no push.

## Why this shape

- **Reviewable, not injected.** Every candidate is labelled and caller-chosen —
  the Letta/MemGPT lesson applied as a hard design rule (the roadmap forbids the
  silent-injection alternative).
- **Measurable, not vibes.** The false-positive rate is a *number* (roadmap exit
  criterion), tracked via accept/dismiss feedback — the Mem0 feedback-analytics
  pattern.
- **No drift.** No ranking decay, no hidden personalization, no learned rank
  steering — the server stays deterministic.

## Measured ceiling

- This is the **light cut** of the broader Anticipate plan. Sessions, SSE push,
  ranking decay, and personalization are all explicitly out of scope for v1.9
  (the roadmap forbids them). The honest ceiling is: it's opt-in pull with
  per-chunk feedback, not a proactive recommender.
- True *proactive* (unsolicited, before-the-query) retrieval is **not** a
  settled peer-reviewed technique; it is most honestly attributed to the
  Generative-Agents/MemGPT architecture line and the Zep search→rerank→construct
  pipeline, not to a single definitive paper.

## Related

- [Calibrated abstention](./06-abstention-verify.md) — the opposite guarantee: knowing when *not* to answer.
- [The memory lifecycle](../memory-lifecycle.md) — where surfaced chunks come from.
- [Features](../features.md) — `POST /suggest`, `/suggest/feedback`, `/suggest/metrics`.