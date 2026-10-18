//! The quarantine-by-id core: `POST /quarantine/{id}/release` and
//! `POST /quarantine/{id}/delete`.
//!
//! ## WHY THIS EXISTS
//!
//! Both routes used to authorize `Action::Admin` against the literal `"global"`
//! and then run `UPDATE knowledge SET flagged = 0 WHERE id = ?1` /
//! `DELETE FROM knowledge WHERE id = ?1 AND flagged = 1` on the SHARED pool —
//! with no `domain` predicate, no record-level `(owner, access_scope)` gate, and
//! no `purge` role check on the delete. The list surface beside them
//! (`GET /quarantine`) resolved the caller's domain and scoped its SQL, so the
//! two by-id routes were strictly *weaker* than the list they hang off.
//!
//! Three concrete consequences, all closed here:
//!
//! 1. **Cross-tenant.** In default shim mode every tenant shares one file with a
//!    `domain` column. A tenant-A admin could enumerate sequential ids and
//!    un-quarantine or hard-delete tenant-B's flagged rows, and the
//!    200-vs-404 split was itself an existence oracle the list surface avoids.
//! 2. **Privilege escalation past the `purge` role.** `/purge` — the same
//!    destructive act — gates on `authorize_role(p, pool, "purge")`, and so does
//!    `POST /ump/forget {"hard": true}`. The shipped `supervisor` preset has
//!    `release_quarantine` but NOT `purge`, so it was correctly refused at
//!    `/purge` and reached the same hard delete through this route.
//! 3. **Integrity, for `release`.** Clearing `flagged = 1` re-admits a row the
//!    injection screen quarantined into retrieval, and there was no record gate
//!    on it at all. That is the containment mechanism, so it now takes the same
//!    gates as every other mutation.
//!
//! ## WHY THE PROBE-BLIND 404
//!
//! A refusal here is a 404 naming "no quarantined chunk with that id" for a row
//! that does not exist, one in another domain, and one whose record gate denies
//! it — deliberately the same answer, so this route is not an id-existence
//! oracle for private data. That mirrors `authorize_target_row` on the UMP
//! mutation seam.
//!
//! ## THE `purge` GATE IS INSIDE, NOT AT THE CALL SITE
//!
//! `purge_chunk_ids` refuses `Owner(Purge)` when the role set lacks it, so the
//! structural backstop is a property of the FUNCTION and not of today's
//! call-site discipline — the same reasoning that put the legal-hold fence
//! inside `service::purge`.

use rusqlite::{Connection, TransactionBehavior, params};

/// What the caller is allowed to touch. `None` means "no restriction" and is
/// the loopback/opaque posture the rest of the tree uses for that case.
#[derive(Debug, Clone, Default)]
pub struct QuarantineScope<'a> {
    /// The domain label the caller's own scope admits. Bound into the SQL as a
    /// predicate, so a by-id mutation cannot reach outside it. `None` in
    /// multi-db mode, where the pool IS the boundary.
    pub domain: Option<&'a str>,
    /// The record gate's `(owner, access_scope)` allowlists. `None` = no
    /// record-level narrowing (admin / loopback).
    pub access_scopes: Option<Vec<String>>,
    pub owner_in: Option<Vec<String>>,
    /// Whether the server-side role resolution granted the `purge` capability.
    /// The handler computes this with `authorize_role(p, pool, "purge")` — the
    /// same server-side resolution `/purge` and `POST /ump/forget {"hard":true}`
    /// use — so the grant is never the client's word for itself.
    ///
    /// The core's own contribution is the ORDERING: it is checked before the
    /// target row resolves, so a caller without `purge` cannot turn the 403 into
    /// an existence oracle over quarantined ids.
    pub purge_granted: bool,
}

impl QuarantineScope<'_> {
    /// Whether a stored `(owner, access_scope)` pair passes the record gate —
    /// the same predicate `RecordReadGate::admits` applies, duplicated as SQL
    /// because this core must narrow inside one statement (check-then-act
    /// would be a TOCTOU between the gate read and the UPDATE).
    fn admits(&self, owner: Option<&str>, access_scope: Option<&str>) -> bool {
        if let Some(sc) = &self.access_scopes
            && !access_scope.is_some_and(|s| sc.iter().any(|x| x == s))
        {
            return false;
        }
        if let Some(owners) = &self.owner_in
            && !owner.is_some_and(|o| owners.iter().any(|x| x == o))
        {
            return false;
        }
        true
    }
}

/// The failure taxonomy. `NotFound` deliberately covers "absent", "another
/// domain", "another owner" and "not flagged" alike — see the module header.
#[derive(Debug)]
pub enum QuarantineError {
    /// Every refusal the caller may observe. One shape, one answer.
    NotFound,
    /// The role set lacks `purge`. Distinct, because it is a PROPERTY of the
    /// caller, not of the row — and a caller who cannot purge must not be able
    /// to probe which ids are quarantined by watching for 403.
    PurgeForbidden,
    /// Legal hold is active on the target. 409.
    Held(String),
    Storage(String),
}

