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

    /// The boot-lazy load state (the first-run download surface). Every
    /// eagerly-constructed backend is `Ready` by definition — only the
    /// [`LazyEmbedder`] overrides it, which is the point: the server BINDS
    /// while the model still downloads, and the app names that posture
    /// instead of staring at a dead port (the S-4 clean-machine witness
    /// measured the old boot order timing out the app's 10-second wait on
    /// every first launch: 16.8 s of HuggingFace fetch inside a 10 s
    /// window).
    fn model_state(&self) -> ModelState {
        ModelState::Ready
    }

    /// The load failure, when [`Self::model_state`] is `Failed` — named in
    /// `/health/db`'s `model_error` so a stuck onboarding names its cause.
    fn model_error(&self) -> Option<String> {
        None
    }

    /// Block until the model reaches a terminal state (`Ready`/`Failed`) or
    /// `timeout` elapses, then report the state. WRITE paths call this
    /// through [`encode_one_for_write`] when they meet the caller-skip
    /// shape: a write is NOT the deterministic answer path — its row either
    /// carries the vector at commit or refuses BY NAME. Eager backends are
    /// `Ready` by construction, so the default never blocks.
    fn wait_until_ready(&self, timeout: std::time::Duration) -> ModelState {
        let _ = timeout;
        self.model_state()
    }
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
///
/// R53a: also the `Encode` surface's single counting seam. Every backend
/// funnels its texts through here before handing them to a model (the static
/// backend at `:214`, bge-m3's multi-output at `:303`, gte at `:383`), so this
/// is where one counter covers all of them. It counts TEXTS, not batches:
/// embed cost scales with texts, and a per-text denominator is the one an
/// operator can compare against a document count.
///
/// The truncation is unconditional and reads only the input's own length — the
/// embed budget is a property of the *input size*, not a routing decision, and
/// nothing here or upstream of it selects a model from content. The embedder
/// itself is chosen once at boot from the retrieval profile.
pub(crate) fn embed_input(text: &str) -> &str {
    crate::decision_class::note_call(crate::decision_class::DecisionClass::Encode);
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
    use super::{EmbedError, Embedder, embed_input};
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

// ── Factory: profile → backend ──────────────────────────────────────────

/// The boot-lazy model state. `Loading` means the HuggingFace fetch (or the
/// local load) is still running in the background thread — the server is UP
/// and serving; `Ready` means encode calls hit the real model; `Failed`
/// means the fetch errored (named in `/health/db`'s `model_error`). Recall
/// in the Loading/Failed states returns the documented caller-skip shape
/// (missing embedding ⇒ empty vec ⇒ caller skips): the deterministic
/// answer path works WITHOUT the model — the fallback is a decision,
/// never a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelState {
    Loading,
    Ready,
    Failed,
}

impl ModelState {
    /// The wire spelling (`/health/db`'s additive `model_state` field).
    pub fn as_str(self) -> &'static str {
        match self {
            ModelState::Loading => "loading",
            ModelState::Ready => "ready",
            ModelState::Failed => "failed",
        }
    }
}

/// The bounded write-path wait through the bind-first load window. Measured:
/// a WARM cache lands the model in 1–3 s, which this covers; a COLD first-run
/// fetch (16.8 s measured) is deliberately NOT waited out — the app's
/// Settings surface names the loading posture, and the write refuses BY NAME
/// so the caller retries when `/health/db` reports `ready`. A const, not an
/// env knob: the bound is part of the write contract, not an operator
/// posture.
pub const WRITE_READY_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// The write-path encode: one text, embedding AT COMMIT or a refusal that
/// NAMES the model posture.
///
/// Reads (recall, suggest) ride the caller-skip law — the deterministic
/// answer path works without the model. Writes do not: a `knowledge` row
/// whose vector silently missing degrades recall quality with no signal, so
/// the write seams funnel through HERE. Meeting the caller-skip shape waits
/// ONCE (bounded by [`WRITE_READY_WAIT`]) for the background load; a model
/// that is `Failed` — or still `Loading` when the bound expires — refuses
/// with its state, and the caller maps that to its own envelope. A `Ready`
/// model that still yields an empty vector is NOT this function's business:
/// that is a true encode failure, and the caller's existing error stands.
pub fn encode_one_for_write(model: &dyn Embedder, text: &str) -> Result<Vec<f32>, ModelState> {
    encode_one_for_write_within(model, text, WRITE_READY_WAIT)
}

