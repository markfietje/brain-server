//! TRACE-style typed edges + validity-aware traversal.
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
//! the reserved `update:`/`supersedes:`/`contradicts:`/
//! `causes:` prefix vocabulary shipped here as `#[allow(dead_code)]` "reserved
//! for v1.6" — v1.6 shipped and closed without consuming it, so the dead
//! vocabulary and its tests are gone. What remains is the *used* surface:
//! `MAX_HOPS`/`MAX_VISITED`, the traversal caps the graph walk enforces.

#![deny(unsafe_code)]

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
    fn caps_are_finite() {
        // Runtime check that the forbidden-list caps are sensible bounds.
        // (Compile-time enforcement would be ideal but clippy flags const
        // asserts; the values are small constants reviewed by hand.)
        assert!((1..=8).contains(&MAX_HOPS));
        assert!((1..=1024).contains(&MAX_VISITED));
    }
}
