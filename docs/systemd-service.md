# systemd service operation (Linux)

**Audience:** the operator running brain-server as a Linux systemd service.

**Scope:** what `deploy/install.sh`, `deploy/systemd/brain-server.service`,
`deploy/uninstall.sh`, and `deploy/clean-cycle-check.sh` do — and what they
deliberately do not do.

**Not this document:** filesystem choice, mount options, memory budget,
backup ranking, and multi-site shape. Those live in
[deployment-filesystem.md](./deployment-filesystem.md) and the appliance
runbook [clean-cycle.md](./clean-cycle.md). This page complements them; it
does not repeat them. General install, configuration, and tiers live in
[deployment.md](./deployment.md).

**Honesty posture.** Verified 2026-10-06 against the files named above in
this tree. Directives, paths, and behaviours below are quoted from those
files. The shutdown timings are measured 2026-09-28 on a physical Ubuntu
host (14 MB store, 53 KB WAL), as recorded in the unit comments and
[clean-cycle.md](./clean-cycle.md) — not re-measured here. If this page and
the scripts disagree, the scripts are right.

---

## 1. Install flow (`deploy/install.sh`)

Run as root:

```sh
sudo ./deploy/install.sh [--prefix /usr/local] [--data /var/lib/brain-server]
sudo systemctl start brain-server
/usr/local/bin/brain-clean-cycle-check
```

What the script does, in order:

1. **Requires root.** Exits 1 otherwise (`install.sh must run as root`).
2. **Refuses to clobber.** If `$DATA/brain.db` exists and `BRAIN_FORCE` is
   not `1`, it exits 1 and prints the upgrade sequence instead:
   `systemctl stop brain-server`, `cp -a $DATA $DATA.bak.$(date ...)`,
   re-run the script, `systemctl start brain-server`. Deliberate override
   is `BRAIN_FORCE=1`. An upgrade that silently overwrites the store is
   treated as unrecoverable, so the installer will not do it.
3. **Creates the service user.** System group and user `brain`
   (`groupadd --system`, `useradd --system --gid brain --home-dir $DATA
   --shell /usr/sbin/nologin`), then `install -d -m 0750 -o brain -g brain`
   for `$DATA` and `$DATA/keys`, and `install -d -m 0755` for
   `$PREFIX/bin`.
4. **Requires local release binaries.** It expects executable
   `target/release/brain-server` and `target/release/brain` relative to the
   script (build with `cargo build --release --bin brain-server --bin brain`).
   Missing source is a hard error. Before copying, it stops a running
   instance matched by **absolute binary path** (`pgrep -f "$src"` /
   `pkill -TERM -f "$src"`, up to 60 s wait). It deliberately never matches
   on the database path or port: `BRAIN_DB_PATH` lives in the environment,
   not in argv, so `pkill -f` on it matches nothing — see also
   [clean-cycle.md](./clean-cycle.md).
5. **Installs helpers.** Writes `$PREFIX/bin/brain-shutdown-stamp` (the
   `ExecStopPost` stamp writer, §2) and installs
   `deploy/clean-cycle-check.sh` as `$PREFIX/bin/brain-clean-cycle-check`.
6. **Installs the unit.** Copies `deploy/systemd/brain-server.service` to
   `/etc/systemd/system/brain-server.service` (mode 0644), then rewrites the
   data path and prefix actually chosen
   (`sed -i "s#/var/lib/brain-server#$DATA#g; s#/usr/local/bin#$PREFIX/bin#g"`),
   runs `systemctl daemon-reload`, and `systemctl enable brain-server.service`.
7. **Prints the local-block warning.** The data volume must be a local
   block filesystem (ext4/xfs); a network filesystem cannot provide the
   advisory locking and shared memory SQLite WAL requires. The server
   refuses to start on one and names the cause (boot check in
   `src/migration.rs`, per the unit comments). Filesystem detail is in
   [deployment-filesystem.md](./deployment-filesystem.md) §1.

Defaults are `--prefix /usr/local` and `--data /var/lib/brain-server`.
The installed unit, data dir, check binary, start command, and log command
(`journalctl -u brain-server -f`) are echoed at the end of a successful run.

> **Custom-path caveat (read before using `--data`).** The unit file itself
> is rewritten for your `$DATA`, but the generated
> `$PREFIX/bin/brain-shutdown-stamp` still writes the compiled-in default
> `/var/lib/brain-server/.shutdown-clean`, and `brain-clean-cycle-check`
> defaults to `BRAIN_DB_PATH=/var/lib/brain-server/brain.db` and
> `BRAIN_STAMP=/var/lib/brain-server/.shutdown-clean` unless the
> corresponding environment overrides are set. A non-default `--data`
> install must align the stamp path explicitly or the morning check will
> look in the wrong place. Likewise `deploy/uninstall.sh` has no
> `--prefix`/`--data` flags and removes the default paths only (§3).

---

## 2. What the unit does (`deploy/systemd/brain-server.service`)

Read the unit before editing it. The directives below are verbatim.

### Drain on stop

