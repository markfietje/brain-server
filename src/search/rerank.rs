//! The profile-gated cross-encoder reranker.
//!
//! Sits after `rrf_fuse` (the rank-fusion stage in [`crate::search`]) and writes
//! into the reserved `rerank_score` / `rerank_truncated` slots on
//! [`crate::search::SearchResult`] provenance.
//!
//! Model resolution (highest quality first):
//!   1. `mixedbread-ai/mxbai-rerank-large-v1` — the golden pick (Apache-2.0,
//!      DeBERTa-v3-large single-label cross-encoder → `logits[:, 0]` score, which is the
//!      exact contract fastembed's `UserDefinedRerankingModel` seam expects). It is
//!      NOT in the FastEmbed registry enum (which only ships bge-reranker-base /
//!      bge-reranker-v2-m3 / jina v1-turbo / jina v2), so it's loaded through the
//!      BYO-ONNX seam from a local dir. Default location: `models/mxbai-rerank-large-v1/`
//!      (override with `BRAIN_RERANK_MODEL_DIR`). Uses the official int8
//!      `onnx/model_quantized.onnx` for CPU/footprint-friendly inference.
//!   2. Fallback: `bge-reranker-v2-m3` (FastEmbed-rs `BGERerankerV2M3`, in-enum,
//!      auto-downloads) when the mxbai files are absent or fail to load.
//!
//! Qwen3-Reranker-0.6B is deliberately NOT wired here: it is `Qwen3ForCausalLM`
//! (ChatML template, last-token logit scoring) — architecturally incompatible with
//! fastembed's rerank seam, which feeds bare `(query, doc)` pairs and reads
//! `logits[:, 0]`. It would load, run, and return meaningless scores. Using it
//! requires a real LLM runtime (llama.cpp / vLLM / TEI), out of scope for the seam.
//!
//! Profile gate: active only on `enterprise`/`desktop`. The Jetson/edge path
//! stays rerank-free (the tier was removed for the 8 s
//! recall timeout — re-added here only where the hardware can afford it).
//!
//! Fail-open contract: a model load failure or a per-call ONNX error leaves the
//! RRF order untouched (provenance stays `rerank_score = None`) — a recall never
//! panics or stalls on a reranker fault. This mirrors the `screen.rs` layer-2
//! fail-soft posture.
#![cfg(feature = "rerank-tier")]

use std::sync::{LazyLock, Mutex};

use fastembed::{
    OnnxSource, RerankInitOptions, RerankInitOptionsUserDefined, RerankerModel, TextRerank,
    TokenizerFiles, UserDefinedRerankingModel,
};

use crate::search::{SearchResult, SearchTelemetry};

/// Directory (relative to CWD or absolute) containing the mxbai-rerank-large-v1
/// files fastembed's BYO-ONNX seam needs: `onnx/model_quantized.onnx`,
/// `tokenizer.json`, `config.json`, `special_tokens_map.json`,
/// `tokenizer_config.json`. Override with `BRAIN_RERANK_MODEL_DIR`.
const DEFAULT_MXBAI_DIR: &str = "models/mxbai-rerank-large-v1";

/// The process-wide reranker handle, loaded lazily on first recall iff
/// `BRAIN_RERANK_ENABLED=1`. The server boot sets that env var when the active
/// profile is `enterprise`/`desktop` (search/mod.rs is lib code and must not
/// depend on the server-private `config` module, so the gate is an env flag the
/// server owns). Mirrors `screen::CLASSIFIER`'s LazyLock<Option<...>> posture:
/// load-on-first-use, fail-soft to None on a model fault (recall falls back to
/// the RRF order — never stalls on a reranker fault).
static RERANKER: LazyLock<Option<Reranker>> = LazyLock::new(|| {
    if std::env::var("BRAIN_RERANK_ENABLED").as_deref() != Ok("1") {
        return None;
    }
    let top_n = std::env::var("BRAIN_RERANK_TOP_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    match Reranker::new(top_n) {
        Ok(r) => {
            tracing::info!("reranker loaded: {} (top_n={top_n})", r.model_id());
            Some(r)
        }
        Err(e) => {
            tracing::warn!("reranker load failed (recall falls back to RRF order): {e}");
            None
        }
    }
});

