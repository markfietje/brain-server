# Warm standby

Standby is a rehearsed spare copy, not failover. An operator-run shipper
copies the live database to a follower directory on a schedule. If the
primary dies, the operator promotes the follower by hand. Nothing here is
automatic, and no page claims otherwise.

## Commands

All three run against copies. They never touch the live database except to
read from it.

- `brain standby start --to <dir> [--interval-secs 30] [--passphrase-file PATH]`
  loops: checkpoint the live DB, write an encrypted base image, copy the
  newest WAL chunk encrypted, then write the signed manifest last. A cycle
  interrupted halfway heals on the next cycle. Three failed cycles in a row
  stop the loop. Interval minimum 5 seconds, default 30.
- `brain standby status [--to <dir>]` verifies the follower (signature over
  the exact manifest bytes plus artifact hashes) and prints cycle age,
  cycles behind, worst-case RPO, sizes, and integrity. Tampering exits 1.
- `brain standby promote-check --from <dir> [--passphrase-file PATH]
  [--expected-signer DID]` rehearses a promote into a temp directory:
  decrypt, restore, open, integrity check. Prints measured RTO/RPO and
  PASS or fail.

## What lands on the follower

`<dir>/base.v3` (encrypted base) plus `wal/NNNN.frame-chunk` files (one
encrypted WAL copy per cycle) plus `manifest.json` with its detached
`manifest.sig.json` (Ed25519, same convention as signed parcels). No
unencrypted byte rests on the follower.

## Secrets

Passphrase comes from `--passphrase-file` or `BRAIN_BACKUP_PASSPHRASE_FILE`
(the file must be 0600; anything readable refuses). Signing needs the
operator key; a ship without a key refuses instead of shipping unsigned.
Default follower dir is `BRAIN_STANDBY_DIR`, else
`~/.local/share/brain-server/standby`.

## Measured numbers

Drill of 2026-09-06 against a copy of the live 48.8 MB database:
checkpoint lag ~0.4 s, worst-case RPO 10.4 s at a 10 s interval, promote
0.55 s, 9,091 promoted rows with the post-cycle commit honestly absent
(inside the RPO window), one flipped byte detected with exit 1.
RPO follows `interval + checkpoint lag`; the status command computes it
per cycle rather than asserting it once.
