//! Conversation-node assembly: chat is a registry of node definitions, not a
//! closed union. Each definition matches session events to a node identity and
//! folds them into per-node state; the assembler replays matches in sequence
//! order and publishes snapshots coalesced by publication policy.
//!
//! Semantics (ported from the harness conversation-runtime pattern; original
//! Rust):
//! - `match(event)` returns `(node_id, Start|Update)`. An Update for an
//!   unknown id stays pending until the Start arrives (out-of-order streams
//!   converge).
//! - Overlapping sequences dedup: a replayed event never duplicates a node.
//! - `Publication::AnimationFrame` coalesces at most one snapshot per frame;
//!   `Immediate` flushes on every event (terminal nodes).
//!
//! Truthful allow: the engine is wired to its slot registry and definitions,
//! but brain's client is request/response today (no live session stream yet) —
//! the runtime event source lands with the streaming surface. The pure core
//! ships tested now so the ABI is stable when it does.

#![allow(dead_code)]

pub mod assembler;
pub mod event_registry;

use serde_json::Value;

/// Which event kinds feed which node kind — the built-in set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Publication {
    Immediate,
    AnimationFrame,
    None,
}

/// The role a matched event plays for its node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Start,
    Update,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeState {
    pub key: String,
    pub kind: &'static str,
    pub seq: u64,
    pub data: Value,
}

impl NodeState {
    fn new(key: String, kind: &'static str, seq: u64, data: Value) -> Self {
        Self {
            key,
            kind,
            seq,
            data,
        }
    }
}

/// A node definition: match + fold. `S` is the owner's state type ("who injects
/// it, owns its type").
pub trait NodeDefinition {
    /// The node family this definition builds (the slot key).
    const KIND: &'static str;
    /// Publication policy for this family.
    const PUBLICATION: Publication;
    /// Does `event` belong to this family? Returns the node identity + role.
    fn matches(event: &Value) -> Option<(String, Role)>;
    /// Fold an event into state. `prev` is `None` for Start.
    fn fold(prev: Option<Value>, event: &Value) -> Value;
}

// ---------------------------------------------------------------------------
// Built-in definitions. Each maps harness-shaped events onto chat nodes.
// ---------------------------------------------------------------------------

/// Assistant streaming → settled (`assistant/delta`, `assistant/end`).
pub struct AssistantNode;
impl NodeDefinition for AssistantNode {
    const KIND: &'static str = "assistant";
    const PUBLICATION: Publication = Publication::AnimationFrame;
    fn matches(event: &Value) -> Option<(String, Role)> {
        let kind = event.get("kind")?.as_str()?;
        let id = event.get("id")?.as_str()?.to_string();
        match kind {
            "assistant/start" => Some((id, Role::Start)),
            "assistant/delta" | "assistant/end" => Some((id, Role::Update)),
            _ => None,
        }
    }
    fn fold(prev: Option<Value>, event: &Value) -> Value {
        let mut s = prev.unwrap_or_else(|| serde_json::json!({"text": "", "settled": false}));
        match event.get("kind").and_then(Value::as_str) {
            Some("assistant/delta") => {
                if let Some(d) = event.get("delta").and_then(Value::as_str) {
                    let mut text = s
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    text.push_str(d);
                    s["text"] = Value::from(text);
                }
            }
            Some("assistant/start" | "assistant/end") => {
                if let Some(d) = event.get("text").and_then(Value::as_str) {
                    s["text"] = Value::from(d);
                }
                if event.get("kind").and_then(Value::as_str) == Some("assistant/end") {
                    s["settled"] = Value::from(true);
                }
            }
            _ => {}
        }
        s
    }
}

