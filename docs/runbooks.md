# Procedures & Runbooks

> Procedures are how a team stops improvising the same thing over and over.
> Brain Server stores the **current, correct way to do something** as a
> retrievable, ordered sequence of steps — so recall returns the same runbook to
> everyone, instead of each person's half-remembered version.

This page is the practical guide to authoring, finding, and maintaining
**procedures** (runbooks) in Brain Server.

## What a procedure is

A procedure is a `procedure`-kind **root** chunk, plus a series of `step`-kind
chunks linked to it with `next_step` edges. The root names the outcome; the
steps give the ordered actions.

```
        ┌────────────────────────────┐
        │  procedure "Onboard a new   │   root chunk (memory_kind=procedure)
        │  engineer"                  │
        └──────────────┬─────────────┘
                       │ next_step
              ┌────────▼────────┐
              │ step 1: "Create │   step chunk (memory_kind=step)
              │  a laptop image" │
              └────────┬────────┘
                       │ next_step
              ┌────────▼────────┐
              │ step 2: "Grant  │   ...
              │  repo access"   │
              └────────┬────────┘
                       ▼
```

Because steps are separate retrievable chunks, a recall can surface the exact
step a person needs, not just the whole runbook.

## Authoring a procedure

### From the CLI (fastest for a quick runbook)

```bash
brain procedure "Onboard a new engineer" \
  --step "Create a laptop image: build from the base image, tag with the date" \
  --step "Grant repo access: add to github team on-call, set membership to maintainer"
```

Rules for `--step`:
- Each step must be `title: content` (colon-separated, both non-empty).
- The root's default content is the title itself if you give no steps.
- Add `--domain <name>` to file the runbook under a team domain.

### Via the API

```bash
curl -X POST http://localhost:8765/procedure \
  -H 'content-type: application/json' \
  -d '{"title":"Onboard a new engineer","content":"Onboard a new engineer","steps":[
        {"title":"Create a laptop image","content":"build from base image, tag with date"},
        {"title":"Grant repo access","content":"add to github team, set maintainer"}
      ]}'
```

The response returns the procedure `id` and the `step_ids`.

## Finding a procedure

- **By recall** — scope to procedures so you don't get ordinary facts back:
  `POST /recall` with `{"query":"onboard new engineer","memory_kind":"procedure"}`,
  or `GET /search?memory_kind=procedure&q=…`. The plugin's `memory_recall` does
  this with `memoryKind: "procedure"`.
- **Read the ordered steps** — `GET /procedure/{id}/steps`.
- **Fetch a single step** — `GET /get/{id}` (the step's chunk id) or `brain get <id>`.
- **Walk a chained workflow** — `GET /graph/traverse` with
  `start: "<procedure title>", kind:"next_step"` walks from one runbook to the
  ones that follow it, so multi-stage processes are discoverable end to end.

## Changing a procedure

Procedures are versioned like any fact: when the steps change, **supersede**
rather than leave two competing runbooks. A new procedure supersedes the old
one (via the same supersession link the review queue uses), so recall returns
the current steps while the old sequence stays recallable `?at=<past>` for
history and audit.

Keep the *same* title when you supersede a procedure, so the "find by outcome"
query still resolves — the current version wins, and older versions are
preserved, not duplicated.

## Authoring habits that make procedures consistent

- **One procedure = one outcome.** A runbook titled "Onboard a new engineer"
  should not also contain "decommission a laptop." Split outcomes so recall
  returns the right one.
- **Title with the outcome, not the owner.** "How to grant emergency DB access"
  outlives "Mark's script." Owner names in titles are how islands start.
- **Steps are imperative and self-contained.** Each step should be actionable
  without the reader having to guess context, since it may be recalled alone.
- **Put the trigger in the root.** The root content should say *when* to run the
  procedure (e.g. "Run when a new engineer starts"), which makes `memory_kind`
  recall match the situation people describe.
- **Reference the source.** Add a `source` label so the team can trace where a
  runbook came from and when it was last reviewed.

## Procedures vs. proposals vs. plain facts

| Content | Where | Gated? |
|---|---|---|
| An ordered, repeatable runbook | `POST /procedure` / `brain procedure` | Direct (no proposal) |
| A durable fact or decision that needs human sign-off | `POST /ingest/proposal` (plugin `memory_store` default) | Yes — Review queue |
| A fact, policy, or note | `POST /ingest` / `POST /ingest/markdown` | Direct (screened) |

Use a **procedure** when there is an order and a repeatable outcome. Use a
**proposal** when a new durable fact should not enter shared recall until a
human approves it. Both are retrievable by `memory_kind`; they answer different
questions.

## Next steps

- **[One Brain for the Whole Team](./team-workflow.md)** — where procedures fit in the shared-store workflow.
- **[Knowledge graph](./knowledge-graph.md)** — `next_step` edges and typed traversal.
- **[Memory lifecycle](./memory-lifecycle.md)** — how a chunk is stored, versioned, and recalled.