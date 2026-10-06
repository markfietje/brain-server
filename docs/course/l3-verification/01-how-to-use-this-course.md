# Lesson 1: How to use this course

**Level:** L3 · **Time:** about 15 minutes · **Read this before running anything**

## Who this is for

You are an auditor, a buyer, a security reviewer, or the engineer who has
to answer one of those three. You do not want a tour. You want to know what
is claimed, and you want to check it yourself. This course is built for
that, and it is built a specific way, so the ground rules come first.

## The rule this system is built on

Every claim has two attachments: the release that shipped it, and a live
command that proves it against a running instance. Not a screenshot. Not a
clause in a policy PDF. A command you run. The full table is the
[proof map](../../trust/proof-map.md), and a scripted walk of it is the
[reproduce script](../../trust/reproduce.md), which runs the whole posture
against a fresh throwaway instance in about three minutes.

This course teaches you to read those two artifacts critically and to run
a handful of checks yourself. It does NOT teach you to conclude "this
system is compliant". No system is compliant in the abstract. This system
has a mapped posture: claim, release, evidence. Your job is to verify the
mapping holds, and to notice what is deliberately not claimed.

## The three habits of a good reviewer here

**Run it on a fresh instance.** Every lesson says this again, and lesson 2
makes it trivial. A check against a demo instance curated by the vendor is
theater. A check against an instance you spun up from the binary you were
given is evidence.

**Read the ceilings.** Every system page carries honest limits, and this
course collects the load-bearing ones in lesson 6. A vendor with zero
written ceilings has not thought about them. This one writes them down,
which is either encouraging or suspicious depending on your mood, and is
in either case verifiable.

**Timestamp your run.** Posture is a property of a version at a time.
Record the version (`brain status`) and the date with your evidence pack,
so nobody has to argue later about what was checked against what.

## What "levels" mean here, once

This course's L1/L2/L3 numbering is course numbering. Separately, the
memory protocol this server implements reports a conformance level of its
own, "UMP 1.0 / L3". When any page means the protocol it writes "UMP L3",
always. You will meet it once, in lesson 3, and it will be labeled.

## The shape of the course

- Lesson 2: the evidence model. What the audit chain is, and why the
  anchor matters more.
- Lesson 3: the reproduce script, walked section by section. This is the
  core of the course.
- Lesson 4: rights and records. DSAR machinery, certificates, tombstones,
  deadlines, and the residue ceiling, each with its check.
- Lesson 5: the human-control claims. Nothing auto-promotes, approvals
  bind to bytes, revocation works, and how to demonstrate each.
- Lesson 6: freshness and limits. How to know the posture you verified is
  the one still running, and the ceilings that are claimed to exist.

Each lesson ends in checks you can run and record. The final exercise
assembles them into an evidence pack: a folder of command outputs with
versions and dates, which is what "we verified it" looks like when it is
true.

## A note on tone

The system's own documentation is blunt about its limits, sometimes to the
point of dry humor. That is deliberate. Documents that never admit a
weakness are documents nobody can check. When you find a ceiling written
down here, it is not candor as decoration. It is the same mapping
discipline as everything else: the limit is the claim, and its presence in
the docs is checkable behavior.

## Next

[Lesson 2: The evidence model](02-the-evidence-model.md)
