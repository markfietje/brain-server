//! Durable-step primitives, part 2: the outbox.
//!
//! Exactly-once event delivery **by idempotency key, not retry count**. A
//! retried `enqueue` with the same key is an idempotent no-op (`INSERT OR
//! IGNORE` is atomic against the `UNIQUE` constraint); `deliver` then advances
//! the row to delivered exactly once. At-least-once delivery + at-most-once
//! *effect* is the classic outbox contract — a duplicate replay cannot double-
//! apply because a replayed key is a no-op receipt.
//!
//! Both writes emit their [`crate::audit::AuditKind::Workflow`] row (see
//! [`super::audit_write`]); a replayed enqueue is deliberately NOT audited —
//! only real state changes reach the chain.

use super::audit_write;
use crate::audit::AuditStatus;
use rusqlite::{Connection, OptionalExtension, params};

// ── the reserved vocabulary ────────────────────────────────────────────────
//
// RESERVED: only kernel paths may enqueue these. `channel/*` re-verifies
// consent + reply-window + approved-proposal inside `enqueue_out`; `steering`
// carries the approve role gate + screen + 4000-char bound in `post_steering`;
// `workflow/valet*` is minted only by the valet crank. The events route is
// AGENT-facing and may not forge any of them — before this gate existed it
// could, which made the three-gate channel law code-false at this seam.
//
// (ponytail: not a general ACL — a closed vocabulary, extend only with a
// kernel writer that owns the gate.)
//
// Per-entry semantics: `channel/` and `workflow/valet` are PREFIX families
// (`channel/out`, `workflow/valet-due`); `steering` is EXACT — the steering
// inbox read spells that literal. New entries default to PREFIX (the stricter
// side); [`topic_is_reserved`] is the single matcher and the
// `reserved_topics_are_declared_in_one_place` pin guards the declaration site.
pub const RESERVED_OUTBOX_TOPICS: &[&str] = &["channel/", "steering", "workflow/valet"];

/// True when a topic carries kernel-only vocabulary. The ONE matcher over
/// [`RESERVED_OUTBOX_TOPICS`] — both the enqueue gate and the handler's
/// 400-mapping read it, so the two can never disagree.
pub fn topic_is_reserved(topic: &str) -> bool {
    RESERVED_OUTBOX_TOPICS
        .iter()
        .any(|reserved| match *reserved {
            // the one declared EXACT entry
            "steering" => topic == *reserved,
            // everything else — today `channel/`, `workflow/valet` — is a PREFIX
            prefix => topic.starts_with(prefix),
        })
}

/// The outbox write error: a reserved-topic refusal is a TYPED outcome (the
/// events route maps it to `400 topic_reserved` + audit row), everything else
/// is the underlying failure's message (rusqlite or the pool).
#[derive(Debug)]
pub enum OutboxError {
    ReservedTopic { topic: String },
    ForeignParent { parent: i64, run: i64 },
    Database(String),
}

impl From<rusqlite::Error> for OutboxError {
    fn from(e: rusqlite::Error) -> Self {
        OutboxError::Database(e.to_string())
    }
}

/// Kernel call sites (append_lineage and the fixed-topic service cores) keep
/// their `rusqlite::Result` plumbing: the reserved branch is unreachable for
/// their hard-coded topics, and this conversion keeps that fact fail-closed —
/// a refusal surfaces as a loud SQLITE-shaped error, never a silent drop.
impl From<OutboxError> for rusqlite::Error {
    fn from(e: OutboxError) -> Self {
        match e {
            OutboxError::Database(msg) => rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                Some(msg),
            ),
            OutboxError::ReservedTopic { topic } => rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                Some(format!(
                    "reserved topic `{topic}` — only kernel writers may enqueue it"
                )),
            ),
            OutboxError::ForeignParent { parent, run } => rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                Some(format!(
                    "parent event {parent} belongs to another run (not {run})"
                )),
            ),
        }
    }
}

