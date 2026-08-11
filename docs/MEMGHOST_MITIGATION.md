# Memory-Poisoning Mitigation in brain-server (ASI06, MemGhost, GhostWriter)

> References — the 2025/2026 memory-poisoning disclosures:
> - **ASI06 Memory and Context Poisoning** — OWASP Top 10 for Agentic
>   Applications (launched 2025-12-09). The canonical category for adversarial
>   content written into an agent's persistent memory so it acts on that content
>   in later sessions. Distinct from the OWASP GenAI LLM Top 10 2026
>   (2026-08-04; memory-adjacent entry **LLM09 Vector and Embedding
>   Weaknesses**).
> - **MemGhost** — "When Claws Remember but Do Not Tell"
>   (arXiv 2607.05189, July 2026; CSA research note 2026-07-23). A crafted
>   email plants a false *persistent* memory in OpenClaw-style agents, hides the
>   change, and sways later sessions without the operator noticing. Reported at
>   87.5% success in background mode against OpenClaw on GPT-5.4 (75% foreground,
>   100% stealth).
> - **GhostWriter** — "When Agents Remember Too Much"
>   (arXiv 2607.06595, July 2026). A two-phase vector (injection + activation)
>   that poisons long-term memory via untrusted tool inputs; ~98% injection and
>   ~60% activation across five agents. Proposes AM-Sentry (admission policy +
>   retrieval screen).

MemGhost and GhostWriter are the canonical examples of a **memory poisoning**
attack (OWASP ASI06). They target exactly the class of plaintext,
silently-mutated memory files (e.g. OpenClaw's `MEMORY.md`) that brain-server
is designed to replace with an audited, human-gated store. This page maps the
attack's stages to brain-server's existing controls — the controls are already
built; this is the operator-facing story of *how* they stop the attack.

## The attack

1. **Plant.** A single crafted message (email, chat, doc) carries instructions
   framed as facts ("the project is cancelled", "the user prefers X").
2. **Write.** The agent ingests them into persistent memory with **no
   verification** and **no user confirmation**.
3. **Hide.** The mutation is silent — no audit trail, no diff, no approval.
4. **Exploit.** Later sessions retrieve the planted fact and act on it as if it
   were the operator's own true memory.

The kill conditions: **unverified writes**, **silent changes**, **no
approval gate**, and **no provenance on retrieval**.

## How brain-server neutralizes each stage

| Stage | brain-server control | Where |
|---|---|---|
| Plant | Every candidate memory is scored but **not** written (proposal). Untrusted content is tagged `untrusted: true`. | `POST /ingest/proposal` · OWASP LLM01:2025 boundary |
| Write | **Human-in-the-loop.** A proposal becomes memory only after `approve`. Nothing is auto-promoted. | `POST /proposals/{id}/approve` |
| Hide | **Append-only SHA-256 audit chain.** Every ingest, approve, reconcile, purge is a hash-linked row. No silent mutation exists. | `src/audit.rs` · `GET /audit/verify` |
| Conflict | A planted fact conflicting with an existing one is surfaced via `contradicts`/`supersedes` evidence links and an **unresolved-contradiction check** — it cannot silently overwrite. | `POST /consolidate/propose` · `brain check-consistency` |
| Provenance | Every recall hit carries `source`, `assertion_kind`, `confidence`, and an evidence span. Retrieval can state **where a memory came from**. | `GET /recall` · `GET /get/{id}` |
| Undo | An accepted-but-wrong memory is **reversible** — supersession undo + DSAR purge with a chain-verifiable certificate. | `POST /consolidate/undo` · `POST /dsar` |

## Operator checklist

- Run with a write-back gate: proposals auto-pending, approval human-owned.
- Keep the plugin's `captureMode` at `proposal` (the default) so auto-captures
  from untrusted turns enter memory only after human approval. `direct` mode
  is for trusted deployments and is still screened by the server-side
  `ingest_one` injection gate (quarantine/reject).
- Verify the audit chain periodically: `brain status` → `/audit/verify` → `ok`.
- On a suspected poisoning: `brain check-consistency` to surface unresolved
  contradictions, then `brain resolve` / `brain undo-resolve` the affected
  chunks, and export (`GET /export`) to confirm the store before purge.
- Keep `INJECTION_POLICY` at `quarantine` so untrusted input is stored but
  excluded from retrieval.

## Why this is defense, not detection

MemGhost and GhostWriter are *content* attacks against an unvetted auto-write
path. brain-server removes the unvetted auto-write path itself (HITL) and makes
every remaining write auditable + reversible, so there is nothing silent to
detect. Retrieval still surfaces what it is asked for; the operator, not the
attacker, owns what is allowed in. This aligns with the ASI06 / AM-Sentry
mitigations: provenance at write time, gated writes, a hash-chained audit log,
and a tombstone path to retire and trace a poisoned entry.
