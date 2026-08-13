# Human in the loop

> **The human in the loop is a job, not a place.**

Brain Server does not treat a human reviewer as a checkbox in the pipeline. It treats
human judgment as a **work product** — a real task with real tooling, real time, and
real consequences — and it is designed so that the operator can actually *do* that job
well instead of rubber-stamping a queue.

This page is the operator's field manual for that job. It answers three questions:

1. **What does meaningful control mean here?** — the four testable conditions.
2. **What is the machine, and what is the human?** — exactly which write decisions reach a person, and which are never automated.
3. **How do I actually evaluate a proposal?** — a step-by-step decision procedure you can follow at your desk.

---

## 1. Meaningful control, not a checkpoint

"Human in the loop" is too often reduced to *a human clicked "approve" somewhere in the
pipeline*. That is a **location**, not **control**. A reviewer who cannot see why a
proposal exists, who has no time to evaluate it, and whose rejection changes nothing is
not in control — they are a rubber stamp.

The literature is consistent on what makes control real. Four testable conditions capture
the essence (adapted from the Production AI Institute's *meaningful human control*
framing, and consistent with Bainbridge's *Ironies of Automation*, Endsley's *automation
conundrum*, Parasuraman & Manzey's *automation bias*, and the CSIRO/UNSW *operative vs.
evaluative agency* work):

| Condition | Question it answers | The failure it prevents |
|---|---|---|
| **Comprehensibility** | Can I understand *why* this proposal exists? | The *explainability paradox* — an explanation that is too shallow or too plausible makes the reviewer *less* critical, not more. |
| **Reviewability** | Do I have *enough* information and *enough time* to judge it? | The *rubber-stamp problem* / *quasi-automation* — approving because review is too costly. |
| **Actionability** | Is rejecting (or correcting) as easy and legitimate as accepting? | The *automation bias* / *default-accept* — rejecting is "not worth the friction." |
| **Consequentiality** | Does my decision actually *change the outcome*? | *Moral crumple zones* — the human is on the hook for a result they never actually steered. |

Every feature in the rest of this page exists to make one of these four conditions true.
If a screen, score, or endpoint does not serve one of them, it is not part of the
human-in-the-loop story — it is decoration.

### A system designed *against* its own failure modes

The four failure modes below are not hypotheticals. They are the documented failure
modes of human-supervised automation, and Brain Server is engineered so that the *default
behaviour* of the machine does not push the operator into them:

- **Out-of-the-loop skill loss** (Bainbridge, 1983) — the operator was *never in* the loop,
  so they never learned to judge. Brain Server's proposals carry a scoring breakdown and a
  sourcing prompt so judgment is *trained*, not assumed.
- **Automation bias** (Parasuraman & Manzey, 2010) — errors of *omission* (trusting the
  machine, not checking) and *commission* (blindly following it). The review card never
  presents a bare "accept/dismiss" binary — it always shows *why*.
- **The explainability paradox** (Harvard Business School, 2024) — a confident,
  shallow explanation makes a reviewer **less** critical. Brain Server shows you *raw
  evidence* (the actual span, source URI, revision, heading, line range) — not a summary
  that someone else wrote.
- **The moral crumple zone** (Millar) — the human is blamed for an outcome the automation
  actually controlled. Every decision — approve, reject, supersede, expire — is written
  to an append-only, tamper-evident audit chain, so your judgment is *reconstructable*.

> **The invariant:** nothing here auto-promotes, auto-decays-away, or auto-deletes. The
> human decides. Zero tokens, no LLM, no background worker decides what becomes memory.

---

## 2. What reaches the human, and what never does

Brain Server is deterministic by design — *recall* and *retrieval* run with no LLM in the
hot path. But **write-back** — the decision of whether a captured fragment becomes part of
the permanent memory — is a *human* decision. That is the boundary, and it is deliberate.

### The human decides (write-back gate)

The proposal gate (`POST /ingest/proposal`, v1.14) is the single seam where new memory
enters. It works like this:

1. A capture is **scored**, never stored: `POST /ingest/proposal` computes
   - **novelty** (vector KNN — is this already known?),
   - **conflict** (does it contradict a stored chunk?),
   - **salience** (a length/entity heuristic — is it worth keeping?),
   and runs it through the prompt-injection screen.
2. It creates **no `knowledge` row**. Until a human approves, the proposal is not part of
   the memory, is not recallable, and has no effect on any retrieval.
3. A human reviews it and, in one transaction, either
   - **approves** it into memory (`POST /proposals/{id}/approve`), optionally **superseding**
     the chunk it contradicts (`?supersedes=<id>`), or
   - **rejects** it (`POST /proposals/{id}/reject`) — audited, never deleted, recorded with
     an optional reason.

