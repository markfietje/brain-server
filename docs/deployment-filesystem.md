# Storage, filesystem and deployment guide

**Audience:** the operator deploying brain-server. This is the reference for
*where the data lives* and *how each deployment shape is built*.

**Honesty posture.** Every claim here was verified against the source tree at
`76f7b22` (2026-09-28) or quoted from a primary source with the citation
inline. Where something is **reasoned rather than measured**, it says so. Where
a commonly-repeated piece of advice has **no primary source**, this document
says so rather than repeating it. If this document and the code disagree, the
code is right.

---

## 1. The recommended filesystem

### The short answer

**A local block filesystem — ext4 or XFS, on a local disk.** There is no
documented preference between the two, and this document does not invent one.

> **There is no primary source that recommends ext4 over XFS (or the reverse) for
> SQLite.** Neither filesystem's manual page mentions SQLite, and no SQLite
> documentation names either filesystem except to prohibit network ones. If you
> have a reason to prefer one — a filesystem your operations team already
> supports, a validated RAID controller, a support contract — use it.

**What is documented, and what you must not do:**

| | |
|---|---|
| **MUST** be a local filesystem | SQLite's own words: *"Your best defense is to not use SQLite for files on a network filesystem."* ([lockingv3.html §6.0](https://www.sqlite.org/lockingv3.html)) |
| **MUST NOT** be NFS / CIFS / SMB / 9p | *"POSIX advisory locking is known to be buggy or even unimplemented on many NFS implementations"* (same source) |
| **MUST NOT** be USB flash | *"USB flash memory sticks seem to be especially pernicious liars regarding sync requests… Pulling out the memory stick while the LED is still flashing will frequently result in database corruption."* ([howtocorrupt.html §3.1](https://www.sqlite.org/howtocorrupt.html)) |
| **SHOULD** be on its own partition or disk | Not for performance. So a filesystem-level `remount-ro` on a full or failed volume does not take the OS down with it. |

### Why network filesystems break it — the mechanism

Not "performance". Three documented requirements:

1. **POSIX advisory locks.** WAL needs the writer to exclude readers.
2. **A unified buffer cache** for memory-mapped I/O. *"Not all operating systems
   have a unified buffer cache. In some operating systems that claim to have a
   unified buffer cache, the implementation is buggy and can lead to corrupt
   databases."* ([mmap.html](https://www.sqlite.org/mmap.html))
3. **Shared memory in the same directory as the database.** The wal-index is an
   mmapped file; *"the only way we have found to guarantee that all processes
   accessing the same database file use the same shared memory is to create the
   shared memory by mmapping a file in the same directory as the database
   itself."* ([wal.html §7](https://www.sqlite.org/wal.html))

### The failure is silent, which is why the server now refuses

`PRAGMA journal_mode=WAL` **does not fail** when it cannot be applied — SQLite
returns the prior mode and the statement succeeds
([wal.html §3](https://www.sqlite.org/wal.html)). A volume that cannot do WAL
would therefore have booted, run, and quietly downgraded the durability that
`brain standby` and `brain shred` assume.

**The server now reads the mode back and refuses to start**, naming the cause
and the remedy (`src/migration.rs`). If you see:

```
journal mode is 'delete', not 'wal' — the data volume cannot do
write-ahead logging. Refusing to start…
```

**move the data directory to a local block filesystem.** That is the fix; there
is no override, by design.

---

## 2. Mount options

### The recommended fstab line (ext4)

```
# /var/lib/brain-server — local SSD/NVMe, ext4.
# The options below are DEFAULTS, written explicitly so a reader knows they
# were chosen rather than inherited. Do not add anything not listed.
UUID=<your-uuid>  /var/lib/brain-server  ext4  defaults,noatime,errors=remount-ro  0 2
```

### What each option is, and why

| Option | Verdict | Source |
|---|---|---|
| `defaults` | keep — `rw,async` | [mount(8)](https://man7.org/linux/man-pages/man8/mount.8.html) |
| `noatime` | keep | *"Do not update inode access times on this filesystem… This works for all inode types (directories too), so it implies nodiratime."* **Honest counterpoint:** `relatime` is already the kernel default since 2.6.30, so the gain for a single-file database is probably marginal. It is safe, not magic. |
| `errors=remount-ro` | keep | *"remount the file system read-only"* — the fail-closed choice. **Verify it took:** the default lives in the *superblock*, not fstab. `tune2fs -l <dev> \| grep -i errors` and keep the output. |
| `barrier` (=1) | **never `nobarrier`** | *"Write barriers enforce proper on-disk ordering of journal commits, making volatile disk write caches safe to use, at some performance penalty."* SQLite: disabling them means *"filesystem corruption can occur"* and *"there is nothing that SQLite can do to work around it."* |
| `data=ordered` | **never `writeback`** | `writeback` *"can allow old data to appear in files after a crash"* — the stale-read-after-crash class a tamper-evident audit chain exists to make impossible. `data=journal` is the documented slowest-but-safest option; whether it is worth its cost here is **unmeasured**. |
| `discard` | **leave off** | *"it is off by default until sufficient testing has been done"* (ext4). On XFS the man page says to use the `fstrim` timer instead. Continuous TRIM is wrong for a WAL that repeatedly rewrites the same blocks. |
| `sync` | **never** | *"In the case of media with a limited number of write cycles (e.g. some flash drives), sync may cause life-cycle shortening."* |
| `nodelalloc` | **no** | The ext4 man page documents the option and gives **no workload advice**. No kernel.org, Red Hat or SQLite source recommends it for databases. The folklore predates `data=ordered` + `auto_da_alloc`, which are ext4's own answers to that class. **Do not set it without a measured A/B.** |
| `commit=60` | **no** | Real, and **ext4-only** — `xfs(5)` has no such option. The man page documents `commit=nrsec` (default 5) but **no source ties it to database throughput**. Writing it on an XFS host is a category error. |
| `inode64` (XFS) | **no action** | Already the default on kernel ≥3.7. |
| `lazytime` | optional | *"significantly reduces writes to the inode table for workloads that perform frequent random writes to preallocated files"* — a good textual match for a checkpointing WAL. **Unmeasured for SQLite.** |

### Verify after mounting

```sh
findmnt -no SOURCE,FSTYPE,OPTIONS /var/lib/brain-server
sudo tune2fs -l "$(findmnt -no SOURCE /var/lib/brain-server | sed 's/\[.*//')" | grep -iE 'errors|features'
```

---

## 3. The tunings the server actually applies

Measured from the tree at `76f7b22` — not from a default.

| Setting | Value | Source | Note |
|---|---|---|---|
| `journal_mode` | `WAL` | `src/migration.rs:57` | Persistent: *"If a process sets WAL mode, then closes and reopens the database, the database will come back in WAL mode."* |
| **`synchronous`** | **`FULL`** | `SynchronousMode` `#[default]` | **The shipped default is the safe one.** In WAL, FULL is ACID. `BRAIN_SYNCHRONOUS=normal` opts into the faster posture. |
| `foreign_keys` | `ON` | per-connection pragma | Off by default in SQLite; set explicitly, as the docs advise. |
| `cache_size` | `-64000` → **62.5 MiB** | `src/capacity.rs` | An *upper bound*, lazily allocated, **per open database file**. The SQLite default is ~2 MB. |
| `mmap_size` | **256 MiB** | `config::DB_MMAP_SIZE_MIB` | **Address space, per database file, and multiplicative in file count.** |
| `temp_store` | `MEMORY` | per-connection | Sorts and `CREATE INDEX` run in RAM — budget for it. |
| `busy_timeout` | 5000 ms | per-connection | A project decision; **SQLite documents no recommended value.** |
| `wal_autocheckpoint` | 1000 pages (≈4 MB) | `DEFAULT_WAL_AUTOCHECKPOINT_PAGES` | SQLite's own default. *"All automatic checkpoints are PASSIVE."* |

### The memory budget, stated

`mmap_size` and `cache_size` are **not** alternatives:

- `mmap_size` is **address space** mapped from the OS page cache — it shares
  pages, and *"The mmap_size applies separately to each database file, so the
  total amount of process address space that could potentially be used is the
  mmap_size times the number of open database files."*
- `cache_size` is a **separate allocation** holding hot pages.
- `temp_store=MEMORY` is a third consumer.

**Size the host at ≥ 1 GiB free RAM for a single-file deployment**, and raise
`mmap_size` only if the address space is actually being used. Two
environment-tunable knobs, both fail-closed on a bad value:
`BRAIN_SYNCHRONOUS`, `BRAIN_WAL_AUTOCHECKPOINT`.

### What you must NOT tune

- **`page_size`** — frozen. *"It is not possible to change the page_size after
  entering WAL mode."* This database is permanently in WAL mode, so a
  maintenance script that sets `PRAGMA page_size=8192` is a no-op at best.
- **`auto_vacuum`** — cannot be enabled after tables exist, and *"because it
  moves pages around within the file, auto-vacuum can actually make
  fragmentation worse."*
- **`journal_mode=OFF` or `MEMORY`** — *"the database file will very likely go
  corrupt."* (That is about the *rollback* journal; it is a different thing from
  `temp_store`.)

### `synchronous=FULL` vs `NORMAL` — the decision to record

> *"WAL mode is safe from corruption with synchronous=NORMAL, and probably
> DELETE mode is safe too on modern filesystems. WAL mode is always consistent
> with synchronous=NORMAL, **but WAL mode does lose durability**. A transaction
> committed in WAL mode with synchronous=NORMAL might roll back following a
> power loss or system crash."*

**The folklore correction, which matters here:** WAL + `NORMAL` **cannot
corrupt** the database. It can **roll back the most recent transactions**. For a
hash-chained audit log, a rolled-back transaction is a *gap in the chain*, not a
crash. SQLite's own text says the loss *"is not important for most
applications"* — **that judgement is the SQLite authors', not this system's.**

**This deployment ships `FULL` (ACID) as the default for exactly that reason.**
Set `BRAIN_SYNCHRONOUS=normal` only if you have measured the commit latency and
accepted the durability trade for a workload that is not the audit chain.

---

## 4. Backup and restore

### The rule that matters

> *"The WAL file is part of the persistent state of the database and should be
> kept with the database if the database is copied or moved. **If a database file
> is separated from its WAL file, then transactions that were previously
> committed to the database might be lost, or the database file might be
> corrupted.**"* ([wal.html §4](https://www.sqlite.org/wal.html))

**Copy `brain.db` and `brain.db-wal` together. `brain.db-shm` is not required** —
it is rebuilt from the WAL and *"is deleted when the last database connection
disconnects."*

**Never delete a `-wal` file by hand.** *"The only safe way to remove a WAL file
is to open the database file using one of the `sqlite3_open()` interfaces then
immediately close the database."*

### Ranked mechanisms

| Mechanism | Use for | Note |
|---|---|---|
| **`brain standby ship`** | off-site, encrypted, signed | The product's own. Runs exactly one cycle and exits. |
| **`VACUUM INTO '<file>'`** | local snapshot, compaction | *"a consistent snapshot of the original database"*, and it **purges all deleted content** from the copy. The target **must not already exist** — use a timestamped name. |
| `sqlite3_rsync` (3.47.0+) | live copy over SSH | Available on the vendored 3.53.2. |
| `cp` | last resort | Only if no transaction is in flight **and** the `-wal` travels. |

### Disk headroom for `VACUUM`

> *"when VACUUMing a database, as much as **twice the size of the original
> database file** is required in free disk space."*

Size the data volume at **≥ 3× the working database size** if you intend to
VACUUM in place. This is a classic on-call surprise.

### The `close()` hazard — read before writing any backup script

> *"the `close()` system call will cancel all POSIX advisory locks on the same
> file for all threads and all file descriptors in the process… To avoid
> corruptions, developers should be careful to **never use `close()` on an SQLite
> database file while one or more database connections are open**."*

**Practical consequence:** do not run a CLI that opens *and closes* the live
database while the service is running — an integrity check, a file-type probe, a
`cp` followed by `sqlite3` — because the probe's `close()` can drop the
**server's** advisory locks. Work on a copy, or stop the service.

### Verification is not what you think

> *"`PRAGMA integrity_check` does not find FOREIGN KEY errors. Use the
> `PRAGMA foreign_key_check` command to find errors in FOREIGN KEY
> constraints."*

Run **both** when verifying a restore.

---

## 5. Deployment scenarios

Each scenario below is complete: the shape, the install, the verification, and
what it does **not** give you.

### Scenario A — single-node appliance (the common case)

*A city hall, a back office, one small machine, powered off at night.*

```sh
sudo ./deploy/install.sh
sudo systemctl start brain-server
/usr/local/bin/brain-clean-cycle-check          # every morning
sudo systemctl stop brain-server                # every evening
```

- **Storage:** one local ext4/XFS partition, mounted `noatime,errors=remount-ro`.
- **Auth:** see [`deployment.md`](deployment.md); loopback + proxy, or a bearer
  token file.
- **Backup:** `brain standby ship --to <off-site dir>` on a timer.
- **Does not give you:** any uptime while the box is off, and no protection from
  fire or flood — those need the off-site copy *and* the battery.
- **Full runbook:** [`clean-cycle.md`](clean-cycle.md).

### Scenario B — two-site with battery and a cold standby

*Outages are routine; a box may be down for hours at a time.*

```
 SITE A                          SITE B
 MiniPC 1  ACTIVE    ──ship──▶   vault (cold, signed)
 UPS-A + LiFePO₄                 UPS-B
 MiniPC 2  STANDBY (cold)
 UPS-B + LiFePO₄
 on a SEPARATE circuit
```

- **Separate circuits are the point.** At 98.8–99.4% availability from power
  alone, two hosts on one circuit die together and the second buys nothing.
- **Per-node UPS.** A shared UPS is a single point of failure wearing a
  redundancy costume. The load is ~100 W, so a second inverter is cheap.
- **The standby stays cold.** `promote-check` needs no running server, so the
  standby boots on demand. The cost is that RPO becomes *time since last ship*.
- **Verify the standby monthly.** Cold standby rots; a disk nobody has read in
  six months is a disk you learn about on the worst day.
- **Does not give you:** automatic failover, or split-brain protection — the
  lease is **not yet implemented**. Do not run two actives.
- **Detail:** [`deployment-reference-architecture.md`](deployment-reference-architecture.md).

### Scenario C — Docker Compose

*Existing Docker estate, no orchestrator.*

```sh
docker compose up -d
```

- **Storage:** a **named volume on local disk**, never a network mount.
- The image already runs as uid 1000 with `read_only`, tmpfs `/tmp`,
  `cap_drop: ALL`, `no-new-privileges`.
- **Does not give you:** node failover, backup scheduling, or the clean-cycle
  verification. Add `brain standby ship` on the host's timer.
- **Detail:** [`docker.md`](docker.md).

### Scenario D — Kubernetes *(not built; shape recorded)*

There is **no Helm chart**, and the earlier one would have used the wrong
primitive. If you build one:

- **`Deployment`, `replicas: 1`**, `strategy: Recreate` — **not** a StatefulSet.
  The StatefulSet `RollingUpdate` documented failure at `replicas: 1` is a wedged
  rollout: *"you must also delete any Pods that StatefulSet had already attempted
  to run with the bad configuration."*
- **`ReadWriteOncePod`**, not `ReadWriteOnce` — the Kubernetes project recommends
  RWOP for production.
- **`persistentVolumeClaimRetentionPolicy: Retain`**, so a rescheduled pod
  reattaches its data.
- **NEVER `hostPath`.** The project labels it single-node-testing-only; a
  rescheduled pod on an empty `hostPath` starts with a **brand-new empty
  database, silently** — and the standby will faithfully replicate the emptiness.
- **StorageClass is a reviewed value.** A RWOP PVC backed by NFS satisfies the
  access mode and still breaks SQLite. The provisioner is a security decision.
- **Do not use an in-cluster CronJob as the only backup.** It protects a
  database with the cluster it runs on. Use the customer's backup system, plus
  `brain standby ship` for the signed artifact.

---

## 6. Troubleshooting

| Symptom | Cause | Action |
|---|---|---|
| `journal mode is 'delete', not 'wal'` | volume cannot do WAL | move to local block storage — §1 |
| Server will not start, `integrity_check` fails | store damaged | restore from the off-site copy; do not VACUUM in place |
| `-wal` file grows without bound | checkpoint starvation — *"if a database has many concurrent overlapping readers and there is always at least one active reader, then no checkpoints will be able to complete"* ([wal.html §6](https://www.sqlite.org/wal.html)) | create **reader gaps**; do NOT raise the autocheckpoint threshold |
| Filesystem remounted read-only | `errors=remount-ro` did its job | check `dmesg` for the underlying I/O error — **this is a hardware/volume event** |
| Crash on a low-memory host | *"An I/O error on a memory-mapped file cannot be caught… results in a program crash"* | lower `mmap_size`, or add RAM — §3 |
| `database is locked` under load | `busy_timeout` exceeded | check `busy_errors_total` and `pool_timeouts_total` on `/metrics` |

---

## 7. Claims this document deliberately does not make

- **That ext4 is better than XFS, or the reverse.** No primary source.
- **That direct I/O helps.** No SQLite page mentions it.
- **That `nodelalloc` helps a database.** No recommendation exists.
- **That `noatime` measurably helps here.** Safe and documented; the gain is
  unmeasured.
- **That `VACUUM INTO` is needed.** It is one ranked option among several.
- **Any compliance conclusion.** This document states what the code does and
  what the statute says. Whether a given deployment satisfies any of it is a
  determination for a qualified assessor and, in the Philippines, for counsel.

### Sources

Fetched 2026-09-28: [sqlite.org/wal.html](https://www.sqlite.org/wal.html) ·
[lockingv3.html](https://www.sqlite.org/lockingv3.html) ·
[vfs.html](https://www.sqlite.org/vfs.html) ·
[mmap.html](https://www.sqlite.org/mmap.html) ·
[pragma.html](https://www.sqlite.org/pragma.html) ·
[howtocorrupt.html](https://www.sqlite.org/howtocorrupt.html) ·
[lang_vacuum.html](https://www.sqlite.org/lang_vacuum.html) ·
[backup.html](https://www.sqlite.org/backup.html) ·
[faq.html](https://www.sqlite.org/faq.html) · [ext4(5)](https://man7.org/linux/man-pages/man5/ext4.5.html) ·
[xfs(5)](https://man7.org/linux/man-pages/man5/xfs.5.html) ·
[mount(8)](https://man7.org/linux/man-pages/man8/mount.8.html) ·
[ext4 journaling](https://www.kernel.org/doc/html/latest/filesystems/ext4/journal.html)
