//! Capacity envelopes.
//!
//! A configuration that exceeds these is *unsupported*: brain-server refuses
//! new ingests with HTTP 507 (Insufficient Storage) until the operator
//! resolves it. Read routes (`/search`, `/recall`, `/get`) are NEVER blocked —
//! an over-capacity brain must still answer. The numbers are documented in
//! `BENCHMARKS.md` §v0.9.9 and are measured, not estimated.
//! [errata-exempt: §v0.9.9 is a BENCHMARKS.md section anchor, not a release label]
//!
//! Lives in the lib (not server-private `config.rs`) so the `bench` and
//! `brain-migrate-rehearse` binaries can assert against the same envelope the
//! server enforces.

/// Which hardware envelope applies. Resolved from `BRAIN_CAPACITY_TARGET`
/// (desktop|jetson). Default: jetson (the conservative choice; the live
/// install is a Jetson Nano 4 GB).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CapacityTarget {
    Desktop,
    #[default]
    Jetson,
}

/// Concurrent-truth p95 ceilings (the Throughput milestone): the p95 of
/// /search under the bench's BENCH_CLIENTS=8 fan-out that a target must stay
/// under for `BENCH_ENVELOPE` to pass.
///
/// desktop = 60 ms: three measured runs against a 1 000-doc release-built
/// corpus (2026-09-05, docs/THROUGHPUT_PROOF_20260905.md) landed merged p95
/// at 22.28 / 22.86 / 23.07 ms — the ceiling is the worst run + ~2.5× margin
/// for shared-dev-box noise (the 200 ms UX ceiling below stays the
/// plugin-facing bound).
///
/// jetson = 150 ms: NOT yet measured under load (no ARM runner — the known
/// repo CI gap); asserted only on local `BENCH_ENVELOPE=jetson` runs.
/// Re-measure on the device and tighten before trusting it.
const P95_CEILING_DESKTOP: u64 = 60;
const P95_CEILING_JETSON: u64 = 150;

/// Resolve the capacity target from `BRAIN_CAPACITY_TARGET` (desktop|jetson).
/// Unknown/empty → Jetson (conservative).
pub fn capacity_target() -> CapacityTarget {
    match std::env::var("BRAIN_CAPACITY_TARGET")
        .ok()
        .map(|s| s.trim().to_lowercase())
        .as_deref()
    {
        Some("desktop") => CapacityTarget::Desktop,
        _ => CapacityTarget::Jetson,
    }
}

/// Per-connection SQLite durability posture for the MAIN pool (the Headroom
/// milestone). `Full` is the SQLite compile default — and therefore the
/// behavior every pool connection exhibited before Headroom (measured
/// 2026-09-05: a fresh connection to the WAL database reports
/// `PRAGMA synchronous` = 2/FULL, because only the one-shot migration
/// connection ever set NORMAL, and `PRAGMA synchronous` is per-connection).
/// `Normal` is the WAL-mode recommendation operators can opt into via
/// `BRAIN_SYNCHRONOUS=normal` — commit fsyncs move to checkpoint time; on
/// power loss the last commits may roll back but the database stays
/// uncorrupted. Available, documented, NOT a default (defaults equal today's
/// behavior by construction; the envelope pin enforces).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SynchronousMode {
    /// `PRAGMA synchronous=FULL` — fsync on every commit. Today's behavior.
    #[default]
    Full,
    /// `PRAGMA synchronous=NORMAL` — the WAL-mode tuning posture.
    Normal,
}

impl SynchronousMode {
    /// The literal token the pragma batch interpolates.
    pub fn pragma_value(self) -> &'static str {
        match self {
            Self::Full => "FULL",
            Self::Normal => "NORMAL",
        }
    }

    /// The lowercase echo form surfaced by `/health/db`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Normal => "normal",
        }
    }
}

