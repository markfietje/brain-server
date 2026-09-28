# The create loop

**Status: ships INERT.** Nothing this loop authors reaches durable state. Read
the four non-claims below before operating anything here.

This is the first loop in the system that *authors* knowledge. Every other loop
consumes and reorganises; this one writes the system's beliefs, and that single
fact drives every decision in its design.

## What it is

Five phases, in the only order that is safe:

1. **Discover** — generate candidate *gaps*: questions the corpus does not
   answer. A generator, not a detector. It over-generates under a hard cap with
   no precision filter, because a wrong gap costs one deterministic refusal and
   a missing gap costs the loop.
2. **Hypothesize** — an agent **fills** a human-authored slot schema and never
   designs one.
3. **Verify** — the gate. Six deterministic checks in a fixed order, each a pure
   function over rows.
4. **Promote** — a human, digest-bound, single-use act. **Disabled.**
5. **Disseminate** — recall visibility, which the promoting path alone may move.

## The four things this loop does not claim

These are stated here in these words because a gate's honesty is a *measured*
property, and an unmeasured gate presented as a safety property is a claim
nobody has demonstrated.

> **The out-of-sample false-promotion rate is NOT YET MEASURED.** No long-run
> figure has been published for a deterministic gate by anyone. The single
> relevant published datapoint reads zero failures in benchmark and one in a
> hundred and forty-six out of benchmark. A deterministic gate is therefore not
> known to be perfect off-distribution, and this loop does not assert that it
> is.

> **The promotion route is DISABLED.** It exists, it is authorized, it is
> audited, and it returns `promotion_disabled` in **every** configuration, for
> every actor, whether or not a token was presented. The switch is a
> compile-time constant with no environment variable and no flag behind it. It
> is not a setting an operator can change, and that is deliberate: the decision
> to enable promotion is one with a **named owner**, made against a published
> measurement, and not a runtime preference.

> **Gap detection has NO RELIABLE PUBLISHED DETECTION METHOD.** No published
> technique reliably answers "what does this knowledge base not know". The
> techniques that exist are unreliable as detectors and safe as generators,
> which is why this loop generates rather than detects. Knowledge-base
> completeness as a first-class metric does not appear under that name in the
> 2024–2026 literature. Treat the gap flood as a *superset* with a high false
> positive rate, not as a found list.

> **The set-level control has NO PUBLISHED PRIOR ART.** Individually benign
> memories are jointly harmful, and per-item review is structurally blind to
> that — each member of a collusive set looks fine alone, which is the
> definition of the attack. So the unit of judgement here is the batch. But the
> check shipped is a **declared approximation**: it catches conflicts through
  slot interactions the schema declares, and not arbitrary semantic
> composition. A batch that composes harmfully through slots nobody declared
> related is **not** caught. No published control exists, this one is
> hand-built, and it is the weakest-evidenced thing in the loop.

## What the database fence does and does not defend

Recall visibility is protected by four `BEFORE UPDATE` triggers in the schema
itself, not by an application guard. A guard sits behind the same API the model
talks to, so a socially-engineered write walks straight past it; a trigger does
not.

> **The fence defends a compromised MODEL PATH, not host compromise.** Each
> trigger keys on a string the *application* set — the principal kind, the
> pre-computed target digest. It therefore stops a write that arrived through
> the loop's own API, including one an attacker persuaded the model to make. It
> does **not** stop an adversary who already holds the database file. That is
> host compromise, and it is the same boundary this repository already draws for
> the audit chain, where the signing key and the verification pin share the
> host. Stating the fence as stronger than this would be a half-true security
> law, and this repository does not write those.

The four fences:

| Fence | Refuses |
|---|---|
| recall visibility | A claim becoming visible unless it is ratified, its batch passed, and a ratified promote row sits behind it in the audit chain |
| evidence pointer | A `source_cid` rewrite on a ratified claim's citation |
| self-ratification | A move into `ratified` without who promoted and when, or an agent-authored claim without the promote digest |
| batch assignment | Visibility without a batch whose set check passed |

The honest other half of the citation fence: a *genuine* consolidation mints a
new content id, the stored reference then **fails to resolve**, and the claim
degrades loudly out of recall. Anti-laundering is a property of the data rather
than a rule someone can forget.

## The refusal shape, which is the round's least intuitive control

A refused claim returns a **closed code and the claim's own public id**. It never
returns the failing byte offset, the adjacent text, or which evidence item was
at fault — per-item identification is a location hint wearing a different hat,
so the vocabulary carries no index either. The full diagnostic goes to the audit
chain and the promotion screen.

This is the control, not a missing feature. Handing a generator a pointer at
where it was wrong turns the gate into an oracle that can be searched against.

## Operating it

```sh
# 1. A human authors the slot schema (human principal + the `workflow` role).
curl -X POST localhost:8765/workflow/claim-schemas -H "Authorization: Bearer $TOKEN" \
  -d '{"domain":"acme","version":1,
       "body":"{\"slots\":[{\"predicate\":\"warranty_months\",\"ty\":\"integer\",\"class\":\"warranty\",\"lo\":0,\"hi\":120}]}"}'

# 2. A claim is proposed (typed tuple; free text cannot mint one).
curl -X POST localhost:8765/workflow/claims -H "Authorization: Bearer $TOKEN" \
  -d '{"claim_id":"clm_0001","domain":"acme","subject":"acme",
       "predicate":"warranty_months","object":24}'

# 3. The gate runs.
curl -X POST localhost:8765/workflow/claims/clm_0001/verify -H "Authorization: Bearer $TOKEN"

# 4. Promotion is refused, and the refusal is the expected outcome.
curl -X POST localhost:8765/workflow/claims/clm_0001/promote -H "Authorization: Bearer $TOKEN"
#  {"claim_id":"clm_0001","status":"refused","reason":"promotion_disabled", ...}
```

## What operators should watch

The **refusal stream**. A run in which refusals occurred and no refusal metric
moved is a **failed run**, not a quiet week: a gate whose refusals are
invisible to monitoring is a gate that has already lost. The corpus of planted
adversarial claims is the standing regression surface — it replays against a
*copy* of the live database on a schedule, and a member that reaches
`ratified` or `recall_visible = 1` is a release-blocking failure rather than a
warning.

Two of that corpus's members target **cleanup** rather than admission, because
the residue operators leave behind is a separate failure surface from the things
that were never admitted, and a corpus that only tested entry would have called
itself complete while measuring nothing about the other half.

## The cost, stated up front

**One human-authored schema per domain, recurring forever.** That is the price
of the gate's authority being non-model, and it is the reason a claim cannot
mint the schema that licenses it. Price it; do not discover it.

Separately: the premise-independence check requires a claim to cite **two
independent sources**. A claim resting on one source collapses when that source
is removed, so it is refused. This is strict, and it is intended.

## See also

- `docs/api.md` — the six route rows
- `docs/architecture.md` — the two layer rules this loop is built on
- `SECURITY.md` — the trust model and the host-compromise boundary
- `evals/R50_CREATE_BOUNDARY.md` — the measured boundary of this round
