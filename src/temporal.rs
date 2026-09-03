//! Deterministic temporal-marker extraction for bi-temporal edges.
//!
//! Graphiti's bi-temporal model (Context7-verified 2026-07-30 against
//! getzep/graphiti:edges.py) attaches two *valid-time* timestamps to every edge:
//! `valid_at` (when the fact became true in the world) and `invalid_at` (when it
//! stopped being true). These are distinct from `created_at` (transaction time:
//! when brain-server *learned* the fact).
//!
//! This module extracts those timestamps from free text deterministically — no
//! LLM, no external API call. It recognizes a bounded set of English temporal
//! markers and emits ISO-8601 (`YYYY-MM-DD HH:MM:SS`) strings, the same format
//! `normalize_since` produces and SQLite compares lexicographically.
//!
//! What it finds:
//!   - Absolute dates: "in 2015", "as of 2024-03", "since January 2020",
//!     "until 2017", "from 2011 to 2017".
//!   - Currency markers: "currently", "as of now", "now" → valid_at = now,
//!     invalid_at = NULL.
//!   - Past-tense invalidation: "was previously", "formerly", "used to be",
//!     "until <date>" → invalid_at populated.
//!
//! What it does NOT find (deliberately — out of scope, would need an LLM):
//!   - Relative dates without anchors ("last year", "recently").
//!   - Inferred durations ("for three years" with no start).
//!   - Negation/conditionals.
//!
//! The output is best-effort: callers use it to populate `valid_at`/`invalid_at`
//! on edges at ingest time; a `None` means "no marker found, leave NULL ⇒ always
//! valid", which is the safe default (existing behavior).

#![deny(unsafe_code)]

use chrono::{NaiveDate, NaiveDateTime, Utc};

/// Result of temporal extraction: the [valid_at, invalid_at) interval in
/// ISO-8601 form. Either or both may be `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemporalInterval {
    pub valid_at: Option<String>,
    pub invalid_at: Option<String>,
}

/// Words/phrases that mark the fact as currently true (valid_at = now).
/// Matched case-insensitively as whole-word boundaries.
const CURRENCY_MARKERS: &[&str] = &[
    "currently",
    "as of now",
    "right now",
    "at present",
    "these days",
    "nowadays",
    "today",
];

/// Words/phrases that mark the fact as no longer true (invalidate it).
/// "was previously X", "formerly X", "X until 2017", "used to be X".
const PAST_MARKERS: &[&str] = &[
    "was previously",
    "formerly",
    "used to be",
    "previously",
    "once was",
    "in the past",
    "no longer",
    "former",
];

/// Find the first 4-digit year (1000–9999) in the text, optionally followed
/// by `-MM` or `-MM-DD`. Scans byte-aligned ASCII boundaries only (years are
/// ASCII, so byte offsets are valid char boundaries). Returns the parsed tuple.
fn find_year(text: &str) -> Option<(i32, u32, Option<u32>)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let prev_ok = i == 0 || !is_word_byte(bytes[i - 1]);
        if prev_ok && bytes[i].is_ascii_digit() {
            let yr: [u8; 4] = [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]];
            if yr.iter().all(|b| b.is_ascii_digit()) {
                let after = i + 4;
                let next_ok = after == bytes.len() || !is_word_byte(bytes[after]);
                if next_ok
                    && let Ok(year) = std::str::from_utf8(&yr).unwrap_or("0").parse::<i32>()
                    && (1000..=9999).contains(&year)
                {
                    let (month, day, _) = parse_month_day(text, after);
                    return Some((year, month, day));
                }
            }
        }
        i += 1;
    }
    None
}

