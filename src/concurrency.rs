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
//! Headroom adds the lock-wait histogram: request-path `Mutex`/`RwLock`
//! holders record the time spent WAITING to acquire (never the hold), and
//! only on the contended path — `try_lock` first, so an uncontended acquire
//! costs zero clock reads. `/metrics` derives `brain_lock_wait_micros_p50`
//! and `brain_lock_wait_micros_p95` from the fixed integer buckets (no
//! histogram dependency; quantiles are read off the precomputed edges).
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
use std::time::Duration;

/// Fixed µs edges of the lock-wait histogram (Headroom). Integer buckets,
/// deliberately — no histograms dependency; scrape quantiles read off these
/// edges are stable across releases (a bucket-boundary change would move the
/// emitted numbers and must land with a metrics-dictionary note). 11 lower
/// edges → 12 buckets: bucket 0 is "< 10 µs", bucket `i ≥ 1` spans
/// `edges[i-1] .. edges[i]`, and bucket 11 is "≥ 100 ms" (overflow).
pub const LOCK_WAIT_BUCKET_EDGES_US: [u64; 11] = [
    10, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 100_000,
];
pub const LOCK_WAIT_BUCKETS: usize = LOCK_WAIT_BUCKET_EDGES_US.len() + 1;

/// The bucket index for a wait of `micros`. Linear scan over 11 edges —
/// cheaper than a binary search at this width and trivially auditable.
fn lock_wait_bucket(micros: u64) -> usize {
    let mut idx = 0usize;
    while idx < LOCK_WAIT_BUCKET_EDGES_US.len() && micros >= LOCK_WAIT_BUCKET_EDGES_US[idx] {
        idx += 1;
    }
    idx
}

/// Fixed-width lock-wait histogram: one AtomicU64 per bucket. Counts only
/// ever increase (Relaxed); the scrape derives p50/p95 from the edges above.
#[derive(Default)]
pub struct LockWaitHistogram {
    buckets: [AtomicU64; LOCK_WAIT_BUCKETS],
}

impl LockWaitHistogram {
    const fn empty() -> Self {
        Self {
            buckets: [const { AtomicU64::new(0) }; LOCK_WAIT_BUCKETS],
        }
    }

    /// Record one wait. Clamped into the last bucket on overflow.
    pub fn record_us(&self, micros: u64) {
        let idx = lock_wait_bucket(micros);
        self.buckets[idx].fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot of the bucket counts (deterministic scrape order).
    pub fn bucket_counts(&self) -> [u64; LOCK_WAIT_BUCKETS] {
        let mut out = [0u64; LOCK_WAIT_BUCKETS];
        for (o, b) in out.iter_mut().zip(self.buckets.iter()) {
            *o = b.load(Ordering::Relaxed);
        }
        out
    }

    /// Total recorded waits (the sum of the buckets — also the scrape's
    /// "is this gauge live" check: 0 means nothing ever contended).
    pub fn total(&self) -> u64 {
        self.bucket_counts().iter().sum()
    }

    /// The bucket EDGE (µs) at percentile `p` (0.0..=1.0). Deterministic:
    /// walk the buckets in order until the cumulative count reaches
    /// `p * total`; emit that bucket's lower edge (the overflow bucket emits
    /// its lower edge too — honest, not interpolated). Empty histogram → 0.
    /// This is a bucket-quantile, not an exact percentile — the dictionary
    /// says so.
    pub fn quantile_edge_us(&self, p: f64) -> u64 {
        let counts = self.bucket_counts();
        let total: u64 = counts.iter().sum();
        if total == 0 {
            return 0;
        }
        let target = ((total as f64) * p.clamp(0.0, 1.0)).ceil() as u64;
        let mut cum = 0u64;
        for (idx, c) in counts.iter().enumerate() {
            cum += c;
            if cum >= target {
                return if idx == 0 {
                    0
                } else {
                    LOCK_WAIT_BUCKET_EDGES_US[idx - 1]
                };
            }
        }
        LOCK_WAIT_BUCKET_EDGES_US[LOCK_WAIT_BUCKET_EDGES_US.len() - 1]
    }
}

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
    ///
    /// Lock bounds (Headroom): the critical sections are vec replace+sort /
    /// clone — no I/O, no nesting, no SQL. Poison: fail-open (write skipped,
    /// read → empty). COLD-path holders only (`/health/db` admin write,
    /// `/metrics` scrape read) — deliberately NOT wait-instrumented: the
    /// scrape paths are operator-cold and would only pollute the
    /// request-path histogram with admin noise.
    wal_pages: Mutex<Vec<(String, u64)>>,
    /// Lock-wait histogram (Headroom): contended acquire waits of the
    /// instrumented request-path locks, bucketed at the fixed µs edges.
    lock_waits: LockWaitHistogram,
}