/// The durability + checkpoint policy applied at EVERY main-pool connection's
/// init beside `busy_timeout` (the Headroom seam). Resolved once at boot
/// (envelope default ⊕ fail-closed env override — `config::validate_durability_env`)
/// and carried on `AppState` so `/health/db` can echo exactly what was applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Durability {
    pub synchronous_mode: SynchronousMode,
    pub wal_autocheckpoint_pages: u32,
}

impl Default for Durability {
    fn default() -> Self {
        Self {
            synchronous_mode: SynchronousMode::default(),
            wal_autocheckpoint_pages: DEFAULT_WAL_AUTOCHECKPOINT_PAGES,
        }
    }
}

/// SQLite's compile-time `wal_autocheckpoint` default — 1 000 pages — which
/// is ALSO the live effective value: nothing in the server ever changed it
/// before Headroom (measured 2026-09-05, fresh-connection readback).
pub const DEFAULT_WAL_AUTOCHECKPOINT_PAGES: u32 = 1_000;

/// The `busy_timeout` the pool has set per-connection since v1.10. Kept in
/// the same batch so the connection-init contract stays one string.
pub const POOL_BUSY_TIMEOUT_MS: u32 = 5_000;

impl Durability {
    /// The per-connection pragma batch. Applied by the named pool-init fn in
    /// `server::bootstrap`; byte-stable so the readback pin can assert on it.
    pub fn pragma_batch(&self) -> String {
        format!(
            "PRAGMA busy_timeout={}; \
             PRAGMA synchronous={}; \
             PRAGMA wal_autocheckpoint={};",
            POOL_BUSY_TIMEOUT_MS,
            self.synchronous_mode.pragma_value(),
            self.wal_autocheckpoint_pages
        )
    }

    /// Apply the batch to a connection (the pool-init body, here so the
    /// readback pin exercises the exact production code path).
    pub fn apply(&self, conn: &mut rusqlite::Connection) -> rusqlite::Result<()> {
        conn.execute_batch(&self.pragma_batch())
    }
}

/// The envelope for the active target. Defaults can be tightened via env vars
/// (`CAPACITY_MAX_DOCS`, `CAPACITY_MAX_DB_MIB`, `CAPACITY_MAX_RSS_MIB`) for
/// testing or constrained deployments.
///
/// The durability fields are DELIBERATELY not env-overridable here: the other
/// knobs default silently on a bad value, which is acceptable for capacity
/// ceilings but not for durability policy — those two resolve through
/// `config::resolve_synchronous` / `config::resolve_wal_autocheckpoint`, where
/// an unknown value REFUSES BOOT (the `BRAIN_WRITE_POSTURE` pattern). The
/// envelope fields are the per-target defaults, pinned by
/// `envelope_defaults_equal_current_behavior`.
pub struct CapacityEnvelope {
    pub max_docs: usize,
    pub max_db_mib: u64,
    pub max_rss_mib: u64,
    /// Concurrent-truth ceiling (the Throughput milestone): the p95 of /search
    /// under the bench's client fan-out that a target must stay under for
    /// the run to pass `BENCH_ENVELOPE`. Values are measured constants set
    /// from live runs minus margin — see the consts below and
    /// docs/THROUGHPUT_PROOF_20260905.md.
    pub search_p95_ms_ceiling: u64,
    /// Per-connection durability default (Headroom). `Full` — the measured
    /// pre-Headroom effective behavior; see [`SynchronousMode`].
    pub synchronous_mode: SynchronousMode,
    /// Per-connection autocheckpoint default (Headroom): SQLite's compile
    /// default 1 000 pages, again the measured pre-Headroom behavior.
    pub wal_autocheckpoint_pages: u32,
}

