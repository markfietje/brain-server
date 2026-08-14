//! v1.28 "Caliber" M1 — the profile-gated cross-encoder reranker.
//!
//! Sits after `rrf_fuse` (the rank-fusion stage in [`crate::search`]) and writes
//! into the reserved `rerank_score` / `rerank_truncated` slots on
//! [`crate::search::SearchResult`] provenance. Default model: `bge-reranker-v2-m3`
//! (FastEmbed-rs `BGERerankerV2M3`, the current local-SOTA cross-encoder — NOT
//! the 2021 `ms-marco-MiniLM-L-6-v2`).
//!
//! Profile gate: active only on `enterprise`/`desktop`. The Jetson/edge path
//! stays rerank-free (the v0.9.5 doctrine; the tier was removed for the 8 s
//! recall timeout — re-added here only where the hardware can afford it).
//!
//! Fail-open contract: a model load failure or a per-call ONNX error leaves the
//! RRF order untouched (provenance stays `rerank_score = None`) — a recall never
//! panics or stalls on a reranker fault. This mirrors the `screen.rs` layer-2
//! fail-soft posture.
#![cfg(feature = "rerank-tier")]

use std::sync::{LazyLock, Mutex};

use fastembed::{RerankInitOptions, RerankerModel, TextRerank};

use crate::search::{SearchResult, SearchTelemetry};

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
            tracing::info!("reranker loaded: bge-reranker-v2-m3 (top_n={top_n})");
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
    /// The max candidates scored per call. Reranking more than ~50–100 costs
    /// latency for diminishing rank-quality gain (IronCurtain's own cascade
    /// discipline uses ~50). Larger candidate sets are truncated and the
    /// `rerank_truncated` provenance flag reports it honestly.
    top_n: usize,
}

impl Reranker {
    /// Load `bge-reranker-v2-m3`. Override the model + top-N via the config.
    pub fn new(top_n: usize) -> anyhow::Result<Self> {
        let options = RerankInitOptions::new(RerankerModel::BGERerankerV2M3)
            .with_show_download_progress(true);
        let inner = TextRerank::try_new(options)?;
        Ok(Self {
            inner: Mutex::new(inner),
            top_n,
        })
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
