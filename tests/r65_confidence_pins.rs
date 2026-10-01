//! The confidence input — the two red-proofs for the keyword-share defect.
//!
//! `classify` computed `confidence = best_hits / total_hits`: the winning
//! category's **share of the keywords that fired**, which is an uncontested-share
//! proxy, not a confidence. Combined with unbounded `lower.contains(k)`
//! matching, a *single* spurious hit scored a perfect `1.00`:
//!
//! | input            | keywords fired | confidence (before) |
//! |------------------|----------------|--------------------|
//! | "the email arrived" | `["ai"]`   | **1.00**           |
//! | "she said again"    | `["ai"]`   | **1.00**           |
//! | "a detailed report" | `["ai"]`   | **1.00**           |
//!
//! Any threshold on that number routes confidently on false positives, which is
//! why the input is fixed before any deferral policy is written.
//!
//! ## The boundary rule, pinned (was underspecified)
//!
//! "Token-boundary match" does not by itself define a tokenizer. The rule this
//! round ships and pins is: **split the lowercased text on non-alphanumeric
//! characters; a keyword fires iff it equals a whole token.** So
//! `ai-assistant` DOES fire `ai` (hyphens split), and `openai` does NOT (the
//! vendor keyword is itself a substring trap — `"openai"` contains `"ai"`).
//!
//! ## Two traps this programme has already hit, both encoded below
//!
//! 1. **A pin that passes with the defect planted** because it reads a hand-built
//!    literal instead of the real value. Every assertion here calls the real
//!    `classify` and reads the real returned struct — no expected-literal tables.
//! 2. **A fixture that emits a second signal**, so a looser rule passes it too.
//!    `evidence_count_discriminates_...` asserts it *discriminates*: it checks
//!    that the two fixtures score the **same** confidence and differ **only** in
//!    the count. A rule that passed both would pass neither distinction.

use brain_server::procedural::classify;

// ── R65.10 · the keyword-share trap ────────────────────────────────────────

/// The defect itself: a text with no technology *signal* must not score a
/// high-confidence technology verdict.
///
/// Each fixture is isolated — asserted to be `general`, so no other category's
/// keyword is quietly firing and accidentally satisfying the assertion. This is
/// the R58a trap: a fixture that "also emits a second signal" passes a looser
/// rule for the wrong reason.
#[test]
fn r65_10_substring_hit_cannot_manufacture_a_confident_verdict() {
    for text in [
        "the email arrived",
        "she said again",
        "a detailed report",
        "they maintain the ledger",
    ] {
        let r = classify(text);
        assert_eq!(
            r.category, "general",
            "no whole-word keyword in {text:?}, so the router must abstain — \
             got {:?}",
            r.category
        );
        assert_eq!(
            r.evidence_count, 0,
            "{text:?} must fire zero keywords, got {:?}",
            r.matched_keywords
        );
        assert_eq!(r.confidence, 0.0, "{text:?} abstains at 0.0");
        assert!(
            r.matched_keywords.is_empty(),
            "{text:?} must report no matched keywords, got {:?}",
            r.matched_keywords
        );
    }
}

/// The strongest single case, kept separate because it is the one that also
/// defeats a *category-level* reading: `"openai"` is a real vendor keyword and
/// contains the technology keyword `"ai"`. Under substring matching the
/// technology vote is manufactured from inside a vendor's name.
#[test]
fn r65_10_openai_does_not_manufacture_a_technology_vote() {
    let r = classify("openai");
    assert_eq!(
        r.category, "vendor",
        "the whole word `openai` is a vendor keyword; `ai` must not fire inside it"
    );
    assert!(
        !r.matched_keywords.iter().any(|k| k == "ai"),
        "substring matching leaked `ai` out of `openai`: {:?}",
        r.matched_keywords
    );
}

