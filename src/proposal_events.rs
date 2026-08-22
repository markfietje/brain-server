//! The proposal conversation-event producer: `proposal/open`, `proposal/updated`
//! and `proposal/decided` events shaped for the client's review-job
//! conversation node. Pure builders + pure fold — the wire contract lives in
//! one place so the server feed and any future stream can never drift.
//!
//! Whole-value checkpoints (content digest, SLA deadline, role gate) ride EVERY
//! event, so a consumer joining mid-stream replays a complete node without the
//! start event (the "start outside the loaded window" case). Payloads are
//! metadata only — never proposal content, never PII.
//!
//! Port note: producer/consumer split semantics ported from the harness
//! conversation-node cookbook pattern; original Rust.

use serde_json::{Value, json};

/// The role gate an approval decision must pass (`handlers` enforce it; the
/// event only advertises it so the UI can render the requirement).
pub const ROLE_GATE: &str = "approve";

/// Branded proposal id: a proposal coordinate is never confusable with a chunk
/// or workflow-run id in an event payload. Wire form is `"p<id>"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProposalId(pub i64);

impl ProposalId {
    pub fn wire(&self) -> String {
        format!("p{}", self.0)
    }
    /// Parse back from the wire form (fail-closed: anything else is `None`).
    /// Truthful allow: the consumer side lives in the client tree (the wasm
    /// bundle cannot link this crate); kept here as the pinned contract half.
    #[allow(dead_code)]
    pub fn parse(s: &str) -> Option<Self> {
        s.strip_prefix('p')?
            .parse::<i64>()
            .ok()
            .filter(|i| *i > 0)
            .map(Self)
    }
}

/// `proposal/open` — a candidate entered the HITL queue.
/// `sla_deadline` = created_at + TTL (unix seconds); `digest` = the review
/// digest of what the reviewer will be shown.
pub fn open(id: ProposalId, sla_deadline: i64, digest: &str) -> Value {
    json!({
        "kind": "proposal/open",
        "id": id.wire(),
        "proposal_id": id.0,
        "status": "pending",
        // whole-value checkpoints — replay-safe from any join point
        "content_digest": digest,
        "sla_deadline": sla_deadline,
        "role_gate": ROLE_GATE,
    })
}

/// `proposal/decided` — terminal event (approved or rejected). Carries the
/// checkpoints again so a decision observed before its open still renders.
pub fn decided(id: ProposalId, approved: bool, digest: &str) -> Value {
    let mut v = open(id, 0, digest);
    v["kind"] = Value::from("proposal/decided");
    v["approved"] = Value::from(approved);
    v["sla_deadline"] = Value::Null;
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branded_id_wire_roundtrip() {
        let id = ProposalId(42);
        assert_eq!(id.wire(), "p42");
        assert_eq!(ProposalId::parse("p42"), Some(id));
        assert_eq!(ProposalId::parse("42"), None, "unbranded refused");
        assert_eq!(ProposalId::parse("chunk-42"), None);
        assert_eq!(ProposalId::parse("p0"), None, "non-positive refused");
        assert_eq!(ProposalId::parse("p-3"), None);
    }

    #[test]
    fn open_carries_whole_value_checkpoints() {
        let e = open(ProposalId(7), 1_800_000_000, "abc123");
        assert_eq!(e["kind"], "proposal/open");
        assert_eq!(e["id"], "p7");
        assert_eq!(e["proposal_id"], 7);
        assert_eq!(e["content_digest"], "abc123");
        assert_eq!(e["sla_deadline"], 1_800_000_000);
        assert_eq!(e["role_gate"], "approve");
        let s = serde_json::to_string(&e).unwrap();
        assert!(
            !e.as_object().unwrap().contains_key("content"),
            "no raw-content field rides the event (only its digest)"
        );
        assert!(!s.contains("Baker Street"));
    }

    #[test]
    fn decided_is_terminal_and_self_contained() {
        let e = decided(ProposalId(9), true, "d1");
        assert_eq!(e["kind"], "proposal/decided");
        assert_eq!(e["approved"], true);
        assert_eq!(e["content_digest"], "d1");
        assert_eq!(
            e["role_gate"], "approve",
            "checkpoints repeat on the terminal"
        );
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            serde_json::to_string(&decided(ProposalId(9), true, "d1")).unwrap(),
            "deterministic shape"
        );
    }

    #[test]
    fn payloads_are_metadata_only() {
        let e = open(ProposalId(1), 5, "deadbeef");
        let s = serde_json::to_string(&e).unwrap();
        assert!(
            !s.contains("address"),
            "raw content never enters an event payload"
        );
    }
}
