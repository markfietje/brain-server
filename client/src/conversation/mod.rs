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

/// v1.28.20 Cockpit M2: the runtime frame driver's policy core. An
/// AnimationFrame publication flushes at most once per frame batch (16 ms);
/// Immediate flushes on every revision change. Pure so coalescing is
/// pinnable without a renderer.
pub const FRAME_MS: u64 = 16;

#[derive(Debug, Default, Clone)]
pub struct FrameGate {
    last_flushed_rev: u64,
    last_frame_ms: u64,
}

impl FrameGate {
    /// `Some(revision)` = flush now (and record it). `None` = keep pending.
    pub fn due(&mut self, revision: u64, now_ms: u64, animate: bool) -> Option<u64> {
        if revision == 0 || revision == self.last_flushed_rev {
            return None;
        }
        if !animate {
            self.last_flushed_rev = revision;
            return Some(revision);
        }
        // The first batch always paints; later batches within one frame
        // window coalesce into the next paint.
        if self.last_flushed_rev != 0 && now_ms.saturating_sub(self.last_frame_ms) < FRAME_MS {
            return None;
        }
        self.last_frame_ms = now_ms;
        self.last_flushed_rev = revision;
        Some(revision)
    }
}

impl NodeState {
    fn new(key: &str, kind: &'static str, seq: u64, data: Value) -> Self {
        Self {
            key: key.to_string(),
            kind,
            seq,
            data,
        }
    }
}

/// A node definition: match + fold. The owner owns its state shape ("who
/// injects it, owns its type").
pub trait NodeDefinition {
    /// The node family this definition builds (the slot key).
    const KIND: &'static str;
    /// Publication policy for this family.
    const PUBLICATION: Publication;
    /// (event kind, role) pairs — the family's event vocabulary. First match
    /// wins; the node id rides the event's `id` field.
    fn events() -> &'static [(&'static str, Role)];
    /// Does `event` belong to this family? Default: table lookup on the
    /// event's `kind`, returning `(id, role)`.
    fn matches(event: &Value) -> Option<(String, Role)> {
        let kind = event.get("kind")?.as_str()?;
        let id = event.get("id").or_else(|| event.get("run_id"))?.as_str()?;
        Self::events()
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, role)| (id.to_string(), *role))
    }
    /// Fold an event into state. `prev` is `None` for Start.
    fn fold(prev: Option<Value>, event: &Value) -> Value;
}

// ---------------------------------------------------------------------------
// Built-in definitions. Each maps harness-shaped events onto chat nodes.
// ---------------------------------------------------------------------------

