# Delete is a verb, not a promise

*2026-10-04. Our erasure story had three layers, and we only wrote down one of
them. A look at what "deleted" actually has to mean before you can promise it.*

Here is a question with an obvious answer until you actually go looking: you
delete a customer's record to satisfy an erasure request. Are you done?

No. You are done when four separate things are true, and they have almost
nothing to do with each other. The application forgot the row. The database file
no longer holds the bytes. The backup you shipped last week no longer holds them.
And the disk platter itself no longer holds them, which on the hardware most of
us actually run, is not something software can promise at all.

We shipped a purge years ago. It deletes rows, in a transaction, with an audit
row, and it is correct. It was also, on its own, a claim about bytes that the
code had no standing to make.

## SQLite does not delete by default

The thing that changed our minds is a single default. `PRAGMA secure_delete` is
**off** in SQLite. Which means when a row is deleted, the page it lived on goes
on the freelist with the old contents still inside it, waiting to be reused. It
will eventually be overwritten by whatever writes next. Eventually is the problem.

Then there is the write-ahead log. Our durability posture depends on it, it is
good, and it means that for a period of time the old row contents are sitting in
a log file on disk in plain form. `VACUUM` does not help, because `VACUUM` reads
the current database and writes a fresh one. It cannot remove bytes from a log
that already holds them.

So the honest erasure is an *ordering*, not a statement. This is the sequence our
`brain shred` command runs, and every step is there because the next one does not
cover it:

1. `secure_delete=ON`, and read the setting back to prove it took
2. `wal_checkpoint(TRUNCATE)`, because the rebuild cannot reach bytes the log holds
3. `VACUUM`, which rebuilds into a fresh file and discards the freelist
4. a second `TRUNCATE`, for pages the rebuild itself released
5. `integrity_check`
6. one hash-chained `forget` row, so the erasure is evidence and not just a side effect

The receipt prints pages before and after, freelist pages after asserted as zero,
the readback, and the audit row id. An operator gets numbers. "Done" is not a
receipt.

## And then there is the flash problem

We can overwrite a file. On a spinning disk, on most SSDs, we cannot reliably
overwrite the *physical* state, because the flash translation layer decides
which block your write lands on, and wear-levelling means the old cell may never
be addressed again. Your `secure_delete` did exactly what it said and the bytes
may still be on the NAND, one indirection away from anything you can reach.

So we do not claim to destroy data on flash media, and the command prints that
ceiling every time it runs rather than leaving it in a wiki page somewhere. Same
for the copies: filesystem duplicates, `.bak` files, and the encrypted chunks on
a standby follower are each shredded or destroyed where they live, not by
running a command against the primary.

## The backup you already sent

A backup taken before the purge still contains the record. This is the part that
made us build the standby cycle differently rather than just adding a command.

Our warm standby copies the write-ahead log to a follower, and the ordering there
is load-bearing in exactly the way the shred ordering is. Passive checkpoint,
then the base snapshot, then the log chunks copied **after** the base, because the
base writer truncates the log. Copy the chunks first and you replay pre-base
frames on restore and roll the whole thing backward. That is the kind of bug
that only appears during a failover, which is the worst time to discover it, so
it is commented in the source and pinned by a test.

The chunks are encrypted with the same path as everything else, Argon2id into
AES-256-GCM, so there is no unencrypted byte at rest on the follower. The
manifest is signed last. Everything else about the format is in the research
note, including why our KDF cost is about three and a half times the library's
own suggested default, which is a margin we chose for a secret whose realistic
exposure is a laptop in a shipping box rather than a credential-stuffing table.

## The anchor that cannot move itself

One more thing worth stealing, because it is small and it is clever.

We can print a fingerprint of current state: the head of the audit chain, a census
of the knowledge store, some row counts. `brain anchor --verify` recomputes and
diffs. If something changed, you know, and the chain tells you whether the change
was legitimate.

The good part is that `brain anchor` is read-only on purpose. It cannot write its
own audit row, because an audit row would move the chain head that the
fingerprint had just recorded. So the operator writes the number down *off the
machine*, and that piece of paper is the evidence.

It caught something real: a knowledge table edited while the audit chain still
verified perfectly clean. The chain was honest about the audit trail and silent
about the data. An in-tree check cannot see that, because it reads the same store
somebody else just edited. A number you wrote down last month can.

And the ceiling, stated plainly: the chain key lives on the same host as the
anchor, so this detects SQL-level tampering and application bugs. It does not
detect an attacker who already has root, because they can forge both. It is a
tripwire, not a wall.

## The part worth taking away

If your product promises deletion, the promise has at least four independent
layers and the one you implemented is the easiest. What took us the longest was
not learning that SQLite keeps freed pages around. It was accepting that "we
deleted it" and "we deleted the bytes" are different claims, and that the
difference is the whole product.

Full write-up, including the KDF parameter reasoning and the SQLite pragma
semantics, in the research note on
[durable local-first state](./../research/16-durable-local-first-state.md).