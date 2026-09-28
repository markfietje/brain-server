# The clean cycle: running brain-server as an appliance

**Audience:** the operator. This is the runbook for a deployment that is
**turned off at the end of the day and back on in the morning** — which is how a
government office actually runs.

The promise this document backs is narrow and checkable: **the data survives
being turned off, and you can prove it.** It is not a high-availability
architecture. A server that is off for fourteen hours a day has no 24×7
availability to protect, and building for one would be solving a problem you do
not have.

---

## The evening: stop it properly

```sh
sudo systemctl stop brain-server
```

That sends `SIGTERM`. The server drains, closes the pool, and runs
`PRAGMA wal_checkpoint(TRUNCATE)`. Then it exits.

**After a clean stop, `systemctl stop` writes a stamp** to
`/var/lib/brain-server/.shutdown-clean` (via `ExecStopPost`). The stamp is how
the morning check distinguishes a *stopped* server from a *killed* one.

**Do not** use `pkill -f brain.db`. `BRAIN_DB_PATH` lives in the process
**environment**, not in its **arguments**, so that pattern matches **nothing** —
and a stop script that reports success while the process is still running is
worse than no stop at all. Match the binary, or let systemd do it.

## The morning: check it came back correct

```sh
/usr/local/bin/brain-clean-cycle-check
```

It prints `PASS` or `FAIL` and answers three things:

| Check | What a failure means |
|---|---|
| `PRAGMA integrity_check` | the store is structurally damaged — **stop and investigate** |
| `PRAGMA journal_mode` | the volume silently cannot do WAL (see below) |
| the clean-shutdown stamp | the last process was **killed**, not stopped |

It exits non-zero on any failure, so it can gate a start script or a health
check.

### If the clean-shutdown stamp is missing

Nothing is lost. SQLite replays an un-checkpointed WAL on the next open — the
server's own shutdown path says so at `src/main.rs:89-90` ("the OS will replay
WAL on next open anyway"). What you lose is **speed**: the first queries after
an unclean stop are slower while the WAL folds in, and the `-wal` file is larger
until it drains.

What you should do is find out **what killed it** — a power cut, an OOM, a
`kill -9`, or an operator with `Control-C` to spare.

---

## The storage rules

### The data volume MUST be a local block filesystem (ext4 or xfs)

This is not a preference. SQLite's own documentation:

> "POSIX advisory locking is known to be buggy or even unimplemented on many NFS
> implementations… **Your best defense is to not use SQLite for files on a network
> filesystem.**"
> — [sqlite.org/lockingv3.html §6.0](https://www.sqlite.org/lockingv3.html), fetched 2026-09-28

WAL needs the mmap'd wal-index **in the same directory as the database** and
requires all processes to be **on the same host**. An NFS/SMB/EFS-backed volume
breaks both, and the symptom is `database is locked` — or worse, corruption.

**The server now refuses to start on such a volume** and names the cause
(`src/migration.rs`). This matters because `PRAGMA journal_mode=WAL` does **not**
fail when it cannot be applied — SQLite silently leaves the prior mode
([wal.html §3](https://www.sqlite.org/wal.html)) — so without the readback a
network volume would boot, run, and quietly downgrade the durability that
`brain standby` and `brain shred` are built around.

`memory` mode is allowed: it is a deliberate in-memory test store with no
filesystem and nothing to downgrade.

### Never copy `brain.db` on its own

> "If a database file is separated from its WAL file, then transactions that
> were previously committed to the database might be lost, or the database file
> might become corrupted."
> — [sqlite.org/wal.html §4](https://www.sqlite.org/wal.html), fetched 2026-09-28

If you copy the store, copy the **whole directory**: `brain.db`, `brain.db-wal`
and `brain.db-shm`, and **quiesce first** (`systemctl stop`). A copy taken while
the service is running is not consistent. This is also why `brain standby`
copies the base *after* a passive checkpoint and the WAL frames *after* that —
the ordering is load-bearing, not incidental.

---

## Backups, and the one thing you must sign for

Two independent things, because they cover different failures.

**`brain standby ship --to <dir>`** — application-level, encrypted, signed
manifest, and the only thing that covers **fire, flood and theft**. It is the
off-site copy. The CronJob/scheduled form runs it; the one-shot verb runs
exactly one cycle and exits with its status, which is what a scheduler needs.

### The off-site copy needs a SIGNED APPROVAL — this is not optional

The **Data Privacy Act of 2012 (RA 10173)**, verified from the primary text on
2026-09-28:

- **§3(l)(3)** defines *sensitive personal information* to include
  **"social security numbers, previous or current health records, licenses… and
  tax returns"** — precisely what a city hall holds.
- **§23(b)** — sensitive PI **"may not be transported or accessed from a
  location off government property"** without the **agency head's approval**;
  off-site access is capped at **1,000 records**, using **"the most secure
  encryption standard recognized by the Commission."**

**So an off-site vault of citizen records is a documented, signed exception, not
a configuration choice.** Record the approval with the vault. If the vault is on
premises, §23(b) is not engaged — which is the simplest way to stay inside it.

**§21 is transfer-with-accountability, not localization**: the controller stays
liable for data *"transferred to a third party… whether domestically or
internationally."* There is **no data-localization mandate** in RA 10173.

*These are quotations of what the instrument says, not a compliance
conclusion. A Philippine counsel confirms scope.*

---

## Re-measuring the stop budget

`TimeoutStopSec=30` in the unit is **measured, not guessed**:

| | measured (2026-09-28, physical Ubuntu host) |
|---|---|
| total `SIGTERM` → exit | **31 ms** |
| `PRAGMA wal_checkpoint(TRUNCATE)`, 14 MB store, 53 KB WAL | **0.2 ms** |
| cold boot → serving | **349 ms** |

30 s is ~1000× the measured shutdown. **The variable term is WAL size at
shutdown, not database size** — a write burst leaves a larger `-wal` and a larger
checkpoint. Re-measure after the store grows materially:

```sh
# quiesce, then time the checkpoint against a COPY — never the live store
sudo systemctl stop brain-server
cp -a /var/lib/brain-server /tmp/ckpt-bench
sqlite3 /tmp/ckpt-bench/brain.db 'PRAGMA wal_checkpoint(TRUNCATE);'
rm -rf /tmp/ckpt-bench
```

Then update `TimeoutStopSec` in the unit and note the new figure here.

> A note on how these numbers were first recorded: an earlier draft of this
> document reported a 12.1 s stop and a 1,056 ms boot. **Both were artifacts of
> the measuring scripts** — a fixed `sleep 12` and a `sleep 1` poll loop. A
> measurement taken with a coarse instrument is a guess with a decimal point.

---

## What this deployment does not do

- **No high availability.** One active node. Losing it means a restore.
- **No Kubernetes.** See `deployment-reference-architecture.md` for the shape a
  larger deployment takes, and for what is deliberately not built.
- **No shared database.** The database is on local disk. A NAS is fine for
  opaque backup artifacts and **never** for `brain.db`.
- **No compliance claim.** This runbook states what the code does and what the
  statute says. Whether a given deployment satisfies any of it is a
  determination for a qualified assessor.