/// Parse an optional `-MM` or `-MM-DD` suffix starting at byte `pos`. Returns
/// the month (default 1), optional day, and bytes consumed.
fn parse_month_day(text: &str, pos: usize) -> (u32, Option<u32>, usize) {
    let bytes = text.as_bytes();
    if pos + 3 > bytes.len() || bytes[pos] != b'-' {
        return (1, None, 0);
    }
    if bytes[pos + 1].is_ascii_digit() && bytes[pos + 2].is_ascii_digit() {
        let mm = (bytes[pos + 1] - b'0') as u32 * 10 + (bytes[pos + 2] - b'0') as u32;
        if (1..=12).contains(&mm) {
            let day_pos = pos + 3;
            if day_pos + 3 <= bytes.len()
                && bytes[day_pos] == b'-'
                && bytes[day_pos + 1].is_ascii_digit()
                && bytes[day_pos + 2].is_ascii_digit()
            {
                let dd =
                    (bytes[day_pos + 1] - b'0') as u32 * 10 + (bytes[day_pos + 2] - b'0') as u32;
                if (1..=31).contains(&dd) {
                    return (mm, Some(dd), 6);
                }
            }
            return (mm, None, 3);
        }
    }
    (1, None, 0)
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Render a (year, month, optional day) as the canonical SQLite-comparable
/// `YYYY-MM-DD HH:MM:SS` string (time = 00:00:00).
fn iso_from_ymd(year: i32, month: u32, day: Option<u32>) -> Option<String> {
    let d = NaiveDate::from_ymd_opt(year, month, day.unwrap_or(1))?;
    Some(
        d.and_hms_opt(0, 0, 0)?
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
    )
}

/// Lowercase the text once for marker matching, keeping the original for
/// offset math against the year scanner. ASCII-only lowering: every marker
/// and year is ASCII, and `str::to_lowercase` (full Unicode) can CHANGE the
/// byte length (e.g. `İ` → `i̇`), which would desynchronize offsets between
/// the lowered copy and the original and panic on non-boundary slicing.
fn contains_marker(lower: &str, markers: &[&str]) -> bool {
    markers.iter().any(|m| lower.contains(m))
}

/// Extract a bi-temporal interval from free text. Pure: same input ⇒ same
/// output, no I/O, no clock dependency for absolute dates. Currency markers
/// ("currently") resolve to `now` at call time (UTC), which is the honest
/// semantics — "currently" means "as of when this was written".
///
/// `now_utc` is injected so tests are deterministic.
pub fn extract_interval(text: &str, now_utc: &NaiveDateTime) -> TemporalInterval {
    let lower = text.to_ascii_lowercase();
    let year = find_year(text);

    // "from X to Y" / "between X and Y" — two dates → [start, end).
    if let Some((ys, ms, ds)) = find_range_start(&lower, text)
        && let Some((ye, me, de)) = find_range_end(&lower, text)
        && let (Some(valid), Some(invalid)) = (iso_from_ymd(ys, ms, ds), iso_from_ymd(ye, me, de))
    {
        return TemporalInterval {
            valid_at: Some(valid),
            invalid_at: Some(invalid),
        };
    }

    let mut interval = TemporalInterval::default();

    // "until <year>" / "<year> until <year>" → invalid_at.
    if contains_marker(&lower, &["until", "through ", "ending "])
        && let Some((y, m, d)) = year
    {
        interval.invalid_at = iso_from_ymd(y, m, d);
    }
    // "since <year>" / "from <year>" / "as of <year>" / "in <year>" → valid_at.
    if contains_marker(
        &lower,
        &["since", "from", "as of", "in ", "starting", "began"],
    ) && let Some((y, m, d)) = year
        && interval.valid_at.is_none()
    {
        interval.valid_at = iso_from_ymd(y, m, d);
    }

    // Bare year with no marker + a currency marker → currently true since that year.
    if interval.valid_at.is_none()
        && interval.invalid_at.is_none()
        && let Some((y, m, d)) = year
        && contains_marker(&lower, CURRENCY_MARKERS)
    {
        interval.valid_at = iso_from_ymd(y, m, d);
    }

    // "currently" / "now" with NO year → valid_at = now.
    if interval.valid_at.is_none() && contains_marker(&lower, CURRENCY_MARKERS) {
        interval.valid_at = Some(now_utc.format("%Y-%m-%d %H:%M:%S").to_string());
    }

    // Past invalidation markers with a year → invalid_at = that year (already
    // handled by the "until" branch for "until <year>"). Pure past markers
    // ("formerly", "was previously") without a year → leave invalid_at None;
    // we cannot place the boundary deterministically, so we don't guess.
    if interval.invalid_at.is_none()
        && interval.valid_at.is_none()
        && contains_marker(&lower, PAST_MARKERS)
        && year.is_none()
    {
        // ponytail: "formerly X" with no date — we know it's no longer current
        // but can't bound the interval. Mark invalid_at = now so a "?at=now"
        // query skips it, without fabricating a start. Honest degradation.
        interval.invalid_at = Some(now_utc.format("%Y-%m-%d %H:%M:%S").to_string());
    }

    interval
}

/// Find the start year in a "from YYYY" / "between YYYY" / "since YYYY" range.
fn find_range_start(lower: &str, orig: &str) -> Option<(i32, u32, Option<u32>)> {
    for kw in &["from ", "between ", "since "] {
        if let Some(pos) = lower.find(kw) {
            let after = &orig[pos + kw.len()..];
            if let Some(y) = year_at_start(after) {
                return Some(y);
            }
        }
    }
    None
}

/// Find the end year in a "to YYYY" / "and YYYY" / "until YYYY" range.
fn find_range_end(lower: &str, orig: &str) -> Option<(i32, u32, Option<u32>)> {
    for kw in &["to ", "and ", "until ", "- "] {
        if let Some(pos) = lower.find(kw) {
            let after = &orig[pos + kw.len()..];
            if let Some(y) = year_at_start(after) {
                return Some(y);
            }
        }
    }
    None
}

/// If `s` starts with (optional spaces then) a 4-digit year, parse it.
fn year_at_start(s: &str) -> Option<(i32, u32, Option<u32>)> {
    let t = s.trim_start();
    let bytes = t.as_bytes();
    if bytes.len() < 4 || !bytes[..4].iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year = std::str::from_utf8(&bytes[..4]).ok()?.parse::<i32>().ok()?;
    if !(1000..=9999).contains(&year) {
        return None;
    }
    let (month, day, _) = parse_month_day(t, 4);
    Some((year, month, day))
}

/// Convenience wrapper: extract with the real current UTC time. Used by the
/// ingest path; tests call [`extract_interval`] with a fixed clock.
pub fn extract_interval_now(text: &str) -> TemporalInterval {
    extract_interval(text, &Utc::now().naive_utc())
}

/// SQL WHERE-clause fragment for the bi-temporal "as of" filter, plus the
/// bound parameter. Appends to an existing WHERE chain. Returns the SQL
/// fragment (caller binds `at` as the single parameter, twice).
///
/// Semantics (Graphiti-validity): an edge is visible at instant `at` iff
///   valid_at IS NULL OR valid_at <= at   (became true at or before `at`)
///   AND (invalid_at IS NULL OR invalid_at > at)  (still true at `at`)
///
/// NULL valid_at means "origin unknown" ⇒ treated as always-valid (visible),
/// matching the additive-migration default for edges created before the
/// bi-temporal columns existed.
///
/// ponytail: the search/retrieve paths inline their own valid-interval filter
/// (chunk-level valid_from/valid_to). This edge-level constant is the reference
/// fragment for the `/graph/traverse` path and future graph queries; kept here
/// so the semantics live in one documented place.
#[allow(dead_code)]
pub const AT_FILTER_SQL: &str =
    " AND (valid_at IS NULL OR valid_at <= ?) AND (invalid_at IS NULL OR invalid_at > ?)";

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn fixed_now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 7, 30)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
    }

    #[test]
    fn extracts_from_to_range() {
        let i = extract_interval("Kamala Harris was CA AG from 2011 to 2017", &fixed_now());
        assert_eq!(i.valid_at.as_deref(), Some("2011-01-01 00:00:00"));
        assert_eq!(i.invalid_at.as_deref(), Some("2017-01-01 00:00:00"));
    }

    #[test]
    fn extracts_since_year_as_valid_at() {
        let i = extract_interval("lives in Berlin since 2020", &fixed_now());
        assert_eq!(i.valid_at.as_deref(), Some("2020-01-01 00:00:00"));
        assert_eq!(i.invalid_at, None);
    }

    #[test]
    fn extracts_until_year_as_invalid_at() {
        let i = extract_interval("worked at Acme until 2019", &fixed_now());
        assert_eq!(i.valid_at, None);
        assert_eq!(i.invalid_at.as_deref(), Some("2019-01-01 00:00:00"));
    }

    #[test]
    fn currently_with_year_sets_valid_at() {
        let i = extract_interval("as of 2024 the CEO is Alice", &fixed_now());
        assert_eq!(i.valid_at.as_deref(), Some("2024-01-01 00:00:00"));
        assert_eq!(i.invalid_at, None);
    }

    #[test]
    fn currently_without_year_uses_now() {
        let i = extract_interval("currently lives in Tokyo", &fixed_now());
        assert_eq!(i.valid_at.as_deref(), Some("2026-07-30 12:00:00"));
        assert_eq!(i.invalid_at, None);
    }

    #[test]
    fn formerly_without_date_marks_invalid_at_now() {
        // Can't bound the interval; honest degradation marks it no-longer-current.
        let i = extract_interval("was formerly the mayor", &fixed_now());
        assert_eq!(i.valid_at, None);
        assert_eq!(i.invalid_at.as_deref(), Some("2026-07-30 12:00:00"));
    }

    #[test]
    fn no_markers_returns_empty() {
        let i = extract_interval("The Eiffel Tower is in Paris", &fixed_now());
        assert_eq!(i, TemporalInterval::default());
    }

    #[test]
    fn handles_month_precision() {
        let i = extract_interval("from 2020-03 to 2021-06", &fixed_now());
        assert_eq!(i.valid_at.as_deref(), Some("2020-03-01 00:00:00"));
        assert_eq!(i.invalid_at.as_deref(), Some("2021-06-01 00:00:00"));
    }

    #[test]
    fn rejects_non_year_four_digits() {
        // "1234" is a valid year; "99999" is not. Ensure we don't grab "1234567".
        let i = extract_interval("order number 99999 was placed", &fixed_now());
        // 9999 would parse but "99999" should not be a lone-year match.
        assert!(i.valid_at.is_none() || i.valid_at.as_deref() != Some("9999-01-01 00:00:00"));
    }

    #[test]
    fn at_filter_sql_is_parameterized() {
        // The fragment must use ? placeholders, never interpolation.
        assert!(AT_FILTER_SQL.contains("?"));
        assert!(!AT_FILTER_SQL.contains("{"));
    }

    #[test]
    fn determinism_same_input_same_output() {
        let a = extract_interval("from 2011 to 2017", &fixed_now());
        let b = extract_interval("from 2011 to 2017", &fixed_now());
        assert_eq!(a, b);
    }

    /// Regression: full-Unicode `to_lowercase` changes byte lengths
    /// (`İ` U+0130 lowercases to a two-char sequence), so keyword offsets found
    /// in the lowered copy panicked when used to slice the original. ASCII-only
    /// lowering keeps every offset aligned; the range must still parse.
    #[test]
    fn unicode_length_changing_input_does_not_panic_and_still_parses() {
        let hostile = format!("{} from 2011 to 2017", "İ".repeat(20));
        let i = extract_interval(&hostile, &fixed_now());
        assert_eq!(i.valid_at.as_deref(), Some("2011-01-01 00:00:00"));
        assert_eq!(i.invalid_at.as_deref(), Some("2017-01-01 00:00:00"));
    }

    /// the deterministic extractor pulls valid_at/invalid_at from
    /// free text. "from 2011 to 2017" → [2011, 2017).
    /// (Relocated verbatim from main.rs's tests block, Spire v1.28.54 —
    /// the pin travels with its subject, `extract_interval`.)
    #[test]
    fn temporal_extractor_populates_edge_interval() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 7, 30)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        let iv = extract_interval("was CA AG from 2011 to 2017", &now);
        assert_eq!(iv.valid_at.as_deref(), Some("2011-01-01 00:00:00"));
        assert_eq!(iv.invalid_at.as_deref(), Some("2017-01-01 00:00:00"));
    }
}
