# Lesson 6: People and their data

**Level:** L1 (101) · **Time:** about 25 minutes · **Console + commands on a throwaway copy**

## What you will do

Learn the everyday side of data rights: a person asks what you hold about
them, or asks you to delete it, and you know the path, the safeguards, and
what paperwork comes out the other end. This is daily work, not legal
theory. (The machinery that makes it provable to an outside auditor is
Level 3's job, and it says so in its own lesson.)

## First, the honest scope

The system finds a person's data by the label it was stored under. Every
memory row carries an owner stamp. When a customer's identity is the owner
label, the system finds those rows precisely. Content that happens to
mention a person without being stored under their label is swept too, by
text search, which is best-effort by nature. And there is one class the
system is plain about: erasure removes it from the database, but backups
taken before the erasure still contain the old bytes. That is true of every
backup in the world, and pretending otherwise would be worse. The honest
statement is: live data erased, backup copies age out on the retention
schedule.

## The request: "what do you have on me"

This is called an export, or DSAR if you meet the term (data subject access
request). The everyday shape:

1. You identify the person, by their owner label. The system uses a
   shorthand of that label, a fingerprint, in prompts, so you can confirm
   you have the right person without pasting their details around.
2. You run the export. The system gathers every row owned by that person.
3. It hands over the export and writes a certificate. The certificate is
   chained like everything else, so "we gave Alice her data on the 12th" is
   a provable fact, not a promise.

There is a preview mode, a dry run, that answers "how many rows, what
kinds" without touching anything. Use it first, always. It is free, it is
read-only, and it turns "I think it worked" into "here is the footprint".

## The request: "delete it all"

Deletion gets more ceremony, because it should. On a throwaway copy:

```bash
B=localhost:8765

# 1. Preview first. Zero rows touched, zero written.
curl -s -X POST $B/dsar -H 'content-type: application/json' \
  -d '{"subject":"alice","dry_run":true}' | python3 -m json.tool | head -15

# 2. The real export+purge. Read the certificate id it returns.
#    (For the CLI form, `brain client dsar` makes the action explicit and
#    required: export, purge, or both. On the API the call above carries it.)
curl -s -X POST $B/dsar -H 'content-type: application/json' \
  -d '{"owner":"alice"}' | python3 -m json.tool | head -15

# 3. The certificate exists, and it chain-verifies.
curl -s $B/dsar | python3 -m json.tool | head -20

# 4. The rows are gone from recall.
curl -s "$B/search?q=<something+alice+owned>" | head -5
```

What the purge leaves behind is a tombstone: a marker saying content with
this fingerprint was deleted on this date, by this decision. It keeps no
content. The marker exists so that a later import or sync cannot resurrect
the deleted data by accident, and so the deletion itself stays provable.

There is also a stronger, physical cleanup the operator can run after a
purge, called shred, which rewrites the database file so the old bytes are
not merely marked free but gone. That is a Level 2 verb, done in quiet
moments, with its own record. You just need to know it exists, so that when
an auditor asks "and after deletion, where are the bytes", the answer has a
second half.

## The clock

Deletion requests usually come with a legal deadline. The system puts the
deadline on the ledger when the request lands, created date plus the window
your organization configured, thirty days by default. The request list shows
it. The point of the clock is not to panic you, it is so that nobody has to
maintain a separate spreadsheet of deadlines, which is exactly the kind of
side-record that drifts and then fails an audit.

## Holds: when deletion is refused, correctly

If litigation is expected for this person, a legal hold can be placed. A
hold does not block the request from being received, it blocks the purge
from executing while the hold stands. If you hit this, it is working as
intended, and the path is: escalate to whoever owns legal in your
organization. The system holds the line, humans decide the case.

## Fixing data, not just deleting it

"Please correct my address" is usually served by the normal memory
operations: the wrong row is superseded by a right one (lesson 7), the
record shows the correction. Data correction is not a special ceremony here,
it is the ordinary, audited edit path, which is rather the point.

## Exercise

On a throwaway copy, with two ingested rows owned by a test subject:

1. Ingest two rows owned by "alice", approve them, confirm they recall.
2. Run the dry run. Read the footprint. Confirm the row counts match what
   you expect.
3. Run the export-only path. Read what comes back.
4. Run the purge. Confirm the search now comes back empty, and find the
   tombstone list in the Data panel.
5. Find the deadline field on the request in the ledger. Where does the
   thirty days (or your configured window) show up?

## What you learned

- Preview first, always. Dry run answers "what would happen" for free.
- Export produces data plus a chain-verified certificate.
- Purge leaves tombstones, blocks resurrection, and backup copies are an
  honestly-named ceiling, not a secret.
- Legal holds stop the purge, not the request. Escalate, humans decide.
- Corrections ride the ordinary, audited supersession path.

## Next

[Lesson 7: Keeping the memory healthy](07-keeping-it-healthy.md)
