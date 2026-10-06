# The brain-server course

> **What this is:** the complete, exercise-driven course for brain-server,
> a self-hosted AI agent memory server (one Rust binary, one SQLite file,
> human-gated writes, deterministic recall, zero tokens per query). Four
> tracks cover every audience the product declares, and every command in
> every exercise exists in the reference documentation.

Pick the track that matches what you do. Nobody needs all four.

| Track | For (the audiences page's own list) | Time | You will be able to |
|---|---|---|---|
| [Level 1: Working on the system](l1-operator/01-what-this-is.md) | Support and contact-center teams; knowledge workers using it as a private second brain | ~2 hours | Clear a review queue well, handle quarantine, run a data request, keep memory healthy |
| [Level 2: Running the system](l2-operations/01-installing-and-configuring.md) | Operators and admins; regulated deployers; edge and field deployments; delivery partners | ~3 hours | Install, configure, back up, restore, wire channels, run the edge, survive a bad day |
| [The builder track](lb-builder/01-why-memory-as-a-substrate.md) | AI and agent builders | ~2 hours | Integrate memory into an agent: API, UMP, MCP, the reference plugin, the architecture |
| [Level 3: Verifying the system](l3-verification/01-how-to-use-this-course.md) | Auditors, buyers, security and compliance reviewers | ~3 hours | Reproduce the whole posture on a throwaway instance and assemble an evidence pack |

Just asking questions? The [AI memory FAQ](faq.md) answers the twenty
people actually ask, each with a link to the lesson that proves it.

## How the exercises work

Exercises use the `brain` command line and plain web requests against a
throwaway copy, never production memory. Level 3 builds the throwaway in
its first check. If you break one, delete it and make another. That is
what it is for. Every command and route taught in this course is verified
against the [CLI](../cli-reference.md) and [API](../api.md) references,
which are themselves machine-checked against the source.

## Two words about words

**We never say the system is "compliant".** The system has a mapped
posture: every claim has a release that shipped it and a live check that
proves it. That table is the [proof map](../trust/proof-map.md). Level 3
teaches you to run it.

**A stale course is worse than no course.** If anything here disagrees
with what the system does, trust the system, and say so. The
[release checklist](../release-checklist.md) and
[changelog](../CHANGELOG.md) are the record of what changed and when.

## Where data rights sit

Everyday how-to is Level 1, lesson 6. The duty machinery, certificates,
tombstones, ledger deadlines, is verified in Level 3, lesson 4. Both name
their sibling.

## A note on "L3"

The memory protocol has its own conformance level, "UMP 1.0 / L3". That is
a protocol level, not course numbering. Pages mean the protocol only when
they write "UMP L3".

## Questions people ask

**Is this a course about AI?** It is a course about giving an AI agent
trustworthy memory: what to approve, what to refuse, how it stays
auditable, and how to prove all of it.

**Can I take just one track?** Yes. Each stands alone, and each links to
the others only where it genuinely needs them.

**How current is it?** It moves with the docs and the same release gates.
Check the changelog date against your version.

## Where to go next

- Understanding what this thing is: [Level 1, lesson 1](l1-operator/01-what-this-is.md).
- About to install it: [Level 2, lesson 1](l2-operations/01-installing-and-configuring.md).
- Building an agent on it: [builder track](lb-builder/01-why-memory-as-a-substrate.md).
- Here to assess it: [Level 3, lesson 1](l3-verification/01-how-to-use-this-course.md).
