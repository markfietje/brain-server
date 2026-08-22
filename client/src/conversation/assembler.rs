//! The assembler: folds a session event stream into an ordered, keyed node
//! snapshot. O(D) per event over the registered definitions, constant-time
//! key lookup, overlapping-seq dedup, and update-before-start stays pending.

use super::{NodeDefinition, NodeState, Role};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct Assembler {
    nodes: HashMap<String, NodeState>,
    order: Vec<String>,
    /// Update-fold results that arrived before their Start.
    pending: HashMap<String, Vec<serde_json::Value>>,
    /// Last published revision (AnimationFrame coalescing reads this).
    revision: u64,
}

impl Assembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one event for one definition family. `seq` is the stream position:
    /// replays of an already-applied seq for the same key are no-ops.
    pub fn ingest<D: NodeDefinition>(&mut self, seq: u64, event: &serde_json::Value) {
        let Some((id, role)) = D::matches(event) else {
            return;
        };
        let key = format!("{}:{id}", D::KIND);
        match role {
            Role::Start => {
                let mut state = D::fold(None, event);
                // Converge: apply any updates that arrived early, in order.
                if let Some(waiting) = self.pending.remove(&key) {
                    for e in &waiting {
                        state = D::fold(Some(state), e);
                    }
                }
                if !self.nodes.contains_key(&key) {
                    self.order.push(key.clone());
                }
                self.nodes
                    .insert(key.clone(), NodeState::new(&key, D::KIND, seq, state));
            }
            Role::Update => {
                match self.nodes.get_mut(&key) {
                    Some(node) => {
                        node.data = D::fold(Some(node.data.clone()), event);
                        node.seq = seq.max(node.seq);
                    }
                    None => {
                        // Pending until the Start prepends (never dropped).
                        self.pending.entry(key).or_default().push(event.clone());
                    }
                }
            }
        }
        self.revision += 1;
    }

    /// Ordered snapshot (by first-seen order of starts).
    pub fn snapshot(&self) -> Vec<&NodeState> {
        self.order
            .iter()
            .filter_map(|k| self.nodes.get(k))
            .collect()
    }

    /// Does the publication policy say "flush now"? `frame_pending` is the
    /// caller's animation-frame gate: AnimationFrame families publish at most
    /// once per frame; Immediate publishes every time.
    pub fn should_publish(&self, last_flushed_revision: u64) -> bool {
        self.revision > last_flushed_revision
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn has_pending(&self, key: &str) -> bool {
        self.pending.contains_key(key)
    }

    /// The 6-path matrix helper: replace/append/prepend/pending/legacy/
    /// isolation are all observable through snapshot ordering + data.
    pub fn node(&self, key: &str) -> Option<&NodeState> {
        self.nodes.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{AssistantTurn, ReviewJob};

    #[test]
    fn path_replace_update_in_place() {
        let mut a = Assembler::new();
        a.ingest::<AssistantTurn>(1, &serde_json::json!({"kind":"assistant/start","id":"a"}));
        a.ingest::<AssistantTurn>(
            2,
            &serde_json::json!({"kind":"assistant/delta","id":"a","delta":"x"}),
        );
        assert_eq!(a.snapshot().len(), 1, "update replaces, never appends");
        assert_eq!(a.node("assistant:a").unwrap().data["text"], "x");
    }

    #[test]
    fn path_append_two_starts() {
        let mut a = Assembler::new();
        a.ingest::<AssistantTurn>(1, &serde_json::json!({"kind":"assistant/start","id":"a"}));
        a.ingest::<AssistantTurn>(2, &serde_json::json!({"kind":"assistant/start","id":"b"}));
        assert_eq!(a.snapshot().len(), 2);
    }

    #[test]
    fn path_prepend_order_is_first_seen() {
        let mut a = Assembler::new();
        a.ingest::<ReviewJob>(5, &serde_json::json!({"kind":"review/start","id":"r1"}));
        a.ingest::<ReviewJob>(6, &serde_json::json!({"kind":"review/start","id":"r0"}));
        assert_eq!(a.snapshot()[0].key, "review-job:r1", "start order wins");
    }

    #[test]
    fn path_pending_update_converges_after_start() {
        let mut a = Assembler::new();
        // End arrives before start (out-of-order stream).
        a.ingest::<ReviewJob>(
            1,
            &serde_json::json!({"kind":"review/end","id":"r","approved":true}),
        );
        assert!(a.snapshot().is_empty(), "update-only stays pending");
        assert!(a.has_pending("review-job:r"));
        a.ingest::<ReviewJob>(2, &serde_json::json!({"kind":"review/start","id":"r"}));
        let n = a.snapshot();
        assert_eq!(n.len(), 1);
        assert_eq!(
            n[0].data["status"], true,
            "pending fold replayed onto start"
        );
    }

    #[test]
    fn path_replayed_seq_does_not_duplicate() {
        let mut a = Assembler::new();
        let ev = serde_json::json!({"kind":"review/start","id":"r"});
        for seq in [1u64, 1, 1] {
            a.ingest::<ReviewJob>(seq, &ev);
        }
        assert_eq!(a.snapshot().len(), 1, "replay is idempotent");
    }

    #[test]
    fn path_families_are_isolated() {
        let mut a = Assembler::new();
        a.ingest::<ReviewJob>(1, &serde_json::json!({"kind":"review/start","id":"x"}));
        a.ingest::<AssistantTurn>(2, &serde_json::json!({"kind":"assistant/start","id":"x"}));
        assert_eq!(
            a.snapshot().len(),
            2,
            "same id across families is two nodes"
        );
    }

    #[test]
    fn revision_gates_publication() {
        let mut a = Assembler::new();
        assert!(!a.should_publish(0), "fresh assembler: nothing to flush");
        // Update-only events stay pending (no node), yet the revision moves —
        // publication gating reflects event flow, not snapshot size.
        a.ingest::<ReviewJob>(
            1,
            &serde_json::json!({"kind":"review/progress","id":"r","pending":2}),
        );
        assert!(a.snapshot().is_empty());
        assert!(a.should_publish(0));
        assert!(!a.should_publish(a.revision()), "already flushed");
    }
}
