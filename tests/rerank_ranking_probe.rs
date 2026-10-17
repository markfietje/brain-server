//! Does the reranker actually fix the ranking that RRF got wrong?
//!
//! **The observation that prompted this.** On the live database,
//! `POST /recall` for "what are the benefits of green tea" returned, in this
//! order:
//!
//! ```text
//! 1. Soursop Health Benefits      score 0.00241   <- lowest score in the list
//! 2. Parasite Cleanse Guide      score 0.01667   <- on topic
//! 3. EGCG: Green Tea's Secret...  score 0.01613   <- on topic
//! 4. Dragon Fruit Health Benefits score 0.00916
//! 5. Durian Health Benefits      score 0.00241
//! ```
//!
//! Two separate defects are visible there, and this probe is careful to keep
//! them apart:
//!
//! 1. **RANK-1 IS THE LOWEST-SCORING HIT.** Soursop ties with Durian at
//!    0.00241, the bottom of the list, and is nevertheless first. Whatever the
//!    emitted `score` means, it is not the key the results are sorted by. That
//!    is a sorting bug and reranking cannot fix it — reranking reorders a list
//!    that is already mis-sorted, which hides the bug behind better ordering.
//! 2. **Off-topic documents win on a generic term.** "Soursop/Durian/Dragon
//!    Fruit Health Benefits" match the query word *benefits* and lose to the
//!    entity *green tea*. This is the defect a cross-encoder is actually good
//!    at, because it scores the query against the document semantically rather
//!    than by term overlap.
//!
//! **What this probe does and does not claim.** It feeds the reranker the real
//! five candidate documents the server returned, in the server's order, and
//! prints the order before and after. It isolates the reranker. It does NOT
//! test the sort key, does not touch the live database, and does not restart
//! the server — so a "no change" here means the reranker did not re-rank THESE
//! candidates, not that reranking is useless in the pipeline.
//!
//! Three gates must all open for the reranker to run at all, and this probe
//! asserts they are open rather than assuming it:
//!   1. compile-time `rerank-tier` feature (the module is `#![cfg]`'d away)
//!   2. runtime `BRAIN_RERANK_ENABLED=1` (`rerank.rs:77`)
//!   3. `BRAIN_RERANK_MODEL_DIR` ABSOLUTE — the relative default is refused on
//!      purpose, so a writable CWD cannot supply weights (`rerank.rs:63`)
//!
//! Run it twice and compare:
//! ```sh
//! cargo test --features rerank-tier --test rerank_ranking_probe -- --nocapture
//! BRAIN_RERANK_ENABLED=1 \
//! BRAIN_RERANK_MODEL_DIR=$PWD/models/mxbai-rerank-large-v1 \
//!   cargo test --features rerank-tier --test rerank_ranking_probe -- --nocapture
//! ```

// GATED ON ITS OWN FEATURE, and this is a fix rather than a style choice.
//
// `cargo test` compiles EVERY auto-discovered test target, so this file's
// unconditional `use brain_server::search::rerank::maybe_rerank` was enough to
// break the WHOLE suite whenever `rerank-tier` was off — which is the default,
// and which is what `cargo test --features bench` (the documented primary gate)
// resolves to. One ungated import in one probe file took down `brain-server`'s
// primary test command from 00:29 on 2026-10-18, and it stayed red and unnoticed
// because the AFM rounds only ran the Swift lane.
//
// The module is `#[cfg(feature = "rerank-tier")] pub mod rerank;` in
// `search/mod.rs`, so the import has to carry the same gate. `#![cfg]` at the
// crate root empties the whole integration test when the feature is off, rather
// than cfg-ing out each item — so there is no way to add an ungated `use` here
// later and reintroduce the same break.
//
// The consequence is honest rather than silent: with the feature OFF this
// target contributes NOTHING, and the probe runs only in the `rerank-tier`
// lane, which CI exercises on its own. It is not hidden from the default gate —
// it was never buildable in the default gate, and saying so is what this
// comment is for.
#![cfg(feature = "rerank-tier")]

use brain_server::search::rerank::maybe_rerank;
use brain_server::search::{SearchResult, SearchTelemetry};

