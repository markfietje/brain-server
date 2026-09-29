//! steward-harness 0.2.0 "FirstLight" — the governed-loop ENGINE.
//!
//! The harness is a binary-side consumer of two stable seams: the SDK's
//! [`WorkflowHost`](brain_engine_sdk::host::WorkflowHost) storage ABI and
//! troubleshoot-core's pure kernels. It contains no storage and no server
//! logic: every durable effect rides the server's substrate projections
//! (`POST /workflow/runs`, CAS state, outbox events). Transport concerns live
//! HERE, never in the SDK — the ABI rule holds.
//!
//! Loop law: the crank is request-scoped and HUMAN-CRANKED (no background
//! worker); steering drains are recorded in `state.steering_log[]` as a LOG
//! that no decision reads (`decide` consults only `status`,
//! `pending_question`, `next_step`, and `next_state`) — a gate rejection
//! becomes a finding row, never a silent skip; budgets bound every turn
//! (`MAX_STEPS_PER_TURN`, ceiling 1000) and crossing 80% of the turn's step
//! budget is a REAL STOP (`StoppedAt::BudgetWarn`), not a flag.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod effects;
pub mod engine;
pub mod inmem;
pub mod remote_host;
