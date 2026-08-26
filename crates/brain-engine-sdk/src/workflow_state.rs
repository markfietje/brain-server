//! The workflow state-key contract — the engine ABI's routing table.
//!
//! The four state keys below are NORMATIVE ABI: every engine routes its next
//! action through [`decide`], and every host stores state as a JSON object
//! carrying at most these routing keys. An engine must not invent a fifth
//! routing key; host-specific payload keys (`steps`, `findings`, `answers`,
//! …) are opaque to this module.
//!
//! | Key               | Meaning                          | Decision          |
//! |-------------------|----------------------------------|-------------------|
//! | `status`          | `done`/`complete` = terminal     | [`Decision::Done`] |
//! | `pending_question`| a human answer is awaited        | [`Decision::AskHuman`] |
//! | `next_step`       | one executable step is named     | [`Decision::RunStep`] |
//! | `next_state`      | the whole state replaces itself  | [`Decision::Advance`] |
//!
//! Precedence: terminal > pending question > step > advance > (fallthrough)
//! `Done`. Versioned with `requires_host`: the key set may only GROW, never
//! reshape — a rename is a breaking release.

use serde_json::Value;

/// The next action an engine driver takes for a run, derived purely from the
/// run's state JSON.
#[derive(Debug, PartialEq, Clone)]
pub enum Decision {
    /// `state.pending_question` is set — a human must answer before the run
    /// proceeds. Engines persist nothing and simply stop.
    AskHuman { question: String },
    /// `state.next_step` names one executable step.
    RunStep { step: String },
    /// `state.next_state` carries the replacement state (a JSON string).
    Advance { next_state: String },
    /// Terminal: `status` is `done`/`complete`, or no routing key matched.
    Done,
}

/// Route a run's state to its next [`Decision`]. Pure; total; never panics.
pub fn decide(state: &Value) -> Decision {
    let status = state
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("active");
    if status == "done" || status == "complete" {
        return Decision::Done;
    }
    if let Some(q) = state.get("pending_question").and_then(|v| v.as_str()) {
        return Decision::AskHuman {
            question: q.to_string(),
        };
    }
    if let Some(s) = state.get("next_step").and_then(|v| v.as_str()) {
        return Decision::RunStep {
            step: s.to_string(),
        };
    }
    if let Some(n) = state.get("next_state").and_then(|v| v.as_str()) {
        return Decision::Advance {
            next_state: n.to_string(),
        };
    }
    Decision::Done
}

/// The fixed public case-status vocabulary (v1.28.36 Keystone): seven words
/// a customer may ever see on a status page. The set is CLOSED — a new
/// internal state must map onto one of these or the page lies.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PublicStatus {
    /// Fresh run, nothing has happened yet.
    Received,
    /// Work is executing (`next_step` named or advancing).
    InProgress,
    /// `pending_question` set — the customer owes us an answer.
    AwaitingYourReply,
    /// An explicit confirmation pause (`status: awaiting-confirmation`).
    AwaitingConfirmation,
    /// Terminal success (`done`/`complete`).
    Resolved,
    /// Terminal close (`closed`/`cancelled`).
    Closed,
}

impl PublicStatus {
    /// The exact wire/build string. Frozen once shipped — pages are static
    /// artifacts that outlive deploys.
    pub fn as_str(&self) -> &'static str {
        match self {
            PublicStatus::Received => "received",
            PublicStatus::InProgress => "in-progress",
            PublicStatus::AwaitingYourReply => "awaiting-your-reply",
            PublicStatus::AwaitingConfirmation => "awaiting-confirmation",
            PublicStatus::Resolved => "resolved",
            PublicStatus::Closed => "closed",
        }
    }
}

