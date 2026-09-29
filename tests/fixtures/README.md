# tests/fixtures — R51 doc-state fixtures (P51.3)

These are the **pre-change document states**, extracted mechanically from git history and
committed *before* the red-proof predicate exists. They are the red-proof substrate for
`R51.1(a)` and `R51.1(b)`, and they make the red-proofs runnable forever.

**Why git history and not a synthetic file.** The operator fixed all three documents on
2026-09-29 (`619839e` in brain-steward-ip, `6007e62` in brain-server). The *trigger states*
— the states the pin exists to catch — therefore exist only at the refs below. A pin written
against "the current file" would never fail, which is exactly why the fixtures come first.

## Provenance

| Fixture | Extracted from | Repo |
|---|---|---|
| `prefix_kernel_architecture.md` | `6007e62^:docs/architecture.md` | brain-server |
| `prefix_plan_six_loops.md` | `619839e^:plans/PLAN_SIX_LOOPS_FINAL_ARCHITECTURE.md` | brain-steward-ip |
| `prefix_blueprint_system_architecture.md` | `619839e^:docs/blueprint/02-SYSTEM_ARCHITECTURE.md` | brain-steward-ip |

## Extraction verification (hit counts confirmed before the fixtures were relied on)

| Assertion | Expected | Confirmed |
|---|---|---|
| `619839e^` plan — Operate as the 5th **chain** box (after `Deflect`) | 2 | 2 (lines 36, 231) |
| `619839e^` blueprint — "run in order" | 1 | 1 |
| `6007e62^` architecture.md — "three nested loops" | 1 | 1 |
| `6007e62^` architecture.md — `Create\|Operate` | 1 | 1 (line 303) |

### The one that matters most

The single `Create|Operate` hit in the pre-fix kernel doc is line 303:

```
D4 --> D5["D5 Operate<br/>observe → attribute → improve"]
```

That is `Deliver`'s **software-lifecycle phase 5** — not the knowledge `Operate`. `Create` is
genuinely absent. **A bare name-mention predicate would have false-passed on the very file
whose gap this pin closes.** This is why the I51.3 predicate reads only enumeration lines
(table rows, list items, diagram boxes, mermaid node/subgraph labels) and never prose.

### A note on counting `Operate`

A loose pattern like `│ *Operate *│` returns **4** hits in the pre-fix plan, not 2 — the
extra two are the `Operate` phase *inside the Deliver box* (lines 62, 248), which is the
software loop's own phase, a different thing that happens to share the name. The pinned
count of 2 is specifically **Operate as the fifth box in the knowledge chain**, i.e. matched
as `Deflect ──▶ Operate`. The two distinct `Operate`s are the same collision the pinned
wording exists to keep apart.
