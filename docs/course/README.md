# The brain-server course

Three courses, three jobs. Pick the one that matches what you do, not the
one that sounds most advanced. Nobody needs all three.

| Course | For | Time | You will be able to |
|---|---|---|---|
| [Level 1: Working on the system](l1-operator/01-what-this-is.md) | People who use the console day to day. Reviewers, team leads, anyone who reads or approves memory. No technical background assumed. | About 2 hours | Clear a review queue well, handle a quarantine, run a data request, keep the memory healthy |
| [Level 2: Running the system](l2-operations/01-installing-and-configuring.md) | The person who installs and looks after the server. Comfortable with a terminal. | About 3 hours | Install, configure, back up, restore, wire up channels, and know what to check when something looks wrong |
| [Level 3: Verifying the system](l3-verification/01-how-to-use-this-course.md) | Auditors, buyers, security reviewers. Written check-first: every lesson ends in something you run yourself. | About 2 hours | Reproduce the whole security and rights posture on a throwaway copy, and know exactly what is claimed and what is not |

## How the exercises work

Exercises use the `brain` command line and plain web requests. They are
written to run against a throwaway copy, never your production memory. Level 3
starts by showing you how to make one in a few minutes. If you break a
throwaway copy, delete it and make another. That is what it is for.

## Two words about words

**We never say the system is "compliant".** The system has a mapped posture:
every claim has a release that shipped it and a live check that proves it.
That table lives in the [proof map](../trust/proof-map.md). Level 3 teaches
you to run it. Any page, vendor deck, or course that tells you a tool makes
you compliant is selling you something, and this is not that.

**A stale course is worse than no course.** If anything in these lessons
disagrees with what the system actually does, trust the system, and please
say so. The [release checklist](../release-checklist.md) and the
[changelog](../CHANGELOG.md) are the record of what changed and when.

## Where data rights sit

One placement decision, made once so you can find things: the everyday how-to
of a person asking for their data (export, fix, delete) is taught in Level 1,
because that is real daily work. The machinery that makes those duties
provable, certificates, tombstones, deadlines on the ledger, is verified in
Level 3. If you came for one and needed the other, both courses name their
sibling.

## A note on "L3"

Confusingly, the memory protocol this server speaks has its own conformance
level called "UMP 1.0 / L3". That is a protocol level, not this course
numbering. When a lesson means the protocol it will say "UMP L3" every time,
so you always know which L3 is talking.

## Where to go next

- New here, and just want to understand what this thing is? [Level 1, lesson 1](l1-operator/01-what-this-is.md).
- About to install it? [Level 2, lesson 1](l2-operations/01-installing-and-configuring.md).
- Here to assess it? [Level 3, lesson 1](l3-verification/01-how-to-use-this-course.md), and budget one sitting for the
  [reproduce script](../trust/reproduce.md) it walks you through.
