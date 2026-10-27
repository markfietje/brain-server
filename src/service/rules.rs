//! The `rules`/`rule_rates` tables' ONE door — C1's named finding
//! `the_rules_insert_door_is_missing`, closed (C2 G-3, plan 28 §4's K7).
//!
//! Until this module the tables had a schema and no writer: no CLI verb, no
//! route, the migration and the migrate-rehearse list their only
//! references. C1's rehearsal had to land its fixture's rows by the
//! operator's DIRECT SQL, and said so on the record. This is that door:
//!
//! * **a local operator act** — the caller names the DB (`brain rules
//!   insert|list --db PATH`, default the data root's), exactly the roster
//!   door's posture. NO HTTP route: a wire writer would let any JWT-bearing
//!   principal mint jurisdiction law, and no measured need crosses that
//!   line.
//! * **one row per natural key** — `(jurisdiction, subject, rule_key)`.
//!   An insert at a revision HIGHER than the stored row's replaces it in
//!   place (the revision is the optimistic-concurrency token); at the same
//!   or lower revision it is REFUSED naming both numbers, so a replay lands
//!   clean (the C1 fixture's own idempotency law) and a stale write cannot
//!   clobber fresh law. `superseded_by` stays NULL: this door keeps one
//!   living row per key, not a lineage.
//! * **one audit row per insert** — kind [`crate::audit::AuditKind::Rules`],
//!   actor named by the caller (`operator` for the CLI; a pack import
//!   attributes `plugin:<pack-id>`), target the hashed natural key, the
//!   body riding the chain as a hash only.
//!
//! `rule_rates` has no writer yet: the rate JSON's consumer is the decision
//! evaluation seam, and it gets its own round when that seam names its
//! shape. The door does not guess it.

use rusqlite::{Connection, OptionalExtension, params};

use crate::packs::RuleRow;

/// One door row as read back: the stored rule plus its row id.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredRule {
    pub id: i64,
    pub jurisdiction: String,
    pub subject: String,
    pub rule_key: String,
    pub body: String,
    pub source_ref: String,
    pub effective_at: i64,
    pub reviewed_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub revision: i64,
}

/// What one insert act did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    /// The row landed (new or revised): the audit row is written.
    Landed(i64),
    /// The natural key already lives at THIS revision: nothing landed, no
    /// audit row — the C1 fixture's replay law ("a replay over an
    /// already-mounted fixture inserts nothing"). The DIRECT CLI verb
    /// surfaces this as a refusal (an operator re-running the same law
    /// should be told, not silently nodded at); the PACK INSTALLER treats
    /// it as the skip it is, so a pack upgrade carrying unchanged rules
    /// imports cleanly.
    Current,
}

/// Insert (or revise) one rule row, reporting what happened.
pub fn insert_outcome(
    conn: &Connection,
    actor: &str,
    row: &RuleRow,
) -> Result<InsertOutcome, String> {
    let existing: Option<(i64, i64)> = conn
        .query_row(
            "SELECT id, revision FROM rules
             WHERE jurisdiction = ?1 AND subject = ?2 AND rule_key = ?3",
            params![row.jurisdiction, row.subject, row.rule_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("rules read: {e}"))?;

    let id = match existing {
        Some((id, stored_revision)) => {
            if row.revision == stored_revision {
                return Ok(InsertOutcome::Current);
            }
            if row.revision < stored_revision {
                return Err(format!(
                    "rule '{}:{}:{}' is already present at revision {stored_revision} \
                     (offered {}) — only a HIGHER revision revises law, and a stale \
                     write cannot clobber fresh law",
                    row.jurisdiction, row.subject, row.rule_key, row.revision
                ));
            }
            conn.execute(
                "UPDATE rules SET body = ?2, source_ref = ?3, effective_at = ?4,
                     reviewed_at = ?5, expires_at = ?6, revision = ?7
                 WHERE id = ?1",
                params![
                    id,
                    row.body,
                    row.source_ref,
                    row.effective_at,
                    row.reviewed_at,
                    row.expires_at,
                    row.revision,
                ],
            )
            .map_err(|e| format!("rules update: {e}"))?;
            id
        }
        None => {
            conn.execute(
                "INSERT INTO rules(jurisdiction, subject, rule_key, body, source_ref,
                     effective_at, reviewed_at, expires_at, revision, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    row.jurisdiction,
                    row.subject,
                    row.rule_key,
                    row.body,
                    row.source_ref,
                    row.effective_at,
                    row.reviewed_at,
                    row.expires_at,
                    row.revision,
                    row.effective_at, // created_at: the row's own clock start
                ],
            )
            .map_err(|e| format!("rules insert: {e} (has the migration run?)"))?;
            conn.last_insert_rowid()
        }
    };

    // one audit row per LANDED act — kind-filtered consumers find
    // jurisdiction-law writes without mining; the body rides the chain as a
    // hash only. A skipped replay writes no evidence, because nothing
    // happened.
    crate::audit::record(
        conn,
        crate::audit::AuditKind::Rules,
        actor,
        &format!("rule:{}:{}:{}", row.jurisdiction, row.subject, row.rule_key),
        crate::audit::AuditStatus::Ok,
        &format!("revision:{}", row.revision),
    );
    Ok(InsertOutcome::Landed(id))
}

