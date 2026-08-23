# The 500 that proved the audit chain works

*2026. A scoreboard endpoint crashed on a column that never existed — and the repair is a better argument for the audit design than the feature ever was.*

We ship an "honest ceilings" post because trust compounds when a vendor states
its limits. This post is the same discipline pointed inward: a real bug we
shipped, found live, and what its root cause says about designing evidence
systems that fail *closed*.

## The bug: querying a column that never existed

`GET /workflow/scoreboard` — the DPO's outcome dashboard over governed runs —
returned `500` with an honest message:

```
no such column: target in SELECT DISTINCT CAST(target AS INTEGER)
FROM audit_events WHERE kind = 'workflow'
```

The scoreboard's job is fail-closed green: a run only counts as "audited" when
an audit row actually references it. The query assumed audit rows carried a
plain-text integer target. They never did. **The audit schema stores hashes** —
`target_hash`, `detail_hash`, SHA-256 over the canonical strings — so a
reader cannot reconstruct references by casting; the information simply isn't
there in plaintext.

This is the same bug class we removed dead executor code for two releases
earlier (INSERTs into columns absent from the migrated DDL). Written against
an imagined schema, shipped behind a route nobody had exercised yet, caught by
the first live sweep.

## The repair: reconstruct honestly or don't reconstruct

Deleting the linkage check would have been easy and wrong — "green" that
can't see the evidence isn't green, it's optimistic. Instead:

1. **Name the canonical reference string.** Every run-bound substrate write —
   open, CAS transition, answer, state read — targets the same string:
   `run:{id}`. Outbox rows target `outbox:{key}`; calibration rows other
   strings. The convention already existed; the fix just reads it.
2. **Reconstruct via membership**: run `id` is audited iff
   `hash("run:{id}")` appears among workflow-kind `target_hash` values.
   One deterministic lookup per candidate run, bounded at 1,000 rows.
3. **Fail closed**: unparseable store, missing table, absent hash — none of it
   counts as green. Absence never lights up.

Pinned by an in-memory regression test with three rows — linked, unlinked,
wrong-kind — asserting exactly one survives.

## The sibling bug: contracts live at boundaries

The same live sweep surfaced a second failure with the same lesson in a
different costume. The token file supports rotation by holding multiple
whitespace-separated slots — the **server** accepts every slot. Our five
client binaries (`brain`, `mcp`, `bench`, both connectors) read the file,
trimmed outer whitespace, and pasted *the whole multi-line blob* into one
`Authorization` header. The embedded newline corrupted the request into an
empty-body 400 before auth even ran.

Server contract: "the file is a set." Client obligation: "send exactly one."
Both were documented; only one side enforced anything. All five binaries now
normalize through one shared helper (`first_token`), pinned by test, so the
next binary inherits the rule instead of re-deriving it.

## Why this wins the review

- **Fail-closed is a design posture, not a flag.** When the scoreboard
  couldn't prove linkage, it said so loudly (500) instead of scoring runs
  green on vibes. Loud failures are cheap; silent optimism is what audits
  find later.
- **Hashed evidence forces honest reconstruction.** Plaintext columns invite
  convenience-reads; hashes force every consumer to name the canonical string
  it trusts. That friction is the feature.
- **Boundaries need one shared implementation.** Five clients, one helper, one
  test. If your rotation story depends on every future client re-implementing
  the parse correctly, you don't have a rotation story.

**The takeaway:** ask vendors how their systems behave when evidence is
missing, ambiguous, or corrupt. "It fails loudly, changes nothing, and here's
the test" is the answer you want. We got to say it because we fixed it in
public first.

*The repaired linkage lives in [`src/handlers/workflow.rs`](https://github.com/markfietje/brain-server/blob/main/src/handlers/workflow.rs) (`audited_run_ids`);
chain verification you can run yourself: `GET /audit/verify` or the scripted
[`docs/trust/reproduce.md`](../trust/reproduce.md) walk-through.*