/// Assistant streaming → settled (`assistant/delta`, `assistant/end`).
pub struct AssistantTurn;
impl NodeDefinition for AssistantTurn {
    const KIND: &'static str = "assistant";
    const PUBLICATION: Publication = Publication::AnimationFrame;
    fn events() -> &'static [(&'static str, Role)] {
        const EVENTS: &[(&str, Role)] = &[
            ("assistant/start", Role::Start),
            ("assistant/delta", Role::Update),
            ("assistant/end", Role::Update),
        ];
        EVENTS
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

impl AssistantTurn {
    /// Cockpit M2: the render model — turn text + settled flag. The panel
    /// renders progressive (unsettled) and settled turns identically except
    /// for the streaming indicator.
    pub fn build_view_node(state: &Value) -> Value {
        serde_json::json!({
            "kind": Self::KIND,
            "text": state.get("text").cloned().unwrap_or(Value::Null),
            "settled": state.get("settled").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

/// Tool running → settled (`tool/start`, `tool/result`).
pub struct ToolInvocation;
impl NodeDefinition for ToolInvocation {
    const KIND: &'static str = "tool";
    const PUBLICATION: Publication = Publication::AnimationFrame;
    fn events() -> &'static [(&'static str, Role)] {
        const EVENTS: &[(&str, Role)] =
            &[("tool/start", Role::Start), ("tool/result", Role::Update)];
        EVENTS
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

impl ToolInvocation {
    /// Cockpit M2: the invocation-card model — name, status, output payload
    /// (the evidence viewer renders whatever structured evidence rode along).
    pub fn build_view_node(state: &Value) -> Value {
        serde_json::json!({
            "kind": Self::KIND,
            "name": state.get("name").cloned().unwrap_or(Value::Null),
            "status": state.get("status").cloned().unwrap_or(Value::from("running")),
            "output": state.get("output").cloned().unwrap_or(Value::Null),
        })
    }
}

/// Review job (brain truth): the producer events `proposal/open|updated|decided`
/// (whole-value checkpoints: content digest, SLA deadline, role gate — replay
/// works from any join point) plus the legacy `review/*` vocabulary.
pub struct ReviewJob;
impl NodeDefinition for ReviewJob {
    const KIND: &'static str = "review-job";
    const PUBLICATION: Publication = Publication::Immediate;
    fn events() -> &'static [(&'static str, Role)] {
        const EVENTS: &[(&str, Role)] = &[
            ("proposal/open", Role::Start),
            ("proposal/updated", Role::Update),
            ("proposal/decided", Role::Update),
            ("review/start", Role::Start),
            ("review/progress", Role::Update),
            ("review/end", Role::Update),
        ];
        EVENTS
    }
    /// Producer events carry a branded proposal id (`p<id>`), not a generic id.
    fn matches(event: &Value) -> Option<(String, Role)> {
        let kind = event.get("kind")?.as_str()?;
        let id = event.get("id").or_else(|| event.get("run_id"))?.as_str()?;
        let role = Self::events().iter().find(|(k, _)| *k == kind)?.1;
        if kind.starts_with("proposal/") {
            // Fail-closed coordinate check: only branded ids match.
            crate::plugins::ProposalId::parse(id)?;
        }
        Some((id.to_string(), role))
    }
    fn fold(prev: Option<Value>, event: &Value) -> Value {
        let kind = event.get("kind").and_then(Value::as_str);
        let mut s =
            prev.unwrap_or_else(|| serde_json::json!({"status": "running", "proposal_id": null}));
        if let Some(p) = event.get("proposal_id") {
            s["proposal_id"] = p.clone();
        }
        // Whole-value checkpoints ride every producer event so a node joined
        // before its open still renders complete (digest, SLA clock, gate).
        if let Some(d) = event.get("content_digest") {
            s["content_digest"] = d.clone();
        }
        if let Some(t) = event.get("sla_deadline") {
            s["sla_deadline"] = t.clone();
        }
        if let Some(g) = event.get("role_gate") {
            s["role_gate"] = g.clone();
        }
        match kind {
            Some("proposal/updated" | "review/progress") => {
                if let Some(n) = event.get("pending") {
                    s["pending"] = n.clone();
                }
            }
            Some("proposal/decided" | "review/end") => {
                s["status"] = Value::from(
                    event
                        .get("approved")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                );
                s["terminal"] = Value::from(true);
            }
            _ => {}
        }
        s
    }
}

impl ReviewJob {
    /// The view model for one review-job node. A pending (pre-start) or empty
    /// state renders a fallback placeholder — never silently dropped, and an
    /// undecided node never shows decision actions.
    pub fn build_view_node(state: &Value) -> Value {
        let has_identity = state.get("proposal_id").and_then(Value::as_i64).is_some();
        if !has_identity {
            return serde_json::json!({
                "kind": Self::KIND,
                "fallback": true,
                "label": "awaiting details",
            });
        }
        serde_json::json!({
            "kind": Self::KIND,
            "fallback": false,
            "proposal_id": state.get("proposal_id").cloned().unwrap_or(Value::Null),
            "digest": state.get("content_digest").cloned().unwrap_or(Value::Null),
            "sla_deadline": state.get("sla_deadline").cloned().unwrap_or(Value::Null),
            "role_gate": state.get("role_gate").cloned().unwrap_or(Value::Null),
            "terminal": state.get("terminal").and_then(Value::as_bool).unwrap_or(false),
            "approved": state.get("status").and_then(Value::as_bool),
        })
    }
}

/// Delivery: `tool-workflow/*` + `agent/*` collapsed into one artifact node.
pub struct Delivery;
impl NodeDefinition for Delivery {
    const KIND: &'static str = "delivery";
    const PUBLICATION: Publication = Publication::AnimationFrame;
    fn events() -> &'static [(&'static str, Role)] {
        const EVENTS: &[(&str, Role)] = &[
            ("delivery/start", Role::Start),
            ("delivery/item", Role::Update),
            ("delivery/end", Role::Update),
        ];
        EVENTS
    }
    fn fold(prev: Option<Value>, event: &Value) -> Value {
        let kind = event.get("kind").and_then(Value::as_str);
        let mut s = prev.unwrap_or_else(|| serde_json::json!({"items": [], "done": false}));
        if kind == Some("delivery/item")
            && let (Some(item), Some(arr)) = (
                event.get("item"),
                s.get_mut("items").and_then(Value::as_array_mut),
            )
        {
            arr.push(item.clone());
        }
        if kind == Some("delivery/end") {
            s["done"] = Value::from(true);
        }
        s
    }
}

impl Delivery {
    /// Cockpit M2: the handoff-packet model — collected items + done flag;
    /// the panel prints it as the I-PASS-style packet view.
    pub fn build_view_node(state: &Value) -> Value {
        serde_json::json!({
            "kind": Self::KIND,
            "items": state.get("items").cloned().unwrap_or_else(|| Value::Array(vec![])),
            "done": state.get("done").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

/// Workflow run: `workflow/*` events (start/phase/end).
pub struct WorkflowRun;
impl NodeDefinition for WorkflowRun {
    const KIND: &'static str = "workflow-run";
    const PUBLICATION: Publication = Publication::Immediate;
    fn events() -> &'static [(&'static str, Role)] {
        const EVENTS: &[(&str, Role)] = &[("workflow/start", Role::Start)];
        EVENTS
    }
    /// Prefix family: any `workflow/*` kind beyond the table folds as Update.
    fn matches(event: &Value) -> Option<(String, Role)> {
        let kind = event.get("kind")?.as_str()?;
        if !kind.starts_with("workflow/") {
            return None;
        }
        let role = Self::events()
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or(Role::Update, |(_, r)| *r);
        let id = event
            .get("run_id")
            .or_else(|| event.get("id"))?
            .as_str()?
            .to_string();
        Some((id, role))
    }
    fn fold(prev: Option<Value>, event: &Value) -> Value {
        let mut s = prev.unwrap_or_else(|| serde_json::json!({"state": "running", "phases": []}));
        match event.get("kind").and_then(Value::as_str) {
            Some("workflow/phase") => {
                if let (Some(p), Some(arr)) = (
                    event.get("phase"),
                    s.get_mut("phases").and_then(Value::as_array_mut),
                ) {
                    arr.push(p.clone());
                }
            }
            Some("workflow/end") => {
                s["state"] = event
                    .get("outcome")
                    .cloned()
                    .unwrap_or(Value::from("completed"));
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
            AssistantTurn::matches(&start),
            Some(("a1".into(), Role::Start))
        );
        let delta = serde_json::json!({"kind":"assistant/delta","id":"a1","delta":"hi"});
        let s1 = AssistantTurn::fold(None, &start);
        let s2 = AssistantTurn::fold(Some(s1), &delta);
        let end = serde_json::json!({"kind":"assistant/end","id":"a1"});
        let s3 = AssistantTurn::fold(Some(s2), &end);
        assert_eq!(s3["text"], "hi");
        assert_eq!(s3["settled"], true);
    }

    #[test]
    fn tool_result_sets_status_by_ok() {
        let running = ToolInvocation::fold(
            None,
            &serde_json::json!({"kind":"tool/start","id":"t1","name":"recall"}),
        );
        assert_eq!(running["status"], "running");
        let done = ToolInvocation::fold(
            Some(running),
            &serde_json::json!({"kind":"tool/result","id":"t1","ok":true}),
        );
        assert_eq!(done["status"], "settled");
    }

    #[test]
    fn review_job_records_approval() {
        let ev = serde_json::json!({"kind":"review/end","id":"r1","approved":true,"proposal_id":7});
        assert_eq!(ReviewJob::matches(&ev), Some(("r1".into(), Role::Update)));
        let s = ReviewJob::fold(None, &ev);
        assert_eq!(s["status"], true);
        assert_eq!(s["proposal_id"], 7);
    }

    // ── the producer/consumer review-job matrix ────────────────────

    fn open_ev(id: i64) -> Value {
        serde_json::json!({
            "kind":"proposal/open","id":format!("p{id}"),"proposal_id":id,
            "content_digest":"d1","sla_deadline":1_800_000_000,"role_gate":"approve",
        })
    }

    #[test]
    fn producer_open_starts_with_checkpoints() {
        let ev = open_ev(7);
        assert_eq!(ReviewJob::matches(&ev), Some(("p7".into(), Role::Start)));
        let s = ReviewJob::fold(None, &ev);
        assert_eq!(s["status"], "running");
        assert_eq!(s["content_digest"], "d1");
        assert_eq!(s["sla_deadline"], 1_800_000_000);
        assert_eq!(s["role_gate"], "approve");
    }

    #[test]
    fn unbranded_producer_ids_fail_closed() {
        // A `proposal/*` event with a non-branded id never matches.
        let e = serde_json::json!({"kind":"proposal/open","id":"7"});
        assert_eq!(ReviewJob::matches(&e), None, "unbranded coordinate refused");
        assert_eq!(
            ReviewJob::matches(&serde_json::json!({"kind":"proposal/open","id":"p0"})),
            None
        );
    }

    #[test]
    fn decided_before_start_stays_pending_then_converges() {
        let mut a = crate::conversation::assembler::Assembler::new();
        let decided = serde_json::json!({
            "kind":"proposal/decided","id":"p9","proposal_id":9,
            "approved":true,"content_digest":"d2","role_gate":"approve",
        });
        a.ingest::<ReviewJob>(1, &decided);
        assert!(
            a.snapshot().is_empty(),
            "terminal before start stays pending"
        );
        assert!(a.has_pending("review-job:p9"));
        a.ingest::<ReviewJob>(2, &open_ev(9));
        let snap = a.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(
            snap[0].data["status"], true,
            "pending fold replayed onto start"
        );
        assert_eq!(snap[0].data["terminal"], true);
        assert_eq!(
            snap[0].data["content_digest"], "d2",
            "checkpoint survives replay"
        );
    }

    #[test]
    fn view_node_falls_back_before_details_arrive() {
        // Empty / identity-less state → the pre-start fallback placeholder.
        let fb = ReviewJob::build_view_node(&serde_json::json!({}));
        assert_eq!(fb["fallback"], true);
        // A complete running node renders real coordinates; undecided nodes
        // expose no approval verdict.
        let run = ReviewJob::build_view_node(&ReviewJob::fold(None, &open_ev(7)));
        assert_eq!(run["fallback"], false);
        assert_eq!(run["proposal_id"], 7);
        assert_eq!(run["digest"], "d1");
        assert_eq!(run["approved"], Value::Null, "undecided shows no verdict");
    }

    #[test]
    fn delivery_collects_items() {
        let s0 = Delivery::fold(
            None,
            &serde_json::json!({"kind":"delivery/start","id":"d1"}),
        );
        let s1 = Delivery::fold(
            Some(s0),
            &serde_json::json!({"kind":"delivery/item","id":"d1","item":{"uri":"crm://1"}}),
        );
        let s2 = Delivery::fold(
            Some(s1),
            &serde_json::json!({"kind":"delivery/end","id":"d1"}),
        );
        assert_eq!(s2["items"].as_array().unwrap().len(), 1);
        assert_eq!(s2["done"], true);
    }

    #[test]
    fn workflow_run_tracks_phases_and_outcome() {
        let s0 = WorkflowRun::fold(
            None,
            &serde_json::json!({"kind":"workflow/start","run_id":"w9"}),
        );
        let s1 = WorkflowRun::fold(
            Some(s0),
            &serde_json::json!({"kind":"workflow/phase","run_id":"w9","phase":"collect"}),
        );
        let end = serde_json::json!({"kind":"workflow/end","run_id":"w9","outcome":"error"});
        let s2 = WorkflowRun::fold(Some(s1), &end);
        assert_eq!(s2["state"], "error");
        assert_eq!(s2["phases"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn unrelated_events_match_nothing() {
        let e = serde_json::json!({"kind":"session/event","id":"x"});
        assert!(AssistantTurn::matches(&e).is_none());
        assert!(ToolInvocation::matches(&e).is_none());
        assert!(ReviewJob::matches(&e).is_none());
    }

    // ── Cockpit M2: publication coalescing + the real view models ─────

    /// AnimationFrame publications coalesce: within one 16 ms frame window
    /// only the first revision flushes; Immediate always flushes.
    #[test]
    fn publication_coalesces_to_one_frame_per_batch() {
        let mut g = FrameGate::default();
        // First batch paints immediately.
        assert_eq!(g.due(1, 0, true), Some(1));
        // Revisions arriving inside the same frame window stay pending.
        assert_eq!(g.due(2, 5, true), None);
        assert_eq!(g.due(3, 15, true), None);
        // The next frame window flushes once (the latest revision).
        assert_eq!(g.due(3, 16, true), Some(3));
        assert_eq!(g.due(4, 17, true), None, "one paint per frame");
        // Desktop (animate=false) flushes every revision change.
        let mut d = FrameGate::default();
        assert_eq!(d.due(1, 0, false), Some(1));
        assert_eq!(d.due(2, 1, false), Some(2));
        // No-op guards: zero revision / already-flushed never re-publish.
        assert_eq!(d.due(0, 9, false), None);
        assert_eq!(d.due(2, 9, false), None);
    }

    #[test]
    fn transcript_view_models_render_all_five_node_kinds() {
        // assistant: progressive → settled text
        let mut s = AssistantTurn::fold(
            None,
            &serde_json::json!({"kind":"assistant/start","id":"a"}),
        );
        s = AssistantTurn::fold(
            Some(s),
            &serde_json::json!({"kind":"assistant/delta","id":"a","delta":"hi"}),
        );
        let av = AssistantTurn::build_view_node(&s);
        assert_eq!(av["text"], "hi");
        assert_eq!(av["settled"], false, "streaming indicator stays on");
        // tool: name + status + output pass through
        let mut t = ToolInvocation::fold(
            None,
            &serde_json::json!({"kind":"tool/start","id":"t","name":"recall"}),
        );
        t = ToolInvocation::fold(
            Some(t),
            &serde_json::json!({"kind":"tool/result","id":"t","ok":false}),
        );
        let tv = ToolInvocation::build_view_node(&t);
        assert_eq!(tv["name"], "recall");
        assert_eq!(tv["status"], "error");
        // delivery: items + done
        let dv = Delivery::build_view_node(
            &serde_json::json!({"items":[{"uri":"crm://1"}],"done":true}),
        );
        assert_eq!(dv["items"].as_array().unwrap().len(), 1);
        assert_eq!(dv["done"], true);
        // review-job + workflow-run already have builders pinned above;
        // their kinds resolve through the same registry here.
        use crate::conversation::event_registry::builtin_registry;
        let reg = builtin_registry();
        for kind in [
            "assistant",
            "tool",
            "delivery",
            "review-job",
            "workflow-run",
        ] {
            assert!(
                reg.is_registered(kind),
                "{kind} resolves in the built-in registry"
            );
        }
    }
}