/// Map a run's four-key state JSON onto the public vocabulary. Pure;
/// deterministic; total; carries NO field of the state outward — callers get
/// a word, never content. Precedence mirrors [`decide`] minus internals:
/// terminal/close > confirmation pause > resolution > pending question >
/// step/advance > fresh.
pub fn public_status(state: &Value) -> PublicStatus {
    let status = state.get("status").and_then(|v| v.as_str());
    match status {
        Some("closed") | Some("cancelled") => return PublicStatus::Closed,
        Some("awaiting-confirmation") | Some("awaiting_confirmation") => {
            return PublicStatus::AwaitingConfirmation
        }
        _ => {}
    }
    match status {
        Some("done") | Some("complete") => return PublicStatus::Resolved,
        _ => {}
    }
    if state.get("pending_question").is_some_and(|v| !v.is_null()) {
        return PublicStatus::AwaitingYourReply;
    }    let stepping = state.get("next_step").is_some_and(|v| !v.is_null())
        || state.get("next_state").is_some_and(|v| !v.is_null());
    if stepping {
        PublicStatus::InProgress
    } else {
        PublicStatus::Received
    }
}

// The context-window derivation (write→select).
//
// The session is unbounded; consumers derive the smallest high-signal window
// from it on demand. The derivation is PURE and deterministic: latest
// checkpoint at-or-before the anchor + the delta after it + per-branch
// findings digests + the open question. Appending events never changes an
// earlier window (prefix stability), so consumers can cache derived slices.

/// One lineage event as the derivation consumes it (the outbox projection).
#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    pub id: i64,
    pub topic: String,
    pub payload_json: String,
}

/// A derived, field-budgeted view over a run's event chain.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ContextWindow {
    /// Latest `workflow/checkpoint` at-or-before the anchor (the anchor
    /// itself). Never truncated away.
    pub checkpoint: Option<EventRow>,
    /// Events strictly after the checkpoint up to the anchor (oldest-first).
    /// Dropped oldest-first when over budget.
    pub delta: Vec<EventRow>,
    /// Non-cryptographic fingerprints of the checkpoint state's `findings[]`
    /// (FNV-1a 64, hex) — the structured notes ride even when the delta is
    /// truncated away.
    pub findings_digests: Vec<String>,
    /// The run's open `pending_question`, if any (never truncated away).
    pub open_question: Option<String>,
    /// True when any delta event was dropped to fit the budget.
    pub truncated: bool,
}

/// Count JSON fields deterministically (no tokenizer dependency): scalar = 1,
/// array = sum of elements, object = 1 + sum of values. Documented
/// approximation for consumers: one counted field ≈ one token, not exactly.
fn count_fields(v: &Value) -> usize {
    match v {
        Value::Null | Value::Bool(_) | Value::Number(_) => 1,
        Value::String(_) => 1,
        Value::Array(a) => a.iter().map(count_fields).sum(),
        Value::Object(o) => 1 + o.values().map(count_fields).sum::<usize>(),
    }
}

