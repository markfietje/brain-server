//! The storage-agnostic host seam.
//!
//! Engines call [`WorkflowHost`]; a host adapter owns pools, transactions, and
//! the audit chain. Every signature is value-typed (`i64`/`&str`) so any
//! transactional backend (SQLite today, Postgres later) implements the trait
//! without an ABI change — engines never see a driver type. The SDK never
//! opens a database; it never even names one.

/// Backend adapters implement [`tx::HostTxHandle`] to power their
/// [`HostTx`] guards.
pub mod tx;

pub use tx::HostTx;

use core::fmt;

/// Typed error vocabulary across the ABI — no stringly-typed failures except
/// the terminal [`HostError::Internal`] bucket, which carries the backend's
/// own message for operators.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum HostError {
    /// A concurrent writer advanced the run; re-read via
    /// [`WorkflowHost::load_state`] and re-diff before retrying.
    Stale { actual_revision: i64 },
    /// The host's write capacity is occupied (another unit of work is active).
    /// Fail-fast by design; retry with backoff.
    Busy,
    /// The referenced run does not exist.
    NotFound,
    /// Infrastructure failure surfaced verbatim from the backend.
    Internal(String),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::Stale { actual_revision } => {
                write!(f, "stale revision (actual {actual_revision})")
            }
            HostError::Busy => write!(f, "host busy"),
            HostError::NotFound => write!(f, "not found"),
            HostError::Internal(m) => write!(f, "internal: {m}"),
        }
    }
}

impl std::error::Error for HostError {}

/// Compare-and-swap conflict vocabulary, mirroring what engines surface to
/// their users (`Stale` = concurrent transition won; `Gone` = run deleted;
/// `Database` = infrastructure failure, not contention).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum CasError {
    Gone,
    Stale { actual_revision: i64 },
    Database(String),
}

impl fmt::Display for CasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CasError::Gone => write!(f, "run gone"),
            CasError::Stale { actual_revision } => {
                write!(f, "cas stale (actual {actual_revision})")
            }
            CasError::Database(m) => write!(f, "database: {m}"),
        }
    }
}

impl std::error::Error for CasError {}

/// Audit row kinds an engine may emit through the host. The host maps these
/// onto its own chain vocabulary at the single implementation site; an
/// unmapped kind audits as an Error row rather than being relabeled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuditKind {
    Workflow,
}

impl AuditKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditKind::Workflow => "workflow",
        }
    }
}

/// Audit outcome for a recorded event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditStatus {
    Ok,
    Denied,
    Error,
}

impl AuditStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditStatus::Ok => "ok",
            AuditStatus::Denied => "denied",
            AuditStatus::Error => "error",
        }
    }
}

/// The stable seam between engine cores and durable storage.
///
/// Contract: every mutating call is atomic in itself and emits its audit row
/// inside the same transaction (a dropped transition leaves no audit row
/// claiming it happened). Calls issued while another caller holds an open
/// [`HostTx`] join that unit of work; hosts are intended single-driver per
/// instance and fail fast with [`HostError::Busy`] on a second concurrent
/// unit. Reads (`load_state`) never contend with writes.
pub trait WorkflowHost: Send + Sync {
    /// Begin a unit of work. All subsequent mutating calls join it until the
    /// guard commits; dropping the guard without committing rolls back.
    fn tx(&self) -> Result<HostTx, HostError>;

    /// Enqueue a payload under an idempotency key. Returns `true` when a new
    /// entry was created, `false` when the key replayed (no-op receipt).
    fn enqueue(
        &self,
        run_id: i64,
        topic: &str,
        payload_json: &str,
        idempotency_key: &str,
    ) -> Result<bool, HostError>;

    /// Atomically advance a run's state iff the caller's view is current.
    fn cas(&self, run_id: i64, expected_rev: i64, state_json: &str) -> Result<(), CasError>;