/// The bound-taking core (the pins' seam: arm 3 drives a SHORT bound so the
/// expiry shape is proven in milliseconds, not five seconds).
pub(crate) fn encode_one_for_write_within(
    model: &dyn Embedder,
    text: &str,
    bound: std::time::Duration,
) -> Result<Vec<f32>, ModelState> {
    let embedding = model.encode_one(text);
    if !embedding.is_empty() {
        return Ok(embedding);
    }
    let state = model.wait_until_ready(bound);
    if state != ModelState::Ready {
        return Err(state);
    }
    Ok(model.encode_one(text))
}

/// The store dimension per retrieval profile — WITHOUT loading the model.
///
/// The migration stamps `embedding_dim` from the embedder's `store_dim()`,
/// which historically forced the full model download to complete BEFORE
/// the migration could run, before the port could bind — the app's bounded
/// 10-second sidecar wait starved on every clean machine (measured in the
/// S-4 clean-machine witness: 16.8 s of first-fetch inside the window, and
/// the app killed the kernel at 10 s — the first-run defect, reproduced).
/// The dimension is a KNOWN CONSTANT of each profile's model, so the table
/// below resolves it statically and the model itself loads in the
/// background after bind.
///
/// PINNED BY `dim_table_matches_the_loaded_factory`: every row must equal
/// the loaded embedder's `store_dim()` for the same profile — the table is
/// a mirror of the factory, and the pin is what keeps it a mirror.
pub fn dim_for_profile(profile: &str) -> usize {
    match profile {
        #[cfg(feature = "neural-embed")]
        "enterprise" => 1_024, // bge-m3 dense (NeuralEmbedder::new(…, 1024))
        #[cfg(feature = "neural-embed")]
        "desktop" => 768, // gte-base-en-v1.5 (GteEmbedder)
        // The static backends share the trait's default store_dim — the
        // `static_embedder_default_store_dim_is_512` pin holds it.
        _ => 512,
    }
}

type BoxedLoader = Box<dyn FnOnce() -> Result<Arc<dyn Embedder>, EmbedError> + Send>;
type Slot = Option<Result<Arc<dyn Embedder>, String>>;

/// The boot-lazy embedder: the server binds while the model downloads.
///
/// Constructed at boot in place of the eager `embedder_for_profile` call;
/// the real backend loads on a background thread (started immediately —
/// the fetch runs PARALLEL to the migrations, not after them). Encode
/// calls in the `Loading`/`Failed` states return the documented caller-skip
/// shape (empty vecs), so recall serves its deterministic answer without
/// the model and picks it up transparently the moment the slot fills. The
/// state rides `/health/db` via the trait's `model_state` (the app's
/// first-run download surface names it, with the model id).
pub struct LazyEmbedder {
    model_id: String,
    dim: usize,
    slot: std::sync::Mutex<Slot>,
    loader: std::sync::Mutex<Option<BoxedLoader>>,
}

impl LazyEmbedder {
    /// The production constructor: the loader is the real factory.
    pub fn new(profile: &str, model_id: &str, dim: usize) -> Self {
        let p = profile.to_string();
        Self::new_with_loader(profile, model_id, dim, move || embedder_for_profile(&p))
    }

    /// The test seam: inject the loader (a closure that either succeeds
    /// with an embedder or fails) — no network, no HF cache in a pin. The
    /// `profile` param is kept for signature parity with [`Self::new`] (the
    /// production loader captures it in its own closure).
    pub fn new_with_loader(
        _profile: &str,
        model_id: &str,
        dim: usize,
        loader: impl FnOnce() -> Result<Arc<dyn Embedder>, EmbedError> + Send + 'static,
    ) -> Self {
        Self {
            model_id: model_id.to_string(),
            dim,
            slot: std::sync::Mutex::new(None),
            loader: std::sync::Mutex::new(Some(Box::new(loader))),
        }
    }

