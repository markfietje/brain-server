# Steward Harness — Governed-Loop Run Page

Era-pin: `steward-harness` manifest **0.2.2** (`tools/steward-harness/Cargo.toml`);
`lib.rs` header comment still reads **0.2.0 "FirstLight"**
(`tools/steward-harness/src/lib.rs`); server **1.29.2** (`Cargo.toml`).
Read and written 2026-10-06. Where this page and a header comment disagree,
the manifest wins and the drift is named, not smoothed over.

This page complements the 3-line engine paragraph in [api.md](./api.md)
("The engine itself lives in `tools/steward-harness` … No engine code runs
in the server") with the operational half: how to crank the loop, what comes
out, how to check it, and where it stops. For the command table see
[cli-reference.md](./cli-reference.md); for the storage ABI see
[engine-sdk.md](./engine-sdk.md); for env wiring see
[configuration.md](./configuration.md).

## What it drives, and what "human-cranked" means

`tools/steward-harness` is the governed-loop **driver**, not the store and
not the server. It implements `load_state → decide → act`, one governed step
at a time (`tools/steward-harness/src/engine.rs`), over the SDK
`WorkflowHost` seam (`crates/brain-engine-sdk/src/host/mod.rs`:
`load_state`, `cas`, `enqueue`, `tx`).

Over the wire (`tools/steward-harness/src/remote_host.rs`) that seam is the
server's workflow substrate routes:

- `POST /workflow/runs` — open a run (`open_run`)
- `GET /workflow/runs/{id}/state` — load `(state_json, revision)`
- `PUT /workflow/runs/{id}/state` — CAS advance (`expected_rev` + `state_json`;
  `409` → stale, reported not panicked)
- `POST /workflow/runs/{id}/events` — outbox emission with idempotency key
- `POST /workflow/runs/{id}/answer` — answer the pending AskHuman question
- `GET /workflow/runs/{id}/steering` — steering-log drain (log only, see below)

Routing inside a turn follows `brain_engine_sdk::decide`
(`crates/brain-engine-sdk/src/workflow_state.rs`): `status` terminal →
`Done`; `pending_question` set → `AskHuman`; `next_step` named → `RunStep`;
`next_state` present → `Advance`; otherwise `Done`.

**Human-cranked, operationally:** there is no background worker, no
scheduler, no daemon thread. A run advances only when a human (or a
role-checked relay of a human, e.g. the bridge-console `crank` verb in
`src/handlers/channel_webhook.rs`) invokes one bounded crank turn. Each turn
is request-scoped, runs at most `max_steps` steps, stops at the first stop
condition, checkpoints the boundary, and hands the run back. A run that needs
more work needs another crank. `brain workflow crank` is the CLI form of that
act; the bridge console `crank` is the same harness binary behind a
role-checked relay (`resolve_harness_bin`, bounded steps, one timeout
window).

Two further laws, both load-bearing:

- **Every durable effect rides the host seam.** CAS persist + outbox event;
  a crash between any two effects replays exactly once by idempotency key
  (`run-{id}-evt-{n}`). Tool effects (`exec` argv-only behind an operator
  allowlist, `http` deny-by-default egress, `events` via outbox) cross only
  the mediated dispatch door (`tools/steward-harness/src/effects.rs`) —
  transport (`reqwest`) lives solely in `remote_host.rs`, pinned by the
  `engine_has_no_direct_effect_paths` test.
- **Steering is a LOG, never a binding channel.** Drained messages append to
  `state.steering_log[]`, which `decide` never reads (it consults only
  `status`, `pending_question`, `next_step`, `next_state`).
  `SteeringReader` is a separate opt-in trait; the storage ABI is untouched.

## Run procedure

Prerequisites: a running server and a resolvable bearer token. The harness
resolves both the same way the CLI does
(`tools/steward-harness/src/remote_host.rs`, [configuration.md](./configuration.md)):

- Base URL: `BRAIN_URL`, default `http://127.0.0.1:8765`. Non-loopback
  plain-HTTP is **refused** (`resolve_base_url`); use `https://` off-host.
- Token ladder: `BRAIN_TOKEN_FILE` → `BRAIN_TOKEN` →
  `~/.config/brain-server/auth-token`.
- Harness binary resolution (CLI, `src/bin/brain.rs` `cmd_workflow`): binary
  beside `brain`, then `BRAIN_STEWARD_BIN`, then `PATH`. The server-side
  console crank instead **requires an absolute** `BRAIN_STEWARD_BIN`
  (relative refuses; PATH never consulted).
- Turn budget: `BRAIN_MAX_STEPS` env → default **24**, ceiling **1000**
  (`crates/brain-troubleshoot-core/src/kernel.rs`: `MAX_STEPS_PER_TURN`,
  `MAX_STEPS_CEILING`, `clamp_max_steps`). Checkpoint cadence:
  `BRAIN_CHECKPOINT_EVERY` → default **25**, clamped **1..=100**
  (`resolve_checkpoint_every` in `src/engine.rs`).

Step-by-step (CLI form):

```bash
# 1. Open a run (kind is "troubleshoot"; domain defaults to "global")
brain workflow open global

# 2. Crank it — one bounded turn (observed CLI form: `crank <run> [steps]`)
brain workflow crank <run> [steps]
# prints: crank run <run>: stopped_at=<…> steps_executed=<n>

# 3a. If it stopped at ask_human, read the question then answer
# (answer is digest-bound to the LIVE pending_question; empty answers refuse)
brain workflow status <run>
brain workflow answer <run> <text>

# 3b. If it stopped at budget / budget_warn, re-crank (same run, larger budget)
brain workflow crank <run> [steps]

# 4. Repeat 2–3 until stopped_at=done; then read the handoff packet
brain workflow handoff <run>
```

Direct-RPC form (same binary, `src/main.rs`): the harness speaks
line-delimited JSON over stdin/stdout. Real verbs: `open-run {domain,
seed?}`, `crank {run_id, run_kind?}`, `ask-human {run_id, answer, digest}`,
`step-result {run_id, expected_rev, state_json}`, `advance {run_id,
next_state}`. `run_kind` is `"live"` (default, fail-closed) or `"replay"`;
anything else is refused. Note for the careful reader: the CLI's crank line
sends a `max_steps` field, but `main.rs` resolves the turn budget from
`BRAIN_MAX_STEPS` env — set the env var if you want a non-default budget.

The harness test lane (no server needed — `InMemHost` in `src/inmem.rs`
carries real CAS revision accounting and key-idempotent outbox semantics):

```bash
cargo test --manifest-path tools/steward-harness/Cargo.toml
```

## Artifacts a run emits

One crank turn returns a `CrankReport` (`src/engine.rs`), echoed as JSON by
the RPC `crank` verb:

- `stopped_at` — one of `ask_human`, `done`, `budget`, `cancelled`,
  `stale` (carries the host's actual revision), `budget_warn` (the 80%
  iteration threshold — a REAL STOP, checkpointed at the step boundary),
  `gates_vacuous` (a `Live` turn whose gates evaluated on nothing — refused
  `Done`, see below).
- `steps_executed`, `warn_threshold_fired`, `hostcalls` (`"<label>/<kind>"
  → count`, additive JSON; the audit chain is the durable count).
- The vacuity census: `gates_declared` (how many of the five declared keys —
  `evidence_refs`, `required_evidence`, `mutations`, `supporting_lines`,
  `needs_approval` — were PRESENT on this turn's queue items), 
...[1512 chars]