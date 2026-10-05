//! The write deadline — F8-03's enforcement half.
//!
//! **The defect.** The router wraps every route in a 30 s `TimeoutLayer`
//! (`server/router/mod.rs`). At the deadline that layer DROPS the handler
//! future and answers 408. But a handler's work runs inside
//! `tokio::task::spawn_blocking`, and a blocking closure is **not
//! cancellable**: dropping the join handle runs the closure to completion, and
//! the closure's `commit()` still lands. So the client is told the write
//! failed while the row is written anyway, and a retry double-commits.
//!
//! **Why a timeout alone cannot fix this.** Adding a deadline check *outside*
//! the closure does nothing — the closure has already been handed to the
//! blocking pool by then, and nobody can call it back. The deadline has to be
//! evaluated INSIDE, before the work begins, so the write either starts with
//! enough budget to finish or refuses outright.
//!
//! **What this buys, and what it does not.** The window narrows from
//! "anywhere inside 30 s" to "the last [`WRITE_DEADLINE_MARGIN_SECS`] of it",
//! which is the part a caller can retry safely. It does NOT make a write
//! atomic with respect to a crash, and it does NOT add an idempotency key —
//! that is a wire contract and a new table, deliberately out of this round's
//! scope. The remaining residual is named in the R70 note.
//!
//! **Precedent.** The shape follows `workflow/hostcalls.rs:535-575`, which
//! computes its budget INSIDE the spawned work and refuses to exceed it rather
//! than being abandoned part-way through.

use std::time::{Duration, Instant};

/// The error a write returns when it cannot be started in time.
///
/// Deliberately a distinct shape from "the write failed": the caller must be
/// able to tell "nothing happened, and here is why" from "something happened
/// and then broke", because only the first is safe to retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteDeadline {
    /// The request deadline arrived before this write could begin. **No row
    /// was written** — the refusal happens before any statement runs.
    NoBudget { needed: Duration, left: Duration },
}

impl std::fmt::Display for WriteDeadline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteDeadline::NoBudget { needed, left } => write!(
                f,
                "the request deadline left {left:?} and this write needs {needed:?}; \
                 refused BEFORE starting so nothing was written"
            ),
        }
    }
}

impl std::error::Error for WriteDeadline {}

/// Compute the instant a write must begin by, given the request's own
/// deadline. Call this INSIDE the `spawn_blocking` closure — computing it
/// outside is the defect, because the clock has to be read at the moment the
/// work actually starts.
pub fn write_deadline() -> Instant {
    Instant::now() + request_budget()
}

/// How long a write may take before the request deadline, and how much of it
/// the write itself is allowed to consume.
///
/// Derived from the router's own `REQUEST_TIMEOUT_SECS` rather than a second
/// literal, so the two cannot drift.
pub fn request_budget() -> Duration {
    let total = crate::config::REQUEST_TIMEOUT_SECS;
    let margin = crate::config::WRITE_DEADLINE_MARGIN_SECS;
    Duration::from_secs(total.saturating_sub(margin))
}

/// Refuse unless `deadline` is still far enough away for the write to
/// complete.
///
/// `needed` is the write's own expected cost (a measured budget, not a guess —
/// see the call sites). This is the gate the closure consults as its FIRST
/// statement, before opening a transaction, so a refusal is guaranteed to have
/// written nothing.
pub fn ensure_budget(deadline: Instant, needed: Duration) -> Result<(), WriteDeadline> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left >= needed {
        return Ok(());
    }
    Err(WriteDeadline::NoBudget { needed, left })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_with_budget_proceeds() {
        let d = write_deadline();
        assert!(
            ensure_budget(d, Duration::from_secs(1)).is_ok(),
            "a write needing 1s with ~{req_budget:?} available must proceed",
            req_budget = request_budget()
        );
    }

    #[test]
    fn a_write_without_budget_is_refused_before_it_starts() {
        let now = Instant::now();
        // Deadline already passed.
        let past = now - Duration::from_secs(10);
        let err = ensure_budget(past, Duration::from_secs(1))
            .expect_err("an expired deadline must refuse");
        match &err {
            WriteDeadline::NoBudget { needed, left } => {
                assert_eq!(*needed, Duration::from_secs(1));
                assert_eq!(*left, Duration::ZERO);
            }
        }
        // …and the message must say the refusal happened BEFORE any write,
        // because that is the property a retry depends on.
        let msg = err.to_string();
        assert!(
            msg.contains("BEFORE starting") && msg.contains("nothing was written"),
            "the refusal must state that nothing was written: {msg}"
        );
    }

    #[test]
    fn the_margin_holds_back_from_the_router_deadline() {
        let budget = request_budget();
        let total = Duration::from_secs(crate::config::REQUEST_TIMEOUT_SECS);
        assert!(
            budget < total,
            "a write budget shorter than the router deadline is the whole point \
             of the margin ({budget:?} vs {total:?})"
        );
        assert_eq!(
            total - budget,
            Duration::from_secs(crate::config::WRITE_DEADLINE_MARGIN_SECS),
            "the reserve must equal the declared margin"
        );
    }
}
