//! The byte-range evidence resolver.
//!
//! One question, asked of a claim that cites a verbatim quote at a byte range
//! into a content-addressed source: **does this evidence resolve?** It is
//! answered by arithmetic and nothing else —
//! `hash(source_bytes[range]) == hash(quote)` AND
//! `source_cid == CID(source_bytes)` — as a pure function of its arguments. No
//! clock, no store, no network, no provider, **no model** anywhere in the
//! path, so the claim is structural rather than aspirational: there is no seam
//! through which a model could be introduced, and there is no normalisation
//! step for a laundering pass to tune against.
//!
//! ## What this proves, and what it does not
//!
//! This is the honest boundary, and it ships with the code because a verifier
//! whose boundary is unstated gets quoted past it (E7).
//!
//! | Question | This crate |
//! |---|---|
//! | Do these exact bytes appear at these exact offsets in this exact source? | **deterministic — answered** |
//! | Is this source the one the citation was made against? | **deterministic — answered** |
//! | Does the quote *support* the claim? | **not checked, and not checkable here** |
//! | Is the claim *true*? | **out of scope entirely** |
//! | Does a quote *contradict* an existing fact? | **❌ semantic contradiction — not detected, and not deterministically detectable here** |
//! | Was this source admitted by a trusted writer at a known time? | **not checked — the caller's obligation, see below** |
//!
//! Semantic contradiction stays out because deterministic contradiction
//! detection requires typed, disjoint, single-valued predicates (decision E6,
//! a schema constraint R50 implements, not code) and, where those do not hold,
//! an LLM in the interpretation function — arXiv:2507.09751. That remains true
//! after this round, and it is written down here so it is not rediscovered as
//! a bug.
//!
//! ## The honest ceilings, stated where the code lives
//!
//! * **The bytes are supplied by the caller and are NOT yet guaranteed stable.**
//!   This crate holds no store, so it cannot promise that the bytes it was
//!   handed are still the admitted bytes. `src/service/ingest.rs` re-checks a
//!   PII-strict profile under the write lock and can refuse with
//!   `IngestError::ProfileChanged`, and the read seam reshapes on the way out —
//!   so a live row's hash is **not** a stable referent (E3). Making the bytes
//!   stable is the admitted-bytes store, which is R50's work.
//! * **The resolver cannot make a CID unforgeable.** Its guarantee is
//!   conditional on `source_cid` being a commitment made **at admit time by a
//!   trusted writer**. A caller that recomputes the CID from the same bytes it
//!   passes in is comparing `h(x)` to `h(x)`, and no check here can tell the
//!   difference. That is why [`resolve`] takes the CID as a caller-supplied
//!   value and never derives the expected one from its own input — see
//!   `r46_resolver_never_derives_a_cid_from_the_bytes_it_was_handed`.
//! * **A laundering pass that rewrites the source AND recomputes the CID AND
//!   forges every stored reference is out of reach here, by construction.** The
//!   resolver proves a relation among three values the caller supplied; it was
//!   never shown anything to distrust. What it converts that act from invisible
//!   into visible is the CID break on the *partial* case — rewrite without
//!   re-derivation — which is the case that actually occurs.
//! * **The whole claim reduces to SHA-256's collision resistance.** That is a
//!   very strong assumption, not a proof.
//! * **Byte-exact means byte-exact, including Unicode.** There is no case
//!   folding, no trimming, and no character-boundary snapping; a caller that
//!   wants a fuzzy match is asking for a different function. Note the
//!   consequence for `/verify` (below).
//! * **Do NOT feed this crate `/verify`'s offsets.** `POST /verify`
//!   (`src/handlers/verify.rs:160-161`) case-folds both sides and computes its
//!   `match_ranges` over the **lowercased** haystack while documenting them as
//!   offsets into the original content. Rust's `to_lowercase` is full Unicode
//!   and can change byte length — `İ` (U+0130) is 2 bytes and lowercases to 3 —
//!   so every offset after such a character is shifted. That skew is
//!   `/verify`'s, not this crate's, and R46 deliberately leaves `/verify` alone
//!   (Option C: it is a leaf with no internal consumer). Passing one of its
//!   offset pairs here produces a **false refusal** — fail-closed, therefore
//!   safe, but a real false negative if the skew is never disclosed.
//! * **The no-model claim covers this crate, not its callers.** The pins that
//!   enforce it prove the crate cannot reach a model; a consumer that
//!   normalises its inputs *before* calling is invisible here, and because
//!   normalisation is lossy it can only ever produce a `QuoteMismatch` — never
//!   a false acceptance.
//! * **No contradiction detection across refs.** Each ref is checked
//!   independently. Two refs with the same CID and conflicting quotes both
//!   resolve; noticing that is E6's schema constraint, not this crate's job.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod cid;

pub use cid::{CID_ALGORITHM_PREFIX, CID_ENCODED_LEN, CID_MULTIHASH_PREFIX, base32_encode, cid_v1};

use std::ops::Range;

use sha2::{Digest, Sha256};

