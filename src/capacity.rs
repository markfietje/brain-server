//! Capacity envelopes (v0.9.9 "Qualify").
//!
//! A configuration that exceeds these is *unsupported*: brain-server refuses
//! new ingests with HTTP 507 (Insufficient Storage) until the operator
//! resolves it. Read routes (`/search`, `/recall`, `/get`) are NEVER blocked —
//! an over-capacity brain must still answer. The numbers are documented in
//! `BENCHMARKS.md` §v0.9.9 and are measured, not estimated.
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

/// The envelope for the active target. Defaults can be tightened via env vars
/// (`CAPACITY_MAX_DOCS`, `CAPACITY_MAX_DB_MIB`, `CAPACITY_MAX_RSS_MIB`) for
/// testing or constrained deployments.
pub struct CapacityEnvelope {
    pub max_docs: usize,
    pub max_db_mib: u64,
    pub max_rss_mib: u64,
}

impl CapacityEnvelope {
    pub fn for_target(target: CapacityTarget) -> Self {
        let (max_docs, max_db_mib, max_rss_mib) = match target {
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
            CapacityTarget::Desktop => (50_000, 2_048, 1024),
            CapacityTarget::Jetson => (10_000, 512, 512),
        };
        Self::from_env(max_docs, max_db_mib, max_rss_mib)
    }

    /// Layer env-var overrides on top of the built-in defaults. Tests use this
    /// to drive the envelope below the live corpus so they can exercise the
    /// 507 path without ingesting 10k real docs.
    fn from_env(d_docs: usize, d_db: u64, d_rss: u64) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn env(docs: usize, db: u64, rss: u64) -> CapacityEnvelope {
        CapacityEnvelope {
            max_docs: docs,
            max_db_mib: db,
            max_rss_mib: rss,
        }
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
        std::env::set_var("CAPACITY_MAX_DOCS", "5");
        let env = CapacityEnvelope::for_target(CapacityTarget::Jetson);
        assert_eq!(
            env.max_docs, 5,
            "CAPACITY_MAX_DOCS env var must override the Jetson default of 10k"
        );
        match prev {
            Some(v) => std::env::set_var("CAPACITY_MAX_DOCS", v),
            None => std::env::remove_var("CAPACITY_MAX_DOCS"),
        }
    }

    #[test]
    fn capacity_envelope_env_overrides_db_mib() {
        let prev = std::env::var("CAPACITY_MAX_DB_MIB").ok();
        std::env::set_var("CAPACITY_MAX_DB_MIB", "50");
        let env = CapacityEnvelope::for_target(CapacityTarget::Jetson);
        assert_eq!(
            env.max_db_mib, 50,
            "CAPACITY_MAX_DB_MIB env var must override the Jetson default of 512"
        );
        match prev {
            Some(v) => std::env::set_var("CAPACITY_MAX_DB_MIB", v),
            None => std::env::remove_var("CAPACITY_MAX_DB_MIB"),
        }
    }

    #[test]
    fn capacity_classify_docs_exceeded_with_env_override() {
        // Integration: classify returns Exceeded when env-constrained max_docs
        // is breached. This proves the env → CapacityEnvelope → classify wire
        // that guard_capacity relies on at runtime.
        let prev = std::env::var("CAPACITY_MAX_DOCS").ok();
        std::env::set_var("CAPACITY_MAX_DOCS", "5");
        let env = CapacityEnvelope::for_target(CapacityTarget::Jetson);
        assert_eq!(
            classify(10, 0, 0, &env),
            CapacityStatus::Exceeded,
            "10 docs must exceed env-overridden limit of 5"
        );
        match prev {
            Some(v) => std::env::set_var("CAPACITY_MAX_DOCS", v),
            None => std::env::remove_var("CAPACITY_MAX_DOCS"),
        }
    }

    // v1.3.0 Bedrock M6: classify() is monotonic — increasing docs/db/rss never
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
