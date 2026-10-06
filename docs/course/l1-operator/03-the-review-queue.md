# Lesson 3: The review queue, done well

**Level:** L1 (101) · **Time:** about 25 minutes · **Console + a few optional commands**

## What you will do

Learn the two decisions you will make most often, approve and reject, and
learn to make them well rather than fast. This is the lesson the whole system
depends on. Everything upstream, capture, screening, scoring, exists to put
a good candidate in front of you. Everything downstream trusts that you
looked.

## What a candidate actually is

When an assistant or an auto-capture wants to add a memory, it cannot just
write it. It files a proposal. A proposal is a draft that becomes real
memory only when a person approves it. The system scores the draft and holds
it. It is, on purpose, hard to promote something by accident.

There is one honest wrinkle to know about, because you may meet it. Direct
writes by the operator, through the command line or the write endpoints, can
go straight in, screened but not held for review, depending on how the
server was configured. The person who runs your server (a Level 2 person)
knows which posture yours uses. The queue you see in the Review panel is
everything waiting for a human, and on a review-configured server, nothing
from an assistant bypasses it.

## The four things to check, in order

The Review panel shows you the candidate's words, its source, its screen
verdict, and its score breakdown. Here is the order Maria from lesson 2
checks them, and why this order works.

**1. Is it true?** Not clever, not well phrased. True. You are the fact
checker. If you cannot tell whether it is true, that is a reject or a
question to the source, never an approve-and-hope.

**2. Is it durable?** "Customer was angry today" is a mood, not a fact.
"Customer canceled the premium plan on the 4th" is a fact. Moods rot.
Facts compound. When you approve moods, future answers get worse, politely
and confidently.

**3. Is it in the right words?** You are approving the exact words on
screen. Not the gist. The approval binds to those bytes, which is why the
panel shows you a digest, a short fingerprint of exactly what you are
approving. If someone edits the candidate after you looked, your approval
no longer matches, and the server refuses it. That refusal is protecting
you.

**4. Do we already know this?** Duplicates are not harmless. Two copies of
the same fact drift apart over time, and then a future answer has to pick
between them. If it is a duplicate, reject it. If it corrects an old memory,
that is a different operation, supersession, and lesson 7 covers it.

## Approve

When you approve, three things happen as one step: the memory is written,
your identity and the content digest go on the audit chain, and the
candidate leaves the queue. If the system thinks the new fact replaces an
old one, the approval can also mark which old memory it supersedes. The old
one then stops being served as current, but is not deleted. History stays.

## Reject

When you reject, the candidate never becomes memory, and the rejection is
recorded: who, what, when. One detail that surprises people: the server
records the rejection itself, not your reason. You can type a reason and it
is accepted, but it is not stored. If your team wants reasons, keep them in
your own case notes. Knowing this prevents a very specific embarrassment in
month three.

## The queue is not a backlog to burn down

The failure mode of every review queue everywhere is that clearing it starts
to feel like the goal. It is not. A fast wrong approve is worse than a slow
queue, every time, because the wrong memory sits in every future answer,
confidently. Some anti-patterns, drawn from the system's own operating
doctrine:

- **Approving by pattern match.** "Looks like the ones I approved before" is
  not a check. Read it.
- **Rejecting anything confusing.** Confusing is a reason to slow down, not
  a category.
- **Bulk-approving at 4:45pm on Friday.** You know who you are. The audit
  trail timestamps everything, and a cluster of approvals in the last hour
  of the week reads exactly like what it is.

The system can even show the team's approval patterns, because uniform,
rhythmic approving is a known sign that humans have stopped reading. That
feature exists to start conversations, not to shame anyone.

## Try it on a throwaway copy

These commands use curl against a local test server. If you do not have one
running, any Level 2 person can make one in minutes, or come back to this
after lesson 1 of Level 2.

```bash
B=localhost:8765

# 1. File a proposal. It is scored and held. No memory row exists yet.
curl -s -X POST $B/ingest/proposal -H 'content-type: application/json' \
  -d '{"content":"Bignay is an antioxidant-rich alternative to blueberry.","title":"t"}'

# 2. See it waiting, and grab its digest (the fingerprint of its words).
curl -s "$B/proposals?status=pending" | python3 -m json.tool | head -20
D=$(curl -s "$B/proposals?status=pending" | python3 -c \
  "import sys,json;print(json.load(sys.stdin)[0]['content_digest'])")

# 3. Approve, carrying the digest. Try approving WITHOUT the digest first.
curl -s -X POST "$B/proposals/1/approve"
# The server refuses: digest_required. Your yes has to name the exact bytes.

# 4. Now with it.
curl -s -X POST "$B/proposals/1/approve?digest=$D"

# 5. Prove it became memory, and that it is now retrievable.
curl -s "$B/search?q=blueberry" | python3 -m json.tool | head -12
```

Step 3 is the lesson in miniature. The refusal is not bureaucracy. It is the
server making "I approve these words" mean something specific.

## Exercise

On a throwaway copy:

1. File two proposals: one plain fact, one opinion dressed as a fact, for
   example "the best product is clearly ours".
2. Approve the fact with the digest. Reject the opinion.
3. Find both decisions in the audit log, `$B/audit?limit=10`, and identify
   which fields tell you who, what, and when.
4. Try to approve the same proposal twice. Read what the server says. A
   decided candidate is gone from the queue, by design.

## What you learned

- Proposals are held drafts. Your approval is the only door into memory.
- Check truth, durability, wording, and duplication, in that order.
- Approval binds to the exact bytes via the digest, and the server enforces it.
- Rejection is recorded, but your typed reason is not stored.
- Speed is not the metric. The queue is a checkpoint, not a backlog.

## Next

[Lesson 4: When the system says no](04-when-it-says-no.md)
