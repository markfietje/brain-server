//! Durable-step primitives, part 1: the transaction wrapper.
//!
//! Brain-server's write discipline is `BEGIN IMMEDIATE` — the exclusive start
//! that serializes the read-modify-write a governed-workflow transition needs
//! (the exact reason `audit::record_tenant` fights the chain-fork race with it).
//! [`WorkflowTx`] is that idiom as an RAII guard: commit on success, roll back on
//! any error DROP. A dropped guard must never leave a half-applied transition
//! that the caller then certifies as applied.

use rusqlite::{Connection, Transaction, TransactionBehavior};

/// A `BEGIN IMMEDIATE` transaction that commits on success or rolls back on
/// drop. Borrowed to the single owning `Connection` so no two transitions can
/// interleave on the same pool connection. Call `.commit()` when every write
/// in the transition has succeeded; dropping without commit rolls back.
pub(crate) struct WorkflowTx<'a> {
    tx: Option<Transaction<'a>>,
}

impl<'a> WorkflowTx<'a> {
    pub(crate) fn begin(conn: &'a mut Connection) -> rusqlite::Result<WorkflowTx<'a>> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Ok(WorkflowTx { tx: Some(tx) })
    }

    /// Access the raw transaction so callers can run INSERT/SELECT/etc.
    pub(crate) fn tx(&mut self) -> &mut Transaction<'a> {
        self.tx.as_mut().expect("workflow tx already consumed")
    }

    /// Commit the transition, consuming the guard. Returns the last inserted
    /// rowid so a caller can return it without a second statement.
    pub(crate) fn commit(mut self) -> rusqlite::Result<i64> {
        let id = self.tx().last_insert_rowid();
        self.tx
            .take()
            .expect("workflow tx already consumed")
            .commit()?;
        Ok(id)
    }
}

impl Drop for WorkflowTx<'_> {
    fn drop(&mut self) {
        // Commit already ran (guard consumed); anything else rolls back.
        if let Some(tx) = self.tx.take() {
            let _ = tx.rollback();
        }
    }
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
    fn workflow_tx_rolls_back_on_error() {
        let mut conn = db();
        // Seed a run.
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        tx.tx()
            .execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, status, created_at, updated_at)
                 VALUES ('global', 'interview', '{}', 'active', 1, 1)",
                [],
            )
            .unwrap();
        tx.commit().unwrap();

        // A transition that errors (invalid `kind`) must roll back — the guard
        // drops without commit, leaving no partial row.
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        let err = tx
            .tx()
            .execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, status, created_at, updated_at)
                 VALUES ('global', NULL, '{}', 'active', 1, 1)",
                [],
            )
            .is_err();
        assert!(err, "the failing write must error");
        drop(tx); // no commit → rollback

        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM workflow_runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "rollback must leave only the committed row");
    }
}