impl CapacityEnvelope {
    pub fn for_target(target: CapacityTarget) -> Self {
        let (max_docs, max_db_mib, max_rss_mib, search_p95_ms_ceiling) = match target {
            // v1.16.x: RSS ceiling raised 320 → 512 MiB (the 320 cap was tuned
            // to a 4 GB Jetson; the live desktop install runs ~180–320 MiB and
            // a transient spike (large /multi-get, backup pass) must not sit
            // in the warning band. RSS is a soft signal anyway (Warning only,
            // never blocks writes).
            // v1.28 "Caliber": Desktop RSS 512 → 1024 MiB — the neural tiers
            // (gte-base-en-v1.5 + bge-reranker-v2-m3) measured ~830 MiB live;
            // 512 would pin the warning band permanently on desktop hardware.
            // Jetson stays 512 (the 4 GB edge contract — edge-default runs the
            // static potion model, ~340 MiB, well under it).
            CapacityTarget::Desktop => (50_000, 2_048, 1024, P95_CEILING_DESKTOP),
            CapacityTarget::Jetson => (10_000, 512, 512, P95_CEILING_JETSON),
        };
        // Durability defaults do NOT vary by target: both envelopes ran the
        // SQLite compile defaults before Headroom (nothing ever set them
        // per-connection), so the behavior-neutral value is the same constant.
        Self::from_env(
            max_docs,
            max_db_mib,
            max_rss_mib,
            search_p95_ms_ceiling,
            SynchronousMode::Full,
            DEFAULT_WAL_AUTOCHECKPOINT_PAGES,
        )
    }

    /// Layer env-var overrides on top of the built-in defaults. Tests use this
    /// to drive the envelope below the live corpus so they can exercise the
    /// 507 path without ingesting 10k real docs.
    fn from_env(
        d_docs: usize,
        d_db: u64,
        d_rss: u64,
        d_p95: u64,
        d_sync: SynchronousMode,
        d_wac: u32,
    ) -> Self {
        let parse_usize = |k: &str, d: usize| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(d)
        };
        let parse_u64 = |k: &str, d: u64| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(d)
        };
        Self {
            max_docs: parse_usize("CAPACITY_MAX_DOCS", d_docs),
            max_db_mib: parse_u64("CAPACITY_MAX_DB_MIB", d_db),
            max_rss_mib: parse_u64("CAPACITY_MAX_RSS_MIB", d_rss),
            search_p95_ms_ceiling: parse_u64("CAPACITY_MAX_P95_MS", d_p95),
            // Passed through untouched: durability resolves fail-closed at
            // boot (config.rs), never silently here — see the struct docs.
            synchronous_mode: d_sync,
            wal_autocheckpoint_pages: d_wac,
        }
    }
}

/// The capacity state of the running server, reported via `/health` and
/// consulted on write paths. `Exceeded` blocks new ingests with HTTP 507;
/// read routes ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityStatus {
    Ok,
    Warning,
    Exceeded,
}

impl CapacityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "warning",
            Self::Exceeded => "exceeded",
        }
    }

    /// Numeric severity: Ok=0, Warning=1, Exceeded=2. Used by the monotonicity
    /// proptest to verify increasing inputs never improve the status.
    pub fn severity(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Warning => 1,
            Self::Exceeded => 2,
        }
    }

    /// True when a new write should be rejected with HTTP 507.
    pub fn blocks_writes(self) -> bool {
        matches!(self, Self::Exceeded)
    }
}

/// Decide the capacity status from the current measurements.
///
/// - `docs` and `db_mib` are **directly controllable** by the operator (delete
///   content, compact the DB). Breaching either is `Exceeded` → HTTP 507.
/// - `rss_mib` is **not directly controllable** (it depends on SQLite cache
///   pressure, model2vec's static footprint, and fragmentation). A breach is
///   `Warning` only — surfaced in `/health` for operator awareness but never
///   blocking writes. This prevents a transient RSS spike (e.g. a large
///   `/multi-get`) from turning the brain read-only.
///
/// Pure; no side effects.
pub fn classify(docs: usize, db_mib: u64, rss_mib: u64, env: &CapacityEnvelope) -> CapacityStatus {
    // Hard gates: directly controllable. Breach → Exceeded → 507 on writes.
    if docs > env.max_docs || db_mib > env.max_db_mib {
        return CapacityStatus::Exceeded;
    }
    // RSS: not directly controllable. Breach → Warning only (never blocks writes).
    // The warning band (90% of any ceiling) and the hard docs/db limits still apply.
    let docs_near = (docs as f64) > env.max_docs as f64 * 0.9;
    let db_near = (db_mib as f64) > env.max_db_mib as f64 * 0.9;
    let rss_over = rss_mib > env.max_rss_mib;
    let rss_near = (rss_mib as f64) > env.max_rss_mib as f64 * 0.9;
    if docs_near || db_near || rss_over || rss_near {
        return CapacityStatus::Warning;
    }
    CapacityStatus::Ok
}

