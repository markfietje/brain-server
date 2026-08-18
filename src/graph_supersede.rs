//! Edge supersession — the write path that makes `relationships` truly
//! bi-temporal (SQL:2011 / the Snodgrass model, matching what Graphiti's
//! `EntityEdge` carries: a valid-time interval and a transaction-time
//! interval).
//!
//! Each relationship row is a *version of a belief* about the same
//! `(from_entity_id, to_entity_id, relation_type)` triple, carrying **four**
//! timestamps:
//!
//! * `valid_at` / `invalid_at` — **valid time**: when the fact was true in the
//!   world (event time). Rows may overlap here historically; a corrected
//!   belief does not rewrite the world-truth record.
//! * `created_at` — **transaction-time START**: when this version was learned.
//! * `superseded_at` — **transaction-time END**: when this version stopped
//!   being the current belief. `NULL` ⇒ the current belief.
//!
//! The invariant: **at most one version per triple is the current belief** —
//! the row whose `superseded_at IS NULL`. Every read that presents "what brain
//! currently believes" filters on that; every *historical* read (the
//! `/graph/relationships/{id}/history` surface) reads all versions.
//!
//! When a belief is corrected, the old version is **retired, never deleted or
//! rewritten**: its `superseded_at` is set to the transaction time of the new
//! version (old `valid_at`/`invalid_at`/`created_at` are preserved verbatim —
//! that is the point of versioning), and the corrected version is inserted with
//! its own window + `superseded_at = NULL`. This is the fail-closed upgrade of
//! the v1.4.0 `INSERT OR IGNORE` no-op (pre-v1.27.22, a corrected belief could
//! not coexist with the version it supersedes — the v1.27.22 BUG-1).
//!
//! [`resolve_edge_insert`] is the pure, unit-testable core (the
//! `page_decayed` idiom — a bare [`Connection`] drives every test). It returns
//! an [`EdgeAction`] so the caller records the transaction-time evidence in the
//! audit log. Supersession is idempotence-preserving: a re-ingest of an
//! unchanged window is a [`EdgeAction::SameWindow`] no-op (history is not
//! churned).

use rusqlite::{params, Connection, OptionalExtension};

/// What an edge ingest resolved to. The caller records the corresponding audit
/// event and decides whether the relationship count for the response moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeAction {
    /// No current version for this triple — a fresh belief was inserted.
    Created { id: i64 },
    /// The current version already carries exactly this window — no write, no
    /// history churn (re-ingest of an unchanged relation is a no-op by design).
    SameWindow { id: i64 },
    /// A newer belief corrected the current version: the old row's
    /// `superseded_at` was set to `now` (retired, preserved verbatim) and the
    /// corrected version was inserted as the new current belief. Both rows
    /// survive (supersession never deletes, never rewrites history).
    Superseded { old_id: i64, new_id: i64 },
}

