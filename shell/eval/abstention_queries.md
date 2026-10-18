# Abstention fixture — a corpus that ABSTAINS

**Phase:** P2 the surface · **Plan 08 §2.3** · **Date:** 2026-10-18

> **The frozen fixture stays frozen.** `tests/fixtures/eval_queries.md` is P1's
> baseline and its SHA-256 is pinned by `eval-preregistration.test.ts`. This is a
> NEW corpus, committed alongside its own digest in
> `shell/eval/abstention_preregistration.json`.

## Why this corpus exists

P1's T3 found **zero abstentions in 106 queries** and preregistered the
consequence: a dedicated abstention fixture is owed. Plan 06 §3 makes
abstention the product's sharpest behaviour, and Plan 02 §7 makes
"`low_confidence` renders as an answer" an exit criterion — neither can be
demonstrated against a corpus that never abstains.

## The shape, and why it is not arbitrary

`/recall` abstains only when the estimator returns `ClarifyQuery` **and** the
final hit list is empty (`abstention_decision`, `src/handlers/recall.rs`).
`ClarifyQuery` is one arm of `HeuristicEstimator::decide`, gated by:

    overlap >= 0.2      (hard — below this: IncreaseTopK)
    gap    >= 0.023     (hard — below this: RunPrf)
    score  <  0.35      (score = 0.4*overlap + 0.3*gap + 0.2*rr + 0.1*lex)

The corpus is built to satisfy all three at once, which is why it looks
contrived. Each row exists to hold one signal in place:

| # | Row | Job it does |
|---|---|---|
| 0 | contract / ratify-or-amend | **both legs at rank 1** — the top hit. Carries the stem `amend` but never the literal query term, so `lexical_density` stays **0** |
| 1 | contract entry, near-identical | **both legs**, pushed to fts rank 2 — its proximity to row 0 collapses the fused `gap` toward the 0.023 floor |
| 2 | contract entry, near-identical | second of the near-tie pair |
| 3 | amendment clause | **fts-only distractor** — lexically adjacent, semantically distant, so it takes an early fts rank WITHOUT joining the both-legs set that drives `overlap` |
| 4 | amendment clause | second fts-only distractor |
| 5 | retention schedule | vector-only filler, unrelated to the stem |
| 6 | audit cycle | vector-only filler |
| 7 | declarations amended | vector-only filler |
| 8 | schedule requires amendment | vector-only filler |

The measured signals for the abstaining query:

    overlap 0.20   gap 0.0240   lexical_density 0.00   score 0.2872  -> clarify_query

and with `min_relevance: "high"` the retained list is empty, so
`decision: low_confidence` is returned.

**Why `lexical_density` must be 0.** It is measured as
`content.contains(query_term)` — a LITERAL substring test against the top hit
only. FTS5 stems (`tokenize='porter unicode61'`), so `entries` matches
`entry` without appearing in it. A corpus whose top hit contains the query
term literally cannot abstain: the extra `0.1*lex` pushes the score over.

**Why the query is `entries` and not `amendments`.** `amendments` measured
`overlap 0.90, score 0.5673` — high overlap is exactly what *prevents*
abstention, because every query term then matches most rows. `entries` matches
one row in both legs and two more lexically, giving `overlap 0.20` exactly.

## The corpus

| 0 | The contract records every change that the parties later chose to ratify or amend. |
| 1 | The contract registers every change that the parties subsequently chose to ratify or amend. |
| 2 | Each contract entry lists every change the parties chose afterwards to ratify or amend. |
| 3 | A contract entry lists each change the parties subsequently chose to ratify or amend. |
| 4 | An amendment clause lets the parties ratify it without touching the rest. |
| 5 | The amendment clause permits ratification without amending anything further. |
| 6 | Retention schedules are reviewed before the annual audit cycle closes each year. |
| 7 | The audit cycle closes annually once every declaration has been amended on file. |
| 8 | Declarations are amended only when the retention schedule requires such a change. |
| 9 | The schedule requires an amendment before the audit cycle may close that year. |

## The judged queries

### A1 — must abstain

| | | |
|---|---|---|
| A1 | infra | "entries" |

Exactly one guaranteed abstention, because that is what was measured.

### Controls — must NOT abstain

These exist so the fixture cannot be satisfied by a corpus where *everything*
abstains. A passing abstention eval with no non-abstaining control is not an
eval.

| | | | |
|---|---|---|---|
| C1 | infra | "amendments" | `overlap 0.90, score 0.5673` |
| C2 | infra | "retention schedule" | `overlap 0.20, lex 1.00, score 0.3800` |
| C3 | infra | "audit cycle" | no cross-leg agreement |

`C1` is the measured near-miss: it matches most rows in both legs, so the
overlap that gate 1 rewards pushes the score well clear of the ceiling. `C2`
names its rows verbatim, so `lexical_density` is 1.00 and the extra `0.1*lex`
alone carries it over — the second, independent way a query can be saved from
abstention. `C3` is the weakest control: it matched no row in both legs, so it
cleared gate 1 the same way the abstaining query does and was stopped by the
`lex` and `score` arms instead.

## Measured results

Every row below was driven through a live kernel
(`target/debug/brain-server`, dedicated port, temporary database) on
2026-10-18. Two arms per query, because a control that returns nothing under
the filter but also nothing without it would pass for the wrong reason:

| id | query | `decision` | overlap | gap | lex | score | hits |
|---|---|---|---|---|---|---|---|
| A1 | `entries` | **`low_confidence`** | 0.20 | 0.0240 | 0.00 | 0.2872 | 0 |
| — | `entries`, unfiltered | `ok` | 0.20 | 0.0240 | 0.00 | 0.2872 | 5 |
| — | `contract entries`, unfiltered | `ok` | 0.20 | 0.0000 | 0.50 | 0.3300 | 5 |
| C1 | `amendments`, unfiltered | `ok` | 0.90 | 0.0243 | 0.00 | 0.5673 | 5 |
| C2 | `retention schedule`, unfiltered | `ok` | 0.20 | 0.0000 | 1.00 | 0.3800 | 5 |
| C3 | `audit cycle`, unfiltered | `ok` | — | — | — | — | 5 |

**Two rows return `decision: ok` with ZERO hits under `min_relevance: high`.**
That is the same emptiness the abstaining query produces, and it is why the
harness must compare the **decision**, not the hit count: a count-based
assertion would score `contract entries` as an abstention too. This is the
§6 token-count trap in its decision-shaped form — the assertion has to be about
*which state came back*, not how much.

**One abstention, not two.** `entries` is the fixture's whole yield, and it is
enough: A1's preregistered floor is **at least one**. A fixture that abstained
on two of two queries would be evidence that abstention is the default, which is
the opposite of the product claim.