The consequence is concrete: **no write to the permanent store happens without a human
signing it.** An LLM cannot inject memory by completing a prompt; a plugin cannot auto-
capture into the store unless the operator has explicitly turned that gate off.

### The human is the review authority, not a ceremony

The same philosophy extends across the write surface:

- **Consolidation** (`/consolidate/propose`) detects duplicates, contradictions, stale
  sources, and near-duplicates, and **proposes** resolutions. Applying them
  (`/consolidate/apply`, `/consolidate/undo`) is a human call.
- **Expiry** is surfaced, never autonomous: nothing "decays away" on its own. Decayed
  chunks are listed (`/decayed`) for human review. Retention limits are a human-set policy.
- **Purge / deletion** is a deliberate, audited human action (`POST /purge`, the DSAR
  workflow). Nothing is silently erased.

### What the machine does *without* the human

Deterministic operations that a human would not add value to:

- **Retrieval and recall** — hybrid search, the knowledge-graph leg, PRF expansion, and
  calibrated abstention all run with no LLM and no human in the path.
- **Span verification** (`/verify`) — a deterministic lexical check that a claim appears
  in a chunk's text. It answers *"is this string there?"*, not *"is this true?"* — the
  truth judgment is always the human's.
- **Prompt-injection screening** — the two-layer screen (blocklist + optional classifier)
  quarantines or rejects suspicious content *automatically*. This is not a write decision;
  it is a *safety* decision made before a human is ever asked to look at a sketchy span.
  Quarantined rows are still surfaced (see the Ops panel) so a human can override.

---

## 3. The dashboard is the control room

The web client (`/app`) is not a settings screen — it is the operator's **control room**,
and every surface maps to one of the four conditions. Four surfaces do the heavy lifting.

### Review panel — the write-back queue (`/review`)

The default landing page and the heart of the human-in-the-loop job. Each card is built to
make *comprehensibility* real:

- **Scoring breakdown** — novelty, conflict, and salience, shown as numbers with their
  meaning, not a single opaque "score."
- **Conflict surface** — if the proposal conflicts with a stored chunk, the card says
  *"conflicts with chunk #N — approve to supersede,"* making the trade-off explicit
  rather than hidden behind a default.
- **Sourcing prompt** — `source_prompt` is PII-screened at persist and shown so you can
  compare the captured fragment against *what the model was doing*, not just a summary.
- **Screen verdict** — a `clean` / `quarantined` badge from the injection screen, so you
  know a layer-2 classifier flagged it.
- **Evidence on demand** — every row opens the shared evidence modal (`GET /get/{id}`),
  showing the *verbatim* span, `source_uri`, revision, heading, and line range. Not a
  paraphrase. Raw evidence.

Every outcome is tracked per row (`RowOutcome`): **Done**, **AlreadyDone** (a 404 with
nothing pending counts as success), **Queued** (offline — replayed later, never dropped),
and **Failed** (surfaced, never silently dropped). Keyboard `A/S/R/J/K` approve/reject/
skip with a WCAG 2.1.4 toggle, and a reject-with-reason editor.

### Memory Operations panel — the pulse (`/ops`)

Added in v1.20.6, this is where the *reviewability* and *consequentiality* conditions are
made operational:

- **Live pending queue with SLA clocks.** The queue is a clock. Every pending proposal
  shows a live countdown to its expiry (`DEFAULT_PROPOSAL_TTL_SECS`, default 7 days).
  Expiring-first ordering means you are never surprised by a silent auto-reject — the
  panel tells you which decisions are *time-critical right now*. (`< 5 min` critical,
  `< 1 hr` warn.)
- **Flagged & quarantined inventory.** What the injection screen caught, read-only, with
  invisible smuggling characters stripped at display so you can actually read it. The
  safety decision is visible and overridable.
- **Gate-health strip.** Approved / rejected / expired counts over a rolling window feed a
  severity hint: **over-rejecting** (are you blocking good captures?) and **under-reviewing**
  (are decisions expiring on you?) are surfaced as operational risks, not hidden in a log.

### Agent Memory Register — the provenance ledger (`/register`)

Added in v1.20.9. A read-only ledger of *who wrote every memory and what it is based on*,
partitions into the three **origin** tiers — `human`, `model`, `imported` — with live
counts and owner/source/memory-kind filters. This makes *consequentiality* auditable: you
can see, at a glance, how much of the store is model-originated and where it came from,
and drill into any row's evidence (source URI, revision, heading, line range).

### Overview — the one-glance dashboard (`/`)

The decision-first home: a 4-card status row (Health / Snapshot / Retention / UMP), a
DAR-chain alert list, and a **top-5 pending queue preview** with one-click Approve/Reject
and a deep link into each review card.

---

## 4. The operator's decision procedure

