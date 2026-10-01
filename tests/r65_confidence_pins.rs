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

// ── The deferral decision ──────────────────────────────────────────────────
//
// The pins below drive the REAL decision path end to end: `classify()` on a
// text, the label resolved into a `RoutingClass` by the real resolver, and the
// real `decide_deferral()` called with the features the classifier actually produced.
//
// ## The two traps this programme has hit, both encoded here
//
// 1. **A pin that passes with the defect planted**, because it reads a
//    hand-built literal instead of the real value. The vocabulary-parity pin
//    derives its expected set from `procedural::CATEGORIES` and the resolved
//    label from the real `classify()` output — never from a restated table.
// 2. **A fixture claiming discrimination must assert it discriminates.** The
//    empty-table pin proves discrimination by showing two classes that differ
//    in *every* observable except the one under test, and by requiring the
//    measured-reliability lookup to be `None` for all of them.
//
// ## Why the "empty table" pin is the load-bearing one
//
// A policy that defers everything scores perfectly on accuracy and is
// worthless in production. So a future round WILL want to grant a class `Auto`.
// This file makes that a loud, deliberate act: the pin fails the moment an
// entry appears without a measured reliability beside it.

use brain_server::workflow::confidence::{
    DeferralFeatures, DeferralOutcome, RoutingClass, decide_deferral, measured_reliability,
    routing_classes_cover_classifier,
};

/// The features a real classification produced, resolved through the real path.
fn deferral_for(text: &str) -> (RoutingClass, DeferralFeatures, DeferralOutcome) {
    let r = classify(text);
    let class = RoutingClass::from_label(r.category);
    let features = DeferralFeatures::new(r.evidence_count, r.confidence);
    let outcome = decide_deferral(class, &features).expect("a finite confidence must decide");
    (class, features, outcome)
}

// ── I65.2a · every class is HumanRequired — the fail-closed default ────────

/// **The round's load-bearing claim.** No class is auto-authorised, because no
/// per-class reliability has been measured.
///
/// This is asserted over BOTH sources of classes — the enum's own `ALL`, and
/// the classifier's real vocabulary — so it cannot pass while a class exists
/// that the enum forgot.
#[test]
fn i65_2a_every_class_defers_and_the_table_ships_empty() {
    // Driven through the real `ALL`, not a hand-built list.
    assert!(
        !RoutingClass::ALL.is_empty(),
        "fixture drifted: the class list is empty"
    );
    for class in RoutingClass::ALL {
        assert_eq!(
            measured_reliability(class),
            None,
            "{class:?} carries a measured reliability — the table is no longer empty, \
             which is a measurement event that must be recorded deliberately"
        );
        for evidence in [0usize, 1, 3, 10, 100] {
            let features = DeferralFeatures::new(evidence, 1.0);
            let outcome = decide_deferral(class, &features).expect("decide");
            assert_eq!(
                outcome,
                DeferralOutcome::Defer,
                "{class:?} with {evidence} keywords must defer while the table is empty"
            );
        }
    }
}

/// **Discrimination assertion** (trap #2). A pin that only checked "the outcome
/// is one of three" would pass an implementation that always returned the first
/// variant for any reason at all.
///
/// So this proves the classes are genuinely distinct inputs: two classes that
/// differ in every observable except the one under test must both reach the same
/// outcome *through different code*, and the resolver must place them apart.
/// If a future implementation made every class collapse to one variant for a
/// reason other than the table, this is where it would show.
#[test]
fn i65_2a_the_fixture_discriminates_between_classes() {
    // Two classes, maximally different inputs.
    let (tech, tech_features, tech_outcome) = deferral_for("cloud saas api software");
    let (fin, fin_features, fin_outcome) =
        deferral_for("ROI is 3x; budget is $50k; they use QuickBooks.");

    // The fixture really is discriminating: different classes, different
    // features. If these ever agree, the fixture stopped isolating the property.
    assert_ne!(
        tech, fin,
        "fixture drifted: both inputs classified the same"
    );
    assert_ne!(
        tech_features, fin_features,
        "fixture drifted: both inputs produced identical features"
    );

    // And the shared outcome is therefore a real decision, not an artefact of
    // the inputs being indistinguishable.
    assert_eq!(tech_outcome, DeferralOutcome::Defer);
    assert_eq!(fin_outcome, DeferralOutcome::Defer);
}

// ── I65.2b · an absent class is refused, never guessed ────────────────────