/// The seam `perform_search_with_prf` calls after fusion + PRF. No-op (preserves
/// the RRF order, leaves `rerank_score = None`) when the reranker is off (edge
/// profile) or failed to load. Records `rerank_ms` into telemetry either way so
/// the provenance stays honest about whether a rerank stage ran.
pub fn maybe_rerank(query: &str, candidates: &mut [SearchResult], _tel: &mut SearchTelemetry) {
    let Some(r) = RERANKER.as_ref() else {
        return; // edge profile / load failure — RRF order stands
    };
    let _ = r.rerank(query, candidates); // fail-open inside
}

/// Force the lazy load at boot (the server calls this when it arms the tier).
/// Without it, the FIRST recall pays the model download inside the request
/// path — observed live: a 503 `recall timed out` on the first query while
/// bge-reranker-v2-m3 downloaded. Warm it before serving.
pub fn warmup() {
    LazyLock::force(&RERANKER);
}

/// The reranker handle, loaded once (the ONNX session is the expensive part).
/// `TextRerank::rerank` takes `&mut self` (ONNX scratch buffers), so the
/// `Mutex` lets the shared handle stay `&self` for the call sites — the lock is
/// uncontended under brain-server's pooled-connection, one-embed-per-task model
/// (same pattern as `embed::NeuralEmbedder` and `screen::onnx::OnnxScorer`).
pub struct Reranker {
    inner: Mutex<TextRerank>,
    /// The model id actually loaded (mxbai-rerank-large-v1 or bge-reranker-v2-m3).
    model_id: std::sync::Arc<str>,
    /// The max candidates scored per call. Reranking more than ~50–100 costs
    /// latency for diminishing rank-quality gain (IronCurtain's own cascade
    /// discipline uses ~50). Larger candidate sets are truncated and the
    /// `rerank_truncated` provenance flag reports it honestly.
    top_n: usize,
}

impl Reranker {
    /// `model_id() -> &str` reports which reranker actually loaded.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Try the golden `mxbai-rerank-large-v1` (BYO-ONNX seam, dir from
    /// `BRAIN_RERANK_MODEL_DIR` or the default), falling back to the in-registry
    /// `bge-reranker-v2-m3` when the XML files are absent or the load fails.
    /// Fail-open sits with the caller (`LazyLock` → `None`), so a missing model
    /// dir degrades to the in-enum model, never to a boot failure.
    pub fn new(top_n: usize) -> anyhow::Result<Self> {
        // Prefer the user-defined model: mxbai-rerank-large-v1 (golden).
        match Self::new_mxbai_user_defined(top_n) {
            Ok(inner) => Ok(Self {
                inner: Mutex::new(inner),
                model_id: std::sync::Arc::from("mixedbread-ai/mxbai-rerank-large-v1"),
                top_n,
            }),
            Err(e) => {
                tracing::warn!(
                    "mxbai-rerank-large-v1 via user-defined seam unavailable \
                     ({e}); falling back to bge-reranker-v2-m3"
                );
                let options = RerankInitOptions::new(RerankerModel::BGERerankerV2M3)
                    .with_show_download_progress(true);
                let inner = TextRerank::try_new(options)?;
                Ok(Self {
                    inner: Mutex::new(inner),
                    model_id: std::sync::Arc::from("BAAI/bge-reranker-v2-m3"),
                    top_n,
                })
            }
        }
    }

