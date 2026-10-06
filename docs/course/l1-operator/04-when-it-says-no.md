# Lesson 4: When the system says no

**Level:** L1 (101) · **Time:** about 20 minutes · **Console + optional commands**

## What you will do

Learn the three ways this system refuses, why each refusal is a feature, and
what to do when you meet one. If you take one habit from Level 1, make it
this: a refusal is information, not a malfunction.

## Refusal one: "I am not confident"

When you search and the best candidates are below the confidence bar, the
system does not serve them anyway. It abstains. In the console and in API
answers you will see this as a low confidence decision, and the assistant
using the memory will say something like "I don't have that in memory with
any confidence".

Why design it this way? Because the alternative is a system that always has
an answer, and some of those answers are invented. A memory system that
guesses is worse than no memory system, because its guesses come wearing the
authority of everything else it got right. The confidence bar is the difference
between a colleague and a fortune teller.

What to do: nothing, usually. Abstention is a correct answer to give a
customer. If it happens constantly for things you know are in memory, that
is worth reporting, and lesson 7 covers what to check.

## Refusal two: the quarantine

Some content never even reaches the queue. When a write comes in carrying
something that looks like an attack on the assistant, hidden instructions,
disguised scripts, invisible characters doing sneaky work, the screen holds
it in quarantine. Quarantined rows are not in memory, not searchable, and
not served to anyone. They wait for a person.

The screen is deliberately suspicious. A customer pasting a weird error log,
or a document full of unusual unicode, can trip it without meaning any harm.
That is the trade the design makes: it would rather hold a harmless oddity
than wave through one real attack.

Where you see it: the Security panel has the quarantine list. What you can
do with a row is exactly two things:

- **Release it.** You have read it, you see what it is, it is safe. It goes
  back into the normal flow and becomes recallable.
- **Delete it.** It is junk or hostile. It goes, with a record.

There is no "release and stop screening things like this". That button does
not exist, on purpose.

A quick way to see it work on a throwaway copy:

```bash
B=localhost:8765

# Ingest something with an obvious script tag inside it.
curl -s -X POST $B/ingest -H 'content-type: application/json' \
  -d '{"content":"normal note <script>alert(1)</script> end"}'

# It is held. Search for it: nothing.
curl -s "$B/search?q=normal+note" | head -5

# The quarantine lists it.
curl -s "$B/quarantine"

# You read it, decide it is what it looks like, and remove it.
curl -s -X POST "$B/quarantine/1/delete"
```

Note the shape of that flow. The machine caught it, held it, and told you.
You decided. The machine never quietly deleted anything on its own.

## Refusal three: "that is not mine to do"

Some things have no path at all, no matter who asks nicely:

- Nothing becomes memory without a person's approval (on the review posture,
  for every assistant capture).
- Nothing is erased without the erasure path, the one with confirmation
  steps and a certificate at the end.
- The assistant cannot delete memory. That capability does not exist for it.
  Deletion is a human action with a human record.

When you bump into one of these walls, the wall is the product working. If a
workflow genuinely needs one of these refusals lifted, that is a Level 2
conversation about configuration, not something to work around.

## The flagged queue, revisited

In lesson 2 Maria checked the flag count before the pending count. Now you
know what flags are for. A flagged item is one the screen or the score wants
a second pair of eyes on. Treat every flag as a "read this one fully before
deciding", and you cannot go far wrong.

## Exercise

1. On a throwaway copy, ingest the script-tag note from above. Find it in
   the Security panel's quarantine list. Release it instead of deleting it,
   then search for it. Confirm it now recalls.
2. Search for something you know is not in memory, any nonsense word. Look
   at the shape of the empty answer. That shape is abstention, not failure.
3. Find the screen verdict on a proposal in the Review panel. Match its
   vocabulary, clean, quarantine, reject, to the three sections of this
   lesson.

## What you learned

- Abstention below the confidence bar is a correct answer, not an error.
- Quarantine holds suspicious writes for a person. Release or delete, your
  call, on the record.
- Some refusals are load-bearing walls. Do not work around them.
- Flags mean "read this one fully".

## Next

[Lesson 5: One record, everyone reading from it](05-one-record.md)
