//! Loop-extension policy boundaries over the SDK [`Hooks`] registry — the
//! first consumer of `brain_engine_sdk::events`. Three wire names, one per
//! loop boundary: before start, before input, before compaction. Policy is
//! constructor-injected ([`LoopHooks::new`]); [`LoopHooks::pass_through`]
//! builds the explicit policy-free driver used everywhere no operator policy
//! is supplied. There is no env knob and no default registry beyond empty.
//!
//! Policy dispatch ([`LoopHooks::run_policy`]) is the SDK waterfall under a
//! deny-closed deadline: the first deny wins monotonically, a deadline
//! expiry denies, a listener panic is contained by the registry (counted,
//! never a denial, never a bypass), and a verdict arriving after the deadline
//! is discarded — it can never be applied. Steering ([`LoopHooks::steer_input`])
//! is ordered mutation of the pending input, refused loud if the result
//! would exceed the session payload cap.
//!
//! Declared ceiling: a listener that ignores its deadline keeps a thread in
//! tokio's bounded blocking pool occupied until it returns — the deadline
//! bounds the LOOP's wait, never the listener's lifetime (tokio's own
//! guidance: `spawn_blocking` is for bounded, short-lived work).

use std::any::Any;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use brain_engine_sdk::events::Hooks;

/// Owned-entry policy: fires before the claim and the invocation row.
pub(crate) const LOOP_BEFORE_START: &str = "loop.before_start";
/// Exchange policy: fires after the claim gate, before admission/provider.
pub(crate) const LOOP_BEFORE_INPUT: &str = "loop.before_input";
/// Compaction policy: fires only when a compaction cycle is actually
/// pending, before the structural gate and any summary provider call.
pub(crate) const LOOP_BEFORE_COMPACTION: &str = "loop.before_compaction";

/// Bounded wait for one policy dispatch; the loop never blocks past this.
pub(crate) const HOOK_DEADLINE: Duration = Duration::from_millis(500);

/// Fixed reason for a deadline expiry — kernel-owned vocabulary, reused as
/// the durable audit detail so the audit chain never carries listener text.
pub(crate) const HOOK_DEADLINE_REASON: &str = "hook deadline exceeded";

/// A listener's deny reason is diagnostics, never content: bounded before it
/// rides the error face, so no input or payload echo can hide in it.
const REASON_CAP: usize = 160;

fn bounded(reason: String) -> String {
    if reason.len() <= REASON_CAP {
        return reason;
    }
    let mut cut = REASON_CAP;
    while !reason.is_char_boundary(cut) {
        cut -= 1;
    }
    reason[..cut].to_string()
}

/// Payload for [`LOOP_BEFORE_START`]: identifies the run entry, nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BeforeStart {
    pub(crate) run_id: i64,
    pub(crate) owner: String,
}

/// Payload for [`LOOP_BEFORE_COMPACTION`]: the token pressure of the cycle
/// the policy is being asked to allow or skip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BeforeCompaction {
    pub(crate) run_id: i64,
    pub(crate) pressure_tokens: usize,
}

/// A refused policy evaluation — deny-closed posture: deadline expiry, a
/// listener deny (bounded reason), or a dispatch failure all land here. The
/// loop maps this onto `LoopError::Hook` and the durable audit row; the
/// audit detail stays a fixed kernel string, never `reason`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HookDenied {
    pub(crate) event: &'static str,
    pub(crate) reason: String,
}

impl fmt::Display for HookDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.event, self.reason)
    }
}

impl std::error::Error for HookDenied {}

/// The kernel's handle on the SDK registry: an `Arc` plus the pinned
/// dispatch deadline. `Clone` shares one registry across driver clones.
#[derive(Clone)]
pub(crate) struct LoopHooks {
    hooks: Arc<Hooks>,
    deadline: Duration,
}

impl LoopHooks {
    /// Wrap a registry under the pinned deadline (C1: 500 ms, named const).
    pub(crate) fn new(hooks: Arc<Hooks>) -> Self {
        Self {
            hooks,
            deadline: HOOK_DEADLINE,
        }
    }

    /// Explicit policy-free construction: an empty registry (a waterfall
    /// with zero listeners is `Ok`). Never env-driven.
    pub(crate) fn pass_through() -> Self {
        Self::new(Arc::new(Hooks::new()))
    }

    #[cfg(test)]
    fn with_deadline(hooks: Arc<Hooks>, deadline: Duration) -> Self {
        Self { hooks, deadline }
    }

