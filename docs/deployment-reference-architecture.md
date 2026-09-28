# Deployment reference architecture

**What this document is.** The shape a larger brain-server deployment takes,
written down so it can be argued with. **What it is not:** a description of a
system anyone has built. Nothing here has been assembled on real hardware, and
every number that is not measured says so.

For the single-host case, read **[`clean-cycle.md`](clean-cycle.md)** instead.
This document is about what happens when one host is not enough.

---

## The two things that decide the shape

**1. Single-writer is a correctness property, not a capacity number.**

The service assumes exactly one writer. That is not a preference — the audit
chain computes `prev_hash` under `BEGIN IMMEDIATE` and appends, and the
resulting chain is only provable if the writer is single and serialized. A
second concurrent writer does not merely slow it down; it forks the chain.

Note what is **not** enforcing this: there is no file-lock anywhere in the
source tree. Single-writer today is SQLite's own transaction-level
serialization, which protects the chain but does **not** stop two processes
opening the same file. On a single host that is fine. Across two, it is the
whole design question.

**2. Power, not hardware, is the dominant failure mode.**

An operator-supplied figure (2026-09-28, **an estimate from personal
experience, not a measurement**): **1–2 hours of outage per week**. Arithmetic
on that, at a ~100 W combined load:

| Outage | Annual loss | Availability on **power alone** |
|---|---|---|
| 1 h/week | 52 h/yr | **99.41%** |
| 2 h/week | 104 h/yr | **98.81%** |

That is **at or just below three nines, from electricity and nothing else.** It
reorders the priorities: **the battery is the primary resilience investment, and
the second host is secondary.**

---

## The shape

```
        ┌─ SITE A ──────────────┐        ┌─ SITE B ─────┐
        │ MiniPC 1   (ACTIVE)   │        │ vault        │
        │ local ext4, brain.db  │──ship──▶ signed, cold │
        │ UPS-A + LiFePO₄       │        │ (backup only)│
        │                        │        │ UPS-B        │
        │ MiniPC 2   (STANDBY)  │        └───────────────┘
        │ cold, promotes        │
        │ UPS-B + LiFePO₄       │
        │ ON A SEPARATE CIRCUIT │
        └────────────────────────┘
```

### Per-node battery, not one shared UPS

A shared UPS is a single point of failure wearing a redundancy costume. The load
is ~100 W, so a second small inverter is cheap insurance. **Each MiniPC gets its
own UPS/battery**; if one fails, only that node is affected.

### The hosts must be on SEPARATE circuits

**Two MiniPCs on the same circuit are one node, not two.** At 98.8–99.4%
availability from power alone, a shared circuit means both die together in the
dominant failure mode and the second host buys almost nothing. Separate circuits
(ideally separate floors or buildings) are what make "two nodes" real.

### The standby stays COLD

`brain standby ship` produces signed artifacts; `brain standby promote-check`
verifies and restores them. **Neither requires a running server** — they are
filesystem and crypto operations on signed files. So the standby is powered down
most of the time and booted on demand.

The trade: a cold standby's RPO is *time since the last successful ship*, not
the 10.4 s the continuously-running shipper achieves. That is a real cost and
it is stated rather than hidden.

**Cold standby rots.** A disk nobody has read in six months is a disk you find
out about on the worst day. Run `brain standby promote-check` **monthly** — it
is a five-minute operator task and it is the single thing that makes a cold
standby trustworthy.

### The vault is off-site, and that is the point

Power resilience covers "the power went out". It does **not** cover fire, flood,
or theft. The off-site vault is the only thing that does, and it is why the
battery and the vault are complementary rather than alternatives.

**And it may need a signature.** Under **RA 10173 §23(b)** (primary text,
fetched 2026-09-28), sensitive personal information — which §3(l)(3) defines to
include social security numbers, health records, licences and tax returns, i.e.
what a city hall holds — **may not be transported off government property**
without the **agency head's approval**, with off-site access capped at 1,000
records and *"the most secure encryption standard recognized by the
Commission."* An off-site vault of citizen records is a **documented, signed
exception**, not a configuration choice. Keeping the vault on the same property
is the simplest way to stay inside it.

*Quotations of what the instrument says, not a compliance conclusion.*

---

## Why two hosts and not three

The "minimum three nodes" rule comes from **quorum consensus** — Raft needs
2/3, so three tolerates one failure. **brain-server does not use consensus by
design.** The promotion decision is a lease, not a consensus protocol, so:

- **Two hosts are enough** for fail-closed promotion.
- A **third node helps only if it is in a different town** — and if that is the
  concern, the right shape is **two vaults in two towns**, not three nodes in
  one building.

---

## Solar sizing — reasoned, not measured

| Bank | Runtime at 100 W | vs a 1 h outage |
|---|---|---|
| 2 kWh LiFePO₄ | ~13.6 h | 14× |
| 5 kWh | ~34 h | 34× |
| 10 kWh | ~68 h | 68× |

(85% inverter efficiency, 80% usable depth of discharge.)

**A battery is sized for the TAIL, not the mean.** A weekly average does not say
how long the *longest* outage was, and that number is unknown. Until it is
known, **every figure above is a requirement to be confirmed, not a result.**

## Still unmeasured, and labelled as such

- **The tail**: the longest observed outage. This is the number that should size
  the bank.
- **Whether outages are scheduled** (load-shedding — a different and partly
  *policy* problem) or unscheduled (grid failure).
- **Kanlaon volcano** siting for Negros Occidental — the region is
  seismically and volcanically active and nobody has checked the current alert
  level.
- **Whether a solar array charges fast enough** to matter during a multi-day
  cloudy spell. The battery does the work; the array only tops it up.

---

## Deliberately not built

| Not doing | Why |
|---|---|
| A Kubernetes Operator | A maintained product — CRD versioning, upgrade paths, compatibility matrix — for a fleet that does not exist. A StatefulSet is also the **wrong primitive** here: its own docs document a `RollingUpdate` wedge at `replicas: 1`, and it recommends `ReadWriteOncePod` over `ReadWriteOnce`. If a chart is ever built it should be a `Deployment` with `Recreate` semantics — and **never `hostPath`**, which the Kubernetes project labels single-node-testing-only and which would silently hand a rescheduled pod an **empty database**. |
| Multi-replica anything | Single-writer is a correctness property (§ above). |
| A NAS on the database path | SQLite forbids a network filesystem for the database (`lockingv3.html` §6.0). A NAS is fine for opaque, hash-verified backup artifacts and **never** for `brain.db`. |
| A managed database (Postgres et al.) | `brain shred` asserts byte-level erasure; MVCC dead tuples survive until vacuum. A shipped, pinned guarantee. |
| Volume-snapshot backup as the *only* backup | Correct only when the whole volume is captured **and** the pod is quiesced. A single-file `brain.db` snapshot is a data-loss bug by SQLite's own definition. |

---

## What this shape does not give you

- **No automatic failover.** Promotion is an operator action with a rehearsed
  command. That is a feature for a small deployment and a limitation for a large
  one.
- **No split-brain protection yet.** The lease is the design; the implementation
  is deferred to a later round. Until it lands, two active instances is
  possible — do not run two.
- **No measured RTO/RPO for this topology.** The 0.55 s / 10.4 s figures are
  for the database restore on one host, measured in a drill, not for a two-site
  failover.
- **No claim of compliance.** See `COMPLIANCE.md` and the qualifications in
  `clean-cycle.md`.
