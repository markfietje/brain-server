//! Signed delivery intents: the kernel mint, and the read-side authenticity check.
//!
//! An intent is the machine's own record that it WANTS an external authority to
//! do something. It is not a command, and nothing HERE acts on one: the mint
//! files the record, the promote gate decides whether it may exist, and the
//! /due crank drains it — each in its own place, so no single writer both
//! decides and sends. The intents are minted, forge-checked, and left
//! `pending` with an observable count until their release is promoted.
//!
//! Two independent fences, and it matters that they are independent:
//!
//! 1. [`RESERVED_OUTBOX_TOPICS`](super::outbox::RESERVED_OUTBOX_TOPICS) — the
//!    `delivery/` prefix. An agent posting to the events route cannot mint a
//!    delivery row at all; the route answers 400 `topic_reserved`.
//! 2. [`intent_is_authentic`] — the topic AND the key. A row that carries a
//!    delivery topic but not a minted key is a forgery that slipped past (1),
//!    or a row written by a future writer that forgot the mint.
//!
//! The second is DEFENCE IN DEPTH, and it degrades rather than refuses. A row
//! whose authenticity cannot be established reads as the generic kind: it is
//! still stored, still visible, still auditable, and simply not an intent.
//! Silently dropping it would lose the evidence that a forgery was attempted;
//! treating it as an intent would let the forgery through. The enqueue gate is
//! the fence; this is the second lock on the same door.
//!
//! What an intent is NOT: an approval, a permission, a release, or a claim that
//! the external system did anything. Authorship is not authority. A signed
//! intent says the machine asked; only an inbound authority observation can
//! say the world changed, and until one arrives the ledger believes nothing.

use rusqlite::Connection;

use super::outbox::{self, KernelOrigin};

/// The reserved root every delivery intent topic sits under. The vocabulary
/// itself is declared in
/// [`RESERVED_OUTBOX_TOPICS`](super::outbox::RESERVED_OUTBOX_TOPICS); this is
/// the same string read by the predicate, so the two can never disagree.
pub(crate) const DELIVERY_ROOT: &str = "delivery/";

/// The kernel mint prefix. A key that does not carry it was not minted here.
pub(crate) const INTENT_KEY_PREFIX: &str = "ddl-intent-";

/// The closed intent vocabulary. A topic outside this set is refused at the
/// mint, so a typo becomes a typed refusal rather than an undrainable row
/// nobody recognises.
pub(crate) const INTENT_TOPICS: &[&str] = &[
    "delivery/intent:vcs",
    "delivery/intent:ci",
    "delivery/observed:vcs",
    "delivery/observed:ci",
    "delivery/plan",
    "delivery/approval-request",
    "delivery/approval-decision",
    "delivery/attestation",
    "delivery/rollback",
    "delivery/outcome",
];

/// What a stored row IS, as the read side classifies it. The generic arm is the
/// degradation target: a delivery topic with an unminted key lands here rather
/// than being dropped or trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntentKind {
    /// Kernel-minted, forge-checked, and NOT dispatchable this round.
    Intent,
    /// The inbound authority observation — what an external system says
    /// happened, which is the only thing that can move the ledger's belief.
    Observed,
    /// A delivery-family row whose key is not a kernel mint. Stored, visible,
    /// auditable, and not an intent.
    Untrusted,
    /// Not a delivery row at all.
    NotDelivery,
}

/// PURE: the authenticity predicate, and a conjunction on purpose.
///
/// Both halves are required. An `||` would admit a forged key on a reserved
/// topic, or a genuine-looking key on a topic outside the reserved root — and
/// either of those is a row the read side would then treat as an intent.
pub(crate) fn intent_is_authentic(topic: &str, key: &str) -> bool {
    topic.starts_with(DELIVERY_ROOT) && key.starts_with(INTENT_KEY_PREFIX)
}

/// PURE: classify a stored row. The demotion, and it never panics — an
/// unverifiable row is data, not a crash.
pub(crate) fn intent_kind(topic: &str, key: &str) -> IntentKind {
    if !topic.starts_with(DELIVERY_ROOT) {
        return IntentKind::NotDelivery;
    }
    if !key.starts_with(INTENT_KEY_PREFIX) {
        return IntentKind::Untrusted;
    }
    if topic.starts_with("delivery/observed:") {
        IntentKind::Observed
    } else {
        IntentKind::Intent
    }
}

/// The mint refusal. Distinct from `OutboxError` because it is a vocabulary
/// problem in the CALLER's topic, not a transaction outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IntentError {
    /// The topic is outside the closed vocabulary. Carried verbatim so the
    /// refusal can name what was refused — this value is kernel-authored, not
    /// caller-supplied text, and never reaches a log line unfiltered.
    UnknownTopic { topic: String },
    /// The store refused. Surfaced rather than swallowed: a lost intent is
    /// exactly the failure the pending gauge exists to make visible.
    Store(String),
}