impl std::fmt::Display for OutboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutboxError::ReservedTopic { topic } => {
                write!(f, "reserved outbox topic `{topic}`")
            }
            OutboxError::ForeignParent { parent, run } => {
                write!(f, "parent event {parent} is not in run {run}")
            }
            OutboxError::Database(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for OutboxError {}

/// Capability token proving the caller is a KERNEL writer (enqueue_out,
/// enqueue_ping, the steering inbox write, the valet crank) — the only
/// constructors of reserved-topic rows. Zero-sized; the `pub(crate)`
/// constructor means any crate-internal caller COULD mint one, but doing so
/// is a visible, reviewable claim (and the
/// `reserved_topics_are_declared_in_one_place` pin holds the literals to
/// the declared kernel paths). External crates cannot construct it at all.
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub struct KernelOrigin {
    _priv: (),
}

impl KernelOrigin {
    pub(crate) fn kernel() -> Self {
        Self { _priv: () }
    }
}

/// The shared insert behind every enqueue flavor: exactly-once by key
/// (INSERT OR IGNORE), audit row on first insert only.
fn insert_row(
    conn: &Connection,
    run_id: i64,
    parent_id: Option<i64>,
    topic: &str,
    payload_json: &str,
    idempotency_key: &str,
    now: i64,
) -> rusqlite::Result<(bool, i64)> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO outbox(run_id, topic, payload_json, status, idempotency_key, created_at, parent_id)
         VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6)",
        params![run_id, topic, payload_json, idempotency_key, now, parent_id],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM outbox WHERE idempotency_key = ?1",
        params![idempotency_key],
        |r| r.get(0),
    )?;
    if n == 1 {
        audit_write(
            conn,
            run_id,
            &format!("outbox:{idempotency_key}"),
            AuditStatus::Ok,
            &format!("enqueue:{topic}"),
        );
    }
    Ok((n == 1, id))
}

/// Enqueue a payload for a topic. Returns the event id plus `true` if a new
/// row was created, `(existing_id, false)` when the key already replayed
/// (idempotent no-op receipt — the id is still resolved so callers can link
/// against the surviving row). A reserved topic refuses here — this wrapper
/// carries NO [`KernelOrigin`], so non-kernel callers can never mint
/// reserved rows even by accident.
pub(crate) fn enqueue(
    conn: &Connection,
    run_id: i64,
    topic: &str,
    payload_json: &str,
    idempotency_key: &str,
    now: i64,
) -> rusqlite::Result<(bool, i64)> {
    enqueue_child(
        conn,
        run_id,
        None,
        topic,
        payload_json,
        idempotency_key,
        now,
    )
    .map_err(rusqlite::Error::from)
}

/// Enqueue a CHILD event parented at `parent_id` — the Lineage release.
/// Same exactly-once discipline as [`enqueue`] (INSERT OR IGNORE + audit only
/// on first insert); the parent is recorded verbatim. Parent validity is the
/// caller's contract; [`verify_outbox_lineage`] is the integrity check.
///
/// THE RESERVED-TOPIC GATE: any topic matching
/// [`RESERVED_OUTBOX_TOPICS`] refuses with [`OutboxError::ReservedTopic`]
/// unless the caller holds [`KernelOrigin`] — use [`enqueue_child_kernel`].
/// This is the one guard at the shared function, not a per-caller patch:
/// every enqueue flavor funnels through here.
pub(crate) fn enqueue_child(
    conn: &Connection,
    run_id: i64,
    parent_id: Option<i64>,
    topic: &str,
    payload_json: &str,
    idempotency_key: &str,
    now: i64,
) -> Result<(bool, i64), OutboxError> {
    if topic_is_reserved(topic) {
        return Err(OutboxError::ReservedTopic {
            topic: topic.to_string(),
        });
    }
    // Lineage integrity: a parent must belong to the SAME run — a
    // foreign-run parent would stitch false ancestry across runs.
    if let Some(parent) = parent_id {
        let owner: Option<i64> = conn
            .query_row(
                "SELECT run_id FROM outbox WHERE id = ?1",
                rusqlite::params![parent],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| OutboxError::Database(e.to_string()))?;
        if owner != Some(run_id) {
            return Err(OutboxError::ForeignParent { parent, run: run_id });
        }
    }
    insert_row(
        conn,
        run_id,
        parent_id,
        topic,
        payload_json,
        idempotency_key,
        now,
    )
    .map_err(|e| OutboxError::Database(e.to_string()))
}

