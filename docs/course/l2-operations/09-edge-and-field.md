# Lesson 9: Edge and field deployments

**Level:** L2 · **Time:** about 20 minutes · **For thin, offline, or air-gapped hardware**

## Questions people ask

**Does it really run offline on small hardware?** Yes, and that is a
design constraint, not a coincidence: one Rust binary, one SQLite file,
local static embeddings (model2vec), zero cloud calls in the recall path,
zero embedding or decision tokens per query. It fits a Jetson or a
Raspberry Pi class machine. There is no telemetry.

**What does an edge deployment look like?** A single domain, loopback or
LAN bind, a reverse proxy if humans reach it, and the appliance rhythm
from lesson 5: clean stop in the evening, morning check (anchor before
stopping, verify after starting). The [deployment](../../deployment.md)
page carries tier profiles (`deploy/tiers/`) that bundle the knobs for
exactly this: pick the tier, boot it, smoke it.

**What is different operationally at the edge?**

- **Updates are deliberate.** No auto-update anything. You ship the
  binary, you own the cadence, and the schema refuse-newer law means a
  version-skewed database is a loud error, never a silent mess.
- **Backups leave the box.** `brain standby ship --to` pointed at
  removable or network storage, or `brain backup` on a schedule you
  actually run. An edge box with local-only backups is a single point of
  failure wearing a hat.
- **Power loss is the normal failure.** WAL means a dirty power-off
  leaves you recoverable, and the morning integrity check (`/ready`,
  `brain doctor`, anchor verify) is how you KNOW rather than hope.
- **Capacity is measured, not guessed.** The eval floors and the
  benchmarks page carry real numbers; size the box against your corpus,
  not against marketing.

## Exercise

1. Boot on the smallest machine you have (or throttle a container).
   Run the quickstart flow end to end.
2. Kill the power (or `kill -9` mid-write, on a throwaway). Start it
   again. Run `brain doctor` and the anchor verify. Record what recovered
   and what the evidence was.
3. Ship one standby cycle to a second directory (standing in for removable
   storage). Verify with `brain standby status`.

## Next

[Lesson 8's capstone](08-when-things-look-wrong.md) plus this one is the
edge exam. [Back to the course index](../README.md).
