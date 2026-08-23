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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
