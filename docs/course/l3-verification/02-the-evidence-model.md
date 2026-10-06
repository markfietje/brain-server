# Lesson 2: The evidence model

**Level:** L3 · **Time:** about 20 minutes · **One fresh instance, three checks**

## The two layers of evidence

Most systems offer you one layer: a log. Logs say what the software says
it did. This system's evidence has a second layer, and understanding the
split is the whole lesson.

**Layer one: the hash-chained audit trail.** Every consequential action,
approvals, rejections, exports, purges, key rotations, writes one row, and
each row carries a fingerprint of the previous row. The chain makes
after-the-fact editing loud: change one row and every later fingerprint
stops matching. You check the whole chain with one call:

```bash
curl -s localhost:8765/audit/verify
# {"ok":true}
```

**Layer two: the off-host anchor.** Chains have a blind spot. An attacker
with full database access could, in principle, rewrite rows AND recompute a
consistent chain, and a chain-only verifier would pass. The anchor closes
this: it fingerprints the state itself, the chain head PLUS a census of the
knowledge content PLUS row counts, and you record that line OFF the host.
Later, verify:

```bash
brain anchor
# record the printed line somewhere that is not this machine

brain anchor --verify "<that line>"
```

Any state change since trips it, legitimate ones too, and the audit chain
then explains which changes were legitimate. What it uniquely catches is a
moved knowledge census on a chain that still verifies: tampering behind the
chain. This exact attack class was found by an audit of this system, and
the anchor is the control built in response. That origin story is
checkable: it is in the audit history with dates.

## Why "who did it" holds up

Chains prove events. This system also binds identity into the events:
approvals carry the approver and the digest of the exact approved bytes.
Recall from Level 1 that an approval without a matching digest is refused.
From your side that means "reviewer X approved content Y at time Z" is a
chain-verifiable statement, not an HR memo.

## Provenance marks

Exported artifacts, remedy drafts, decision packets, knowledge-base
manifests, carry signed provenance marks: machine-generated or human, who
signed, when. Where a disclosure duty needs "was this AI-generated", the
mark answers with a signature over the bytes, and the verification is a
command in the UMP surface. The marks never claim more than they do: the
signed statement is about generation and custody, not about truth.

## Run it

Fresh throwaway instance (the reproduce script's section zero is exactly
this, three lines):

```bash
DB=/tmp/brain-audit-$$.db; PORT=18799
BRAIN_DB_PATH=$DB BIND_PORT=$PORT ./target/release/brain-server &
sleep 2
B=localhost:$PORT

curl -s $B/audit/verify          # chain ok on a fresh instance
curl -s "$B/audit?limit=3"       # rows carry the back-reference field
brain anchor                     # record this line off-host
# ... make one change, ingest anything ...
brain anchor --verify "<recorded>"   # trips, and explains via the chain
```

The last step is the one to sit with. The verify TRIPS on your own
legitimate write, and then the audit trail tells you what the change was.
That is the intended experience: the anchor notices everything, the chain
accounts for everything, together they leave nothing unexplained.

## What to record for your evidence pack

- The `/audit/verify` output and your instance version, together.
- One anchor line and one anchor verify trip, with the explanation found.
- The date. Always the date.

## Next

[Lesson 3: The reproduce script, walked](03-run-the-reproduce-script.md)
