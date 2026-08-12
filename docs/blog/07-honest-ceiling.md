# The honest ceiling

*2026. What we deliberately do not claim — and why that's the most important thing we ship.*

Every memory-store vendor will tell you what their product does. Almost none
will tell you what it can't. This post is the exception, on purpose, because
**an honest ceiling is a trust asset and a procurement advantage** — and because
a deterministic system is one whose limits you can actually state.

## The ceilings, stated plainly

**Retrieval is deterministic, not SOTA-generative.**
The retrieval stack is reference-faithful and reproducible, but it is *not* an
LLM-based ranker. It won't catch paraphrase the way a generative model can.
`/verify` is lexical — a claim must literally appear in the text; it will not
match a paraphrase. That's a feature for audit (the span is provable) and a
limit for understanding. We don't claim semantic-match verification.

**Live multi-hop graph quality is corpus-bound.**
The Personalized PageRank leg is the right mechanism, but on a working
noisy corpus ~94% of knowledge-graph edges were `tagged_with` taxonomy noise.
The mechanism ships; the *corpus* is an operator concern. Good graph recall
depends on re-ingesting with a real linker. We don't claim the mechanism fixes
a noisy graph by itself.

**Abstention is heuristic, not learned.**
`ClarifyQuery` abstention is calibrated on rank-agreement signals, not a judged
corpus. A judged-corpus recall floor (`brain eval --floor`) is an operator step
we provide but don't run for you. We don't claim a measured SOTA recall number.

**The security matrix is 100% coverage, not 100% risk elimination.**
OWASP 2026 itself says LLM01 (prompt injection) has no prevention. What
survives an adaptive attack is segregation + gates + least privilege. At-rest
encryption, mTLS, A2A federation, OIDC authorization-code, multi-team tenancy —
all owned v2.x ceilings. We don't claim what we haven't built.

**Multi-process audit, local-first storage.**
The audit chain is single-process today; distributed audit is a named future
ceiling. Storage is one local SQLite-family file — great for privacy and
portability, which also means no managed-cloud scale-out. We don't claim a
SaaS we're not.

## Why this wins the review

A vendor who volunteers its limits reads as *credible*. It means:

- **No bait-and-switch at procurement.** The buyer discovers the real costs
  from the blog, not after signing.
- **Verifiable by construction.** Every ceiling is paired with the thing that
  *does* work and the command to prove it (the proof map).
- **The roadmap is honest.** Each ceiling names its upgrade path and version —
  tenancy → v2.0, OTel → v1.20.7, SOC 2 kit → v1.20.10. "We don't do X yet"
  is followed by "and here's when X lands," not silence.

**The takeaway:** in a category drowning in "revolutionary memory," the most
differentiating sentence is "here's what we can't do, and how you'll know."
Adopt the thing that tells you its limits; you'll be defending *that* one to
your own compliance team.

*Every ceiling above is expanded with its mechanism + upgrade path in
[`docs/research/`](../research/01-bi-temporal.md) and [`docs/trust/proof-map.md`](../trust/proof-map.md).*