    /// Load mxbai-rerank-large-v1 (int8) through fastembed's BYO-ONNX seam.
    /// Requires `onnx/model_quantized.onnx` + the 4 tokenizer files in the model
    /// dir. Errors (rather than panics) if any file is missing — the caller falls
    /// back to bge-reranker-v2-m3. `_top_n` is reserved for a future per-call
    /// max_length cap; model_max_length (512) bounds the tokenizer today.
    fn new_mxbai_user_defined(_top_n: usize) -> anyhow::Result<TextRerank> {
        let dir = std::env::var("BRAIN_RERANK_MODEL_DIR")
            .unwrap_or_else(|_| DEFAULT_MXBAI_DIR.to_string());
        let model = UserDefinedRerankingModel::new(
            OnnxSource::File(
                std::path::PathBuf::from(&dir)
                    .join("onnx")
                    .join("model_quantized.onnx"),
            ),
            TokenizerFiles {
                tokenizer_file: std::fs::read(
                    std::path::PathBuf::from(&dir).join("tokenizer.json"),
                )
                .map_err(|e| anyhow::anyhow!("missing tokenizer.json in {dir}: {e}"))?,
                config_file: std::fs::read(std::path::PathBuf::from(&dir).join("config.json"))
                    .map_err(|e| anyhow::anyhow!("missing config.json in {dir}: {e}"))?,
                special_tokens_map_file: std::fs::read(
                    std::path::PathBuf::from(&dir).join("special_tokens_map.json"),
                )
                .map_err(|e| anyhow::anyhow!("missing special_tokens_map.json in {dir}: {e}"))?,
                tokenizer_config_file: std::fs::read(
                    std::path::PathBuf::from(&dir).join("tokenizer_config.json"),
                )
                .map_err(|e| anyhow::anyhow!("missing tokenizer_config.json in {dir}: {e}"))?,
            },
        );
        let options = RerankInitOptionsUserDefined::new();
        TextRerank::try_new_from_user_defined(model, options)
    }

    /// Rerank the fused candidates in place. Sets `rerank_score` on each
    /// survivor; sets `rerank_truncated = true` when the input exceeded
    /// `top_n` (so provenance honestly reports low-rank candidates were
    /// dropped before scoring).
    ///
    /// Fail-open: on any ONNX/lock error, leaves `fused` in its RRF order and
    /// returns `Ok(())` — never propagates a reranker fault to the recall path.
    /// The caller times the call and records `rerank_ms` into telemetry.
    pub fn rerank(&self, query: &str, fused: &mut [SearchResult]) -> anyhow::Result<()> {
        if fused.len() <= 1 {
            return Ok(()); // nothing to order
        }
        let truncated = fused.len() > self.top_n;
        let n = fused.len().min(self.top_n);

        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return Ok(()), // poisoned — fail open
        };
        // Score the top-n by RRF order. rerank() returns results sorted by score
        // descending; we re-apply that order to the slice tail-included.
        let docs: Vec<&str> = fused[..n].iter().map(|r| r.content.as_str()).collect();
        let results = match guard.rerank(query, &docs, false, None) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("reranker call failed (fail-open to RRF order): {e}");
                return Ok(());
            }
        };
        // `results` is score-descending with `index` into the top-n window.
        // Reorder the window by the reranker's ordering; carry the score + the
        // truncated flag. Pull the window out of the slice so we can rebuild it.
        let window: Vec<SearchResult> = fused[..n].to_vec();
        let mut reordered: Vec<SearchResult> = Vec::with_capacity(n);
        let mut placed = vec![false; window.len()];
        let mut top_score: Option<f32> = None;
        for r in &results {
            let idx = r.index;
            if idx < window.len() && !placed[idx] {
                placed[idx] = true;
                let mut item = window[idx].clone();
                item.provenance.rerank_score = Some(r.score);
                item.provenance.rerank_truncated = truncated;
                if top_score.is_none() {
                    top_score = Some(r.score);
                }
                reordered.push(item);
            }
        }
        // Any candidates the reranker didn't return (shouldn't happen) keep
        // their RRF order with no rerank score, so nothing is silently dropped.
        for (i, item) in window.iter().enumerate() {
            if !placed[i] {
                reordered.push(item.clone());
            }
        }
        // Splice the reordered top-n back; the un-reranked tail (n..) keeps its
        // RRF order, flagged truncated so the consumer sees they were dropped
        // from scoring.
        fused[..n].clone_from_slice(&reordered);
        if truncated {
            for r in &mut fused[n..] {
                r.provenance.rerank_truncated = true;
            }
        }
        // Record the new top-1 score on the (already-set) top result's
        // provenance. No extra telemetry field — the caller owns rerank_ms.
        let _ = top_score;
        Ok(())
    }
}
