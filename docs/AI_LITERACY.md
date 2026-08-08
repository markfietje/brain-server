# AI Literacy — Deployer Playbook (EU AI Act Art 4)

**Artifact for:** `COMPLIANCE.md` §6.4 · **Applies to:** brain-server 1.16.7
· **Last updated:** 2026-08-08

EU AI Act **Art 4** (Regulation (EU) 2024/1689) requires providers and
deployers to take reasonable steps to ensure a sufficient level of AI literacy
among the people who operate or use the system. This page is the operational
playbook for the memory component: what it is, why it is inspectable, and how
a deployer demonstrates literacy against the controls the server already ships.

## What this component is — and is not

brain-server is a **memory component for an AI assistant**. It stores what the
client sends it, indexes it (embeddings + lexical + knowledge graph), and
serves deterministic retrieval (`/recall`, `/search`).

It does **not** generate content, reason, or decide on its own. It retrieves,
it proposes, and it records. That distinction matters for Art 4 literacy: the
"AI decisions" a person is asked to be literate about here are narrow and
concrete — *what was retrieved, and who approved a write* — and every one of
them has a control.

## The controls that make it inspectable (the literacy substance)

| Ask a person can answer | Control |
|---|---|
| *What informed this retrieval?* | Recall trace — `GET /recall/{trace_id}/trace` replays the injected chunks, scores, abstention decision, and domains searched (Art 22 "meaningful information about the logic"). |
| *Who approved this write?* | Proposal gate — `POST /ingest/proposal` scores but writes nothing; memory becomes permanent only via human approval (`/proposals` review queue). |
| *Is anything quarantined?* | Quarantine list (`/quarantine`) — flagged rows are excluded from retrieval until reviewed. |
| *Can a subject delete themselves?* | DSAR console + deletion certificate (`/dsar`, `/tombstones`) — locate → export → purge → certificate. |
| *Has the audit chain been tampered with?* | `/audit/verify` — the SHA-256 hash chain verifies end to end. |
| *How did a memory enter, and is it AI-derived?* | `/export` provenance + `/.well-known/ai-notice` (Art 50) — `source`, `assertion_kind`, `confidence` per row. |

## How a deployer demonstrates literacy

Literacy is a practice, not a document. The concrete, repeatable cadence:

1. **Use the dashboard weekly.** Review the `/proposals` queue (approve /
   reject), check `/quarantine`, and read a couple of recall traces so the
   person operating the system can state *why* a given answer was produced.
2. **Verify the chain on a schedule.** Run `/audit/verify` (or the `brain
   doctor` / `/metrics` chain-ok gauge) and keep the passing result as the
   audit evidence file.
3. **Run a DSAR drill before you need one.** Execute a purge against test
   subject data end to end (locate → export → purge → certificate) so the
   operator is literate in the deletion workflow before a real request
   arrives. (The report's CRA 30-minute drill deadline is the same muscle.)

The dashboard, trace, approval queue, and DSAR console *are* the literacy
surface — using them on a cadence is the evidence. For the machine-readable
disclosure side, see `COMPLIANCE.md` §7 and `/.well-known/ai-notice`.

## Honest ceiling

This artifact documents **what the component makes inspectable and how to
operate it**. Art 4 literacy for the *whole* AI system (the assistant an
organization runs on top of brain-server) is the deployer's broader program
and is out of scope for a memory component — this playbook covers the
component's slice and how to evidence it.