```ini
ExecStart=/usr/local/bin/brain-server
KillSignal=SIGTERM
KillMode=mixed
ExecStopPost=/usr/local/bin/brain-shutdown-stamp
```

`systemctl stop brain-server` sends `SIGTERM`. That is the signal the drain
path handles: stop accepting, close the pool, then
`PRAGMA wal_checkpoint(TRUNCATE)` (`src/main.rs`: checkpoint-on-shutdown
block; best-effort — a failure is logged, not fatal, because SQLite replays
an un-checkpointed WAL on the next open). `ExecStopPost` then stamps
`date -Is` into `/var/lib/brain-server/.shutdown-clean`. The stamp's
**absence** after a stop means the process was killed, not stopped — that is
the signal the morning check reads (§4).

Do not stop the service with `pkill -f brain.db`. Same reason as the
installer: the database path is not in argv, so the pattern matches nothing
while reporting success.

### Timeouts

```ini
TimeoutStopSec=30
TimeoutStartSec=90
```

`TimeoutStopSec=30` is the stop budget. Per the unit comments: measured
`SIGTERM` → exit was **31 ms** total, of which `wal_checkpoint(TRUNCATE)`
was **0.2 ms** (14 MB store, 53 KB WAL, physical Ubuntu host, 2026-09-28).
30 s is ~1000× the measured shutdown. The variable term is **WAL size at
shutdown, not database size** — a write burst leaves a larger `-wal` and a
larger checkpoint.

A too-short timeout does not lose rows: SQLite replays the WAL on next open
(`src/main.rs:89-90`). It costs recovery latency and a larger `-wal` until
it drains. Re-measure against a **copy** after the store grows materially —
the command is in [clean-cycle.md](./clean-cycle.md) — then update
`TimeoutStopSec` and record the new figure.

`TimeoutStartSec=90` caps the start phase.

### Restart

```ini
Restart=on-failure
RestartSec=5
```

A non-clean exit is restarted after 5 s. A clean stop (`systemctl stop`,
exit 0) is **not** restarted. `Restart=on-failure` does not fix a
deterministic boot failure — a bad volume, a refused bind, a missing key
fails the same way every 5 s until the cause is removed. See §5.

### Identity, environment, and sandbox

```ini
Type=simple
User=brain
Group=brain
WorkingDirectory=/var/lib/brain-server
After=network-online.target
Wants=network-online.target
WantedBy=multi-user.target
```

```ini
Environment=BRAIN_DB_PATH=/var/lib/brain-server/brain.db
Environment=BRAIN_UMP_KEY_DIR=/var/lib/brain-server/keys
Environment=BIND_HOST=127.0.0.1
Environment=BIND_PORT=8765
Environment=RUST_LOG=info
```

Least-privilege set, verbatim: `NoNewPrivileges=true`, `PrivateTmp=true`,
`PrivateDevices=true`, `ProtectHome=true`, `ProtectSystem=strict` with the
single exception `ReadWritePaths=/var/lib/brain-server`,
`ProtectKernelTunables=true`, `ProtectKernelModules=true`,
`ProtectControlGroups=true`, `RestrictSUIDSGID=true`,
`RestrictRealtime=true`, `LockPersonality=true`, and fully dropped
`CapabilityBoundingSet=` / `AmbientCapabilities=`. The server binds an
unprivileged loopback port and writes one directory; it is granted nothing
else. Auth, bind, and provider configuration beyond these five defaults
live in [deployment.md](./deployment.md) and
[configuration.md](./configuration.md) — the unit does not invent them.

---

## 3. Uninstall guarantees (`deploy/uninstall.sh`)

```sh
sudo ./deploy/uninstall.sh
```

1. Requires root (`uninstall.sh must run as root`).
2. If `brain-server.service` is active, stops it
   (`systemctl stop brain-server.service` — SIGTERM → drain →
   `wal_checkpoint(TRUNCATE)`), then `systemctl disable` it.
3. Removes the unit (`/etc/systemd/system/brain-server.service`) and the
   four binaries (`/usr/local/bin/brain-server`, `/usr/local/bin/brain`,
   `/usr/local/bin/brain-clean-cycle-check`,
   `/usr/local/bin/brain-shutdown-stamp`), then `systemctl daemon-reload`.

**Guarantee: removes the service, never the state.** The data directory
(`$DATA`, default `/var/lib/brain-server`) is intact and untouched — store,
audit chain, and keys all still there. The server cannot start after this
without a reinstall.

Deliberate data removal is spelled out, not automated. The script instructs:

1. `brain shred --db $DATA/brain.db` — asserts byte-level erasure; not
   optional ceremony.
2. Remove the directory by hand. The destructive command is deliberately
   **not** written out — an operator who types it has decided to.

Limit: the script takes no flags and removes the default
`/usr/local/bin/*` paths. A custom `--prefix`/`--data` install is only
partly uninstalled by it; remove the relocated paths by hand.

---

## 4. The morning clean-cycle check (`deploy/clean-cycle-check.sh`)

Run before opening the console:

```sh
/usr/local/bin/brain-clean-cycle-check
```

