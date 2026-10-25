# 22 · The Post-Convergence Watch (October 2026): the approval boundary gets a name, the tool metadata gets an attacker

*The problem.* The convergence check ([doc 21](./21-cloud-agent-convergence-oct-2026.md))
settled the platform question and left two operational debts: the lanes it
opened had no standing sweep, and the findings it surfaced had no recorded
disposition. This file is the first month of both. Its subject is narrower
than doc 21's: what the new watch lanes and feeds caught in their first pass,
and what each finding means for a store whose central claim is that a human
gate — bound to exact bytes — decides what enters permanent memory.

*The method.* Two standing lanes were added to the daily sweep, implemented
in a local research-watch tool with query
strings tested live on 2026-10-12: **oversight-integrity**
(`all:"human-in-the-loop" AND all:"agent" AND (all:"approval" OR all:"oversight"
OR all:"hijack")`) and **agent-identity** (`all:"agent identity" OR
all:"Agent2Agent" OR all:"agent card" OR all:"non-human identity"`). The
lanes deliberately broaden the proposal ("approval / oversight / hijack" and
"+ non-human identity") because the broadened terms are what caught the
headline paper on the first run. Most entries were triaged at abstract level
on 2026-10-12; two earned full reads the same day, and one — Loopjacking —
was then reproduced as a **live defense verification** against an isolated
instance of the shipped binary. Verdict vocabulary follows doc 20. All
papers are preprints unless marked.

## Threat-input: the approval boundary gets a name

- **[A2M: Trace-Optimized Agent Hijacking in the MCP
  Ecosystem](https://arxiv.org/abs/2609.26761)** (Li, Wang et al., 2026-09-22)
  — **THREAT-INPUT for the bridge lane.** Two-stage black-box attack: optimize
  a tool's *metadata* to raise its invocation probability, then hijack through
  the invocation. The MCP spec's own response — tool descriptions and
  annotations are untrusted unless from a trusted server — is exactly the
  posture this repo already holds, and the reason the plugin's tools are
  static and self-authored. Residual exposure recorded honestly: the
  third-party servers an operator configures into a host are the attack
  surface, and that surface lives in the host's config, not this tree.

## Loopjacking, read in full and run for real: the digest law under the paper's own procedure

*The full read* ([arXiv:2609.21081](https://arxiv.org/abs/2609.21081), Kumar,
2026-09-17 — preprint, single researcher, evidence cutoff 2026-09-10). The
threat model is deliberately narrow: an attacker with **asymmetric
authority**, able to influence pending workflow state but unable to approve
or execute directly, with no model control and no race required. Two
variants carry the name. **Representation-based**: the operation the
approval UI shows is a lossy or misleading rendering of bytes already in
the request. **Post-approval substitution**: the human approves exact A;
pending state is swapped to B before the decision is consumed, and
execution uses B under the stale approval. Tested positive across fourteen
versions of two major orchestration frameworks (Agno AgentOS, seven
releases, 5/5 exploits; LangGraph Agent Server, twelve versions, 12/12);
a frontier vendor's agent SDK serves as the paper's negative control,
rejecting mutated invocations outright. The proposed defense bundle — a
canonical approval record binding tool identity, every material argument,
initiator and approver principals, scope, freshness and consumption status,
with a digest over exactly the bytes shown; use-time re-comparison against
the fully resolved operation; atomic consumption; an audit record; and a
policy denying non-approvers from mutating pending state — maps one-for-one
onto what R117, R102, and R103 already ship. The paper's own non-defense
list (an approval button alone; id-only binding; a run-level approved flag;
presentation-time-only checks; representation-only fixes) describes none of
this repo's approve-path patterns.

*The live reproduction* (2026-10-12; isolated instance of the shipped
binary — throwaway database, two-lane token file, the agent principal as
the paper's maker, the operator bearer as the approver; the operator's live
deployment untouched). **Fourteen checks, fourteen held.**

- **V1, representation.** A hostile proposal (bidi overrides, an exfil
  beacon image, a fence sentinel) rendered **canonical** in the pending
  queue — 161 raw chars to 128 shown, hostiles stripped — and
  `sha256(displayed) == the displayed digest`: one function
  (`review_digest = sha256(sanitize_read(content))`) generates both the
  display and its binding, which is defense #1 verbatim. Approval with a
  digest over the *raw attacker bytes* → **409 "proposal content changed
  since it was displayed — reload and re-approve"** — the paper's use-time
  comparison shipped as the refusal message.
- **V2, substitution.** The maker's mutation attempt (`/ump/revise` on a
  pending proposal) → 422, and the pending row stayed byte-identical after:
  there is no mutation path to win with. Approval without a digest →
  **400 `digest_required`** (fail-closed, no legacy pass). Approval with the
  substituted digest → **409**. The maker consuming its own proposal →
  **403**: the agent principal is role-gated out of approval entirely
  (R103/R115) — the paper's attacker cannot pull the trigger at all. The
  operator with the displayed digest → 200, and the promoted record's bytes
  equal the displayed canonical bytes, not the raw hostile ones.
- **Consumption semantics.** Replay of the spent decision → 404; two
  concurrent correct-digest approvals → exactly one 200 and one 404 (the
  CAS consumes atomically); no-bearer → 401 at the boundary.

*The drill candidate resolves.* The earlier wording asked for a composite
red-first drill; the full read splits it — variant-1 is already pinned
(`console_pending_shows_the_digest_bound_bytes`), and variant-2 has no
reachable mutation path to attack: the absence is the invariant, now
exercised by procedure rather than pin. *Honesty notes from the run:* two
harness bugs invalidated the first two executions (an auth-disabled boot
from a wrong env var, then a scripting error dropping the bearer) — the
server's behavior was identical across all three runs, and the paper's own
negative-control discipline is what exposed the harness as the liar. The
audit-on-denial half rests on the shipped pins rather than a fresh live row
(the ad-hoc query missed its table). Remaining ceiling: the
disclosed-elision display path (PII masking) was not exercised — the threat
model already states it as a design ceiling, with the digest binding the
full canonical bytes.

## Adjacent validations, recorded without integrations

- **[Hardware Keystores for AI Agent Signing Workflows: A Zero-Trust MCP
  Enforcement Architecture](https://arxiv.org/abs/2608.06130)** (Sambrook &
  Sovio, 2026-08-06) — the key-lifecycle counterpart: agent signing keys in
  software-accessible locations are readable by anything the agent can reach,
  and HSM/TPM anchoring is the fix class. This repo's keys (chain key, token
  files, webhook secrets) are deliberately file-based under the secret-file
  law (0600, mode-enforced readers); the paper is the citation for the
  declined posture's ceiling, not an integration.