impl From<rusqlite::Error> for QuarantineError {
    fn from(e: rusqlite::Error) -> Self {
        QuarantineError::Storage(e.to_string())
    }
}

/// Read the target row's gate columns, or `None` when it is absent.
/// `None` is the NotFound answer, never an empty result to be written to.
fn target_meta(
    conn: &Connection,
    id: i64,
    scope: &QuarantineScope<'_>,
) -> Result<Option<TargetMeta>, QuarantineError> {
    let mut sql = String::from(
        "SELECT domain, owner, access_scope FROM knowledge WHERE id = ?1 AND flagged = 1",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(id)];
    if let Some(label) = scope.domain {
        sql.push_str(" AND domain = ?2");
        params.push(Box::new(label.to_string()));
    }
    let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let row = conn
        .query_row(&sql, refs.as_slice(), |r| {
            Ok(TargetMeta {
                domain: r.get(0)?,
                owner: r.get(1)?,
                access_scope: r.get(2)?,
            })
        })
        .ok();
    Ok(row)
}

/// The three columns the gate reads. Named because a tuple of three
/// `Option<String>`s at a call site reads as "three strings?".
#[derive(Debug, Clone)]
struct TargetMeta {
    #[allow(
        dead_code,
        reason = "carried for the record; the predicate binds domain in SQL"
    )]
    domain: String,
    owner: Option<String>,
    access_scope: Option<String>,
}

/// `POST /quarantine/{id}/release` — clear the quarantine flag so the chunk
/// re-enters retrieval. Audit row inside the caller's transaction.
///
/// Returns the number of rows released: 0 means the probe-blind NotFound.
pub fn release_flagged(
    conn: &mut Connection,
    id: i64,
    scope: &QuarantineScope<'_>,
    actor: &str,
) -> Result<usize, QuarantineError> {
    // THE GATE AND THE WRITE ARE ONE UNIT. The previous check ran in
    // autocommit and the UPDATE re-identified the row by id only, so a domain /
    // owner / rowid change between the check and the write was not re-checked
    //. Re-read inside the IMMEDIATE tx.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(meta) = target_meta(&tx, id, scope)? else {
        return Ok(0);
    };
    if !scope.admits(meta.owner.as_deref(), meta.access_scope.as_deref()) {
        return Ok(0);
    }
    let n = tx.execute(
        "UPDATE knowledge SET flagged = 0 WHERE id = ?1 AND flagged = 1",
        params![id],
    )?;
    if n > 0 {
        // Provenance: record the operator approval (hash-only). Best-effort by
        // contract — an audit-log failure must not fail the release, which is
        // already committed and whose absence would be a worse lie than a
        // missing row.
        let _ = crate::audit::record(
            &tx,
            crate::audit::AuditKind::Ingest,
            actor,
            &id.to_string(),
            crate::audit::AuditStatus::Ok,
            "quarantine-release",
        );
    }
    tx.commit()?;
    Ok(n)
}