/// Absence of evidence is not evidence of competence. An unrecognised label
/// must resolve to the absence class and defer.
///
/// This is the typo case: `"complaince"` must not be silently mapped onto
/// `compliance`, because a class that earned authority would then be reachable
/// by misspelling it.
#[test]
fn i65_2b_an_unrecognised_label_defers_and_is_never_guessed() {
    for label in [
        "complaince",
        "FINANCE",
        "Compliance",
        "",
        "technolog",
        "factual",
    ] {
        let class = RoutingClass::from_label(label);
        assert_eq!(
            class,
            RoutingClass::HumanUnmeasured,
            "{label:?} must resolve to the absence class, never to a neighbour"
        );
        let outcome = decide_deferral(class, &DeferralFeatures::new(50, 1.0)).expect("decide");
        assert_eq!(
            outcome,
            DeferralOutcome::Defer,
            "{label:?} must defer even on overwhelming evidence"
        );
    }
}

// ── I65.2c · vocabulary parity, both directions ────────────────────────────

/// The classifier's vocabulary and the deferral policy must agree on what a
/// label IS. A label the classifier emits with no policy variant would resolve
/// to the absence class — safe, but a silent degradation.
///
/// **Both directions, derived from the real source** (trap #1): the classifier's
/// labels against the policy's resolver, and the policy's own enum against the
/// same labels. A hand-built expected list would pass beside a real drift.
#[test]
fn i65_2c_routing_classes_cover_the_classifier_vocabulary() {
    // Forward: every real classifier label resolves to a KNOWN class.
    assert!(
        routing_classes_cover_classifier(),
        "the classifier emits a label the deferral policy does not model — it would \
         silently degrade to HumanRequired"
    );
    for label in brain_server::procedural::CATEGORIES {
        let class = RoutingClass::from_label(label);
        assert_ne!(
            class,
            RoutingClass::HumanUnmeasured,
            "classifier label {label:?} resolved to the absence class"
        );
    }

    // Reverse: every enum variant except the absence class is a real label.
    // The absence class is deliberately NOT a classifier label — it exists for
    // labels the classifier does not know — so it is excluded by name.
    for class in RoutingClass::ALL {
        if class == RoutingClass::HumanUnmeasured {
            continue;
        }
        assert!(
            brain_server::procedural::CATEGORIES.contains(&class.as_str()),
            "{class:?} ({:?}) is not a classifier label — it is dead policy surface",
            class.as_str()
        );
    }
}

// ── I65.3 · monotonicity · I65.4 · purity ─────────────────────────────────

proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(128))]

    /// The authority the machine is granted for one outcome.
    ///
    /// **This pin was vacuous twice before it earned its name, and both failures
    /// are the finding:**
    ///
    /// 1. The first version ranked `Stop` as the *most* human outcome, so the
    ///    assertion "more evidence never reduces human involvement" passed
    ///    against a planted inversion. The rank was simply backwards.
    /// 2. The second version asserted `requires_human()`, which is `true` for
    ///    **every** outcome by construction — so it could not distinguish `Defer`
    ///    from `Stop` and passed against the same plant.
    ///
    /// The honest property, and the one that survives both: **the outcome is a
    /// function of the class alone, not of the evidence.** A deferral policy that
    /// answers differently as keywords pile up is a policy whose decision depends
    /// on how many words happened to match — the same keyword-share defect the
    /// whole round exists to repair, reappearing one layer up. So: same class,
    /// any evidence, same outcome.
    #[test]
    fn proptest_outcome_depends_on_the_class_not_the_evidence(
        class_index in 0usize..9,
        low in 0usize..8,
        delta in 1usize..32,
    ) {
        let class = RoutingClass::ALL[class_index];
        let fewer = decide_deferral(class, &DeferralFeatures::new(low, 0.5)).expect("decide");
        let more = decide_deferral(class, &DeferralFeatures::new(low + delta, 0.5)).expect("decide");
        prop_assert_eq!(
            fewer, more,
            "class {:?} decided {:?} at {} keywords but {:?} at {} — the outcome \
             must not depend on how many keywords happened to fire",
            class, fewer, low, more, low + delta
        );
        // And every outcome that reaches here is a human decision, whatever it is.
        prop_assert!(
            more.requires_human(),
            "class {:?} produced {:?}, which takes the case away from a human",
            class, more
        );
    }

    /// A confidence the measurement cannot express must be refused, not carried
    /// into a receipt as a number.
    #[test]
    fn proptest_non_finite_confidence_is_refused(class_index in 0usize..9) {
        let class = RoutingClass::ALL[class_index];
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let features = DeferralFeatures::new(3, bad);
            let result = decide_deferral(class, &features);
            prop_assert!(
                result.is_err(),
                "{:?} accepted a non-finite confidence ({bad})",
                class
            );
        }
    }
}