/// FNV-1a 64 over bytes — a stable, dependency-free fingerprint. NOT a
/// security primitive; it names a finding so consumers can dedupe notes.
fn fnv1a(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Derive the context window at the LATEST event (the common consumer call).
pub fn derive_context(events: &[EventRow], budget_fields: usize) -> ContextWindow {
    let anchor = events.last().map(|e| e.id);
    derive_context_at(events, anchor, budget_fields)
}

/// Derive the window at-or-before `at_event` (`None` → before anything).
/// Pure; no clocks; never panics on malformed payloads.
pub fn derive_context_at(
    events: &[EventRow],
    at_event: Option<i64>,
    budget_fields: usize,
) -> ContextWindow {
    let upto = |e: &EventRow| at_event.is_none_or(|a| e.id <= a);
    let scoped: Vec<&EventRow> = events.iter().filter(|e| upto(e)).collect();
    let checkpoint = scoped
        .iter()
        .rev()
        .find(|e| e.topic == "workflow/checkpoint")
        .map(|e| (*e).clone());
    let delta_start = checkpoint.as_ref().map_or(0, |c| {
        scoped.iter().position(|e| e.id == c.id).unwrap_or(0) + 1
    });
    let mut delta: Vec<EventRow> = scoped
        .iter()
        .skip(delta_start)
        .map(|e| (*e).clone())
        .collect();

    // Findings digests + open question from the checkpoint snapshot (or the
    // earliest known state when no checkpoint exists yet).
    let state_json = checkpoint
        .as_ref()
        .map(|c| c.payload_json.clone())
        .unwrap_or_default();
    let state: Value = serde_json::from_str(&state_json).unwrap_or(Value::Null);
    let findings_digests: Vec<String> = state
        .get("findings")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|f| fnv1a(f.to_string().as_bytes())).collect())
        .unwrap_or_default();
    let open_question = state
        .get("pending_question")
        .and_then(Value::as_str)
        .map(str::to_string);

    // Field budget: drop OLDEST delta first until the payload fields fit.
    // Checkpoint, notes, and open question are never dropped.
    let mut truncated = false;
    if budget_fields > 0 {
        while !delta.is_empty() {
            let used: usize = delta
                .iter()
                .filter_map(|e| serde_json::from_str::<Value>(&e.payload_json).ok())
                .map(|v| count_fields(&v))
                .sum();
            if used <= budget_fields {
                break;
            }
            delta.remove(0);
            truncated = true;
        }
    }

    ContextWindow {
        checkpoint,
        delta,
        findings_digests,
        open_question,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn public_status_maps_every_decision_state_deterministically() {
        // The fixture table is the contract: every internal shape lands on
        // exactly one public word, and the word set is closed.
        let fixtures: Vec<(Value, PublicStatus)> = vec![
            (json!({}), PublicStatus::Received),
            (json!({"status": "active"}), PublicStatus::Received),
            (
                json!({"status": "active", "next_step": "inventory"}),
                PublicStatus::InProgress,
            ),
            (
                json!({"status": "active", "next_state": "{\"x\":1}"}),
                PublicStatus::InProgress,
            ),
            (
                json!({"status": "active", "pending_question": "PII stays in"}),
                PublicStatus::AwaitingYourReply,
            ),
            (
                json!({"status": "awaiting-confirmation"}),
                PublicStatus::AwaitingConfirmation,
            ),
            (
                json!({"status": "awaiting_confirmation"}),
                PublicStatus::AwaitingConfirmation,
            ),
            (json!({"status": "done"}), PublicStatus::Resolved),
            (json!({"status": "complete"}), PublicStatus::Resolved),
            (json!({"status": "closed"}), PublicStatus::Closed),
            (json!({"status": "cancelled"}), PublicStatus::Closed),
            // Precedence pins: terminal/close beat everything; the
            // confirmation pause beats a pending question; a pending question
            // beats resolution.
            (
                json!({"status": "closed", "pending_question": "x"}),
                PublicStatus::Closed,
            ),
            (
                json!({"status": "awaiting-confirmation", "pending_question": "x"}),
                PublicStatus::AwaitingConfirmation,
            ),
            (
                json!({"status": "done", "pending_question": "x"}),
                PublicStatus::Resolved,
            ),
        ];
        for (state, want) in fixtures {
            let round =
                serde_json::from_str::<Value>(&serde_json::to_string(&state).expect("fixture"))
                    .expect("fixture");
            assert_eq!(public_status(&round), want, "drift for {round}");
        }
    }

    #[test]
    fn decision_keys_are_frozen_abi() {
        // Fixture round-trip: each of the four keys routes exactly one
        // variant, byte-identical through a serde_json round-trip. A rename
        // or reshuffle of any key breaks here BEFORE it breaks engines.
        let fixtures: Vec<(Value, Decision)> = vec![
            (json!({"status": "done"}), Decision::Done),
            (json!({"status": "complete"}), Decision::Done),
            (
                json!({"pending_question": "which disk group?"}),
                Decision::AskHuman {
                    question: "which disk group?".into(),
                },
            ),
            (
                json!({"next_step": "inventory"}),
                Decision::RunStep {
                    step: "inventory".into(),
                },
            ),
            (
                json!({"next_state": "{\"status\":\"active\"}"}),
                Decision::Advance {
                    next_state: "{\"status\":\"active\"}".into(),
                },
            ),
            (json!({}), Decision::Done),
        ];
        for (v, want) in fixtures {
            let round = serde_json::from_str::<Value>(&serde_json::to_string(&v).unwrap()).unwrap();
            assert_eq!(decide(&round), want, "routing drifted for {round}");
        }
        // Precedence is part of the contract too.
        assert_eq!(
            decide(&json!({"status": "done", "pending_question": "x"})),
            Decision::Done,
            "terminal beats pending"
        );
        assert!(
            matches!(
                decide(&json!({"pending_question": "x", "next_step": "y"})),
                Decision::AskHuman { .. }
            ),
            "pending beats step"
        );
    }

    fn ev(id: i64, topic: &str, payload: &str) -> EventRow {
        EventRow {
            id,
            topic: topic.to_string(),
            payload_json: payload.to_string(),
        }
    }

    fn chain() -> Vec<EventRow> {
        vec![
            ev(1, "workflow/start", r#"{"note":"open"}"#),
            ev(2, "workflow/log", r#"{"line":"s1"}"#),
            ev(
                3,
                "workflow/checkpoint",
                r#"{"steps":[1],"findings":["f1"],"pending_question":"ship?"}"#,
            ),
            ev(4, "workflow/log", r#"{"line":"s2"}"#),
            ev(5, "workflow/log", r#"{"line":"s3"}"#),
        ]
    }

    #[test]
    fn window_is_latest_checkpoint_plus_delta_plus_notes() {
        let w = derive_context(&chain(), 10_000);
        let cp = w.checkpoint.expect("checkpoint anchor");
        assert_eq!(cp.id, 3);
        assert_eq!(
            w.delta.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![4, 5],
            "delta = events after the checkpoint"
        );
        assert_eq!(w.findings_digests.len(), 1, "the finding rides as a note");
        assert_eq!(
            w.open_question.as_deref(),
            Some("ship?"),
            "open question surfaced"
        );
        assert!(!w.truncated);
    }

    #[test]
    fn truncation_drops_oldest_delta_first_and_flags() {
        // Budget 2 fits only ONE of the two delta payloads ({...}=2 fields
        // each) → the oldest (id 4) is dropped, newest kept, flag set.
        let w = derive_context(&chain(), 2);
        assert_eq!(w.delta.iter().map(|e| e.id).collect::<Vec<_>>(), vec![5]);
        assert!(w.truncated);
        // The anchor + notes survive ANY truncation.
        assert_eq!(w.checkpoint.expect("kept").id, 3);
        assert_eq!(w.open_question.as_deref(), Some("ship?"));
        assert_eq!(w.findings_digests.len(), 1);
        // Zero budget means UNBOUNDED (no budget requested) — documented.
        let w0 = derive_context(&chain(), 0);
        assert_eq!(w0.delta.len(), 2);
        assert!(!w0.truncated, "budget 0 = no budget");
        assert_eq!(w0.checkpoint.unwrap().id, 3);
    }

    #[test]
    fn appending_events_never_changes_earlier_windows() {
        let events = chain();
        let before = derive_context_at(&events, Some(4), 10_000);
        let mut grown = events.clone();
        grown.push(ev(6, "workflow/log", r#"{"line":"s4"}"#));
        grown.push(ev(7, "workflow/checkpoint", r#"{"steps":[2]}"#));
        let after = derive_context_at(&grown, Some(4), 10_000);
        assert_eq!(
            before, after,
            "prefix stability: later events change nothing at id 4"
        );
    }

    #[test]
    fn window_at_askhuman_includes_open_question() {
        let events = chain();
        let w = derive_context_at(&events, Some(3), 10_000);
        assert_eq!(
            w.open_question.as_deref(),
            Some("ship?"),
            "the AskHuman pause is visible in the window"
        );
        assert!(w.delta.is_empty(), "checkpoint IS the anchor → no delta");
        // A malformed checkpoint payload degrades to empty notes, no panic.
        let mut bad = chain();
        bad[2].payload_json = "not-json".to_string();
        let wb = derive_context(&bad, 10_000);
        assert!(wb.findings_digests.is_empty());
        assert_eq!(wb.open_question, None);
    }
}