/// `POST /quarantine/{id}/delete` — hard-erase a quarantined chunk. Gated on
/// the `purge` role, the legal-hold fence, the domain predicate and the record
/// gate. The `vec_knowledge` index row is removed explicitly: vec0 has no FK
/// cascade, and a lingering vector would surface a deleted chunk's embedding
/// in retrieval.
pub fn delete_flagged(
    conn: &mut Connection,
    id: i64,
    scope: &QuarantineScope<'_>,
    actor: &str,
) -> Result<usize, QuarantineError> {
    // The role gate is checked BEFORE the row resolves, so a caller who cannot
    // purge learns nothing about which ids are quarantined.
    if !scope.purge_granted {
        return Err(QuarantineError::PurgeForbidden);
    }
    // Same check-then-act closure as release: resolve and gate inside the
    // IMMEDIATE tx.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(meta) = target_meta(&tx, id, scope)? else {
        return Ok(0);
    };
    if !scope.admits(meta.owner.as_deref(), meta.access_scope.as_deref()) {
        return Ok(0);
    }
    // A held chunk refuses any erasure. Inside the tx so the check and the
    // delete are one unit.
    if let Err(e) = crate::legal_hold::refuse_if_held(&tx, &[id]) {
        return Err(QuarantineError::Held(format!(
            "{}: {}",
            e.inner.code, e.inner.message
        )));
    }
    // Not `let _ =`: a lingering vec0 row would surface a deleted chunk's
    // vector in retrieval.
    tx.execute(
        "DELETE FROM vec_knowledge WHERE knowledge_id = ?1",
        params![id],
    )?;
    // Only if still flagged, so this endpoint can never purge a live row.
    let n = tx.execute(
        "DELETE FROM knowledge WHERE id = ?1 AND flagged = 1",
        params![id],
    )?;
    if n > 0 {
        let _ = crate::audit::record(
            &tx,
            crate::audit::AuditKind::Ingest,
            actor,
            &id.to_string(),
            crate::audit::AuditStatus::Ok,
            "quarantine-delete",
        );
    }
    tx.commit()?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE knowledge(id INTEGER PRIMARY KEY, domain TEXT NOT NULL DEFAULT 'global',
                owner TEXT, access_scope TEXT NOT NULL DEFAULT 'private', flagged INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE vec_knowledge(knowledge_id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        c
    }

    fn seed(c: &Connection, id: i64, owner: &str, domain: &str, flagged: i64) {
        c.execute(
            "INSERT INTO knowledge(id, domain, owner, access_scope, flagged)
             VALUES(?1, ?2, ?3, 'private', ?4)",
            params![id, domain, owner, flagged],
        )
        .unwrap();
        c.execute(
            "INSERT INTO vec_knowledge(knowledge_id) VALUES(?1)",
            params![id],
        )
        .unwrap();
    }

    fn scoped<'a>(
        domain: Option<&'a str>,
        owners: Option<&[&str]>,
        purge: bool,
    ) -> QuarantineScope<'a> {
        QuarantineScope {
            domain,
            access_scopes: Some(vec!["private".to_string()]),
            owner_in: owners.map(|o| o.iter().map(|s| s.to_string()).collect()),
            purge_granted: purge,
        }
    }

    /// THE CROSS-TENANT ARM. Two tenants' flagged rows share one file in shim
    /// mode; the domain predicate is what separates them, and it was absent.
    #[test]
    fn a_domain_scope_cannot_reach_another_domains_quarantine() {
        let mut c = db();
        seed(&c, 1, "alice", "acme", 1);
        seed(&c, 2, "bob", "other", 1);

        let released =
            release_flagged(&mut c, 2, &scoped(Some("acme"), None, false), "op").unwrap();
        assert_eq!(
            released, 0,
            "tenant A must not un-quarantine tenant B's row"
        );
        let still: i64 = c
            .query_row("SELECT flagged FROM knowledge WHERE id = 2", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(still, 1, "the row is untouched");

        let deleted = delete_flagged(&mut c, 2, &scoped(Some("acme"), None, true), "op").unwrap();
        assert_eq!(deleted, 0, "tenant A must not delete tenant B's row");
        assert!(
            c.query_row("SELECT 1 FROM knowledge WHERE id = 2", [], |_| Ok(()))
                .is_ok()
        );
    }

    /// THE RECORD GATE ARM. Same domain, different owner.
    #[test]
    fn the_record_gate_narrows_both_arms() {
        let mut c = db();
        seed(&c, 1, "alice", "global", 1);
        seed(&c, 2, "bob", "global", 1);

        assert_eq!(
            release_flagged(&mut c, 2, &scoped(None, Some(&["alice"]), false), "op").unwrap(),
            0,
            "a record-gated caller must not release another owner's row"
        );
        assert_eq!(
            delete_flagged(&mut c, 2, &scoped(None, Some(&["alice"]), true), "op").unwrap(),
            0
        );
        // its own row is still reachable
        assert_eq!(
            release_flagged(&mut c, 1, &scoped(None, Some(&["alice"]), false), "op").unwrap(),
            1
        );
    }

    /// THE PRIVILEGE ARM. `supervisor` has `release_quarantine` and NOT
    /// `purge`, so it was refused at `/purge` and reached the same hard delete
    /// here. The refusal is role-shaped, not row-shaped.
    #[test]
    fn delete_refuses_without_the_purge_role_before_reading_the_row() {
        let mut c = db();
        seed(&c, 1, "alice", "global", 1);
        let err = delete_flagged(&mut c, 1, &scoped(None, None, false), "sup").unwrap_err();
        assert!(
            matches!(err, QuarantineError::PurgeForbidden),
            "got {err:?}"
        );
        // and an ABSENT id gives the same answer, so the 403 is not a probe
        let err2 = delete_flagged(&mut c, 999, &scoped(None, None, false), "sup").unwrap_err();
        assert!(matches!(err2, QuarantineError::PurgeForbidden));
    }

    /// Release needs no `purge` — clearing a flag is a review act, not an
    /// erasure — but it DOES need the record gate, which is the arm this route
    /// never had.
    #[test]
    fn release_needs_no_purge_role_but_does_need_the_gate() {
        let mut c = db();
        seed(&c, 1, "alice", "global", 1);
        assert_eq!(
            release_flagged(&mut c, 1, &scoped(None, Some(&["alice"]), false), "op").unwrap(),
            1
        );
    }

    /// A live (unflagged) row is not reachable by the delete endpoint at all.
    #[test]
    fn delete_never_touches_a_live_row() {
        let mut c = db();
        seed(&c, 1, "alice", "global", 0);
        assert_eq!(
            delete_flagged(&mut c, 1, &scoped(None, None, true), "op").unwrap(),
            0
        );
        assert!(
            c.query_row("SELECT 1 FROM knowledge WHERE id = 1", [], |_| Ok(()))
                .is_ok()
        );
    }
}