    /// Spawn the background load. Called ONCE at boot, right after
    /// construction — the fetch runs parallel to the migrations, and the
    /// slot fills whenever it lands (the server is long since bound).
    pub fn start_background_load(self: &std::sync::Arc<Self>) {
        let me = std::sync::Arc::clone(self);
        std::thread::Builder::new()
			.name("embed-model-load".to_string())
			.spawn(move || {
				let loader = me
					.loader
					.lock()
					.ok()
					.and_then(|mut g| g.take());
				let Some(loader) = loader else {
					return; // already taken — never double-fetch
				};
				let outcome = loader().map_err(|e| e.to_string());
				match &outcome {
					Ok(_) => tracing::info!("background model load complete"),
					Err(e) => tracing::error!("background model load FAILED: {e} — recall serves the deterministic fallback; restart retries the fetch"),
				}
				if let Ok(mut slot) = me.slot.lock() {
					*slot = Some(outcome);
				}
			})
			.expect("spawn the embed-model-load thread");
    }
}

impl Embedder for LazyEmbedder {
    fn encode(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        let guard = self.slot.lock();
        match guard.as_deref() {
            // Ready: delegate verbatim (the golden-vector contract).
            Ok(Some(Ok(m))) => m.encode(texts),
            // Loading or Failed: the caller-skip law — "encode returns one
            // Vec<f32> per input, in order; a failed/empty input yields an
            // empty vec" — so the skip shape is ONE EMPTY VEC PER TEXT (row
            // alignment preserved; a whole-batch empty list would desync
            // every zip). One warn names the posture (the log is the
            // onboarding signal when a fetch is stuck).
            state => {
                tracing::warn!(
                    "model not ready ({}); skipping embed of {} text(s)",
                    match state {
                        Ok(Some(Err(_))) => "load failed",
                        _ => "still loading",
                    },
                    texts.len()
                );
                texts.iter().map(|_| Vec::new()).collect()
            }
        }
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn store_dim(&self) -> usize {
        self.dim
    }
    fn model_state(&self) -> ModelState {
        match self.slot.lock().as_deref() {
            Ok(Some(Ok(_))) => ModelState::Ready,
            Ok(Some(Err(_))) => ModelState::Failed,
            _ => ModelState::Loading,
        }
    }
    /// WRITE paths' bounded wait through the load window: poll the slot until
    /// it is terminal or the bound expires, then report the truth. `Failed`
    /// short-circuits on the FIRST poll — a dead fetch must not cost the
    /// caller five seconds to learn it. Polling (50 ms), not a condvar: the
    /// slot is a plain `Mutex<Option<..>>` shared with `encode`, and a
    /// 50 ms poll against a lock that is only ever held for microseconds is
    /// invisible beside the fetch it is waiting on.
    fn wait_until_ready(&self, timeout: std::time::Duration) -> ModelState {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let state = self.model_state();
            if state != ModelState::Loading {
                return state;
            }
            if std::time::Instant::now() >= deadline {
                return ModelState::Loading;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    fn model_error(&self) -> Option<String> {
        self.slot
            .lock()
            .as_deref()
            .ok()
            .and_then(|slot| slot.as_ref().and_then(|r| r.as_ref().err()).cloned())
    }
}

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

    /// BIND-FIRST (the S-4 clean-machine finding): the dim table must mirror
    /// the LOADED factory's `store_dim()` for every profile — the migration
    /// stamps the table's number while the model downloads, and a drift
    /// would stamp a dimension the real model never emits (a cross-dim
    /// store, the exact failure the profile law exists to prevent). The
    /// feature-gated rows mirror the factory's own cfg gates.
    #[test]
    fn dim_table_matches_the_loaded_factory() {
        for profile in [
            "edge-default",
            "quality-local",
            "air-gapped",
            "compact",
            "multilingual",
        ] {
            let table = dim_for_profile(profile);
            let loaded = embedder_for_profile(profile)
                .unwrap_or_else(|e| panic!("{profile}: factory load failed: {e}"))
                .store_dim();
            assert_eq!(
                table, loaded,
                "dim_for_profile({profile}) = {table} but the loaded embedder reports {loaded} — the table drifted from the factory"
            );
        }
        // The feature-gated rows: only assertable when the feature builds
        // (the factory's arms vanish without it — both resolve to the 512
        // static default then, which the loop above already covers).
        #[cfg(feature = "neural-embed")]
        {
            assert_eq!(
                dim_for_profile("enterprise"),
                embedder_for_profile("enterprise").unwrap().store_dim()
            );
            assert_eq!(
                dim_for_profile("desktop"),
                embedder_for_profile("desktop").unwrap().store_dim()
            );
        }
    }

    /// The lazy embedder starts `Loading`, serves the caller-skip shape
    /// (one empty vec PER TEXT — the documented row-alignment law), and
    /// flips to `Ready` when the slot fills — the encode picks the real
    /// model up TRANSPARENTLY. The loader is channel-gated so the loading
    /// phase is deterministic (an in-memory stub completes in µs — the
    /// race this pin's first draft lost).
    #[test]
    fn lazy_embedder_serves_skip_shape_then_picks_up_the_model() {
        struct Stub;
        impl Embedder for Stub {
            fn encode(&self, texts: &[&str]) -> Vec<Vec<f32>> {
                texts.iter().map(|t| vec![t.len() as f32]).collect()
            }
            fn model_id(&self) -> &str {
                "stub"
            }
        }
        // The gate: the loader blocks until THIS thread releases it, so
        // the loading-phase asserts run against a genuinely unfilled slot.
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let lazy = LazyEmbedder::new_with_loader("edge-default", "stub/model", 512, move || {
            let _ = gate.recv();
            Ok(Arc::new(Stub))
        });
        let lazy = Arc::new(lazy);
        lazy.start_background_load();
        // Loading (the thread is parked on the gate): the skip shape is one
        // EMPTY VEC PER TEXT — not an empty list (row alignment).
        assert_eq!(lazy.model_state(), ModelState::Loading);
        let during = lazy.encode(&["a", "bb"]);
        assert_eq!(during, vec![Vec::<f32>::new(), Vec::<f32>::new()]);
        // Release the loader; the slot fills and the SAME call site picks
        // the real model up transparently.
        drop(release);
        let mut waited = 0;
        while lazy.model_state() == ModelState::Loading && waited < 5_000 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 10;
        }
        assert_eq!(lazy.model_state(), ModelState::Ready);
        assert_eq!(lazy.encode(&["a", "bb"]), vec![vec![1.0], vec![2.0]]);
        assert!(lazy.model_error().is_none());
    }

    /// A failed load is a NAMED posture (`Failed` + the error string), not
    /// a silent permanent skip — `/health/db` carries both.
    #[test]
    fn lazy_embedder_names_a_failed_load() {
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let lazy = LazyEmbedder::new_with_loader("edge-default", "stub/model", 512, move || {
            let _ = gate.recv();
            Err(EmbedError::Load("hf unreachable".to_string()))
        });
        let lazy = Arc::new(lazy);
        lazy.start_background_load();
        assert_eq!(lazy.model_state(), ModelState::Loading);
        drop(release);
        let mut waited = 0;
        while lazy.model_state() == ModelState::Loading && waited < 5_000 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 10;
        }
        assert_eq!(lazy.model_state(), ModelState::Failed);
        // The error rides the EmbedError Display (the wire spelling).
        assert_eq!(
            lazy.model_error().as_deref(),
            Some("embedder load failed: hf unreachable")
        );
        // And the encode STILL serves the per-text skip shape (recall works).
        assert_eq!(lazy.encode(&["x"]), vec![Vec::<f32>::new()]);
    }