/// The KERNEL writers' enqueue: identical row discipline, but reserved
/// vocabulary is mintable. `origin` must come from [`KernelOrigin::kernel`];
/// the four holders are `enqueue_out` / `enqueue_ping` (channels.rs), the
/// steering inbox write (`enqueue_steering_tx`), and the valet crank.
#[allow(clippy::too_many_arguments)] // mirrors enqueue_child + the origin token; a params struct would hide the capability
pub(crate) fn enqueue_child_kernel(
    conn: &Connection,
    run_id: i64,
    parent_id: Option<i64>,
    topic: &str,
    payload_json: &str,
    idempotency_key: &str,
    now: i64,
    _origin: KernelOrigin,
) -> Result<(bool, i64), OutboxError> {
    insert_row(
        conn,
        run_id,
        parent_id,
        topic,
        payload_json,
        idempotency_key,
        now,
    )
    .map_err(|e| OutboxError::Database(e.to_string()))
}

/// Append a lineage event at the run's CURRENT tip: the child-parent idiom
/// every governed flow shares (handover offers, case notes). The tip read
/// and the insert ride the CALLER's transaction, so a `BEGIN IMMEDIATE`
/// transition can never fork the chain. Same exactly-once discipline as
/// [`enqueue_child`]; returns the new event's row id.
pub(crate) fn append_lineage(
    conn: &Connection,
    run_id: i64,
    topic: &str,
    payload_json: &str,
    idempotency_key: &str,
    now: i64,
) -> rusqlite::Result<i64> {
    let parent: Option<i64> = conn
        .query_row(
            "SELECT MAX(id) FROM outbox WHERE run_id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let (_, id) = enqueue_child(
        conn,
        run_id,
        parent,
        topic,
        payload_json,
        idempotency_key,
        now,
    )?;
    Ok(id)
}

/// Chain-integrity check beside the audit chain: every non-root `parent_id`
/// must reference an existing row of the SAME run with a SMALLER id. Smaller-id
/// parents make cycles impossible by construction; this verifies the stored
/// rows actually obey the law (orphan / cross-run / forward links fail).
pub(crate) fn verify_outbox_lineage(conn: &Connection, run_id: i64) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM outbox c
          WHERE c.run_id = ?1 AND c.parent_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1 FROM outbox p
                 WHERE p.id = c.parent_id
                   AND p.run_id = c.run_id
                   AND p.id < c.id
            )",
        params![run_id],
        |r| r.get::<_, i64>(0).map(|bad| bad == 0),
    )
}