/// Tool running → settled (`tool/start`, `tool/result`).
pub struct ToolNode;
impl NodeDefinition for ToolNode {
    const KIND: &'static str = "tool";
    const PUBLICATION: Publication = Publication::AnimationFrame;
    fn matches(event: &Value) -> Option<(String, Role)> {
        let kind = event.get("kind")?.as_str()?;
        let id = event.get("id")?.as_str()?.to_string();
        match kind {
            "tool/start" => Some((id, Role::Start)),
            "tool/result" => Some((id, Role::Update)),
            _ => None,
        }
    }
    fn fold(prev: Option<Value>, event: &Value) -> Value {
        match (prev, event.get("kind").and_then(Value::as_str)) {
            (None, _) => serde_json::json!({
                "name": event.get("name").cloned().unwrap_or(Value::Null),
                "status": "running",
            }),
            (Some(mut s), Some("tool/result")) => {
                s["status"] = Value::from(
                    if event.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                        "settled"
                    } else {
                        "error"
                    },
                );
                if let Some(out) = event.get("output") {
                    s["output"] = out.clone();
                }
                s
            }
            (Some(s), _) => s,
        }
    }
}

/// Review job (brain truth): `review/start|progress|end`.
pub struct ReviewJobNode;
impl NodeDefinition for ReviewJobNode {
    const KIND: &'static str = "review-job";
    const PUBLICATION: Publication = Publication::Immediate;
    fn matches(event: &Value) -> Option<(String, Role)> {
        let kind = event.get("kind")?.as_str()?;
        let id = event.get("id")?.as_str()?.to_string();
        match kind {
            "review/start" => Some((id, Role::Start)),
            "review/progress" | "review/end" => Some((id, Role::Update)),
            _ => None,
        }
    }
    fn fold(prev: Option<Value>, event: &Value) -> Value {
        let mut s =
            prev.unwrap_or_else(|| serde_json::json!({"status": "running", "proposal_id": null}));
        if let Some(p) = event.get("proposal_id") {
            s["proposal_id"] = p.clone();
        }
        match event.get("kind").and_then(Value::as_str) {
            Some("review/progress") => {
                if let Some(n) = event.get("pending") {
                    s["pending"] = n.clone();
                }
            }
            Some("review/end") => {
                s["status"] = Value::from(
                    event
                        .get("approved")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                );
            }
            _ => {}
        }
        s
    }
}

/// Delivery: `tool-workflow/*` + `agent/*` collapsed into one artifact node.
pub struct DeliveryNode;
impl NodeDefinition for DeliveryNode {
    const KIND: &'static str = "delivery";
    const PUBLICATION: Publication = Publication::AnimationFrame;
    fn matches(event: &Value) -> Option<(String, Role)> {
        let kind = event.get("kind")?.as_str()?;
        let id = event.get("id")?.as_str()?.to_string();
        match kind {
            "delivery/start" => Some((id, Role::Start)),
            "delivery/item" | "delivery/end" => Some((id, Role::Update)),
            _ => None,
        }
    }
    fn fold(prev: Option<Value>, event: &Value) -> Value {
        let mut s = prev.unwrap_or_else(|| serde_json::json!({"items": [], "done": false}));
        match event.get("kind").and_then(Value::as_str) {
            Some("delivery/item") => {
                if let Some(item) = event.get("item") {
                    if let Some(arr) = s.get_mut("items").and_then(Value::as_array_mut) {
                        arr.push(item.clone());
                    }
                }
            }
            Some("delivery/end") => s["done"] = Value::from(true),
            _ => {}
        }
        s
    }
}