    /// The background thread lands its outcome in the slot (the production
    /// path: start_background_load → the slot fills without any test poke).
    #[test]
    fn background_load_fills_the_slot() {
        struct Stub;
        impl Embedder for Stub {
            fn encode(&self, texts: &[&str]) -> Vec<Vec<f32>> {
                texts.iter().map(|_| vec![1.0]).collect()
            }
            fn model_id(&self) -> &str {
                "stub-bg"
            }
        }
        let lazy = Arc::new(LazyEmbedder::new_with_loader(
            "edge-default",
            "stub/bg",
            512,
            || Ok(Arc::new(Stub)),
        ));
        lazy.start_background_load();
        let mut waited = 0;
        while lazy.model_state() == ModelState::Loading && waited < 5_000 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 10;
        }
        assert_eq!(lazy.model_state(), ModelState::Ready);
        assert_eq!(lazy.model_id(), "stub/bg");
        assert_eq!(lazy.store_dim(), 512);
    }

    /// The bind-first WRITE law, arm 1: a write that meets the load window
    /// WAITS (bounded) and lands the vector when the slot fills — the warm-
    /// cache window (1–3 s measured) is covered, so the common boot-window
    /// write succeeds instead of refusing.
    #[test]
    fn a_write_waits_through_the_load_window_and_lands() {
        struct Stub;
        impl Embedder for Stub {
            fn encode(&self, texts: &[&str]) -> Vec<Vec<f32>> {
                texts.iter().map(|_| vec![7.0]).collect()
            }
            fn model_id(&self) -> &str {
                "stub-write-wait"
            }
        }
        let lazy = Arc::new(LazyEmbedder::new_with_loader(
            "edge-default",
            "stub/write-wait",
            512,
            move || {
                std::thread::sleep(std::time::Duration::from_millis(150));
                Ok(Arc::new(Stub) as Arc<dyn Embedder>)
            },
        ));
        lazy.start_background_load();
        // The write arrives INSIDE the window (the loader is still asleep).
        assert_eq!(lazy.model_state(), ModelState::Loading);
        let got = encode_one_for_write(lazy.as_ref(), "payload")
            .expect("the write must land once the slot fills");
        // Anti-vacuity: the landed vector is the stub's, not an empty skip.
        assert_eq!(got, vec![7.0]);
        assert_eq!(lazy.model_state(), ModelState::Ready);
    }

    /// Arm 2: a FAILED load refuses BY NAME and FAST — the refusal names
    /// `Failed` (never a bare "generation failed") and does not burn the
    /// whole bound learning it.
    #[test]
    fn a_write_refuses_by_name_when_the_model_failed() {
        let lazy = Arc::new(LazyEmbedder::new_with_loader(
            "edge-default",
            "stub/write-fail",
            512,
            || Err(EmbedError::Load("hf unreachable".to_string())),
        ));
        lazy.start_background_load();
        let mut waited = 0;
        while lazy.model_state() == ModelState::Loading && waited < 5_000 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 10;
        }
        let t0 = std::time::Instant::now();
        let refusal = encode_one_for_write(lazy.as_ref(), "payload")
            .expect_err("a failed load must refuse the write");
        assert_eq!(refusal, ModelState::Failed, "the refusal NAMES the state");
        assert!(
            t0.elapsed() < WRITE_READY_WAIT,
            "Failed must short-circuit the wait, not burn the bound"
        );
    }

    /// Arm 3: a model that NEVER lands refuses `Loading` at the bound — the
    /// cold-first-run posture: named, bounded, retryable. The loader stalls
    /// past every bound the pin uses; the expiry shape is proven through the
    /// bound-taking seam in milliseconds (the production 5 s bound itself is
    /// the const `WRITE_READY_WAIT`, pinned by arms 1–2).
    #[test]
    fn a_write_refuses_loading_at_the_bound() {
        let lazy = Arc::new(LazyEmbedder::new_with_loader(
            "edge-default",
            "stub/write-stall",
            512,
            move || {
                std::thread::sleep(std::time::Duration::from_secs(30));
                Ok(Arc::new(StubNever) as Arc<dyn Embedder>)
            },
        ));
        lazy.start_background_load();
        let refusal = encode_one_for_write_within(
            lazy.as_ref(),
            "payload",
            std::time::Duration::from_millis(200),
        )
        .expect_err("a stalling load must refuse at the bound");
        assert_eq!(refusal, ModelState::Loading, "the bound expires as Loading");
        // The stall in the loader is deliberately 30 s: the helper must NOT
        // have waited for the slot — it must have refused at its own bound,
        // which is far shorter than the stall.
        assert_eq!(lazy.model_state(), ModelState::Loading);
    }

    /// Arm 3's stub: never actually reached (the loader sleeps past every
    /// bound this test uses); exists only to typecheck the loader closure.
    struct StubNever;
    impl Embedder for StubNever {
        fn encode(&self, _: &[&str]) -> Vec<Vec<f32>> {
            vec![vec![1.0]]
        }
        fn model_id(&self) -> &str {
            "stub-never"
        }
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