    /// The shared policy runner (C3): the registry's waterfall for `event`
    /// over `payload`, wrapped in the deny-closed deadline. On an allow the
    /// dispatch report is re-emitted LIVE on the same event name for
    /// observation — in-memory only, never a system of record.
    pub(crate) async fn run_policy<E>(
        &self,
        event: &'static str,
        payload: &E,
    ) -> Result<(), HookDenied>
    where
        E: Any + Clone + Send + 'static,
    {
        let hooks = Arc::clone(&self.hooks);
        let owned = payload.clone();
        let dispatched = tokio::time::timeout(
            self.deadline,
            tokio::task::spawn_blocking(move || hooks.waterfall(event, &owned)),
        );
        match dispatched.await {
            // Late verdicts are discarded with the dropped handle: the deny
            // already happened and nothing can apply the straggler.
            Err(_elapsed) => Err(HookDenied {
                event,
                reason: HOOK_DEADLINE_REASON.into(),
            }),
            Ok(Ok(Ok(report))) => {
                self.hooks.emit(event, &report);
                Ok(())
            }
            Ok(Ok(Err(reason))) => Err(HookDenied {
                event,
                reason: bounded(reason),
            }),
            Ok(Err(join)) => Err(HookDenied {
                event,
                reason: bounded(format!("hook dispatch failed: {join}")),
            }),
        }
    }