/// The other direction, so the boundary rule is pinned on **both** sides: a
/// hyphen is a boundary, so `ai-assistant` and `AI-driven` DO fire `ai`. A pin
/// that only asserted rejections would pass a matcher that simply never fired.
#[test]
fn r65_10_punctuation_is_a_boundary_so_real_tokens_still_fire() {
    for text in [
        "an ai-assistant rollout",
        "an AI-driven strategy",
        "cloud-native, API-first",
        "repetitive/manual work",
    ] {
        let r = classify(text);
        assert_ne!(
            r.category, "general",
            "{text:?} contains whole-word keywords and must classify — the \
             boundary rule must not over-reject"
        );
        assert!(
            r.evidence_count >= 1,
            "{text:?} must report at least one fired keyword, got {:?}",
            r.matched_keywords
        );
    }
}

/// The negative-side control for the pin above: proves the two directions are
/// genuinely distinguished by one rule, not by two special cases.
#[test]
fn r65_10_boundary_rule_is_consistent_both_ways() {
    // FIRES: whole token present.
    assert_ne!(classify("ai-assistant").category, "general");
    // DOES NOT FIRE: keyword present only as an infix.
    assert_eq!(classify("email").category, "general");
    assert_eq!(classify("said").category, "general");
    assert_eq!(classify("detail").category, "general");
    // DOES NOT FIRE: a *different* lexicon entry's infix.
    assert_eq!(classify("maintain").category, "general");
    assert_eq!(classify("openai").category, "vendor");
    // Case is still folded (existing contract preserved).
    assert_eq!(classify("AI").category, "technology");
}

// ── R65.10b · evidence_count is the input a threshold actually needs ───────

/// `confidence` alone cannot distinguish "one keyword fired and agreed" from
/// "ten keywords fired and all ten agreed" — both score `1.0`. The count
/// separates them.
///
/// **This fixture asserts that it discriminates.** Both inputs are drawn from
/// the same category and both must score an identical `confidence` of exactly
/// `1.0`; the ONLY difference is the count. A fixture that let the confidences
/// differ would be satisfied by any monotonic score and would prove nothing —
/// the R58a trap.
#[test]
fn r65_10b_evidence_count_separates_what_confidence_cannot() {
    // One technology keyword fires; nothing else fires anywhere.
    let one = classify("cloud");
    // All TEN technology keywords fire, and nothing else.
    //
    // Fixture isolation, asserted rather than assumed: this string is built
    // from the technology lexicon alone, and every word is checked against the
    // OTHER seven categories below. An earlier draft ended "...saas automation
    // model" preceded by "budget" — a finance keyword — which dropped the share
    // to 10/11 and made the fixture emit a second signal. The trap fires on the
    // person writing the fixture, not only on the code under test.
    let ten_text = "ai ml llm api cloud saas automation integration software model";
    let ten = classify(ten_text);

    // Every token of the ten-keyword fixture is technology-only: asserted, not
    // assumed. An earlier draft of this fixture contained "budget" — a finance
    // keyword — which dropped the share to 10/11 and made the fixture emit a
    // second signal. The trap fires on whoever writes the fixture, not only on
    // the code under test.
    for token in ten_text.split_whitespace() {
        let solo = classify(token);
        assert_eq!(
            solo.category, "technology",
            "fixture token {token:?} must route to technology alone, got {:?}",
            solo.category
        );
        assert_eq!(
            solo.evidence_count, 1,
            "fixture token {token:?} must fire exactly one keyword, got {:?}",
            solo.matched_keywords
        );
    }

    // Both are technology, uncontested.
    assert_eq!(
        one.category, "technology",
        "fixture drifted: {:?}",
        one.category
    );
    assert_eq!(
        ten.category, "technology",
        "fixture drifted: {:?}",
        ten.category
    );

    // THE DISCRIMINATION ASSERTION: confidence is identical between the two
    // fixtures. If these ever differ, the fixture has stopped isolating the
    // property it claims to isolate and must be rewritten, not re-tuned.
    assert_eq!(
        one.confidence, 1.0,
        "the single-keyword fixture must still score a perfect share"
    );
    assert_eq!(
        ten.confidence, 1.0,
        "the ten-keyword fixture must still score a perfect share"
    );
    assert_eq!(
        one.confidence, ten.confidence,
        "both fixtures score an uncontested share — that equality IS the trap. \
         If confidence differed here, this pin would no longer be testing the \
         thing it claims to test."
    );

    // The count is what separates them.
    assert_eq!(
        one.evidence_count, 1,
        "expected exactly one fired keyword, got {:?}",
        one.matched_keywords
    );
    assert_eq!(
        ten.evidence_count, 10,
        "expected all ten technology keywords, got {:?}",
        ten.matched_keywords
    );
    assert!(
        one.evidence_count != ten.evidence_count,
        "the count must actually discriminate where the share cannot"
    );

    // The count is DERIVED FROM the real matched list, never an independent
    // count — so it cannot drift from what a client can see.
    assert_eq!(one.evidence_count, one.matched_keywords.len());
    assert_eq!(ten.evidence_count, ten.matched_keywords.len());
}

