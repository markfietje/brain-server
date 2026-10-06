# Lesson 10: Determinism, evals, and self-measurement

**Level:** L3 deep dive · **Time:** about 25 minutes · **Run the measuring sticks**

## Questions people ask

**How do I know retrieval quality is real?** Pinned floors on a frozen
corpus: a fixed query set with a committed baseline, recall and MRR
floors enforced in CI (`brain eval --floor r5=0.85 r10=0.9`), and
published measured numbers rather than adjectives. The frozen corpus and
its baseline are versioned artifacts, so "quality" is a diffable property
across releases.

**What stops drift after shipping?** The drift census: `brain census`
re-scores the frozen corpus and diffs every cell against the committed
baseline under one global tolerance. A breach writes a hash-chained
finding and exits non-zero; a clean pass writes NOTHING (green rows are
noise that buries the red one). Unbaselined cells are reported loudly by
name, because an empty denominator is not a clean bill. It is
cron-driven on purpose: a measurer inside the thing it measures is a
correlated failure.

**What about claims the system makes about the world?** The disproof
sweep: every stored claim can carry a disproof condition, and
`brain disproof` evaluates each against the claim's own subject. Three
states, counted separately: satisfied (the disproof was NOT observed,
claim stands), REFUTED (observed, listed by id, does not stand), and no
verdict (prose conditions, never green). It writes nothing, ever: a sweep
that wrote its own verdicts back would be a promotion path, and promotion
is disabled.

**What about delivery determinism?** Replay gates: delivery traces are
recorded with per-stage digests, and the live promotion seam refuses a
trace whose re-derived evidence diverges. An always-refuse gate would be
as useless as an always-pass one, and an anti-vacuity pin holds the
middle.

## The self-measurement theme, named

Notice the pattern across lessons 7 to 10: chains, floors, census,
disproof, replay. This is a system that measures itself and refuses to
grade its own homework (every measurer writes nothing, runs on cron, or
is checked by an independent pin). As an assessor, that pattern is the
thing to verify, more than any single number: pick any two of the five
measuring sticks, run them, and record both the output and the fact that
the run itself was read-only.

## The L3 graduation

You now have: the reproduce script (lesson 3), the rights machinery
(lesson 4), the control claims with negative controls (lesson 5), the
ceilings (lesson 6), and the internals with bytes, attacks, matrix hunts,
and measuring sticks (lessons 7 to 10). Assemble the full pack, stamp it
with version and date, and you have done a genuine technical assessment
of an AI memory system in two afternoons, most of which was the system
showing you itself.

## Course complete

Back to the [course index](../README.md).
