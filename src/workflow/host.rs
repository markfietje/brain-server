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

use crate::Pool;
use crate::pool::PooledConn;
#[cfg(test)]
use crate::pool::SqliteConnectionManager;
use brain_engine_sdk::host::tx::HostTxHandle;
use brain_engine_sdk::host::{AuditKind, AuditStatus, CasError, HostError, HostTx, WorkflowHost};
use rusqlite::Connection;

enum Lane {
    Idle,
    Active(Box<PooledConn>),
    Failed,
}

pub(crate) struct SqliteWorkflowHost {
    inner: Arc<HostInner>,
}

struct HostInner {
    pool: Pool,
    /// Lock bounds: lane -> pool -> SQLite; no callback, reentry or await.
    /// Held until transaction finalization is classified. Poison recovery is
    /// for quarantining resources only, never certification of healthy state.
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

    fn lock(&self) -> Result<MutexGuard<'_, Lane>, HostError> {
        lock_lane(&self.inner)
    }

    /// Run `f` on the unit's connection when a unit is open, else on a
    /// transient pooled connection. Outer error = infra (pool acquisition);
    /// inner error = the op's own result type.
    fn scoped<T, E>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, E>,
    ) -> Result<Result<T, E>, String> {
        let guard = self.lock().map_err(|_| "host_lane_failed".to_string())?;
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

fn lock_lane(inner: &HostInner) -> Result<MutexGuard<'_, Lane>, HostError> {
    let guard = match inner.lane.lock() {
        Ok(guard) => guard,
        Err(poison) => {
            let mut guard = poison.into_inner();
            if let Lane::Active(conn) = &mut *guard {
                conn.quarantine();
            }
            *guard = Lane::Failed;
            return Err(HostError::Internal("host_lane_poisoned".into()));
        }
    };
    if matches!(*guard, Lane::Failed) {
        return Err(HostError::Internal("host_lane_failed".into()));
    }
    Ok(guard)
}

impl HostTxHandle for SqliteUnitHandle {
    fn finish(self: Box<Self>, commit: bool) -> Result<(), HostError> {
        let mut guard = lock_lane(&self.inner)?;
        let Lane::Active(conn) = &mut *guard else {
            return Err(HostError::Internal("unit already finished".into()));
        };
        let stmt = if commit { "COMMIT" } else { "ROLLBACK" };
        let (result, reusable) = match conn.execute_batch(stmt) {
            Ok(()) if conn.is_autocommit() => (Ok(()), true),
            Err(_) if commit && !conn.is_autocommit() => {
                if conn.execute_batch("ROLLBACK").is_ok() && conn.is_autocommit() {
                    (
                        Err(HostError::Internal("commit_failed_rolled_back".into())),
                        true,
                    )
                } else {
                    (
                        Err(HostError::Internal("transaction_indeterminate".into())),
                        false,
                    )
                }
            }
            _ => (
                Err(HostError::Internal("transaction_indeterminate".into())),
                false,
            ),
        };
        if !reusable {
            conn.quarantine();
        }
        *guard = if reusable { Lane::Idle } else { Lane::Failed };
        result
    }
}

