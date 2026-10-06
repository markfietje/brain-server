# The agent loop

The agent loop (`src/agentloop/`, 11 modules) is the kernel-side async driver
that runs one model exchange at a time: it takes caller-owned input, projects
durable session history into a bounded provider request, streams one assistant
turn, executes any tool calls the model asked for, and repeats until the model
stops asking or a bound stops the loop. It is the engine inside the Solve
stage's agentic crank (see [Architecture](./architecture.md) §Where each stage
is implemented) — not a route, not a policy, not a human.

> **No routes.** `src/agentloop/mod.rs` states it plainly: the loop ships no
> wire tables of its own; a loop-serving route ships its wire tables in its
> own release. The one production route that drives this loop is
> `POST /workflow/cases/{id}/gdl` (`src/handlers/case_run.rs`). The
> `/workflow/delivery/*` family (e.g.
> `GET /workflow/delivery/runs/{id}/trace`,
> `GET /workflow/delivery/runs/{id}/replay-verify`,
> `POST /workflow/delivery/runs/{id}/advance`) belongs to the delivery loop, a
> different axis — it does not drive, read, or observe this loop's
> conversation. (The delivery trace appendix once read the shared session
> table and no longer does; see Limits.)

## What the agent loop is and is not

**It is:**

- The five-step driver in `src/agentloop/run_loop.rs`: input → context →
  stream → tool-exec → loop. Each provider call is one harness turn
  (`start_run` snapshots config → stream → `message_end` persists → 
  `finish_run` settles and audits `RunEnd`).
- The owner of the loop's bounds: at most `DEFAULT_MAX_TURNS = 8` provider
  turns per exchange, tool output truncated to `TOOL_OUTPUT_CAP = 16 KiB`
  before it enters history or context, tool wall-clock cut at `TOOL_TIMEOUT
  = 30 s` and surfaced to the model as a tool error.
- The single provider-dispatch seam (`admitted_stream`): one root-owned
  dispatch permit shared by every view and child, short ledger admission
  under the budget mutex, cancellation rechecked immediately before the
  `provider.stream` call. No alternate dispatch path exists.
- The durability story: the session narrative lands in `agent_session_events`
  (append-only, audited in-tx, exactly-once by idempotency key) while the
  harness queue drains into the outbox — two consumers, two tables, one
  audit chain. A terminal exchange receipt certifies both writes completed.

**It is not:**

- A provider. The provider is a pluggable seam (`LlmProvider` in
  `src/agentloop/provider.rs`): constructor-injected, channel-based,
  and **dropping the receiver is the cancel** — no detached producer can
  outlive a cancelled turn. Zero provider code ships in-tree for selection;
  tests ride the loopback fixture (`LoopbackProvider`, scripted turns, never
  a production provider). The one real egress is the HTTP adapter
  (`src/agentloop/provider_http.rs`); see Gates.
- A shell, a filesystem, or a process spawner. The loop never spawns a
  process or opens a file itself. Tool execution rides the SDK registry over
  an injected `ExecutionEnv`; the only process path is the mediated `exec`
  bridge (`src/agentloop/exec.rs`), which takes an argv **array** (no shell)
  through `hostcalls::build` mediation. The operator allowlist
  `BRAIN_ENGINE_EXEC_ALLOWLIST` is the trust anchor and empty means deny-all,
  fail-closed.
- A rewriter of history. Compaction appends one `compaction` event; rows are
  referenced, never mutated. Context assembly reshapes around the latest
  boundary.
- A human. Nothing in this directory approves, publishes, or resolves. On
  close the machine emits proposals; humans dispose. See Gates and human
  seams.

## Lifecycle / phases

One exchange (`run_turns_keyed`, with caller-persisted request keys for
explicit retries; convenience `run_turns` always mints a new exchange):

1. **Claim and admit.** `loop.before_start` policy runs before the claim and
   the invocation row. Then the case claim is acquired, the harness turn
   opens, and `loop.before_input` policy runs after the claim gate but
   before admission and any provider call. Admission is idempotent by
   request key: an exact retry returns the stored receipt — it dispatches
   nothing, debits nothing, reserves nothing.
2. **Context.** Replay (capped at `session_log::REPLAY_CAP = 500` rows,
   `PAYLOAD_CAP_BYTES = 64 KiB` per payload) is projected by
   `src/agentloop/context.rs` (`scoped-context-v2`) into the closed
   `ChatMessage` vocabulary: user, assistant (with re-keyed tool calls
   `e{exchange}:t{turn}:c{index}`), tool results, delegation summaries,
   compaction summaries. Complete tool groups only — a partial group, an
   orphan result, a mismatch, or a duplicate refuses loudly. `control:*`
   rows and `canceled` markers are not conversation; a `compaction` row in
   the projected window refuses (`CompactionBoundary`) so callers must route
   through `reconstruct`/`admit`, never drop the boundary to bypass it.
3. **Stream.** The assembled `ProviderRequest` (system prompt, messages,
   tools — value-typed, provider-neutral) is size-checked (`REQUEST_CAP =
   1 MiB`) and streamed as typed deltas. One provider call may overshoot a
   budget reservation; the actual usage still counts.
