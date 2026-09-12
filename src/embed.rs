//! The embedding abstraction.
//!
//! isolates the model behind a trait so the active profile selects the backend
//! without every call site changing its model choice. The default impl delegates
//! to `model2vec_rs::StaticModel` (`potion-retrieval-32M` — the edge/Jetson
//! contract, byte-identical to today). The `neural-embed` feature adds a
//! transformer impl (`BAAI/bge-m3` via FastEmbed-rs) for the `enterprise`
//! profile, with its native sparse + ColBERT outputs exposed for a future 4th
//! RRF leg + late-interaction rerank.
//!
//! Why a trait, not a struct: `AppState` holds `Arc<dyn Embedder>`, so a recall
//! site is profile-agnostic — it calls `model.encode_one(&q)` whether the
//! backend is the static model2vec model or a 568M transformer. The ~10 encode
//! call sites become one mechanical edit each (`model.encode(...)→ encode_one`),
//! not a per-profile branch.
//!
//! Status: the abstraction + the static (default) path ship here, fully tested
//! in isolation. The `neural-embed` path is feature-gated and compiles only
//! with `--features neural-embed` (pulls `fastembed`). `AppState` rewiring +
//! the ~10 call-site edits are the gated follow-up (the wide-blast-radius step
//! to prototype before committing) — this module is the prototype.

#![deny(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

// ── Error ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum EmbedError {
    /// Model load failed (HF fetch outage, repo takeover, bad cache).
    Load(String),
}

impl std::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbedError::Load(m) => write!(f, "embedder load failed: {m}"),
        }
    }
}
impl std::error::Error for EmbedError {}

// ── The universal dense trait ───────────────────────────────────────────────

/// The dense-embedding contract every backend satisfies. Object-safe so
/// `AppState` can hold `Arc<dyn Embedder>`.
///
/// `encode` returns one `Vec<f32>` per input, in order. A failed/empty input
/// yields an empty vec — this matches today's `.into_iter().next().unwrap_or_default()`
/// contract exactly, so a call site that does `model.encode_one(&t)` preserves
/// the "missing embedding ⇒ empty vec ⇒ caller skips" behavior.
pub trait Embedder: Send + Sync {
    /// Encode a batch. Returns one vector per input, in order.
    ///
    /// The input is budgeted to [`MAX_EMBED_CHARS`] chars per text
    /// (the embed budget): embedding sits behind a process-wide model lock, and a
    /// 1 MiB ingest must not pin it (second-pass audit). Stored text stays
    /// verbatim — only the VECTOR input is clipped, identically for every
    /// caller, so no two paths can diverge into different vectors for the
    /// same chunk.
    fn encode(&self, texts: &[&str]) -> Vec<Vec<f32>>;

    /// Single-input convenience — the idiom the ~10 call sites use today
    /// (`model.encode(std::slice::from_ref(&t)).into_iter().next().unwrap_or_default()`).
    /// Default impl preserves that exact behavior.
    fn encode_one(&self, text: &str) -> Vec<f32> {
        self.encode(&[text]).into_iter().next().unwrap_or_default()
    }

    /// Store dimension — profile-derived, NOT a fixed 512. The edge static
    /// model is 512 (matches the current `vec0 int8[512]` store); a neural
    /// backend overrides this to its native dim (1024 for bge-m3). The
    /// migration creates `vec_knowledge` at the active embedder's `store_dim`.
    fn store_dim(&self) -> usize {
        512
    }

    /// The model id (HF repo or local path) — for `/health` reporting +
    /// provenance, mirroring the existing `MODEL_ID` surfacing.
    fn model_id(&self) -> &str;
}

// ── Default backend: model2vec StaticModel (edge/Jetson, unchanged) ─────────

/// The embed input budget: embedding is a resource-bound
/// operation behind a process-wide model lock — a 1 MiB ingest must not pin
/// it (second-pass audit). Stored text stays verbatim; only the VECTOR
/// input is clipped, identically at every backend's boundary (one helper,
/// all callers), so no two paths diverge into different vectors for the
/// same chunk. Pinned by `embed_input_is_budgeted`.
pub const MAX_EMBED_CHARS: usize = 8_000;