impl WorkflowHost for SqliteWorkflowHost {
    fn tx(&self) -> Result<HostTx, HostError> {
        let mut guard = self.lock()?;
        if matches!(&*guard, Lane::Active(_)) {
            return Err(HostError::Busy);
        }
        let conn = self.inner.pool.get().map_err(|e| {
            // Contention telemetry (Throughput): the lane's checkout arm.
            crate::concurrency::note_pool_timeout();
            HostError::Internal(e.to_string())
        })?;
        conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| {
            // Contention telemetry (Throughput): the lane's BEGIN arm.
            crate::concurrency::note_busy_error(&e);
            HostError::Internal(format!("begin failed: {e}"))
        })?;
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
        self.enqueue_with_parent(run_id, None, topic, payload_json, idempotency_key)
            .map(|(created, _)| created)
    }

    /// The lineage-aware write: parents the event at `parent_event_id`
    /// when present. Same exactly-once + same-tx-audit discipline as `enqueue`.
    fn enqueue_with_parent(
        &self,
        run_id: i64,
        parent_event_id: Option<i64>,
        topic: &str,
        payload_json: &str,
        idempotency_key: &str,
    ) -> Result<(bool, i64), HostError> {
        let now = chrono::Utc::now().timestamp();
        match self.scoped(|conn| {
            super::outbox::enqueue_child(
                conn,
                run_id,
                parent_event_id,
                topic,
                payload_json,
                idempotency_key,
                now,
            )
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
        // Best-effort only: a missing audit row need not create a detectable
        // chain gap. This void method cannot certify lifecycle evidence.
    }

    /// Checked lifecycle audit: an independent transaction on a fresh pooled
    /// connection; the row AND its required head pin must commit before
    /// `Ok(())`. Refuses an open unit with Busy — checked evidence never
    /// joins another caller's uncommitted transaction. The target grammar is
    /// EXACTLY the emitted `run:<positive-id>` form (no embedded references,
    /// no trailing junk, no global fallback); a failed tenant query is NOT
    /// the global tenant. Confirmed no-write refusals are typed
    /// `SettlementRefused`; outcomes that cannot be certified stay
    /// `Internal("transaction_indeterminate")` and quarantine the
    /// connection.
    fn audit_settlement(
        &self,
        kind: AuditKind,
        actor: &str,
        target: &str,
        status: AuditStatus,
        detail: &str,
    ) -> Result<(), HostError> {
        let mut guard = self.lock()?;
        if matches!(&*guard, Lane::Active(_)) {
            return Err(HostError::Busy);
        }
        let chain_kind = match kind {
            AuditKind::Workflow => crate::audit::AuditKind::Workflow,
            // Unmapped SDK kinds refuse loudly in the checked path — never a
            // silent relabel onto the wrong vocabulary. Refused before any
            // work: a confirmed refusal.
            _ => return Err(HostError::SettlementRefused),
        };
        let run_id = settlement_target_run(target)?;
        let mut conn = self
            .pool_get()
            .map_err(|_| HostError::Internal("settlement_pool".into()))?;
        let tenant = tenant_for_run(&conn, run_id)?;
        let result = crate::audit::record_tenant_checked(
            &conn,
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
        if matches!(result, Err(crate::audit::AuditWriteError::Indeterminate))
            || !conn.is_autocommit()
        {
            conn.quarantine();
            *guard = Lane::Failed;
            return Err(HostError::Internal("transaction_indeterminate".into()));
        }
        result.map(|_| ()).map_err(|_| HostError::SettlementRefused)
    }

    /// Checked delivery: an independent IMMEDIATE transaction on a fresh
    /// pooled connection, serialized against the lane. `Lane::Active`
    /// refuses with Busy — a checked receipt must never certify another
    /// caller's uncommitted unit. `Ok(true)` means this delivery's outbox
    /// row and its paired audit evidence committed; `Ok(false)` is a
    /// CONFIRMED replay: the surviving row verified as the SAME
    /// (tenant, run, topic, key, payload) identity. Same key with a changed
    /// payload, or the same key under another run, is NOT a receipt.
    fn enqueue_settlement(
        &self,
        run_id: i64,
        topic: &str,
        payload_json: &str,
        idempotency_key: &str,
    ) -> Result<bool, HostError> {
        let guard = self.lock()?;
        if matches!(&*guard, Lane::Active(_)) {
            return Err(HostError::Busy);
        }
        // Target grammar: the lifecycle target is exactly `run:<positive-id>`.
        if run_id <= 0 {
            return Err(HostError::SettlementRefused);
        }
        let mut conn = self
            .pool_get()
            .map_err(|_| HostError::Internal("settlement_pool".into()))?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| HostError::SettlementRefused)?;
        let tenant = tenant_for_run(&tx, run_id)?;
        let now = chrono::Utc::now().timestamp();
        let (created, id) = match super::outbox::enqueue_child(
            &tx,
            run_id,
            None,
            topic,
            payload_json,
            idempotency_key,
            now,
        ) {
            Ok(pair) => pair,
            // Reserved topic / foreign parent / INSERT failure: the
            // transaction is dropped here, which rolls the remains back — a
            // confirmed no-write refusal.
            Err(_) => return Err(HostError::SettlementRefused),
        };
        if !created {
            // EXACT replay binding: resolve the surviving row by its rowid
            // (O(1)) and require the full delivery identity to match. A key
            // alone can point at another run or a different payload — that
            // is a false receipt, refused.
            let identity = tx
                .query_row(
                    "SELECT run_id, topic, payload_json FROM outbox WHERE id = ?1",
                    rusqlite::params![id],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    },
                )
                .map_err(|_| HostError::Internal("receipt_identity_lookup_failed".into()))?;
            if identity.0 != run_id || identity.1 != topic || identity.2 != payload_json {
                return Err(HostError::SettlementRefused);
            }
            return Ok(false);
        }
        // Bounded transaction-local evidence postcondition: THE audit row
        // this delivery just wrote is the newest chain row in this
        // serialized transaction — an O(1) rowid seek, work independent of
        // chain size. Whole-chain verification remains a separate validation
        // concern, not per-delivery work.
        let evidenced: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM audit_events WHERE id = (SELECT MAX(id) FROM audit_events) \
                 AND kind='workflow' AND actor='workflow' AND status='ok' \
                 AND target_hash=?1 AND detail_hash=?2 AND tenant_id=?3)",
                rusqlite::params![
                    crate::audit::hash(&format!("outbox:{idempotency_key}")),
                    crate::audit::hash(&format!("enqueue:{topic}")),
                    tenant
                ],
                |r| r.get(0),
            )
            .map_err(|_| HostError::Internal("evidence_verification_failed".into()))?;
        if !evidenced {
            // Transaction dropped → rollback: the delivery is not
            // acknowledged without its paired evidence.
            return Err(HostError::SettlementRefused);
        }
        if tx.commit().is_err() {
            // COMMIT failure leaves the transaction active (SQLite
            // transaction docs); the Transaction drop rolled the remains
            // back. Verified autocommit = confirmed no-commit refusal;
            // anything else is uncertain and quarantines the connection.
            if conn.is_autocommit() {
                return Err(HostError::SettlementRefused);
            }
            conn.quarantine();
            return Err(HostError::Internal("transaction_indeterminate".into()));
        }
        Ok(created)
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

/// The checked lifecycle target grammar: EXACTLY the emitted
/// `run:<positive-id>` form — canonical decimal, no sign, no leading zeros,
/// no embedded references, no trailing junk. Bounded parsing (the target is
/// capped, then a prefix strip + a full numeric parse). Anything else is a
/// confirmed no-work refusal; there is NO global fallback.
fn settlement_target_run(target: &str) -> Result<i64, HostError> {
    const MAX_TARGET_LEN: usize = 128;
    let invalid = || HostError::SettlementRefused;
    if target.len() > MAX_TARGET_LEN {
        return Err(invalid());
    }
    let Some(rest) = target.strip_prefix("run:") else {
        return Err(invalid());
    };
    let bytes = rest.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_digit() || (bytes.len() > 1 && bytes[0] == b'0') {
        return Err(invalid());
    }
    rest.parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(invalid)
}

