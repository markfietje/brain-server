//! v1.20.0 M3 — offline-tolerance action queue (queue + replay, not full
//! offline). A write action issued while the backend is unreachable is queued
//! locally (bounded, deduped by a deterministic client idempotency key) and
//! replayed when `/health` is green again. The backend's non-idempotent
//! contract is handled client-side: a replay that hits 404-no-pending counts
//! as applied (the v1.16.0 M3 AlreadyDone rule), so nothing double-applies.
//!
//! v1.28.1 "Holdall" M4 (F-12/F-34/F-35): destruction discipline —
//! 1. **No payload beyond the minimum**: the persisted form is `(kind, ids +
//!    owner | subject_hash, queued_at)` — free-text reasons and raw DSAR
//!    subjects never touch site-local storage (a DSAR subject is personal
//!    data; a hash is evidence, not the email). The purge owner is the one
//!    deliberate exception (v1.27.21 N8): an owner-scoped purge without its
//!    owner replays as a no-op body, so the scope travels or the erasure
//!    silently never happens.
//! 2. **No auto-fire of irreversible ops**: `split_for_replay` parks every
//!    destructive action (`Purge`/`Dsar`) behind an explicit human review
//!    list — replaying re-prompts the subject (the hash is not reversible).
//!
//! v1.27.21 (N5/N7): a per-item retry counter (persisted, capped at
//! `MAX_REPLAY_RETRIES`) parks a persistently-failing action after five
//! server-answered failures instead of refiring forever, and the DSAR digest
//! is salted per install (SHA-256(salt ‖ subject)) so the persisted hash is
//! useless as a precomputed/rainbow-table target.
//!
//! ponytail: persisted via the i18n localStorage pref seam (web only; no-op
//! native — a native offline queue is keyring work). The v1.18.1 secret rule
//! applies — never queue a token, only actions.

use crate::api::ApiError;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Queue bound — a flaky session can pile up more, but memory + replay cost
/// stay bounded. Oldest first (FIFO) beyond the cap.
pub const MAX_QUEUED_ACTIONS: usize = 100;

/// The queue's localStorage key (via `i18n::pref_save`/`pref_load`).
pub const QUEUE_PREF_KEY: &str = "action_queue";

/// v1.27.21 (N7): the per-install DSAR salt's localStorage key — its own seam,
/// next to (not inside) the queue, so adding it never rewrites queued items.
pub const SALT_PREF_KEY: &str = "dsar_subject_salt";

/// v1.27.21 (N5): after this many server-answered failures an auto-replayed
/// action parks in the review banner instead of refiring on every reconnect.
pub const MAX_REPLAY_RETRIES: u8 = 5;

