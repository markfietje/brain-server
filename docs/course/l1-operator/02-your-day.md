# Lesson 2: Your day on the system, end to end

**Level:** L1 (101) · **Time:** about 20 minutes · **A browser helps, no commands required**

## What you will do

Follow one person through one shift. By the end you will recognize every
screen they touched and every decision they made, and you will know which of
those decisions was theirs alone.

The screens in this lesson are the console's twelve panels. You do not need
to memorize them. Four of them carry most of the day, and this lesson points
at each one when it happens.

## 8:55, before the queue opens

Maria signs in and lands on the **Overview** panel. It is the home screen:
status cards, anything the system wants to draw attention to, and the count
of items waiting for a person. She is looking for two numbers. How many
proposals are pending, and is anything flagged?

Today: nine pending, one flagged. The flag matters more than the count. A
flagged item is the system saying "look closer at this one before you
decide". We will meet flags properly in lesson 4.

## 9:00, the review queue

The **Review** panel is where candidates wait. These are things the system
or an assistant drafted that would become memory if a person said yes. Nobody
has said yes yet, and nothing becomes memory until someone does.

Maria opens the first one. A capture from a customer conversation suggests
the customer prefers callbacks before noon. The panel shows her several
things at once, and this is the part worth slowing down on:

- **What the candidate says**, word for word.
- **Where it came from**, the source.
- **A screen verdict**, what the injection screen thought of it.
- **A score breakdown**, why the system ranked it the way it did.

She is not grading the score. She is answering one question: is this true,
and is it useful? The system already decided it was relevant enough to
propose. Truth is her department.

She approves. Two things happen. The memory is written, and the audit trail
records that she, specifically, approved these exact words, at this time.
We come back to why that second part matters in lesson 5.

The next candidate is a near-duplicate of something already in memory. She
rejects it. Rejecting is not deleting. The record of the rejection stays,
forever, with the same who-what-when. You never erase the history of a no.

## 11:30, the system says no first

A recall comes back with no results, and the assistant tells the customer
"I don't have that in memory with any confidence". Annoying? A little. Wrong?
Not even slightly.

This system has a confidence bar. Below it, the honest answer is "not sure",
and the system gives that answer instead of a smooth guess. You will see it
often. Every time you do, it refused to make something up, which is the
single most expensive mistake the chatty kind of system makes.

## 13:00, a person asks for their data

A customer wants to know what the system holds about them. This is a normal
request with a normal path, and lesson 6 walks it in detail. The short
version for today: the operator runs an export, the system gathers every row
belonging to that person, hands over the export, and writes a certificate
proving the export happened. If the person asks for deletion, that path
exists too, with more care around it, because deleting is a one-way door.

Maria files the request and moves on. It took minutes.

## 15:00, shift handover

Her shift ends at three. She opens the run she was working on and emits a
handoff packet. This is a summary built by the machine from the run's own
record: what the case was, where it stands, what is still open, what the next
person should look at first.

She reads it before sending it. That is not a formality. The machine drafts,
she decides, and if the draft is wrong she fixes it or throws it away. The
person receiving it starts their shift knowing exactly what Maria knew, from
the same single record, not from Maria's memory of her memory.

## 16:00, someone else's turn

Priya picks up the handoff. She does not ask Maria anything. She does not
check three different tools to see if they agree. She reads the packet, opens
the run, and continues. The audit trail already shows her that the morning's
approvals were Maria's, on which words, at which times.

## The shape of the day, zoomed out

Notice what Maria never did. She never wondered whether a memory was the
"real" one. She never copied anything between systems. She never trusted a
summary she could not check. And every consequential thing she did, approving,
rejecting, exporting, handing over, left a record with her name on it that
she cannot quietly edit later.

That is the job. Everything else in this course is technique.

## Exercise

You will need the console open, any instance, even an empty one.

1. Find the Overview panel and write down the pending count.
2. Open the Review panel. If it is empty, that is fine, look at the columns
   and controls. Which piece of information would you check first on a real
   candidate, and why?
3. Find the Audit panel. Look at the kinds of events it records. Find one
   kind that corresponds to a decision by a person.

There are no wrong answers to 2 and 3. In lesson 3 we do this for real,
with live candidates and the two decisions you will make thousands of times:
approve, and reject.

## What you learned

- Four panels carry the day: Overview, Review, Audit, and the run view.
- Approve writes memory and records you. Reject keeps the memory out and
  keeps the record of your no.
- "Not sure" from the machine is the system working.
- Handovers are drafts you check, built from the one record everyone shares.

## Next

[Lesson 3: The review queue, done well](03-the-review-queue.md)
