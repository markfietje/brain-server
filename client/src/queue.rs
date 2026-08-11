//! v1.20.0 M3 — offline-tolerance action queue (queue + replay, not full
//! offline). A write action issued while the backend is unreachable is queued
//! locally (bounded, deduped by a deterministic client idempotency key) and
//! replayed when `/health` is green again. The backend's non-idempotent
//! contract is handled client-side: a replay that hits 404-no-pending counts
//! as applied (the v1.16.0 M3 AlreadyDone rule), so nothing double-applies.
//!
//! ponytail: persisted via the i18n localStorage pref seam (web only; no-op
//! native — a native offline queue is keyring work). Queued reject reasons /
//! purge owners are operator-typed text in site-local storage; the v1.18.1
//! secret rule applies — never queue a token, only actions.

use crate::api::ApiError;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// Queue bound — a flaky session can pile up more, but memory + replay cost
/// stay bounded. Oldest first (FIFO) beyond the cap.
pub const MAX_QUEUED_ACTIONS: usize = 100;

/// The queue's localStorage key (via `i18n::pref_save`/`pref_load`).
pub const QUEUE_PREF_KEY: &str = "action_queue";

/// One write action that could not reach the server. Every action has a
/// deterministic `key()` = its semantic JSON, so identical actions dedupe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum QueuedAction {
    Approve {
        id: i64,
        supersedes: Option<i64>,
    },
    Reject {
        id: i64,
        reason: Option<String>,
    },
    Purge {
        chunk_ids: Vec<i64>,
        owner: Option<String>,
    },
    Dsar {
        subject: String,
        action: String,
    },
}

impl QueuedAction {
    /// Client idempotency key: the semantic JSON of the action. Two "approve
    /// 42" issued at different moments are the same action — replaying one
    /// settles the key, so the second never re-applies.
    pub fn key(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Pure: add unless an identical action is already queued. Drops the oldest
/// past the cap. Returns whether the action was added.
pub fn queue_add(queue: &mut Vec<QueuedAction>, action: QueuedAction) -> bool {
    let key = action.key();
    if queue.iter().any(|a| a.key() == key) {
        return false;
    }
    if queue.len() >= MAX_QUEUED_ACTIONS {
        queue.remove(0);
    }
    queue.push(action);
    true
}

/// Pure: serde round-trip for persistence.
pub fn queue_to_json(queue: &[QueuedAction]) -> String {
    serde_json::to_string(queue).unwrap_or_else(|_| "[]".to_string())
}

/// Pure: parse persisted queue JSON; anything unparseable → empty (never
/// block launch on a corrupt pref).
pub fn queue_from_json(raw: &str) -> Vec<QueuedAction> {
    serde_json::from_str(raw).unwrap_or_default()
}

/// The offline signal. One-line match so every call site (panels + replay)
/// shares the same definition of "unreachable vs server-answered".
pub fn is_offline(e: &ApiError) -> bool {
    matches!(e, ApiError::Network(_))
}

/// Pure: did a replayed action settle on the server? A network error is
/// temporary (stay queued); a 404-no-pending means someone already decided it
/// (v1.16.0 M3 AlreadyDone rule — treat as applied, never re-fire).
pub fn replay_applied<T>(res: &Result<T, ApiError>) -> bool {
    match res {
        Ok(_) => true,
        Err(e) if !is_offline(e) => matches!(e, ApiError::Status(404, _)),
        Err(_) => false,
    }
}

/// Accessor for the global queue signal (the i18n accessor-fn idiom — a
/// `static Signal` can't be `.set()` without an immutable-static borrow error).
pub fn queue() -> Global<Signal<Vec<QueuedAction>>, Vec<QueuedAction>> {
    Signal::global(|| -> Vec<QueuedAction> { Vec::new() })
}

fn persist(queue: &[QueuedAction]) {
    crate::i18n::pref_save(QUEUE_PREF_KEY, &queue_to_json(queue));
}

/// Enqueue for replay (dedupes; persists; callable from any panel). Returns
/// whether the action was queued.
pub fn enqueue(action: QueuedAction) -> bool {
    let mut q = queue().write();
    let added = queue_add(&mut q, action);
    let snapshot = q.clone();
    drop(q);
    persist(&snapshot);
    added
}

/// Drain the whole queue for replay (persists the empty state; failed actions
/// are re-enqueued individually by the caller's `enqueue`).
pub fn take_all() -> Vec<QueuedAction> {
    let mut q = queue().write();
    let items = std::mem::take(&mut *q);
    drop(q);
    persist(&[]);
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_dedupes_identical_actions_by_semantic_key() {
        let mut q = Vec::new();
        assert!(queue_add(
            &mut q,
            QueuedAction::Approve {
                id: 7,
                supersedes: None
            }
        ));
        assert!(!queue_add(
            &mut q,
            QueuedAction::Approve {
                id: 7,
                supersedes: None
            }
        ));
        assert_eq!(q.len(), 1);
        // A different action with the same id is distinct.
        assert!(queue_add(
            &mut q,
            QueuedAction::Reject {
                id: 7,
                reason: None
            }
        ));
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn queue_bounds_at_cap_and_evicts_oldest() {
        let mut q = Vec::new();
        for i in 0..(MAX_QUEUED_ACTIONS + 5) {
            assert!(queue_add(
                &mut q,
                QueuedAction::Reject {
                    id: i as i64,
                    reason: None
                }
            ));
        }
        assert_eq!(q.len(), MAX_QUEUED_ACTIONS);
        // The oldest (id 0..4) fell out; the newest five survived.
        let newest = (MAX_QUEUED_ACTIONS + 4) as i64;
        assert!(!q.contains(&QueuedAction::Reject {
            id: 2,
            reason: None
        }));
        assert!(q.contains(&QueuedAction::Reject {
            id: newest,
            reason: None
        }));
    }

    #[test]
    fn queue_serde_round_trips_and_corrupt_pref_loads_empty() {
        let q = vec![
            QueuedAction::Approve {
                id: 1,
                supersedes: Some(3),
            },
            QueuedAction::Dsar {
                subject: "a@b.c".into(),
                action: "both".into(),
            },
        ];
        assert_eq!(queue_from_json(&queue_to_json(&q)), q);
        assert!(queue_from_json("not json {{{").is_empty());
    }

    #[test]
    fn replay_applied_counts_ok_and_already_done_but_never_offline() {
        assert!(replay_applied::<serde_json::Value>(&Ok(
            serde_json::Value::Null
        )));
        assert!(replay_applied::<serde_json::Value>(&Err(ApiError::Status(
            404,
            "no pending".into()
        ))));
        assert!(!replay_applied::<serde_json::Value>(&Err(
            ApiError::Status(500, "boom".into())
        )));
    }
}
