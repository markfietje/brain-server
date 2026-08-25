# The loop runs: what it means for an engine to ask permission

*2026. v1.28 in four acts — FirstLight, Anvil, Settle, Relay: an autonomous engine that opens a run, mediates every tool-effect through one auditable door, settles exactly where it said it would, and now hands the run to a colleague under the same law.*

For two years the answer to "can an agent change its own memory?" was *no, a
human approves that*. v1.28 answers the harder follow-up: **what happens when
an agent needs to *work* — multi-step, tool-using, state-changing work — without
becoming an unaccountable process?** The answer shipped in four acts, and
none of them is "trust the model."

## Act I: the loop is real (FirstLight, v1.28.15)

A workflow engine existed on paper before it existed on the wire: routes
declared, an SDK seam defined, a stub echoing `{"ok":true}`. FirstLight
replaced the stub with a real governed loop over **role-gated HTTP routes**:

- Opening a run, advancing its state (CAS, `409 {actual_revision}` on stale),
  enqueueing events (exactly-once by idempotency key), and draining advisory
  steering all require the `workflow` role. Answering an AskHuman question
  requires `approve`. No role, no route — deny-by-default, not policy-doc-by-default.
- **AskHuman binds to the live bytes**: an answer carries the SHA-256 digest of
  the pending question; drift between what the engine showed and what the human
  answered → rejection, run untouched.
- Open + audit row commit in one transaction. A transition whose audit row
  fails rolls back with it — the chain cannot lag the state.

The honest part: the engine is human-cranked (`brain workflow crank`). No
background worker, no autonomy by accident. Agency is granted one crank at a
time, which is precisely how you want to meet it the first time.

## Act II: every tool-effect crosses one door (Anvil, v1.28.16)

An engine that can't act is a spreadsheet. An engine that can act unmediated is
a liability. Anvil closes the gap: all seven hostcall kinds resolve to a
handler — an explicit mediation or an explicit refusal, never an absence.

- **`exec`**: argv-only, no shell, pinned working directory, per-stream output
  caps, a hard time bound — and the operator allowlist is empty by default,
  which means **deny ALL exec until an operator names the binaries**.
- **`http`**: egress is deny-by-default. Destination hosts must be allowlisted;
  remote destinations speak HTTPS only; redirects are refused.
- **`events`**: the outbox is the only event door, `workflow/*` topics only,
  bounded payloads, idempotency keys required.
- **`ui`**: a named refusal ("reserved"), so the vocabulary stays closed and
  silence is never ambiguous.

Every canonicalized dispatch tallies into a per-run counter — denials count
too — and every refusal audits. If an engine tried something, the chain says
so even when the engine says nothing.

## Act III: settlement is law, not hope (Settle, v1.28.17)

Long-running loops die mid-flight — cancelled, killed, out of budget. Settle
pins what happens *then*:

- **Budget enforcement fails closed**: an exhausted window or an unenforceable
  budget denies the dispatch (`BudgetExceeded`) *before any handler runs*.
  Previously that guard could never fire; now it is the law and it is tested.
- **Cancel settles between steps**, never mid-step, never splitting a CAS/event
  twin into half a state change.
- **Event keys derive from persisted step count** — this fixed a real bug: a
  cancelled-then-resumed run re-keyed events from 1, and the exactly-once gate
  silently swallowed every resumed step's event twin. Exactly-once that breaks
  on resume isn't exactly-once; now it is, and a conformance test proves it.

## Act IV: the loop has colleagues, and a lawful way to leave them (Watchbill/Crew/Relay, v1.28.25–.27)

A single-crank loop proved agency could be governed. The follow-the-sun line asked the harder operational question: **what happens when the human half of the loop changes at a shift boundary?** Three releases answered it with data and gates, not hope.

- **Watchbill (.25) made the schedule first-class**: one row per site's on-call window, the handover overlap window *derived* from each shift pair at read time — no scheduler daemon. At the boundary the queue re-scopes to the incoming site while open runs keep their envelopes: the queue follows the sun, cases don't.
- **Crew (.26) made the people visible without a heartbeat**: presence rides the caller's own transaction — every mutating act is its beacon, a rolled-back transition leaves no ghost. Skills tags are proposal-gated (agents cannot self-tag), and the DPO switch fails **open to hidden**: an unreadable config means an empty roster, never more visibility than configured.
- **Relay (.27) closed the loop's exit**: `POST /handover/offer` refuses unless the I-PASS packet answers the five questions the receiving team needs (the refusal carries the MISSING list — the machine coaches the protocol); acceptance CAS-transfers ownership in the same transaction as the receipt, never touching the SLA clock; decline requires a screened reason. Offer, decision, and audit land in ONE transaction — a handover that can't write its audit row doesn't happen.

The honest ceiling, stated in the changelog and repeated here: packet completeness reads the stored *shape* — a run can carry a complete-looking packet that is substantively empty. The gate enforces the protocol's form; judgment stays human.

## Why this wins the review

- **Auditable agency.** Every state change, every tool-effect, every handover, every refusal: one hash-chained trail your auditor can replay. "What did the agent do?" has a query, not a folklore answer.
- **Fail-closed by construction.** Missing role, empty allowlist, exhausted budget, wrong digest, incomplete packet, missing decline reason — every gate denies loudly. Nothing defaults to yes.
- **The autonomy dial is explicit.** Crank-by-crank today; the mediation doors mean wider autonomy later doesn't require new trust, just new grants — and follow-the-sun now works because *handing agency to a different human* is as governed as exercising it.

**The takeaway:** trustworthy automation isn't a model with guardrail prompts.
It's a loop whose every effect is mediated, counted, audited, and settled on
terms the *operator* wrote down first.

*Mechanism detail lives in the [API reference](../api.md) (workflow routes +
hostcall mediations) and [`SECURITY.md`](../../SECURITY.md); the proof walk-through is
[`docs/trust/proof-map.md`](../trust/proof-map.md).*