/// The `general` path reports `0`, not a fabricated count.
#[test]
fn r65_10b_general_path_reports_zero_evidence() {
    let r = classify("the cat sat on the mat");
    assert_eq!(r.category, "general");
    assert_eq!(r.evidence_count, 0);
    assert_eq!(r.confidence, 0.0);
    assert!(r.matched_keywords.is_empty());
}

/// `confidence == 0.0` and `category == "general"` are **the same signal**, not
/// two independent facts. Pinned across a corpus so the pair cannot drift into
/// disagreeing — a caller testing only one of them is relying on an accident.
#[test]
fn r65_10b_zero_confidence_and_general_are_one_signal() {
    let corpus = [
        ("the cat sat on the mat", true),
        ("the email arrived", true),
        ("openai", false),
        ("cloud", false),
        ("ROI is 3x; budget is $50k; they use QuickBooks.", false),
        ("We need an LLM integration via their SaaS API.", false),
        ("", true),
        ("!!! ??? ...", true),
    ];
    for (text, expect_general) in corpus {
        let r = classify(text);
        assert_eq!(
            r.category == "general",
            expect_general,
            "corpus entry {text:?} routed to {:?}",
            r.category
        );
        assert_eq!(
            r.confidence == 0.0,
            r.category == "general",
            "confidence==0.0 and category==general must be the same signal \
             (disagreed on {text:?}: conf={} cat={:?})",
            r.confidence,
            r.category
        );
        // And the third member of that signal agrees too.
        assert_eq!(
            r.evidence_count == 0,
            r.category == "general",
            "evidence_count==0 must agree with general (disagreed on {text:?})"
        );
    }
}

// ── R65.11 · behaviour preservation ────────────────────────────────────────

