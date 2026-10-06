# Lesson 8: When things look wrong

**Level:** L2 · **Time:** about 25 minutes · **Commands, one deliberate failure, one capstone**

## What you will do

Build your troubleshooting order of operations, meet the verification verbs
that answer "is the DATA sound" as opposed to "is the process up", and run
the Level 2 capstone.

## First, the triage order

When something looks wrong, resist the urge to restart. Restarting is the
LAST step of troubleshooting here, not the first, because a restart
destroys the in-memory evidence of what went wrong.

1. **Is it up?** `brain doctor`, then the logs
   (`~/Library/Logs/brain-server.log` and `.err.log` on macOS, journalctl
   under systemd).
2. **Is the data sound?** `curl -s localhost:8765/audit/verify` should say
   ok. This is the difference between a process problem (restartable) and
   an integrity problem (escalate immediately).
3. **Is it memory-side?** Pending, expired, or superseded, in that order.
   Level 1 lesson 7's checklist resolves most "why doesn't it recall".
4. **Is it drift?** The next section.

## The three verification verbs

These answer questions nothing else in the stack can:

```bash
# The drift census: re-scores a frozen gold corpus and diffs against the
# committed baseline. A breach writes a findings row and exits non-zero.
# Run from cron. It writes nothing on a clean pass, on purpose.
brain census

# Disproof sweep: checks every stored claim's own disproof condition.
# Three states: satisfied, REFUTED (listed by id), no verdict (prose
# conditions, never green). Writes nothing, ever. Run from cron.
brain disproof

# The off-host witness: fingerprint the whole state, record the line
# somewhere that is NOT this machine, and verify it later.
brain anchor
brain anchor --verify "<the recorded line>"
```

The anchor deserves its own paragraph, because it is the one control that
catches the attack nothing else does. The audit chain catches tampering
WITH the chain. An attacker who moves rows around while keeping the chain
consistent is caught by the anchor: it fingerprints the knowledge content
itself, and any change since your recorded line trips the verify, with the
chain then explaining which changes were legitimate. Record the line on
paper, in a password manager, on a second machine. The off-host copy IS
the evidence. This system was audited against exactly this attack class,
and the anchor is what came of it.

## Routing refusals, read as diagnostics

If you use the routing seam, its refusals are informative on purpose: an
undeclared queue escalates to a human rather than inventing a destination,
an unrecognized class label is refused outright, and a supplied confidence
number is accepted and DISCARDED, with the receipt saying so. When routing
refuses, it is telling you the declared vocabulary and reality disagree.
Fix the vocabulary, not the refusal.

## The incident shape for credentials

If you suspect a token leak: rotate first (`brain token rotate`, hot, no
restart), then read the auth-failure feed and the audit trail for what the
old token did, then revoke identities if agent or JWT tokens were involved.
The order matters: cut access, then investigate. You met the reasons in
lesson 2.

## When you do restart

Cleanly, through the service manager (`launchctl`, `systemctl`), never
kill -9 mid-write. The clean-cycle page has the full liturgy, and the
morning-after check is the anchor verify, which is why you anchor before
stopping.

## The Level 2 capstone

On throwaway instances, one sitting:

1. Install and configure with a profile. Two-line token file, review
   posture.
2. Ingest a directory (dry-run first), trip the screen once, handle the
   quarantine row.
3. Register a client, set DPA terms, place a hold.
4. Back up, verify, restore to a fresh path, confirm the memory.
5. Ship three standby cycles and run promote-check. Write down your
   measured recovery numbers.
6. Anchor, record the line OFF-HOST (a note on your phone counts), then
   verify it.
7. Break something on purpose: edit a row in the database directly with a
   sqlite client, then run the anchor verify and the audit verify. Read
   what each says. Put it back by restoring your backup.

Step 7 is the graduation exercise. If you can detect your own tampering
and recover from it, you can run this system through a bad day.

## What you learned

- Triage order: up, then integrity, then memory states, then drift.
  Restart last.
- census (drift), disproof (claims), anchor (the off-host witness against
  behind-the-chain tampering).
- Routing refusals are vocabulary diagnostics.
- Credential incidents: rotate, then investigate.
- The capstone: tamper, detect, recover.

## Where to go next

You now run the box. [Level 3](../l3-verification/01-how-to-use-this-course.md)
is recommended reading even if nobody is auditing you yet, because it
teaches you to see the system the way an assessor will, and its checks are
the same ones you will reach for on your worst day.

Further reading: [Warm standby](../../standby.md),
[The clean cycle](../../clean-cycle.md),
[CLI reference](../../cli-reference.md).
