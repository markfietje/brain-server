//! The governed-workflow substrate.
//!
//! Storage + primitives only; **no engine code**. The `*-core` crates
//! (interview-core, consensus-core, executor-core) write through
//! these primitives into the workflow tables; this module is their durable,
//! audit-backed contract with the server.
//!
//! Every mutating primitive emits its own [`crate::audit::AuditKind::Workflow`]
//! row — the fence holds of the FUNCTION, not call-site discipline (the
//! `purge_chunk_ids` precedent): a future engine crate cannot write a
//! workflow row that the audit chain cannot reconstruct. [`audit::record_tenant`]
//! is transaction-aware (SAVEPOINT when nested), so a transition + its audit
//! row commit atomically inside a [`tx::WorkflowTx`] and roll back together.
//!
//! `#![allow(dead_code)]` is a truthful allow (the connector-module precedent,
//! not a blanket one): this substrate ships one release before its consumers —
//! the engine crates that call [`tx::WorkflowTx`], [`outbox::enqueue`] etc.
//! land after this release. Every item is `pub(crate)` and covered by its own
//! test, so the clippy dead-code watchdog stays a real gate once the callers
//! arrive — any dead item removed then re-flags. Do not delete items of this
//! module until the crates consume it.

#![allow(dead_code)]

pub(crate) mod evidence;
pub(crate) mod outbox;
pub(crate) mod state;
pub(crate) mod tx;

use crate::audit::{record_tenant, AuditKind, AuditStatus};
use rusqlite::{params, Connection};

/// The audit actor for substrate writes. The engine crates drive the writes;
/// this layer stamps them with its own identity so an audit reader can
/// distinguish substrate-emitted rows from future handler-emitted ones.
const ACTOR: &str = "workflow";

/// Emit the `AuditKind::Workflow` row for a substrate write against `run_id`.
///
/// Best-effort by design (the [`crate::audit`] contract): `record_tenant`
/// never fails the primary write — a dropped row reads as a gap in the chain
/// and bumps the `/health` `audit_commit_failures` counter, never as a forged
/// continuation. The row carries the run's `domain` as tenant so per-tenant
/// scoping at the SQL layer holds; an unknown run (deleted mid-write) audits
/// against the `global` default.
fn audit_write(conn: &Connection, run_id: i64, target: &str, status: AuditStatus, detail: &str) {
    let tenant: String = conn
        .query_row(
            "SELECT domain FROM workflow_runs WHERE id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "global".to_string());
    record_tenant(
        conn,
        AuditKind::Workflow,
        ACTOR,
        target,
        status,
        detail,
        &tenant,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::verify_chain;
    use crate::workflow::outbox;
    use crate::workflow::state::{cas_update, CasError};
    use crate::workflow::tx::WorkflowTx;
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'interview', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn
    }

    fn workflow_rows(conn: &Connection) -> Vec<(String, String, String)> {
        let mut stmt = conn
            .prepare("SELECT kind, status, tenant_id FROM audit_events WHERE kind = 'workflow' ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }

    #[test]
    fn workflow_writes_are_audit_chained() {
        let conn = db();
        assert!(outbox::enqueue(&conn, 1, "intake", r#"{"a":1}"#, "k-1", 1).unwrap());
        let id: i64 = conn
            .query_row("SELECT id FROM outbox", [], |r| r.get(0))
            .unwrap();
        outbox::deliver(&conn, id, 2).unwrap();
        cas_update(&conn, 1, 0, r#"{"v":1}"#, "active", 3).unwrap();
        assert_eq!(
            cas_update(&conn, 1, 0, r#"{"v":2}"#, "active", 4).unwrap_err(),
            CasError::Stale { actual_revision: 1 }
        );

        assert_eq!(
            workflow_rows(&conn),
            vec![
                ("workflow".into(), "ok".into(), "acme".into()),
                ("workflow".into(), "ok".into(), "acme".into()),
                ("workflow".into(), "ok".into(), "acme".into()),
                ("workflow".into(), "denied".into(), "acme".into()),
            ],
            "enqueue + deliver + cas ok audit ok; a stale CAS audits denied"
        );
        assert!(verify_chain(&conn), "the audit chain still verifies");
    }

    #[test]
    fn audit_rolls_back_with_the_transition() {
        let mut conn = db();
        {
            let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
            outbox::enqueue(wtx.tx(), 1, "intake", r#"{"a":1}"#, "k-tx", 1).unwrap();
            // Drop without commit: transition AND its audit row must vanish.
        }
        assert!(
            workflow_rows(&conn).is_empty(),
            "a rolled-back transition must leave no audit row claiming it happened"
        );
    }
}