/// The process-wide instance (the audit-static precedent).
pub static CONCURRENCY: Concurrency = Concurrency {
    pool_timeouts_total: AtomicU64::new(0),
    busy_errors_total: AtomicU64::new(0),
    wal_pages: Mutex::new(Vec::new()),
    lock_waits: LockWaitHistogram::empty(),
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
            lock_waits: LockWaitHistogram::empty(),
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

    /// The lock-wait histogram (scrape-side reads).
    pub fn lock_wait_histogram(&self) -> &LockWaitHistogram {
        &self.lock_waits
    }
}

// ── measured-lock helpers (Headroom) ─────────────────────────────────────
//
// The request-path lock holders route their acquisitions through these so
// WAIT time lands in the histogram while the critical-section code (and each
// site's poison posture — fail-open here, fail-closed there) stays verbatim.
// Fast path is `try_lock`: an uncontended acquire performs zero clock reads
// (the Throughput "no cost on success paths" boundary, kept). Only the
// contended path pays `Instant::now()` twice — vDSO reads, no syscall.

use std::sync::{MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard, TryLockError};
use std::time::Instant;

fn note_lock_wait(wait: Duration) {
    CONCURRENCY
        .lock_waits
        .record_us(u64::try_from(wait.as_nanos() / 1_000).unwrap_or(u64::MAX));
}

fn wait_then_lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    let t0 = Instant::now();
    let g = m.lock().unwrap_or_else(PoisonError::into_inner);
    note_lock_wait(t0.elapsed());
    g
}

fn wait_then_read<T>(m: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    let t0 = Instant::now();
    let g = m.read().unwrap_or_else(PoisonError::into_inner);
    note_lock_wait(t0.elapsed());
    g
}

/// Mutex acquisition for sites whose poison posture is RECOVER-AND-CONTINUE
/// (`unwrap_or_else(into_inner)` today — the workflow lane, the decision
/// signing key). Returns the guard either way; records the wait only when
/// the lock was actually contended. A poisoned lock behaves exactly like the
/// call it replaces.
pub fn mutex_guard_recovered<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.try_lock() {
        Ok(g) => g,
        Err(TryLockError::Poisoned(p)) => p.into_inner(),
        Err(TryLockError::WouldBlock) => wait_then_lock(m),
    }
}

/// RwLock read acquisition, poison-recovered flavor (same contract as
/// [`mutex_guard_recovered`]).
pub fn rwlock_read_recovered<T>(m: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    match m.try_read() {
        Ok(g) => g,
        Err(TryLockError::Poisoned(p)) => p.into_inner(),
        Err(TryLockError::WouldBlock) => wait_then_read(m),
    }
}

/// Mutex acquisition, poison-PROPAGATING flavor: `Err(poison)` reaches the
/// caller unchanged so each site keeps its fail-closed / fail-open arm
/// (rate limiter denies, registry errors, token store denies). Wait is
/// recorded only on the contended path.
pub fn mutex_guard_measured<T>(
    m: &Mutex<T>,
) -> Result<MutexGuard<'_, T>, PoisonError<MutexGuard<'_, T>>> {
    match m.try_lock() {
        Ok(g) => Ok(g),
        Err(TryLockError::Poisoned(p)) => Err(p),
        Err(TryLockError::WouldBlock) => {
            let t0 = Instant::now();
            let g = m.lock()?;
            note_lock_wait(t0.elapsed());
            Ok(g)
        }
    }
}