/// Workflow run: `workflow/*` events (start/phase/end).
pub struct WorkflowRunNode;
impl NodeDefinition for WorkflowRunNode {
    const KIND: &'static str = "workflow-run";
    const PUBLICATION: Publication = Publication::Immediate;
    fn matches(event: &Value) -> Option<(String, Role)> {
        let kind = event.get("kind")?.as_str()?;
        let id = event
            .get("run_id")
            .or_else(|| event.get("id"))?
            .as_str()?
            .to_string();
        match kind {
            k if k.starts_with("workflow/") => Some((
                id,
                if k == "workflow/start" {
                    Role::Start
                } else {
                    Role::Update
                },
            )),
            _ => None,
        }
    }
    fn fold(prev: Option<Value>, event: &Value) -> Value {
        let mut s = prev.unwrap_or_else(|| serde_json::json!({"state": "running", "phases": []}));
        match event.get("kind").and_then(Value::as_str) {
            Some("workflow/phase") => {
                if let Some(p) = event.get("phase") {
                    if let Some(arr) = s.get_mut("phases").and_then(Value::as_array_mut) {
                        arr.push(p.clone());
                    }
                }
            }
            Some("workflow/end") => {
                s["state"] = Value::from(
                    event
                        .get("outcome")
                        .cloned()
                        .unwrap_or(Value::from("completed")),
                );
            }
            _ => {}
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_folds_stream_to_settled() {
        let start = serde_json::json!({"kind":"assistant/start","id":"a1","text":""});
        assert_eq!(
            AssistantNode::matches(&start),
            Some(("a1".into(), Role::Start))
        );
        let delta = serde_json::json!({"kind":"assistant/delta","id":"a1","delta":"hi"});
        let s1 = AssistantNode::fold(None, &start);
        let s2 = AssistantNode::fold(Some(s1), &delta);
        let end = serde_json::json!({"kind":"assistant/end","id":"a1"});
        let s3 = AssistantNode::fold(Some(s2), &end);
        assert_eq!(s3["text"], "hi");
        assert_eq!(s3["settled"], true);
    }

    #[test]
    fn tool_result_sets_status_by_ok() {
        let running = ToolNode::fold(
            None,
            &serde_json::json!({"kind":"tool/start","id":"t1","name":"recall"}),
        );
        assert_eq!(running["status"], "running");
        let done = ToolNode::fold(
            Some(running),
            &serde_json::json!({"kind":"tool/result","id":"t1","ok":true}),
        );
        assert_eq!(done["status"], "settled");
    }

    #[test]
    fn review_job_records_approval() {
        let ev = serde_json::json!({"kind":"review/end","id":"r1","approved":true,"proposal_id":7});
        assert_eq!(
            ReviewJobNode::matches(&ev),
            Some(("r1".into(), Role::Update))
        );
        let s = ReviewJobNode::fold(None, &ev);
        assert_eq!(s["status"], true);
        assert_eq!(s["proposal_id"], 7);
    }

    #[test]
    fn delivery_collects_items() {
        let s0 = DeliveryNode::fold(
            None,
            &serde_json::json!({"kind":"delivery/start","id":"d1"}),
        );
        let s1 = DeliveryNode::fold(
            Some(s0),
            &serde_json::json!({"kind":"delivery/item","id":"d1","item":{"uri":"crm://1"}}),
        );
        let s2 = DeliveryNode::fold(
            Some(s1),
            &serde_json::json!({"kind":"delivery/end","id":"d1"}),
        );
        assert_eq!(s2["items"].as_array().unwrap().len(), 1);
        assert_eq!(s2["done"], true);
    }

    #[test]
    fn workflow_run_tracks_phases_and_outcome() {
        let s0 = WorkflowRunNode::fold(
            None,
            &serde_json::json!({"kind":"workflow/start","run_id":"w9"}),
        );
        let s1 = WorkflowRunNode::fold(
            Some(s0),
            &serde_json::json!({"kind":"workflow/phase","run_id":"w9","phase":"collect"}),
        );
        let end = serde_json::json!({"kind":"workflow/end","run_id":"w9","outcome":"error"});
        let s2 = WorkflowRunNode::fold(Some(s1), &end);
        assert_eq!(s2["state"], "error");
        assert_eq!(s2["phases"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn unrelated_events_match_nothing() {
        let e = serde_json::json!({"kind":"session/event","id":"x"});
        assert!(AssistantNode::matches(&e).is_none());
        assert!(ToolNode::matches(&e).is_none());
        assert!(ReviewJobNode::matches(&e).is_none());
    }
}