4. **Tool-exec.** Each requested call is journaled as `control:tool_intent`,
   executed via the registry under the loop's `ExecutionEnv`, truncated to
   the output cap, and appended as `tool_result` plus `control:tool_done`.
   New calls validate codec and shape only (`validate_calls`) — syntax, not
   authorization; the registry and capability gate still decide what may run.
5. **Loop or settle.** Tool-free assistant turn → `Completed`. Turn cap →
   `TurnCapReached`. Token ceiling crossed → `BudgetExceeded`. Cancellation
   → `Canceled` (the harness is aborted on the same settlement path as
   finish; a `canceled` session event is appended). Provider failure after
   admission → `ProviderFailed`, durably finalized before the caller sees
   it. A partial write retains the claim and **refuses retry** — there is no
   automatic projection repair or tool replay.

The outcome vocabulary (`RunOutcome`) is terminal and typed; `LoopError`
(`Harness` / `Provider` / `Persist` / `Hook`) is for infrastructure and
contract breaches only. Cancellation and the turn cap are outcomes, not
errors.

Compaction rides the loop top at the Idle boundary (`just_before_call`,
all-on by default with `tool_result_clearing` and `selective_retention`):
pressure is the SDK's own numbers (compact at ≥ 16k window tokens, keep
~20k verbatim), the summary is produced by the loop's own provider under a
dedicated system prompt, and the result commits as one event with a
sequence manifest. Each committed compaction is then probed as an
experiment (`src/agentloop/compaction_probes.rs`): 10 deterministic lexical
probes, integer per-mille arithmetic, degradation at ≥ 500‰ lost latches
the conservative posture — no further auto-compaction this episode. A weak
baseline (fewer than half the probes hitting pre-compaction) is reported as
no signal, never as degraded. At most `max_events_per_episode = 16`
compactions per episode; past that the loop stops loudly at the
budget-exhausted terminal with a named session-log row.

Delegation (`src/agentloop/subagents.rs`) is a child `LoopDriver` over the
same host and the same run: same audit chain, kind-prefixed narrative
(`child:<name>:`), one parent-visible `subagent_result` event. The child
gets a **narrowed** environment (capability subtraction, never addition —
the process grant dies on a disjoint command ask) and a subset of the
parent's tools. Budgets reserve from the root only (depth-bounded; deeper
nesting refuses structurally), and a started call that ends without
`MessageEnd` marks the shared authority accounting-incomplete and refuses
further dispatch rather than inventing a number. There are no nested
fibers and no parallel identity — the child rides the parent's principal.
Outcomes mirror the loop's: `Completed` / `BudgetExceeded` / `Capped` /
`Canceled` / `ProviderFailed`.

## Gates and human seams

Three policy boundaries, all constructor-injected (`LoopHooks`), never
env-driven, each denied loudly with one coarse audit row and no payload
echo: `loop.before_start` (before claim and invocation row),
`loop.before_input` (after claim, before admission/provider — deny retains
the claim), `loop.before_compaction` (only when a cycle is genuinely
pending; deny skips it and pressure re-evaluates next exchange). Dispatch
runs under a deny-closed 500 ms deadline (`HOOK_DEADLINE`); expiry denies,
a late verdict can never be applied, listener panics are contained (counted,
never a denial, never a bypass), and deny reasons are bounded to 160 chars.
With no operator policy supplied the driver is built `pass_through()` —
an empty registry whose waterfall is `Ok`.

