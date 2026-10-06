# Lesson 7: Keeping the memory healthy

**Level:** L1 (101) · **Time:** about 20 minutes · **Commands on a throwaway copy**

## What you will do

Learn the small, boring habits that keep a shared memory worth trusting, and
the two repair operations you will actually use: merging duplicates, and
replacing an outdated fact with a corrected one.

## The weekly check

One command does most of the looking:

```bash
brain check-consistency
```

It reports four classes of trouble:

- **Duplicates.** Two copies of the same fact. They will drift apart. Merge
  or reject one.
- **Near-duplicates.** Two facts that are suspiciously similar. Usually an
  update that should have been a supersession.
- **Conflicts.** Two facts that cannot both be true. This is the important
  one. A conflict means some future answer was about to pick a side
  silently, and you get to pick it openly instead.
- **Stale sources.** Where memory came from that has since gone quiet.

None of these are emergencies. All of them get worse if ignored. Once a
week, run it, clear what it found, done in ten minutes.

## Supersession: the correction that keeps history

Facts change. The wrong move is deleting the old fact and writing a new
one, because then the system has no story of what changed, and any answer
built on the old fact yesterday becomes unexplainable today.

The right move is supersession:

```bash
# The new chunk replaces the old chunk.
brain resolve <new_chunk_id> <old_chunk_id>

# Changed your mind? Supersession is reversible.
brain undo-resolve <old_chunk_id>
```

After a resolve, the old chunk stops being served as current, but it stays
in the record, linked to its replacement. Answers now come from the new
fact. The question "what did we believe in March, and why did it change"
still has an answer. In the Review panel, an approval can carry the
supersedes link in the same motion, so a correcting capture becomes a
correcting approval.

This is also the answer to lesson 6's "please correct my address". Wrong
row superseded by right row, one audited operation.

## Retention: forgetting on purpose

Memory can carry an expiry per kind of content. The operator sets it
(`brain retention get` shows the current policy, `brain retention set`
changes it). When content passes its expiry, it decays out of current
recall. This is not deletion, the rows are not gone, it is a purposeful
"stop serving this as current". Some rules of thumb people learn the hard
way:

- Preferences deserve expiry. People change their minds.
- Contracts and legal facts deserve long or no expiry, plus a human review
  date instead of silent forever.
- Moods never belonged in memory anyway (lesson 3).

## Capture hygiene, the two-minute version

Healthy memory is mostly about what you let in, and the team habits are
short:

- Approve durable facts, not transcripts. If the candidate reads like chat,
  it will age like chat.
- One fact per candidate where you can. Two facts in one row means one goes
  stale and drags the other.
- Reject duplicates without guilt. The queue is not a quota.

## When recall looks wrong

If the assistant keeps saying "not confident" for things you know are
there, three checks, in order:

1. **Is it actually approved?** Pending candidates do not recall. It is
   astonishing how often this is the answer.
2. **Is it expired?** Check the retention policy for that kind.
3. **Is it superseded?** A replaced fact is deliberately out of current
   recall, and the replacing fact is what recalls.

If all three are fine, then it is a real puzzle, and that is the moment to
involve your Level 2 person, who has diagnostics you do not need in daily
life.

## Exercise

On a throwaway copy:

```bash
B=localhost:8765

# 1. Create a fact, approve it, note its chunk id.
#    (Use the flow from lesson 3, then:)
curl -s "$B/search?q=<your+fact>" | python3 -m json.tool | head -8

# 2. Create a corrected version, approve it, and supersede:
curl -s -X POST "$B/proposals/2/approve?digest=<digest2>&supersedes=<chunk1>"
#    Wait: supersedes is checked against LIVE candidates. If your server
#    numbers differ, the point is the &supersedes= parameter, not the ids.

# 3. Search again. The new fact serves. The old one is out of current recall.

# 4. Run the consistency check and read what it knows about both rows.
brain check-consistency
```

## What you learned

- A weekly `check-consistency` pass, ten minutes, prevents the slow rot.
- Supersession replaces facts while keeping the story. `resolve` and
  `undo-resolve`.
- Retention expires content by kind, on purpose.
- Recall surprises are usually pending, expired, or superseded. Check in
  that order.

## Next

[Lesson 8: Where the lines are](08-where-the-lines-are.md)
