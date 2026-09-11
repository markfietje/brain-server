# The week runtime enforcement got a standard

*2026-09-11, updated for v1.28.81. OWASP donated the Agent Control Standard on September 1 and
published the LLM Top 10 2026 in August. This post maps both to controls
already running here; the former gap section below is now marked as closed server-side, with boundaries stated.*

Two things happened in the first week of September. OWASP published the
2026 Top 10 for LLM Applications, the first edition weighted by
documented incidents (6,639 of them, a quarter of the score), and
accepted the donated Agent Control Standard, a v0.1 specification for
runtime agent control: middleware hooks at every agent decision point,
guardian agents returning allow, deny, or modify, OpenTelemetry tracing
mapped to OCSF, and a dynamic Agent Bill of Materials. The ranking shift
that matters most is Excessive Agency climbing from sixth to third. The
category rename that matters most is System Prompt Leakage becoming
Hidden Context Exposure, with retrieved documents, agent memory, and
tool responses named as carrying the same confidentiality risk as the
system prompt.

Read that rename twice. It is the last year of our read-seam work stated
as an industry category. Retrieved content is hidden context. We treat
it that way: fenced, stripped, labeled, never trusted as instruction.

The ACS mapping, control by control:

- **Middleware hooks at memory operations.** Our recall path runs
  through the plugin's `before_prompt_build` hook and the server's
  authorize-then-screen write path. A hook that fires before the act is
  the whole pattern. Ours are framework-specific rather than
  ACS-conformant, the spec is v0.1 and its hook vocabulary is still
  settling, but the enforcement point is the same one.
- **Allow, deny, modify.** The proposal gate returns exactly these
  shapes: approve, reject, edit-and-reapprove, with the digest binding
  the decision to the reviewed bytes. Quorum adds a second approver.
- **Traceability.** Every mutation lands on a keyed hash chain with a
  pinned head, and read events replay through recall traces. OTel spans
  carry the decision path where the feature is compiled in.
- **Static BOM.** A CycloneDX SBOM ships per release with a selfcheck
  gate that refuses badges without it.

Update: the dynamic half is now live server-side in v1.28.81. `GET /ops/agents/bom`, Read on global, returns CycloneDX 1.6 regenerated per request. It names the server service, embedder and classifier models, knowledge-store domains, enforcement posture, and the static SBOM artifact. MCP tool inventory remains fork-side through catalog pins, and the calling agent's own tools and models remain outside this process by construction. So the closed part is the server runtime inventory; the remaining work is a cross-process AgBOM that also covers caller-side tools and models.

Procurement translation: ask vendors whether their controls run at
invocation time or only at configuration time. Permission granted at
setup and never re-checked is how excessive agency happens. Our gates
run per call, per approval, per recall. That is the property ACS
standardizes, and it is already the architecture here.

## Sources

- [OWASP GenAI LLM Top 10 2026](https://genai.owasp.org/resource/owasp-genai-llm-top-10-2026/)
- [Agent Control Standard](https://agentcontrolstandard.org/)
- [OWASP announcement, Sept 1 2026](https://www.prnewswire.com/news-releases/owasp-genai-security-project-releases-2026-top-10-for-llm-applications-debuts-agent-control-standard-and-new-resources-for-securing-generative-and-agentic-ai-302867085.html)
- [CSA research note on both releases](https://labs.cloudsecurityalliance.org/research/csa-research-note-owasp-genai-top10-2026-agent-control-stand/)