impl std::fmt::Display for IntentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTopic { topic } => {
                write!(f, "`{topic}` is not a delivery intent topic")
            }
            Self::Store(detail) => write!(f, "{detail}"),
        }
    }
}

/// Mint one intent, INSIDE the caller's transaction.
///
/// `run_id` + `seq` is the key, the `valet-` precedent: a stable, replay-safe
/// identity that the read side can authenticate without a second store. The
/// sequence is the caller's — the minter does not read `MAX(seq)`, because a
/// writer that computed its own ordinal would race a concurrent one for it.
///
/// The audit row rides the shared insert (first insert only), so a replayed
/// mint is a receipt and not a second chain entry.
pub(crate) fn mint_intent(
    conn: &Connection,
    run_id: i64,
    seq: i64,
    topic: &str,
    payload_json: &str,
    now: i64,
) -> Result<(bool, i64), IntentError> {
    if !INTENT_TOPICS.contains(&topic) {
        return Err(IntentError::UnknownTopic {
            topic: topic.to_string(),
        });
    }
    let key = format!("{INTENT_KEY_PREFIX}{run_id}-{seq}");
    outbox::enqueue_child_kernel(
        conn,
        run_id,
        None,
        topic,
        payload_json,
        &key,
        now,
        KernelOrigin::kernel(),
    )
    .map_err(|e| IntentError::Store(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut db = Connection::open_in_memory().expect("open");
        crate::migration::run_migration(&mut db, 512).expect("migration");
        db
    }

    fn open_run(db: &Connection) -> i64 {
        db.execute(
            "INSERT INTO workflow_runs(domain, kind, status, state_json, state_revision, \
             created_at, updated_at) VALUES ('personal','delivery','active','{}',1,1,1)",
            [],
        )
        .expect("run");
        db.last_insert_rowid()
    }

    #[test]
    fn authenticity_is_a_conjunction_of_root_and_mint() {
        assert!(intent_is_authentic("delivery/intent:vcs", "ddl-intent-7-0"));
        // A reserved topic with a forged key is not an intent.
        assert!(!intent_is_authentic("delivery/intent:vcs", "forged"));
        assert!(!intent_is_authentic("delivery/intent:vcs", ""));
        // A minted key on a topic outside the root is not an intent either.
        assert!(!intent_is_authentic("workflow/case-note", "ddl-intent-7-0"));
        // The empty strings are the boundary: a blank topic never authenticates.
        assert!(!intent_is_authentic("", ""));
        assert!(!intent_is_authentic("", "ddl-intent-7-0"));
    }

    #[test]
    fn forged_key_degrades_to_untrusted_and_is_never_dropped() {
        assert_eq!(
            intent_kind("delivery/intent:vcs", "ddl-intent-7-0"),
            IntentKind::Intent
        );
        assert_eq!(
            intent_kind("delivery/observed:ci", "ddl-intent-7-1"),
            IntentKind::Observed
        );
        // The degradation: stored, classified, not an intent.
        assert_eq!(
            intent_kind("delivery/intent:vcs", "forged"),
            IntentKind::Untrusted
        );
        // A non-delivery row is untouched by the widening.
        assert_eq!(
            intent_kind("workflow/case-note", "k1"),
            IntentKind::NotDelivery
        );
    }

    #[test]
    fn the_mint_refuses_a_topic_outside_the_vocabulary() {
        let db = db();
        let run_id = open_run(&db);
        let refused = mint_intent(&db, run_id, 0, "delivery/teleport", "{}", 1);
        assert!(matches!(refused, Err(IntentError::UnknownTopic { .. })));
        // A refused mint lands nothing.
        let n: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE run_id=?1 AND topic LIKE 'delivery/%'",
                [run_id],
                |r| r.get(0),
            )
            .expect("query");
        assert_eq!(n, 0);
    }

    #[test]
    fn a_minted_intent_is_authentic_and_replay_is_a_receipt() {
        let db = db();
        let run_id = open_run(&db);

        let (inserted, id) = mint_intent(&db, run_id, 0, "delivery/intent:vcs", "{}", 1)
            .expect("the first mint inserts");
        assert!(inserted);
        // A replayed mint is an idempotent no-op that still resolves the id.
        let (again, same_id) = mint_intent(&db, run_id, 0, "delivery/intent:vcs", "{}", 2)
            .expect("the replay is a receipt");
        assert!(!again);
        assert_eq!(id, same_id, "a replay must resolve the SAME row");

        let key: String = db
            .query_row(
                "SELECT idempotency_key FROM outbox WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .expect("key");
        assert!(
            intent_is_authentic("delivery/intent:vcs", &key),
            "the stored key must authenticate — the mint and the predicate read the same prefix"
        );
    }
}
