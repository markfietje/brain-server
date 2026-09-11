# Prompt injection made stateful, and the memory layer that was built for it

*2026. The 2026-09-06 audit found our fences and gates held. The second-pass audit, same trees, harder questions, found the seams those closures had, and we fixed them at fixpoint. This post is the market context for why we run these audits at all: 2026's research says memory is where prompt injection goes to persist, and almost nobody ships the controls that survive it.*

## The threat got a name, a paper, and a benchmark

For two years, "prompt injection" meant a hostile turn: the model reads something
malicious, maybe obeys it, and the conversation ends. 2026 made it **stateful**.
The attack now writes itself into the one place your agent trusts most, its own
memory, and replays every turn after.

The research landed in quick succession:

- **OWASP's [Top 10 for Agentic Applications 2026](https://genai.owasp.org/resource/owasp-top-10-for-agentic-applications-for-2026/)**
  ranks Agent Goal Hijack #1 and defines
  [ASI06 Memory & Context Poisoning](https://vectorize.io/articles/owasp-asi06)
  as a top-tier risk, with the [Gemini Memory Attack](https://genai.owasp.org/2025/12/09/owasp-top-10-for-agentic-applications-the-benchmark-for-agentic-security-in-the-age-of-autonomous-ai/)
  as their named example. OWASP now incubates a dedicated
  [Agent Memory Guard project](https://owasp.org/www-project-agent-memory-guard/).
- **Unit42 demonstrated indirect prompt injection persisting into long-term
  memory**, the poisoned note waits in the store and fires in a later session
  ([Palo Alto Networks](https://unit42.paloaltonetworks.com/indirect-prompt-injection-poisons-ai-longterm-memory/)).
- The **[MemPoison paper](https://arxiv.org/html/2605.29960v1)** measured up to
  **0.95 attack success rate** across memory mechanisms, and it generalizes
  across designs.
- The framing that stuck:
  ["prompt injection made stateful"](https://christian-schneider.net/blog/persistent-memory-poisoning-in-ai-agents/).
- And the tell that the market knows: **[MemGuard](https://github.com/ac12644/MemGuard)**
  exists specifically to bolt trust scores and quarantine onto
  Mem0/Zep/Letta/LangMem *after the fact*.

Read those together and the conclusion is uncomfortable: if your memory layer
auto-extracts and auto-writes what an agent says, you have given prompt
injection a database.

## What "answer it architecturally" means

Brain-server + its OpenClaw plugin were built with the assumption that
*everything trying to enter memory is hostile until a human says otherwise*.
The 2026 threat model describes our roadmap; here is the shipped answer, layer
by layer:

1. **Screen at ingest.** Every write passes a deterministic injection screen
   (instruction-override blocklist with translation families, typoglycemia and
   encoding tiers, optional local ONNX classifier). Suspect content is
   quarantined, excluded from *every* retrieval leg: full-text, graph, and
   vector. `Reject` policy never persists it at all.
2. **The human promotion gate.** By default, nothing an agent captures becomes
   memory. It lands as a proposal in a review queue, scored deterministically,
   carrying the exact capture context. The operator approves, and the approval
   is **digest-bound**: it must carry the SHA-256 of the exact bytes the
   reviewer saw (`400 digest_required` when absent, `409` on drift). Rankers
   rank; they never promote.
3. **Read-seam strips that survive re-assembly.** A single-pass strip is not a
   closure: our second-pass audit demonstrated `<scr<script>ipt>` welding back
   into a live `<script>` after the element strip, and nested markdown
   constructs healing back into auto-fetch images after the dereference. The
   strips now iterate to a **fixed point**, pinned by tests with the exact
   adversarial vectors.
4. **The fence, and the labels.** Every injected hit is wrapped in an
   unforgeable `UNTRUSTED_*` fence, stripped of invisible-Unicode and bidi
   smuggling, dereferenced of image/link refs, tagged `untrusted: true`, and
   hits that arrive without the flag are **dropped**, not injected. Captures
   from group/channel traffic carry a visible `[memory | channel-capture]`
   taint label for their whole life, and the host marks replayed labels in
   inbound text as untrusted.
5. **Scoped principals, not shared gods.** The agent authenticates as a scoped
   principal, recall/store/propose, no purge, no domains, no identity
   operations, with a probe-blind kill-switch wired into every auth door, and
   MCP tool scope capped by env.

## The honest comparison

The memory-layer market is real and good at what it does: **Mem0** for
ecosystem and extraction pipelines, **Zep/Graphiti** for temporal knowledge
graphs, **Letta** for self-editing agent memory. We don't lead retrieval-quality
benchmarks, our docs mark that **pending** rather than claiming it, and
bi-temporal recall has better-published implementations. The
[2026 comparisons](https://blog.devgenius.io/ai-agent-memory-systems-in-2026-mem0-zep-hindsight-memvid-and-everything-in-between-compared-96e35b818da8)
are right that no system wins every dimension.

What none of them ship as native architecture is the axis above: ingestion
screening with quarantine, a digest-bound human promotion gate, untrusted-fence
rendering with fail-safe drop, provenance taint labels, tamper-evident audit
with pinned heads, erasure certificates with tombstones and legal holds, and a
local-first zero-token economy. The proof that the market treats this as a
bolt-on is MemGuard's existence, trust scores layered onto other people's
memory stores, and OWASP incubating a memory-guard project of its own.

If your deployment is a regulated or customer-facing one, ask your memory
vendor the 2026 questions: *what happens to content your screener flags? who
can promote memory, and what binds that approval to the reviewed bytes? how do
you prove an embedding was deleted? where does the audit chain's key live?*
We published our answers as
[a control matrix](../OWASP_AGENTIC_2026.md) and a
[live proof map](../trust/proof-map.md), and a
[second-pass audit](../AUDIT.md#second-pass-audit-addendum--v12876-selfheal-2026-09-09) of our own closures,
because the first pass is where the work starts, not where it ends.

## Sources

- [OWASP Top 10 for Agentic Applications 2026](https://genai.owasp.org/resource/owasp-top-10-for-agentic-applications-for-2026/)
- [OWASP announcement, the benchmark for agentic security](https://genai.owasp.org/2025/12/09/owasp-top-10-for-agentic-applications-the-benchmark-for-agentic-security-in-the-age-of-autonomous-ai/)
- [Unit42: Indirect Prompt Injection Poisons Long-Term Memory](https://unit42.paloaltonetworks.com/indirect-prompt-injection-poisons-ai-longterm-memory/)
- [MemPoison (arXiv)](https://arxiv.org/html/2605.29960v1)
- [Persistent Memory Poisoning in AI Agents](https://christian-schneider.net/blog/persistent-memory-poisoning-in-ai-agents/)
- [MemGuard](https://github.com/ac12644/MemGuard) · [OWASP Agent Memory Guard](https://owasp.org/www-project-agent-memory-guard/)
- [OWASP AI Agent Security Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/AI_Agent_Security_Cheat_Sheet.html)
- [AI Agent Memory Systems in 2026 compared](https://blog.devgenius.io/ai-agent-memory-systems-in-2026-mem0-zep-hindsight-memvid-and-everything-in-between-compared-96e35b818da8)
- [Five systems, six dimensions, no winner](https://medium.com/@wasowski.jarek/i-compared-5-ai-agent-memory-systems-across-6-dimensions-none-wins-6a658335ed0a)
