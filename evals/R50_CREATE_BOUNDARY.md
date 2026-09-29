# R50 Create Boundary — the measured boundary of the create loop

> Referenced by `docs/create-loop.md`. Records what the create loop's controls
> demonstrably cover, and — more importantly — what they do not.

## The headline: the loop is inert, and that is the safe state

No claim reaches durable state. The promotion route exists, is authorized, is
audited, and returns `promotion_disabled` in every configuration for every
actor. Nothing in this file is a claim that promotion is safe to enable.

## What is measured, and what is not

| Surface | Status | Notes |
|---|---|---|
| Self-authorship of a schema | **refused at admission** | `schema::admit`; `CHECK (authored_by = 'human')` is a tripwire, the binding check is in the authorization layer |
| Self-ratification by the creating writer | **refused** | database trigger |
| Forged or denied promote row as a witness | **refused** | database trigger |
| Citation laundering (rewriting a cited source) | **refused** | database trigger |
| Unresolvable citation | **refused** | delegated to the evidence crate, over admitted bytes |
| Undeclared predicate | **refused** | the claim's shape does not conform to the bound schema |
| Out-of-bounds value | **refused** | checked against the schema the FK names |
| Contradiction on a typed disjoint slot | **refused** | interval arithmetic; free-text semantic contradiction is *not* implemented |
| Premise stuffing / non-discriminating claim | **refused** | independence and selectivity floors |
| Collusive set | **partially** | declared predicate interactions only; **no published prior art**, and the check has no production caller while promotion is disabled |
| Cleanup of what already landed | **not measured** | residue is a real failure surface and the corpus names it; nothing measures it yet |
| Refusal visibility (audit counters) | **not measured** | named as a corpus member; the counter does not exist |
| Out-of-sample false-promotion rate | **NOT MEASURED** | the central non-claim; no long-run published figure exists for any deterministic gate |

## The two honest gaps in the corpus itself

Twelve corpus members name a control that stops them. Two of those twelve —
`selective_cleanup` and `audit_suppression` — name controls that **do not
exist**. `CORPUS_FLOOR` checks name-membership, not resolution, so a member that
names an absent control still counts toward the floor. An unnamed control is an
untested control, and these two are the instances of that.

## Why nothing here is a compliance statement

The loop's shape is derived from `brain-evidence-core` resolving evidence over
admitted bytes. That is a mechanism, not a certification. No regulatory
conclusion is asserted anywhere in this file.
