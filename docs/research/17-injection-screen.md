# The Two-Layer Injection Screen: mechanical tiers, a local classifier, and honest degradation

**File:** `src/screen.rs` (two-layer screen, 1,819 lines) ·
`src/strip_invisible.rs` + `plugin/fixtures/invisible-classes.json` (the
canonical invisible set) · `src/handlers/gate.rs` (`sanitize_read`, the read seam)

## The problem

An agent memory store is a write surface an attacker can reach, and the payloads
that matter are the ones that persist. A prompt injection that convinces a model
to exfiltrate a key is bad. The same injection written into a memory store is
worse, because it is still there on every future turn, and because the operator
who reads it later has no way to know it was aimed at the model rather than at
them.

Filtering such content by keyword is the obvious approach and it fails in ways
that are now well documented. The attacker writes the instruction in another
language. Or scrambles the letters so the words are not present but a model's
tokenizer reassembles them anyway. Or encodes them. Or splits a dangerous tag so
that any single substring match fails while the renderer reassembles a live
element. Each of these defeats a matcher that only looks at bytes.

The harder design problem is not catching attacks. It is that **a screen which
fails must fail in a direction you chose on purpose**, and that choice has to be
written down. A screen that silently stops scoring is indistinguishable from a
screen that has decided everything is fine.

## The references

- **Prompt injection as a durable property of the store, not the turn.** The
  agentic-security literature treats injection primarily as a per-request
  hazard. Memory changes the shape: the payload is *replayed* on every future
  retrieval, so a single successful write becomes a persistent attack. This is
  the reason the screen sits at the write seam rather than at the recall seam,
  and why the read seam carries a second, independent transform.
- **Unicode confusables and invisible formatting.** Unicode Technical Standard
  #39 addresses confusable characters; the `Cf` general category covers format
  characters such as zero-width joiners and bidirectional overrides. Trojan
  Source (CVE-2021-42574) demonstrated that a source file's *rendered* form can
  differ from its executed form, which is the same class of confusion applied to
  text a human reviews and a model reads.
- **Typoglycemia and tokenization.** Obfuscated spellings defeat naive
  substring matching because the dangerous terms are not present in the input as
  contiguous text. The relevant property is that a subword tokenizer will
  reassemble a scrambled word from fragments, so the *encoder* sees an
  instruction the *grep* does not. The correct defense is therefore not a better
  grep but a tier that reasons at the same granularity the model will.
- **The phrase "defense in depth" with teeth.** The real requirement is that each
  layer fails independently, which is only true if the layers are implemented in
  different ways. Two substring passes over the same string are one layer wearing
  two hats.

## The deterministic way brain-server implements it

**Two layers, and the second is opt-in by absence.** Layer one is mechanical and
always present: an invisible-character strip, a pattern check, and a phrase
blocklist. Layer two is a local ONNX classifier over the `injection-classifier`
feature. When the feature is not built, layer two short-circuits to `Clean` and
the default build is byte-identical to a build that never had it. That property
is asserted, not hoped for.

**The verdict set has three states, and the middle one is the interesting one.**
`Reject` returns HTTP 400 and writes nothing. `Quarantine` stores the record
*flagged*, excluded from retrieval until a human reviews it. `Clean` proceeds.
Quarantine is the state that makes the system usable: refusing every suspicious
write trains people to route around the screen, and accepting them is the attack.
Storing-and-flagging keeps the evidence and contains it.

**Layer one is deliberately broader than English.** The phrase blocklist covers
the same six instruction-override intents across Spanish, German, French, Dutch,
and Filipino, driven from one table so the languages cannot drift apart. On top
of that sits a typoglycemia tier: first character plus last character plus
sorted middle, which matches a scrambled word without matching every anagram,
with a length floor. Exact keywords never trip it, so the bare word "system"
stays prose. A bounded encoding tier inspects base64 and hex runs of at least 24
characters, the first eight runs only, decoding at most 4 KiB at exactly one
level. The bound matters more than the detection: an unbounded decoder is itself
a denial-of-service surface.

