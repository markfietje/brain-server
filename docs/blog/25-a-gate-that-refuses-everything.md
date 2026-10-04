# A gate that refuses everything is not a gate

*2026-10-04. The loop line spent this round proving that its own controls can
fire. That turned out to be the harder half of the work, and it is the half
nobody puts in a launch post.*

There is a failure mode in gated systems that is the opposite of the one everybody
watches for. You build a control. You test that it blocks the bad case. It passes.
You ship.

Now consider the version where the control blocks *everything*. Your bad-case
test still passes. So does your good-case test, if somebody later writes one,
because a refusal is still a refusal. The gate is green, the audit trail is full
of honest denials, and the system has quietly stopped doing its job while every
signal says it is fine.

We hit exactly this recently, and the reason we noticed is worth more than the
fix.

## The incident, briefly

We added a replay-determinism gate to a live promotion seam. The gate compares
what a delivery run re-derives against what it recorded, and refuses the
promotion when they disagree. We proved it works the obvious way: delete the gate,
watch a deliberately divergent trace get promoted, watch the gate stop it.

Then we ran the second check, which is the one that mattered. We planted a gate
that refused *every* promotion unconditionally. The divergent-trace proof from a
minute earlier still passed. Green. Because that proof only ever asked whether a
bad case gets blocked, and an always-refuse gate blocks every bad case in
existence.

The only reason we caught it is a separate pin asserting the fixture actually
diverges. Without that pin we would have shipped a gate that refuses everything,
proves nothing, and reports success.

## Why this is easy to miss and hard to fix

The awkward part is that the vacuous version is *safer-looking*. A gate that
blocks the bad case has evidence of working. A gate that blocks everything has
evidence of being strict. Both produce refusals in the audit chain. Only one of
them is a control, and nothing in the data distinguishes them, because the
refusals look identical.

The fix is not clever. It is refusing to treat a one-sided test as a proof. A
gate needs both halves: it blocks the bad case, *and* it allows the good case.
Write the second test or the first one proves nothing. That is a discipline
problem wearing a testing problem's clothes, which is why it survives so long.

## Where this shows up across the tree

We went looking after the incident, and the pattern was everywhere.

- **Fixtures that must actually break something.** A test that mutates a fixture
  and expects a mismatch has to mutate *exactly one* ordinal, and has to actually
  diverge. We learned this the hard way: a fixture rewrote a `seq` value and
  expected one mismatch, and produced three, because the sort order moved and the
  digest covers the `seq` too. The lesson is not the three. It is that the pin
  now asserts the **verdict**, not a **count**, because a count is a fact about
  the fixture rather than about the property under test.
- **Conjunctions that are true by definition.** A coverage check that passes
  vacuously when a requirement is *removed* from the list is structurally
  incapable of noticing, because popping a requirement can only make coverage
  look better. Ours declared the wrong watcher for this and we rewrote it to widen
  the claim rather than narrow the test.
- **Declared survivors.** Some mutants cannot be killed by the available checks.
  The honest move is to *declare* them, then have the panel verify the
  declaration, so a stale one is reported as its own failure.
- **Word-level language gates.** Source pins that scan for a forbidden construct
  in library code fired on their own doc comments and on `expect` calls. The fix
  was to restructure the code, not to soften the pin, and one of them lost a
  duplicate bounds check in the process.

## The part I keep coming back to

Every one of these is a test asserting that a *test* is honest. That is a strange
thing to build and an easy thing to skip, because the meta-tests do not make the
product better. They make the product's *evidence* better, which only matters if
somebody is going to read the evidence.

But somebody always is. That is the entire argument for a governed loop: at some
point a regulator, an auditor, or the person who has to answer for a decision
asks "how do you know this control works?", and the only acceptable answer is a
test that could have failed. A control whose test cannot fail is not evidence. It
is decoration with an audit trail.

So the loop now carries a rule about its own tests, not just about its data. It
is the same law the rest of the system runs on, applied one level up: a claim you
cannot demonstrate is not a claim, it is a hope, and the system refuses to record
it as one.

Where this is documented, the mechanism write-up sits with the governed
diagnostic loop research note, and the inert-control non-claims are stated
directly in [the create loop](../create-loop.md).