The model-facing boundaries are equally explicit. Context shaping masks PII
unconditionally before the read seam and refuses credential tripwires and
suspicious patterns — but detection is bounded markers, not a scanner, and
framing does not guarantee model obedience (see Limits). The HTTP provider
adapter requires HTTPS, screens the endpoint for SSRF with DNS pinning,
follows no redirects, retries nothing, keeps key material on the
server-owned root-confined secret path, and carries the declared sampling
contract (`temperature: 0.0`) on every request — refused before the send if
absent, which buys attribution ("the contract was honoured; upstream
moved"), not determinism.

Humans enter at the edges this loop deliberately leaves open:

- **Launch is operator-only.** `POST /workflow/cases/{id}/gdl` launches one
  episode on a fresh `troubleshoot` run with a bounded `{ticket}` body
  only; caller-selected provider fields get `400 gdl_request_migrated`.
  Provider destination, model, and secret are server-owned via
  `BRAIN_GDL_PROVIDER_BASE_URL`, `BRAIN_GDL_PROVIDER_MODEL`,
  `BRAIN_GDL_PROVIDER_SECRET_FILE`, and `BRAIN_GDL_PROVIDER_SECRET_ROOT`.
  JWT callers need domain Write plus the `workflow` role; role-less JWTs,
  unknown roles, and `agent@loopback` bearers are refused before any
  secret, DNS, or provider work (era-pin: v1.29.0, 2026-09-25, "GDL
  boundary, governed decisions, and model identity").
- **Stuck is human-routed, not machine-resolved.** The workflow driver
  carries the `AskHuman` stop verdict (`src/workflow/driver.rs`), and the
  operator answers through `POST /workflow/runs/{id}/handoff/decision` and
  `POST /workflow/runs/{id}/back-referral/return` (era-pin: v1.28.92,
  2026-09-22).
- **Close proposes; only the gate disposes.** On resolve the loop enqueues
  capture proposals on the pending `/proposals` queue
  (`capture_proposals_on_resolve`) — proposals only, never publication.
  What meaningful control over those proposals looks like is the subject of
  [Human in the loop](./human-in-the-loop.md): comprehensibility,
  reviewability, actionability, consequentiality. The promotion path's
  disabled-by-construction posture is documented in [The create
  loop](./create-loop.md) — a different loop, but the same moral: an
  unmeasured gate presented as a safety property is a claim nobody has
  demonstrated. For the team habits that keep proposals and domains
  trustworthy (write-location conventions, review as gate), see [One Brain
  for the Whole Team](./team-workflow.md).

## How to operate / observe it

- **Configure the provider profile or run deterministic.** All four
  `BRAIN_GDL_PROVIDER_*` variables set together, or none. Partial profiles
  refuse bootstrap; readiness reports `gdl_provider:
  disabled|configured|invalid` with no URL, path, model, or credential
  retained. A deployment that never configures the profile runs the
  deterministic posture only. `BRAIN_ENGINE_EXEC_ALLOWLIST` governs the exec
  bridge independently: unset or empty denies all exec.
- **Launch and read the terminal.** `POST /workflow/cases/{id}/gdl` with
  `{"ticket": "…"}` (1–8192 bytes after trimming). A provider failure after
  admission is durably terminal: the first launch returns `503
  gdl_provider_failed`, a later launch against that run returns `409`
  without replaying provider work. The request carries a 25-second total
  body deadline and receiver cancellation drops the in-flight HTTP future.
- **Verify, don't re-run.** The audit chain verifies offline
  (`verify_chain` — mediated exec writes land in the same chain the loop
  writes). Session history replays from `agent_session_events` via
  `session_log::replay`. Delivery's `replay-verify` comparator is the model
  for this posture: it recomputes addresses from stored columns and never
  re-runs a model — and it reads `delivery_traces`, not the conversation
  log.
- **Watch the meters.** Every provider call is token-metered (per-class
  telemetry is recorded adjacent to the call in `admitted_stream`, so the
  counter and the guard read off one place); deployments surface this on
  `/metrics`. Hook denies and steers land as coarse audit rows
  (`loop.before_start` / `loop.before_input` / `loop.before_compaction`,
  fixed kernel reasons only). Compaction commits carry their probe report
  (`control:compaction_probe`: probes, pre/post hits, lost-per-mille,
  degraded) beside the summary event.
- **Inspect the assembly, not the environment.** The `ctx.*` service tree
  (`ctx.tools`, `ctx.llm`, `ctx.sessions`, `ctx.systemPrompt`,
  `ctx.compaction`, `ctx.sandbox`, `ctx.agents`, `ctx.agentLoop`,
  `ctx.evidence`, `ctx.scoring`; `web` vs `headless` profiles in
  `src/agentloop/services.rs`) renders through a pure, env-blind inspector:
  profile name, mounted keys, scalar config, provider **name** only —
  `sandbox: unavailable (denied)`, always.

## Honest limits

- **The loop is unprivileged by construction, and that is load-bearing.**
  Deny-all scoped envs, empty tool registries (an unknown tool is a loud
  refusal), capability subtraction on delegation, fail-closed exec. Any
  deployment that widens these to "make the demo work" has left the
  documented posture — say so out loud.
- **Provider output is untrusted input.** Masking, tripwires, framing, and
  the sampling contract raise the cost of misuse; none of them prove safety
  or obedience. Encoded or unmarked secrets are an explicit ceiling of the
  marker-based tripwire, and a confident summary is unverified prose until a
  human or a check says otherwise.
- **Budgets bound dispatch, not physics.** One in-flight provider call may
  overshoot its reservation; unknown spend (a call ending without
  `MessageEnd`) refuses the whole authority rather than guessing. The turn
  cap, token budget, output cap, request cap, payload cap, and compaction
  quota are stops, not guarantees about what happens before the stop.
- **No automatic repair.** A partial write keeps the claim and refuses
  retry; there is no projection repair, no tool replay, no silent fallback
  provider, no in-seam retry loop. A stuck claim is an operator problem
  with an audit trail, not a self-healing system.
- **Compaction forgets on purpose.** A summary preserves decisions, open
  questions, and intent — it does not preserve evidence bytes. Retrieval
  against compacted history is measured per-compaction by the probes, and a
  degraded window latches conservative for the episode; but the probes are
  lexical, local, and integer — a faithful-looking summary that drops
  meaning without dropping tokens is outside what they can see.
- **History note (era-pinned, v1.29.2 tree).** The GDL launch boundary and
  provider/secret hardening closed in v1.29.0 (2026-09-25); the exec OS
  boundary and the human handoff-decision routes landed in v1.28.92
  (2026-09-22). Anything in this file that a newer round has moved is
  wrong — correct this page when the tree moves, never the other way round.
