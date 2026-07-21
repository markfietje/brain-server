//! v0.9.1 Milestone 5 — Recall quality eval.
//!
//! Measures recall@5 across the search configurations to prove each lever
//! (hybrid, PRF) adds marginal lift. The metric itself is unit-tested here;
//! the full model-backed eval runs against a live server (see BENCHMARKS.md
//! for recorded numbers).

#![cfg(test)]

// Eval document set — 10 topically distinct notes. Used by the manual eval
// harness (ingest these, then query and measure recall@5).
#[allow(dead_code)]
const DOCS: &[&str] = &[
    "Bignay is a tropical fruit and a good alternative to blueberry, rich in antioxidants.",
    "The Rust programming language guarantees memory safety without a garbage collector.",
    "Vitamin D3 supplementation improves immune function and bone density in deficient adults.",
    "The GDPR is a European regulation protecting the personal data of EU residents.",
    "Gut microbiome diversity affects inflammation markers and immune system regulation.",
    "SQLite is an embedded relational database with FTS5 full-text search support.",
    "ISO 9001 is the international standard for quality management systems.",
    "Ownership and borrowing are Rust's core concepts for compile-time memory safety.",
    "Antioxidants in tropical fruits like bignay help reduce oxidative stress.",
    "The GDPR covers any organization processing EU residents' data, with fines up to four percent of global revenue.",
];

// Eval queries — each maps to the indices of relevant docs (0-based).
// A query passes recall@5 if ALL relevant docs appear in the top 5.
#[allow(dead_code)]
const QUERIES: &[(&str, &[usize])] = &[
    ("blueberry alternative fruit", &[0, 8]),
    ("memory safe programming language", &[1, 7]),
    ("vitamin supplements immune health", &[2]),
    ("EU data protection regulation", &[3, 9]),
    ("gut inflammation microbiome", &[4]),
    ("embedded database search", &[5]),
    ("quality management standard", &[6]),
    ("GDPR organization coverage", &[3, 9]),
    ("antioxidants tropical fruit stress", &[0, 8]),
    ("Rust ownership borrowing", &[1, 7]),
];

/// recall@k: fraction of relevant docs found in the top-k results.
fn recall_at_k(results: &[i64], relevant: &[usize], k: usize) -> f32 {
    if relevant.is_empty() {
        return 1.0;
    }
    let top_k: std::collections::HashSet<i64> = results.iter().take(k).copied().collect();
    let found = relevant
        .iter()
        .filter(|&&r| top_k.contains(&(r as i64)))
        .count();
    found as f32 / relevant.len() as f32
}

#[test]
fn test_recall_at_k_metric() {
    // Pure unit test of the recall metric — no model needed.
    let results = vec![0, 3, 5, 1, 7, 2, 9]; // ids returned in rank order
                                             // relevant = [0, 8]. 0 is in top-5 (rank 0); 8 is not in results at all.
    let r5 = recall_at_k(&results, &[0, 8], 5);
    assert!(
        (r5 - 0.5).abs() < 1e-6,
        "1 of 2 relevant in top-5 -> 0.5, got {r5}"
    );

    // All relevant in top-k
    let r = recall_at_k(&[1, 7, 0], &[1, 7], 5);
    assert!((r - 1.0).abs() < 1e-6, "both relevant in top-5 -> 1.0");

    // None relevant
    let r = recall_at_k(&[2, 4, 6], &[0, 8], 5);
    assert!((r - 0.0).abs() < 1e-6, "no relevant -> 0.0");

    // Empty relevant set → perfect recall by convention
    let r = recall_at_k(&[1, 2], &[], 5);
    assert!((r - 1.0).abs() < 1e-6, "empty relevant -> 1.0");
}

#[test]
fn test_eval_query_coverage() {
    // Sanity: every query has at least one relevant doc, and all doc indices
    // are within the DOCS set bounds.
    for (q, relevant) in QUERIES {
        assert!(!relevant.is_empty(), "query '{q}' has no relevant docs");
        for &idx in *relevant {
            assert!(
                idx < DOCS.len(),
                "query '{q}' references out-of-bounds doc {idx}"
            );
        }
    }
}
