# Two people have to say yes

*2026-09-11. v1.28.80 adds an optional second approver to the memory
promotion path. This post explains the failure mode it exists for: the
rubber stamp.*

Every approval queue in production converges on the same behavior. The
reviewer trusts the system, the items blur together, and approval becomes
a reflex. Security literature has a name for the resulting hole:
approval fatigue laundering. A poisoned entry does not need to fool the
reviewer. It only needs to arrive on a busy afternoon.

The standard answer is telemetry: measure approval uniformity, flag the
reviewer who approves everything. That is detection after the fact. It
tells you the stamp got rubbery last month. The entry is already in
memory, already recalled, already acted on.

The structural answer is older than software. Banks call it four eyes.
No payment moves on one signature, not because every cashier is suspect,
but because two independent judgments fail differently than one tired
judgment repeated twice.

`BRAIN_APPROVAL_QUORUM=2` ports that rule to memory promotion. The first
approval does not promote. It records a hash-chained row and returns
`pending_second`. Promotion needs a different principal, and a repeat by
the same principal is refused outright. The default stays single approval,
because a personal deployment with one operator and a quorum of two is a
deadlock, not a control. Enterprise pilots turn it on.

Two people saying yes is not twice as slow. It is the difference between
a gate and a ritual.