/// The pure, unit-testable core: resolve one edge write.
///
/// * No current version → insert → [`EdgeAction::Created`].
/// * Current version, identical window → [`EdgeAction::SameWindow`] (no write;
///   the pre-v1.27.22 no-op behavior on unchanged data, preserved).
/// * Current version, differing window → [`EdgeAction::Superseded`]: retire the
///   current version at `now` (transaction-time END, old row preserved
///   verbatim) and insert the corrected version as the new current belief
///   (transaction-time START = `now`).
///
/// `now` is the transaction timestamp in the `%Y-%m-%d %H:%M:%S` (UTC) format
/// the rest of the temporal machinery compares as TEXT. Callers fetch it once
/// per transaction (SQL `strftime('%Y-%m-%d %H:%M:%S','now','utc')`) so
/// `old.superseded_at == new.created_at` exactly — a clean handoff the history
/// surface relies on.
///
/// `knowledge_id` anchors the new version to the memory chunk that produced it.
/// Fail-closed: any SQL error propagates (the caller rolls back the whole
/// ingest — the D-1 "never certify silence" rule applied to the graph); a
/// supersession is never a silent half-write.
pub fn resolve_edge_insert(
    conn: &Connection,
    from_id: i64,
    to_id: i64,
    kind: &str,
    knowledge_id: i64,
    window: (Option<&str>, Option<&str>),
    now: &str,
) -> Result<EdgeAction, rusqlite::Error> {
    // The current version of this triple, if any. At most one row has
    // `superseded_at IS NULL` (the supersession invariant); the guard orders
    // by id so a legacy corrupt DB (multiple open) still resolves
    // deterministically to the newest.
    let open: Option<(i64, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT id, valid_at, invalid_at FROM relationships
              WHERE from_entity_id = ?1 AND to_entity_id = ?2 AND relation_type = ?3
                AND superseded_at IS NULL
              ORDER BY id DESC LIMIT 1",
            params![from_id, to_id, kind],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;

    let Some((old_id, old_valid, old_invalid)) = open else {
        let _ = conn.execute(
            "INSERT INTO relationships \
                (from_entity_id, to_entity_id, relation_type, knowledge_id, \
                 valid_at, invalid_at, created_at, superseded_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![from_id, to_id, kind, knowledge_id, window.0, window.1, now],
        )?;
        let new_id = conn.last_insert_rowid();
        return Ok(EdgeAction::Created { id: new_id });
    };

    // Identical window ⇒ the current belief is unchanged ⇒ idempotent no-op
    // (history is not churned).
    let same_window = old_valid.as_deref() == window.0 && old_invalid.as_deref() == window.1;
    if same_window {
        return Ok(EdgeAction::SameWindow { id: old_id });
    }

    // Differing window ⇒ a corrected belief: retire the current version at the
    // transaction time `now` (transaction-time END; the old row's valid
    // interval + created_at are preserved verbatim) and insert the corrected
    // version as the new current belief. The handoff is exact:
    // old.superseded_at == new.created_at == `now`.
    conn.execute(
        "UPDATE relationships SET superseded_at = ?1 WHERE id = ?2",
        params![now, old_id],
    )?;
    conn.execute(
        "INSERT INTO relationships \
            (from_entity_id, to_entity_id, relation_type, knowledge_id, \
             valid_at, invalid_at, created_at, superseded_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
        params![from_id, to_id, kind, knowledge_id, window.0, window.1, now],
    )?;
    let new_id = conn.last_insert_rowid();
    Ok(EdgeAction::Superseded { old_id, new_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn schema(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE entities (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 name TEXT NOT NULL,
                 entity_type TEXT);
             CREATE TABLE relationships (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 from_entity_id INTEGER NOT NULL,
                 to_entity_id INTEGER NOT NULL,
                 relation_type TEXT NOT NULL,
                 knowledge_id INTEGER,
                 valid_at TIMESTAMP,
                 invalid_at TIMESTAMP,
                 created_at TIMESTAMP,
                 superseded_at TIMESTAMP);
             INSERT INTO entities (id, name) VALUES (1, 'a'), (2, 'b');",
        )
        .unwrap();
    }

    fn current_ids(conn: &Connection) -> Vec<i64> {
        let mut stmt = conn
            .prepare("SELECT id FROM relationships WHERE superseded_at IS NULL ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, i64>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    #[test]
    fn reingest_unchanged_relation_is_noop_no_write() {
        let conn = Connection::open_in_memory().unwrap();
        schema(&conn);
        let first = resolve_edge_insert(
            &conn,
            1,
            2,
            "works_at",
            10,
            (Some("2020-01-01 00:00:00"), None),
            "2023-01-01 00:00:00",
        )
        .unwrap();
        assert!(matches!(first, EdgeAction::Created { .. }));
        assert_eq!(current_ids(&conn).len(), 1);
        let old_id = current_ids(&conn)[0];

        let again = resolve_edge_insert(
            &conn,
            1,
            2,
            "works_at",
            10,
            (Some("2020-01-01 00:00:00"), None),
            "2024-01-01 00:00:00",
        )
        .unwrap();
        assert_eq!(again, EdgeAction::SameWindow { id: old_id });
        // No new row, no churn.
        assert_eq!(current_ids(&conn).len(), 1);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM relationships", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn new_window_retires_old_version_and_inserts_new_current() {
        let conn = Connection::open_in_memory().unwrap();
        schema(&conn);
        resolve_edge_insert(
            &conn,
            1,
            2,
            "works_at",
            10,
            (Some("2020-01-01 00:00:00"), None),
            "2020-02-01 00:00:00",
        )
        .unwrap();
        let old_id = current_ids(&conn)[0];

        let action = resolve_edge_insert(
            &conn,
            1,
            2,
            "works_at",
            11,
            (Some("2023-01-01 00:00:00"), None),
            "2023-03-01 00:00:00",
        )
        .unwrap();
        let EdgeAction::Superseded {
            old_id: retired,
            new_id,
        } = action
        else {
            panic!("expected Superseded, got {action:?}");
        };
        assert_eq!(retired, old_id);
        // Old version retired at the new transaction time, preserved verbatim.
        let row: (Option<String>, Option<String>, String) = conn
            .query_row(
                "SELECT valid_at, invalid_at, superseded_at FROM relationships WHERE id = ?1",
                params![old_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        // Valid interval + created_at untouched; only the transaction-time END set.
        assert_eq!(row.0.as_deref(), Some("2020-01-01 00:00:00"));
        assert_eq!(row.1, None);
        assert_eq!(row.2, "2023-03-01 00:00:00");
        // New version is the sole current belief.
        assert_eq!(current_ids(&conn), vec![new_id]);
    }

    #[test]
    fn supersession_never_deletes_old_row_and_handoff_is_exact() {
        let conn = Connection::open_in_memory().unwrap();
        schema(&conn);
        resolve_edge_insert(
            &conn,
            1,
            2,
            "works_at",
            10,
            (Some("2020-01-01 00:00:00"), None),
            "2020-02-01 00:00:00",
        )
        .unwrap();
        let old_id = current_ids(&conn)[0];
        let now = "2025-05-05 05:05:05";
        let action = resolve_edge_insert(
            &conn,
            1,
            2,
            "works_at",
            11,
            (Some("2022-06-01 00:00:00"), None),
            now,
        )
        .unwrap();
        let EdgeAction::Superseded { new_id, .. } = action else {
            panic!("expected Superseded");
        };

        // Both rows survive; both versions carry four timestamps.
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM relationships", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(current_ids(&conn).len(), 1);
        // old.superseded_at == new.created_at == now (the exact handoff the
        // history surface relies on).
        let (old_sup, new_created): (String, String) = conn
            .query_row(
                "SELECT o.superseded_at, n.created_at
                 FROM relationships o JOIN relationships n ON n.id = ?1
                 WHERE o.id = ?2",
                params![new_id, old_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(old_sup, now);
        assert_eq!(new_created, now);
    }

    #[test]
    fn backdated_overlap_preserves_both_valid_intervals() {
        let conn = Connection::open_in_memory().unwrap();
        schema(&conn);
        // Old version believed to be true 2020–2025.
        resolve_edge_insert(
            &conn,
            1,
            2,
            "works_at",
            10,
            (Some("2020-01-01 00:00:00"), Some("2025-01-01 00:00:00")),
            "2021-01-01 00:00:00",
        )
        .unwrap();
        let old_id = current_ids(&conn)[0];
        // Correction: it was actually true from 2023 (backdated onset).
        let action = resolve_edge_insert(
            &conn,
            1,
            2,
            "works_at",
            11,
            (Some("2023-01-01 00:00:00"), None),
            "2024-06-01 00:00:00",
        )
        .unwrap();
        let EdgeAction::Superseded { new_id, .. } = action else {
            panic!("expected Superseded");
        };
        // The old version's valid interval is NOT rewritten (it was the truth
        // believed at its time) — only transaction-time retirement. The two
        // valid intervals overlap (2023–2025), correctly, because they are
        // different beliefs, not a single row being edited.
        let (old_va, old_via): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT valid_at, invalid_at FROM relationships WHERE id = ?1",
                params![old_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(old_va.as_deref(), Some("2020-01-01 00:00:00"));
        assert_eq!(old_via.as_deref(), Some("2025-01-01 00:00:00"));
        assert_eq!(current_ids(&conn), vec![new_id]);
    }

    #[test]
    fn legacy_multiple_open_edges_resolve_to_newest() {
        let conn = Connection::open_in_memory().unwrap();
        schema(&conn);
        // Simulate a pre-invariant DB: two open rows for the same triple. The
        // resolver must deterministically pick the newest and supersede it.
        conn.execute_batch(
            "INSERT INTO relationships \
                (from_entity_id, to_entity_id, relation_type, valid_at, invalid_at, \
                 created_at, superseded_at) VALUES \
             (1, 2, 'rel', '2020-01-01 00:00:00', NULL, '2020-01-01 00:00:00', NULL), \
             (1, 2, 'rel', '2022-01-01 00:00:00', NULL, '2022-01-01 00:00:00', NULL);",
        )
        .unwrap();
        let action = resolve_edge_insert(
            &conn,
            1,
            2,
            "rel",
            99,
            (Some("2024-01-01 00:00:00"), None),
            "2024-02-01 00:00:00",
        )
        .unwrap();
        let EdgeAction::Superseded { old_id, new_id } = action else {
            panic!("expected Superseded");
        };
        // The newest prior (id 2) was retired at the transaction time; id 1 is a
        // legacy orphan open row (written before the invariant). We do not reap
        // it here — supersession must never delete — but note the M2 traversal
        // anti-join (`NOT EXISTS` a newer open edition) is what keeps reads
        // correct: it converges to the newest open edition regardless.
        assert_eq!(old_id, 2);
        // new_id is the newest open edition; the legacy orphan (1) does not
        // disappear, but the resolver's own reads/traversal pick `new_id`.
        assert_eq!(new_id, 3);
        assert!(current_ids(&conn).contains(&1));
        assert!(current_ids(&conn).contains(&3));
        let retired: String = conn
            .query_row(
                "SELECT superseded_at FROM relationships WHERE id = 2",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(retired, "2024-02-01 00:00:00");
    }
}
