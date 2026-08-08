# MemGhost Attack — Mitigation

> Reference: **MemGhost** — "When Claws Remember but Do Not Tell" (arXiv 2607.05189, July 2026). A crafted email plants a false *persistent* memory in OpenClaw-style agents, hides the change, and sways later sessions without the operator noticing. Reported at 87.5% success in background mode against OpenClaw on GPT-5.4, with a 108-case WhisperBench benchmark.

MemGhost is the canonical example of a **memory poisoning** attack. It targets exactly the class of plaintext, silently-mutated memory files (e.g. OpenClaw's `MEMORY.md`) that Brain Server is designed to replace with an **audited, human-gated store**. This page maps the attack's stages to Brain Server's existing controls — the controls are already built; this is the operator-facing story of *how* they stop the attack.

## The attack

1. **Plant.** A single crafted message (email, chat, doc) carries instructions framed as facts ("the project is cancelled", "the user prefers X").
2. **Write.** The agent ingests them into persistent memory with **no verification** and **no user confirmation**.
3. **Hide.** The mutation is silent — no audit trail, no diff, no approval.
4. **Exploit.** Later sessions retrieve the planted fact and act on it as if it were the operator's own true memory.

The kill conditions: **unverified writes**, **silent changes**, **no approval gate**, and **no provenance on retrieval**.

## How Brain Server neutralizes each stage

| Stage | Brain Server control | Where |
|---|---|---|
| Plant | Every candidate memory is scored but **not** written (proposal). Untrusted content is tagged `untrusted: true`. | `POST /ingest/proposal` · OWASP LLM01:2025 boundary |
| Write | **Human-in-the-loop.** A proposal becomes memory only after `approve`. Nothing is auto-promoted. | `POST /proposals/{id}/approve` |
| Hide | **Append-only SHA-256 audit chain.** Every ingest, approve, reconcile, purge is a hash-linked row. No silent mutation exists. | `/audit/verify` |
| Conflict | A planted fact conflicting with an existing one is surfaced via `contradicts` / `supersedes` evidence links and an **unresolved-contradiction check** — it cannot silently overwrite. | `POST /consolidate/propose` · `brain check-consistency` |
| Provenance | Every recall hit carries `source`, `assertion_kind`, `confidence`, and an evidence span. Retrieval can state **where a memory came from**. | `/recall` · `/get/{id}` |
| Undo | An accepted-but-wrong memory is **reversible** — supersession undo + DSAR purge with a chain-verifiable certificate. | `POST /consolidate/undo` · `/dsar` |

## Operator checklist

- Run with a write-back gate: proposals auto-pending, approval human-owned.
- Verify the audit chain periodically: `brain status` → `/audit/verify` → `ok`.
- On a suspected poisoning: `brain check-consistency` to surface unresolved contradictions, then `brain resolve` / `brain undo-resolve` the affected chunks, and export (`GET /export`) to confirm the store before purge.
- Keep `INJECTION_POLICY` at `quarantine` so untrusted input is stored but excluded from retrieval.

## Why this is defense, not detection

MemGhost is a *content* attack against an unvetted auto-write path. Brain Server removes the unvetted auto-write path itself (human-in-the-loop) and makes every remaining write **auditable + reversible**, so there is nothing silent to detect. Retrieval still surfaces what it is asked for; the operator, not the attacker, owns what is allowed in.

## Related

- **[AI Literacy](AI-Literacy)** — the operator playbook (Art 4) that includes the weekly-verify + DSAR-drill cadence this page's checklist supports.
- **[Security](Security)** — the wider threat model.
- **[Governance & Compliance](Governance-and-Compliance)** — the audit chain, proposal gate, and DSAR in the framework maps.
