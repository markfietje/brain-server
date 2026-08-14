# One Brain for the Whole Team

> Stop working on your own island. A shared brain means the fact someone
> learned yesterday is the fact you fetch today — not a screenshot on someone's
> screen, a stale wiki page, or a re-derivation nobody asked for.

This page is the **operator-oriented** guide to making one `brain-server`
into everyone's shared memory. It assumes the API and CLI from
**[Quickstart](./quickstart.md)** and **[CLI reference](./cli-reference.md)**;
it focuses on the *habits and structure* that turn a single store into a
team asset instead of a personal scratchpad.

## 1. One server, many domains

A single server hosts many **domains** — each a scoped knowledge graph with its
own auto-routing centroids. Domains are the team boundary: namespaces like
`engineering`, `support`, `sales`, `hr` keep one topic from leaking into
another's answers while still being one installation to run, back up, and audit.

- **Name domains by the work, not the person.** `engineering` and `support`
  scale as people join; `mark` and `jess` don't.
- **Scope a recall to a domain** (`domain: "engineering"` in `/recall`, or
  `brain query "<q>" --domain engineering`) so you don't get cross-topic
  answers.
- **Retrieval auto-routes** by per-domain centroids and only falls back across
  domains on a confident miss — so a shared store still gives topic-correct
  answers.

Every ingest stamps `source` + immutable `revision` and an `origin` tier
(`human` / `model` / `imported`). The team can see, at a glance, how much of
each domain is model-originated and who/what it came from.

## 2. The shared rule: *every durable fact gets a home*

The single most effective team habit is a **write location convention**. Decide,
once, where each kind of knowledge lives, and the recall results become
predictable for everyone:

| Kind of knowledge | Where it goes | How | Retrieval |
|---|---|---|---|
| Decisions, policies, rules | `domain` + a clear title | `POST /ingest` / `brain ingest-dir` | `/recall` scoped to the domain |
| **Runbooks / how-to / procedure** | Procedure (steps) | `POST /procedure`, `brain procedure` | `GET /procedure/{id}/steps`, recall with `memory_kind:"procedure"` |
| New facts that need a human sign-off | Proposal (gated) | plugin `memory_store`, `POST /ingest/proposal` | `GET /proposals` Review queue |
| A fact that changed | Supersede, don't delete | `?supersedes=<id>` / `brain resolve <new> <old>` | history kept; `?at=<past>` recalls the old version |

The discipline is: **amend by superseding, not by re-writing.** Two competing
"current" versions of a fact are the earliest form of the island problem.
Supersession keeps one authoritative version and expires the old one — with the
old value still recallable at the time it was true.

## 3. Review as a team gate, not a bottleneck

Write-back is human-gated by default: a plugin `memory_store` with
`captureMode: "proposal"` lands as a **proposal**, not a memory. A human approves,
rejects (optionally superseding a conflict), or suggests re-ingest.

- **The Review queue is ordered by expiry first** — decisions that will
  auto-expire are surfaced before ones that can wait, so nothing silently
  rolls off.
- **The reviewer calibration strip** shows approve-rate, median decision
  latency, edit-rate, and screen-override rate. If anyone is rubber-stamping
  (approve-rate > 0.9 over ≥ 20 decisions), the strip says so. This keeps the
  gate honest for the *whole* team, not just one reviewer.
- **Erasure stays with admins** — reviewers can approve/reject but only an
  operator with the `brain` binary purges or DSARs. The authority split is
  deliberate.

For a team this means: shared content gets a shared, auditable quality gate,
and nobody can silently inject a bad fact into everyone's recall.

## 4. Procedures are the antidote to islands

The fastest way back from "everyone re-figures it out themselves" is to make
the current, correct way to do something **retrievable as a procedure**. A
procedure is a `procedure`-kind root chunk with ordered `step` chunks linked by
`next_step` edges — so the team can walk the *same* steps every time instead of
N personal improvisations.

- **Author once** with `brain procedure "<title>" --step "title: content" --step "title: content"` or `POST /procedure`.
- **Find on demand** — scope recall to `memory_kind:"procedure"` (or the plugin's `memory_recall`).
- **Walk it in order** — `GET /procedure/{id}/steps` returns the ordered steps.
- **Related runbooks** — `GET /graph/traverse` with `kind:"next_step"` walks
  from a procedure to what follows, so chained workflows are discoverable.

Keep procedures **small and singular** (one procedure = one outcome), title them
with the outcome ("Onboard a new engineer" not "John's stuff"), and supersede a
procedure when it changes rather than keeping two.

## 5. Make capture a default, not a chore

Cross-off the "did I write it down?" tax by making capture automatic:

- **autoCapture on** lets the plugin propose a capture after a successful turn —
  it stays a *proposal*, so it's captured but still human-gated.
- **autoRecall on** (default) means every turn pulls the *current, shared*
  answer first; the team is competing with the shared memory, not their own
  island of what they happen to remember.
- **Strictness:** `strictDomain` (default off) lets the server route across
  domains on a confident miss; turn it on once a domain is well-populated to
  tighten precision.

## 6. Hygiene that keeps the shared store trustworthy

- **Put the source with the fact.** Ingest with a `source` label and keep
  `[[relation::entity]]` links so provenance and the graph stay meaningful.
- **Use the skip patterns.** `BRAIN_INGEST_SKIP_PATTERNS` lets you define
  prefixes that are never ingested (e.g. `!redacted`), so junk doesn't pollute
  shared recall.
- **Reconcile sources.** `brain reconcile <path>` and
  `POST /sources/reconcile` sweep orphans from deleted sources so the shared
  store doesn't answer from dead material.
- **Check consistency.** `brain check-consistency` surfaces duplicates,
  conflicts, and stale sources — run it as part of a team cadence, not just
  when something looks wrong.

## 7. Everyone sees the same audit

A tamper-evident SHA-256 audit chain records every ingest, approval, denial,
and purge. That is a *shared* guarantee the whole team relies on: the store
everyone draws from has not been secretly rewritten. DSAR workflows give a
chain-verifiable deletion certificate, so "the shared brain" also extends to
"the shared compliance story."

## Next steps

- **[Quickstart](./quickstart.md)** — get a server up and add your first domain.
- **[Procedures & runbooks](./runbooks.md)** — author, find, and maintain team procedures.
- **[Memory lifecycle](./memory-lifecycle.md)** — how a fact travels from capture to recall.
- **[Security](./security.md)** — multi-operator auth, tokens, and the audit chain.