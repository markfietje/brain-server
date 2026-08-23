//! The one `WorkflowHost` implementation: the SQLite pool adapter.
//!
//! Owns a single write lane — the substrate is single-writer by nature
//! (`BEGIN IMMEDIATE` serializes every transition), so the lane costs at most
//! one pooled connection and turns former lock contention into an explicit
//! fail-fast `Busy`. A unit of work opens the lane's transaction; mutating
//! calls issued while it is open join it, calls outside any unit run
//! standalone on their own pooled connection with identical audit semantics.
//! Reads never touch the lane. Honest ceiling: a `HostTx` leaked via
//! `mem::forget` holds the lane until the process ends; engines drive units
//! on one thread and commit or drop.

use std::sync::{Arc, Mutex, MutexGuard};

use brain_engine_sdk::host::tx::HostTxHandle;
use brain_engine_sdk::host::{AuditKind, AuditStatus, CasError, HostError, HostTx, WorkflowHost};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

type Pool = r2d2::Pool<SqliteConnectionManager>;
type PooledConn = r2d2::PooledConnection<SqliteConnectionManager>;

enum Lane {
    Idle,
    Active(Box<PooledConn>),
}

pub(crate) struct SqliteWorkflowHost {
    inner: Arc<HostInner>,
}

struct HostInner {
    pool: Pool,
    lane: Mutex<Lane>,
}

