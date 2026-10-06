# Lesson 4: Getting memory in, from everywhere

**Level:** L2 · **Time:** about 25 minutes · **Commands against a throwaway instance**

## What you will do

Learn every front door for knowledge, and what each door does to what comes
through it. The system's screening law is simple: everything untrusted is
screened at WRITE time, and everything served is shaped at READ time. Every
path below feeds that same law.

## Your own notes: directories

```bash
brain ingest-dir ~/notes/health --dry-run    # ALWAYS dry-run first
brain ingest-dir ~/notes/health              # then for real
brain ingest-dir ~/notes/v2 -r               # -r/--replace: reingest over old
brain reconcile ~/notes --dry-run            # find sources deleted on disk
brain source-delete 77 --yes                 # retire a source deliberately
```

The dry-run habit is the entire skill. It prints what would land, you read
it, then you commit. `reconcile` sweeps the other direction: files you
deleted on disk that still have memory rows.

## Structured knowledge: procedures

```bash
brain procedure "Refund escalation" \
  --step "Check entitlement: query the order, not the customer's claim" \
  --step "If over limit: escalate with the run id, never a screenshot" \
  --domain ops
```

A procedure is a root plus ordered steps, ingested as one transaction.
Procedures are the antidote to knowledge that lives in one person's head:
the runbook the new hire follows is the same one the assistant cites.

## From another brain-server: parcels

Parcels move approved knowledge between servers, cryptographically signed.

```bash
brain parcel export --domain acme --since 2026-01-01 --out acme.ump
brain parcel import --file acme.ump --domain acme \
  --expected-signer did:key:z6Mk...        # REQUIRED, no default
brain parcel ledger                          # the crossing record
```

Two rules with teeth. Quarantined rows never leave in an export. And
imported rows land as PENDING PROPOSALS, never direct memory: even a
cryptographically verified import gets a human look on the way in. The
`--expected-signer` flag is mandatory because the server refuses without
it: an import that does not name whose signature it expects is an import
that cannot detect the wrong signer.

## From your tools: connectors

```bash
brain connect github --app-id 1 --install-id 2 \
  --key-file ~/keys/app.pem --repo ourco/ops --repo ourco/infra
brain sync github
brain connector-status
```

The GitHub connector (App auth) brings issues and discussions in as stamped
source rows. CRM case connectors exist behind a feature gate. What
connectors are NOT: a live sync of everything forever. They are reconciled
snapshots with honest stamping, and the connectors page says exactly what
each one claims and does not.

## From conversations: captures

Assistant and auto-capture writes arrive as proposals (on the review
posture) or screened direct writes (open posture). You met this from the
operator side in Level 1. Your job on this side is only to know which
posture is on, and to make sure captures carry their origin, owner versus
channel, so downstream labeling and exclusion work.

## The screen, one more time, because it is load-bearing

Every path above feeds the same injection screen. When it sees hidden
instructions, disguised scripts, invisible characters, encoded payloads, it
quarantines and tells you. Two operational notes:

- The screen has a second layer, an optional classifier model, that
  auto-loads when its artifacts are present and refuses to guess when they
  are not. `/health/db` tells you which posture you are in.
- Hostile-looking input in QUARANTINE is inert. It is not searchable, not
  served, and only a human moves it.

## Exercise

On a throwaway instance:

1. Make a directory with three text files, one containing a `<script>` tag.
   Dry-run the ingest. Read the report. Ingest for real. Find the third
   file in quarantine.
2. Create a procedure with three steps. Retrieve step two through search.
3. Export a parcel of your approved rows, then import it into a SECOND
   throwaway instance. Confirm everything landed as pending proposals, and
   that your approvals there are new decisions by you, not carried over.
4. Run `brain reconcile` on your notes dir after deleting one file on disk.

## What you learned

- Dry-run first is the whole skill for bulk ingest.
- Procedures are one-transaction runbooks.
- Parcels are signed, quarantined rows never export, and imports always
  land as proposals.
- Connectors are reconciled snapshots, honestly stamped.
- Every door feeds the same screen, and quarantine is inert storage.

## Next

[Lesson 5: Backups and standby](05-backups-and-standby.md)