/// Purity, proven by shape rather than by assertion: the decision is a total
/// function of its two arguments, so the same input cannot produce two
/// different outcomes. Run it repeatedly, interleaved, and demand identity.
///
/// (The strong form — no I/O, no clock — is structural: `decide` takes only
/// `&DeferralFeatures` and returns an enum; there is no place for a clock read
/// or a query to hide.)
#[test]
fn i65_4_decide_is_pure_across_repeated_and_interleaved_calls() {
    let inputs: Vec<(&str, usize, f32)> = vec![
        ("cloud saas api software", 4, 1.0),
        ("the cat sat on the mat", 0, 0.0),
        ("ROI is 3x; budget is $50k.", 3, 0.6),
        ("complaince", 9, 1.0),
    ];
    let first: Vec<DeferralOutcome> = inputs
        .iter()
        .map(|(t, _, _)| {
            let r = classify(t);
            decide_deferral(
                RoutingClass::from_label(r.category),
                &DeferralFeatures::new(r.evidence_count, r.confidence),
            )
            .expect("decide")
        })
        .collect();

    // Interleave and repeat: same inputs, same outputs, every time.
    for _ in 0..3 {
        for (i, (t, _, _)) in inputs.iter().enumerate() {
            let r = classify(t);
            let again = decide_deferral(
                RoutingClass::from_label(r.category),
                &DeferralFeatures::new(r.evidence_count, r.confidence),
            )
            .expect("decide");
            assert_eq!(
                again, first[i],
                "decide is not pure: {t:?} gave {again:?} on a repeat call"
            );
        }
    }
}

/// The outcome vocabulary and its strings are a compatibility surface, frozen
/// the way `DecisionClass::as_str` is — a downstream routing consumer reads
/// these, so adding or renaming one is a wire-visible change.
#[test]
fn i65_4_the_outcome_vocabulary_is_frozen() {
    let labels: Vec<&str> = DeferralOutcome::ALL.iter().map(|o| o.as_str()).collect();
    assert_eq!(
        labels,
        vec!["defer", "clarify", "stop"],
        "the deferral outcome vocabulary is preregistered and frozen"
    );
    // Every outcome is a human decision — the whole point of the type.
    for outcome in DeferralOutcome::ALL {
        assert!(
            outcome.requires_human(),
            "{outcome:?} does not require a human, which contradicts the seam's purpose"
        );
    }
}

/// The real-life path: a knowledge worker's text goes in, and what comes out is
/// something an operator can act on and explain. This is the round-trip a
/// caller actually performs.
#[test]
fn i65_2a_the_real_path_defers_an_ambiguous_case_with_a_reason() {
    // An ambiguous business sentence — the kind that reaches a queue.
    //
    // FIXTURE ISOLATION, ASSERTED NOT ASSUMED (trap #2, hit while writing this
    // file): an earlier draft read "we should probably revisit the onboarding
    // thing at some point" and asserted `general`. It failed — **`onboarding`
    // is a `business_process` lexicon entry**, so the fixture emitted a second
    // signal and the assertion was about the wrong class. Every token is
    // therefore checked against the real classifier below, so the fixture
    // cannot drift back.
    let text = "we should probably revisit that thing at some point";
    let probe = classify(text);
    assert_eq!(
        probe.category, "general",
        "fixture drifted: {:?} fired on {:?}",
        text, probe.matched_keywords
    );
    assert_eq!(
        probe.matched_keywords.len(),
        0,
        "fixture drifted: {:?} is not keyword-free",
        text
    );

    let (class, features, outcome) = deferral_for(text);

    // The abstain class, no evidence — and therefore a deferral.
    assert_eq!(class, RoutingClass::General, "no keyword fired");
    assert_eq!(features.evidence_count, 0);
    assert_eq!(outcome, DeferralOutcome::Defer);

    // And the operator can state WHY in terms a human reads: the class, the
    // evidence count, and the outcome — none of which requires reading the
    // source to reconstruct.
    assert!(!class.as_str().is_empty());
    assert_eq!(class.as_str(), "general");
    assert_eq!(outcome.as_str(), "defer");
}
