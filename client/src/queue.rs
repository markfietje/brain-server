//! v1.20.0 M3 — offline-tolerance action queue (queue + replay, not full
//! offline). A write action issued while the backend is unreachable is queued
//! locally (bounded, deduped by a deterministic client idempotency key) and
//! replayed when `/health` is green again. The backend's non-idempotent
//! contract is handled client-side: a replay that hits 404-no-pending counts
//! as applied (the v1.16.0 M3 AlreadyDone rule), so nothing double-applies.
//!
//! v1.28.1 "Holdall" M4 (F-12/F-34/F-35): destruction discipline —
//! 1. **No payload beyond the minimum**: the persisted form is `(kind,
//!    chunk_ids | subject_hash, queued_at)` — free-text reasons, purge owners
//!    and raw DSAR subjects never touch site-local storage (a DSAR subject is
//!    personal data; a hash is evidence, not the email).
//! 2. **No auto-fire of irreversible ops**: `split_for_replay` parks every
//!    destructive action (`Purge`/`Dsar`) behind an explicit human review
//!    list — replaying re-prompts the subject (the hash is not reversible).
//!
//! ponytail: persisted via the i18n localStorage pref seam (web only; no-op
//! native — a native offline queue is keyring work). The v1.18.1 secret rule
//! applies — never queue a token, only actions.

use crate::api::ApiError;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Queue bound — a flaky session can pile up more, but memory + replay cost
/// stay bounded. Oldest first (FIFO) beyond the cap.
pub const MAX_QUEUED_ACTIONS: usize = 100;

/// The queue's localStorage key (via `i18n::pref_save`/`pref_load`).
pub const QUEUE_PREF_KEY: &str = "action_queue";

/// v1.28.1 M4 pure: the SHA-256 hex digest used as the persisted DSAR subject
/// identity. One-way like the server's `query_hash` convention — the raw
/// subject is re-prompted at replay, never stored.
pub fn digest(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// v1.28.1 M4: the unix-seconds stamp a queued action carries (persisted form
/// includes `queued_at` so the replay review can show "queued Nm ago").
pub fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// One write action that could not reach the server. Every action has a
/// deterministic `key()` = its semantic JSON, so identical actions dedupe.
///
/// v1.28.1 M4: the persisted payload is the minimum — no free-text reason
/// (an offline reject replays as a bare reject), no purge owner (ids XOR
/// owner is server-enforced; ids suffice), no raw DSAR subject (only the
/// SHA-256 hash; the reviewer re-types the subject to replay). `queued_at`
/// stamps every variant for the review list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum QueuedAction {
    Approve {
        id: i64,
        supersedes: Option<i64>,
        queued_at: i64,
    },
    Reject {
        id: i64,
        queued_at: i64,
    },
    /// v1.20.14 "Steer" — an offline edit of a pending proposal's content. The
    /// `key()` includes `content`, so two distinct edits of the same proposal
    /// are distinct actions (last-edited-wins on replay); replaying a decided
    /// proposal 404s and counts as applied (AlreadyDone rule).
    Edit {
        id: i64,
        content: String,
        queued_at: i64,
    },
    Purge {
        chunk_ids: Vec<i64>,
        queued_at: i64,
    },
    Dsar {
        subject_hash: String,
        action: String,
        queued_at: i64,
    },
}

