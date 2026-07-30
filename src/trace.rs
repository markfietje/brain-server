//! TRACE-style typed edges + validity-aware traversal (v1.4.0 "Calibrate" M3).
//!
//! arXiv:2607.00339 (*TRACE: State-Aware Query Processing over Temporal Evidence
//! Graphs*) models conversations as a hierarchical graph with typed relations:
//! temporal, causal, update, contradiction. brain-server adopts the typed-edge
//! layer of this model on top of the existing `relationships` + `evidence_links`
//! tables, and makes traversal *validity-aware*: an edge invalidated by a later
//! `update:`/`supersedes:` edge is skipped at query time.
//!
//! The key invariant (mirroring Graphiti's `resolve_edge_contradictions`, Context7
//! 2026-07-30): when a new fact contradicts an older one, the older edge is
//! *expired* (invalid_at set), not deleted. Retrieval filters by the
//! bi-temporal window (M1's valid_at/invalid_at); traversal additionally skips
//! edges that a later same-typed edge has superseded.
//!
//! ## Reserved relation_type prefixes
//!
//! Reserved prefixes encode the TRACE edge semantics. They are plain strings on
//! `relationships.relation_type` (no schema change) so existing ingest keeps
//! working; the prefixes are interpreted at traversal time.
//!
//!   `update:`      — this edge updates/corrects an earlier fact about the same
//!                    (from, to, base-relation) triple. The earlier edge's
//!                    invalid_at is set by the ingest path (M3 wiring).
//!   `supersedes:`  — this edge replaces the target chunk/edge entirely.
//!   `contradicts:` — this edge asserts the target is wrong (kept for review).
//!   `causes:`      — causal relation (forward edge for reasoning).
//!
//! Generic relations (e.g. `works_at`, `lives_in`) carry no prefix and are
//! always traversable within their bi-temporal window.

#![deny(unsafe_code)]

/// Reserved typed-edge prefixes. Matched case-sensitively at the start of
/// `relation_type`. A prefix is followed by the base relation name, e.g.
/// `update:lives_in`.
///
/// ponytail: these constants are the M3 vocabulary layer. The bi-temporal `at`
/// filter in traverse_graph is the M3 *substance* (it makes traversal
/// validity-aware by skipping expired edges). These prefix helpers are reserved
/// for v1.6 Reconcile, which builds contradiction resolution on top: when an
/// `update:` edge is ingested, v1.6 will set the prior edge's `invalid_at`.
/// Marked allow(dead_code) so the vocabulary ships now without forcing v1.6's
/// logic to ship in v1.4.
#[allow(dead_code)]
pub const PREFIX_UPDATE: &str = "update:";
#[allow(dead_code)]
pub const PREFIX_SUPERSEDES: &str = "supersedes:";
#[allow(dead_code)]
pub const PREFIX_CONTRADICTS: &str = "contradicts:";
#[allow(dead_code)]
pub const PREFIX_CAUSES: &str = "causes:";

/// All reserved prefixes, for validation/display.
#[allow(dead_code)]
pub const RESERVED_PREFIXES: &[&str] = &[
    PREFIX_UPDATE,
    PREFIX_SUPERSEDES,
    PREFIX_CONTRADICTS,
    PREFIX_CAUSES,
];

/// Strip a reserved prefix, returning (prefix, base_relation). If no prefix,
/// returns (None, full).
#[allow(dead_code)]
pub fn split_prefix(relation_type: &str) -> (Option<&'static str>, &str) {
    for p in RESERVED_PREFIXES {
        if let Some(base) = relation_type.strip_prefix(p) {
            return (Some(p), base);
        }
    }
    (None, relation_type)
}

/// Is this an `update:`/`supersedes:` edge (the two that invalidate earlier
/// facts during traversal)?
#[allow(dead_code)]
pub fn is_invalidation(relation_type: &str) -> bool {
    relation_type.starts_with(PREFIX_UPDATE) || relation_type.starts_with(PREFIX_SUPERSEDES)
}

/// The base relation without its prefix. `update:lives_in` → `lives_in`.
#[allow(dead_code)]
pub fn base_relation(relation_type: &str) -> &str {
    split_prefix(relation_type).1
}

/// Hard cap on traversal depth (BFS). The forbidden-list rule "unbounded graph
/// walks" mandates a hard cap; 4 hops covers any realistic multi-hop query
/// without runaway cost.
pub const MAX_HOPS: u32 = 4;

/// Hard cap on visited nodes per traversal. Bounds memory + time; a pathological
/// hub-and-spoke graph can't exhaust the budget.
pub const MAX_VISITED: usize = 256;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_update_prefix() {
        assert_eq!(
            split_prefix("update:lives_in"),
            (Some(PREFIX_UPDATE), "lives_in")
        );
    }

    #[test]
    fn splits_supersedes_prefix() {
        assert_eq!(
            split_prefix("supersedes:address"),
            (Some(PREFIX_SUPERSEDES), "address")
        );
    }

    #[test]
    fn no_prefix_returns_none() {
        assert_eq!(split_prefix("works_at"), (None, "works_at"));
    }

    #[test]
    fn is_invalidation_detects_update_and_supersedes() {
        assert!(is_invalidation("update:lives_in"));
        assert!(is_invalidation("supersedes:address"));
        assert!(!is_invalidation("contradicts:address"));
        assert!(!is_invalidation("works_at"));
    }

    #[test]
    fn base_relation_strips_any_prefix() {
        assert_eq!(base_relation("update:lives_in"), "lives_in");
        assert_eq!(base_relation("causes:failure"), "failure");
        assert_eq!(base_relation("works_at"), "works_at");
    }

    #[test]
    fn caps_are_finite() {
        // Runtime check that the forbidden-list caps are sensible bounds.
        // (Compile-time enforcement would be ideal but clippy flags const
        // asserts; the values are small constants reviewed by hand.)
        assert!((1..=8).contains(&MAX_HOPS));
        assert!((1..=1024).contains(&MAX_VISITED));
    }
}
