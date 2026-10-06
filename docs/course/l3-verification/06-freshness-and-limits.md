# Lesson 6: Freshness and limits

**Level:** L3 · **Time:** about 20 minutes · **Read + one final exercise**

## Why freshness is its own lesson

Everything you verified in lessons 2 through 5 was true of a version at a
moment. An evidence pack that cannot answer "which version, when" is a
postcard, not evidence. This closing lesson is about keeping your
verification attached to reality as reality moves.

## The staleness discipline

When you verified: record `brain status` output (version) and the date,
with every pack. When you re-verify: the [changelog](../../CHANGELOG.md)
says what moved between your version and now, and the
[release checklist](../../release-checklist.md) is the gate each release
passed. Re-running the reproduce script after any significant upgrade takes
three minutes, and it is the cheapest insurance in this whole course. If
you verify once and never again, say so in your report, with the date of
the once.

The system helps you here in one specific way worth noticing: its
documentation discipline ties claims to releases, and its docs gates fail
the build when a doc's claims drift from the code. That is unusual, and it
is checkable: find any security-relevant claim in the docs, and the proof
map names the command that evidences it today, not at some historical
moment.

## The ceilings, collected

These are the honest limits a thorough reviewer should walk away knowing.
Each is stated in the system's own docs, several print at runtime, and
none is hidden in an appendix.

- **Backups retain erased content** taken before an erasure. Backups age
  out on the retention schedule. Shred cleans the live file, not copies.
- **Standby is warm, never hot.** A rehearsed manual promotion, measured
  RTO and RPO, no automatic failover, no zero-loss claim.
- **DSAR locate is label-precise, mention-sweep best-effort.** Ownership
  labeling at capture is therefore part of the rights posture.
- **Session re-authentication on live streams is polling-based.** A
  revocation lands within the interval, not instantly, on that surface.
- **Workload identity between components is static shared secrets.** The
  design keeps every seam loopback or credential-gated, and says plainly
  that per-boot ephemeral identity was considered and declined, with
  reasons, in the threat model.
- **The injection screen is a tripwire, not a boundary.** It catches what
  it knows; the fence around recalled memory and the human gate are the
  actual controls.
- **A local-model chat window does not exist in this release.** The chat
  integration provides memory only.

Read that list once more and notice what it is: a map of where NOT to take
the system's other claims at face value. A vendor that gives you this map
is doing half your job for you. Verify a sample anyway. Pick any ceiling,
find its statement in the docs or its runtime output, and confirm it is
where the pack says it is. That sample check is the difference between
reading marketing with limits and verifying an engineering artifact.

## Where the deep material lives

For the questions this course deliberately did not chase: the
[proof map](../../trust/proof-map.md) is the claim-to-evidence index, the
[architecture](../../architecture.md) page is the layering law with a
request-flow diagram, the [compliance](../../compliance.md) page carries
the regulatory mapping with its own provenance labels (including which
dates are verified against primary sources and which are not), and the
security and threat-model pages enumerate controls and residual risk in
the operators' own vocabulary.

## The final exercise

Assemble the evidence pack. One folder, from your fresh-instance runs:

1. Version stamp and date (`brain status` output).
2. Reproduce script output, complete run.
3. One anchor line and its verify trip, with the chain's explanation.
4. The rights set: dry-run footprint, certificate with chain_verifies,
   tombstone count, one hold refusal.
5. The control set: undigested approval refusal, agent-token write
   refusal, and both negative controls.
6. One ceiling, located in the docs or runtime output, quoted with its
   page or command.

Six items. If you can produce that folder, you have done more verification
than most procurement processes ever ask for, and you did it in an
afternoon, which is rather the point of how this system is built.

## Where the courses meet

You have now seen all three audiences' views: the operator who decides,
the keeper who runs, and you, who checks. If you take one impression from
the whole course, let it be this: the system was built by people who
assume you will not trust it, and who would rather show you the command
than argue. Meet them there.

## Course complete

Back to [the course index](../README.md), or straight to the
[reproduce script](../../trust/reproduce.md) for one more pass.
