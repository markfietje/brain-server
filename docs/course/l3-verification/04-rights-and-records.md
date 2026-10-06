# Lesson 4: Rights and records, verified

**Level:** L3 · **Time:** about 25 minutes · **Commands against a fresh instance**

## What you are verifying

The rights machinery has two halves: what a person can get (access,
correction, erasure) and what the organization can show (certificates,
registries, deadlines). Both halves are checkable here, and the checks are
the claims. Level 1 taught the everyday operation. This lesson verifies
the machinery behind it.

## The dry run proves nothing happened

```bash
curl -s -X POST localhost:8765/dsar -H 'content-type: application/json' \
  -d '{"subject":"alice","dry_run":true}'
```

The response is a footprint: counts by table, zero rows touched. For your
evidence pack, this is the control test that the real run can be compared
against. It also demonstrates a posture: the system would rather show you
what it is about to do than narrate afterward.

## The certificate is chain evidence, not a PDF

Run the real export and purge (the reproduce script, section three), then:

```bash
curl -s localhost:8765/dsar/<id>/certificate    # chain_verifies: true
curl -s localhost:8765/tombstones               # rows: hash + date, no content
```

Three things to check and record. The certificate carries `chain_verifies`.
The tombstone carries a content fingerprint and a date, and NO content.
And the erasure reached the PROPOSALS behind the memories, not just the
memories, which was a real finding in this system's own history (approved
proposals used to survive the purge of the memories they created, which
made certificates overstate erasure; it was found by an audit and fixed
with a schema change, on the record). You do not need to trust that story:
purge a subject whose memory came from a proposal and check the proposals
list.

## The deadline is on the ledger

```bash
curl -s localhost:8765/dsar    # request rows carry created_at + deadline
```

The window is created-date plus the configured number of days, thirty by
default, and it is ON the request row where you can see it. For an
assessor this is the difference between "we meet SLAs" (assertion) and
"every request row carries its own deadline and a completed-at" (query).

## The residue ceiling, stated precisely

Erasure removes rows from the live database. It does NOT reach into backup
files taken before the erasure, standby follower chunks, or the physics of
SSD wear-leveling. Beyond the logical purge there is an operator-run
physical cleanup (shred) for the live file, with its own audit row, and
its own printed list of what it does not touch. The claim to verify is
that the system SAYS this, in its output and its docs, every time.
Ceilings that only appear in an auditor's report are marketing. Ceilings
that print at runtime are engineering.

## Registries

Two registers ride the same audit chain. The RoPA (records of processing)
is proposable and readable from the CLI (`brain ropa list`). The Article
30 register is served from the system surface. Both are rows with history,
not documents with dates in their filenames, which is the checkable
difference.

## Holds block the purge, visibly

Place a hold (Level 2 lesson 3), attempt the purge, observe the refusal,
lift the hold via the API, and observe the purge proceed. For your pack:
the refusal itself, with the hold id in it. A legal hold that lives only
in a lawyer's email folder is not a control. This one is a row with a
reason field and an audit trail.

## The one honest limit about finding people's data

Locate is by owner label, plus a best-effort text sweep for mentions. Data
stored under a label that does not match the subject will be missed by the
precise path and may or may not be caught by the sweep. This is why the
ingest-side owner stamping matters, and why a review process that catches
mislabeled ownership at capture time is part of the rights posture, not
just hygiene. Ask how captures get their owner labels. That question is
the assessment.

## Exercise

On a fresh instance: run the dry run, the real both-action, read the
certificate, count the tombstones, place a hold on a second subject and
watch the purge refuse, lift it, purge. Record every output with the
version. That folder is the rights section of your evidence pack.

## Next

[Lesson 5: The human-control claims](05-the-human-control-claims.md)
