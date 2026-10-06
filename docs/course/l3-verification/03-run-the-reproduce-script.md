# Lesson 3: The reproduce script, walked

**Level:** L3 · **Time:** about 30 minutes including the run · **You need the binary, jq, curl**

## What you are about to do

Run the system's own verification walkthrough against a fresh throwaway
instance, and understand each section well enough to defend trusting it.
The script lives at [trust/reproduce.md](../../trust/reproduce.md) and
takes about three minutes. This lesson walks its sections in order, with
what each proves and what it deliberately does not.

## Section zero: the fresh instance

The script starts by launching a server against a throwaway database path
on a scratch port. This is not a formality, it is the methodology: every
claim is checked against an instance YOU started from the artifact YOU
were given, with no production data and no curated state. If a vendor ever
offers to run the demo for you, that offer is the finding.

## Section one: the tamper-evident chain

`/audit/verify` returns ok, and sample rows carry the back-reference
field. Proven: the chain exists and is intact from the first row. Not
proven: that the chain was never behind-the-chain attacked. That is the
anchor's job, lesson 2, and the script's job is the faster claim.

## Section two: the human gate

This is the flagship check, and the shape matters as much as the result:

- A proposal is filed. It returns a proposal id, NOT a knowledge row.
- The pending queue shows it, with its content digest.
- Approval WITHOUT the digest is refused (digest required).
- Approval WITH the digest promotes it, returning the chunk id.
- Only then does search find it.

Proven: nothing auto-promotes, approvals bind to exact bytes, and the
refusal is enforced by the server, not by a UI convention. While you are
here, note the create-loop honesty: the machinery that would let the
system author and promote its own knowledge ships deliberately disabled,
and the docs say so in those words. A system that hid that machinery's
existence would be harder to trust than one that ships it switched off.

## Section three: DSAR with a certificate

Export a subject's data, read the certificate, check it chain-verifies,
see the tombstone after a purge. Proven: the rights path produces its own
verifiable evidence. Lesson 4 does this one in depth.

## Section four: identity surfaces

JWKS is served, the protocol conformance reads "UMP 1.0 / L3" (protocol
level, not course level, as lesson 1 warned), a capability token can be
minted, and a read-only token attempting a write gets refused. Proven: the
identity model is real, capability-scoped, and enforced at the API.

## What the script does not claim

Reading the script critically means noticing its scope. It verifies
behavior of a fresh instance. It does not verify your deployment, your
network, your processes, or your people. When you package its output as
evidence, package it AS what it is: a control test of the artifact. The
gap between artifact and deployment is what your site visit and your
configuration review cover, and the system helps you there too: the
configuration is enumerable, and the audit trail shows how the deployment
actually behaved, which is usually more interesting than how it was
configured to behave.

## Run it now

Open [trust/reproduce.md](../../trust/reproduce.md), follow it top to
bottom against a scratch instance, and keep the terminal output. Every
command is safe as written, and the lesson's exercise is the run itself.

One addition worth making: at the end, run `brain status` and record the
version next to your outputs. An evidence pack without a version stamp is
a pack that expires silently.

## If a step fails

A failure during reproduction is not an emergency, it is the single most
useful datum the exercise can produce. Write down the exact step, the
exact output, and the version, and report it. The maintainers' disposition
to bug reports about verification steps is on record in the changelog, and
it is a reasonable thing to ask about in any assessment: what happened the
last time a check failed.

## Next

[Lesson 4: Rights and records, verified](04-rights-and-records.md)