    /// Read a run's current `(state_json, state_revision)` — the recovery
    /// half of the CAS contract after a `Stale` conflict.
    fn load_state(&self, run_id: i64) -> Result<Option<(String, i64)>, HostError>;

    /// Record an audit event. Best-effort by contract: a dropped row reads as
    /// a gap in the chain, never as a forged continuation.
    fn audit(&self, kind: AuditKind, actor: &str, target: &str, status: AuditStatus, detail: &str);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A minimal in-memory host proving engines can drive the whole seam with
    /// no database behind it — the port any future backend implements.
    #[derive(Default)]
    struct MockHost {
        committed: AtomicBool,
        rolled_back: AtomicBool,
    }

    struct MockHandle {
        host: Arc<MockHost>,
    }

    impl tx::HostTxHandle for MockHandle {
        fn finish(self: Box<Self>, commit: bool) -> Result<(), HostError> {
            if commit {
                self.host.committed.store(true, Ordering::SeqCst);
            } else {
                self.host.rolled_back.store(true, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    impl WorkflowHost for MockHost {
        fn tx(&self) -> Result<HostTx, HostError> {
            Ok(HostTx::new(Box::new(MockHandle {
                host: Arc::new(MockHost::default()),
            })))
        }
        fn enqueue(
            &self,
            _run_id: i64,
            _topic: &str,
            _payload_json: &str,
            _idempotency_key: &str,
        ) -> Result<bool, HostError> {
            Ok(true)
        }
        fn cas(&self, _run_id: i64, _expected_rev: i64, _state_json: &str) -> Result<(), CasError> {
            Ok(())
        }
        fn load_state(&self, _run_id: i64) -> Result<Option<(String, i64)>, HostError> {
            Ok(Some((r#"{"v":1}"#.into(), 1)))
        }
        fn audit(&self, _k: AuditKind, _a: &str, _t: &str, _s: AuditStatus, _d: &str) {}
    }

    #[test]
    fn trait_is_object_safe_and_send_sync() {
        // Engines hold `Arc<dyn WorkflowHost>`; this pin breaks at compile
        // time if the trait ever stops being dyn-compatible or shareable.
        fn assert_host(h: Arc<dyn WorkflowHost>) -> Arc<dyn WorkflowHost> {
            h
        }
        let h: Arc<dyn WorkflowHost> = Arc::new(MockHost::default());
        let _ = assert_host(h);
    }

    #[test]
    fn host_tx_commits_explicitly() {
        let inner = Arc::new(MockHost::default());
        let handle = MockHandle {
            host: inner.clone(),
        };
        let t = HostTx::new(Box::new(handle));
        t.commit().unwrap();
        assert!(inner.committed.load(Ordering::SeqCst));
        assert!(
            !inner.rolled_back.load(Ordering::SeqCst),
            "no double-finish"
        );
    }

    #[test]
    fn host_tx_rolls_back_on_drop() {
        let inner = Arc::new(MockHost::default());
        let handle = MockHandle {
            host: inner.clone(),
        };
        drop(HostTx::new(Box::new(handle)));
        assert!(inner.rolled_back.load(Ordering::SeqCst));
    }

    #[test]
    fn error_vocabulary_displays() {
        assert_eq!(
            HostError::Stale { actual_revision: 3 }.to_string(),
            "stale revision (actual 3)"
        );
        assert_eq!(HostError::Busy.to_string(), "host busy");
        assert_eq!(CasError::Gone.to_string(), "run gone");
        assert_eq!(
            CasError::Stale { actual_revision: 9 }.to_string(),
            "cas stale (actual 9)"
        );
    }

    #[test]
    fn audit_vocabulary_strings() {
        assert_eq!(AuditKind::Workflow.as_str(), "workflow");
        assert_eq!(AuditStatus::Ok.as_str(), "ok");
        assert_eq!(AuditStatus::Denied.as_str(), "denied");
        assert_eq!(AuditStatus::Error.as_str(), "error");
    }
}
