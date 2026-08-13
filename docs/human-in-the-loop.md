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

### Erasure is a human action, not an agent capability

The write-back gate governs *entering* memory. The **erase side** is governed by the same
philosophy and an even harder rule: **memory can be erased, and only a human can erase it.**
An agent can read, and an agent can *propose* writes — but an agent **cannot delete** memory.

The reason is the product's governing control on memory — *"memory you can see, approve, and
erase."* Each verb is a **human-owned action**, and erasure is the most consequential of the
three because it is **unrecoverable**. A deleted memory is gone; there is no audit trail that
brings its *content* back. Granting an LLM that lever — the ambient authority to permanently
destroy stored knowledge mid-conversation, with no human gate — is exactly the shape of control
the design refuses to hand to the machine.

In practice this means:

- The agent's surface is **read + propose**: `memory_recall` / `memory_get` / `memory_verify` /
  `memory_graph_entity`, and `memory_store` (which, in the default `captureMode: "proposal"`,
  submits to the review queue rather than writing).
- The plugin's `memory_forget` tool was **removed in v1.20.25** — an agent can no longer
  hard-delete memory autonomously. (The server `DELETE /memory/{id}` route is untouched; only
  the *agent-facing tool* was taken away.)
- **Erasure is performed by a human** through the operator console and the HTTP API, both of
  which call the audited `DELETE /memory/{id}` / `POST /purge` / DSAR paths. (The `brain` CLI
  has no erasure command — delete is a console/API action.)

So the full authority model, stated plainly:

| Action | Who may perform it |
|---|---|
| Read / recall / verify | Agent **and** human |
| Propose a write (proposal queue) | Agent **and** human |
| Approve a write into memory | **Human only** (or an operator who set `captureMode: "direct"`) |
| Erase / purge / DSAR | **Human only** |

This asymmetry is deliberate and load-bearing: the model can contribute knowledge and read it,
but the two irreversible acts — **admitting** memory and **removing** memory — both require a
person.