/// The DIRECT verb's insert: strict — a same-revision replay is a refusal
/// naming the law, not a silent skip.
pub fn insert(conn: &Connection, actor: &str, row: &RuleRow) -> Result<i64, String> {
    match insert_outcome(conn, actor, row)? {
        InsertOutcome::Landed(id) => Ok(id),
        InsertOutcome::Current => Err(format!(
            "rule '{}:{}:{}' is already present at revision {} — only a HIGHER \
             revision revises law, and a replay of the same revision is refused",
            row.jurisdiction, row.subject, row.rule_key, row.revision
        )),
    }
}

/// The door's list: every living row, natural-key ordered.
pub fn list(conn: &Connection) -> Result<Vec<StoredRule>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, jurisdiction, subject, rule_key, body, source_ref,
                    effective_at, reviewed_at, expires_at, revision
             FROM rules
             ORDER BY jurisdiction, subject, rule_key",
        )
        .map_err(|e| format!("rules read: {e} (has the migration run?)"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(StoredRule {
                id: r.get(0)?,
                jurisdiction: r.get(1)?,
                subject: r.get(2)?,
                rule_key: r.get(3)?,
                body: r.get(4)?,
                source_ref: r.get(5)?,
                effective_at: r.get(6)?,
                reviewed_at: r.get(7)?,
                expires_at: r.get(8)?,
                revision: r.get(9)?,
            })
        })
        .map_err(|e| format!("rules read: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("rules read: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("db");
        crate::migration::run_migration(&mut conn, 512).expect("migrate fixture");
        conn
    }

    fn row(key: &str, revision: i64) -> RuleRow {
        RuleRow {
            jurisdiction: "ph".to_string(),
            subject: "support-tickets".to_string(),
            rule_key: key.to_string(),
            body: "Resolved ticket corpus is retained 90 days post-resolution.".to_string(),
            source_ref: "c1-support-fixture/packs/support-escalation-pack.json#retention"
                .to_string(),
            effective_at: 1_700_000_000,
            reviewed_at: Some(1_700_000_000),
            expires_at: None,
            revision,
        }
    }

    /// C1's fixture rows (plans repo `c1-support-fixture/sql/rules-support.sql`),
    /// verbatim in data — the register sentence honored: they ARE the door's
    /// first regression fixtures.
    fn c1_fixture_rows() -> Vec<RuleRow> {
        let mount_epoch: i64 = 1_769_875_200; // the runbook's mount instant, fixed for the fixture
        vec![
            RuleRow {
                jurisdiction: "ph".into(),
                subject: "support-tickets".into(),
                rule_key: "ticket_retention".into(),
                body: "Resolved ticket corpus (resolution notes, chat transcripts) is \
                       retained 90 days post-resolution, then purged by the retention \
                       clock; a legal hold suspends the clock for the held rows only."
                    .into(),
                source_ref: "c1-support-fixture/packs/support-escalation-pack.json#retention"
                    .into(),
                effective_at: mount_epoch,
                reviewed_at: Some(mount_epoch),
                expires_at: None,
                revision: 1,
            },
            RuleRow {
                jurisdiction: "ph".into(),
                subject: "support-tickets".into(),
                rule_key: "breach_posture".into(),
                body: "A suspected personal-data breach inside ticket content is \
                       escalated to the supervisor queue immediately and the breach \
                       channel (webhook) fires; customer notification follows the \
                       72-hour posture."
                    .into(),
                source_ref: "c1-support-fixture/packs/support-escalation-pack.json#breach".into(),
                effective_at: mount_epoch,
                reviewed_at: Some(mount_epoch),
                expires_at: None,
                revision: 1,
            },
            RuleRow {
                jurisdiction: "ph".into(),
                subject: "support-tickets".into(),
                rule_key: "dsar_clock".into(),
                body: "A customer access or erasure request over ticket content is \
                       answered through the DSAR flow within the statutory window; the \
                       ticket's retention clock yields to the request until it closes."
                    .into(),
                source_ref: "c1-support-fixture/packs/support-escalation-pack.json#dsar".into(),
                effective_at: mount_epoch,
                reviewed_at: Some(mount_epoch),
                expires_at: None,
                revision: 1,
            },
        ]
    }

    #[test]
    fn the_c1_fixture_rows_land_through_the_door_and_audit_one_row_each() {
        let conn = db();
        for r in c1_fixture_rows() {
            insert(&conn, "operator", &r).expect("the fixture row lands");
        }
        let stored = list(&conn).unwrap();
        assert_eq!(stored.len(), 3, "the three fixture rows live");
        assert_eq!(
            stored
                .iter()
                .map(|r| r.rule_key.as_str())
                .collect::<Vec<_>>(),
            vec!["breach_posture", "dsar_clock", "ticket_retention"]
        );
        let audited: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='rules'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audited, 3, "one audit row per insert act");
    }

    #[test]
    fn a_replay_of_the_same_revision_is_refused_and_lands_nothing() {
        let conn = db();
        let r = row("ticket_retention", 1);
        insert(&conn, "operator", &r).unwrap();
        let err = insert(&conn, "operator", &r).unwrap_err();
        assert!(err.contains("already present at revision 1"), "{err}");
        assert_eq!(list(&conn).unwrap().len(), 1, "the replay landed nothing");
        let audited: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='rules'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audited, 1, "a refusal is not an act — no audit row");
    }

    #[test]
    fn a_higher_revision_replaces_in_place_a_lower_one_is_refused() {
        let conn = db();
        insert(&conn, "operator", &row("ticket_retention", 1)).unwrap();
        let mut revised = row("ticket_retention", 2);
        revised.body = "Amended: retention shortened to 30 days.".to_string();
        let id = insert(&conn, "operator", &revised).unwrap();
        let stored = list(&conn).unwrap();
        assert_eq!(stored.len(), 1, "the door keeps ONE living row per key");
        assert_eq!(stored[0].id, id, "the row is revised in place");
        assert_eq!(stored[0].revision, 2);
        assert_eq!(stored[0].body, revised.body);

        let mut stale = row("ticket_retention", 1);
        stale.body = "a stale write".to_string();
        let err = insert(&conn, "operator", &stale).unwrap_err();
        assert!(err.contains("already present at revision 2"), "{err}");
    }

    #[test]
    fn the_actor_is_attributable_operator_or_pack() {
        let conn = db();
        insert(&conn, "operator", &row("ticket_retention", 1)).unwrap();
        insert(&conn, "plugin:support-pack", &row("breach_posture", 1)).unwrap();
        let actors: Vec<String> = conn
            .prepare("SELECT actor FROM audit_events WHERE kind='rules' ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(actors, vec!["operator", "plugin:support-pack"]);
    }
}
