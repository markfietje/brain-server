//! `POST /verify` — deterministic span verification.
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

use crate::AppState;
use crate::handlers::{HandlerError, MAX_QUERY};

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
    // AuthZ read gate, scoped to the requested domain.
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

    // resolve pool from X-Brain-Domain header (same path as /get/{id}).
    let pool = crate::handlers::resolve_domain_pool(&state.registry, domain.as_deref())
        .unwrap_or(state.pool.clone());
    // The read scope is the header-resolved domain
    // label, bound into the SQL exactly like /get/{id} so a chunk id can
    // never cross domains in shim mode (the pool is shared there). The
    // composite record gate + row-domain re-auth mirror /get too — /verify
    // answers "does this claim appear in chunk X", a cross-domain
    // content-confirmation oracle without them.
    let label = domain
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("global")
        .to_string();
    let record_gate = crate::handlers::gate::record_read_gate(&principal.0, &state.pool);
    let gate_principal = principal.0.clone();
    let chunk_id = req.chunk_id;
    let claim_for_task = claim.clone();

    let content = tokio::task::spawn_blocking(move || -> Result<Option<String>, HandlerError> {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        let r = crate::service::recall::chunk_for_verify(&conn, chunk_id, &label);
        match r {
            Ok(Some((content, row_domain, row_owner, row_scope))) => {
                // belt-and-braces: re-authorize on the row's OWN domain + the
                // record gate, so a predicate loosening can never turn /verify
                // into a cross-domain oracle. Miss (not leak) on denial.
                if !crate::handlers::can_read_domain(&gate_principal, &row_domain)
                    || !record_gate.admits(&row_owner, &row_scope)
                {
                    return Ok(None);
                }
                Ok(Some(content))
            }
            Ok(None) => Ok(None),
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
/// ## THE OFFSETS ARE INTO `content`, OR THEY ARE INTO NOTHING
///
/// Kept pure so the contract (case-insensitive, non-overlapping, bounded) can
/// be unit-tested without AppState or a database.
///
/// The obvious implementation lowercases both sides and reports offsets into
/// the LOWERCASED haystack — which is a different string. Rust's
/// `to_lowercase` is full Unicode and can change a character's byte length:
/// `İ` (U+0130) is 2 bytes and lowercases to `i` + U+0307 at 3 bytes, `ẞ`
/// (U+1E9E) is 3 bytes and lowercases to `ß` at 2. Every offset after such a
/// character is skewed, so for `"İstanbul"` (9 bytes) a match of `"stanbul"`
/// in the lowercased buffer at `[3, 10)` is **out of range for `content`** —
/// and a consumer doing the documented `&content[start..end]` panics.
///
/// (The skew was known and disclosed at
/// `crates/brain-evidence-core/src/lib.rs`, which warns against feeding these
/// offsets to its byte-exact resolver — but `openapi.yaml` publishes
/// "`[start, end)` byte offsets within chunk `content`" to every consumer, and
/// a wrong published contract is a bug whether or not one crate documented it.)
///
/// So the folded haystack is built WITH a per-byte map back to the original
/// char span. A match in the folded string maps to `[starts[s], ends[e-1])`:
/// the whole original characters whose folded bytes it covers. A partial-fold
/// match (folded `i` inside `İ`'s `i`+U+0307) therefore returns the whole
/// `İ`, never a mid-char offset, and an EXACT match of a length-changing fold
/// is reported as supported rather than silently omitted.
///
/// The bound is O(content + claim) plus the matches, over inputs already
/// bounded by `MAX_CONTENT` at ingest and `MAX_QUERY` on the claim.
pub fn verify_claim(content: &str, claim: &str) -> Vec<[usize; 2]> {
    if claim.is_empty() {
        return Vec::new();
    }
    let needle = claim.to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    // Build the case-folded haystack WITH a map from every folded byte to the
    // original char's span. Lowercasing can change a char's byte length
    // (`İ` 2→3, `ẞ` 3→2), so a folded offset is not a content offset; the map
    // is what makes the published `[start,end)` contract true for EVERY
    // occurrence (audit: the previous scan-forward locator reported an exact
    // `İstanbul`/`İstanbul` match as UNSUPPORTED).
    let mut hay = String::with_capacity(content.len());
    let mut starts: Vec<usize> = Vec::with_capacity(content.len());
    let mut ends: Vec<usize> = Vec::with_capacity(content.len());
    for (off, ch) in content.char_indices() {
        let end = off + ch.len_utf8();
        for lc in ch.to_lowercase() {
            for _ in 0..lc.len_utf8() {
                starts.push(off);
                ends.push(end);
            }
            hay.push(lc);
        }
    }
    if needle.len() > hay.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut from = 0usize;
    while out.len() < MAX_MATCH_RANGES {
        let Some(rel) = hay[from..].find(&needle) else {
            break;
        };
        let s = from + rel;
        let e = s + needle.len();
        // `e - 1` is the last folded byte of the match; `ends` maps it to the
        // end of the original char that produced it. A multi-byte fold that
        // matched only its first byte therefore still returns the whole
        // original char, never a mid-char offset.
        out.push([starts[s], ends[e - 1]]);
        // Non-overlapping in folded space; the standard matcher is linear per
        // call and the total is O(content + claim).
        from = e;
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

    /// EVERY RETURNED RANGE MUST INDEX `content`. This is the published
    /// contract (`openapi.yaml`: "`[start, end)` byte offsets within chunk
    /// `content`"), and a consumer doing the documented
    /// `&content[start..end]` panics when it is false.
    ///
    /// Red-first: restoring the direct push of `abs` (an offset into the
    /// LOWERCASED haystack) reds this on every character whose `to_lowercase`
    /// changes its byte length.
    #[test]
    fn verify_claim_ranges_always_index_content() {
        for prefix in [
            "",
            "İ",
            "ẞ",
            "İstanbul",
            "İİİ",
            "aİ",
            "ẞİx",
            "plain İstanbul tail",
            "İİ",
        ] {
            for claim in ["stanbul", "x", "tanbul", "a", "i", "ss", "İ", "İstanbul"] {
                let ranges = verify_claim(prefix, claim);
                assert!(
                    ranges.len() <= MAX_MATCH_RANGES,
                    "cap respected for {prefix:?}/{claim:?}"
                );
                // THE SUPPORT ARM: if the folded claim occurs in the
                // folded content, the matcher MUST report at least one range.
                // The previous test asserted only INSIDE the loop, so zero
                // ranges passed — the exact reason `İstanbul`/`İstanbul` was
                // reported unsupported while the pin stayed green.
                if !claim.is_empty() && prefix.to_lowercase().contains(&claim.to_lowercase()) {
                    assert!(
                        !ranges.is_empty(),
                        "content={prefix:?} claim={claim:?} folds to a match but returned no range"
                    );
                }
                let mut prev_end = 0usize;
                for r in &ranges {
                    assert!(
                        r[1] <= prefix.len(),
                        "content={prefix:?} ({}) claim={claim:?} range={r:?} is OUT OF RANGE",
                        prefix.len()
                    );
                    assert!(
                        prefix.is_char_boundary(r[0]) && prefix.is_char_boundary(r[1]),
                        "content={prefix:?} claim={claim:?} range={r:?} splits a character"
                    );
                    assert!(r[0] >= prev_end, "ranges must stay non-overlapping");
                    prev_end = r[1];
                    // The documented slice must work, and the folded claim must
                    // be COVERED by it (a partial-fold match may include the
                    // whole original char, e.g. `İ` for the folded `i`).
                    let sliced = &prefix[r[0]..r[1]];
                    assert!(
                        sliced.to_lowercase().contains(&claim.to_lowercase()),
                        "content={prefix:?} claim={claim:?} range={r:?} slices to {sliced:?}"
                    );
                }
            }
        }
        // The exact match the audit found reported UNSUPPORTED.
        let c = "İstanbul";
        let exact = verify_claim(c, c);
        assert_eq!(exact.len(), 1, "an exact İstanbul match must be supported");
        assert_eq!(&c[exact[0][0]..exact[0][1]], c);
        assert_eq!(verify_claim("İ", "İ"), vec![[0, 2]]);
        // The specific 9-byte suffix case stays safe.
        let ranges = verify_claim(c, "stanbul");
        assert_eq!(&c[ranges[0][0]..ranges[0][1]], "stanbul");
    }
}
