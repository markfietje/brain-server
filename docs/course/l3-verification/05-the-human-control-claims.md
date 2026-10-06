# Lesson 5: The human-control claims

**Level:** L3 · **Time:** about 20 minutes · **A few checks, one argument**

## The claims

Every "human in the loop" vendor makes the same three promises. Here they
are as this system makes them, each with the check that turns the promise
into evidence.

**Claim one: nothing becomes memory without a person.** Checked by the
reproduce script, section two: file a proposal, confirm it is held, watch
an undigested approval refuse, watch the digested one write memory. The
scope caveat you must record: the direct write endpoints, on an
`open` posture instance, insert immediately, screened but not gated. The
console and assistant captures ride the proposal path. Which posture a
deployment runs is a configuration fact, visible in the running config,
and your report should state it as a fact, not assume it.

**Claim two: the approval binds to the exact bytes.** The digest mechanics,
level 1 lesson 3, are enforced server-side. The check: edit a pending
proposal's content between reading and approving, and the approval is
refused as a mismatch. What this closes is the quiet swap, showing a
reviewer one text and storing another. It also closes the subtler variant:
edits invisible to the eye, zero-width characters and the like, still move
the digest. The digest binds to raw bytes, not to the displayed
impression, which is the strict direction.

**Claim three: the assistant cannot do human things.** The agent token
cannot approve, cannot purge, cannot delete memory at all (the capability
does not exist for it, not merely forbidden). The check: attempt each with
the agent credential and record the refusals. Then the negative control
that makes the test meaningful: succeed at each with the operator
credential. A refusal test without a success control proves only that
something is broken.

## The surveillance counterweight

Human control that nobody watches degrades into rubber-stamping. The
system measures its own loop: review independence risk and approval
uniformity on the scoreboard. As an assessor, this is where the interesting
questions live, because it converts "do humans review" (trivially yes)
into "are the reviews real" (the question that matters). Ask to see the
scoreboard over a few weeks of real queue data. A team of four with
perfectly uniform approval rhythm is a finding, and the system will show
it to you itself.

## Revocation and its one refusal

Agent and JWT identities revoke mid-flight. The static operator bearer
refuses revocation loudly, naming rotation plus restart as the remedy,
because a success reply over an unkilable credential would be a lie
during an incident. You can verify both behaviors in minutes, and the
refusal's existence is itself evidence of design intent: someone thought
about what a revoke verb owes the person clicking it during a bad day.

## The disabled self-promotion, and why you should poke it

The machinery for the system authoring and promoting its own knowledge
exists, and ships inert, behind a compile-time off switch with no
environment override. The documentation states the non-claim in plain
words. As an assessor, this is a gift of a test: ask what the promotion
route answers on the deployment under review (a named refusal), and ask
to see the configuration that would enable it (there is none, by design,
until a deliberate future release). Claims about absence are only ever as
good as the machinery that enforces the absence.

## Exercise

1. All three claim checks above, including the negative controls. For the
   digest mismatch: file a proposal, fetch the queue, then file a SECOND
   edit of the same proposal if your flow allows it, or simply attempt the
   approve with a digest from different content, and read the refusal.
2. Attempt one write with a read-scoped capability token. Record the
   refusal code.
3. Read a scoreboard with any history available to you. If the history is
   synthetic, say so in your pack, and note what real data would show.

## Next

[Lesson 6: Freshness and limits](06-freshness-and-limits.md)