- **[Weakening Assumptions for Publicly-Verifiable Deletion](https://arxiv.org/abs/2304.09846)**
  (Bartusek, Khurana et al., 2023, peer-reviewed) — cryptographically
  well-founded deletion *receipts* from one-way functions. The erasure lane's
  shipped answer is byte-gone evidence plus audit receipts; this is the
  formal-crypto vocabulary for the same claim, recorded for the lane's
  register, not adopted.

## MindMemOS, read in full: the substrate mirror and the token bill

The landscape one-liner earned a full read
([arXiv:2608.12428](https://arxiv.org/abs/2608.12428), Liang, Cui et al.,
2026-08-12, Huawei Noah's Ark Lab; self-reported LOCOMO 94.03 against Zep
85.22 and Mem0 64.20). Verdict: **validates the substrate, refused as
machinery, three hooks recorded** — and it is the strongest independent
convergence with this repo's data model in any sweep so far.

**The mirror.** Its memory structure is an entity–property–time graph with
semantic relations between entities and *timeline links between successive
records on the same entity–property pair* — the entities/relations graph,
the bi-temporal `valid_at`/`invalid_at` pair, and supersession chains,
re-derived independently. Its recall fuses BM25 + dense with RRF (§1's
stack). Its write path classifies candidates with an LLM but maps them
through a **deterministic** Add/Reinforce/Update/Merge/Skip gate — even the
most LLM-saturated system in the lane converges on deterministic admission
control, the weaker cousin of the human review queue. Its "Dreaming"
consolidation is detect-then-act with traceable merge/archive/**supersedes**
mutations and a stated preference for non-destructive updates when
uncertain: doc 12's reviewable consolidation, with the trace claim met by
application logs instead of a tamper-evident chain.

**The bill.** Every evolution mechanism is LLM-driven — extraction and
fusion on write, an agentic retrieval planner on recall, consolidation
planning, feedback triage, and MindMemEvolve (evolutionary search over
memory *schemas*, scored by an LLM judge on held-out QA). The paper's own
cost table is the standing law's best external evidence: ~5.5–5.8M tokens
for schema evolution, 25–27M tokens per skill-evolved agent run versus
10.4M without. The zero-token core refuses that bill by constitution — and
keeps recall microseconds-cheap and every answer reconstructable from its
inputs.

**Three hooks recorded, none adopted today:**

1. *Reinforce as a consolidation verb* — their admission gate distinguishes
   reinforcing an existing memory from adding a near-duplicate. A
   deterministic, reviewable reinforce proposal (attach evidence to the
   standing row rather than propose a second row) extends the doc-12
   vocabulary.
2. *A consolidation-compression KPI* — their AMCR (~20% active-memory
   compression from consolidation) is deterministic to compute; a `bench
   eval` metric in that shape would give the consolidation pass its
   `answer_in_context` moment.
3. *Feedback scoping* — their three-way corrective-signal classification
   (task-temporary / scenario-specific / long-term) is a decay-scoping
   vocabulary beside the UMP feedback outcomes.

Fixture note: **MemoryAgentBench FactConsolidation** (where dreaming
measurably improves fact scores) and their LOCOMO/PersonaMem baseline table
ride to [doc 13](./13-benchmark-landscape-2026.md). One ecosystem note:
MindMemOS ships plugins for the agent-host family this repo's own plugin
mirrors into.

## Standards movement (the register's lane; see doc 19 for the rows)

- **MCP specification 2026-07-28** landed an extensions mechanism — Tasks
  (async long-running operations with durable handles), Skills over MCP, MCP
  Apps — and states normatively that tool annotations are untrusted. The
  Tasks extension is the protocol-level shape of the workflows lane; the
  bridge's wire-version parity discipline now has a spec cadence to track.
- **OWASP GenAI** expanded into this repo's exact lanes: the **Agent Control
  Standard (ACS)** (existence and scope verified; control set unread), memory
  and context poisoning holding a numbered slot (**ASI06**) in the Agentic
  Applications Top 10, and a quarterly **exploit roundup** now feeding the
  daily sweep.
- **A2A v1.0** (Linux Foundation / Agentic AI Foundation): Agent Cards and
  delegation — the interoperability surface the `origin` label will
  eventually have to speak to. Watch item; nothing speaks A2A here.

## The watch infrastructure itself

The lanes ride research-watch, a local zero-dependency sweeper
sharing the arxiv-watch conventions (read-only MCP server, `--mark-seen`
is the only writer, report.md + feed.xml + watchlist.opml on every run).
It covers sixteen keyless sources — the two arXiv lanes above, OWASP
GenAI, MITRE ATLAS data, CISA advisories, NIST news, Project Zero, the
MCP/A2A/Rekor release feeds, Sigstore, practitioner blogs, and query-scoped
Hacker News via the Algolia API after hnrss.org proved unreliable. The division
of labor is single-responsibility by design: arxiv-watch owns the eight
standing lanes, research-watch owns everything else, so no source is swept
twice and no seen-state is duplicated.

- [Governance-State Corruption in Long-Term Memory for LLM Agents:
  A Cross-Model Stress Test](https://arxiv.org/abs/2610.30698) (preprint,
  2026-10-22): faults that alter which retained records enter model-visible
  context while stored content stays fixed — content-integrity does not
  cover governance-integrity. Integrated 2026-10-25 as R-GOV (v1.29.32
  "Governed"): `knowledge.governance_hash` over `(owner, access_scope,
  domain)` + `brain verify-governance`, with the local-attacker caveat
  stated in THREAT_MODEL and the gate-law register. (Correction recorded:
  this round's execution prompt claimed the Oct 2026 sweep "already
  carried" this paper — it did not; this line is the cross-reference, added
  by the round that used it.)

*The ceiling.* The Loopjacking entry is the one full read with a live
defense verification — fourteen checks against an isolated instance of the
shipped binary, not the operator's live deployment, with the PII-masking
display path (a stated design ceiling) deliberately unexercised. A2M
remains abstract-level, and both papers are preprints: single-researcher,
no independent reproduction, evidence cutoffs in September 2026. The
standards entries are snapshots of moving specifications (the SSE framework
is a draft; MCP extensions are opt-in and young). The watch tool's sources
are live as of 2026-10-12 and can rot — the sweep reports per-source health
precisely so that rot is visible instead of silent. No affiliation with any
listed work.
