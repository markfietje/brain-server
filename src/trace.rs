//! TRACE-style typed edges + validity-aware traversal.
//!
//! Models conversations as a hierarchical graph with typed relations (temporal,
//! causal, update, contradiction) over the existing `relationships` +
//! `evidence_links` tables. Traversal is validity-aware: an edge invalidated by
//! a later `update:`/`supersedes:` edge is skipped at query time.
//!
//! Key invariant (mirroring Graphiti's `resolve_edge_contradictions`): when a
//! new fact contradicts an older one, the older edge is expired (invalid_at
//! set), not deleted. Retrieval filters by the bi-temporal window; traversal
//! additionally skips superseded same-typed edges.
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
