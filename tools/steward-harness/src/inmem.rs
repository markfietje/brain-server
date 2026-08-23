//! A harness-local in-memory [`WorkflowHost`] — the port any backend
//! implements, and the test double for gold-set replays. Deliberately NOT a
//! promotion of the SDK's cfg(test) MockHost: it carries real CAS revision
//! accounting and key-idempotent outbox semantics.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use brain_engine_sdk::host::tx::HostTx;
use brain_engine_sdk::host::tx::HostTxHandle;
use brain_engine_sdk::host::{AuditKind, AuditStatus, CasError, HostError, WorkflowHost};

struct Inner {
    runs: HashMap<i64, (String, i64)>, // id -> (state_json, revision)
    outbox: Vec<(i64, String, String, String)>, // run_id, topic, payload, key
    keys: HashSet<String>,
    events: Vec<String>,
}

/// Single write lane: an open tx blocks a second one (`HostError::Busy`),
/// mirroring the server's BEGIN IMMEDIATE posture.
pub struct InMemHost {
    inner: Mutex<Inner>,
    open_tx: Mutex<usize>,
}

struct NoopHandle;

impl HostTxHandle for NoopHandle {
    fn finish(self: Box<Self>, _commit: bool) -> Result<(), HostError> {
        Ok(())
    }
}

impl InMemHost {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                runs: HashMap::new(),
                outbox: Vec::new(),
                keys: HashSet::new(),
                events: Vec::new(),
            }),
            open_tx: Mutex::new(0),
        }
    }

    /// Seed (or replace) a run's state at revision 0.
    pub fn seed(&self, run_id: i64, state_json: &str) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.runs.insert(run_id, (state_json.to_string(), 0));
    }

    pub fn state(&self, run_id: i64) -> Option<(String, i64)> {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.runs.get(&run_id).cloned()
    }

    pub fn outbox_len(&self, run_id: i64) -> usize {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.outbox.iter().filter(|(r, ..)| *r == run_id).count()
    }

    /// The run's outbox rows: `(topic, payload, idempotency_key)`.
    pub fn outbox_of(&self, run_id: i64) -> Vec<(String, String, String)> {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.outbox
            .iter()
            .filter(|(r, ..)| *r == run_id)
            .map(|(_, t, p, k)| (t.clone(), p.clone(), k.clone()))
            .collect()
    }

    /// The audit rows written through the host seam.
    pub fn audit_log(&self) -> Vec<String> {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.events.clone()
    }
}

impl Default for InMemHost {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkflowHost for InMemHost {
    fn tx(&self) -> Result<HostTx, HostError> {
        let mut g = self.open_tx.lock().unwrap_or_else(|p| p.into_inner());
        if *g > 0 {
            return Err(HostError::Busy);
        }
        *g += 1;
        Ok(HostTx::new(Box::new(NoopHandle)))
    }

    fn enqueue(
        &self,
        run_id: i64,
        topic: &str,
        payload_json: &str,
        idempotency_key: &str,
    ) -> Result<bool, HostError> {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if !g.keys.insert(idempotency_key.to_string()) {
            return Ok(false);
        }
        g.outbox.push((
            run_id,
            topic.to_string(),
            payload_json.to_string(),
            idempotency_key.to_string(),
        ));
        Ok(true)
    }

    fn cas(&self, run_id: i64, expected_rev: i64, state_json: &str) -> Result<(), CasError> {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match g.runs.get_mut(&run_id) {
            Some(entry) if entry.1 == expected_rev => {
                *entry = (state_json.to_string(), expected_rev + 1);
                Ok(())
            }
            Some(entry) => Err(CasError::Stale {
                actual_revision: entry.1,
            }),
            None => Err(CasError::Gone),
        }
    }

    fn load_state(&self, run_id: i64) -> Result<Option<(String, i64)>, HostError> {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        Ok(g.runs.get(&run_id).cloned())
    }

    fn audit(&self, kind: AuditKind, actor: &str, target: &str, status: AuditStatus, detail: &str) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.events.push(format!(
            "{}/{}/{}/{}/{}",
            kind.as_str(),
            actor,
            target,
            status.as_str(),
            detail
        ));
    }
}