The friction this imposes is **by procedure, not by accident.** Erasure is the one action
that cannot be undone, so the system refuses to make it cheap. Every delete is human-initiated,
attributed to a named principal, and recorded on the SHA-256 audit chain — the operator is
never "the system did it," they are "I did this, here is why." That is what the full
procedure in **[§7 The erasure procedure](#7-the-erasure-procedure)** formalizes: a repeatable,
auditable path for *every* deletion intent, with the "see-before-erase" and confirm steps that
force responsibility before anything is lost.

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
| `INJECTION_POLICY` | `quarantine` | `reject` vs `quarantine` for screen hits. |
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

## 7. The erasure procedure

*How a human actually removes memory.* This is the companion to §4 (which is about the
*write* gate — deciding what gets in). Erasure is the *remove* gate, and it is deliberately
harder: memory that is gone cannot be brought back. This section is the repeatable, auditable
path for every deletion intent, and the justification for the friction.

### Who may do what

| Role | Review / reject | Approve into memory | Erase / purge / DSAR | Scriptable (CLI) |
|---|---|---|---|---|
| Reviewer / QA / operator | ✅ | ✅ | ❌ | — |
| **Admin** | ✅ | ✅ | ✅ | `reconcile` / `source-delete` only |
| Agent (LLM) | ❌ | ❌ | ❌ | ❌ |

Two hard rules follow from the table:

1. **An agent can never erase.** The agent's surface is read + propose. The agent-facing
   `memory_forget` tool was removed in v1.20.25; an agent cannot hard-delete memory, period.
   The only way an LLM "becomes" a superuser is by obtaining a credential a human owns — so
   the human gate is only as strong as that credential never being readable by the agent (see
   *Why the friction exists* below).
2. **Reviewers and QA catch bad memory *before* it is admitted; only Admin can remove it
   afterwards.** The default QA posture is therefore **reject** at the queue. If QA finds a bad
   memory that is already approved, the correct move is to flag it for an Admin — not to hold
   delete authority.

### The decision flow

```
Operator / QA wants a memory removed
   │
   ▼
WHAT is being removed, and why?
   │
   ├─ A proposal still waiting in the Review queue (NOT yet memory)
   │     └─► Reviewer: REJECT (with a reason)      → audited; never persists. No Admin needed.
   │
   ├─ An already-admitted memory that is WRONG / stale / sensitive
   │     └─► Reviewer has NO delete authority
   │           ├─ record the evidence, then
   │           └─► Admin: Data panel → purge by chunk id(s) or owner
   │                 soft (ump/forget) OR hard (/purge) → tombstone + audit row
   │
   ├─ A DATA SUBJECT's data (GDPR Art 17 erasure)
   │     └─► Admin: Subjects (DSAR) console
   │           locate → PREVIEW footprint (dry-run, see-before-erase)
   │           → confirm → purge → deletion certificate (chain-verifiable)
   │
   ├─ Content the injection screen FLAGGED (quarantined)
   │     └─► Admin: Security panel → quarantine
   │           → RELEASE (admit) or DELETE (purge) the quarantined chunk
   │
   └─ A SOURCE / import (not individual memories)
         └─► Operator: `brain source-delete <id>`  (the CLI's only delete surface)
```

### The steps, path by path

**Path A — bad proposal (QA, no Admin needed).** Reject from the Review panel with a reason.
Rejection is audited, the reason enters the chain, and the content never becomes memory. This
is the *primary* QA delete: it happens before admission, so nothing has to be un-done.

**Path B — bad already-approved memory (Admin).** The reviewer cannot delete; they flag it.
Admin opens the Data panel, enters the chunk id(s) or owner, and chooses **soft** (`ump/forget`,
tombstoned) or **hard** (`/purge`, erased). Either writes a tombstone reason + audit row.
Default to soft unless the content must be physically gone (e.g., sensitive).

**Path C — data-subject erasure (Admin).** Subjects (DSAR) console: locate the subject →
**Preview footprint** (a dry-run of exactly what the live purge would erase, touching nothing) →
confirm → purge → receive a chain-verifiable **deletion certificate**. This is the GDPR Art 17
path and the one to use when a customer or a client's customer asks for erasure.

**Path D — quarantined content (Admin).** Security panel: the injection screen already held
the content out of memory. The Admin either **releases** it (admit after review) or **deletes**
it (purge). The safety decision is visible and overridable.

**Path E — a source / import (operator).** `brain source-delete <id>` is the **only** CLI
delete surface. It removes a source and its association; it is not a memory-content eraser.

### Why the friction exists (the justification)

- **Erasure is unrecoverable.** A deleted memory is gone; the audit trail proves *that* a
  delete happened and *who* did it, but it cannot restore the content. The human gate is the
  price of making the irreversible act deliberate instead of cheap.
- **It forces responsibility and accountability.** Every delete is human-initiated, bound to a
  named principal, and written to the SHA-256 chain that `/audit/verify` proves end-to-end. The
  system can always answer *"who deleted what, when, and why?"* — that is the accountability a
  SOC 2 / GDPR / EU AI Act review demands.
- **It defends against AI impersonation.** The threat is not an LLM "pretending" to be human —
  it is an LLM *obtaining the credential that proves humanity*. Because deletion requires a
  credential a human owns and an agent cannot read, an injected agent cannot escalate to erase.
  If a future power-user `brain forget` is ever added, it must keep this invariant: **no
  deletion without a human-owned credential that is not ambiently available to the agent.**
- **It is procedure, not a flag.** The see-before-erase preview, the confirm step, and the
  tombstone reason turn deletion into a repeatable, auditable discipline. A prompt or a config
  flag can be flipped by accident; a procedure cannot be.

### Is this negotiable for a deployment?

The gating above is the **default posture**, not a law. If a customer — a BPO, a contact
center, an enterprise — genuinely needs a different delete surface (e.g., a reviewer-scoped
"remove" on the review queue, or a power-user `brain forget`), we are **happy to include it**,
but **only under certain circumstances**, and the same invariants hold:

- **Human-owned credential only.** Any added surface must require a credential a human holds
  that an agent cannot read. No deletion may run on a token ambiently available to the LLM.
- **Still audited.** Every delete, by any surface, writes the same tombstone + SHA-256 audit
  row. No unlogged bypass.
- **Soft-first.** New surfaces default to tombstone (`ump/forget`); hard erase stays an
  explicit, extra step.
- **Role-scoped, least-privilege.** A reviewer-scoped remove flags for Admin erasure rather
  than hard-deleting directly; it never grants the reviewer Admin's full purge authority.

A customer asking for deletion flexibility is not asking us to weaken the model — they are
asking for the *right role* to be able to act. We can tune which role, on which surface, as
long as the four invariants above are preserved.

### The honest ceiling

"Audited" means *attributable and provable after the fact* — it does not mean *impossible to
abuse*. A rogue Admin acting within their own authority is not stopped by the ledger; the
ledger only guarantees you can find out. Prevention comes from the credential isolation above
and from least-privilege role assignment — not from the audit chain. And the CLI delete gap
(`source-delete` only) is deliberate: scriptable deletion is where accidents live. The trade is
a slower path for power users in exchange for a smaller surface for the machine.

---

## Next steps

- **Features** — the full capability tour: [Features](./features.md)
- **Overview** — why Brain Server exists: [Overview](./overview.md)
- **Client GUI** — every panel of the control room: [Client GUI](./client-gui.md)
- **MemGhost mitigation** — why the human gate is the poisoning countermeasure: [MemGhost](./MEMGHOST_MITIGATION.md)
