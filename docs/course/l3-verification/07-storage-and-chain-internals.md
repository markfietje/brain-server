# Lesson 7: Storage and chain, down to the bytes

**Level:** L3 deep dive · **Time:** about 30 minutes · **Read + one sqlite session**

## Questions people ask

**What exactly is on disk?** One SQLite database in WAL mode, ~75 tables,
with three retrieval index legs beside the rows: a vector index (vec0
tables, local static embeddings), FTS5 full-text, and the graph tables.
Newer schemas refuse to open on older binaries and vice versa (a
refuse-newer probe pins it), so version skew is a loud error, never
undefined behavior. WAL means readers never block the writer and the
backup path can checkpoint cleanly (that checkpoint is step one of both
the backup and the standby cycle, on purpose).

**How does the hash chain work, precisely?** Each audit row carries the
hash of the previous row, chained from the first. Verification walks the
whole chain (`/audit/verify`). The chain lives in the SAME database, which
is why the anchor (lesson 2) exists for the behind-the-chain class. There
is also a signing epoch: key rotations bump a generation, signatures from
the previous epoch verify against the previous key (one rotation deep),
and older epochs are marked forgeable rather than pretended strong.

**How do I check all this myself?**

```bash
sqlite3 /tmp/brain-x.db ".tables" | head          # the shape
sqlite3 /tmp/brain-x.db "SELECT COUNT(*) FROM audit_events;"
sqlite3 /tmp/brain-x.db ".schema audit_events" | head -12
brain anchor                                      # content census + chain head
```

Read-only curiosity is safe. WRITES outside the server are not: editing
rows by hand is exactly the tamper class the anchor catches, and the
Level 2 capstone has you do it deliberately once, to a throwaway.

## What to verify, and how to phrase it

For your evidence pack, the storage claims worth checking: the schema
version and the refuse-newer behavior (open a newer DB with an older
binary, record the refusal), the WAL files beside the main DB after
writes (`.db-wal`, `.db-shm`), and the pre-migration backup + marker the
server keeps beside the database, both created private (0600) since the
mode-law round. Each is a one-command check, and each has been at some
point a real finding in this system's own audit history, which is the
honest reason the checks exist.

## Exercise

1. Boot a throwaway, ingest a few rows, stop it cleanly. Inspect the
   files: main DB, WAL, the pre-migration artifacts, permissions.
2. Count audit events versus knowledge rows. Explain the ratio.
3. Delete ONE knowledge row with sqlite directly. Run `/audit/verify`
   (it may still pass, and now you know why), then `brain anchor
   --verify` against a pre-tamper anchor line (it trips). That gap between
   the two tools is the entire lesson.

## Next

[Lesson 8: The screen pipeline and the egress posture](08-screen-and-egress.md)