impl SqliteWorkflowHost {
    pub(crate) fn new(pool: Pool) -> Self {
        SqliteWorkflowHost {
            inner: Arc::new(HostInner {
                pool,
                lane: Mutex::new(Lane::Idle),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Lane> {
        // Only SQL runs under this lock (no engine code), so poison cannot be
        // a swallowed logic error; recover and continue rather than wedge
        // every future workflow write.
        self.inner.lane.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` on the unit's connection when a unit is open, else on a
    /// transient pooled connection. Outer error = infra (pool acquisition);
    /// inner error = the op's own result type.
    fn scoped<T, E>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, E>,
    ) -> Result<Result<T, E>, String> {
        let guard = self.lock();
        if let Lane::Active(conn) = &*guard {
            return Ok(f(conn));
        }
        drop(guard);
        let conn = self.pool_get()?;
        Ok(f(&conn))
    }

    fn pool_get(&self) -> Result<PooledConn, String> {
        self.inner.pool.get().map_err(|e| e.to_string())
    }

    /// Run a read-only closure on a pooled connection. The mediated-handler
    /// read seam (knowledge_suggest): hostcall handlers never touch the pool
    /// directly — they come through here, so lane discipline stays in one
    /// place.
    pub(crate) fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        let conn = self.pool_get()?;
        f(&conn)
    }
}

struct SqliteUnitHandle {
    inner: Arc<HostInner>,
}

impl HostTxHandle for SqliteUnitHandle {
    fn finish(self: Box<Self>, commit: bool) -> Result<(), HostError> {
        // The connection leaves the lane either way: on success it returns to
        // the pool clean; on failure it drops (closing it), so a broken
        // transaction can never leak to another caller.
        let conn = {
            let mut guard = self.inner.lane.lock().unwrap_or_else(|e| e.into_inner());
            match std::mem::replace(&mut *guard, Lane::Idle) {
                Lane::Active(c) => *c,
                Lane::Idle => {
                    return Err(HostError::Internal("unit already finished".into()));
                }
            }
        };
        let stmt = if commit { "COMMIT" } else { "ROLLBACK" };
        match conn.execute_batch(stmt) {
            Ok(()) => Ok(()),
            Err(e) if commit => Err(HostError::Internal(format!("commit failed: {e}"))),
            // Rollback is best-effort by contract (the guard's Drop cannot
            // return an error); a failed rollback surfaces as a dropped unit.
            Err(_) => Ok(()),
        }
    }
}

impl WorkflowHost for SqliteWorkflowHost {
    fn tx(&self) -> Result<HostTx, HostError> {
        let mut guard = self.lock();
        if matches!(&*guard, Lane::Active(_)) {
            return Err(HostError::Busy);
        }
        let conn = self
            .inner
            .pool
            .get()
            .map_err(|e| HostError::Internal(e.to_string()))?;
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| HostError::Internal(format!("begin failed: {e}")))?;
        *guard = Lane::Active(Box::new(conn));
        Ok(HostTx::new(Box::new(SqliteUnitHandle {
            inner: Arc::clone(&self.inner),
        })))
    }

    fn enqueue(
        &self,
        run_id: i64,
        topic: &str,
        payload_json: &str,
        idempotency_key: &str,
    ) -> Result<bool, HostError> {
        let now = chrono::Utc::now().timestamp();
        match self.scoped(|conn| {
            super::outbox::enqueue(conn, run_id, topic, payload_json, idempotency_key, now)
        }) {
            Ok(Ok(created)) => Ok(created),
            Ok(Err(e)) => Err(HostError::Internal(e.to_string())),
            Err(s) => Err(HostError::Internal(s)),
        }
    }

    fn cas(&self, run_id: i64, expected_rev: i64, state_json: &str) -> Result<(), CasError> {
        let now = chrono::Utc::now().timestamp();
        match self.scoped(|conn| {
            super::state::cas_update(conn, run_id, expected_rev, state_json, "active", now)
        }) {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(s) => Err(CasError::Database(s)),
        }
    }

    fn load_state(&self, run_id: i64) -> Result<Option<(String, i64)>, HostError> {
        let conn = self.pool_get().map_err(HostError::Internal)?;
        conn.query_row(
            "SELECT state_json, state_revision FROM workflow_runs WHERE id = ?1",
            rusqlite::params![run_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(HostError::Internal(other.to_string())),
        })
    }

    fn audit(&self, kind: AuditKind, actor: &str, target: &str, status: AuditStatus, detail: &str) {
        let _ = self.scoped(|conn| -> Result<(), rusqlite::Error> {
            let tenant = tenant_for_target(conn, target);
            let chain_kind = match kind {
                AuditKind::Workflow => crate::audit::AuditKind::Workflow,
                _ => {
                    // Unmapped SDK kinds audit loudly as Error rows — never a
                    // silent relabel onto the wrong vocabulary.
                    crate::audit::record_tenant(
                        conn,
                        crate::audit::AuditKind::Workflow,
                        actor,
                        target,
                        crate::audit::AuditStatus::Error,
                        "unmapped sdk audit kind",
                        &tenant,
                    );
                    return Ok(());
                }
            };
            crate::audit::record_tenant(
                conn,
                chain_kind,
                actor,
                target,
                match status {
                    AuditStatus::Ok => crate::audit::AuditStatus::Ok,
                    AuditStatus::Denied => crate::audit::AuditStatus::Denied,
                    AuditStatus::Error => crate::audit::AuditStatus::Error,
                },
                detail,
                &tenant,
            );
            // Art.12 decision evidence is recorded HERE, on the host write
            // path — never in engine code — so a workflow cannot modify its
            // own evidence. Coarse fields only; `detail` stays out of the
            // decision record (it may carry free-form context).
            #[cfg(feature = "compliance-pack")]
            {
                let _ = crate::audit::decision::record_decision(
                    conn,
                    &crate::audit::decision::DecisionInput {
                        actor_id: actor,
                        role: "engine",
                        policy_version: env!("CARGO_PKG_VERSION"),
                        prompt_class: "workflow",
                        tool: target,
                        model_id: "",
                        outcome: status.as_str(),
                    },
                );
            }
            Ok(())
        });
        // Best-effort by contract: scoped only fails on pool exhaustion, and a
        // dropped audit row reads as a gap, never a forged continuation.
    }
}

/// Resolve the audit tenant from a `run:<id>` reference ANYWHERE in the
/// target (the engines' convention, incl. `workflow/hostcall/<kind>/run:<id>`);
/// anything else audits against `global`.
fn tenant_for_target(conn: &Connection, target: &str) -> String {
    let run_id = target
        .match_indices("run:")
        .filter_map(|(i, _)| target[i + 4..].split(['/', ' ']).next())
        .find_map(|rest| rest.parse::<i64>().ok());
    run_id
        .map(|id| {
            conn.query_row(
                "SELECT domain FROM workflow_runs WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get::<_, String>(0),
            )
            .unwrap_or_else(|_| "global".to_string())
        })
        .unwrap_or_else(|| "global".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::verify_chain;
    use crate::config;
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;

    fn host() -> (SqliteWorkflowHost, tempfile::NamedTempFile) {
        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = SqliteConnectionManager::file(tmp.path());
        let pool = r2d2::Pool::builder().max_size(4).build(mgr).unwrap();
        run_migration(&mut pool.get().unwrap(), config::DB_MMAP_SIZE_MIB).unwrap();
        pool.get()
            .unwrap()
            .execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
                 VALUES ('acme', 'interview', '{\"v\":0}', 0, 'active', 1, 1)",
                [],
            )
            .unwrap();
        (SqliteWorkflowHost::new(pool), tmp)
    }

    fn workflow_audit_rows(tmp: &tempfile::NamedTempFile) -> Vec<(String, String)> {
        let conn = rusqlite::Connection::open(tmp.path()).unwrap();
        let mut stmt = conn
            .prepare("SELECT status, tenant_id FROM audit_events WHERE kind='workflow' ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }

    #[test]
    fn unit_commit_persists_transition_and_audit() {
        let (host, tmp) = host();
        {
            let unit = host.tx().unwrap();
            host.enqueue(1, "intake", r#"{"a":1}"#, "k-c").unwrap();
            unit.commit().unwrap();
        }
        assert_eq!(
            workflow_audit_rows(&tmp),
            vec![("ok".into(), "acme".into())],
            "the committed enqueue carries its audit row in the same tx"
        );
        assert!(verify_chain(
            &rusqlite::Connection::open(tmp.path()).unwrap()
        ));
    }

    #[test]
    fn unit_drop_rolls_back_transition_and_audit() {
        let (host, tmp) = host();
        {
            let _unit = host.tx().unwrap();
            host.enqueue(1, "intake", r#"{"a":1}"#, "k-r").unwrap();
            // Drop without commit: transition AND its audit row must vanish.
        }
        assert!(
            workflow_audit_rows(&tmp).is_empty(),
            "a rolled-back transition leaves no audit row claiming it happened"
        );
    }

    #[test]
    fn second_unit_is_busy_fail_fast() {
        let (host, _tmp) = host();
        let u1 = host.tx().unwrap();
        assert_eq!(host.tx().unwrap_err(), HostError::Busy);
        drop(u1);
        // Lane released by the drop-rollback; a new unit opens cleanly.
        let _u2 = host.tx().unwrap();
    }

    #[test]
    fn enqueue_without_unit_is_standalone_atomic() {
        let (host, tmp) = host();
        assert!(host.enqueue(1, "steer", "{}", "k-s").unwrap());
        assert!(
            !host.enqueue(1, "steer", "{}2", "k-s").unwrap(),
            "replay is a no-op"
        );
        assert_eq!(workflow_audit_rows(&tmp).len(), 1, "replay audits once");
    }

    #[test]
    fn cas_conflicts_map_to_sdk_vocabulary_and_load_state_recovers() {
        let (host, _tmp) = host();
        host.cas(1, 0, r#"{"v":1}"#).unwrap();
        assert_eq!(
            host.cas(1, 0, r#"{"v":2}"#).unwrap_err(),
            CasError::Stale { actual_revision: 1 },
            "a stale writer gets the SDK conflict vocabulary"
        );
        let (json, rev) = host.load_state(1).unwrap().unwrap();
        assert_eq!(rev, 1);
        assert_eq!(json, r#"{"v":1}"#, "load_state is the Stale-recovery read");
        assert_eq!(
            host.cas(42, 0, "{}").unwrap_err(),
            CasError::Gone,
            "a missing run reads Gone"
        );
        assert!(host.load_state(42).unwrap().is_none());
    }

    /// Art.12 write-path independence: the host (not engine code) appends a
    /// decision record for every audited workflow event, and the exported
    /// record verifies outside the host with the configured key.
    #[cfg(feature = "compliance-pack")]
    #[test]
    fn host_records_decision_evidence_that_verifies_outside() {
        // deterministic signed path for this process — installed under the
        // same lock the compliance tests use, so env writes never race reads.
        let _key = crate::handlers::compliance::tests::ensure_test_key();
        let (host, tmp) = host();
        {
            let unit = host.tx().unwrap();
            host.audit(
                AuditKind::Workflow,
                "engine-x",
                "run:1",
                AuditStatus::Ok,
                "milestone",
            );
            unit.commit().unwrap();
        }
        let conn = rusqlite::Connection::open(tmp.path()).unwrap();
        let (actor, tool, outcome): (String, String, String) = conn
            .query_row(
                "SELECT actor_id, tool, outcome FROM decision_records",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (actor.as_str(), tool.as_str(), outcome.as_str()),
            ("engine-x", "run:1", "ok")
        );
        // export → verify OUTSIDE the host path (plain connection + key only)
        let exported = crate::audit::decision::list_decisions(&conn, None, 10).unwrap();
        assert_eq!(exported.len(), 1);
        assert!(exported[0].sig.is_some());
        let _g = crate::audit::decision::decision_test_lock();
        assert!(crate::audit::decision::verify_decisions(&conn).unwrap());
        assert!(
            crate::audit::verify_chain(&conn),
            "the audit_events chain that anchors decisions still verifies"
        );
    }

    #[test]
    fn audit_maps_sdk_vocabulary_and_resolves_tenant() {
        let (host, tmp) = host();
        host.audit(
            AuditKind::Workflow,
            "engine-x",
            "run:1",
            AuditStatus::Ok,
            "milestone",
        );
        host.audit(
            AuditKind::Workflow,
            "engine-x",
            "external-artifact",
            AuditStatus::Denied,
            "gate refused",
        );
        let rows = workflow_audit_rows(&tmp);
        assert_eq!(
            rows,
            vec![
                ("ok".into(), "acme".into()),
                ("denied".into(), "global".into()),
            ],
            "`run:<id>` targets resolve the run's domain tenant"
        );
        assert!(verify_chain(
            &rusqlite::Connection::open(tmp.path()).unwrap()
        ));
    }
}
