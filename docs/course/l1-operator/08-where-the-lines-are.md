# Lesson 8: Where the lines are

**Level:** L1 (101) · **Time:** about 15 minutes · **No commands**

## What you will do

Finish Level 1 with the map of what this system will never do, who is
allowed to do what, and where to go when something is beyond your lane.
These are the lines that keep the trust you have been building all course.

## The lines, one last time

**Nothing enters memory without a person.** On the review posture, every
assistant capture waits for an approval. There is no setting that hands
this to the machine, and the create-loop machinery that would let the
system author its own knowledge is shipped deliberately disabled. If that
ever changes, it will be a loud, deliberate release, not a quiet Tuesday.

**No silent edits.** Memory does not get rewritten in place behind you.
Corrections are new facts that supersede old ones, on the record.

**No erasure without ceremony.** Deleting a person's data is a confirmed,
certificated, tombstoned operation. The assistant has no delete capability
at all.

**No guessing.** Below the confidence bar, the answer is "not sure". You
have met this as abstention, as quarantine, as refusals with reasons.

**One record.** Approvals, answers, audits, reports. Same source. The
disagreement-ending property from lesson 5 holds for everyone, including
people assessing your organization.

## Who may do what, in one table

| Action | Who |
|---|---|
| Read memory, search, recall | Team members with read access |
| Approve or reject proposals | Reviewers (that is you) |
| Ingest directly, run consistency, supersede | Operators |
| Export a person's data | Operators, on request |
| Purge a person's data | Operators, with confirmation, on the record |
| Place or lift legal holds, configure retention | Admins |
| Rotate keys and tokens, restore backups | The server keeper (Level 2) |

If you are asked to do something below your line, the system itself will
usually refuse first, and that refusal is doing you a favor. The second
favor is this course: you now know why.

## On tokens, in plain words

Access to the server is by credential. In the simplest setup there is one
secret token. In the two-token setup there are two: one for the person
(POWERFUL, operates the console, must never live in an assistant) and one
for the assistant (LIMITED, can remember and recall, cannot approve or
delete). The split exists so that a chatty assistant can never act with
human authority. If you ever find an assistant configured with the person
token, that is a real problem worth raising immediately, and the server's
own audit trail will show what it did while misconfigured.

## What is deliberately not here

From lesson 1, still true: there is no local-model chat window in this
release. The chat integration provides memory only. It loads no language
model of its own, which is why it is fast and free to run. If someone
promises you that feature today, they are describing a different product.

## How to ask for help, usefully

When you involve your Level 2 person or the maintainers, bring three things
and you will get help fast:

1. **What you did**, the exact steps or commands.
2. **What you expected, and what happened instead**, with the system's own
   words, not a paraphrase of them.
3. **When it happened**, because the audit chain turns a timestamp into a
   full story in seconds.

You may have noticed those are the same three things the audit trail keeps.
Once you internalize that, writing a useful report and reading one both
become trivial.

## Where Level 1 ends

You can now work a day on this system the way it is meant to be worked:
read the queue with judgment, treat refusals as information, run a data
request end to end, keep the memory healthy, and know exactly which
decisions were always yours.

Two doors from here. [Level 2](../l2-operations/01-installing-and-configuring.md)
teaches the running of the server itself, for when you are the person on
call. [Level 3](../l3-verification/01-how-to-use-this-course.md) is for the
people who arrive with a clipboard: it shows them, and you, how every claim
in this course checks out against a live system.

## Final exercise

The only exam that matters: on a throwaway copy, run one full day, alone.

1. Ingest two candidates from "a conversation" (write them yourself), one
   durable fact, one opinion.
2. Reject the opinion, approve the fact, and find your approval on the
   chain.
3. Ingest a corrected fact and supersede the first one.
4. Trip the screen with a nasty input, find it in quarantine, delete it.
5. Run a dry-run export for a test person, then the real export, and read
   the certificate.
6. Emit a handoff packet for the whole session and check whether you would
   hand it to a colleague.

If you can do those six things without looking anything up, Level 1 is
yours. If you cannot, the lesson to revisit is the one you just left.

## What you learned

- The five lines: no entry without a person, no silent edits, no casual
   erasure, no guessing, one record.
- Who may do what, and why the system's refusals enforce the table.
- Two tokens, two power levels, and why the split exists.
- Useful help requests are who, what, when. The chain keeps all three.
