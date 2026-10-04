# Catch it on the way in, because it comes back every turn

*2026-10-04. Prompt injection gets treated as a per-request problem. In a memory
store that is the wrong shape, and the screen has to sit somewhere specific
because of it.*

Here is the thing about prompt injection in a chat application: it is a problem
for one turn. The model reads something hostile, does something unwise, and the
conversation moves on. Bad, but bounded.

Move the same payload into a memory store and the arithmetic changes completely.
The store hands that text back to the model on **every future retrieval**. The
attack is not a request, it is a resident. And now add the second audience nobody
mentions: the human operator who opens the console three weeks later to read what
the agent wrote down. That person has no way to know a sentence in a memory was
aimed at the model rather than at them.

That is why our injection screen sits at the write seam instead of the recall
seam. Screening on read means the payload gets injected first and defended
second, every time. Screening on write means it never becomes memory at all.

## Keyword filtering is not a security boundary

The obvious implementation is a blocklist. It is also the obvious target, and the
ways it fails are now well documented.

Write the instruction in Spanish and your English list never sees it. Scramble
the letters and the dangerous words are not in the input as text, but a subword
tokenizer reassembles them from fragments, so the encoder sees an instruction
your grep does not. Encode it and it is not there either. And split a tag with
invisible characters so no substring matches while the renderer reassembles a live
element.

The lesson from all of these is uncomfortable and specific: **a better grep is
not the fix**, because the mismatch is between two different granularities. Your
matcher reads bytes, the model reads tokens. Those are different views, and
anything that only looks at bytes is playing a different game from the thing it
is trying to constrain.

So the mechanical layer is built to attack the byte-level tricks properly. It
strips invisible characters before anything else looks at the text, which closes
the zero-width-split family outright. It covers the same six
instruction-override intents across five languages from a single table, because a
translation gap is a free bypass and one data structure cannot drift the way five
ad-hoc lists will. It has a typoglycemia tier that matches a scrambled word by
its first character, last character, and sorted middle, which catches
`1gnore prev10us` without matching every anagram that exists. And exact keywords
never trip that tier, so the bare word "system" stays prose instead of becoming a
false positive that trains people to route around the screen.

## Three states, and the middle one is the product

A screen with two outcomes, allow and refuse, is a screen that gets turned off.

Refuse everything and operators route around it, which is a worse outcome than
having no screen, because now the control exists on paper and nobody uses it.
Allow everything is the attack. So there are three verdicts, and the middle one
does the real work.

`Reject` returns a 400 and writes nothing. `Quarantine` **stores the record,
flagged it**, excluded from retrieval, waiting for a human. `Clean` proceeds.

Quarantine is what makes the thing deployable. It preserves the evidence, which
matters for an attack you want to investigate. It contains the payload, which
matters for the agent that would otherwise read it back forever. And it lets a
reviewer override, which is the one thing neither of the other two states can do.

## When the model is unavailable, the screen still runs

Layer one is deterministic, costs nothing, and cannot fail in this process. Layer
two is a local classifier, and it can fail: the artifact might be missing, the
feature might not be compiled, inference might error.

We made layer two fail **open**, and that is a genuinely uncomfortable decision to
write down, so here is the reasoning. If an inference error took the screen down,
we would have converted an availability problem into an availability problem with
worse properties and no security benefit, because the mechanical layer was
sitting right there working for free. Failing closed on a model-loading problem
would make the system less safe and less available at the same time.

The thing that makes that trade acceptable is that it is visible. The health
endpoint reports the classifier posture as a tri-state: `on` when it is loaded and
scoring, `off` when someone explicitly opted out, `absent` when nothing resolved
or the feature is not built. It also reports the policy, which includes an `allow`
value that disables screening entirely. So a configuration that turns screening
down shows up on the health surface instead of being a silent change in posture,
and an operator can confirm the classifier is genuinely running rather than
assuming it.

If you cannot tell which posture you are in, fail-open is not a safety property.
It is a surprise.

## Two seams, doing different jobs

Screening decides what gets **stored**. `sanitize_read` decides what gets
**emitted**, and it runs unconditionally over every text field leaving the
process.

Keeping those separate is not redundancy, it is architecture. Storing verbatim
means a later change to the screening rules does not invalidate every approval
digest already in the system, which is a real problem we would otherwise have
had. And because the read seam is independent, a record that was legitimately
stored can still be shaped safely on the way out.

The read seam drops a closed set of hostile element names and hostile URL
schemes, after the markdown strip, and there is a pinned test asserting that
benign content comes through byte-identical so the filter cannot quietly become
the thing that mangles legitimate content.

## What this does not do

The classifier is a tripwire, not a control. Its miss rate on attack shapes we
have never seen is unmeasured, and it cannot be measured honestly without a
labelled adversarial corpus we do not have. It narrows the surface. It does not
close it, and anyone who tells you a classifier closed injection is selling
either a benchmark or a feeling.

Layer two is not even in the default build. Default builds are mechanical only.
Any statement about classifier coverage describes a feature-gated build.

The language coverage is five languages and six intents, which is exactly what we
thought of. And the encoding tier decodes one level only, on purpose, to bound the
work. That is a missed-detection surface we accepted to avoid giving an attacker
a decoder to point at unbounded input. It is one of the few places in this system
where we chose availability over completeness, deliberately, and it is written
down rather than discovered.

Full mechanism write-up, including why verdicts may only move in one direction,
is in the research note on
[the two-layer injection screen](./../research/17-injection-screen.md).