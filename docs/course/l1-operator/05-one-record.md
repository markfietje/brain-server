# Lesson 5: One record, everyone reading from it

**Level:** L1 (101) · **Time:** about 20 minutes · **Console + optional commands**

## What you will do

Understand the single idea that makes everything else in this system
trustworthy: there is one record, and everything reads from it. Your approvals,
the assistant's answers, the audit you show a customer, the report you send
an auditor, all come from the same place, so they cannot drift apart.

## The record has three layers

Think of it as three layers, each one answering a different question.

**Memory answers "what do we know".** The approved facts. What the assistant
is given when it answers.

**The audit chain answers "how did it get there".** Every consequential
action, approvals, rejections, exports, deletions, configuration changes,
lands on a chain where each entry carries a fingerprint of the entry before
it. Change any entry after the fact and every fingerprint after it stops
matching. Tampering is not prevented, it is made loud and visible.

**The runs answer "what happened in this case".** Troubleshooting cases,
their steps, their questions, their handovers. A run is where the day's real
work lives, and its timeline is served from the same single store.

## Why fingerprints matter to you

You will never compute one. Someone assessing the system might, and Level 3
shows them how. What matters at your level is what the fingerprints make
true:

- Nobody, including the person who runs the server, can quietly rewrite the
  history of who approved what.
- Your yes from six months ago is still provably your yes, on those exact
  bytes. The digest you met in lesson 3 is part of this.
- "The system did it" is never a complete sentence here. Something did it,
  under some identity, at some time, and the chain says which.

One healthy habit, and it takes ten seconds: when something consequential
happens, an export, a purge, a key change, glance at the audit panel and see
the event landed. The chain only earns trust if people occasionally look at
it.

## The run timeline and the handover

In lesson 2, Maria emitted a handoff packet at shift end. Here is what it
actually is. The run's timeline is a series of events: the case opened, a
step completed, the machine asked a human a question, the human answered,
a note was added, the case closed. The handoff packet is a summary built
from those events, in a standard format (I-PASS, if you meet the term:
severity, patient, action, situation, safety concerns, borrowed from
medicine's handoff discipline).

You can also add case notes yourself. Notes are screened like every other
write, they land on the timeline, and you can use them to flag that the
customer should be asked again, which the system records as its own event
type. When you write "customer asked us to check back next week", that is
not a wish, it is a tracked thing.

## Everyone sees the same audit

The Audit panel is not an admin-only view. Everyone on the team reads the
same chain. When two people disagree about whether something was approved,
the chain ends the argument in about fifteen seconds. This is, quietly, one
of the system's best team features: it removes the "I thought you did it"
class of argument entirely.

## Try it

On a throwaway copy, with one approved proposal from lesson 3:

```bash
B=localhost:8765

# The chain verifies as a whole.
curl -s $B/audit/verify
# Expect {"ok":true} or an "ok" verdict in the JSON envelope.

# Recent events, newest first. Find your approval.
curl -s "$B/audit?limit=5" | python3 -m json.tool | head -30
```

In the console, open the Audit panel and filter by kind. Count how many
distinct event kinds exist. The variety is the point: this is not a log of
server noise, it is a ledger of decisions.

## Exercise

1. Approve a proposal, then find that approval in the audit chain. Which
   fields carry who, what, and when?
2. Open a run in the console (or ask your Level 2 person to create one).
   Find the timeline. Emit a handoff packet and read it. Would you hand
   this to a colleague as-is?
3. Ask a colleague to do step 1 on their own approval. Compare what you
   both see. Same chain, same shape, different names.

## What you learned

- One store feeds memory, audit, and case runs. Nothing can drift.
- The audit chain makes history tamper-evident, and your decisions stay
  provably yours.
- Handovers are generated from the run's own timeline, in a standard
  format, and you check them before sending.
- Everyone reads the same audit. Use it to end arguments.

## Next

[Lesson 6: People and their data](06-people-and-their-data.md)
