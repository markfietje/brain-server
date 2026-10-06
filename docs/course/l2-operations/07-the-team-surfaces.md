# Lesson 7: The team surfaces

**Level:** L2 · **Time:** about 20 minutes · **Commands + browser**

## What you will do

Tour the surfaces that make this a team system rather than a personal one:
governed runs, workload visibility, the scoreboard, shift imports, and the
complaints lifecycle. You will not use all of them. You will know they
exist and what each is FOR.

## Governed runs

A run is a case with a spine: steps, gates, questions, events, on a
timeline the machine advances only when the rules allow and humans say yes.

```bash
brain workflow open support          # open a run in a domain
brain workflow status <run>          # where it stands, what it waits for
brain workflow answer <run> "the order was placed on the 3rd"   # answer its question
brain workflow approve <run> 2       # clear a human gate on step 2
brain workflow crank <run> 5         # advance up to 5 transitions
brain workflow note <run> "customer asked to recheck next week" --reask
brain workflow handoff <run>         # the I-PASS packet from lesson 5 of Level 1
```

The word that matters is GOVERNED: some steps are gated on human approval,
some pauses are the machine asking a question it must not answer itself
(the answer binds to the live question, so a stale answer cannot satisfy a
new question), and every transition lands on the timeline. When you crank,
you advance it. You do not autopilot it.

## Workload and coverage

Two read-only views over the same record:

- **Workload** shows each person's open burden and fatigue signals. It
  alerts. It never reassigns. The system deliberately does not move work
  between humans on its own.
- **Coverage** joins the team's skills to the kinds of work arriving, so
  you can see the gap before it becomes a queue.

## The scoreboard

The scoreboard shows run outcomes and one quietly clever number: review
independence risk. If the same person approves everything, rhythmically,
the score rises. You met the human version in Level 1 lesson 3, the
4:45pm-Friday bulk approve. This is the machine noticing it for you, on a
dashboard, before an auditor does.

## Shifts and skills: WFM import

```bash
brain wfm-import shifts.csv --dry-run
brain wfm-import skills.json --domain acme
```

Shifts land as schedule data. Skills land as PROPOSALS, because changing
who-can-do-what is a governed act, not a spreadsheet paste. The dry run
shows the parse before anything lands. The seam is versioned and
additive-only, so the file your WFM tool exports today keeps working as
fields are added.

## Complaints, the whole lifecycle

The system carries an ISO 10002-shaped complaint path on the same single
record: complaints become governed runs, acknowledgment is its own audited
step with its own sweep for overdue ones, closure has a confirm gate so
nobody closes what is not resolved, and safety-relevant complaints route
toward the serious-incident path before any complaint keyword is consulted.
The public-facing "how to complain" page is generated from a policy
document you publish, and the KB builder refuses to ship the page without
one. An audience-facing promise with no published policy behind it is
exactly the kind of claim this system refuses to make.

## The public knowledge base

```bash
brain kb build --domain public --out ./kb --with-case-status --locales en,de,fr
```

Publishes approved articles as a static site, deterministic bytes, with a
signed manifest carrying provenance marks. If your organization needs to
disclose that content is machine-generated, the marks ride the manifest,
per artifact, and the verification is a command, not a squint.

## Exercise

1. Open a run, let it ask you a question, answer it, approve a step, crank
   it twice, then emit the handoff. Read the packet.
2. Import a two-row skills CSV with `--dry-run`, then for real. Confirm the
   skills landed as proposals, and approve one.
3. Build a KB for a domain with one published article. Open the index page
   it produced.
4. Find the workload view. If you are alone on your throwaway instance,
   the fatigue signals will be quiet. Note what it WOULD show.

## What you learned

- Runs are governed: gates, bound answers, timelines, handovers.
- Workload alerts but never reassigns. Coverage shows the skill gap.
- The scoreboard watches review independence, the human-drift signal.
- WFM skills import as proposals. Complaints and the KB are claim-safe by
  construction.

## Next

[Lesson 8: When things look wrong](08-when-things-look-wrong.md)