/// Saturation visibility for the embedder ceiling (std-only, zero new
/// dependencies).
///
/// The model is the tier's serialization point (the neural backends hold the
/// forward pass under a `Mutex`; the static backend is CPU-bound on the same
/// box). This gauge answers "how deep was the queue at the model" without
/// touching the lock discipline: `enter()` bumps in-flight and folds the
/// peak into `max_observed` (both `Relaxed` — each counter is independent
/// and only ever increases, the `concurrency.rs` precedent). The guard
/// decrements on drop, so early returns and panics cannot leak the count.
/// Read the peak from `/metrics`-side code via [`SatGauge::max_observed`];
/// [`SatGauge::current`] is the live depth. Keep the `Mutex` — the gauge
/// observes the ceiling, it never replaces the lock.
#[derive(Debug, Default)]
pub struct SatGauge {
    current: AtomicUsize,
    max_observed: AtomicUsize,
}

impl SatGauge {
    /// Enter the ceiling: bumps in-flight, folds the peak. Hold the
    /// returned guard for the whole critical section.
    pub fn enter(&self) -> SatGuard<'_> {
        let cur = self.current.fetch_add(1, Ordering::Relaxed) + 1;
        // Fold the peak: only ever raises max_observed; a lost CAS race
        // retries, a stale read just means another thread already raised it.
        let mut peak = self.max_observed.load(Ordering::Relaxed);
        while cur > peak {
            match self.max_observed.compare_exchange_weak(
                peak,
                cur,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => peak = actual,
            }
        }
        SatGuard { gauge: self }
    }

    /// Live queue depth at the model.
    pub fn current(&self) -> usize {
        self.current.load(Ordering::Relaxed)
    }

    /// Deepest queue observed since process start (monotonic).
    pub fn max_observed(&self) -> usize {
        self.max_observed.load(Ordering::Relaxed)
    }
}

/// RAII slot at the embedder ceiling. Drops the in-flight count on scope
/// exit — panic-safe by construction.
#[derive(Debug)]
pub struct SatGuard<'a> {
    gauge: &'a SatGauge,
}