**Verdicts can only move in one direction.** The classifier runs on the *stripped*
text rather than the raw input, which closes a real disagreement: a payload
split by a zero-width joiner can evade a line-anchored pattern matcher, so raw
and stripped inputs can yield different verdicts for the same logical string.
After the strip, a verdict may move `Clean` to `Quarantine` or `Reject` and never
the reverse. The design point is that a normalization pass may only ever make the
system more suspicious, never less.

**The classifier is budgeted, because inference is a shared resource.**
`MAX_SCORED_SENTENCES` is 64 and `MAX_SCORED_CHARS` is 16000, so a one-megabyte
body containing a million sentence fragments cannot turn a write into a long
serialized inference stall. The first 64 sentences of the first 16000 characters
are scored. Input beyond the budget is unscored, which is a documented
degradation of a tripwire tier rather than a claim about the unscored remainder.

**A tripwire degrades open, deliberately.** If the classifier is unavailable or
an inference fails, the score contribution is zero and the verdict falls back to
the mechanical layer. This is the uncomfortable choice and it is the correct one
here. Layer one is deterministic, costs nothing, and cannot fail in this
process, so failing *closed* on a model-loading problem would convert an
availability problem into an availability problem with worse properties and no
security benefit. The posture is named in the module docs rather than left for a
reader to infer from the code path.

That posture is surfaced rather than assumed. `GET /health` echoes
`injection_classifier` as a tri-state (`on` when loaded and scoring, `off` for an
explicit `BRAIN_INJECTION_CLASSIFIER=off` opt-out, `absent` when no artifact
resolved or the feature is not compiled), beside a
`injection_classifier_loaded` boolean. It also echoes `injection_policy`, which
matters because the policy includes `allow`, which disables the screen entirely.
A configuration that turns screening off is therefore visible on the health
surface instead of being a silent change in posture, and an operator can confirm
the opt-in model is genuinely active rather than assuming it. This is what makes
the fail-open trade legible: without the echo, a healthy service screening one
layer deep would be indistinguishable from one that is not.

**Write-time screen, read-time seam, and they are not the same function.**
Screening decides whether content is *stored*. `sanitize_read` decides what is
*emitted*, and it is unconditional over every text field on the way out. Storing
verbatim and sanitizing at read is what keeps a later change to the screen from
invalidating approval digests, and it is why a write-time verdict and a
read-time appearance can legitimately differ. The one thing that must never
happen is content that is screened on write and then reassembled into markup on
read, so the read seam also drops a closed set of hostile element names and
hostile URL schemes, after the markdown strip, with the surviving benign cases
pinned byte-identical.

## Measured ceiling

- **The classifier is a tripwire, not a control.** Its false-negative rate on
  unseen attack shapes is unmeasured and cannot be measured without a labelled
  adversarial corpus we do not have. It narrows the surface; it does not close it.
- **Layer two is absent from default builds.** Default builds are mechanical
  only. Any claim about classifier coverage describes a feature-gated build.
- **The phrase blocklist is finite and its maintainers are its limit.** Five
  languages and six intents cover what we thought of. Novel phrasing in an
  uncovered language is out of scope by construction.
- **One decode level is a real ceiling.** Double-encoded payloads are not
  decoded twice, by design, to bound the work. This is a missed-detection surface
  accepted for a denial-of-service bound, and it is one of only a few places in
  the system where we chose availability over completeness on purpose.
- **Fail-open on inference error is a real trade.** It is correct given that
  layer one is free and deterministic, but it means an operator can observe a
  healthy service that is quietly screening one layer deep. The `/health` posture
  echo is the mitigation, which makes that echo load-bearing rather than
  decorative. It is also the honest answer to "how do I know what posture am I
  in": read the health surface, do not infer it from the fact that the service is
  up.
- **Budget truncation is unscored input.** Bytes past the 16000-character limit
  are not classified. The budget prevents a denial-of-service stall, and it also
  means an attacker can place a payload past the limit. The mechanical layer
  still reads the whole string, which bounds this but does not eliminate it.