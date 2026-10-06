# Lesson 9: Your second brain (solo use)

**Level:** L1 · **Time:** about 20 minutes · **For the one-person deployment**

## Questions people ask

**Do I need a team for this?** No. Everything works solo: you are the
reviewer, the operator, and the audience. This lesson is the one-person
path through the same machine.

**What changes solo?** Your disciplines, not the machinery. Three habits
carry everything:

1. **End-of-day capture.** Five minutes: file what mattered today as
   proposals (facts, decisions, procedures). Tomorrow-you approves them
   with fresh eyes. The queue-as-separation still works with an audience
   of one.
2. **Weekly ten minutes.** `brain check-consistency`, clear duplicates,
   supersede what changed. Lesson 7, unchanged.
3. **Anchor + backup rhythm.** `brain anchor` recorded off-host after
   anything precious, `brain backup` on a schedule. Your memory is now
   the thing you would most hate to lose.

**What is temporal recall and why will you love it?** Ask what you knew
at a moment in time: recall accepts point-in-time queries, so "what did I
believe about the project in March, before the pivot" is one query, not
archaeology. Facts supersede, nothing is silently rewritten, so your past
selves stay reachable.

**Reminders?** The valet: `brain valet add "renew the domain" --at
2027-01-02 --repeat yearly`, `brain valet brief` each morning. Consent
control is yours by default (it tells you, it does not act).

## The solo setup, start to finish

```bash
brain setup                 # pick the personal profile
brain ingest-dir ~/notes --dry-run   # your existing notes, dry-run first
brain procedure "Weekly review" --step "clear queue" --step "consistency" --step "anchor"
brain anchor                # record the line somewhere off-host
```

## Exercise

1. Capture three facts about a current project as proposals. Approve them
   tomorrow. Note whether tomorrow-you edits one: that gap is the value.
2. Supersede one of them with a correction. Recall at the pre-correction
   time point.
3. Anchor, record off-host, install a weekly backup, and put "weekly
   review" in your calendar. The system is now a habit, which is the only
   form of memory that survives.

## Next

This is the last L1 lesson. [Back to the course index](../README.md).