    /// Ordered steer of the pending input (C5): registration-order mutators
    /// over the exchange input. A mutator panic is contained by the registry
    /// and skips only that mutator. A steer that pushes the input past the
    /// session payload cap is refused loud — never silently truncated. The
    /// CALLER decides whether an application happened by comparing content.
    pub(crate) fn steer_input(&self, input: &mut String) -> Result<(), HookDenied> {
        self.hooks.serial(LOOP_BEFORE_INPUT, input);
        if input.len() > crate::workflow::session_log::PAYLOAD_CAP_BYTES {
            return Err(HookDenied {
                event: LOOP_BEFORE_INPUT,
                reason: format!(
                    "steered input exceeds payload cap ({})",
                    crate::workflow::session_log::PAYLOAD_CAP_BYTES
                ),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use brain_engine_sdk::events::{HooksError, Verdict};
    use std::sync::atomic::{AtomicBool, Ordering};

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn hooks_with(
        event: &'static str,
        f: impl Fn(String) -> Verdict + Send + Sync + 'static,
    ) -> LoopHooks {
        let hooks = Hooks::new();
        hooks.on::<String, _>(event, "test", f).unwrap();
        LoopHooks::new(Arc::new(hooks))
    }

    #[test]
    fn deadline_expiry_denies_and_the_late_verdict_is_discarded() {
        let finished = Arc::new(AtomicBool::new(false));
        let listener = Arc::clone(&finished);
        let registry = Hooks::new();
        registry
            .on::<String, _>(LOOP_BEFORE_INPUT, "slow-allow", move |_| {
                std::thread::sleep(Duration::from_millis(150));
                listener.store(true, Ordering::SeqCst);
                Verdict::Allow
            })
            .unwrap();
        let pinned = LoopHooks::with_deadline(Arc::new(registry), Duration::from_millis(20));
        rt().block_on(async move {
            let verdict = pinned
                .run_policy(LOOP_BEFORE_INPUT, &"slow allow".to_string())
                .await
                .expect_err("a deadline expiry must deny");
            assert_eq!(verdict.reason, "hook deadline exceeded");
            assert_eq!(verdict.event, LOOP_BEFORE_INPUT);
            // The listener DID finish — its Allow came after the decision
            // and could not flip it: the verdict was discarded.
            for _ in 0..100 {
                if finished.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(finished.load(Ordering::SeqCst));
        });
    }

    #[test]
    fn panic_is_contained_never_a_denial_and_never_a_bypass() {
        let panicky = hooks_with(LOOP_BEFORE_INPUT, |_| -> Verdict {
            std::panic::panic_any("listener blew up")
        });
        // Panic alone: contained, counted — the run continues (no denial).
        rt().block_on(async {
            assert!(
                panicky
                    .run_policy(LOOP_BEFORE_INPUT, &"x".to_string())
                    .await
                    .is_ok()
            );
        });
        // Panic BEFORE a deny listener: the deny still runs — a panic can
        // never bypass policy.
        let hooks = Hooks::new();
        hooks
            .on::<String, _>(LOOP_BEFORE_INPUT, "chaos", |_| -> Verdict {
                std::panic::panic_any("boom")
            })
            .unwrap();
        hooks
            .on::<String, _>(LOOP_BEFORE_INPUT, "gate", |_| Verdict::Deny("no".into()))
            .unwrap();
        let gated = LoopHooks::new(Arc::new(hooks));
        rt().block_on(async {
            let denied = gated
                .run_policy(LOOP_BEFORE_INPUT, &"x".to_string())
                .await
                .expect_err("the deny after the panic must stand");
            assert_eq!(denied.reason, "no");
        });
    }

    #[test]
    fn zero_listener_pass_through_is_allow() {
        let hooks = LoopHooks::pass_through();
        rt().block_on(async {
            assert!(
                hooks
                    .run_policy(
                        LOOP_BEFORE_START,
                        &BeforeStart {
                            run_id: 1,
                            owner: "o".into(),
                        }
                    )
                    .await
                    .is_ok()
            );
            assert!(
                hooks
                    .run_policy(
                        LOOP_BEFORE_COMPACTION,
                        &BeforeCompaction {
                            run_id: 1,
                            pressure_tokens: 10,
                        }
                    )
                    .await
                    .is_ok()
            );
            let mut input = "x".to_string();
            assert!(hooks.steer_input(&mut input).is_ok());
            assert_eq!(input, "x");
        });
    }

    #[test]
    fn deny_listener_refuses_with_bounded_reason() {
        let long = "d".repeat(400);
        let hooks = Hooks::new();
        hooks
            .on::<BeforeStart, _>(LOOP_BEFORE_START, "gate", move |_| {
                Verdict::Deny(long.clone())
            })
            .unwrap();
        let hooks = LoopHooks::new(Arc::new(hooks));
        rt().block_on(async move {
            let denied = hooks
                .run_policy(
                    LOOP_BEFORE_START,
                    &BeforeStart {
                        run_id: 1,
                        owner: "o".into(),
                    },
                )
                .await
                .expect_err("a deny listener refuses the boundary");
            assert_eq!(denied.event, LOOP_BEFORE_START);
            assert_eq!(denied.reason.len(), REASON_CAP);
        });
    }

    #[test]
    fn registration_validation_is_the_registrys_and_propagates() {
        let hooks = Hooks::new();
        assert!(matches!(
            hooks.on::<String, _>("", "prov", |_| Verdict::Allow),
            Err(HooksError::Invalid(_))
        ));
        assert!(matches!(
            hooks.on::<String, _>(LOOP_BEFORE_INPUT, "", |_| Verdict::Allow),
            Err(HooksError::Invalid(_))
        ));
        assert!(
            hooks
                .on::<String, _>(LOOP_BEFORE_INPUT, "prov", |_| Verdict::Allow)
                .is_ok()
        );
    }

    #[test]
    fn steer_applies_mutators_in_order_and_refuses_cap_excess() {
        let hooks = Hooks::new();
        hooks
            .on_mutate::<String, _>(LOOP_BEFORE_INPUT, "first", |s: &mut String| {
                s.push('a');
            })
            .unwrap();
        hooks
            .on_mutate::<String, _>(LOOP_BEFORE_INPUT, "second", |s: &mut String| {
                s.push('b');
            })
            .unwrap();
        let steered = LoopHooks::new(Arc::new(hooks));
        let mut input = "x".to_string();
        steered.steer_input(&mut input).unwrap();
        assert_eq!(input, "xab");

        let hooks = Hooks::new();
        let big = "y".repeat(crate::workflow::session_log::PAYLOAD_CAP_BYTES);
        hooks
            .on_mutate::<String, _>(LOOP_BEFORE_INPUT, "flood", move |s: &mut String| {
                s.push_str(&big);
            })
            .unwrap();
        let flooding = LoopHooks::new(Arc::new(hooks));
        let mut input = "x".to_string();
        let refused = flooding
            .steer_input(&mut input)
            .expect_err("a cap-exceeding steer is a loud refusal, not a truncation");
        assert!(refused.reason.contains("payload cap"));
        assert!(input.len() > crate::workflow::session_log::PAYLOAD_CAP_BYTES);
    }

    #[test]
    fn payloads_are_typed_bounded_and_clonable() {
        let start = BeforeStart {
            run_id: 7,
            owner: "owner".into(),
        };
        let cloned = start.clone();
        assert_eq!(cloned, start);
        assert_eq!(cloned.run_id, 7);
        let compaction = BeforeCompaction {
            run_id: 7,
            pressure_tokens: 1234,
        };
        assert_eq!(compaction.clone(), compaction);
    }
}
