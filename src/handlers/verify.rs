//! `POST /verify` — deterministic span verification (v1.5.0 "Epistemic" M5).
//!
//! Given a `claim` and a `chunk_id`, returns whether the claim is supported by
//! the chunk's actual text via deterministic case-insensitive substring match.
//! No embeddings, no LLM, no model load — O(content.len()) over one chunk.
//!
//! This is the hallucination-resistance primitive: an agent that recalled a
//! fact can verify "the brain said X" against the original source before
//! acting on it. Mismatch surfaces as `unsupported_claim`.
//!
//! Reference: arXiv:2607.00895 (span-level hallucination detection). The paper
//! proposes model-based span detection; this implementation is the deterministic
//! lexical baseline the evidence-gated roadmap sanctions for v1.5 (no neural
//! net in the hot path).

use axum::extract::State;
use axum::response::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::handlers::auth::OptPrincipal;

use crate::handlers::{HandlerError, MAX_QUERY};
use crate::AppState;

/// Cap on collected match ranges. A claim that matches 10k positions in a
/// 1 MiB chunk is not more informative than one that matches 100; capping
/// keeps the response bounded without losing the "supported" signal.
const MAX_MATCH_RANGES: usize = 100;

#[derive(Debug, Deserialize)]
pub struct VerifyRequest {
    pub chunk_id: i64,
    /// The claim to verify. Whitespace-trimmed before matching. Bounded by
    /// [`MAX_QUERY`] (2_000 chars) — the same limit `/recall` enforces.
    pub claim: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyResponse {
    pub chunk_id: i64,
    /// `true` when at least one match range was found.
    pub supported: bool,
    /// `supported` ⇒ `"supported"`; else `"unsupported_claim"`. Stable
    /// machine-readable code for agent escalation logic.
    pub decision: &'static str,
    /// `[start, end)` byte offsets within the chunk `content`. Capped at
    /// [`MAX_MATCH_RANGES`]; further matches are not reported but do not
    /// change `supported`.
    pub match_ranges: Vec<[usize; 2]>,
}

/// `POST /verify`
pub async fn verify(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    headers: axum::http::HeaderMap,
    Json(req): Json<VerifyRequest>,
) -> Result<Json<VerifyResponse>, HandlerError> {
    // v1.12.1 "Harden": AuthZ read gate, scoped to the requested domain.
    // `None` (no JWT) = superuser.
    let domain = crate::handlers::domain_from_headers(&headers);
    super::authorize(
        &principal.0,
        crate::auth::Action::Read,
        "",
        domain.as_deref().unwrap_or("global"),
    )?;
    let claim = req.claim.trim().to_string();
    if claim.is_empty() {
        return Err(HandlerError::bad_request(
            "claim_empty",
            "claim must be non-empty",
        ));
    }
    if claim.chars().count() > MAX_QUERY {
        return Err(HandlerError::bad_request(
            "claim_too_long",
            format!("claim exceeds {MAX_QUERY} chars"),
        ));
    }

    // v1.0.0: resolve pool from X-Brain-Domain header (same path as /get/{id}).
    let pool = crate::handlers::resolve_domain_pool(&state.registry, domain.as_deref())
        .unwrap_or(state.pool.clone());
    let chunk_id = req.chunk_id;
    let claim_for_task = claim.clone();

    let content = tokio::task::spawn_blocking(move || -> Result<Option<String>, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let r = conn.query_row(
            "SELECT content FROM knowledge WHERE id = ?1",
            rusqlite::params![chunk_id],
            |row| row.get::<_, String>(0),
        );
        match r {
            Ok(c) => Ok(Some(c)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(HandlerError::internal(format!("query failed: {e}"))),
        }
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    let Some(content) = content else {
        return Err(HandlerError::not_found(format!(
            "no chunk with id {}",
            req.chunk_id
        )));
    };

    let ranges = verify_claim(&content, &claim_for_task);
    let supported = !ranges.is_empty();
    Ok(Json(VerifyResponse {
        chunk_id: req.chunk_id,
        supported,
        decision: if supported {
            "supported"
        } else {
            "unsupported_claim"
        },
        match_ranges: ranges,
    }))
}

/// Pure deterministic matcher: returns non-overlapping `[start, end)` byte
/// offsets of case-insensitive occurrences of `claim` within `content`.
/// Capped at [`MAX_MATCH_RANGES`]. Empty `claim` returns an empty vec.
///
/// Kept pure so the contract (case-insensitive, non-overlapping, bounded) can
/// be unit-tested without AppState or a database.
pub fn verify_claim(content: &str, claim: &str) -> Vec<[usize; 2]> {
    if claim.is_empty() {
        return Vec::new();
    }
    // ponytail: `str::match_indices` operates on bytes and is the stdlib
    // primitive for this — no regex dep needed. Case-folding both sides
    // allocates two lowercased Strings; bounded by MAX_QUERY on the claim
    // side and MAX_CONTENT on the content side (enforced at ingest), so this
    // is bounded work.
    let hay = content.to_lowercase();
    let needle = claim.to_lowercase();
    let nlen = needle.len();
    if nlen > hay.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    while start + nlen <= hay.len() {
        if let Some(rel) = hay[start..].find(&needle) {
            let abs = start + rel;
            out.push([abs, abs + nlen]);
            if out.len() >= MAX_MATCH_RANGES {
                break;
            }
            // Non-overlapping: advance past this match. Overlapping matches
            // add no information for support detection.
            start = abs + nlen;
        } else {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_claim_finds_case_insensitive_matches() {
        let ranges = verify_claim("Foo bar FOO baz foo", "foo");
        // Three non-overlapping matches at byte offsets 0, 8, 16.
        assert_eq!(ranges, vec![[0, 3], [8, 11], [16, 19]]);
    }

    #[test]
    fn verify_claim_returns_byte_offsets_into_original_content() {
        // Byte offsets must index into the original `content`, not the
        // lowercased haystack — they're identical for ASCII but the contract
        // is "offsets into content". This pins it for ASCII.
        let content = "Rust is fast. Rust is safe.";
        let ranges = verify_claim(content, "rust");
        assert_eq!(ranges, vec![[0, 4], [14, 18]]);
        // Round-trip: slicing content at the range yields the match (modulo case).
        assert_eq!(&content[ranges[0][0]..ranges[0][1]], "Rust");
    }

    #[test]
    fn verify_claim_is_non_overlapping() {
        // "aaa" in "aaaaa" yields matches at 0 and 3, not 0/1/2/3/...
        let ranges = verify_claim("aaaaa", "aa");
        assert_eq!(ranges, vec![[0, 2], [2, 4]]);
    }

    #[test]
    fn verify_claim_empty_claim_returns_empty() {
        assert!(verify_claim("anything", "").is_empty());
    }

    #[test]
    fn verify_claim_no_match_returns_empty() {
        assert!(verify_claim("hello world", "missing").is_empty());
    }

    #[test]
    fn verify_claim_capped_at_max_ranges() {
        // 1000 matches but only MAX_MATCH_RANGES reported.
        let content = "x".repeat(1000);
        let ranges = verify_claim(&content, "x");
        assert_eq!(ranges.len(), MAX_MATCH_RANGES);
    }

    #[test]
    fn verify_claim_unicode_preserves_byte_boundaries() {
        // Multibyte: offsets are BYTE offsets into content. "É" is 2 bytes
        // in UTF-8 (0xC3 0x89); its lowercase "é" is also 2 bytes. The match
        // must land on a valid char boundary either way.
        let content = "Café Café";
        let ranges = verify_claim(content, "café");
        // Two matches; verify each range is a valid UTF-8 window.
        assert_eq!(ranges.len(), 2);
        for r in &ranges {
            let _ = content[r[0]..r[1]].to_string(); // panics if not char-aligned
        }
    }
}