This is the "how you actually do the job" part. When a proposal card is in front of you,
this is a defensible, repeatable evaluation. It treats you as a *critical evaluator*, not
a queue-clearer.

1. **Read the fragment, not the badges.** Badges (screen verdict, score) are input, not
   the answer. Read the actual captured text first.
2. **Check the sourcing prompt.** Ask: *was the model in a position to know this?* A
   fragment captured mid-task is context; a fragment captured because a prompt told the
   model to "remember this" is instruction. The two have different trust.
3. **Read the evidence, don't trust the summary.** Open the evidence modal. Is the span
   really there? Is the source URI real and current? The explainability paradox says a
   plausible summary makes you less critical — so don't take the summary's word for it.
4. **Treat `quarantined` as reject-until-proven.** If the injection screen flagged it,
   the default posture is *do not admit this to memory*. Override only with positive
   evidence, not with "it looks fine to me."
5. **Resolve conflicts deliberately.** If it conflicts with chunk #N, deciding to
   *supersede* is a real judgment: is the new fragment *true and replacing* the old, or
   are they both valid and merely different? Supersession expires the old chunk at a
   timestamp — it is a factual claim about the world, not bookkeeping.
6. **Reject with a reason.** A rejection with a reason is a data point; a bare rejection
   is a black box. The reason goes into the audit chain and is how the system (and you,
   next quarter) learns *why* captures are bad.
7. **Watch the gate-health strip, not just the queue.** If you are over-rejecting, the
   gate is catching too much and good capture is dying in the queue. If you are
   under-reviewing, decisions are expiring on you and the gate is deciding by silence. Both
   are *your* operational signals.
8. **Prefer suggest-re-ingest over drop.** When a fragment is worth keeping but badly
   captured, editing and re-ingesting preserves the knowledge. Rejection is for *not-worth-
   keeping*, not for *badly-captured*.

### Anti-patterns to actively avoid

- **Batch-accepting "because they're probably fine."** The scoring breakdown is there so
  you can *sample the evidence* — spot-check across the queue, not just at the top.
- **Only ever rejecting.** Over-rejection is as much a failure as under-review — it is
  automation bias in reverse, and it starves the memory.
- **Treating the SLA clock as the deadline to rubber-stamp.** The clock exists so a stale
  decision doesn't get made on context that has moved on. If it's near expiry and you
  haven't evaluated it, the honest answer is often *let it expire* (which auto-rejects with
  an audit trail) rather than a rushed approve.

---

## 5. Configuration that changes the loop

| Setting | Default | Effect on the loop |
|---|---|---|
| `BRAIN_PROPOSAL_TTL_SECS` | 7 days | How long a proposal can sit pending. Expiry auto-rejects with an audit row. |
| Plugin `captureMode` | `proposal` | Whether auto-capture routes through the review queue (`proposal`) or writes directly (`direct`, still screen-gated). |
| `BRAIN_INJECTION_THRESHOLD_HIGH/LOW` | — | Classifier banding thresholds: ≥ high → reject, ≥ low → quarantine. Flippable without restart. |
| `BRAIN_INJECTION_POLICY` | `quarantine` | `reject` vs `quarantine` for screen hits. |
| PII control | read-time | Deterministic output redaction for principals without `pii:read`; no write-time placeholder vault. |
| Per-kind retention | — | Query-time kind-default expiry; `GET /retention` sets overrides. |

Changing the proposal TTL changes the *reviewability* budget. A tighter TTL forces faster
review; a looser one gives the reviewer more time but lets stale context accumulate.
Either is a deliberate operator policy, not a default you inherit silently.

---

## 6. The audit trail is how consequentiality is proven

Every decision you make — approve, reject (with reason), supersede, expire, purge,
consolidate — is appended to the **SHA-256 hash chain** (`/audit`, `/audit/verify`). The
chain is tamper-evident: any edit to a prior row breaks every subsequent hash, and
`/audit/verify` recomputes it. This is what makes the human-in-the-loop *consequential*:
your judgment is not just performed, it is **recorded and reconstructable**, so that later —
for a recall trace, a compliance audit, or a DSAR — the question *"who decided this, why,
and on what evidence?"* has a verifiable answer.

See [**Security**](./security.md) for the chain itself and
[**MemGhost mitigation**](./MEMGHOST_MITIGATION.md) for how the human gate is the
countermeasure to memory-poisoning attacks.

---

## Next steps

- **Features** — the full capability tour: [Features](./features.md)
- **Overview** — why Brain Server exists: [Overview](./overview.md)
- **Client GUI** — every panel of the control room: [Client GUI](./client-gui.md)
- **MemGhost mitigation** — why the human gate is the poisoning countermeasure: [MemGhost](./MEMGHOST_MITIGATION.md)
