//! Loom (v1.28.60) — CPU parallelism as an opt-in, profile-gated tier.
//!
//! WHY GATED: the Jetson memory doctrine forbids spending cores on the edge
//! default, so parallelism is never a default. Activation requires ALL THREE:
//! the `loom` feature compiled, the capacity target != Jetson, and
//! `BRAIN_LOOM=1` (fail-closed parse — an unknown value refuses boot, the
//! `BRAIN_WRITE_POSTURE`/durability pattern; a typo must never silently mean
//! "off" for a feature the operator believes is on). The pool is capped at
//! min(cores-1, 4) so ingest never starves the tokio blocking pool on a
//! shared box.
//!
//! THE INVARIANT (load-bearing): no cross-chunk reduction. Every fan-out is
//! an ordered per-item map collected in input order — embeddings and decoded
//! vectors are pure fns of their input, so results are byte-identical to the
//! serial loop. Adding ANY cross-item reduction (a batched encode, a shared
//! accumulator, a parallel fold) breaks `loom_preserves_fused_ranks`; the
//! plan file (`IMPLEMENTATION_PLAN_v1.28.60_Loom.md`) enumerates the fan-out
//! sites exhaustively so a third site cannot arrive without amending it.
//!
//! NO ASYNC MIXING: the fan-out lives INSIDE a `spawn_blocking` body — rayon
//! never touches the tokio runtime's threads, and the tokio runtime never
//! touches rayon's. With the feature off (or the resolver saying off), every
//! site takes the `iter().map()` path: byte-identical work, today's shape.

/// How many loom worker threads a core count buys. The cap is the whole
/// point: min(cores-1, 4), floored at 1 so a single-core box still works.
pub fn cap_from(cores: usize) -> usize {
    cores.saturating_sub(1).clamp(1, 4)
}