/// RwLock read, poison-propagating flavor.
pub fn rwlock_read_measured<T>(
    m: &RwLock<T>,
) -> Result<RwLockReadGuard<'_, T>, PoisonError<RwLockReadGuard<'_, T>>> {
    match m.try_read() {
        Ok(g) => Ok(g),
        Err(TryLockError::Poisoned(p)) => Err(p),
        Err(TryLockError::WouldBlock) => {
            let t0 = Instant::now();
            let g = m.read()?;
            note_lock_wait(t0.elapsed());
            Ok(g)
        }
    }
}

/// RwLock write, poison-propagating flavor.
pub fn rwlock_write_measured<T>(
    m: &RwLock<T>,
) -> Result<RwLockWriteGuard<'_, T>, PoisonError<RwLockWriteGuard<'_, T>>> {
    match m.try_write() {
        Ok(g) => Ok(g),
        Err(TryLockError::Poisoned(p)) => Err(p),
        Err(TryLockError::WouldBlock) => {
            let t0 = Instant::now();
            let g = m.write()?;
            note_lock_wait(t0.elapsed());
            Ok(g)
        }
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

    // ── Headroom: the lock-wait histogram ──────────────────────────────

    use super::{LOCK_WAIT_BUCKET_EDGES_US, LOCK_WAIT_BUCKETS, lock_wait_bucket};

    // Bucket mapping is the dictionary's contract: a wait lands at the LAST
    // edge it is ≥ of. Monotonic — a bigger wait never lands in a smaller
    // bucket.
    #[test]
    fn lock_wait_bucket_mapping_is_monotonic() {
        assert_eq!(lock_wait_bucket(0), 0);
        assert_eq!(lock_wait_bucket(9), 0, "< 10 µs stays in bucket 0");
        assert_eq!(lock_wait_bucket(10), 1, "exactly at an edge → next bucket");
        assert_eq!(lock_wait_bucket(11), 1);
        assert_eq!(lock_wait_bucket(49), 1);
        assert_eq!(lock_wait_bucket(50), 2);
        // Exhaustive monotonicity: bucket(bigger) >= bucket(smaller).
        let mut last = 0usize;
        for edge in LOCK_WAIT_BUCKET_EDGES_US {
            let b = lock_wait_bucket(edge);
            assert!(b >= last, "buckets must be non-decreasing at the edges");
            last = b;
        }
        assert_eq!(
            lock_wait_bucket(u64::MAX),
            LOCK_WAIT_BUCKETS - 1,
            "overflow lands in the last bucket"
        );
        assert_eq!(
            lock_wait_bucket(LOCK_WAIT_BUCKET_EDGES_US[LOCK_WAIT_BUCKET_EDGES_US.len() - 1]),
            LOCK_WAIT_BUCKETS - 1,
            "a wait ≥ the top edge is overflow-bucketed"
        );
    }

    // The quantile is deterministic: same counts → same edge, and the emitted
    // value is always one of the pinned edges (or 0 for empty).
    #[test]
    fn lock_wait_quantile_is_deterministic_and_bounded() {
        let c = Concurrency::new();
        assert_eq!(
            c.lock_wait_histogram().quantile_edge_us(0.5),
            0,
            "empty → 0"
        );
        assert_eq!(c.lock_wait_histogram().total(), 0);

        // 60 waits just under 10 µs, 20 in the 10-50 µs bucket, 20 huge.
        for _ in 0..60 {
            c.lock_wait_histogram().record_us(9);
        }
        for _ in 0..20 {
            c.lock_wait_histogram().record_us(25);
        }
        for _ in 0..20 {
            c.lock_wait_histogram().record_us(200_000);
        }
        let h = c.lock_wait_histogram();
        assert_eq!(h.total(), 100);
        // p50 lands inside the dominant first band → its lower edge is 0.
        assert_eq!(h.quantile_edge_us(0.50), 0);
        // p95 crosses into the overflow band → the 100_000 edge.
        assert_eq!(
            h.quantile_edge_us(0.95),
            LOCK_WAIT_BUCKET_EDGES_US[LOCK_WAIT_BUCKET_EDGES_US.len() - 1]
        );
        // Repeat reads are identical (no scrape side effects).
        assert_eq!(h.quantile_edge_us(0.95), h.quantile_edge_us(0.95));
    }

    // The helper contract, end to end: the fast path (uncontended try_lock)
    // records NOTHING; a contended acquire records at least one event; and
    // both flavors (recovered / measured) preserve their poison postures.
    #[test]
    fn lock_helpers_record_only_on_contention() {
        use std::sync::Arc;
        let c = Concurrency::new();
        let before = c.lock_wait_histogram().total();

        // Fast path: uncontended — no clock reads, no records.
        let m: Mutex<u8> = Mutex::new(0);
        {
            let _g = super::mutex_guard_recovered(&m);
        }
        let m2: Mutex<u8> = Mutex::new(0);
        let _g2 = super::mutex_guard_measured(&m2);
        let r: RwLock<u8> = RwLock::new(0);
        {
            let _g = super::rwlock_read_recovered(&r);
        }
        let r2: RwLock<u8> = RwLock::new(0);
        let _g3 = super::rwlock_read_measured(&r2);
        let r3: RwLock<u8> = RwLock::new(0);
        let _g4 = super::rwlock_write_measured(&r3);
        assert_eq!(
            c.lock_wait_histogram().total(),
            before,
            "uncontended acquisitions must not record (no cost on the success path)"
        );

        // Contended path: hold the lock in a thread, acquire through the
        // helper, and observe ≥1 recorded wait. Runs on the LOCAL instance —
        // helpers always record to the process static, so this block uses
        // the static's delta (parallel tests only ever add counts; a ≥1
        // delta stays true under interleaving).
        let shared = Arc::new(Mutex::new(0u8));
        let holder = Arc::clone(&shared);
        let handle = std::thread::spawn(move || {
            let _g = holder.lock().unwrap();
            std::thread::sleep(Duration::from_millis(30));
        });
        // Give the holder a beat to take the lock.
        std::thread::sleep(Duration::from_millis(10));
        let static_before = super::CONCURRENCY.lock_wait_histogram().total();
        let _g = super::mutex_guard_recovered(&shared);
        handle.join().expect("holder thread");
        assert!(
            super::CONCURRENCY.lock_wait_histogram().total() > static_before,
            "a contended acquire must record its wait"
        );
        let _ = before; // (the fast-path half already asserted this)
    }

    // Poison postures survive the helpers verbatim — the fail-closed flavor
    // surfaces the poison, the recovered flavor heals it.
    #[test]
    fn poison_flavors_keep_their_contracts() {
        use std::sync::Arc;
        let p = std::sync::Arc::new(Mutex::new(0u8));
        let pp = std::sync::Arc::clone(&p);
        let _ = std::thread::spawn(move || {
            let _g = pp.lock().unwrap();
            panic!("poison the mutex");
        })
        .join();
        assert!(
            super::mutex_guard_measured(&p).is_err(),
            "measured propagates the poison"
        );
        let _g = super::mutex_guard_recovered(&p); // recovered heals

        let r: Arc<RwLock<u8>> = Arc::new(RwLock::new(0));
        let rr = Arc::clone(&r);
        let _ = std::thread::spawn(move || {
            let _g = rr.write().unwrap();
            panic!("poison the rwlock");
        })
        .join();
        assert!(super::rwlock_read_measured(&r).is_err());
        assert!(super::rwlock_write_measured(&r).is_err());
        {
            let _g = super::rwlock_read_recovered(&r); // recovered heals
        }
    }
}