/// The seven pre-existing pins, asserted here against the **real** router so a
/// change to the shipped surface cannot pass unnoticed by relying on the
/// source file's own assertions alone.
///
/// The substring matcher matched real vendors inside ordinary prose. Every one
/// of these inputs contains its keywords as whole words, so all seven must
/// survive the boundary rule unchanged.
#[test]
fn r65_11_the_seven_shipped_pins_still_hold_under_the_boundary_rule() {
    // classify_returns_general_when_no_keyword_matches
    let r = classify("the cat sat on the mat");
    assert_eq!(r.category, "general");
    assert_eq!(r.confidence, 0.0);
    assert!(r.matched_keywords.is_empty());

    // classify_detects_compliance
    let r = classify("This client handles patient records; HIPAA and PII apply.");
    assert_eq!(r.category, "compliance");
    assert!(r.confidence > 0.0);
    assert!(r.matched_keywords.iter().any(|k| k == "hipaa"));
    assert!(r.matched_keywords.iter().any(|k| k == "pii"));

    // classify_detects_technology
    let r = classify("We need an LLM integration via their SaaS API.");
    assert_eq!(r.category, "technology");
    assert!(r.matched_keywords.iter().any(|k| k == "llm"));

    // classify_detects_finance
    let r = classify("ROI is 3x; budget is $50k; they use QuickBooks.");
    assert_eq!(r.category, "finance");
    assert!(r.confidence > 0.0);

    // classify_is_case_insensitive
    assert_eq!(classify("HIPAA HIPAA hipaa").category, "compliance");
    assert_eq!(classify("AWS azure AZURE").category, "vendor");

    // classify_confidence_is_winning_fraction
    let r = classify("AI automation budget");
    assert_eq!(r.category, "technology");
    assert!(
        (r.confidence - (2.0 / 3.0)).abs() < 1e-6,
        "2 of 3 keywords agree: got {}",
        r.confidence
    );
    assert_eq!(
        r.evidence_count, 2,
        "the two technology keywords that fired"
    );

    // classify_vendor_lexicon_matches_real_vendors
    assert_eq!(classify("OpenAI vs Anthropic vs Google").category, "vendor");
    assert_eq!(classify("They're on Microsoft Azure").category, "vendor");
}

// ── property · idempotence ────────────────────────────────────────────────

// proptest expands into module-level `#[test]` items, so the macro is NOT
// wrapped in an outer `#[test]` fn — that nests a test inside a test and does
// not compile ("cannot test inner items"). House shape, same as
// `src/capacity.rs` and `src/chunker.rs`.

// `classify` is idempotent on repeated input: same text, same verdict. The
// router must not accumulate state across calls (a `HashMap` accumulator, a
// cached lexicon, a counter) — a defect that would make one call's answer
// depend on the calls before it.
//
// Context is passed as explicit arguments: proptest expands assertions inside a
// generated closure, where `format_args!` cannot implicitly capture.
use proptest::prelude::*;

proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(64))]
    #[test]
    fn proptest_classify_is_idempotent(text in ".{0,200}") {
        let once = classify(&text);
        let twice = classify(&text);
        prop_assert_eq!(
            once.category, twice.category,
            "category drifted on repeat for {:?}", text
        );
        prop_assert_eq!(
            once.matched_keywords, twice.matched_keywords,
            "matched_keywords drifted on repeat for {:?}", text
        );
        prop_assert_eq!(
            once.evidence_count, twice.evidence_count,
            "evidence_count drifted on repeat for {:?}", text
        );
        prop_assert_eq!(
            once.confidence.to_bits(), twice.confidence.to_bits(),
            "confidence drifted on repeat for {:?}", text
        );
        // Re-running through the real router a third time is still stable.
        let thrice = classify(&text);
        prop_assert_eq!(
            thrice.category, once.category,
            "category drifted on the third call for {:?}", text
        );
    }
}

// A structural law that holds for every input, not a fixture: the count is
// always the length of the reported list, every reported keyword is a
// lowercased lexicon entry, and no keyword is ever reported twice.
proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(64))]
    #[test]
    fn proptest_evidence_count_always_matches_the_reported_list(text in ".{0,200}") {
        let r = classify(&text);
        prop_assert_eq!(
            r.evidence_count, r.matched_keywords.len(),
            "count must equal the reported list length for {:?}", text
        );
        let mut seen: Vec<&str> = Vec::new();
        for k in &r.matched_keywords {
            prop_assert!(
                !seen.contains(&k.as_str()),
                "keyword {:?} reported twice for {:?}", k, text
            );
            prop_assert!(
                k.chars().all(|c| c.is_ascii_lowercase()),
                "keyword {:?} is not a lowercased lexicon entry for {:?}", k, text
            );
            seen.push(k);
        }
        // The general path is the only zero-evidence path.
        prop_assert_eq!(
            r.evidence_count == 0, r.category == "general",
            "zero evidence must mean general for {:?}", text
        );
    }
}
