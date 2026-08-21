//! The unit-of-work guard: commit on explicit call, roll back on drop.
//!
//! RAII mirrors the host's own transaction discipline — a guard dropped
//! without committing can never leave a half-applied transition the caller
//! then certifies as applied. The backend-specific work hides behind a boxed
//! handle so the guard itself stays value-typed and Send across the ABI.

use super::HostError;
use core::fmt;

/// Backend-private finish hook: `commit == true` commits, `false` rolls back.
/// Implemented by host adapters; never called by engines.
pub trait HostTxHandle: Send {
    fn finish(self: Box<Self>, commit: bool) -> Result<(), HostError>;
}

/// An open unit of work on a [`WorkflowHost`](super::WorkflowHost).
pub struct HostTx {
    handle: Option<Box<dyn HostTxHandle>>,
}

impl HostTx {
    /// Adapters only: wrap a backend finish hook into a guard.
    pub fn new(handle: Box<dyn HostTxHandle>) -> Self {
        HostTx {
            handle: Some(handle),
        }
    }

    /// Commit the unit, consuming the guard.
    pub fn commit(mut self) -> Result<(), HostError> {
        match self.handle.take() {
            Some(h) => h.finish(true),
            None => Err(HostError::Internal("unit already finished".into())),
        }
    }
}

impl Drop for HostTx {
    fn drop(&mut self) {
        // Commit already ran (guard consumed); anything else rolls back.
        // Best-effort, matching the host's own rollback discipline: a failed
        // rollback surfaces as a dropped unit, never as a committed lie.
        if let Some(h) = self.handle.take() {
            let _ = h.finish(false);
        }
    }
}

impl fmt::Debug for HostTx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostTx")
            .field("open", &self.handle.is_some())
            .finish()
    }
}