impl Drop for SatGuard<'_> {
    fn drop(&mut self) {
        self.gauge.current.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Char-boundary-safe head truncation to [`MAX_EMBED_CHARS`].
pub(crate) fn embed_input(text: &str) -> &str {
    text.char_indices()
        .nth(MAX_EMBED_CHARS)
        .map_or(text, |(i, _)| &text[..i])
}

/// The default embedder. Wraps `model2vec_rs::StaticModel` and delegates
/// verbatim — this is the no-behavior-change backend for `edge-default`,
/// `quality-local`, `air-gapped`, and `compact` (formerly `multilingual`). The golden-vector test
/// (`static_embedder_matches_model2vec_golden`, #[ignore] — operator-run, HF
/// fetch) proves byte-identical output to the pre-trait `StaticModel.encode`.
pub struct StaticEmbedder {
    inner: model2vec_rs::model::StaticModel,
    id: String,
    /// Saturation gauge for the static ceiling (see [`SatGauge`]). Held for
    /// the whole `encode` call — the queue depth at the model is visible
    /// without changing the lock-free `&self` discipline of this backend.
    sat: SatGauge,
}

impl StaticEmbedder {
    /// Load a model2vec static model. `silent=true` matches the existing
    /// `main.rs` boot call (`from_pretrained(id, None, Some(true), None)`).
    pub fn new(id: impl Into<String>) -> Result<Self, EmbedError> {
        let id = id.into();
        let inner = model2vec_rs::model::StaticModel::from_pretrained(&id, None, Some(true), None)
            .map_err(|e| EmbedError::Load(format!("model2vec {id}: {e}")))?;
        Ok(Self {
            inner,
            id,
            sat: SatGauge::default(),
        })
    }

    /// Deepest encode queue observed on this backend (the ceiling signal).
    pub fn saturation_peak(&self) -> usize {
        self.sat.max_observed()
    }

    /// Live encodes in flight on this backend.
    pub fn saturation_current(&self) -> usize {
        self.sat.current()
    }
}

impl Embedder for StaticEmbedder {
    fn encode(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        // The gauge observes; the encode stays verbatim (byte-identical
        // contract — the golden-vector test pins the output).
        let _slot = self.sat.enter();
        // model2vec-rs 0.1.4: `encode(&self, sentences: &[String]) -> Vec<Vec<f32>>`.
        // It takes owned Strings (not generic AsRef<str>), so build the slice.
        // The one allocation per call is negligible vs the static-token lookup.
        let owned: Vec<String> = texts.iter().map(|s| embed_input(s).to_string()).collect();
        self.inner.encode(&owned)
    }
    fn model_id(&self) -> &str {
        &self.id
    }
}

// ── Neural backend: bge-m3 via FastEmbed-rs (feature-gated) ─────────────────
//
// bge-m3 (verified via FastEmbed-rs docs + the BAAI card, 2026-08-14): emits
// dense + sparse + ColBERT from ONE forward pass. FastEmbed-rs exposes this as
// `Bgem3Embedding::embed(&mut self, texts, batch) -> Bgem3EmbeddingOutput {
// dense, sparse, colbert }`. The `&mut self` requirement (ONNX session scratch)
// is absorbed by an interior `Mutex`, so the dense path still goes through the
// shared `Arc<dyn Embedder>` (immutable) — the lock is uncontended under
// brain-server's pooled-connection, one-embed-per-task model.

#[cfg(feature = "neural-embed")]
pub mod neural {
    use super::{EmbedError, Embedder};
    use std::sync::Mutex;

    /// All three bge-m3 outputs from one forward pass. A future consumer
    /// gets `sparse` (a 4th RRF leg, BM25-like) and `colbert` (a
    /// late-interaction rerank) at zero extra model load.
    pub struct MultiOutput {
        /// Dense vectors (1024-d) — the vector leg, same as `Embedder::encode`.
        pub dense: Vec<Vec<f32>>,
        /// Sparse lexical weights — a future 4th RRF leg.
        pub sparse: Vec<fastembed::SparseEmbedding>,
        /// Per-token ColBERT vectors — a future late-interaction rerank.
        pub colbert: Vec<Vec<Vec<f32>>>,
    }

    /// The enterprise-profile embedder. 1024-d dense, 8192 context, MIT.
    pub struct NeuralEmbedder {
        // `Bgem3Embedding::embed` takes `&mut self` (ONNX scratch buffers);
        // the Mutex lets the dense path stay `&self` for `Arc<dyn Embedder>`.
        /// Lock bounds (Headroom): the critical section is the full ONNX
        /// forward pass (dense+sparse+colbert) — CPU-bound, long hold, but
        /// no I/O, no nesting, no SQL. THE known serialization point of the
        /// enterprise tier: concurrent ingests queue here, and the acquire
        /// wait gauge is exactly what makes that queue visible. Poison:
        /// fail-closed-for-data (warn + empty output — the row is dropped,
        /// never a corrupt embedding). Request-path holder (embed seam),
        /// wait-measured.
        inner: Mutex<fastembed::Bgem3Embedding>,
        id: String,
        dim: usize,
    }

    impl NeuralEmbedder {
        /// Load bge-m3. fastembed 5.17.4 ships the quantized `BGEM3Q` variant
        /// (2-3× CPU speedup, the only published variant) — the Jetson-friendly
        /// choice that also runs on desktop. `intra_threads` can be capped via
        /// `new_with_threads`; the default uses all cores.
        pub fn new(model_id: impl Into<String>, dim: usize) -> Result<Self, EmbedError> {
            Self::new_with_threads(model_id, dim, None)
        }

        /// Same as [`new`](Self::new) with an explicit CPU-thread cap (1 for the
        /// Jetson, None = all cores on desktop).
        pub fn new_with_threads(
            model_id: impl Into<String>,
            dim: usize,
            intra_threads: Option<usize>,
        ) -> Result<Self, EmbedError> {
            let id = model_id.into();
            let mut options = fastembed::Bgem3InitOptions::new(fastembed::Bgem3Model::BGEM3Q)
                .with_show_download_progress(true);
            if let Some(n) = intra_threads {
                options = options.with_intra_threads(n);
            }
            let inner = fastembed::Bgem3Embedding::try_new(options)
                .map_err(|e| EmbedError::Load(format!("bge-m3-q {id}: {e}")))?;
            Ok(Self {
                inner: Mutex::new(inner),
                id,
                dim,
            })
        }

        /// The full dense+sparse+colbert output — the 4th-leg + rerank seam.
        /// Fails closed to an empty `MultiOutput` on lock/embed failure so a
        /// recall never panics on a model error — but the failure is `warn!`ed
        /// (never certify silence), and the caller's empty-vec guard drops
        /// the row rather than writing a corrupt embedding.
        pub fn embed_multi(&self, texts: &[&str]) -> MultiOutput {
            let owned: Vec<String> = texts.iter().map(|s| embed_input(s).to_string()).collect();
            let mut m = match crate::concurrency::mutex_guard_measured(&self.inner) {
                Ok(m) => m,
                Err(_) => {
                    tracing::warn!("embedder mutex poisoned; skipping encode (model outage)");
                    return MultiOutput {
                        dense: vec![],
                        sparse: vec![],
                        colbert: vec![],
                    };
                }
            };
            match m.embed(&owned, None) {
                Ok(out) => MultiOutput {
                    dense: out.dense,
                    sparse: out.sparse,
                    colbert: out.colbert,
                },
                Err(e) => {
                    tracing::warn!("model encode failed; skipping row: {e}");
                    MultiOutput {
                        dense: vec![],
                        sparse: vec![],
                        colbert: vec![],
                    }
                }
            }
        }
    }

    impl Embedder for NeuralEmbedder {
        fn encode(&self, texts: &[&str]) -> Vec<Vec<f32>> {
            // Dense-only path: reuse the multi-output forward pass but discard
            // sparse/colbert. A future optimization can add a dense-only ONNX
            // graph if the sparse/colbert heads' compute shows in the p95.
            self.embed_multi(texts).dense
        }
        fn store_dim(&self) -> usize {
            self.dim
        }
        fn model_id(&self) -> &str {
            &self.id
        }
    }

    /// The desktop-profile embedder: `Alibaba-NLP/gte-base-en-v1.5` (~137M, 768-d,
    /// FastEmbed `GTEBaseENV15`). Dense-only (no sparse/colbert heads — pair with
    /// the rerank tier for precision). 54.09 MTEB-retrieval / strong English at
    /// ~1/4 the bge-m3 footprint — the laptop/AMD-desktop tier.
    ///
    /// Why not `gte-modernbert-base` (55.33, the slightly-better model)? It's
    /// NOT in FastEmbed's enum — it needs the custom-ONNX path
    /// (`try_new_from_user_defined` + a manual HF fetch). `gte-base-en-v1.5` is
    /// the in-enum variant that ships the desktop tier with zero custom-ONNX
    /// risk; gte-modernbert-base is the verified-future upgrade.
    pub struct GteEmbedder {
        /// Lock bounds (Headroom): same shape as [`NeuralEmbedder::inner`] —
        /// CPU-bound ONNX inference under the lock (long hold, no I/O, no
        /// nesting, no SQL); poison fails closed-for-data (empty vec). The
        /// acquire wait is the desktop tier's model-queue signal.
        inner: Mutex<fastembed::TextEmbedding>,
        id: String,
    }

    impl GteEmbedder {
        pub fn new(model_id: impl Into<String>) -> Result<Self, EmbedError> {
            let id = model_id.into();
            let opts = fastembed::InitOptions::new(fastembed::EmbeddingModel::GTEBaseENV15)
                .with_show_download_progress(true);
            let inner = fastembed::TextEmbedding::try_new(opts)
                .map_err(|e| EmbedError::Load(format!("gte-base-en-v1.5 {id}: {e}")))?;
            Ok(Self {
                inner: Mutex::new(inner),
                id,
            })
        }
    }

    impl Embedder for GteEmbedder {
        fn encode(&self, texts: &[&str]) -> Vec<Vec<f32>> {
            let owned: Vec<String> = texts.iter().map(|s| embed_input(s).to_string()).collect();
            let mut m = match crate::concurrency::mutex_guard_measured(&self.inner) {
                Ok(m) => m,
                Err(_) => {
                    tracing::warn!("embedder mutex poisoned; skipping encode (model outage)");
                    return vec![];
                }
            };
            match m.embed(&owned, None) {
                Ok(embs) => embs.into_iter().map(|e| e.to_vec()).collect(),
                Err(e) => {
                    tracing::warn!("model encode failed; skipping row: {e}");
                    vec![]
                }
            }
        }
        fn store_dim(&self) -> usize {
            768
        }
        fn model_id(&self) -> &str {
            &self.id
        }
    }
}

// ── Factory: profile → backend ──────────────────────────────────────────────

/// Resolve the embedder for the active profile. Mirrors `config::model_id_for_profile`
/// but returns the typed backend. `edge-default` / `quality-local` / `air-gapped`
/// → static potion (the Jetson contract, byte-identical to today); `enterprise`
/// → bge-m3 (feature-gated); `compact` (legacy `multilingual`) → static potion-base-2M.
///
/// Profile + model-id constants live server-side in `config.rs`; this lib-level
/// factory hardcodes the three model ids (the same literals config uses) so the
/// lib stays free of the server-private `config` module. The server's boot path
/// calls this with `config::model_profile()`; the literals are the contract.
///
/// `AppState::model` becomes `Arc<dyn Embedder>` populated by this factory at
/// boot, replacing today's `Arc::new(StaticModel::from_pretrained(...))`. The
/// rewiring + the ~10 call-site edits are the gated follow-up to this module.
pub fn embedder_for_profile(profile: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
    match profile {
        #[cfg(feature = "neural-embed")]
        "enterprise" => Ok(Arc::new(neural::NeuralEmbedder::new("BAAI/bge-m3", 1024)?)),
        #[cfg(feature = "neural-embed")]
        "desktop" => Ok(Arc::new(neural::GteEmbedder::new(
            "Alibaba-NLP/gte-base-en-v1.5",
        )?)),
        "compact" | "multilingual" => {
            Ok(Arc::new(StaticEmbedder::new("minishlab/potion-base-2M")?))
        }
        _ => Ok(Arc::new(StaticEmbedder::new(
            "minishlab/potion-retrieval-32M",
        )?)),
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedder_trait_is_object_safe() {
        // The whole point: AppState holds Arc<dyn Embedder>. If this stops
        // compiling, the trait gained a non-object-safe item (generic, Self).
        fn _accepts_shared(s: Arc<dyn Embedder>) -> usize {
            s.store_dim()
        }
        let _ = _accepts_shared;
    }

    #[test]
    fn encode_one_preserves_the_idiom() {
        // A stub embedder that returns known vectors — proves encode_one wraps
        // encode the same way the call sites' .into_iter().next().unwrap_or_default() did.
        struct Stub;
        impl Embedder for Stub {
            fn encode(&self, texts: &[&str]) -> Vec<Vec<f32>> {
                texts
                    .iter()
                    .map(|t| {
                        if t.is_empty() {
                            vec![]
                        } else {
                            vec![t.len() as f32]
                        }
                    })
                    .collect()
            }
            fn model_id(&self) -> &str {
                "stub"
            }
        }
        let s = Stub;
        assert_eq!(s.encode_one("hello"), vec![5.0]); // 5 chars
        let empty: Vec<f32> = s.encode_one("");
        assert!(empty.is_empty()); // empty input ⇒ empty vec (the contract)
    }

    /// The vector input is budgeted (v1.28.76): a 1 MiB text clips to
    /// MAX_EMBED_CHARS chars at the embedder boundary — char-boundary-safe,
    /// short texts untouched.
    #[test]
    fn embed_input_is_budgeted() {
        assert_eq!(embed_input("short prose"), "short prose");
        let flood = "a".repeat(MAX_EMBED_CHARS * 3);
        assert_eq!(embed_input(&flood).len(), MAX_EMBED_CHARS);
        // Multi-byte boundary: the cut lands ON a char edge, never mid-char.
        let wide = "é".repeat(MAX_EMBED_CHARS + 10);
        assert_eq!(embed_input(&wide).chars().count(), MAX_EMBED_CHARS);
    }

    #[test]
    fn sat_gauge_tracks_peak_and_returns_to_rest() {
        // Unit pin for the gauge alone: peak folds monotonically, the count
        // returns to zero when every guard drops (early-return/panic safety
        // comes from Drop, exercised here by scope exit).
        let g = SatGauge::default();
        assert_eq!(g.current(), 0);
        assert_eq!(g.max_observed(), 0);
        {
            let _a = g.enter();
            assert_eq!(g.current(), 1);
            {
                let _b = g.enter();
                let _c = g.enter();
                assert_eq!(g.current(), 3);
                assert_eq!(g.max_observed(), 3);
            }
            assert_eq!(g.current(), 1, "dropped guards must decrement");
            assert_eq!(g.max_observed(), 3, "the peak never comes down");
        }
        assert_eq!(g.current(), 0);
        assert_eq!(g.max_observed(), 3);
    }

    /// Timed contention test (MEASURE): the embedder-ceiling shape — one
    /// `Mutex` (kept, never replaced) + the [`SatGauge`] observing it. Eight
    /// threads start together behind a barrier, each holding the ceiling for
    /// a simulated 50 ms forward pass. Asserts: (1) every output is correct
    /// (the ceiling never corrupts), (2) the gauge saw the full pile-up
    /// (peak == 8 — saturation is VISIBLE), (3) wall time proves
    /// serialization (≥ the better part of 8 × 50 ms — the ceiling works).
    /// The measured numbers print to stderr (`-- --nocapture` to see them).
    #[test]
    fn embedder_ceiling_serializes_contention_and_reports_peak() {
        use std::sync::{Barrier, Mutex};
        use std::time::{Duration, Instant};

        const THREADS: usize = 8;
        const HOLD: Duration = Duration::from_millis(50);

        struct CeiledStub {
            ceiling: Mutex<()>,
            sat: SatGauge,
        }
        impl Embedder for CeiledStub {
            fn encode(&self, texts: &[&str]) -> Vec<Vec<f32>> {
                let _slot = self.sat.enter();
                let _hold = self.ceiling.lock().unwrap();
                std::thread::sleep(HOLD); // the simulated forward pass
                texts.iter().map(|t| vec![t.len() as f32]).collect()
            }
            fn model_id(&self) -> &str {
                "ceiled-stub"
            }
        }

        let stub = std::sync::Arc::new(CeiledStub {
            ceiling: Mutex::new(()),
            sat: SatGauge::default(),
        });
        let barrier = std::sync::Arc::new(Barrier::new(THREADS + 1));
        let mut handles = Vec::with_capacity(THREADS);
        for i in 0..THREADS {
            let (s, b) = (
                std::sync::Arc::clone(&stub),
                std::sync::Arc::clone(&barrier),
            );
            handles.push(std::thread::spawn(move || {
                b.wait(); // all threads enter the ceiling together
                s.encode_one(&format!("payload-{i}"))
            }));
        }
        let t0 = Instant::now();
        barrier.wait(); // release the pile-up, start the clock
        let mut outs = Vec::with_capacity(THREADS);
        for h in handles {
            outs.push(h.join().expect("worker"));
        }
        let wall = t0.elapsed();
        let peak = stub.sat.max_observed();
        eprintln!(
            "embedder ceiling: {THREADS} threads × {} ms hold → wall={} ms peak={} rest={}",
            HOLD.as_millis(),
            wall.as_millis(),
            peak,
            stub.sat.current()
        );
        // (1) correctness under contention: each payload encodes to its own len.
        let mut lens: Vec<usize> = outs.iter().map(|v| v[0] as usize).collect();
        lens.sort_unstable();
        assert_eq!(
            lens,
            vec![9; THREADS],
            "every payload must survive the ceiling"
        );
        // (2) saturation visible: the gauge saw the whole pile-up.
        assert_eq!(peak, THREADS, "the gauge must report the full queue depth");
        assert_eq!(stub.sat.current(), 0, "all guards dropped at rest");
        // (3) serialization proved by the clock: 8 × 50 ms serialized = 400
        // ms; allow generous scheduler slop (barrier skew only ADDS time, and
        // `sleep` never returns early, so this bound is one-sided safe).
        assert!(
            wall >= Duration::from_millis(300),
            "wall={wall:?}: the ceiling must serialize, not parallelize"
        );
    }

    #[test]
    fn static_embedder_default_store_dim_is_512() {
        // The Jetson/edge contract: potion stays 512 to match the vec0 store.
        // A neural backend overrides this; the default does not.
        struct Static;
        impl Embedder for Static {
            fn encode(&self, _: &[&str]) -> Vec<Vec<f32>> {
                vec![]
            }
            fn model_id(&self) -> &str {
                "x"
            }
        }
        assert_eq!(Static.store_dim(), 512);
    }

    /// Golden-vector regression: `StaticEmbedder` output is byte-identical to
    /// the pre-trait `StaticModel.encode` for N fixture texts. The
    /// no-behavior-change proof for the edge path. `#[ignore]` because it
    /// fetches the model from HuggingFace (operator-run, like the eval harness).
    #[test]
    #[ignore = "fetches model from HF; run with --ignored to verify byte-parity"]
    fn static_embedder_matches_model2vec_golden() {
        let id = "minishlab/potion-retrieval-32M";
        let direct = model2vec_rs::model::StaticModel::from_pretrained(id, None, Some(true), None)
            .expect("load");
        let wrapped = StaticEmbedder::new(id).expect("load");
        let texts_owned: Vec<String> = [
            "hello world",
            "the quick brown fox",
            "embedding abstraction",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let expected: Vec<Vec<f32>> = direct.encode(&texts_owned);
        let texts: Vec<&str> = texts_owned.iter().map(String::as_str).collect();
        let got = wrapped.encode(&texts);
        assert_eq!(expected.len(), got.len(), "row count mismatch");
        for (i, (e, g)) in expected.iter().zip(got.iter()).enumerate() {
            assert_eq!(e.len(), g.len(), "dim mismatch at row {i}");
            for (j, (a, b)) in e.iter().zip(g.iter()).enumerate() {
                assert!((a - b).abs() < 1e-6, "row {i} dim {j}: {a} != {b}");
            }
        }
    }

    /// End-to-end neural path: load BGE-M3 via FastEmbed, encode one sentence,
    /// assert the dense output is 1024-d + the multi-output exposes sparse +
    /// colbert. Requires `--features neural-embed` + a one-time HF download
    /// (~600 MB for the quantized variant). The (a) verification of the v1.28
    /// architecture: dense+sparse+colbert really do come out of one pass in Rust.
    #[cfg(feature = "neural-embed")]
    #[test]
    #[ignore = "downloads BGE-M3 from HF (~600MB); run with --features neural-embed --ignored neural_loads_and_emits_three_outputs"]
    fn neural_loads_and_emits_three_outputs() {
        let model = super::neural::NeuralEmbedder::new("BAAI/bge-m3", 1024).expect("load");
        // The dense path (what AppState uses via the trait):
        let dense = model.encode_one("What is BGE-M3?");
        assert_eq!(dense.len(), 1024, "BGE-M3 dense must be 1024-d");
        // The full multi-output (v1.30's 4th leg + rerank seam):
        let mo = model.embed_multi(&[
            "What is BGE-M3?",
            "BGE-M3 is a multi-function embedding model.",
        ]);
        assert_eq!(mo.dense.len(), 2, "one dense vec per input");
        assert_eq!(
            mo.sparse.len(),
            2,
            "one sparse vec per input (the 4th RRF leg)"
        );
        assert_eq!(
            mo.colbert.len(),
            2,
            "one colbert vec per input (the rerank)"
        );
        assert!(
            !mo.sparse[0].indices.is_empty(),
            "sparse must carry lexical weights"
        );
        assert!(
            !mo.colbert[0].is_empty(),
            "colbert must carry token vectors"
        );
    }
}
