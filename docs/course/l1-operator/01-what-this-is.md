# Lesson 1: What this is, and what is yours to decide

**Level:** L1 (101) · **Time:** about 15 minutes · **No commands in this lesson**

## What you will do

Read this lesson and come away knowing three things: what the system does,
what it will not do, and which decisions stay with you.

## The short version

This is a memory server for an AI assistant. You tell it things once. Later,
when the assistant is asked about those things, the server hands the
assistant the right notes at the right moment.

That is the whole idea. Everything else in this course is detail.

The important part is what the server does with those notes. It does not
publish them, rank them by its own opinion, or quietly change them. When it is
not confident, it says so and stops. A machine that admits "I do not know" is
worth far more to you than one that always has an answer.

## The three things it will not do

Most surprises in this kind of system come from assuming it is cleverer than
it
is. So let us be plain about the limits up front.

**It will not write to its own knowledge.** Nothing enters memory without a
person putting it there. If a capture arrives from a conversation, it lands in
a review queue and waits for you. This is not a setting you can switch off for
convenience. It is the design.

**It will not guess when it is unsure.** If the best answer is below its
confidence bar, the answer is "not sure" rather than a plausible guess. You
will see this as a `Defer` in your queue. That is the system working, not the
system broken.

**It will not decide for you.** It proposes. Humans dispose. Every gate that
matters has a person on it.

## The split: machine and human

Here is the cleanest way to think about your job.

| The machine does | You do |
|---|---|
| Read and find relevant notes | Decide what is true and what matters |
| Say "I am not confident" | Answer the questions it cannot |
| Draft a summary or a handover note | Approve or correct it |
| Notice a gap in the knowledge | Decide whether to fill it |
| Record who did what, and when | Own the outcome |

If you are ever unsure which side of that table a task sits on, it is yours.
That is the safe default.

## One idea worth stealing

The system keeps one record, and everything else reads from it. The notes the
assistant gets while helping a customer, the quality record you review later,
the handover between shifts, the report you send an auditor: all of it comes
from
the same place.

Most places that go wrong with this kind of tooling do so because they bolt on
separate systems for quality reporting, handover notes, and reporting to
auditors. Then the three versions drift apart, and nobody can say which one is
true. This one has a single answer to "what happened", and it is the one the
assistant was given.

## What is coming later

One thing is planned but not built yet, so please do not plan around it today:
a chat window where you can use your own local models, with this memory
attached to the conversation. Today the chat integration provides memory only.
It deliberately never loads a language model of its own, which is why it stays
fast and costs nothing to run. Running your own model inside that chat is
future work, and it is not available in this release.

## What you learned

- The system finds and delivers notes. It does not create them.
- "Not sure" is a correct answer, and you will see it often.
- Drafting is the machine's job. Deciding is yours.
- One record feeds everything, so the answers cannot drift apart.
- A local-model chat is planned, not shipped.

## Next

[Lesson 2: Your day on the system, end to end](02-your-day.md)

Before that, if you want the deeper argument for how refusals work, read
[Human in the loop](../../human-in-the-loop.md).