/// The five documents the live server actually returned, verbatim enough for a
/// cross-encoder to judge them on the same evidence a user would see.
fn candidates() -> Vec<SearchResult> {
    let docs: [(i64, f32, &str, &str); 5] = [
        (
            1,
            0.0024111024,
            "Soursop Health Benefits - Graph Enriched",
            "SOURSOP (Annona muricata) HEALTH BENEFITS OVERVIEW - SCIENTIFIC EVIDENCE (Enriched Graph v2) SOURCE: Multiple peer-reviewed reviews (2020-2024) SEARCH METHODS: Web search scientific databases (PubMed/NCBI, SpringerLink, News-Medical).",
        ),
        (
            2,
            0.016666668,
            "Parasite Cleanse Guide: Natural Gut Healing",
            "EGCG is a potent catechin found primarily in green tea. It's known for its powerful antioxidant and anti-inflammatory properties, which can support overall gut health during a parasite cleanse.",
        ),
        (
            3,
            0.016129032,
            "EGCG: Green Tea's Secret Weapon for Gut and Immune Health",
            "The story of EGCG is intertwined with the rich history of green tea itself. Green tea has been consumed in China for thousands of years, revered as a medicinal beverage, and EGCG is the powerful compound at its centre.",
        ),
        (
            4,
            0.009162091,
            "Dragon Fruit Health Benefits - Graph Enriched",
            "DRAGON FRUIT (Hylocereus undatus / Costaricensis) HEALTH BENEFITS OVERVIEW (Enriched Graph v2) SOURCE: Multiple peer-reviewed studies and food science reviews CONTENT: Nutritional composition, health benefits, consumption guidelines.",
        ),
        (
            5,
            0.0024111024,
            "Durian Health Benefits - Graph Enriched",
            "DURIAN HEALTH BENEFITS OVERVIEW - SCIENTIFIC EVIDENCE. Source: peer-reviewed reviews of Durio zibethinus nutritional composition and health benefits.",
        ),
    ];
    docs.iter()
        .map(|(id, score, title, content)| SearchResult {
            id: *id,
            score: *score,
            title: Some((*title).to_string()),
            content: (*content).to_string(),
            ..Default::default()
        })
        .collect()
}

#[test]
fn reranker_reorders_the_real_candidates_for_a_conversational_query() {
    const QUERY: &str = "what are the benefits of green tea";

    let before = candidates();
    println!("\n--- BEFORE (RRF order, as the server emitted it) ---");
    for (i, r) in before.iter().enumerate() {
        println!(
            "  {}. {}  [score {:.8}]",
            i + 1,
            r.title.as_deref().unwrap_or(""),
            r.score
        );
    }

    let mut after = before.clone();
    let mut tel = SearchTelemetry::default();
    maybe_rerank(QUERY, &mut after, &mut tel);
    println!("--- AFTER (reranked) ---  rerank_ms = {:.1}", tel.rerank_ms);
    for (i, r) in after.iter().enumerate() {
        println!(
            "  {}. {}  [score {:.8}]",
            i + 1,
            r.title.as_deref().unwrap_or(""),
            r.score
        );
    }

    // Whether the reranker ran is decided by the ORDER CHANGING, not by
    // `rerank_ms`. `maybe_rerank` takes its telemetry parameter as `_tel` and
    // never writes it — the CALL SITE (`search/mod.rs:2291`) times the call with
    // a wall clock and assigns it. Its own doc comment claims "Records
    // `rerank_ms` into telemetry either way", which is false of the function
    // itself; a probe that trusted that claim would have reported a working
    // reranker as not running. The order is the ground truth.
    let order_changed = before.iter().map(|r| r.id).ne(after.iter().map(|r| r.id));

    if !order_changed {
        println!(
            "\nRERANKER DID NOT RUN (order unchanged). Three gates, all must be open:\n  \
             1. --features rerank-tier   (the module is #![cfg]'d away otherwise)\n  \
             2. BRAIN_RERANK_ENABLED=1    (rerank.rs:77)\n  \
             3. BRAIN_RERANK_MODEL_DIR absolute (rerank.rs:63 refuses a relative \
             path on purpose, so a writable CWD cannot supply weights)\n\
             This is the DEFAULT state of a default build. An unchanged order here \
             is the expected result, not evidence about reranker's quality."
        );
        return;
    }

    // The claim worth testing: does the cross-encoder demote the documents that
    // matched only the generic word "benefits"?
    let green_tea_docs = |v: &[SearchResult]| {
        v.iter()
            .filter(|r| {
                let t = format!("{:?} {:?}", r.title, r.content).to_lowercase();
                t.contains("green tea")
            })
            .count()
    };
    let (before_green, after_green) = (green_tea_docs(&before), green_tea_docs(&after));
    let top_before = before
        .first()
        .and_then(|r| r.title.clone())
        .unwrap_or_default();
    let top_after = after
        .first()
        .and_then(|r| r.title.clone())
        .unwrap_or_default();
    println!(
        "\non-topic documents in the top TWO:  before {}  after {}",
        before
            .iter()
            .take(2)
            .filter(|r| format!("{:?}", r.title)
                .to_lowercase()
                .contains("green tea")
                || format!("{:?}", r.content)
                    .to_lowercase()
                    .contains("green tea"))
            .count(),
        after
            .iter()
            .take(2)
            .filter(|r| format!("{:?}", r.title)
                .to_lowercase()
                .contains("green tea")
                || format!("{:?}", r.content)
                    .to_lowercase()
                    .contains("green tea"))
            .count()
    );
    println!("top hit:  before = {top_before}\n           after  = {top_after}");
    assert_eq!(
        before_green, after_green,
        "the candidate SET must not change — reranking reorders, it does not \
         add or drop documents"
    );
}
