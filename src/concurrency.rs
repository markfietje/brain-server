//! Contention telemetry — the Throughput milestone's counters.
//!
//! Process-local truth: two monotonic counters + the WAL-pending snapshot,
//! read by `/metrics` and `/health/db`, incremented ONLY at existing error
//! arms (Throughput boundary: no new error paths, no syscalls added to any
//! success path):
//!   * `pool_timeouts_total` — r2d2 checkout failures, counted at the
//!     handler seam `HandlerError::db_down` (the shared
//!     `pool.get().map_err` arm) and the workflow lane's checkout arm;
//!   * `busy_errors_total` — SQLITE_BUSY-family errors observed at the
//!     governed-write BEGIN sites (`WorkflowTx::begin`, the workflow lane's
//!     `BEGIN IMMEDIATE`); the audit seam keeps its own dedicated
//!     `brain_db_busy_total` (audit-tx settle busy) unchanged;
//!   * `wal_pages` — per-domain WAL frames not yet checkpointed, refreshed
//!     ONLY by `/health/db` (the `PRAGMA wal_checkpoint(PASSIVE)` row runs
//!     there and nowhere else — admin cold path, never per request).
//!
//! The counters are process statics (the `audit::BUSY_HITS` precedent) so
//! deep write-path code can increment them without plumbing `AppState`
//! through every layer; `AppState.concurrency` carries the `&'static` handle
//! so the scrape surfaces read them through state like everything else.
//! Ordering is `Relaxed` — each counter is independent and only ever
//! increases; no cross-variable invariants exist to protect (the proptest
//! below pins monotonicity under that relaxation).

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// The contention state. Held as a process static; `AppState.concurrency`
/// is a `&'static` alias to this instance. Tests construct their own via
/// [`Concurrency::new`] so property tests never race the global.
pub struct Concurrency {
    /// `brain_pool_timeouts_total` — r2d2 checkout failures since process
    /// start. Monotonic; Relaxed.
    pool_timeouts_total: AtomicU64,
    /// `brain_busy_errors_total` — SQLITE_BUSY-family errors at the
    /// governed-write BEGIN sites since process start. Monotonic; Relaxed.
    busy_errors_total: AtomicU64,
    /// Per-domain WAL frames not yet checkpointed (`log - checkpointed` from
    /// the PASSIVE checkpoint row), written only by `/health/db`, read by
    /// `/metrics`. Sorted-vec (not a map): `Vec::new()` is const-stable for
    /// the static, and sorted order keeps scrape emission deterministic.
    wal_pages: Mutex<Vec<(String, u64)>>,
}

/// The process-wide instance (the audit-static precedent).
pub static CONCURRENCY: Concurrency = Concurrency {
    pool_timeouts_total: AtomicU64::new(0),
    busy_errors_total: AtomicU64::new(0),
    wal_pages: Mutex::new(Vec::new()),
};

impl Default for Concurrency {
    fn default() -> Self {
        Self::new()
    }
}

impl Concurrency {
    /// Test-local instance; identical semantics to the static.
    pub fn new() -> Self {
        Self {
            pool_timeouts_total: AtomicU64::new(0),
            busy_errors_total: AtomicU64::new(0),
            wal_pages: Mutex::new(Vec::new()),
        }
    }

    /// Count a pool checkout failure (r2d2 `get()` error — r2d2 0.8's only
    /// error shape is "timed out waiting for connection"). Existing error
    /// arms only; never on the success path.
    pub fn note_pool_timeout(&self) {
        self.pool_timeouts_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Count a SQLITE_BUSY-family rusqlite error observed at a governed-write
    /// BEGIN site. Non-busy errors are ignored here (they keep their own
    /// propagation paths).
    pub fn note_busy_error(&self, e: &rusqlite::Error) {
        if crate::audit::is_busy_error(e) {
            self.busy_errors_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Replace the WAL snapshot (called by `/health/db` after its sweep).
    /// Stored sorted so `/metrics` emission is deterministic; a domain
    /// re-reported replaces its previous value.
    pub fn set_wal_snapshot(&self, entries: Vec<(String, u64)>) {
        if let Ok(mut v) = self.wal_pages.lock() {
            *v = entries;
            v.sort();
            v.dedup_by(|a, b| a.0 == b.0);
        }
    }

    /// The WAL snapshot as sorted `(domain, pending)` pairs (deterministic
    /// scrape output). Empty when `/health/db` has not run yet this process.
    pub fn wal_snapshot(&self) -> Vec<(String, u64)> {
        self.wal_pages.lock().map(|v| v.clone()).unwrap_or_default()
    }

    pub fn pool_timeouts(&self) -> u64 {
        self.pool_timeouts_total.load(Ordering::Relaxed)
    }

    pub fn busy_errors(&self) -> u64 {
        self.busy_errors_total.load(Ordering::Relaxed)
    }
}

/// Static-side helpers so deep write-path code (workflow cores, the handler
/// error seam) needs no state plumbing.
pub fn note_pool_timeout() {
    CONCURRENCY.note_pool_timeout();
}

pub fn note_busy_error(e: &rusqlite::Error) {
    CONCURRENCY.note_busy_error(e);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::ffi;

    fn busy_err(code: i32) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(ffi::Error::new(code), Some("drill".into()))
    }

    // Throughput pin: the pool-timeout counter is monotonic under Relaxed
    // ordering — any interleaving of fetch_adds only ever raises the loaded
    // value. Runs on a LOCAL instance so parallel tests never race the global.
    proptest::proptest! {
        #[test]
        fn pool_timeout_counter_is_monotonic_under_relaxed_ordering(bumps in 0u64..1_000u64) {
            let c = Concurrency::new();
            let mut last = c.pool_timeouts();
            for _ in 0..bumps {
                c.note_pool_timeout();
                let now = c.pool_timeouts();
                proptest::prop_assert!(now == last + 1, "counter must increase by exactly 1");
                last = now;
            }
            proptest::prop_assert_eq!(c.pool_timeouts(), bumps);
        }

        #[test]
        fn busy_note_counts_only_busy_family(seed in 0u64..1_000u64) {
            let c = Concurrency::new();
            // Busy-family codes always count.
            for code in [ffi::SQLITE_BUSY, ffi::SQLITE_BUSY_SNAPSHOT, ffi::SQLITE_BUSY_RECOVERY] {
                let before = c.busy_errors();
                c.note_busy_error(&busy_err(code));
                proptest::prop_assert!(
                    c.busy_errors() == before + 1,
                    "busy code {} must count",
                    code
                );
            }
            // Non-busy errors never count — the gauge stays put.
            for _ in 0..=seed % 10 {
                c.note_busy_error(&busy_err(ffi::SQLITE_READONLY));
            }
            proptest::prop_assert_eq!(c.busy_errors(), 3);
        }
    }

    #[test]
    fn wal_snapshot_is_sorted_and_replaceable() {
        let c = Concurrency::new();
        assert!(
            c.wal_snapshot().is_empty(),
            "no snapshot until /health/db writes one"
        );
        c.set_wal_snapshot(vec![("work".to_string(), 7), ("global".to_string(), 3)]);
        assert_eq!(
            c.wal_snapshot(),
            vec![("global".to_string(), 3), ("work".to_string(), 7)],
            "snapshot must come back sorted by domain for deterministic scrapes"
        );
        c.set_wal_snapshot(vec![("global".to_string(), 0)]);
        assert_eq!(
            c.wal_snapshot().len(),
            1,
            "a new snapshot replaces, not merges"
        );
    }
}
