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

/// The default embedder. Wraps `model2vec_rs::StaticModel` and delegates
/// verbatim — this is the no-behavior-change backend for `edge-default`,
/// `quality-local`, `air-gapped`, and `compact` (formerly `multilingual`). The golden-vector test
/// (`static_embedder_matches_model2vec_golden`, #[ignore] — operator-run, HF
/// fetch) proves byte-identical output to the pre-trait `StaticModel.encode`.
pub struct StaticEmbedder {
    inner: model2vec_rs::model::StaticModel,
    id: String,
}

impl StaticEmbedder {
    /// Load a model2vec static model. `silent=true` matches the existing
    /// `main.rs` boot call (`from_pretrained(id, None, Some(true), None)`).
    pub fn new(id: impl Into<String>) -> Result<Self, EmbedError> {
        let id = id.into();
        let inner = model2vec_rs::model::StaticModel::from_pretrained(&id, None, Some(true), None)
            .map_err(|e| EmbedError::Load(format!("model2vec {id}: {e}")))?;
        Ok(Self { inner, id })
    }
}

impl Embedder for StaticEmbedder {
    fn encode(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        // model2vec-rs 0.1.4: `encode(&self, sentences: &[String]) -> Vec<Vec<f32>>`.
        // It takes owned Strings (not generic AsRef<str>), so build the slice.
        // The one allocation per call is negligible vs the static-token lookup.
        let owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
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
            let owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
            let mut m = match self.inner.lock() {
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
            let owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
            let mut m = match self.inner.lock() {
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
