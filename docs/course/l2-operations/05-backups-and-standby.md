# Lesson 5: Backups and standby

**Level:** L2 · **Time:** about 25 minutes · **Commands, one drill, no production data**

## What you will do

Learn the backup and restore path, then the warm standby loop, then run the
one drill that separates "we have backups" from "we have restores".

## The backup

```bash
brain backup ~/backups/brain-$(date +%F).enc \
  --passphrase-file ~/.config/brain-server/backup.pass
```

The backup is encrypted (AES-256-GCM), checksummed, and excludes secrets by
design: your key material does not go wandering around inside backup files.
The passphrase is required, read from a file, and that file is a secret
like any other (0600). The current format is v3; older formats restore
fine, and the format flag exists if you need to write one.

`brain doctor --backup <path> --passphrase-file <path>` verifies a backup
without restoring it. Do that after every backup job you set up. A backup
nobody verified is a hope, not a backup.

## The restore, and its three doors

```bash
brain restore ~/backups/brain-2026-10-06.enc --passphrase-file ~/.config/brain-server/backup.pass
```

Restore asks before it overwrites anything, always, unless you pass `--yes`.
`--force` skips only the liveness probe (is the target a live database),
never the human gate. Two special refusals worth knowing cold:

- **A backup with no audit chain in it refuses to restore** unless you pass
  `--allow-chainless`, and then it restores with a loud disclosure that the
  result's history is unverifiable. The flag exists for archaeology, not
  for Tuesdays.
- **Legacy, unkeyed chains** restore marked `forgeable: true` until you
  re-anchor the chain. The system would rather call its own history weak
  than let you believe it is strong.

One mistake the docs see often enough to warn about: restore takes the
database path from `BRAIN_DB_PATH` or the default, NOT from a positional
argument. Point the env var, then restore.

## Warm standby: a rehearsed follower

Standby is warm, never hot. What that means, exactly: a second machine
receives an encrypted, signed stream of the database (base plus WAL
chunks), and it can be PROMOTED by a rehearsed manual step. There is no
automatic failover, and there is no zero-data-loss claim anywhere, because
there is no zero data loss.

The loop, owned by a scheduler you control (cron, launchd, a timer):

```bash
brain standby ship --to /follower/dir --passphrase-file ~/.config/brain-server/backup.pass
brain standby status --to /follower/dir
```

`ship` does one cycle and exits: checkpoint, encrypted base, encrypted WAL
chunk, signed manifest written last. An interrupted cycle self-heals on the
next one. `status` verifies the manifest signature and every artifact hash,
and FAILS on any tamper or torn cycle. It prints the follower's age and the
honest recovery-point math: worst case, the shipping interval plus
checkpoint lag.

## The drill

```bash
brain standby promote-check --from /follower/dir \
  --passphrase-file ~/.config/brain-server/backup.pass
```

This is the whole point of standby. It restores the follower into a
temporary directory through the same restore path production would use,
replays the WAL, runs the database integrity check, and prints the measured
recovery time and the computed recovery point. You run this BEFORE you ever
need it, on a schedule, because the day you need it is the worst day to
learn the follower has been silently broken for a month.

## The appliance rhythm

There is a whole operating discipline for stop-in-the-evening,
check-in-the-morning appliance deployments, the "clean cycle": stop it
properly (checkpoints land), check it came back correct in the morning
(anchor before, verify after). It has its own page, and if you run an
appliance-style deployment, that page is your liturgy. The one rule from it
that applies to everyone: never kill the process mid-write when a clean
stop is available.

## Exercise

On a throwaway instance:

1. Take a backup. Verify it with `brain doctor --backup`.
2. Restore it to a FRESH path (set `BRAIN_DB_PATH` to a new file), and
   confirm your memory is there.
3. Break the passphrase on purpose (wrong file) and read the refusal.
4. Set up a standby directory, ship three cycles, run status, then
   `promote-check`. Write down the two numbers it prints. Those are your
   real recovery objectives, measured, not promised.

## What you learned

- Encrypted, checksummed backups, secrets excluded, verify after every run.
- Restore always asks; chainless and legacy images refuse or disclose.
- Warm standby, no hot failover, no zero-loss claim. Ship, status,
  promote-check.
- Drills are the difference between having backups and having restores.

## Next

[Lesson 6: Channels and assistants](06-channels-and-assistants.md)

Further reading: [Warm standby](../../standby.md),
[The clean cycle](../../clean-cycle.md).