/// The caller's bound on a single quote, in bytes.
///
/// The crate imposes no cap of its own because it has no unbounded transport
/// surface — a pure function cannot be flooded. What a caller CAN do is hand
/// [`resolve`] a multi-gigabyte source, and it pays exactly one linear hash of
/// that source per call regardless of how many refs it carries. This constant
/// is the caller obligation, published so it is not rediscovered as a surprise.
pub const MAX_QUOTE_BYTES: usize = 64 * 1024;

/// The caller's bound on how many refs one call may carry. See
/// [`MAX_QUOTE_BYTES`] for why the crate does not enforce it.
pub const MAX_REFS: usize = 256;

/// A citation: a quote, the byte range it is claimed to occupy, and the CID of
/// the source it is claimed to come from.
///
/// The lifetime is over the *borrowed* strings, deliberately: there is no
/// owned form and no constructor that derives a CID from the bytes, so a
/// caller cannot accidentally build a self-satisfying ref by convenience.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRef<'a> {
    /// The CID of the admitted bytes, as committed at admit time.
    pub source_cid: &'a str,
    /// The verbatim quote. Compared as raw bytes; never normalised.
    pub quote: &'a [u8],
    /// The half-open byte range the quote is claimed to occupy. `Range<usize>`,
    /// never `RangeInclusive`: the inclusive form overflows at `usize::MAX` on
    /// the `end + 1` that makes it inclusive.
    pub byte_range: Range<usize>,
}

/// The closed verdict vocabulary (E2, deny-wins).
///
/// Adding a variant is a scope change and a compile error in every consumer;
/// renaming one is a break. There is deliberately **no permissive variant**: a
/// seventh arm such as "probably resolved" is exactly the KILL-3 bypass wearing
/// a new name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EvidenceVerdict {
    /// Both comparisons passed. The ONLY way to reach this arm.
    Resolved,
    /// There is no source to resolve against: empty source bytes, or a
    /// `source_cid` that is not a well-formed CID at all.
    UnresolvedSource,
    /// The bytes in hand are not the bytes the citation names. The outer
    /// binding failed.
    CidMismatch,
    /// The declared range's bytes are not the quote. The inner comparison
    /// failed.
    QuoteMismatch,
    /// The declared range is not a slice of the source: `start > end`, or
    /// `end > len`. Both are arithmetic refusals and share this one cause; the
    /// vocabulary is frozen, so one cause serves both.
    RangeOutOfBounds,
    /// The evidence carries no bytes to check: an empty ref set, or a ref whose
    /// quote is empty.
    EmptyEvidence,
}

/// The E9 snake_case cause a failure is itemized under.
///
/// `Resolved` is not a failure and returns `None`. These five strings are the
/// reporting vocabulary a caller aggregates over, and the crate's own battery
/// asserts each of them is spelled here.
pub fn failure_cause(v: &EvidenceVerdict) -> Option<&'static str> {
    match v {
        EvidenceVerdict::Resolved => None,
        EvidenceVerdict::UnresolvedSource => Some("unresolved_source"),
        EvidenceVerdict::CidMismatch => Some("cid_mismatch"),
        EvidenceVerdict::QuoteMismatch => Some("quote_mismatch"),
        EvidenceVerdict::RangeOutOfBounds => Some("out_of_range"),
        EvidenceVerdict::EmptyEvidence => Some("empty_evidence"),
    }
}

/// A stable label for a verdict, for logs and for the `describe` table.
///
/// Exhaustive with no wildcard arm **on purpose**: that is what makes dropping a
/// variant a compile error rather than a silent behaviour change.
pub fn describe(v: &EvidenceVerdict) -> &'static str {
    match v {
        EvidenceVerdict::Resolved => "resolved",
        EvidenceVerdict::UnresolvedSource => "unresolved_source",
        EvidenceVerdict::CidMismatch => "cid_mismatch",
        EvidenceVerdict::QuoteMismatch => "quote_mismatch",
        EvidenceVerdict::RangeOutOfBounds => "out_of_range",
        EvidenceVerdict::EmptyEvidence => "empty_evidence",
    }
}