fn core_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Why loom is off. The `/health/db` echo renders these as `off:<reason>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffReason {
    /// The `loom` feature was not compiled into this binary.
    NoFeature,
    /// The capacity target is Jetson — the memory doctrine wins.
    Jetson,
    /// Compiled + desktop-class, but the operator did not set `BRAIN_LOOM=1`.
    EnvOff,
}

/// The boot-resolved loom decision: one resolution, one truth — the pool
/// install, the two fan-out sites, and the `/health/db` echo all read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoomState {
    Active { threads: usize },
    Off { reason: OffReason },
}

impl Default for LoomState {
    /// The default-compiled reality: the feature is off unless shipped AND
    /// resolved on — test scaffolding (AppState literals) gets the truth.
    fn default() -> Self {
        LoomState::Off {
            reason: OffReason::NoFeature,
        }
    }
}

impl LoomState {
    /// The `/health/db` echo shape: `active (N threads)` | `off:<reason>`.
    pub fn describe(&self) -> String {
        match self {
            LoomState::Active { threads } => format!("active ({threads} threads)"),
            LoomState::Off { reason } => match reason {
                OffReason::NoFeature => "off:no-feature".to_string(),
                OffReason::Jetson => "off:jetson".to_string(),
                OffReason::EnvOff => "off:env".to_string(),
            },
        }
    }
}

/// Fail-closed `BRAIN_LOOM` parse: unset/empty → off; `0`/`1` → that value;
/// ANYTHING else is a boot refusal — in every feature state and on every
/// target (the parse runs before the other gates so a typo never slides).
fn parse_env(raw: Option<&str>) -> Result<bool, String> {
    match raw.unwrap_or("").trim() {
        "" | "0" => Ok(false),
        "1" => Ok(true),
        other => Err(format!("BRAIN_LOOM='{other}' is invalid; must be 0 or 1")),
    }
}

/// The pure decision core (unit-pinned below): fail-closed parse first, then
/// the three gates in the plan's order. On Jetson the TARGET is the reason —
/// the env is irrelevant (Jetson never looms, even with `BRAIN_LOOM=1`).
fn decide(
    compiled: bool,
    target: crate::capacity::CapacityTarget,
    requested: Result<bool, String>,
) -> Result<LoomState, String> {
    let requested = requested?;
    if !compiled {
        return Ok(LoomState::Off {
            reason: OffReason::NoFeature,
        });
    }
    if target == crate::capacity::CapacityTarget::Jetson {
        return Ok(LoomState::Off {
            reason: OffReason::Jetson,
        });
    }
    if requested {
        Ok(LoomState::Active {
            threads: cap_from(core_count()),
        })
    } else {
        Ok(LoomState::Off {
            reason: OffReason::EnvOff,
        })
    }
}

/// Boot-time resolution: `decide` over the real env + target + feature state.
/// Called from the bootstrap beside the durability resolution; an `Err`
/// refuses the boot there (fail-closed).
pub fn resolve() -> Result<LoomState, String> {
    decide(
        cfg!(feature = "loom"),
        crate::capacity::capacity_target(),
        parse_env(std::env::var("BRAIN_LOOM").ok().as_deref()),
    )
}

#[cfg(feature = "loom")]
use rayon::prelude::*;

#[cfg(feature = "loom")]
static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();

/// Build the capped pool. A failed build degrades to serial (log + None) —
/// the SAFE direction: serial is the pre-loom behavior, and a pool fault
/// must never block serving.
#[cfg(feature = "loom")]
fn build_pool(threads: usize) -> Option<rayon::ThreadPool> {
    match rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("brain-loom-{i}"))
        .build()
    {
        Ok(pool) => Some(pool),
        Err(e) => {
            tracing::warn!("loom pool build failed; fan-out stays serial: {e}");
            None
        }
    }
}

/// Install the global pool from the boot-resolved state. First call wins
/// (boot calls it exactly once); off/failed builds install `None`, which
/// keeps every site on the serial path.
#[allow(unused_variables)]
pub fn install(state: &LoomState) {
    #[cfg(feature = "loom")]
    {
        let built = match state {
            LoomState::Active { threads } => build_pool(*threads),
            LoomState::Off { .. } => None,
        };
        let _ = POOL.get_or_init(|| built);
    }
}

/// The installed pool, if loom is compiled AND resolved active AND the build
/// succeeded. `None` in every other case — the serial path.
#[cfg(feature = "loom")]
fn pool() -> Option<&'static rayon::ThreadPool> {
    POOL.get().and_then(|p| p.as_ref())
}

/// Whether the fan-out seam will actually parallelize (feature compiled +
/// pool installed at boot + build succeeded). The fan-out SITES consult this
/// to decide whether a pre-pass is worth building; being wrong in either
/// direction is only a performance question, never a correctness one.
pub fn active() -> bool {
    #[cfg(feature = "loom")]
    {
        pool().is_some()
    }
    #[cfg(not(feature = "loom"))]
    {
        false
    }
}

/// The one fan-out seam: an ORDERED per-item map, run on the capped loom pool
/// when active, serially otherwise. The collect is order-preserving in both
/// arms (rayon's `par_iter` over a slice is indexed) — that is the invariant
/// `loom_preserves_fused_ranks` pins. Items are pure per-item computations;
/// a cross-item reduction here is a plan amendment, not a refactor.
pub fn fan_out<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    #[cfg(feature = "loom")]
    if let Some(pool) = pool() {
        return pool.install(|| items.par_iter().map(&f).collect::<Vec<R>>());
    }
    items.iter().map(f).collect()
}

/// The fan-out seam against an EXPLICIT pool — the test seam for the loom
/// arm of the determinism pins (the global install is boot-owned and
/// once-only; tests must not fight over it). Feature-gated: without `loom`
/// there is no pool type to name.
#[cfg(feature = "loom")]
pub fn fan_out_with_pool<T, R, F>(pool: &rayon::ThreadPool, items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    pool.install(|| items.par_iter().map(&f).collect::<Vec<R>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── fail-closed parse ────────────────────────────────────────────────

    #[test]
    fn brain_loom_parse_is_fail_closed() {
        assert!(!parse_env(None).unwrap(), "unset → off");
        assert!(!parse_env(Some("")).unwrap(), "empty → off");
        assert!(!parse_env(Some(" 0 ")).unwrap());
        assert!(parse_env(Some("1")).unwrap());
        for bad in ["yes", "on", "2", "true", "01", "-1"] {
            assert!(parse_env(Some(bad)).is_err(), "'{bad}' must refuse");
        }
    }

    // ── the thread cap ───────────────────────────────────────────────────

    #[test]
    fn loom_thread_cap_respected() {
        // Pure cap: min(cores-1, 4), floored at 1 — pinned across the plausible
        // core range so a future change to the formula trips this by name.
        assert_eq!(cap_from(1), 1, "single-core: the floor holds");
        assert_eq!(cap_from(2), 1, "two cores: one worker (cores-1)");
        assert_eq!(cap_from(4), 3, "four cores: cores-1 under the cap");
        assert_eq!(cap_from(5), 4, "five cores: the cap bites");
        for cores in 6..=64 {
            assert_eq!(cap_from(cores), 4, "{cores} cores: still the cap");
        }
        // The real pool honors it: a built pool has exactly the cap's threads.
        #[cfg(feature = "loom")]
        {
            let cap = cap_from(core_count());
            let pool = build_pool(cap).expect("pool builds");
            assert_eq!(
                pool.current_num_threads(),
                cap,
                "the pool must carry exactly the capped thread count"
            );
            assert!((1..=4).contains(&cap));
        }
    }

    // ── the resolution matrix ────────────────────────────────────────────

    #[test]
    fn jetson_never_looms() {
        use crate::capacity::CapacityTarget;
        // The load-bearing row: Jetson + compiled + operator-opt-in is STILL
        // off — the target gate outranks the env.
        assert_eq!(
            decide(true, CapacityTarget::Jetson, Ok(true)).ok(),
            Some(LoomState::Off {
                reason: OffReason::Jetson
            }),
        );
        assert_eq!(
            decide(true, CapacityTarget::Jetson, Ok(false)).ok(),
            Some(LoomState::Off {
                reason: OffReason::Jetson
            }),
            "on Jetson the target is the reason, env or not"
        );
        // Desktop rows for contrast (the gate is the target, not the box).
        assert!(matches!(
            decide(true, CapacityTarget::Desktop, Ok(true)),
            Ok(LoomState::Active { .. })
        ));
        assert_eq!(
            decide(true, CapacityTarget::Desktop, Ok(false)).ok(),
            Some(LoomState::Off {
                reason: OffReason::EnvOff
            })
        );
        // Not compiled: off with its own reason even when the operator asks.
        assert_eq!(
            decide(false, CapacityTarget::Desktop, Ok(true)).ok(),
            Some(LoomState::Off {
                reason: OffReason::NoFeature
            })
        );
        // Fail-closed parse outranks everything — a typo refuses even on a
        // Jetson box with the feature absent.
        assert!(decide(false, CapacityTarget::Jetson, Err("yolo".into())).is_err());
        assert!(decide(true, CapacityTarget::Desktop, Err("yolo".into())).is_err());
    }

    // ── the ordered fan-out ──────────────────────────────────────────────

    #[test]
    fn fan_out_preserves_order_and_values() {
        let items: Vec<u64> = (0..1000).collect();
        let serial: Vec<u64> = items.iter().map(|i| i * i).collect();
        let fanned = fan_out(&items, |i| i * i);
        assert_eq!(fanned, serial, "order and values survive the seam");
        // Empty and single-item edges stay degenerate.
        let empty: Vec<u8> = Vec::new();
        assert!(fan_out(&empty, |i| *i).is_empty());
        assert_eq!(fan_out(&[7u8], |i| *i), vec![7]);
    }

    #[cfg(feature = "loom")]
    #[test]
    fn fan_out_with_pool_matches_serial() {
        let pool = build_pool(cap_from(core_count())).expect("pool builds");
        let items: Vec<usize> = (0..500).collect();
        let serial: Vec<String> = items.iter().map(|i| format!("v{i}")).collect();
        let fanned = fan_out_with_pool(&pool, &items, |i| format!("v{i}"));
        assert_eq!(fanned, serial);
    }

    // THE order-invariant property (proptest): a batch's per-id results are
    // a function of the item, never of the item's position — shuffle the
    // batch and the per-id outputs must be identical. This is what makes
    // the fan-out safe for ingest (chunk sequence preserved by
    // construction, not by care).
    use proptest::prelude::*;
    proptest! {
        #[test]
        fn loom_batch_order_invariant(batch in proptest::collection::vec(any::<u64>(), 1..64)) {
            // Deterministic per-id "embedding": a pure fn of the payload.
            let embed = |payload: u64| -> [f32; 4] {
                [
                    (payload & 0xff) as f32 / 255.0,
                    ((payload >> 8) & 0xff) as f32 / 255.0,
                    ((payload >> 16) & 0xff) as f32 / 255.0,
                    ((payload >> 24) & 0xff) as f32 / 255.0,
                ]
            };
            // Unique ids: pair payload with its index as the id.
            let items: Vec<(u64, u64)> =
                batch.iter().enumerate().map(|(i, p)| (i as u64, *p)).collect();
            let canonical_by_id: std::collections::HashMap<u64, [f32; 4]> = fan_out(
                &items,
                |(id, p)| (*id, embed(*p)),
            )
            .into_iter()
            .collect();

            // Three deterministic permutations (reverse, rotate, odd/even
            // split) — the property must hold for ANY arrival order, so fixed
            // ones will do: a violation is order-sensitivity, not
            // permutation-coverage.
            let mut reversed = items.clone();
            reversed.reverse();
            let rotated: Vec<(u64, u64)> = {
                let (a, b) = items.split_at(items.len() / 2);
                b.iter().chain(a.iter()).copied().collect()
            };
            let mut evens_first: Vec<(u64, u64)> =
                items.iter().filter(|(i, _)| i % 2 == 0).copied().collect();
            evens_first.extend(items.iter().filter(|(i, _)| i % 2 == 1).copied());

            for permuted in [reversed, rotated, evens_first] {
                let got = fan_out(&permuted, |(id, p)| (*id, embed(*p)));
                for (id, vec) in got {
                    assert_eq!(
                        canonical_by_id.get(&id),
                        Some(&vec),
                        "per-id output moved when the batch was shuffled (id {id})"
                    );
                }
            }
        }
    }

    // ── the fused-rank pin ───────────────────────────────────────────────

    /// A deterministic stub embedder: a pure fn of the text (8-dim, weighted
    /// byte mix, length-salted). The pin's invariant is ORDER-preserving
    /// fan-out, not model fidelity — model output parity is embed.rs's
    /// golden pin (`static_embedder_matches_model2vec_golden`); a stub keeps
    /// this test CI-safe (no HuggingFace fetch).
    struct StubEmbedder;
    impl StubEmbedder {
        fn embed(&self, text: &str) -> Vec<f32> {
            let mut v = [0.0f32; 8];
            for (i, b) in text.bytes().enumerate() {
                v[i % 8] += (b as f32 / 255.0) * ((i % 5) as f32 + 1.0);
            }
            let salt = text.len() as f32;
            v.iter().map(|x| x / (salt + 1.0)).collect()
        }
    }

    /// Rank order by cosine (desc, id asc on ties) — the same shape the
    /// recall path's fusion feeds. Pure: no DB, no model fetch.
    fn rank_order(emb: &[(u64, Vec<f32>)], query: &[f32]) -> Vec<u64> {
        let mut scored: Vec<(u64, f32)> = emb
            .iter()
            .map(|(id, v)| (*id, crate::search::cosine_sim(query, v)))
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        scored.into_iter().map(|(id, _)| id).collect()
    }

    /// THE determinism pin: the frozen gold corpus embedded serially vs
    /// through the loom seam must give byte-equal per-doc blobs, and the
    /// fused rank order + eval metrics over those blobs must be identical.
    /// Serial arm always; under `--features loom` the loom arm runs through
    /// an explicit capped pool (the global install is boot-owned).
    #[test]
    fn loom_preserves_fused_ranks() {
        // The frozen gold corpus: varied lengths, repeated topics, exact and
        // near duplicates — the shapes that make mis-ordered fan-out visible.
        const GOLD: [&str; 12] = [
            "the retention policy expires stale evidence after ninety days",
            "the retention policy expires stale evidence after ninety days",
            "audit chains are hash-linked; every write settles inside the caller's transaction",
            "audit chains are hash-linked and every write settles with its evidence",
            "jetson nano 4gb runs the edge profile with the static potion embedder",
            "the desktop profile selects the neural bge-m3 embedder at boot",
            "read routes are never blocked by capacity; writes refuse with 507",
            "sanitize_read gates every emitted text field unconditionally",
            "the workflow lane begins immediate and settles the audit row atomically",
            "quarantined plants store flagged with no graph edges",
            "the vector index is vec0 int8 unit-quantized with binary sidecars",
            "recall fuses lexical and vector passes with reciprocal rank fusion",
        ];
        const QUERIES: [&str; 4] = [
            "retention expiry policy",
            "audit chain hash links",
            "jetson edge embedder",
            "vector int8 quantization",
        ];
        let model = StubEmbedder;
        let docs: Vec<(u64, String)> = GOLD
            .iter()
            .enumerate()
            .map(|(i, t)| (i as u64, (*t).to_string()))
            .collect();

        let embed_bytes = |t: &str| -> Vec<u8> {
            model
                .embed(t)
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect()
        };

        let serial: Vec<(u64, Vec<u8>)> =
            docs.iter().map(|(id, t)| (*id, embed_bytes(t))).collect();

        #[cfg(feature = "loom")]
        let loom: Vec<(u64, Vec<u8>)> = {
            let pool = build_pool(cap_from(core_count())).expect("pool builds");
            let emb = fan_out_with_pool(&pool, &docs, |(_, t)| model.embed(t));
            docs.iter()
                .zip(emb)
                .map(|((id, _), v)| {
                    (
                        *id,
                        v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>(),
                    )
                })
                .collect()
        };
        #[cfg(not(feature = "loom"))]
        let loom: Vec<(u64, Vec<u8>)> = fan_out(&docs, |(id, t)| (*id, embed_bytes(t)));

        // 1. Byte-equal blobs, per doc, in order.
        assert_eq!(
            serial, loom,
            "loom blobs diverged from serial — the fan-out is not order-preserving"
        );

        // 2. The fused rank order over the blobs is identical for every query.
        let emb_by: Vec<(u64, Vec<f32>)> = loom
            .iter()
            .map(|(id, blob)| {
                (
                    *id,
                    blob.chunks(4)
                        .map(|b| f32::from_le_bytes(b.try_into().expect("4-byte chunk")))
                        .collect(),
                )
            })
            .collect();
        for q in QUERIES {
            let qv = model.embed(q);
            let s_ranks = rank_order(&emb_by, &qv);
            let l_ranks = rank_order(&emb_by, &qv);
            assert_eq!(s_ranks, l_ranks, "fused rank order moved for query '{q}'");

            // 3. The eval metrics over the ranked lists are identical (the
            // "run brain eval" leg, in-process via the eval module).
            let relevant: Vec<i64> = s_ranks.iter().take(3).map(|id| *id as i64).collect();
            let retrieved_s: Vec<i64> = s_ranks.iter().take(5).map(|id| *id as i64).collect();
            let retrieved_l: Vec<i64> = l_ranks.iter().take(5).map(|id| *id as i64).collect();
            assert_eq!(
                crate::eval::mrr(&retrieved_s, &relevant),
                crate::eval::mrr(&retrieved_l, &relevant),
            );
            assert_eq!(
                crate::eval::precision_at_k(&retrieved_s, &relevant, 5),
                crate::eval::precision_at_k(&retrieved_l, &relevant, 5),
            );
        }
    }
}
