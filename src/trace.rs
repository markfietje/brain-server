//! TRACE-style typed edges + validity-aware traversal.
//!
//! Models conversations as a hierarchical graph with typed relations over the
//! existing `relationships` + `evidence_links` tables. Traversal is
//! validity-aware and current-belief-aware: an edge whose
//! `superseded_at` is set (a corrected belief has replaced it, transaction-time
//! END per the bi-temporal model) is skipped at query time.
//!
//! Key invariant (mirroring Graphiti's `resolve_edge_contradictions`): when a
//! new fact corrects an older one, the older edge is retired (`superseded_at`
//! set), never deleted — its valid interval + created_at are preserved for
//! historical/as-of reads. Traversal filters by the bi-temporal window AND
//! skips superseded same-typed edges, so a corrected belief yields one live
//! edge per triple.
//!
//! The reserved `update:`/`supersedes:`/`contradicts:`/`causes:` vocabulary was
//! removed (v1.6 closed without consuming it). What remains: the *used*
//! surface, the traversal caps `MAX_HOPS`/`MAX_VISITED`.

#![deny(unsafe_code)]

/// Hard cap on traversal depth (BFS). The forbidden-list rule "unbounded graph
/// walks" mandates it; 4 hops covers any realistic multi-hop query.
pub const MAX_HOPS: u32 = 4;

/// Hard cap on visited nodes per traversal. Bounds memory + time; a pathological
/// hub-and-spoke graph can't exhaust the budget.
pub const MAX_VISITED: usize = 256;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_are_finite() {
        // Sense-check the forbidden-list caps (clippy flags const asserts, so
        // these small constants are reviewed by hand rather than compile-time).
        assert!((1..=8).contains(&MAX_HOPS));
        assert!((1..=1024).contains(&MAX_VISITED));
    }
}