/// The stored-documents count feeding the envelope: fail-open by contract
/// (a read error counts ZERO docs — a transient DB failure must never turn
/// the brain read-only). Shared by every ingest path's write guard.
pub fn knowledge_docs(conn: &rusqlite::Connection) -> usize {
    conn.query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0)
        .max(0) as usize
}

/// The open database's size in bytes, measured from SQLite itself
/// (`page_count × page_size`) rather than from a filesystem stat of a
/// state-derived path. The path-injection seam: the capacity surfaces used
/// to `fs::metadata(&state.db_path)`, which put an axum-State-derived path
/// into a path expression; measuring through the open connection leaves NO
/// path expression on the surface at all. Equals the main DB file's size
/// (WAL excluded from both). Best-effort: 0 when the pragma read fails —
/// the same fail-open posture as the stat it replaces.
pub fn db_size_bytes(conn: &rusqlite::Connection) -> u64 {
    let pages: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap_or(0);
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap_or(0);
    (pages.max(0) as u64).saturating_mul(page_size.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(docs: usize, db: u64, rss: u64) -> CapacityEnvelope {
        CapacityEnvelope {
            max_docs: docs,
            max_db_mib: db,
            max_rss_mib: rss,
            search_p95_ms_ceiling: u64::MAX,
            synchronous_mode: SynchronousMode::Full,
            wal_autocheckpoint_pages: DEFAULT_WAL_AUTOCHECKPOINT_PAGES,
        }
    }

    // ── Headroom pins ──────────────────────────────────────────────────

    /// The release's thesis, pinned: the new envelope fields equal the
    /// PRE-CHANGE effective behavior for every target. Measured 2026-09-05
    /// against the bundled rusqlite 0.40.1: a fresh connection to the WAL
    /// database (the exact shape of a pooled connection — only
    /// `busy_timeout=5000` in its init) reports `PRAGMA synchronous` = 2
    /// (FULL) and `PRAGMA wal_autocheckpoint` = 1000, because synchronous
    /// is per-connection and nothing ever set it. Changing either default is
    /// a BEHAVIOR change: it needs its own release, its own bench table,
    /// and this pin's deliberate edit — never a drive-by.
    #[test]
    fn envelope_defaults_equal_current_behavior() {
        for (target, name) in [
            (CapacityTarget::Desktop, "desktop"),
            (CapacityTarget::Jetson, "jetson"),
        ] {
            let e = CapacityEnvelope::for_target(target);
            assert_eq!(
                e.synchronous_mode,
                SynchronousMode::Full,
                "{name}: envelope synchronous default drifted from the pre-Headroom \
                 effective behavior (FULL, the SQLite compile default)"
            );
            assert_eq!(
                e.wal_autocheckpoint_pages, 1_000,
                "{name}: envelope wal_autocheckpoint default drifted from the pre-Headroom \
                 effective behavior (1000 pages, the SQLite compile default)"
            );
        }
    }

    /// The pragma batch reads back on a real connection through the exact
    /// production path (`Durability::apply`): FULL/1000 by default, and a
    /// tuned NORMAL/256 round-trips too — proving the env-override seam can
    /// actually move the connection's policy when an operator sets it.
    #[test]
    fn db_size_bytes_measures_through_the_open_connection() {
        // The path-injection closure: the capacity surfaces take NO filesystem
        // path argument — the size comes from the open database itself, so no
        // axum-State-derived path ever reaches a path expression.
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch("CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('x');")
            .expect("seed");
        let size = db_size_bytes(&conn);
        let page_size: i64 = conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .expect("page_size");
        assert!(size > 0, "a seeded db must measure non-zero");
        assert_eq!(
            size as i64 % page_size,
            0,
            "the measurement is page-aligned: {size} vs page_size {page_size}"
        );
    }

    #[test]
    fn pool_init_pragmas_read_back() {
        let dir = std::env::temp_dir().join("brain_headroom_readback");
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let path = dir.join("readback.db");
        let _ = std::fs::remove_file(&path);
        // WAL is persistent: stamp it once so the connection is in the same
        // journal mode the live DB is (synchronous semantics do not differ by
        // journal mode here, but the readback should mirror production).
        {
            let c = rusqlite::Connection::open(&path).expect("open");
            c.execute_batch("PRAGMA journal_mode=WAL;").expect("wal");
        }
        let mut conn = rusqlite::Connection::open(&path).expect("open 2");
        Durability::default()
            .apply(&mut conn)
            .expect("pragma batch applies");
        let sync: i64 = conn
            .query_row("PRAGMA synchronous", [], |r| r.get(0))
            .expect("synchronous readback");
        let wac: i64 = conn
            .query_row("PRAGMA wal_autocheckpoint", [], |r| r.get(0))
            .expect("autocheckpoint readback");
        assert_eq!(sync, 2, "default durability reads back FULL (=2)");
        assert_eq!(wac, 1_000, "default autocheckpoint reads back 1000");

        // The tuned posture round-trips (what BRAIN_SYNCHRONOUS=normal +
        // BRAIN_WAL_AUTOCHECKPOINT=256 produce at boot).
        let tuned = Durability {
            synchronous_mode: SynchronousMode::Normal,
            wal_autocheckpoint_pages: 256,
        };
        tuned.apply(&mut conn).expect("tuned batch applies");
        let sync: i64 = conn
            .query_row("PRAGMA synchronous", [], |r| r.get(0))
            .expect("synchronous readback 2");
        let wac: i64 = conn
            .query_row("PRAGMA wal_autocheckpoint", [], |r| r.get(0))
            .expect("autocheckpoint readback 2");
        assert_eq!(sync, 1, "NORMAL reads back (=1)");
        assert_eq!(wac, 256, "tuned autocheckpoint reads back 256");
        let _ = std::fs::remove_file(&path);
    }

    /// The batch carries the unchanged busy_timeout alongside the new policy
    /// — the connection-init contract stays one string, one seam.
    #[test]
    fn pragma_batch_keeps_busy_timeout() {
        let b = Durability::default().pragma_batch();
        assert!(
            b.contains(&format!("PRAGMA busy_timeout={}", POOL_BUSY_TIMEOUT_MS)),
            "batch must keep the historical 5000 ms busy_timeout: {b}"
        );
        assert!(b.contains("PRAGMA synchronous=FULL"), "{b}");
        assert!(b.contains("PRAGMA wal_autocheckpoint=1000"), "{b}");
    }

    // ponytail: classify is the one non-trivial bit in the capacity surface —
    // these asserts fail immediately if the thresholds drift.
    #[test]
    fn classify_ok_when_well_under_all_ceilings() {
        let e = env(10_000, 512, 320);
        assert_eq!(classify(1_000, 100, 200, &e), CapacityStatus::Ok);
    }

    #[test]
    fn classify_warning_within_10pct_of_a_ceiling() {
        let e = env(10_000, 512, 320);
        // 9_501 > 9_000 (90% of 10k) → warning, even though under the hard limit.
        assert_eq!(classify(9_501, 100, 200, &e), CapacityStatus::Warning);
    }

    #[test]
    fn classify_exceeded_at_any_hard_limit() {
        let e = env(10_000, 512, 320);
        // docs + db are hard gates → Exceeded.
        assert_eq!(classify(10_001, 100, 200, &e), CapacityStatus::Exceeded);
        assert_eq!(classify(1_000, 513, 200, &e), CapacityStatus::Exceeded);
        // RSS is a SOFT signal — a breach is Warning, not Exceeded (RSS is not
        // directly controllable; a spike must not turn the brain read-only).
        assert_eq!(classify(1_000, 100, 321, &e), CapacityStatus::Warning);
        assert_eq!(classify(1_000, 100, 400, &e), CapacityStatus::Warning);
    }

    #[test]
    fn exceeded_status_blocks_writes() {
        assert!(!CapacityStatus::Ok.blocks_writes());
        assert!(!CapacityStatus::Warning.blocks_writes());
        assert!(CapacityStatus::Exceeded.blocks_writes());
    }

    #[test]
    fn capacity_envelope_env_overrides_docs_limit() {
        let prev = std::env::var("CAPACITY_MAX_DOCS").ok();
        unsafe { std::env::set_var("CAPACITY_MAX_DOCS", "5") };
        let env = CapacityEnvelope::for_target(CapacityTarget::Jetson);
        assert_eq!(
            env.max_docs, 5,
            "CAPACITY_MAX_DOCS env var must override the Jetson default of 10k"
        );
        match prev {
            Some(v) => unsafe { std::env::set_var("CAPACITY_MAX_DOCS", v) },
            None => unsafe { std::env::remove_var("CAPACITY_MAX_DOCS") },
        }
    }

    #[test]
    fn capacity_envelope_env_overrides_db_mib() {
        let prev = std::env::var("CAPACITY_MAX_DB_MIB").ok();
        unsafe { std::env::set_var("CAPACITY_MAX_DB_MIB", "50") };
        let env = CapacityEnvelope::for_target(CapacityTarget::Jetson);
        assert_eq!(
            env.max_db_mib, 50,
            "CAPACITY_MAX_DB_MIB env var must override the Jetson default of 512"
        );
        match prev {
            Some(v) => unsafe { std::env::set_var("CAPACITY_MAX_DB_MIB", v) },
            None => unsafe { std::env::remove_var("CAPACITY_MAX_DB_MIB") },
        }
    }

    #[test]
    fn capacity_classify_docs_exceeded_with_env_override() {
        // Integration: classify returns Exceeded when env-constrained max_docs
        // is breached. This proves the env → CapacityEnvelope → classify wire
        // that guard_capacity relies on at runtime.
        let prev = std::env::var("CAPACITY_MAX_DOCS").ok();
        unsafe { std::env::set_var("CAPACITY_MAX_DOCS", "5") };
        let env = CapacityEnvelope::for_target(CapacityTarget::Jetson);
        assert_eq!(
            classify(10, 0, 0, &env),
            CapacityStatus::Exceeded,
            "10 docs must exceed env-overridden limit of 5"
        );
        match prev {
            Some(v) => unsafe { std::env::set_var("CAPACITY_MAX_DOCS", v) },
            None => unsafe { std::env::remove_var("CAPACITY_MAX_DOCS") },
        }
    }

    // classify() is monotonic — increasing docs/db/rss never
    // improves the status.
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn proptest_classify_is_monotonic(
            docs in 0u64..100_000u64,
            db_mib in 0u64..4_000u64,
            rss_mib in 0u64..2_000u64
        ) {
            let env = CapacityEnvelope::for_target(CapacityTarget::Desktop);
            let docs_usize = docs as usize;
            let s1 = classify(docs_usize, db_mib, rss_mib, &env);
            let worse_docs = docs_usize + (docs_usize / 10).max(1);
            let s2 = classify(worse_docs, db_mib, rss_mib, &env);
            prop_assert!(s2.severity() >= s1.severity(),
                "increasing docs from {docs} to {worse_docs} must not improve status: {s1:?} -> {s2:?}");
        }
    }
}