/// v1.27.21 (N7) pure: SHA-256(salt ‖ subject) as hex — the salted DSAR
/// digest. An empty salt is exactly the legacy unsalted form (plain
/// SHA-256(subject)), so pre-salt persisted hashes still verify as
/// `digest_salted("", subject)`.
pub fn digest_salted(salt: &str, subject: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(subject.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Monotonic salt-generation counter — two salts minted inside the same
/// clock tick must still differ.
static SALT_SEQ: AtomicU64 = AtomicU64::new(0);

/// v1.27.21 (N7): fresh salt entropy. Sub-second clock + a process counter:
/// the salt is NOT a secret (it sits in the same localStorage as the hash) —
/// its only job is uniqueness per install, which defeats precomputed tables
/// and cross-install correlation; no `rand` dep for that.
fn generate_salt() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seq = SALT_SEQ.fetch_add(1, Ordering::Relaxed);
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(seq.to_le_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Accessor for the per-install DSAR salt signal (the i18n accessor-fn idiom).
/// `None` until first use or the launch-time restore sets it.
pub fn salt() -> Global<Signal<Option<String>>, Option<String>> {
    Signal::global(|| None)
}

/// v1.27.21 (N7): the current salt, minting + persisting one on first use.
/// The in-memory signal is authoritative for the session; `pref_save` makes
/// it survive reloads (best-effort, like every pref seam here).
pub fn ensure_salt() -> String {
    if let Some(s) = salt().read().clone() {
        return s;
    }
    let fresh = generate_salt();
    crate::i18n::pref_save(SALT_PREF_KEY, &fresh);
    salt().set(Some(fresh.clone()));
    fresh
}

/// v1.27.21 (N7): the digest to persist for a queued DSAR subject — salted
/// with the per-install salt (the row carries `salted: true` so replay knows
/// the verification form).
pub fn dsar_subject_hash(subject: &str) -> (String, bool) {
    let salt = ensure_salt();
    (digest_salted(&salt, subject), true)
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
/// (an offline reject replays as a bare reject), no raw DSAR subject (only
/// its SHA-256 hash; the reviewer re-types the subject to replay). The purge
/// owner is persisted (v1.27.21 N8) because it is the erasure's scope, not
/// commentary. `queued_at` stamps every variant for the review list.
///
/// v1.27.21 (N5/N6/N7): `retries` counts server-answered replay failures (a
/// legacy persisted item without it decodes as 0) and is volatile — `key()`
/// normalizes it (with `queued_at`) out of the identity; `salted` marks a
/// DSAR hash as install-salted (a pre-salt item decodes as legacy).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum QueuedAction {
    Approve {
        id: i64,
        supersedes: Option<i64>,
        queued_at: i64,
        #[serde(default)]
        retries: u8,
    },
    Reject {
        id: i64,
        queued_at: i64,
        #[serde(default)]
        retries: u8,
    },
    /// v1.20.14 "Steer" — an offline edit of a pending proposal's content. The
    /// `key()` includes `content`, so two distinct edits of the same proposal
    /// are distinct actions (last-edited-wins on replay); replaying a decided
    /// proposal 404s and counts as applied (AlreadyDone rule).
    Edit {
        id: i64,
        content: String,
        queued_at: i64,
        #[serde(default)]
        retries: u8,
    },
    /// v1.27.21 (N8): `owner` is the erasure's scope — replaying an
    /// owner-scoped purge without it would send an empty body (a no-op the
    /// operator mistakes for an applied erasure).
    Purge {
        chunk_ids: Vec<i64>,
        #[serde(default)]
        owner: Option<String>,
        queued_at: i64,
        #[serde(default)]
        retries: u8,
    },
    Dsar {
        subject_hash: String,
        action: String,
        /// v1.27.21 (N7): `false` on pre-salt items — replay re-prompts the
        /// subject but cannot verify it against the unsalted digest.
        #[serde(default)]
        salted: bool,
        queued_at: i64,
        #[serde(default)]
        retries: u8,
    },
}

impl QueuedAction {
    /// Client idempotency key: the semantic JSON of the action. Two "approve
    /// 42" issued at different moments are the same action — replaying one
    /// settles the key, so the second never re-applies.
    ///
    /// v1.27.21 (N6): the volatile bookkeeping (`queued_at`, `retries`) is
    /// zeroed first, so the key is the action's identity, not its history —
    /// a re-enqueued retry must collapse onto the original, not coexist.
    pub fn key(&self) -> String {
        let mut norm = self.clone();
        norm.clear_volatile();
        serde_json::to_string(&norm).unwrap_or_default()
    }

    fn clear_volatile(&mut self) {
        match self {
            QueuedAction::Approve {
                queued_at, retries, ..
            }
            | QueuedAction::Reject {
                queued_at, retries, ..
            }
            | QueuedAction::Edit {
                queued_at, retries, ..
            }
            | QueuedAction::Purge {
                queued_at, retries, ..
            }
            | QueuedAction::Dsar {
                queued_at, retries, ..
            } => {
                *queued_at = 0;
                *retries = 0;
            }
        }
    }

    /// v1.27.21 (N5): server-answered replay failures so far.
    pub fn retries(&self) -> u8 {
        match self {
            QueuedAction::Approve { retries, .. }
            | QueuedAction::Reject { retries, .. }
            | QueuedAction::Edit { retries, .. }
            | QueuedAction::Purge { retries, .. }
            | QueuedAction::Dsar { retries, .. } => *retries,
        }
    }

    fn bump_retries(&mut self) {
        match self {
            QueuedAction::Approve { retries, .. }
            | QueuedAction::Reject { retries, .. }
            | QueuedAction::Edit { retries, .. }
            | QueuedAction::Purge { retries, .. }
            | QueuedAction::Dsar { retries, .. } => *retries = retries.saturating_add(1),
        }
    }

    /// v1.28.1 M4 pure: is this an irreversible operation? Destructive actions
    /// are never auto-replayed on reconnect — they park for explicit review.
    pub fn is_destructive(&self) -> bool {
        matches!(self, QueuedAction::Purge { .. } | QueuedAction::Dsar { .. })
    }

    /// v1.27.21 (N5) pure: does this item demand a human instead of another
    /// auto-replay? Destructive by construction, or its retry budget is
    /// spent — either way the review banner owns it from here.
    pub fn is_parked(&self) -> bool {
        self.is_destructive() || self.retries() >= MAX_REPLAY_RETRIES
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
                    // Purge: ids + owner survive — the owner is the erasure's
                    // scope (v1.27.21 N8), dropping it would neuter the replay.
                    "Purge" => {
                        let ids: Vec<i64> = inner
                            .get("chunk_ids")
                            .and_then(|c| serde_json::from_value(c.clone()).ok())
                            .unwrap_or_default();
                        let owner = inner
                            .get("owner")
                            .and_then(|o| o.as_str())
                            .map(str::to_string)
                            .filter(|o| !o.trim().is_empty());
                        Some(serde_json::json!({ "Purge": {
                            "chunk_ids": ids,
                            "owner": owner,
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
/// (approve/reject/edit) replay automatically on reconnect — until their
/// retry budget is spent (v1.27.21 N5); destructive ones (purge/dsar) always
/// park for the explicit human review list.
#[cfg(test)]
pub fn split_for_replay(queue: Vec<QueuedAction>) -> (Vec<QueuedAction>, Vec<QueuedAction>) {
    let mut auto = Vec::new();
    let mut parked = Vec::new();
    for a in queue {
        if a.is_parked() {
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

/// v1.28.1 M4: drain ONLY the auto-replayable subset. Destructive actions —
/// and, since v1.27.21 (N5), actions whose retry budget is spent — stay queued:
/// they are the review banner's rows and must survive reloads; only an
/// explicit human decision removes them. Idempotent: a second call on a
/// parked-only queue drains nothing.
pub fn take_replayable() -> Vec<QueuedAction> {
    let mut q = queue().write();
    let mut auto = Vec::new();
    q.retain(|a| {
        if a.is_parked() {
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

/// v1.28.1 M4 pure: the destructive subset — the review banner's rows. Since
/// v1.27.21 (N5) this is the whole parked set: destructive OR retry-exhausted.
pub fn parked(queue: &[QueuedAction]) -> Vec<QueuedAction> {
    queue.iter().filter(|a| a.is_parked()).cloned().collect()
}

/// v1.28.1 M4: replace the queue's parked subset with `kept` (the review
/// banner's post-decision rows); auto-replayable items are untouched.
/// Persists the result. Auto-replay of the survivors is impossible: they are
/// parked by construction (destructive, or retries spent).
pub fn replace_parked(kept: Vec<QueuedAction>) {
    let mut q = queue().write();
    q.retain(|a| !a.is_parked());
    for a in kept {
        if !q.contains(&a) {
            q.push(a);
        }
    }
    let snapshot = q.clone();
    drop(q);
    persist(&snapshot);
}

/// v1.27.21 (N5) pure: record one more failed replay of `action` on its queued
/// twin (matched by idempotency key — the drain already removed the fired
/// copy). Keeps the original `queued_at` (the earliest stamp wins, N6) and
/// bumps `retries` toward the park threshold. Returns whether a twin was
/// found (false = the operator dismissed it mid-flight; nothing to record).
pub fn requeue_in(queue: &mut [QueuedAction], action: &QueuedAction) -> bool {
    let key = action.key();
    match queue.iter_mut().find(|a| a.key() == key) {
        Some(twin) => {
            twin.bump_retries();
            true
        }
        None => false,
    }
}

/// v1.27.21 (N5): the replay loop's failure path for a server-answered error —
/// bump the queued twin's retry count (at the cap the item parks itself and
/// the banner takes over); persist.
pub fn requeue_failed(action: &QueuedAction) {
    let mut q = queue().write();
    requeue_in(&mut q, action);
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
            retries: 0,
        }
    }

    fn reject(id: i64, queued_at: i64) -> QueuedAction {
        QueuedAction::Reject {
            id,
            queued_at,
            retries: 0,
        }
    }

    #[test]
    fn queue_dedupes_identical_actions_by_semantic_key() {
        let mut q = Vec::new();
        assert!(queue_add(&mut q, approve(7)));
        // v1.27.21 (N6): a different stamp — or a different retry count — is
        // still the same action; the earliest entry wins.
        assert!(!queue_add(
            &mut q,
            QueuedAction::Approve {
                id: 7,
                supersedes: None,
                queued_at: 9000,
                retries: 3,
            }
        ));
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].key(), approve(7).key());
        // A different action with the same id is distinct.
        assert!(queue_add(&mut q, reject(7, 1000)));
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn queue_bounds_at_cap_and_evicts_oldest() {
        let mut q = Vec::new();
        for i in 0..(MAX_QUEUED_ACTIONS + 5) {
            assert!(queue_add(&mut q, reject(i as i64, i as i64)));
        }
        assert_eq!(q.len(), MAX_QUEUED_ACTIONS);
        // The oldest (id 0..4) fell out; the newest five survived.
        let newest = (MAX_QUEUED_ACTIONS + 4) as i64;
        assert!(!q.contains(&reject(2, 2)));
        assert!(q.contains(&reject(newest, newest)));
    }

    #[test]
    fn queue_serde_round_trips_and_corrupt_pref_loads_empty() {
        let q = vec![
            QueuedAction::Approve {
                id: 1,
                supersedes: Some(3),
                queued_at: 42,
                retries: 0,
            },
            QueuedAction::Dsar {
                subject_hash: digest_salted("", "a@b.c"),
                action: "both".into(),
                salted: false,
                queued_at: 42,
                retries: 0,
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
            reject(2, 1),
            QueuedAction::Edit {
                id: 3,
                content: "x".into(),
                queued_at: 1,
                retries: 0,
            },
            QueuedAction::Purge {
                chunk_ids: vec![9, 10],
                owner: None,
                queued_at: 1,
                retries: 0,
            },
            QueuedAction::Dsar {
                subject_hash: digest_salted("s", "s@x"),
                action: "both".into(),
                salted: true,
                queued_at: 1,
                retries: 0,
            },
        ];
        let (auto, parked) = split_for_replay(q);
        // Only the irreversible ops park (at retry 0).
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

    /// v1.27.21 (N5): a persistently-failing auto-replay parks at the cap —
    /// five server-answered failures and the item joins the review banner
    /// instead of refiring on every reconnect; one below the cap it still
    /// auto-replays.
    #[test]
    fn retry_exhausted_items_park_for_manual_decision() {
        let at_cap = QueuedAction::Reject {
            id: 5,
            queued_at: 1,
            retries: MAX_REPLAY_RETRIES,
        };
        let one_left = QueuedAction::Reject {
            id: 6,
            queued_at: 1,
            retries: MAX_REPLAY_RETRIES - 1,
        };
        assert!(at_cap.is_parked());
        assert!(!one_left.is_parked());
        let (auto, parked) = split_for_replay(vec![at_cap, one_left]);
        assert_eq!(auto.len(), 1);
        assert_eq!(parked.len(), 1, "the capped item demands a human");
    }

    /// v1.27.21 (N5): the failure path bumps the queued twin's retry count,
    /// keeps the earliest stamp (N6), and is a no-op when the row was
    /// dismissed mid-flight.
    #[test]
    fn requeue_failed_bumps_the_queued_twin_only() {
        let mut q = vec![reject(9, 100)];
        let fired = reject(9, 100);
        assert!(requeue_in(&mut q, &fired));
        assert_eq!(
            q[0],
            QueuedAction::Reject {
                id: 9,
                queued_at: 100,
                retries: 1,
            },
            "retries bump; the original (earliest) stamp survives"
        );
        // Dismissed mid-flight: no twin, nothing recorded, no panic.
        assert!(!requeue_in(&mut q, &reject(10, 100)));
        assert_eq!(q.len(), 1);
        // Saturating: the cap never overflows the counter.
        let mut capped = vec![QueuedAction::Reject {
            id: 11,
            queued_at: 1,
            retries: u8::MAX,
        }];
        assert!(requeue_in(&mut capped, &reject(11, 1)));
        assert_eq!(capped[0].retries(), u8::MAX);
    }

    #[test]
    fn queue_payload_contains_no_subject_plaintext() {
        let subject = "alice@example.priv";
        let q = vec![
            QueuedAction::Dsar {
                subject_hash: digest_salted("salt", subject),
                action: "both".into(),
                salted: true,
                queued_at: 7,
                retries: 0,
            },
            QueuedAction::Purge {
                chunk_ids: vec![1],
                owner: Some("owner@co".into()),
                queued_at: 7,
                retries: 0,
            },
            QueuedAction::Reject {
                id: 4,
                queued_at: 7,
                retries: 0,
            },
        ];
        let json = queue_to_json(&q);
        // The subject's raw form never appears; its digest does.
        let d = digest_salted("salt", subject);
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
            !json.contains("\"reason\""),
            "free-text reason is not persisted: {json}"
        );
        // v1.27.21 (N8): the purge owner IS persisted — it is the erasure's
        // scope, and dropping it would replay a silent no-op.
        assert!(
            json.contains("\"owner\":\"owner@co\""),
            "the purge scope travels: {json}"
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
            !json.contains("offline while on call"),
            "legacy free-text reason does not survive: {json}"
        );
        // v1.27.21 (N8): the legacy purge owner migrates — scope over scrubbing.
        assert!(json.contains("boss@co"), "the purge scope migrates: {json}");
        assert!(json.contains("kept edit text"), "edit content migrates");
        assert!(
            out.iter()
                .all(|a| !matches!(a, QueuedAction::Approve { queued_at: 0, .. })),
            "migrated items carry a real queued_at stamp"
        );
    }

    /// v1.27.21 (N7): a pre-salt persisted DSAR item decodes as legacy —
    /// parkable, `salted: false`, no retry count — never blocking launch.
    #[test]
    fn legacy_unsalted_dsar_decodes_without_salt_marker() {
        let raw = format!(
            r#"[{{"Dsar":{{"subject_hash":"{}","action":"both","queued_at":9}}}}]"#,
            digest_salted("", "old@x")
        );
        let out = queue_from_json(&raw);
        assert_eq!(out.len(), 1);
        match &out[0] {
            QueuedAction::Dsar {
                salted, retries, ..
            } => {
                assert!(!*salted, "no salt marker → legacy, replay re-prompts only");
                assert_eq!(*retries, 0, "no retry field decodes as zero");
            }
            other => panic!("expected a Dsar row, got {other:?}"),
        }
        assert!(out[0].is_parked(), "legacy rows still park for review");
    }

    /// v1.27.21 (N7): the salted digest is deterministic, salt-dependent, and
    /// an empty salt is exactly the legacy unsalted SHA-256 (known-answer
    /// vector) — so pre-salt hashes remain interpretable.
    #[test]
    fn salted_digest_is_deterministic_salt_dependent_and_legacy_compatible() {
        assert_eq!(
            digest_salted("s1", "a@b.c"),
            digest_salted("s1", "a@b.c"),
            "same salt + subject → same digest"
        );
        assert_ne!(
            digest_salted("s1", "a@b.c"),
            digest_salted("s2", "a@b.c"),
            "the salt must not be a no-op"
        );
        assert_ne!(digest_salted("s1", "a@b.c"), digest_salted("s1", "a@b.d"));
        // Empty salt == plain SHA-256(subject) — the pre-salt form.
        assert_eq!(
            digest_salted("", "a@b.c"),
            "d648b243a3e817eaa3309e00e183483f2867baadf522099f0c2121770536b25a"
        );
        // Two fresh salts never collide (clock + counter entropy).
        assert_ne!(generate_salt(), generate_salt());
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