Exit 0 is `PASS — safe to serve`. Non-zero is `FAIL` with the reason on
stdout, suitable for gating a start script. Overrides:
`BRAIN_BIN` (default `/usr/local/bin/brain`),
`BRAIN_DB_PATH` (default `/var/lib/brain-server/brain.db`),
`BRAIN_STAMP` (default `/var/lib/brain-server/.shutdown-clean`).

Three checks, in script order:

| # | Check | FAIL means |
|---|---|---|
| 1a | `PRAGMA integrity_check` via `sqlite3` (expects `ok`) | store structurally damaged — stop and investigate; restore from the off-site copy, do not VACUUM in place |
| 1b | `PRAGMA journal_mode` (expects `wal` or `memory`) | volume cannot do WAL — move the data to a local block filesystem (ext4/xfs); cites `sqlite.org/lockingv3.html` §6.0. `memory` is the deliberate in-memory test store |
| 2 | `brain anchor --db $DB` through the server's own verifier | anchor failed — the off-host anchor no longer matches; possible behind-the-chain tampering |
| 3 | stamp file exists | no clean-shutdown stamp — previous process was **killed**, not stopped; WAL replays automatically but expect a slower first query and a larger `-wal` until it drains; investigate what killed it (power, OOM, `kill -9`, operator) |

Skips are honest, not silent: without `sqlite3` installed the script prints
`[skip]` for integrity and journal mode; without an executable `$BIN` it
prints `[skip]` for the audit chain. A `PASS` with skips is a partial check
— install what is missing before trusting it.

Evening/morning cadence, storage rules, backup rules, and the signed
off-site approval live in [clean-cycle.md](./clean-cycle.md).
Single-node and two-site shapes live in
[deployment-filesystem.md](./deployment-filesystem.md) §5.

---

## 5. Troubleshooting a failed service

Work in this order. Every command below appears in the scripts or their
output, or in the linked runbooks — nothing here is a second way to stop
the server.

1. **Is it the unit or the store?**
   `systemctl status brain-server` and `journalctl -u brain-server -f`.
   A `journal mode is 'delete', not 'wal'` refusal is the storage gate:
   move the data to a local block filesystem. There is no override, by
   design. Detail: [deployment-filesystem.md](./deployment-filesystem.md)
   §1 and §6.
2. **Was the last stop clean?** Run the morning check (§4) and read the
   stamp line. Missing stamp + slow first query = killed process with WAL
   replay, not corruption. Find the killer before serving.
3. **Is it restart-looping?** `Restart=on-failure` with `RestartSec=5`
   retries a failing boot indefinitely. Stop the loop
   (`sudo systemctl stop brain-server`), fix the cause (volume, bind,
   auth material per [deployment.md](./deployment.md)), then start once.
4. **Did a deploy just land?** Confirm the unit at
   `/etc/systemd/system/brain-server.service` matches
   `deploy/systemd/brain-server.service` plus your `--prefix`/`--data`
   rewrite, then `systemctl daemon-reload`. Confirm the binaries in
   `$PREFIX/bin` are the just-built release pair — `install.sh` refuses to
   proceed without them.
5. **Is the check itself degraded?** `[skip]` lines mean a missing
   `sqlite3` or `$BIN`. Install them and re-run; do not promote a
   skipped check to a passed one.

Never delete a `-wal` file by hand, never copy `brain.db` without its
`-wal`, and never probe the live database with a tool that opens and
closes it while the service runs (the probe's `close()` can drop the
server's POSIX advisory locks). Ranked copy mechanisms and the `close()`
hazard are in [deployment-filesystem.md](./deployment-filesystem.md) §4.

---

## 6. Honest limits

- **One host, one active, no failover.** The unit manages a single
  `Type=simple` process. Losing the host means a restore from the signed
  off-site copy. Automatic failover and split-brain protection are not
  built — do not run two actives. Larger shapes are recorded, with
  unmeasured parts labelled, in
  [deployment-reference-architecture.md](./deployment-reference-architecture.md).
- **The stop budget is one measurement, not a law.** 30 s covers ~1000× a
  31 ms shutdown with a 53 KB WAL (2026-09-28). A store with a far larger
  WAL at shutdown checkpoints longer. Re-measure per
  [clean-cycle.md](./clean-cycle.md) after material growth; until then the
  margin is reasoned, not proven.
- **Custom `--prefix`/`--data` installs are second-class.** The stamp
  writer keeps the default data path, the check defaults keep the default
  paths, and uninstall removes the default paths only. Non-default layouts
  work only with explicit `BRAIN_STAMP`/`BRAIN_DB_PATH` alignment and
  manual uninstall of relocated files.
- **A skipped check is not a passed check.** Without `sqlite3` or the
  `brain` CLI the morning script reports `[skip]` and can still exit
  `PASS`. Treat that as unverified, not as healthy.
- **No compliance conclusion.** This page states what the unit and scripts
  do. Whether a given deployment satisfies any statute or framework is a
  determination for a qualified assessor (and, in the Philippines, for
  counsel) — same posture as
  [deployment-filesystem.md](./deployment-filesystem.md) §7.