/// Resolve a checked write's tenant from an ALREADY-VALIDATED run id. A
/// missing run is `NotFound`; a failed read is a fixed internal refusal —
/// neither is ever the global tenant.
fn tenant_for_run(conn: &Connection, run_id: i64) -> Result<String, HostError> {
    conn.query_row(
        "SELECT domain FROM workflow_runs WHERE id = ?1",
        rusqlite::params![run_id],
        |r| r.get::<_, String>(0),
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => HostError::NotFound,
        _ => HostError::Internal("tenant_resolution_failed".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::verify_chain;
    use crate::config;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;

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

    fn settlement_count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0))
            .expect("settlement evidence count")
    }

    fn rt_block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }

    // These candidates intentionally exercise the existing methods before
    // additive checked methods exist. Retarget their calls to the checked
    // methods after API agreement; legacy HostTx joining must stay supported.
    #[test]
    fn q1_false_receipt_changed_payload_refuses() {
        let (host, _tmp) = host();
        host.enqueue_settlement(1, "workflow/log", r#"{"v":1}"#, "q1-identity")
            .expect("first delivery creates the row");
        // Same tenant, same run, same topic, same key — but a DIFFERENT
        // payload: that is not a replay of this delivery, so the checked
        // path must refuse rather than acknowledge.
        let receipt = host.enqueue_settlement(1, "workflow/log", r#"{"v":2}"#, "q1-identity");
        assert!(
            receipt.is_err(),
            "same key with changed payload is not a receipt: {receipt:?}"
        );
    }

    #[test]
    fn q1_false_receipt_other_run_same_key_refuses() {
        let (host, tmp) = host();
        {
            let conn = rusqlite::Connection::open(tmp.path()).unwrap();
            conn.execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
                 VALUES ('acme', 'interview', '{}', 0, 'active', 1, 2)",
                [],
            )
            .unwrap();
        }
        host.enqueue_settlement(1, "workflow/log", "{}", "q1-cross-run")
            .expect("delivery for run 1");
        // The same key presented under ANOTHER run of the same tenant must
        // not return run 1's row as this run's receipt.
        let receipt = host.enqueue_settlement(2, "workflow/log", "{}", "q1-cross-run");
        assert!(
            receipt.is_err(),
            "another run in the same tenant is not a receipt: {receipt:?}"
        );
        // Run 1's own exact replay still acknowledges once, false.
        let replay = host
            .enqueue_settlement(1, "workflow/log", "{}", "q1-cross-run")
            .expect("exact replay resolves");
        assert!(!replay, "exact replay is an idempotent false receipt");
    }

    #[test]
    fn target_grammar_rejects_embedded_prefixes_trailing_junk_and_nonpositive_ids() {
        let (host, _tmp) = host();
        for bad in [
            "workflow/hostcall/read/run:1", // embedded reference, not the target
            "run:1/more",                   // trailing junk
            "run:7 x",                      // trailing junk after a space
            "xrun:1",                       // prefix not at the start
            "run:",                         // absent run
            "run:-1",                       // non-positive
            "run:0",                        // non-positive
            "run:+1",                       // not the emitted form
            "run:01",                       // not the canonical emitted form
            "run:999999999999999999999999", // out of range
            "",                             // empty target
        ] {
            let result = host.audit_settlement(
                AuditKind::Workflow,
                "harness",
                bad,
                AuditStatus::Ok,
                "RunStart",
            );
            assert!(result.is_err(), "grammar must reject {bad:?}: {result:?}");
            // A failed parse must never have audited against the global
            // tenant: the only row that could exist is none.
            let observer = rusqlite::Connection::open(_tmp.path()).unwrap();
            assert_eq!(
                settlement_count(&observer, "SELECT COUNT(*) FROM audit_events"),
                0,
                "{bad:?} must not fall back to the global tenant"
            );
        }
        // The exact emitted form resolves the tenant and writes.
        host.audit_settlement(
            AuditKind::Workflow,
            "harness",
            "run:1",
            AuditStatus::Ok,
            "RunStart",
        )
        .expect("the exact emitted grammar resolves");
        assert_eq!(workflow_audit_rows(&_tmp).len(), 1);
    }

    /// Confirmed no-write refusals are typed `SettlementRefused` (the SDK
    /// restores Idle; retry-safe), while uncertain outcomes stay
    /// `Internal` (the SDK terminalizes). Both classes through the real host.
    #[test]
    fn settlement_classification_confirmed_refused_vs_unknown() {
        // Confirmed: the trigger aborts ONLY the statement; the checked
        // writer rolls back the remains and can certify no commit.
        let (host_a, tmp_a) = host();
        {
            let conn = rusqlite::Connection::open(tmp_a.path()).unwrap();
            let aborted_hash = crate::audit::hash("RunStart");
            conn.execute_batch(&format!(
                "CREATE TRIGGER refuse_start_abort BEFORE INSERT ON audit_events
                 WHEN NEW.detail_hash='{aborted_hash}'
                 BEGIN SELECT RAISE(ABORT, 'fixture_abort'); END;"
            ))
            .unwrap();
        }
        let confirmed = host_a.audit_settlement(
            AuditKind::Workflow,
            "harness",
            "run:1",
            AuditStatus::Ok,
            "RunStart",
        );
        assert_eq!(
            confirmed,
            Err(HostError::SettlementRefused),
            "verified rollback is a confirmed refusal"
        );
        drop(host_a);
        // Uncertain: RAISE(ROLLBACK) ends the transaction inside the failed
        // statement; autocommit cannot certify what committed.
        let (host_b, tmp_b) = host();
        {
            let conn = rusqlite::Connection::open(tmp_b.path()).unwrap();
            let rolled_hash = crate::audit::hash("RunStart");
            conn.execute_batch(&format!(
                "CREATE TRIGGER refuse_start_rollback BEFORE INSERT ON audit_events
                 WHEN NEW.detail_hash='{rolled_hash}'
                 BEGIN SELECT RAISE(ROLLBACK, 'fixture_rollback'); END;"
            ))
            .unwrap();
        }
        let unknown = host_b.audit_settlement(
            AuditKind::Workflow,
            "harness",
            "run:1",
            AuditStatus::Ok,
            "RunStart",
        );
        assert_eq!(
            unknown,
            Err(HostError::Internal("transaction_indeterminate".into())),
            "uncertain outcomes stay Internal and terminalize"
        );
    }

    /// P2: the production initializer batches must survive connection
    /// replacement — every PRAGMA re-applied on the fresh connection, file
    /// identity and durable rows preserved, TEMP markers gone. The domain
    /// batch here mirrors `domain_registry::open_with_migration` verbatim;
    /// the main-pool batch comes from the real `Durability::pragma_batch`.
    #[test]
    fn p2_production_initializers_survive_connection_replacement() {
        use crate::capacity::Durability;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let domain_batch = "PRAGMA journal_mode=WAL; \
             PRAGMA synchronous=NORMAL; \
             PRAGMA foreign_keys=ON; \
             PRAGMA cache_size=-64000; \
             PRAGMA temp_store=MEMORY; \
             PRAGMA busy_timeout=5000;";
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        {
            let counter = Arc::clone(&counter);
            let mgr = SqliteConnectionManager::file(tmp.path()).with_init(move |c| {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                c.execute_batch(domain_batch)
            });
            let pool = r2d2::Pool::builder().max_size(1).build(mgr).unwrap();
            {
                let conn = pool.get().unwrap();
                conn.execute_batch("CREATE TABLE durable(n); INSERT INTO durable VALUES (5); CREATE TEMP TABLE marker(n);")
                .unwrap();
                let mut conn = conn;
                conn.quarantine();
            }
            // Replacement: the initializer re-applied (readbacks), marker gone.
            let conn = pool.get().unwrap();
            assert_eq!(
                settlement_count(&conn, "PRAGMA synchronous"),
                1,
                "synchronous=NORMAL re-applied"
            );
            assert_eq!(
                settlement_count(&conn, "PRAGMA foreign_keys"),
                1,
                "foreign_keys=ON re-applied"
            );
            assert_eq!(
                settlement_count(&conn, "PRAGMA cache_size"),
                -64000,
                "cache_size re-applied"
            );
            assert_eq!(
                settlement_count(&conn, "PRAGMA temp_store"),
                2,
                "temp_store=MEMORY re-applied (2 = MEMORY)"
            );
            let journal_mode: String = conn
                .query_row("PRAGMA journal_mode", [], |r| r.get(0))
                .unwrap();
            assert_eq!(journal_mode, "wal", "journal_mode readback");
            assert!(conn.prepare("SELECT * FROM marker").is_err());
            assert_eq!(
                settlement_count(&conn, "SELECT n FROM durable"),
                5,
                "file identity: durable rows survive replacement"
            );
        }
        // The MAIN pool init: the real Durability batch re-applies too.
        let durability = Durability::default();
        let counter2 = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        {
            let init_counter = Arc::clone(&counter2);
            let batch = durability.pragma_batch();
            let mgr = SqliteConnectionManager::file(tmp.path()).with_init(move |c| {
                init_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                c.execute_batch(&batch)
            });
            let pool = r2d2::Pool::builder().max_size(1).build(mgr).unwrap();
            {
                let mut conn = pool.get().unwrap();
                assert!(durability.apply(&mut conn).is_ok());
                conn.quarantine();
            }
            let conn = pool.get().unwrap();
            assert_eq!(
                settlement_count(&conn, "PRAGMA busy_timeout"),
                crate::capacity::POOL_BUSY_TIMEOUT_MS as i64,
                "main-pool busy timeout re-applied on replacement"
            );
            assert_eq!(
                settlement_count(&conn, "PRAGMA wal_autocheckpoint"),
                durability.wal_autocheckpoint_pages as i64,
                "main-pool autocheckpoint policy re-applied"
            );
            assert_eq!(
                counter2.load(std::sync::atomic::Ordering::SeqCst),
                2,
                "a quarantined connection was replaced, initializer re-ran"
            );
        }
    }

    /// P3: finalization ordering under a real cross-thread barrier — while
    /// the owning unit is open, no concurrent checked work observes an idle
    /// lane (Busy, never a provisional receipt); the clean path becomes
    /// available only AFTER finalization is classified.
    #[test]
    fn p3_barrier_holds_finalization_no_early_idle() {
        let (host, _tmp) = host();
        let host = Arc::new(host);
        let unit = host.tx().expect("lane open");
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let h = Arc::clone(&host);
        let worker = std::thread::spawn(move || {
            // The owning unit is open: the lane is NOT idle. The signal is
            // sent AFTER the observation so the main thread cannot classify
            // finalization before the busy lane has actually been seen —
            // signalling first left that ordering to the scheduler, and a
            // descheduled worker observed an idle lane under load.
            assert!(matches!(h.tx(), Err(HostError::Busy)));
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            h.tx().expect("clean path opens only after finalization")
        });
        started_rx.recv().unwrap();
        assert_eq!(
            host.audit_settlement(
                AuditKind::Workflow,
                "harness",
                "run:1",
                AuditStatus::Ok,
                "RunStart"
            ),
            Err(HostError::Busy),
            "checked audit cannot certify inside an open unit"
        );
        assert_eq!(
            host.enqueue_settlement(1, "workflow/log", "{}", "p3-barrier"),
            Err(HostError::Busy),
            "checked delivery cannot acknowledge inside an open unit"
        );
        unit.commit().expect("finalization classified clean");
        release_tx.send(()).unwrap();
        let reopened = worker.join().expect("worker joined");
        drop(reopened);
        let again = host.tx().expect("lane reusable after both units closed");
        drop(again);
    }

    /// L4: the real AgentHarness over SqliteWorkflowHost, driven through the
    /// actual loop — separate-connection reads of exact lifecycle evidence
    /// (targets, statuses, detail hashes, tenant), ends matched to starts,
    /// the `run_invocation` policy-snapshot pair INCLUDED, valid chain head,
    /// and every lifecycle row riding the checked writer (actor `harness`).
    #[test]
    fn l4_real_harness_lifecycle_evidence_with_policy_pair() {
        use crate::agentloop::provider::{LoopbackProvider, scripted_text};
        use crate::config;
        use crate::migration::run_migration;
        use crate::register_sqlite_vec::register_sqlite_vec;
        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = SqliteConnectionManager::file(tmp.path());
        let pool: Pool = r2d2::Pool::builder().max_size(4).build(mgr).unwrap();
        run_migration(&mut pool.get().unwrap(), config::DB_MMAP_SIZE_MIB).unwrap();
        pool.get().unwrap().execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
            [],
        ).unwrap();
        let host = Arc::new(SqliteWorkflowHost::new(pool.clone()));
        let harness = Arc::new(brain_engine_sdk::harness::AgentHarness::new(
            host.clone(),
            "test-model",
            "you are a steward",
        ));
        let provider = LoopbackProvider::new(
            "loopback",
            vec![scripted_text("first"), scripted_text("final")],
        );
        let driver = crate::agentloop::run_loop::LoopDriver::new(
            pool.clone(),
            host,
            harness,
            provider,
            Vec::new(),
            brain_engine_sdk::env::ExecutionEnv {
                fs: Arc::new(brain_engine_sdk::env::DenyAll),
                read_only: true,
                allow_process: false,
                root: "/".into(),
                allowed_commands: vec![],
            },
            Default::default(),
            "",
            crate::agentloop::hooks::LoopHooks::pass_through(),
        );
        rt_block_on(driver.run_turns(
            1,
            "drive the loop",
            &tokio_util::sync::CancellationToken::new(),
        ))
        .expect("loop completed");

        // Separate connection: the evidence reads.
        let observer = Connection::open(tmp.path()).unwrap();
        let start_hash = crate::audit::hash("RunStart");
        let end_finished = crate::audit::hash("RunEnd:finished");
        let target = crate::audit::hash("run:1");
        let starts: i64 = observer
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='workflow' AND actor='harness' \
                 AND target_hash=?1 AND status='ok' AND detail_hash=?2 AND tenant_id='acme'",
                rusqlite::params![target, start_hash],
                |r| r.get(0),
            )
            .unwrap();
        let finished: i64 = observer
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='workflow' AND actor='harness' \
                 AND target_hash=?1 AND status='ok' AND detail_hash=?2 AND tenant_id='acme'",
                rusqlite::params![target, end_finished],
                |r| r.get(0),
            )
            .unwrap();
        let aborted: i64 = observer
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE detail_hash=?1",
                rusqlite::params![crate::audit::hash("RunEnd:aborted")],
                |r| r.get(0),
            )
            .unwrap();
        // The policy-snapshot start/finish pair PLUS the completed turn:
        // the loop completes at turn 1 ("first" is tool-free), so exactly
        // 2 starts matched by 2 finished ends, nothing aborted.
        assert_eq!(starts, 2, "policy pair + one completed turn started");
        assert_eq!(finished, 2, "every start settled finished exactly once");
        assert_eq!(aborted, 0);
        // Every lifecycle row is the checked writer's; the chain verifies.
        let lifecycle_actors: i64 = observer
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE detail_hash IN (?1,?2) AND actor != 'harness'",
                rusqlite::params![start_hash, end_finished],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(lifecycle_actors, 0, "no legacy void lifecycle evidence");
        let pin = crate::audit::read_head_pin(&observer).expect("committed head pin");
        assert_eq!(
            pin.id,
            settlement_count(&observer, "SELECT MAX(id) FROM audit_events")
        );
        assert_eq!(Some(pin.hash), crate::audit::chain_head(&observer));
        assert!(crate::audit::verify_chain(&observer));
    }

    /// L4 (bounded acknowledgment work): over a long chain, checked delivery
    /// still acknowledges exactly-once, and the evidence probe's query plan
    /// is a constant-work seek — no whole-chain scan.
    #[test]
    fn l4_bounded_acknowledgment_work() {
        let (host, tmp) = host();
        {
            let conn = Connection::open(tmp.path()).unwrap();
            for n in 0..3000 {
                conn.execute(
                    "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash, tenant_id, prev_hash) \
                     VALUES ('workflow', 'seed', ?1, 'ok', ?2, 'acme', NULL)",
                    rusqlite::params![format!("seed-{n}"), format!("seed-{n}")],
                )
                .unwrap();
            }
        }
        // Long chain does not break the exactly-once receipt contract.
        assert!(
            host.enqueue_settlement(1, "workflow/log", "{}", "bounded-ack")
                .unwrap(),
            "fresh delivery creates"
        );
        assert!(
            !host
                .enqueue_settlement(1, "workflow/log", "{}", "bounded-ack")
                .unwrap(),
            "exact replay acknowledges false exactly once"
        );
        let conn = Connection::open(tmp.path()).unwrap();
        let probe = "SELECT EXISTS(SELECT 1 FROM audit_events \
                     WHERE id = (SELECT MAX(id) FROM audit_events) \
                     AND kind='workflow' AND actor='workflow' AND status='ok' \
                     AND target_hash='t' AND detail_hash='d' AND tenant_id='acme')";
        let mut stmt = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {probe}"))
            .unwrap();
        let plan: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(
            plan.iter().all(|line| !line.contains("SCAN audit_events")),
            "the acknowledgment probe must not scan the chain table: {plan:?}"
        );
        assert!(
            plan.iter().any(|line| line.contains("SEARCH audit_events")),
            "the acknowledgment probe must seek by rowid: {plan:?}"
        );
    }

    /// L1: a refused RunStart means no usable turn, no provider call, and an
    /// empty (but valid) audit chain — the failure is distinguishable from
    /// genuine absence.
    #[test]
    fn l1_start_failure_no_turn_no_provider_empty_chain() {
        use crate::agentloop::provider::{LoopbackProvider, scripted_text};
        let (host, tmp) = host();
        let runstart = crate::audit::hash("RunStart");
        {
            let conn = Connection::open(tmp.path()).unwrap();
            conn.execute_batch(&format!(
                "CREATE TRIGGER refuse_runstart BEFORE INSERT ON audit_events
                 WHEN NEW.detail_hash='{runstart}'
                 BEGIN SELECT RAISE(ABORT, 'fixture_l1'); END;"
            ))
            .unwrap();
        }
        let pool: Pool = r2d2::Pool::builder()
            .max_size(2)
            .build(SqliteConnectionManager::file(tmp.path()))
            .unwrap();
        let host = Arc::new(host);
        let harness = Arc::new(brain_engine_sdk::harness::AgentHarness::new(
            Arc::clone(&host),
            "m",
            "s",
        ));
        let provider = LoopbackProvider::new("loopback", vec![scripted_text("never")]);
        let provider_view = Arc::clone(&provider);
        let driver = crate::agentloop::run_loop::LoopDriver::new(
            pool,
            host,
            harness,
            provider,
            Vec::new(),
            brain_engine_sdk::env::ExecutionEnv {
                fs: Arc::new(brain_engine_sdk::env::DenyAll),
                read_only: true,
                allow_process: false,
                root: "/".into(),
                allowed_commands: vec![],
            },
            Default::default(),
            "",
            crate::agentloop::hooks::LoopHooks::pass_through(),
        );
        rt_block_on(async {
            let error = driver
                .run_turns(1, "input", &tokio_util::sync::CancellationToken::new())
                .await;
            assert!(error.is_err(), "the refused start surfaces");
        });
        assert!(
            provider_view.requests().is_empty(),
            "no provider dispatch without a usable turn"
        );
        let observer = Connection::open(tmp.path()).unwrap();
        // No lifecycle evidence exists for the refused start: the chain
        // contains only the pre-turn control-event rows, and nothing claims
        // a turn began (failure is visible, not silently absent).
        let lifecycle: i64 = {
            let mut stmt = observer
                .prepare("SELECT COUNT(*) FROM audit_events WHERE detail_hash IN (?1, ?2, ?3)")
                .unwrap();
            stmt.query_row(
                rusqlite::params![
                    crate::audit::hash("RunStart"),
                    crate::audit::hash("RunEnd:finished"),
                    crate::audit::hash("RunEnd:aborted")
                ],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            lifecycle, 0,
            "no lifecycle row claims the refused start happened"
        );
        assert!(
            crate::audit::verify_chain(&observer),
            "an empty chain still verifies"
        );
    }

    #[test]
    fn settlement_active_enqueue_requires_busy_not_provisional_receipt() {
        let (host, tmp) = host();
        let observer = Connection::open(tmp.path()).expect("independent observer");
        let unit = host.tx().expect("external unit");
        let receipt = host.enqueue_settlement(1, "workflow/log", "{}", "settlement-active");
        let replay = host.enqueue_settlement(1, "workflow/log", "{}", "settlement-active");
        let visible = settlement_count(&observer, "SELECT COUNT(*) FROM outbox");
        drop(unit);
        assert_eq!(visible, 0, "no independently committed receipt exists");
        assert_eq!(
            settlement_count(&observer, "SELECT COUNT(*) FROM outbox"),
            0
        );
        assert!(workflow_audit_rows(&tmp).is_empty());
        // Both a new entry and its same-transaction replay refuse with Busy:
        // a checked receipt never certifies an uncommitted unit.
        assert!(matches!(receipt, Err(HostError::Busy)));
        assert!(matches!(replay, Err(HostError::Busy)));
    }

    #[test]
    fn settlement_active_audit_must_not_join_external_unit() {
        let (host, tmp) = host();
        let unit = host.tx().expect("external unit");
        let checked = host.audit_settlement(
            AuditKind::Workflow,
            "harness",
            "run:1",
            AuditStatus::Denied,
            "RunEnd:aborted",
        );
        let provisional = host
            .scoped(|conn| {
                conn.query_row("SELECT COUNT(*) FROM audit_events", [], |r| {
                    r.get::<_, i64>(0)
                })
            })
            .expect("active connection")
            .expect("provisional evidence count");
        let visible = workflow_audit_rows(&tmp).len();
        drop(unit);
        assert_eq!(visible, 0);
        assert!(workflow_audit_rows(&tmp).is_empty());
        // The checked method refuses with Busy and leaves the external unit
        // untouched, rather than inserting into it.
        assert!(matches!(checked, Err(HostError::Busy)));
        assert_eq!(
            provisional, 0,
            "settlement cannot join an external transaction"
        );
    }

    #[test]
    fn settlement_lifecycle_evidence_visible_from_separate_connection() {
        let (host, tmp) = host();
        for (status, detail) in [
            (AuditStatus::Ok, "RunStart"),
            (AuditStatus::Denied, "RunEnd:aborted"),
        ] {
            host.audit(AuditKind::Workflow, "harness", "run:1", status, detail);
        }
        let observer = Connection::open(tmp.path()).expect("independent observer");
        let mut stmt = observer
            .prepare(
                "SELECT actor, target_hash, status, detail_hash, tenant_id
                 FROM audit_events WHERE kind = 'workflow' ORDER BY id",
            )
            .expect("lifecycle evidence query");
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .expect("lifecycle evidence rows")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("decode every evidence row");
        assert_eq!(rows.len(), 2);
        for (row, (status, detail)) in rows
            .iter()
            .zip([("ok", "RunStart"), ("denied", "RunEnd:aborted")])
        {
            assert_eq!(row.0, "harness");
            assert_eq!(row.1, crate::audit::hash("run:1"));
            assert_eq!(row.2, status);
            assert_eq!(row.3, crate::audit::hash(detail));
            assert_eq!(row.4, "acme");
        }
        let pin = crate::audit::read_head_pin(&observer).expect("committed head pin");
        assert_eq!(
            pin.id,
            settlement_count(&observer, "SELECT MAX(id) FROM audit_events")
        );
        assert_eq!(Some(pin.hash), crate::audit::chain_head(&observer));
        assert!(verify_chain(&observer));
    }

    #[test]
    fn settlement_tenant_read_failure_must_not_audit_global() {
        let (host, tmp) = host();
        let observer = Connection::open(tmp.path()).expect("independent observer");
        observer
            .execute("UPDATE workflow_runs SET domain = x'80' WHERE id = 1", [])
            .expect("synthetic unreadable tenant");
        assert!(
            observer
                .query_row("SELECT domain FROM workflow_runs WHERE id = 1", [], |r| {
                    r.get::<_, String>(0)
                })
                .is_err(),
            "fixture must fail domain decoding, not run lookup"
        );
        host.audit_settlement(
            AuditKind::Workflow,
            "harness",
            "run:1",
            AuditStatus::Ok,
            "RunStart",
        )
        .expect_err("failed tenant query must refuse the checked audit");
        assert_eq!(
            settlement_count(&observer, "SELECT COUNT(*) FROM audit_events"),
            0,
            "failed tenant query is not global authority"
        );
    }

    #[test]
    fn settlement_enqueue_audit_failure_cannot_acknowledge_outbox() {
        let (host, tmp) = host();
        let observer = Connection::open(tmp.path()).expect("independent observer");
        observer
            .execute_batch(
                "CREATE TRIGGER settlement_reject_audit BEFORE INSERT ON audit_events
                 BEGIN SELECT RAISE(ABORT, 'fixture_audit_refused'); END;",
            )
            .expect("audit-only failure fixture");
        let receipt = host.enqueue_settlement(1, "workflow/log", "{}", "settlement-unaudited");
        let durable_outbox = settlement_count(&observer, "SELECT COUNT(*) FROM outbox");
        let durable_audit = settlement_count(&observer, "SELECT COUNT(*) FROM audit_events");
        assert_eq!(durable_audit, 0);
        assert!(
            receipt.is_err(),
            "unaudited delivery cannot be acknowledged"
        );
        assert_eq!(
            durable_outbox, 0,
            "delivery and evidence must roll back together"
        );
    }

    #[test]
    fn settlement_indeterminate_audit_discards_before_host_drop() {
        let (original, tmp) = host();
        drop(original);
        let pool = Pool::builder()
            .max_size(1)
            .build(SqliteConnectionManager::file(tmp.path()))
            .expect("one slot");
        {
            let conn = pool.get().expect("fixture");
            conn.execute_batch(
                "CREATE TEMP TABLE marker(n);
                CREATE TRIGGER refuse_audit BEFORE INSERT ON audit_events
                BEGIN SELECT RAISE(ROLLBACK, 'synthetic rollback'); END;",
            )
            .expect("rollback trigger");
        }
        let host = SqliteWorkflowHost::new(pool.clone());
        assert_eq!(
            host.audit_settlement(
                AuditKind::Workflow,
                "harness",
                "run:1",
                AuditStatus::Ok,
                "RunStart"
            ),
            Err(HostError::Internal("transaction_indeterminate".into()))
        );
        assert!(host.tx().is_err());
        let conn = pool.get().expect("slot returned before host drop");
        assert!(conn.prepare("SELECT * FROM marker").is_err());
        assert_eq!(
            settlement_count(&conn, "SELECT COUNT(*) FROM audit_events"),
            0
        );
    }

    #[test]
    fn settlement_uncertain_finalization_refuses_lane_reuse() {
        let (original, tmp) = host();
        drop(original);
        let pool = Pool::builder()
            .max_size(1)
            .build(SqliteConnectionManager::file(tmp.path()))
            .expect("one slot");
        pool.get()
            .expect("fixture")
            .execute_batch("CREATE TEMP TABLE marker(n);")
            .expect("marker");
        let host = SqliteWorkflowHost::new(pool.clone());
        let unit = host.tx().expect("unit");
        // Synthetic outcome uncertainty: transaction is already gone when
        // the owning handle attempts finalization. Autocommit cannot certify
        // whether that owner's work committed.
        host.scoped(|conn| conn.execute_batch("ROLLBACK"))
            .expect("lane")
            .expect("synthetic intervening finalization");
        assert!(unit.commit().is_err());
        assert!(
            host.tx().is_err(),
            "unknown finalization must terminalize the lane"
        );
        drop(host);
        let conn = pool.get().expect("slot released after host drop");
        assert!(conn.is_autocommit());
        assert!(
            conn.prepare("SELECT * FROM marker").is_err(),
            "uncertain connection discarded"
        );
    }

    #[test]
    fn settlement_pool_discards_unfinished_connection() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let tmp = tempfile::NamedTempFile::new().expect("temporary database");
        let initialized = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&initialized);
        let mgr = SqliteConnectionManager::file(tmp.path()).with_init(move |conn| {
            counter.fetch_add(1, Ordering::SeqCst);
            conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=0;")
        });
        let pool = Pool::builder().max_size(1).build(mgr).expect("one slot");
        {
            let conn = pool.get().expect("initial checkout");
            conn.execute_batch(
                "CREATE TABLE durable(value); INSERT INTO durable VALUES (7);
                 CREATE TEMP TABLE connection_marker(value);",
            )
            .expect("fixture");
        }
        {
            let conn = pool.get().expect("healthy reuse");
            assert_eq!(initialized.load(Ordering::SeqCst), 1);
            assert!(conn.prepare("SELECT * FROM connection_marker").is_ok());
            conn.execute_batch("BEGIN IMMEDIATE; INSERT INTO durable VALUES (8);")
                .expect("unfinished transaction");
        }
        let conn = pool.get().expect("replacement checkout");
        assert!(
            conn.is_autocommit(),
            "open transaction must be discarded on return"
        );
        assert!(conn.prepare("SELECT * FROM connection_marker").is_err());
        assert_eq!(initialized.load(Ordering::SeqCst), 2);
        assert_eq!(settlement_count(&conn, "SELECT SUM(value) FROM durable"), 7);
        assert_eq!(settlement_count(&conn, "PRAGMA foreign_keys"), 1);
    }

    #[test]
    fn settlement_failed_host_commit_returns_clean_pool_connection() {
        let (original, tmp) = host();
        drop(original);
        // One slot makes reuse deterministic: no other clean connection can
        // accidentally conceal a transaction returned to the pool.
        let mgr = SqliteConnectionManager::file(tmp.path())
            .with_init(|conn| conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=0;"));
        let pool = Pool::builder()
            .max_size(1)
            .build(mgr)
            .expect("one-slot pool");
        {
            let conn = pool.get().expect("fixture connection");
            assert_eq!(settlement_count(&conn, "PRAGMA foreign_keys"), 1);
            conn.execute_batch(
                "CREATE TABLE settlement_parent(id INTEGER PRIMARY KEY);
                 CREATE TABLE settlement_child(parent_id INTEGER REFERENCES settlement_parent(id)
                    DEFERRABLE INITIALLY DEFERRED);",
            )
            .expect("isolated deferred-constraint fixture");
        }
        let host = SqliteWorkflowHost::new(pool.clone());
        let unit = host.tx().expect("external unit");
        host.scoped(|conn| conn.execute("INSERT INTO settlement_child VALUES (1)", []))
            .expect("active connection")
            .expect("deferred violation must allow INSERT");
        assert!(
            host.enqueue(1, "workflow/log", "{}", "settlement-commit")
                .is_ok()
        );
        let commit = unit.commit();
        assert!(
            commit.is_err(),
            "deferred constraint must fail actual COMMIT"
        );
        let conn = pool.get().expect("subsequent checkout");
        let clean = conn.is_autocommit();
        let inherited = settlement_count(&conn, "SELECT COUNT(*) FROM settlement_child");
        if !clean {
            conn.execute_batch("ROLLBACK")
                .expect("clean up pre-repair leaked transaction");
        }
        drop(conn);
        let observer = Connection::open(tmp.path()).expect("independent observer");
        assert_eq!(
            settlement_count(&observer, "SELECT COUNT(*) FROM outbox"),
            0
        );
        assert_eq!(
            settlement_count(&observer, "SELECT COUNT(*) FROM audit_events"),
            0
        );
        assert!(crate::audit::read_head_pin(&observer).is_none());
        assert!(
            clean && inherited == 0,
            "failed COMMIT must not leak into next checkout"
        );
        drop(
            host.tx()
                .expect("lane can open a fresh unit after known rollback"),
        );
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
        // `_key` (ensure_test_key) already holds the crate-wide decision
        // lock for this whole record→verify span; re-acquiring would
        // self-deadlock the non-reentrant mutex.
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