/// Resolve an event id's ancestry chain (target first, root last) — the
/// `GET /workflow/runs/{id}/events?branch=` read.
pub fn branch_chain(conn: &Connection, run_id: i64, event_id: i64) -> rusqlite::Result<Vec<i64>> {
    let mut chain = Vec::new();
    let mut cur = Some(event_id);
    while let Some(id) = cur {
        let row: Option<(i64, Option<i64>)> = conn
            .query_row(
                "SELECT id, parent_id FROM outbox WHERE id = ?1 AND run_id = ?2",
                params![id, run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        match row {
            Some((id, parent)) => {
                chain.push(id);
                cur = parent;
                if chain.len() > 100_000 {
                    // Defensive bound; smaller-id parents make this
                    // unreachable on well-formed chains.
                    break;
                }
            }
            None => break,
        }
    }
    chain.reverse();
    Ok(chain)
}

/// Mark a pending outbox row delivered (idempotent: delivering twice is a
/// no-op kept at the first delivered_at). Uses `RETURNING` so the audit row
/// needs no second lookup.
pub(crate) fn deliver(conn: &Connection, id: i64, now: i64) -> rusqlite::Result<()> {
    let run_id: Option<i64> = conn
        .query_row(
            "UPDATE outbox SET status = 'delivered', delivered_at = COALESCE(delivered_at, ?2)
              WHERE id = ?1 AND status = 'pending'
              RETURNING run_id",
            params![id, now],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    if let Some(run_id) = run_id {
        audit_write(
            conn,
            run_id,
            &format!("outbox:{id}"),
            AuditStatus::Ok,
            "delivered",
        );
    }
    Ok(())
}

// ── the steering inbox: the bounded write + its drain read ────────────────

/// The steering inbox write, shared by `POST .../steering` and the inbound
/// Signal webhook (`[case N] ...` messages). Drop-oldest cap + enqueue +
/// presence touch in ONE tx on one connection so the inbox bound can never
/// race past 100. `payload` is the ALREADY-sanitized JSON envelope.
pub(crate) fn enqueue_steering_tx(
    tx: &Connection,
    id: i64,
    domain: &str,
    payload: &str,
    actor: &str,
) -> Result<(), String> {
    let cnt: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM outbox WHERE run_id=?1 AND topic='steering'",
            params![id],
            |r| r.get(0),
        )
        .map_err(|e| format!("{e}"))?;
    if cnt >= 100 {
        tx.execute(
            "DELETE FROM outbox WHERE id IN (SELECT id FROM outbox WHERE run_id=?1 AND topic='steering' ORDER BY id ASC LIMIT ?2)",
            params![id, cnt - 99],
        )
        .map_err(|e| format!("{e}"))?;
    }
    let now = chrono::Utc::now().timestamp();
    let key = format!("steering-{id}-{now}-{}", rand::random::<u32>());
    // Kernel writer (the `steering` topic is reserved vocabulary):
    // the approve-role gate + screen + bound live at the HTTP seam; this fn
    // is the only other legitimate minter of the topic.
    enqueue_child_kernel(
        tx,
        id,
        None,
        "steering",
        payload,
        &key,
        now,
        KernelOrigin::kernel(),
    )
    .map_err(|e| format!("{e}"))?;
    super::crew::touch_cranking(tx, domain, actor, Some(&format!("run:{id}")));
    Ok(())
}

/// The steering inbox read (the drain half): strictly-after `since`, oldest
/// first, bounded at 100 — the read side of [`enqueue_steering_tx`]'s cap.
pub(crate) fn steering_inbox(
    conn: &Connection,
    run_id: i64,
    since: i64,
) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, payload_json FROM outbox
         WHERE run_id=?1 AND topic='steering' AND id > ?2 ORDER BY id ASC LIMIT 100",
    )?;
    let it = stmt.query_map(params![run_id, since], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(it.filter_map(Result::ok).collect())
}

// ── the lineage reads: the events surface and its derivatives ─────────────

/// One lineage event row: (id, parent_id, topic, payload_json, status).
pub(crate) type EventRowTuple = (i64, Option<i64>, String, String, String);

/// The events page: ordered by
/// id, bounded at 1000, `since` backfilling the reconnect gap with only rows
/// strictly after the given id (a resuming consumer replays nothing twice).
/// The caller owns branch narrowing (retain on [`branch_chain`]'s result).
pub(crate) fn events_page(
    conn: &Connection,
    run_id: i64,
    since: Option<i64>,
) -> rusqlite::Result<Vec<EventRowTuple>> {
    let mut stmt = conn.prepare(
        "SELECT id, parent_id, topic, payload_json, status FROM outbox
          WHERE run_id = ?1 AND (?2 IS NULL OR id > ?2) ORDER BY id ASC LIMIT 1000",
    )?;
    let it = stmt.query_map(params![run_id, since], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    it.collect()
}

/// The run's FULL event chain, uncapped: (id, topic, payload_json). The
/// context derivation budgets field counts downstream — the cap lives
/// there, not here.
pub(crate) fn events_all(
    conn: &Connection,
    run_id: i64,
) -> rusqlite::Result<Vec<(i64, String, String)>> {
    let mut stmt = conn
        .prepare("SELECT id, topic, payload_json FROM outbox WHERE run_id = ?1 ORDER BY id ASC")?;
    let it = stmt.query_map(params![run_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    it.collect()
}

/// One event of ONE run: (topic, payload_json). `None` when the id does not
/// exist ON this run — the run-scoping predicate is the point.
pub(crate) fn event_of_run(
    conn: &Connection,
    event_id: i64,
    run_id: i64,
) -> rusqlite::Result<Option<(String, String)>> {
    conn.query_row(
        "SELECT topic, payload_json FROM outbox WHERE id=?1 AND run_id=?2",
        params![event_id, run_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

/// The run's root event id (0 when the chain has not started — the caller
/// treats root 0 as "no root snapshot").
pub(crate) fn root_event_id(conn: &Connection, run_id: i64) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MIN(id), 0) FROM outbox WHERE run_id=?1",
        params![run_id],
        |r| r.get(0),
    )
}

/// The run's opening event payload, if any.
pub(crate) fn first_event_payload(
    conn: &Connection,
    run_id: i64,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT payload_json FROM outbox WHERE run_id=?1 ORDER BY id ASC LIMIT 1",
        params![run_id],
        |r| r.get(0),
    )
    .optional()
}

/// Step-event topics for the handoff packet: everything EXCEPT checkpoints,
/// bounded at 200 (row errors skip — the packet is best-effort assembled).
pub(crate) fn non_checkpoint_topics(
    conn: &Connection,
    run_id: i64,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT topic FROM outbox
          WHERE run_id=?1 AND topic != 'workflow/checkpoint' ORDER BY id LIMIT 200",
    )?;
    let it = stmt.query_map(params![run_id], |r| r.get::<_, String>(0))?;
    Ok(it.filter_map(Result::ok).collect())
}

/// The LATEST checkpoint payload of the run, if any (newest wins).
pub(crate) fn latest_checkpoint_payload(
    conn: &Connection,
    run_id: i64,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT payload_json FROM outbox
          WHERE run_id=?1 AND topic='workflow/checkpoint' ORDER BY id DESC LIMIT 1",
        params![run_id],
        |r| r.get(0),
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn
    }

    #[test]
    fn outbox_idempotent_by_key() {
        let conn = db();
        let (first, id1) = enqueue(&conn, 1, "intake", r#"{"a":1}"#, "pip-1", 1).unwrap();
        assert!(first);
        let (replay, id2) = enqueue(&conn, 1, "intake", r#"{"a":2}"#, "pip-1", 1).unwrap();
        assert!(!replay, "same key replays as a no-op");
        assert_eq!(id1, id2, "the replay resolves the surviving row's id");
        // Only one row exists; payload unchanged (the first write wins).
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE idempotency_key='pip-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "replayed key must not create a second row");
        let status: String = conn
            .query_row(
                "SELECT status FROM outbox WHERE idempotency_key='pip-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "pending");

        let id: i64 = conn
            .query_row("SELECT id FROM outbox", [], |r| r.get(0))
            .unwrap();
        deliver(&conn, id, 2).unwrap();
        deliver(&conn, id, 3).unwrap(); // second deliver is a no-op
        let (status, at): (String, i64) = conn
            .query_row(
                "SELECT status, delivered_at FROM outbox WHERE id=?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "delivered");
        assert_eq!(at, 2, "delivered_at must stay the first delivery");
    }

    #[test]
    fn outbox_enqueue_audits_once_not_on_replay() {
        let conn = db();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('global', 'interview', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        assert!(
            enqueue(&conn, 1, "intake", r#"{"a":1}"#, "k-9", 1)
                .unwrap()
                .0
        );
        assert!(
            !enqueue(&conn, 1, "intake", r#"{"a":2}"#, "k-9", 1)
                .unwrap()
                .0
        );
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'workflow'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "the replayed enqueue must not add a second audit row");
    }

    /// Children parent verbatim; a replayed child key is a
    /// no-op that audits once and keeps the ORIGINAL parent (first write wins).
    #[test]
    fn enqueue_child_parents_and_audits_once() {
        let conn = db();
        let (_, root) = enqueue(&conn, 1, "workflow/log", "{}", "root-k", 1).unwrap();
        let (created, child) =
            enqueue_child(&conn, 1, Some(root), "workflow/log", "{}", "child-k", 2).unwrap();
        assert!(created);
        let stored: Option<i64> = conn
            .query_row(
                "SELECT parent_id FROM outbox WHERE idempotency_key='child-k'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, Some(root));
        // Replay with a DIFFERENT parent: the first write wins.
        let (replay, again) =
            enqueue_child(&conn, 1, None, "workflow/log", "{}2", "child-k", 3).unwrap();
        assert!(!replay);
        assert_eq!(again, child);
        let stored: Option<i64> = conn
            .query_row(
                "SELECT parent_id FROM outbox WHERE idempotency_key='child-k'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, Some(root), "replay never re-parents");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'workflow'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2, "root + one child audit; the replay adds none");
    }

    /// verify_outbox_lineage_detects_orphans_and_cycles — fixture rows break
    /// each law (orphan parent, cross-run parent, forward id) and the check
    /// reports them; a well-formed chain (incl. all-NULL legacy roots) passes.
    #[test]
    fn verify_outbox_lineage_detects_orphans_and_cycles() {
        let conn = db();
        // Legacy flat run: every row a root — verify passes.
        enqueue(&conn, 1, "t", "{}", "l1", 1).unwrap();
        enqueue_child(&conn, 1, None, "t", "{}", "l2", 2).unwrap();
        assert!(verify_outbox_lineage(&conn, 1).unwrap());

        // Well-formed chain root -> child -> grandchild.
        let (_, r) = enqueue(&conn, 2, "t", "{}", "r", 1).unwrap();
        let (_, c) = enqueue_child(&conn, 2, Some(r), "t", "{}", "c", 2).unwrap();
        enqueue_child(&conn, 2, Some(c), "t", "{}", "g", 3).unwrap();
        assert!(verify_outbox_lineage(&conn, 2).unwrap());
        assert_eq!(
            branch_chain(&conn, 2, g_id(&conn, "g")).unwrap(),
            vec![r, c, g_id(&conn, "g")],
            "the ancestor chain reads root-first"
        );

        // Orphan: parent id points at nothing. (The FK would refuse this
        // write on a live path; the fixture disables it to place the corrupt
        // row and prove verify DETECTS it rather than the constraint.)
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        conn.execute(
            "INSERT INTO outbox(run_id, topic, payload_json, status, idempotency_key, created_at, parent_id)
             VALUES (3, 't', '{}', 'pending', 'orphan', 1, 99999)",
            [],
        )
        .unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        assert!(!verify_outbox_lineage(&conn, 3).unwrap());

        // Cross-run parent: same-run law violated even though the id exists.
        // Placed by raw SQL — the live gate refuses this write, so
        // the fixture bypasses it exactly like the orphan case above to
        // prove verify DETECTS legacy violations rather than the gate.
        let (_, other) = enqueue(&conn, 5, "t", "{}", "other", 1).unwrap();
        conn.execute(
            "INSERT INTO outbox(run_id, topic, payload_json, status, idempotency_key, created_at, parent_id)
             VALUES (4, 't', '{}', 'pending', 'xrun', 1, ?1)",
            rusqlite::params![other],
        )
        .unwrap();
        assert!(!verify_outbox_lineage(&conn, 4).unwrap());

        // Forward link (parent has a LARGER id): the stored rows disobey the
        // by-construction ordering, so it fails exactly like a cycle would.
        conn.execute(
            "INSERT INTO outbox(id, run_id, topic, payload_json, status, idempotency_key, created_at)
             VALUES (5001, 6, 't', '{}', 'pending', 'fwd-parent', 2)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO outbox(id, run_id, topic, payload_json, status, idempotency_key, created_at, parent_id)
             VALUES (5000, 6, 't', '{}', 'pending', 'fwd-child', 1, 5001)",
            [],
        )
        .unwrap();
        assert!(!verify_outbox_lineage(&conn, 6).unwrap());
    }

    fn g_id(conn: &Connection, key: &str) -> i64 {
        conn.query_row(
            "SELECT id FROM outbox WHERE idempotency_key=?1",
            params![key],
            |r| r.get(0),
        )
        .unwrap()
    }

    // ── v1.28.63 "Wardline": the reserved-topic gate (X-W1/X-W2) ───────────

    /// The vocabulary semantics the const comment declares: `channel/` and
    /// `workflow/valet` match as prefixes, `steering` EXACTLY, and the
    /// engine-facing topics stay unreserved.
    #[test]
    fn reserved_vocabulary_semantics() {
        assert!(topic_is_reserved("channel/out"));
        assert!(topic_is_reserved("channel/ping"));
        assert!(topic_is_reserved("channel/anything-new"));
        assert!(topic_is_reserved("workflow/valet-due"));
        assert!(topic_is_reserved("workflow/valet/other"));
        assert!(topic_is_reserved("steering"));
        // steering is EXACT — spelled only as the inbox literal.
        assert!(!topic_is_reserved("steering-archive"));
        // The agent-facing families stay open.
        assert!(!topic_is_reserved("workflow/log"));
        assert!(!topic_is_reserved("workflow/checkpoint"));
        assert!(!topic_is_reserved("crm/case/closed"));
        assert!(!topic_is_reserved("intake"));
    }

    /// The events-route shape: enqueue_child refuses every reserved family
    /// with the TYPED error and lands no row (the forge paths are dead).
    #[test]
    fn enqueue_child_refuses_reserved_topics() {
        let conn = db();
        for topic in [
            "channel/out",
            "channel/ping",
            "steering",
            "workflow/valet-due",
        ] {
            let err = enqueue_child(&conn, 1, None, topic, "{}", "forge-1", 1).unwrap_err();
            assert!(
                matches!(
                    err,
                    OutboxError::ReservedTopic { topic: ref t } if t == topic
                ),
                "{topic} must refuse typed: {err:?}"
            );
        }
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "a refused forge lands no row");
    }

    /// A parent from another run (or a nonexistent id) refuses with
    /// the TYPED error and lands no row; a same-run parent admits.
    #[test]
    fn post_event_refuses_foreign_parent() {
        let conn = db();
        let (_, p1) = enqueue(&conn, 1, "intake", "{}", "w10-a", 1).unwrap();
        let (_, p2) = enqueue(&conn, 2, "intake", "{}", "w10-b", 1).unwrap();
        let err = enqueue_child(&conn, 1, Some(p2), "note", "{}", "w10-c", 2).unwrap_err();
        assert!(
            matches!(err, OutboxError::ForeignParent { parent, run } if parent == p2 && run == 1),
            "foreign parent must refuse typed: {err:?}"
        );
        let err = enqueue_child(&conn, 1, Some(999_999), "note", "{}", "w10-d", 2).unwrap_err();
        assert!(
            matches!(err, OutboxError::ForeignParent { .. }),
            "missing parent must refuse typed: {err:?}"
        );
        let (created, _) = enqueue_child(&conn, 1, Some(p1), "note", "{}", "w10-e", 2).unwrap();
        assert!(created, "same-run parent admits");
    }

    /// The kernel writers still mint reserved rows through the token — the
    /// gate is a fence around the vocabulary, not a ban on it.
    #[test]
    fn kernel_writers_still_mint_reserved_rows() {        let conn = db();
        for (topic, key) in [
            ("channel/out", "k-out"),
            ("steering", "k-steer"),
            ("workflow/valet-due", "k-valet"),
        ] {
            let (created, _) =
                enqueue_child_kernel(&conn, 1, None, topic, "{}", key, 1, KernelOrigin::kernel())
                    .unwrap();
            assert!(created, "kernel mint of {topic} must land");
        }
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 3);
    }

    /// The kernel steering writer still lands its reserved-topic rows after
    /// the gate (X-W2): the approve-role gate + screen + bound live at the
    /// HTTP seam; this fn is the topic's only other legitimate minter.
    #[test]
    fn kernel_steering_still_enqueues() {
        let conn = db();
        enqueue_steering_tx(
            &conn,
            7,
            "global",
            r#"{"message":"prefer the cheaper SKU when specs match"}"#,
            "operator",
        )
        .unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE run_id = 7 AND topic = 'steering'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "the kernel steering write must land");
    }

    /// The rusqlite-facing conversion (append_lineage + fixed-topic service
    /// cores keep `rusqlite::Result` plumbing): a reserved refusal converts
    /// to a LOUD constraint error, never a silent drop. For a hard-coded
    /// non-reserved topic the branch is unreachable — exercised here through
    /// the raw wrapper a future caller would misuse.
    #[test]
    fn reserved_refusal_converts_to_loud_sql_error() {
        let conn = db();
        let err = enqueue(&conn, 1, "steering", "{}", "loud-1", 1).unwrap_err();
        match err {
            rusqlite::Error::SqliteFailure(_, msg) => {
                assert!(msg.unwrap_or_default().contains("reserved topic"));
            }
            other => panic!("expected a loud SQL-shaped refusal, got {other:?}"),
        }
    }
}