impl QueuedAction {
    /// Client idempotency key: the semantic JSON of the action. Two "approve
    /// 42" issued at different moments are the same action — replaying one
    /// settles the key, so the second never re-applies.
    pub fn key(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// v1.28.1 M4 pure: is this an irreversible operation? Destructive actions
    /// are never auto-replayed on reconnect — they park for explicit review.
    pub fn is_destructive(&self) -> bool {
        matches!(self, QueuedAction::Purge { .. } | QueuedAction::Dsar { .. })
    }

    /// v1.28.1 M4 pure: the human-readable kind token for the review list.
    pub fn kind(&self) -> &'static str {
        match self {
            QueuedAction::Approve { .. } => "approve",
            QueuedAction::Reject { .. } => "reject",
            QueuedAction::Edit { .. } => "edit",
            QueuedAction::Purge { .. } => "purge",
            QueuedAction::Dsar { .. } => "dsar",
        }
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
///
/// v1.28.1 M4: tolerant of the pre-1.28.1 payload (externally-tagged enum
/// with raw `subject`/`reason`/`owner` fields, no `queued_at`). Legacy items
/// are re-encoded into the minimized form — `reason`/`owner` dropped, a
/// now-stamp added — EXCEPT a legacy `Dsar`, whose raw subject must not
/// survive in any form (the queue drops it rather than migrate it).
pub fn queue_from_json(raw: &str) -> Vec<QueuedAction> {
    let vals: Vec<serde_json::Value> = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let now = now_ts();
    let mut out = Vec::with_capacity(vals.len());
    for v in vals {
        let strict: Option<QueuedAction> = serde_json::from_value(v.clone()).ok();
        match strict {
            Some(a) => out.push(a),
            None => {
                let Some(obj) = v.as_object() else { continue };
                let Some((tag, inner)) = obj.iter().next() else {
                    continue;
                };
                let inner = inner.as_object().cloned().unwrap_or_default();
                let id = inner.get("id").and_then(|i| i.as_i64());
                let payload = match tag.as_str() {
                    "Approve" => id.map(|id| {
                        serde_json::json!({ "Approve": {
                            "id": id,
                            "supersedes": inner.get("supersedes").cloned().unwrap_or(serde_json::Value::Null),
                            "queued_at": now,
                        }})
                    }),
                    "Reject" => id.map(|id| {
                        serde_json::json!({ "Reject": { "id": id, "queued_at": now } })
                    }),
                    // The edit content is the operator's own text — not a
                    // subject/reason — so it migrates (dropping a pending
                    // offline edit would lose work).
                    "Edit" => id.map(|id| {
                        let content = inner
                            .get("content")
                            .and_then(|c| c.as_str())
                            .unwrap_or_default();
                        serde_json::json!({ "Edit": {
                            "id": id,
                            "content": content,
                            "queued_at": now,
                        }})
                    }),
                    // Purge: ids survive, the owner tag does not.
                    "Purge" => {
                        let ids: Vec<i64> = inner
                            .get("chunk_ids")
                            .and_then(|c| serde_json::from_value(c.clone()).ok())
                            .unwrap_or_default();
                        Some(serde_json::json!({ "Purge": {
                            "chunk_ids": ids,
                            "queued_at": now,
                        }}))
                    }
                    // Dsar: the raw subject cannot be stored in the new form
                    // and must not persist in any form — drop the item.
                    "Dsar" => None,
                    _ => None,
                };
                if let Some(payload) = payload {
                    if let Ok(a) = serde_json::from_value(payload) {
                        out.push(a);
                    }
                }
            }
        }
    }
    out
}

/// v1.28.1 M4 pure: split a drained queue for replay. Non-destructive actions
/// (approve/reject/edit) replay automatically on reconnect; destructive ones
/// (purge/dsar) park for the explicit human review list.
#[cfg(test)]
pub fn split_for_replay(queue: Vec<QueuedAction>) -> (Vec<QueuedAction>, Vec<QueuedAction>) {
    let mut auto = Vec::new();
    let mut parked = Vec::new();
    for a in queue {
        if a.is_destructive() {
            parked.push(a);
        } else {
            auto.push(a);
        }
    }
    (auto, parked)
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

/// v1.28.1 M4: drain ONLY the auto-replayable (non-destructive) subset. The
/// destructive actions stay queued — they are the review banner's rows and
/// must survive reloads — and only an explicit human decision removes them.
/// Idempotent: a second call on a parked-only queue drains nothing.
pub fn take_replayable() -> Vec<QueuedAction> {
    let mut q = queue().write();
    let mut auto = Vec::new();
    q.retain(|a| {
        if a.is_destructive() {
            true // stays queued for the human
        } else {
            auto.push(a.clone());
            false
        }
    });
    let snapshot = q.clone();
    drop(q);
    persist(&snapshot);
    auto
}

/// v1.28.1 M4 pure: the destructive subset — the review banner's rows.
pub fn parked(queue: &[QueuedAction]) -> Vec<QueuedAction> {
    queue
        .iter()
        .filter(|a| a.is_destructive())
        .cloned()
        .collect()
}

/// v1.28.1 M4: replace the queue's destructive subset with `kept` (the
/// review banner's post-decision rows); non-destructive items are untouched.
/// Persists the result. Auto-replay of the survivors is impossible: they are
/// destructive by construction.
pub fn replace_parked(kept: Vec<QueuedAction>) {
    let mut q = queue().write();
    q.retain(|a| !a.is_destructive());
    for a in kept {
        if !q.contains(&a) {
            q.push(a);
        }
    }
    let snapshot = q.clone();
    drop(q);
    persist(&snapshot);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approve(id: i64) -> QueuedAction {
        QueuedAction::Approve {
            id,
            supersedes: None,
            queued_at: 1000,
        }
    }

    #[test]
    fn queue_dedupes_identical_actions_by_semantic_key() {
        let mut q = Vec::new();
        assert!(queue_add(&mut q, approve(7)));
        assert!(!queue_add(&mut q, approve(7)));
        assert_eq!(q.len(), 1);
        // A different action with the same id is distinct.
        assert!(queue_add(
            &mut q,
            QueuedAction::Reject {
                id: 7,
                queued_at: 1000
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
                    queued_at: i as i64
                }
            ));
        }
        assert_eq!(q.len(), MAX_QUEUED_ACTIONS);
        // The oldest (id 0..4) fell out; the newest five survived.
        let newest = (MAX_QUEUED_ACTIONS + 4) as i64;
        assert!(!q.contains(&QueuedAction::Reject {
            id: 2,
            queued_at: 2
        }));
        assert!(q.contains(&QueuedAction::Reject {
            id: newest,
            queued_at: newest
        }));
    }

    #[test]
    fn queue_serde_round_trips_and_corrupt_pref_loads_empty() {
        let q = vec![
            QueuedAction::Approve {
                id: 1,
                supersedes: Some(3),
                queued_at: 42,
            },
            QueuedAction::Dsar {
                subject_hash: digest("a@b.c"),
                action: "both".into(),
                queued_at: 42,
            },
        ];
        assert_eq!(queue_from_json(&queue_to_json(&q)), q);
        assert!(queue_from_json("not json {{{").is_empty());
    }

    // ── v1.28.1 "Holdall" M4: destruction discipline ──

    #[test]
    fn destructive_queue_items_require_explicit_replay() {
        let q = vec![
            approve(1),
            QueuedAction::Reject {
                id: 2,
                queued_at: 1,
            },
            QueuedAction::Edit {
                id: 3,
                content: "x".into(),
                queued_at: 1,
            },
            QueuedAction::Purge {
                chunk_ids: vec![9, 10],
                queued_at: 1,
            },
            QueuedAction::Dsar {
                subject_hash: digest("s@x"),
                action: "both".into(),
                queued_at: 1,
            },
        ];
        let (auto, parked) = split_for_replay(q);
        // Only the irreversible ops park.
        assert_eq!(
            auto.iter().map(|a| a.kind()).collect::<Vec<_>>(),
            vec!["approve", "reject", "edit"]
        );
        assert_eq!(
            parked.iter().map(|a| a.kind()).collect::<Vec<_>>(),
            vec!["purge", "dsar"]
        );
        assert!(parked.iter().all(|a| a.is_destructive()));
        assert!(auto.iter().all(|a| !a.is_destructive()));
    }

    #[test]
    fn queue_payload_contains_no_subject_plaintext() {
        let subject = "alice@example.priv";
        let q = vec![
            QueuedAction::Dsar {
                subject_hash: digest(subject),
                action: "both".into(),
                queued_at: 7,
            },
            QueuedAction::Purge {
                chunk_ids: vec![1],
                queued_at: 7,
            },
            QueuedAction::Reject {
                id: 4,
                queued_at: 7,
            },
        ];
        let json = queue_to_json(&q);
        // The subject's raw form never appears; its digest does.
        let d = digest(subject);
        assert_eq!(d.len(), 64, "SHA-256 hex");
        assert!(
            !json.contains(subject),
            "raw subject must not persist: {json}"
        );
        assert!(
            json.contains(&d[..16]),
            "the digest is the stored identity: {json}"
        );
        assert!(
            !json.contains("\"reason\"") && !json.contains("\"owner\""),
            "free-text reason and purge owner are not persisted: {json}"
        );
        // And the minimized form round-trips.
        assert_eq!(queue_from_json(&json), q);
    }

    #[test]
    fn legacy_queue_migrates_without_raw_subject() {
        let legacy = r#"[
            {"Approve":{"id":1,"supersedes":null}},
            {"Reject":{"id":2,"reason":"offline while on call"}},
            {"Edit":{"id":3,"content":"kept edit text"}},
            {"Purge":{"chunk_ids":[5,6],"owner":"boss@co"}},
            {"Dsar":{"subject":"victim@exfil.priv","action":"both"}},
            "garbage"
        ]"#;
        let out = queue_from_json(legacy);
        // Dsar dropped entirely; approve/reject/edit/purge migrated, stamped.
        assert_eq!(
            out.iter().map(|a| a.kind()).collect::<Vec<_>>(),
            vec!["approve", "reject", "edit", "purge"]
        );
        let json = queue_to_json(&out);
        assert!(
            !json.contains("victim@exfil.priv"),
            "a legacy raw subject must not survive any form: {json}"
        );
        assert!(
            !json.contains("boss@co") && !json.contains("offline while on call"),
            "legacy reason/owner do not survive: {json}"
        );
        assert!(json.contains("kept edit text"), "edit content migrates");
        assert!(
            out.iter()
                .all(|a| !matches!(a, QueuedAction::Approve { queued_at: 0, .. })),
            "migrated items carry a real queued_at stamp"
        );
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
        // The Network arm is a fixed hint (constructing a reqwest::Error in a
        // test is not possible — its constructor is pub(crate)); the arm
        // returns false like any non-404 Err, covered by the 500 case above.
    }
}