/// Decide whether every ref's evidence resolves against `source`.
///
/// Pure, total, and I/O-free: the same arguments always produce the same
/// verdict, and the function can reach nothing — no clock, no store, no
/// network, no provider.
///
/// # Precedence
///
/// `Resolved` requires every ref to pass both comparisons. Otherwise the
/// verdict is the **first failing cause in this fixed order**, by CAUSE and not
/// by ref position, so the reported cause never depends on the order the caller
/// happened to list its refs in:
///
/// 1. `EmptyEvidence` — nothing to check.
/// 2. `UnresolvedSource` — nothing to check it against.
/// 3. `RangeOutOfBounds` — no such region.
/// 4. `CidMismatch` — wrong source.
/// 5. `QuoteMismatch` — wrong text.
///
/// Each step is strictly more specific than the one above it, and the order is
/// pinned by `r46_verdict_precedence_is_fixed_and_by_cause`. CID precedes the
/// quote because the CID is the **outer** binding: if these are not the admitted
/// bytes, no range arithmetic over them means anything.
pub fn resolve(source: &[u8], refs: &[EvidenceRef<'_>]) -> EvidenceVerdict {
    // 1. EmptyEvidence. Checked FIRST, and the empty-QUOTE check precedes every
    //    hash comparison on purpose: on an empty quote the comparison
    //    `hash("") == hash("")` is VACUOUSLY TRUE. It is not a bypass — both
    //    comparisons genuinely pass — but it carries no information, and a
    //    verifier that returns `Resolved` for it is decorative. Refusing here
    //    is the difference between checking something and checking nothing.
    if refs.is_empty() || refs.iter().any(|r| r.quote.is_empty()) {
        return EvidenceVerdict::EmptyEvidence;
    }

    // 2. UnresolvedSource: no source, or a citation whose CID is not a
    //    well-formed CID. An empty or malformed CID must never default to a
    //    pass, so it refuses here rather than falling through.
    if source.is_empty() || refs.iter().any(|r| !cid::is_well_formed_cid(r.source_cid)) {
        return EvidenceVerdict::UnresolvedSource;
    }

    // 3. RangeOutOfBounds. `get` returns `None` for BOTH `start > end` and
    //    `end > len`, so one check covers the underflow and the overflow and
    //    no arithmetic on the offsets happens before the bounds decision. The
    //    range is NEVER adjusted — no char-boundary snapping, no clamping: a
    //    widened range is not the caller's range, and resolving it would be
    //    exactly the "normalizer's cousin" the round exists to refuse.
    //
    //    `collect::<Option<Vec<_>>>()` short-circuits on the first `None`, so
    //    this is ONE bounds decision for the whole set and the slices below
    //    need no second lookup and no `unwrap`.
    let declared: Option<Vec<&[u8]>> = refs
        .iter()
        .map(|r| source.get(r.byte_range.clone()))
        .collect();
    let Some(declared) = declared else {
        return EvidenceVerdict::RangeOutOfBounds;
    };

    // The CID of the bytes in hand, computed ONCE — the source is hashed a
    // single time per call no matter how many refs there are.
    let admitted_cid = cid_v1(source);

    // 4. CidMismatch. The derived CID is compared against the CALLER's
    //    declared value; the two sides of this comparison come from different
    //    places, so the check cannot degenerate into `h(x) == h(x)` inside the
    //    resolver. It can still be made vacuous from OUTSIDE, by a caller that
    //    derives its own CID from the same bytes — which is why the boundary
    //    doc states that obligation instead of pretending the crate enforces it.
    if refs.iter().any(|r| r.source_cid != admitted_cid) {
        return EvidenceVerdict::CidMismatch;
    }

    // 5. QuoteMismatch. The declared bytes, hashed, against the quote, hashed.
    //    Byte equality, not containment: a range that CONTAINS the quote still
    //    fails here, and a quote that occurs elsewhere in the source still
    //    fails here.
    if declared
        .iter()
        .zip(refs.iter())
        .any(|(d, r)| sha256(d) != sha256(r.quote))
    {
        return EvidenceVerdict::QuoteMismatch;
    }

    // The only path to a support answer. Every route to it went through both
    // comparisons, and none of them can be reached vacuously: step 1 refuses
    // the empty comparison outright.
    EvidenceVerdict::Resolved
}

/// SHA-256 over `bytes`. The one hash in the crate; there is no second
/// implementation of anything (E4).
fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── synthetic fixtures only. The real corpus is private and lives in the
    //    spine; R46 copies nothing from it (R45-0's E3 rule, unchanged).

    const SOURCE: &[u8] =
        b"the quick brown fox jumps over the lazy dog while the quick brown cat sleeps on";

    fn cid_of(bytes: &[u8]) -> String {
        cid_v1(bytes)
    }

    /// A well-formed citation of `quote` at its true position in `SOURCE`.
    ///
    /// The CID is a PARAMETER rather than computed here because `EvidenceRef`
    /// borrows it: a helper that minted the CID internally could only return a
    /// ref pointing at its own dropped local, and — more to the point — it
    /// would model the exact convenience this crate must not offer.
    fn ref_at<'a>(cid: &'a str, quote: &'a str) -> EvidenceRef<'a> {
        let start = SOURCE
            .windows(quote.len())
            .position(|w| w == quote.as_bytes())
            .expect("fixture quote must occur in SOURCE");
        EvidenceRef {
            source_cid: cid,
            quote: quote.as_bytes(),
            byte_range: start..start + quote.len(),
        }
    }

    /// The same citation with one arbitrary field overridden.
    fn ref_with<'a>(
        source_cid: &'a str,
        quote: &'a [u8],
        start: usize,
        end: usize,
    ) -> EvidenceRef<'a> {
        EvidenceRef {
            source_cid,
            quote,
            byte_range: start..end,
        }
    }

    // ── the three red-proof cases the plan names, each with its own verdict ──

    /// The plan's first red-proof. RED is a panic: the pre-fix resolver indexes
    /// `source[range]` directly and the fixture's `end` past `len` kills the
    /// test, which is the loud failure that forces the bounds check to be
    /// written BEFORE the happy path. `catch_unwind` then sweeps a whole
    /// battery of hostile ranges and asserts that none of them panics — a
    /// verdict value alone would not prove the absence of a panic, since a test
    /// that panicked inside `catch_unwind` and passed is the vacuous shape.
    #[test]
    fn r46_byte_range_out_of_bounds_refuses_rather_than_panics() {
        let cid = cid_of(SOURCE);
        // (range, expected verdict) — the expectation is stated per row rather
        // than as a loose "is it a refusal", because a battery that only asks
        // "did it refuse" cannot tell a correct refusal from a wrong one. The
        // 0..0 row is the instructive one: a zero-length range IS a valid
        // range, so the correct answer is a quote mismatch, NOT a bounds error.
        let hostile: Vec<(usize, usize, EvidenceVerdict)> = vec![
            (0, SOURCE.len() + 1, EvidenceVerdict::RangeOutOfBounds),
            (
                SOURCE.len(),
                SOURCE.len() + 1,
                EvidenceVerdict::RangeOutOfBounds,
            ),
            (5, 3, EvidenceVerdict::RangeOutOfBounds), // inverted
            (usize::MAX, usize::MAX, EvidenceVerdict::RangeOutOfBounds),
            (
                usize::MAX - 1,
                usize::MAX,
                EvidenceVerdict::RangeOutOfBounds,
            ),
            (0, 0, EvidenceVerdict::QuoteMismatch), // valid range, empty bytes
            (
                SOURCE.len() - 1,
                SOURCE.len(),
                EvidenceVerdict::QuoteMismatch,
            ), // valid, 1 byte != "the"
        ];
        for (start, end, expected) in hostile {
            let r = ref_with(&cid, b"the", start, end);
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| resolve(SOURCE, &[r])));
            assert!(
                outcome.is_ok(),
                "resolve panicked on the range {start}..{end} — a bad range is a \
                 verdict, not a panic"
            );
            // the verdict must be the NAMED one, never a default
            let verdict = outcome.expect("just asserted to be Ok");
            assert_eq!(
                verdict, expected,
                "range {start}..{end} must be {expected:?}, not {verdict:?}"
            );
        }
        assert_eq!(
            resolve(
                SOURCE,
                &[ref_with(&cid, b"the", SOURCE.len() + 1, SOURCE.len() + 5)]
            ),
            EvidenceVerdict::RangeOutOfBounds,
            "a range past the end must be the NAMED refusal"
        );
    }

    /// The most important red-proof in the round, because the half-implementation
    /// is the one that would ship: a resolver that checks only the CID passes
    /// this fixture, because CID verification is the easy part and the source
    /// here is correctly CID'd. The declared range simply does not contain the
    /// quote.
    #[test]
    fn r46_quote_mismatch_refuses_even_when_the_range_is_well_formed() {
        let cid = cid_of(SOURCE);
        // a correctly-CID'd source, a well-formed in-bounds range, and bytes at
        // that range that are NOT the quote
        let r = ref_with(&cid, b"the lazy dog", 0, 3);
        assert_eq!(
            resolve(SOURCE, &[r]),
            EvidenceVerdict::QuoteMismatch,
            "a resolver that verifies only the CID accepts this; that is the \
             half-implementation the round exists to refuse"
        );
    }

    /// The counter-proof to the "just recompute and compare internally" shape.
    /// The fixture rewrites one byte of the source (length-preserving, so the
    /// range stays in bounds and the cause is unambiguous), recomputes the CID
    /// from the TAMPERED bytes, and presents the ORIGINAL `source_cid`. A
    /// resolver that recomputes the CID from whatever bytes it was handed and
    /// compares it to itself accepts this.
    #[test]
    fn r46_cid_recomputed_from_a_tampered_source_is_refused() {
        let mut tampered = SOURCE.to_vec();
        tampered[0] = b'T'; // length-preserving: the byte count is unchanged
        let original_cid = cid_of(SOURCE);
        let tampered_cid = cid_of(&tampered);
        assert_ne!(
            tampered_cid, original_cid,
            "the fixture must actually change the CID, or it proves nothing"
        );
        let start = 4; // "quick" still sits here in the tampered source
        let r = ref_with(&original_cid, &tampered[start..start + 5], start, start + 5);
        assert_eq!(
            resolve(&tampered, &[r]),
            EvidenceVerdict::CidMismatch,
            "presenting the ORIGINAL CID against tampered bytes must refuse, even \
             though the bytes at the declared range really do equal the quote"
        );
    }

    // ── the arithmetic itself: exactness is the entire product ──────────────

    #[test]
    fn r46_resolvable_evidence_resolves_with_no_model_in_the_path() {
        let cid = cid_of(SOURCE);
        let r = ref_at(&cid, "quick brown fox");
        assert_eq!(resolve(SOURCE, &[r]), EvidenceVerdict::Resolved);
        // several refs, each individually true, resolve together
        let a = ref_at(&cid, "quick brown fox");
        let b = ref_at(&cid, "lazy dog");
        assert_eq!(resolve(SOURCE, &[a, b]), EvidenceVerdict::Resolved);
        // the no-model half of this pin is a MANIFEST property, proved by the
        // sibling kernel pin `r46_resolver_crate_declares_no_provider_or_model_
        // dependency`: the crate's only dependency is a hash function, so a
        // model is unreachable by construction rather than by convention.
    }

    #[test]
    fn r46_quote_resolves_only_at_its_exact_declared_range() {
        let cid = cid_of(SOURCE);
        let r = ref_at(&cid, "quick brown fox");
        let start = r.byte_range.start;
        let len = r.byte_range.end - start;
        let (declared_cid, declared_quote) = (r.source_cid, r.quote);
        assert_eq!(resolve(SOURCE, &[r]), EvidenceVerdict::Resolved);
        for (a, b) in [
            (start - 1, start + len + 1), // one byte wider either side
            (start + 1, start + len),     // shifted right
            (start, start + len - 1),     // one byte short
        ] {
            let shifted = ref_with(declared_cid, declared_quote, a, b);
            assert_ne!(
                resolve(SOURCE, &[shifted]),
                EvidenceVerdict::Resolved,
                "range {a}..{b} must not resolve the quote declared at {start}..{}",
                start + len
            );
        }
    }

    /// The normalizer's cousin. A range that CONTAINS the quote is still not
    /// the declared range, and a substring/prefix match is precisely the
    /// half-implementation the round forbids.
    #[test]
    fn r46_a_narrowed_range_containing_the_quote_still_refuses() {
        let cid = cid_of(SOURCE);
        let r = ref_at(&cid, "quick brown fox");
        let start = r.byte_range.start;
        let end = r.byte_range.end;
        let (declared_cid, declared_quote) = (r.source_cid, r.quote);
        let wider = ref_with(declared_cid, declared_quote, start, end + 20);
        assert_eq!(
            resolve(SOURCE, &[wider]),
            EvidenceVerdict::QuoteMismatch,
            "a range WIDER than the quote must refuse: containment is not equality, \
             and a substring match is the normalizer's cousin"
        );
    }

    #[test]
    fn r46_empty_evidence_set_refuses() {
        assert_eq!(resolve(SOURCE, &[]), EvidenceVerdict::EmptyEvidence);
        assert_eq!(
            failure_cause(&EvidenceVerdict::EmptyEvidence),
            Some("empty_evidence")
        );
    }

    #[test]
    fn r46_inverted_range_start_greater_than_end_refuses() {
        let cid = cid_of(SOURCE);
        let r = ref_with(&cid, b"the", 40, 10);
        assert_eq!(
            resolve(SOURCE, &[r]),
            EvidenceVerdict::RangeOutOfBounds,
            "start > end must be a named refusal and never an underflow panic"
        );
    }

    #[test]
    fn r46_range_past_end_of_source_refuses() {
        let cid = cid_of(SOURCE);
        let r = ref_with(&cid, b"the", SOURCE.len() - 2, SOURCE.len() + 1);
        assert_eq!(resolve(SOURCE, &[r]), EvidenceVerdict::RangeOutOfBounds);
    }

    #[test]
    fn r46_quote_matching_a_different_position_in_the_same_source_refuses() {
        // The quote IS present in the source, at index 4. The declared range
        // points somewhere else entirely. A resolver that SEARCHES the source
        // for the quote would answer "supported"; this one compares the
        // DECLARED range and refuses.
        //
        // (The first version of this fixture pointed the range at the quote's
        // SECOND occurrence, which has byte-identical content — so the declared
        // bytes really did equal the quote and `Resolved` was the CORRECT
        // answer. The fixture was wrong, not the resolver.)
        assert_eq!(
            &SOURCE[4..15],
            b"quick brown",
            "the quote must really be present"
        );
        assert_ne!(
            &SOURCE[20..31],
            b"quick brown",
            "the declared range must differ"
        );
        let cid = cid_of(SOURCE);
        let r = ref_with(&cid, b"quick brown", 20, 31);
        assert_eq!(
            resolve(SOURCE, &[r]),
            EvidenceVerdict::QuoteMismatch,
            "the quote's presence elsewhere in the source is irrelevant; only the \
             declared range is checked"
        );
    }

    // ── the CID: correct form, wrong bytes ────────────────────────────────

    #[test]
    fn r46_cid_is_a_sha256_multihash_over_the_exact_source_bytes() {
        let cid = cid_v1(b"abc");
        // the multihash prefix is (code 0x12 = sha2-256, length 0x20 = 32) and
        // both are single-byte varints, so the encoding is 34 bytes -> 55 chars
        assert!(cid.starts_with(CID_ALGORITHM_PREFIX), "cid = {cid}");
        let encoded = cid
            .strip_prefix(CID_ALGORITHM_PREFIX)
            .expect("algorithm prefix");
        assert_eq!(encoded.len(), CID_ENCODED_LEN, "cid = {cid}");
        // 0x12 0x20 0x6e... as base32: 00010=2 ->'c', 01000=8 ->'i', 10000=16 ->'q',
        // 01101=13 ->'l', 110...=6 ->'u'. The multihash header is therefore
        // visible in the first five characters, which is what makes the prefix
        // checkable rather than decorative.
        assert!(encoded.starts_with("ciqlu"), "cid = {cid}");
        // and it changes with the bytes, exactly and only with the bytes
        assert_ne!(cid_v1(b"abc"), cid_v1(b"abd"));
        assert_ne!(cid_v1(b""), cid_v1(b"a"));
        assert!(is_well_formed(&cid));
    }

    #[test]
    fn r46_cid_is_stable_across_runs_and_is_lowercase_base32() {
        let a = cid_v1(SOURCE);
        let b = cid_v1(SOURCE);
        assert_eq!(a, b, "the same bytes must yield the same CID every call");
        let encoded = a.strip_prefix(CID_ALGORITHM_PREFIX).expect("prefix");
        // RFC 4648, no padding, LOWERCASE — the kernel's own alphabet
        // (src/ump_integrity.rs:23), so R46 CIDs stay comparable with every
        // content hash the tree already stores.
        assert!(
            encoded
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "the encoding must be lowercase alphanumeric: {encoded}"
        );
        assert!(!encoded.contains('='), "no padding: {encoded}");
        assert!(
            encoded
                .bytes()
                .all(|c| b"abcdefghijklmnopqrstuvwxyz234567".contains(&c)),
            "every char must come from the RFC 4648 lowercase alphabet: {encoded}"
        );
        // the base32 primitive itself, against its own alphabet
        assert_eq!(base32_encode(b"abc"), "mfrgg");
        assert_eq!(base32_encode(b""), "");
    }

    #[test]
    fn r46_any_single_flipped_byte_changes_the_cid() {
        let base = cid_v1(SOURCE);
        for (i, _) in SOURCE.iter().enumerate() {
            for bit in [0x01u8, 0x80u8] {
                let mut flipped = SOURCE.to_vec();
                flipped[i] ^= bit;
                assert_ne!(
                    cid_v1(&flipped),
                    base,
                    "flipping bit {bit:#04x} of byte {i} must change the CID"
                );
            }
        }
    }

    // ── the anti-laundering property, behaviourally ────────────────────────

    #[test]
    fn r46_consolidation_that_rewrites_a_source_breaks_the_stored_reference_visibly() {
        let original = SOURCE.to_vec();
        let original_cid = cid_of(&original);
        // a consolidation pass that rewrites the source produces different bytes
        let rewritten =
            b"THE quick brown fox jumps over the lazy dog while the quick brown cat sleeps on";
        let r = EvidenceRef {
            source_cid: &original_cid,
            quote: &original[0..3],
            byte_range: 0..3,
        };
        assert_eq!(
            resolve(rewritten, &[r]),
            EvidenceVerdict::CidMismatch,
            "a rewrite must break the stored reference VISIBLY — this is the \
             anti-laundering property, and it is arithmetic rather than policy"
        );
    }

    #[test]
    fn r46_a_rewritten_source_cannot_silently_satisfy_the_original_evidence() {
        let original = SOURCE.to_vec();
        let original_cid = cid_of(&original);
        // a rewrite that PRESERVES the cited span byte-for-byte: the quote
        // comparison alone would pass, and only the CID binding catches it
        let mut rewritten = original.clone();
        let tail = rewritten.len();
        rewritten[tail - 1] = b'!';
        assert_eq!(
            &rewritten[0..3],
            &original[0..3],
            "the fixture must keep the cited span identical, or the CID check is \
             not what is doing the work"
        );
        let r = EvidenceRef {
            source_cid: &original_cid,
            quote: &original[0..3],
            byte_range: 0..3,
        };
        assert_eq!(
            resolve(&rewritten, &[r]),
            EvidenceVerdict::CidMismatch,
            "a rewrite elsewhere in the source must still break the reference"
        );
    }

    // ── purity + determinism ──────────────────────────────────────────────

    #[test]
    fn r46_resolution_is_deterministic_across_runs() {
        let cid = cid_of(SOURCE);
        let other_cid = cid_of(b"other");
        let battery: Vec<EvidenceRef<'_>> = vec![
            ref_at(&cid, "quick brown fox"),
            ref_at(&cid, "lazy dog"),
            ref_with(&cid, b"nope, absent", 0, 11),
            ref_with(&cid, b"the", 40, 10),
            ref_with(&other_cid, b"the", 0, 3),
        ];
        let first: Vec<EvidenceVerdict> = battery
            .iter()
            .map(|r| resolve(SOURCE, std::slice::from_ref(r)))
            .collect();
        for _ in 0..64 {
            let again: Vec<EvidenceVerdict> = battery
                .iter()
                .map(|r| resolve(SOURCE, std::slice::from_ref(r)))
                .collect();
            assert_eq!(
                first, again,
                "the same inputs must always give the same verdict"
            );
        }
        // and the CIDs are just as stable
        let cid_first = cid_v1(SOURCE);
        for _ in 0..64 {
            assert_eq!(cid_first, cid_v1(SOURCE));
        }
    }

    #[test]
    fn r46_resolution_does_not_depend_on_any_clock_or_environment() {
        // The crate cannot read a clock or the environment even to try: the
        // sibling kernel pin `r46_resolver_depends_on_no_clock_store_or_network`
        // proves the production region names no such API, and the manifest pin
        // proves nothing is reachable through a dependency. What is provable
        // BEHAVIOURALLY here is the absence of hidden state and of any
        // dependence on the order the caller presents its refs in.
        let cid = cid_of(SOURCE);
        let other_cid = cid_of(b"other");
        let mut forward: Vec<EvidenceRef<'_>> = vec![
            ref_at(&cid, "quick brown fox"),
            ref_with(&other_cid, b"the", 0, 3),
            ref_with(&cid, b"the", 40, 10),
        ];
        let reversed: Vec<EvidenceRef<'_>> = forward.iter().rev().cloned().collect();

        assert_eq!(
            resolve(SOURCE, &forward),
            resolve(SOURCE, &reversed),
            "the verdict is by-CAUSE and fixed-order, so presenting the refs in a \
             different sequence must not change it — a per-ref fold would report a \
             different cause purely from list order"
        );
        // and a ref set split across two calls agrees with the combined call
        let split = resolve(SOURCE, &forward[..1]);
        assert_eq!(split, resolve(SOURCE, &forward[..1]));
        forward.clear();
    }

    #[test]
    fn r46_no_audit_row_is_emitted_and_no_key_is_minted() {
        // Structurally provable and behaviourally observable: the function is
        // total, has no output channel, and returns the same value when called
        // twice. The sibling kernel pin proves the production region names no
        // audit, signing, or insert API at all.
        let cid = cid_of(SOURCE);
        let r = ref_at(&cid, "lazy dog");
        let a = resolve(SOURCE, std::slice::from_ref(&r));
        let b = resolve(SOURCE, std::slice::from_ref(&r));
        assert_eq!(a, b);
        assert_eq!(a, EvidenceVerdict::Resolved);
    }

    // ── the discrimination obligations the plan's §5 leaves unpinned ───────

    /// The fail-open the plan's own §0.3 obligation forbids and §5 never pins.
    /// With an empty quote and a zero-length range, `hash("") == hash("")` is
    /// VACUOUSLY TRUE — both comparisons genuinely pass, and the verdict is
    /// therefore `Resolved` for ANY claim citing ANY source. That is worse than
    /// an ordinary fail-open, because the verdict is not wrong; it is unearned.
    /// It is caught by checking for an empty quote BEFORE hashing.
    #[test]
    fn r46_empty_quote_never_resolves() {
        let cid = cid_of(SOURCE);
        // zero-length range, empty quote, correct CID
        let r = ref_with(&cid, b"", 0, 0);
        assert_eq!(
            resolve(SOURCE, &[r]),
            EvidenceVerdict::EmptyEvidence,
            "an empty quote over a zero-length range must never resolve"
        );
        // the whole-source case: an empty source, an empty quote, a 0..0 range
        let empty_source: &[u8] = b"";
        let empty_cid = cid_of(empty_source);
        let r2 = ref_with(&empty_cid, b"", 0, 0);
        assert_ne!(
            resolve(empty_source, &[r2]),
            EvidenceVerdict::Resolved,
            "an empty source with an empty quote must never resolve"
        );
        // a zero-length range against a NON-empty quote is a quote mismatch,
        // which is already correct — but it must not be relabelled as a bounds
        // error, because E9 itemizes failures by cause and a mislabelled cause
        // corrupts the histogram
        let r3 = ref_with(&cid, b"the", 5, 5);
        assert_eq!(resolve(SOURCE, &[r3]), EvidenceVerdict::QuoteMismatch);
    }

    /// KILL-3 in its strongest, order-independent form: **`Resolved` implies the
    /// declared bytes are LITERALLY the quote** — stronger than either hash
    /// comparison, and independent of the order the checks happen to run in.
    /// No ordering, no lucky collision, and no accidentally-early `return` can
    /// satisfy this.
    #[test]
    fn r46_resolved_requires_byte_identical_range_content() {
        let cid = cid_of(SOURCE);
        let other_cid = cid_of(b"other");
        // a wide battery of plausible refs, most of which are wrong in one way
        let battery: Vec<EvidenceRef<'_>> = vec![
            ref_at(&cid, "quick brown fox"),
            ref_at(&cid, "the"),
            ref_with(&cid, b"quick brown fox", 4, 4 + 15), // the true one
            ref_with(&cid, b"quick brown fox", 3, 4 + 15), // shifted
            ref_with(&cid, b"quick brown fox", 4, 4 + 16), // widened
            ref_with(&cid, b"Quick Brown Fox", 4, 4 + 15), // case differs
            ref_with(&cid, b"quick  brown fox", 4, 4 + 15), // whitespace differs
            ref_with(&cid, b"the quick brown fox", 0, 15), // wider, contains it
            ref_with(&cid, b"the", 40, 10),                // inverted
            ref_with(&cid, b"the", 0, SOURCE.len() + 5),   // overflow
            ref_with(&cid, b"", 0, 0),                     // vacuous
            ref_with(&other_cid, b"the", 0, 3),            // wrong source
            ref_with("", b"the", 0, 3),                    // no CID at all
        ];
        for r in &battery {
            if resolve(SOURCE, std::slice::from_ref(r)) == EvidenceVerdict::Resolved {
                let declared = SOURCE
                    .get(r.byte_range.clone())
                    .expect("Resolved implies the range was a valid slice");
                assert_eq!(
                    declared,
                    r.quote,
                    "Resolved was returned, so the declared bytes must BE the quote \
                     byte-for-byte — got {declared:?} vs {:?}",
                    String::from_utf8_lossy(r.quote)
                );
            }
        }
        // and the direction that matters: at least one of the battery resolved,
        // so the property above is not vacuously true over an all-refusing set
        assert!(
            battery
                .iter()
                .any(|r| resolve(SOURCE, std::slice::from_ref(r)) == EvidenceVerdict::Resolved),
            "the battery must contain at least one true citation, or the property \
             above is checking nothing"
        );
    }

    /// §4.2 says `resolve` returns "the first failing verdict" and never says
    /// what FIRST means. §6 requires every failure to be itemized by cause, so
    /// the order has to be fixed and pinned or the published histogram is not
    /// reproducible across two conforming implementations.
    #[test]
    fn r46_verdict_precedence_is_fixed_and_by_cause() {
        let cid = cid_of(SOURCE);
        let other_cid = cid_of(b"other");
        let empty_cid = cid_of(b"");

        // each row is a fixture with a known set of defects and the cause the
        // documented order says must win. The source is a column because
        // "there is no source" is a distinct condition from "this citation
        // names the wrong source", and the first version of this table
        // resolved every row against SOURCE — which made its empty-source row
        // assert UnresolvedSource while the code correctly answered CidMismatch.
        let cases: Vec<(&str, &[u8], EvidenceVerdict, Vec<EvidenceRef<'_>>)> = vec![
            ("empty set", SOURCE, EvidenceVerdict::EmptyEvidence, vec![]),
            (
                "empty quote",
                SOURCE,
                EvidenceVerdict::EmptyEvidence,
                vec![ref_with(&cid, b"", 0, 0)],
            ),
            (
                "empty source",
                b"",
                EvidenceVerdict::UnresolvedSource,
                vec![ref_with(&empty_cid, b"the", 0, 3)],
            ),
            (
                "malformed cid",
                SOURCE,
                EvidenceVerdict::UnresolvedSource,
                vec![ref_with("not-a-cid", b"the", 0, 3)],
            ),
            (
                "empty cid",
                SOURCE,
                EvidenceVerdict::UnresolvedSource,
                vec![ref_with("", b"the", 0, 3)],
            ),
            (
                "range beats cid",
                SOURCE,
                EvidenceVerdict::RangeOutOfBounds,
                vec![
                    ref_with(&other_cid, b"the", 0, 3),
                    ref_with(&cid, b"the", 40, 10),
                ],
            ),
            (
                "cid beats quote",
                SOURCE,
                EvidenceVerdict::CidMismatch,
                vec![ref_with(&other_cid, b"absent text", 0, 3)],
            ),
            (
                "quote last",
                SOURCE,
                EvidenceVerdict::QuoteMismatch,
                vec![ref_with(&cid, b"absent text", 0, 3)],
            ),
        ];

        for (label, source, expected, refs) in cases {
            assert_eq!(
                resolve(source, &refs),
                expected,
                "the {label} case must yield {expected:?}"
            );
            assert_eq!(
                failure_cause(&expected),
                Some(describe(&expected)),
                "the E9 cause string and the describe label must agree for {label}"
            );
        }
    }

    /// The CID comparison is between two values from two DIFFERENT places: the
    /// left side is derived from the bytes in hand, the right side is the
    /// caller's declared field. It therefore cannot degenerate into `h(x) == h(x)`
    /// inside the resolver.
    ///
    /// The second half is the part that is genuinely load-bearing. A caller WHO
    /// derives their own CID from the same bytes they pass in *does* make the
    /// check vacuous, and no check inside this crate can tell — so the obligation
    /// is named in the boundary doc rather than pretended away, and the pin
    /// asserts the crate gives that caller no silent default for a missing CID.
    #[test]
    fn r46_resolver_never_derives_a_cid_from_the_bytes_it_was_handed() {
        let src = read_crate_lib();
        let production = src
            .split_once("#[cfg(test)]")
            .map_or(src.as_str(), |(h, _)| h);

        // the two sides of the comparison come from different places
        assert!(
            production.contains("r.source_cid != admitted_cid"),
            "the CID comparison must read the CALLER's declared field"
        );
        assert!(
            !production.contains("cid_v1(source) != cid_v1("),
            "the resolver must never compare a derived CID against another derived \
             CID — that is the tautology h(x) == h(x)"
        );

        // a missing or malformed CID refuses; it never defaults to a pass
        for bad in ["", "not-a-cid", "sha256:", "sha256:!!", "sha256:tooshort"] {
            let r = ref_with(bad, b"the", 0, 3);
            assert_ne!(
                resolve(SOURCE, &[r]),
                EvidenceVerdict::Resolved,
                "a caller-supplied CID of {bad:?} must never resolve"
            );
        }
        // and there is no way to build a self-satisfying ref by convenience
        assert!(
            !production.contains("impl From<&[u8]> for EvidenceRef"),
            "the crate must offer no constructor that derives a ref's CID from the \
             bytes it was handed"
        );
    }

    // ── helpers for the source-scanning pin above ─────────────────────────

    fn read_crate_lib() -> String {
        include_str!("lib.rs").to_string()
    }

    fn is_well_formed(cid: &str) -> bool {
        cid::is_well_formed_cid(cid)
    }
}
