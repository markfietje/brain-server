//! The compaction worker: admit under pressure, summarize via the loop's
//! OWN provider, never rewrite history.
//!
//! Admission policy is the SDK's (`prompt.rs`: pressure at 16k window
//! tokens, verbatim tail 20k) — this module only MEASURES the window and
//! DRIVES the phases: at the loop-top Idle boundary the harness's
//! `compact()` gate runs (structural ops are Idle-only by harness law; the
//! phase enum itself has no setter the driver may reach, so the gate call
//! IS the structural admission), then the summary is produced by a provider
//! call — a compaction prompt is model work, never kernel logic.
//!
//! History is append-only and prefix-stable: a compaction appends ONE
//! `compaction` event (summary + the seq it compacts through); older rows
//! are referenced, never mutated. Context assembly reshapes around the
//! LATEST compaction event — summary first, then the verbatim tail after
//! it. The three strategy knobs (just-before-call admission, tool-result
//! clearing, selective retention) are policy switches with pinned
//! all-on defaults; each shapes the SUMMARY INPUT only, never the log.

use crate::agentloop::provider::{ChatMessage, Role};
use crate::workflow::session_log::SessionEventRow;

/// Deterministic token estimate (chars ÷ 4, rounded up). The kernel never
/// calls a tokenizer: the estimate only gates WHEN compaction admits, so it
/// must be deterministic and cheap, not exact — pinned by test.
pub(crate) const CHARS_PER_TOKEN: usize = 4;

/// Kinds whose payloads ride into the summary input VERBATIM however old
/// they are (the user's own words are the thread a summary must not blur).
pub(crate) const RETAINED_KINDS: &[&str] = &["user"];

/// The stub that replaces a cleared tool-result body in the summary input.
pub(crate) const CLEARED_TOOL_RESULT: &str = "[tool result cleared by compaction]";

/// The three compaction strategies as switches, defaults pinned all-on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompactionPolicy {
    /// Admit compaction just before a provider call (the loop-top check).
    pub just_before_call: bool,
    /// Replace old tool-result BODIES with a stub in the summary input
    /// (the tail keeps them verbatim; the log keeps them forever).
    pub tool_result_clearing: bool,
    /// Carry retained kinds (user turns) verbatim into the summary input.
    pub selective_retention: bool,
}

pub(crate) const DEFAULT_COMPACTION_POLICY: CompactionPolicy = CompactionPolicy {
    just_before_call: true,
    tool_result_clearing: true,
    selective_retention: true,
};

/// Estimate the token weight of one text.
pub(crate) fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(CHARS_PER_TOKEN)
}

/// The session window's estimated token weight.
pub(crate) fn window_tokens(events: &[SessionEventRow]) -> usize {
    events
        .iter()
        .map(|e| estimate_tokens(&e.payload_json))
        .sum()
}

/// A compaction split: the HEAD (older events, to be summarized — the log
/// keeps them untouched) and the TAIL (recent events carried verbatim).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CompactionSplit {
    pub head: Vec<SessionEventRow>,
    pub tail: Vec<SessionEventRow>,
}

/// Plan a compaction over the replayed window: `None` while pressure is
/// under the SDK's threshold; otherwise split at the verbatim-tail boundary
/// (the most recent events fitting the SDK's keep-verbatim budget).
pub(crate) fn plan(events: &[SessionEventRow]) -> Option<CompactionSplit> {
    let budget = brain_engine_sdk::prompt::KEEP_VERBATIM_TOKENS;
    if !brain_engine_sdk::prompt::should_compact(window_tokens(events)) {
        return None;
    }
    // Walk back from the newest event, filling the verbatim budget; the
    // first (oldest) event that would overflow starts the head. At least
    // one event always stays in the tail — a compaction that summarized
    // everything would leave no anchor.
    let mut tail_tokens = 0usize;
    let mut split_at = events.len();
    for (i, event) in events.iter().enumerate().rev() {
        let w = estimate_tokens(&event.payload_json);
        if tail_tokens + w > budget && split_at < events.len() {
            break;
        }
        tail_tokens += w;
        split_at = i;
    }
    Some(CompactionSplit {
        head: events[..split_at].to_vec(),
        tail: events[split_at..].to_vec(),
    })
}

/// Shape the summary input per policy: retained kinds verbatim, cleared
/// tool-result bodies stubbed, everything else passed as-is for the model
/// to compress. The knobs only reshape THIS input — the log is untouched.
pub(crate) fn summary_input(split: &CompactionSplit, policy: CompactionPolicy) -> Vec<ChatMessage> {
    split
        .head
        .iter()
        .map(|e| {
            let text = if policy.selective_retention && RETAINED_KINDS.contains(&e.kind.as_str()) {
                e.payload_json.clone()
            } else if policy.tool_result_clearing && e.kind == "tool_result" {
                serde_json::json!({
                    "kind": e.kind,
                    "body": CLEARED_TOOL_RESULT,
                    "seq": e.seq,
                })
                .to_string()
            } else {
                e.payload_json.clone()
            };
            ChatMessage {
                role: Role::User,
                text,
            }
        })
        .collect()
}

/// The durable compaction event payload: the summary, the seq it compacts
/// through, and where the verbatim tail starts.
pub(crate) fn compaction_event_json(summary: &str, split: &CompactionSplit) -> String {
    serde_json::json!({
        "summary": summary,
        "compacted_through_seq": split.head.last().map(|e| e.seq),
        "tail_from_seq": split.tail.first().map(|e| e.seq),
    })
    .to_string()
}

/// Reshape a replayed window around its LATEST compaction event: the
/// summary to lead with, and the verbatim tail after it. A window with no
/// compaction event is returned whole (the whole log is the tail).
pub(crate) fn context_window(events: &[SessionEventRow]) -> (Option<String>, &[SessionEventRow]) {
    let Some(idx) = events.iter().rposition(|e| e.kind == "compaction") else {
        return (None, events);
    };
    // A malformed compaction row is a data bug — a missing summary fails
    // toward an absent lead message (tail still applies), never toward
    // dropping or duplicating context.
    let summary = serde_json::from_str::<serde_json::Value>(&events[idx].payload_json)
        .ok()
        .and_then(|v| v.get("summary")?.as_str().map(str::to_string));
    (summary, &events[idx + 1..])
}

/// The cache-stable system prompt for the summary call. Deterministic
/// assembly, no timestamps — same discipline as the turn prompt.
pub(crate) const COMPACTION_SYSTEM_PROMPT: &str = "You are the session compactor. Summarize the given session events into \
     a compact factual brief that preserves decisions, open questions, and \
     user intent. Output only the brief.";

#[cfg(test)]
mod tests {
    use super::*;

    fn event(seq: i64, kind: &str, payload: &str) -> SessionEventRow {
        SessionEventRow {
            seq,
            kind: kind.into(),
            payload_json: payload.into(),
            created_at: seq,
        }
    }

    fn big(n: usize) -> String {
        // 4,000 chars ≈ 1,000 tokens per event.
        "x".repeat(n)
    }

    #[test]
    fn pressure_thresholds_ride_the_sdk_constants() {
        // Falsifiable pin: the SDK owns the numbers; the worker owns nothing.
        assert_eq!(brain_engine_sdk::prompt::COMPACT_PRESSURE_TOKENS, 16_000);
        assert_eq!(brain_engine_sdk::prompt::KEEP_VERBATIM_TOKENS, 20_000);
        assert_eq!(CHARS_PER_TOKEN, 4);
    }

    #[test]
    fn below_pressure_returns_none() {
        // 10 events × 1,000 tokens = 10k < 16k pressure.
        let events: Vec<_> = (1..=10)
            .map(|i| event(i, "user", &format!("{{\"q\":\"{i}\"}}")))
            .collect();
        assert!(plan(&events).is_none());
        assert!(!brain_engine_sdk::prompt::should_compact(window_tokens(
            &events
        )));
    }

    #[test]
    fn pressure_splits_verbatim_tail_byte_equal() {
        // 30 events × 1,000 tokens = 30k ≥ 16k → compaction admits; the
        // 20k verbatim budget keeps the 20 most recent, head = 10 oldest.
        let events: Vec<_> = (1..=30)
            .map(|i| event(i, "assistant", &big(4_000)))
            .collect();
        let split = plan(&events).expect("pressure crossed");
        assert_eq!(split.head.len(), 10, "30k window − 20k verbatim = 10k head");
        assert_eq!(split.tail.len(), 20);
        assert_eq!(split.head[0].seq, 1);
        assert_eq!(split.tail[0].seq, 11);
        // The tail is VERBATIM — byte-equal payloads, referenced not mutated.
        assert_eq!(split.tail[0].payload_json, big(4_000));
        assert!(window_tokens(&split.tail) <= brain_engine_sdk::prompt::KEEP_VERBATIM_TOKENS);
    }

    #[test]
    fn tool_result_clearing_stubs_only_the_head() {
        let mut events: Vec<_> = (1..=25).map(|i| event(i, "user", &big(4_000))).collect();
        events.insert(
            0,
            event(
                0,
                "tool_result",
                &format!("{{\"output\":\"{}\"}}", big(200)),
            ),
        );
        let split = plan(&events).expect("pressure crossed");
        let input = summary_input(
            &split,
            CompactionPolicy {
                just_before_call: true,
                tool_result_clearing: true,
                selective_retention: false,
            },
        );
        let cleared = &input[0].text;
        assert!(cleared.contains(CLEARED_TOOL_RESULT));
        assert!(!cleared.contains(&big(200)[..100]), "the body is gone");
        // The tail's tool results (if any) keep their bodies — only the
        // summary input is reshaped, never the split itself.
        assert!(
            split
                .tail
                .iter()
                .all(|e| !e.payload_json.contains(CLEARED_TOOL_RESULT))
        );
    }

    #[test]
    fn selective_retention_keeps_user_turns_verbatim() {
        let events: Vec<_> = (1..=30)
            .map(|i| {
                if i % 3 == 0 {
                    event(i, "user", &format!("{{\"q\":\"user says {i} exactly\"}}"))
                } else {
                    event(i, "assistant", &big(4_000))
                }
            })
            .collect();
        let split = plan(&events).expect("pressure crossed");
        let input = summary_input(
            &split,
            CompactionPolicy {
                just_before_call: true,
                tool_result_clearing: true,
                selective_retention: true,
            },
        );
        let head_users: Vec<&str> = split
            .head
            .iter()
            .filter(|e| e.kind == "user")
            .map(|e| e.payload_json.as_str())
            .collect();
        for payload in head_users {
            assert!(
                input.iter().any(|m| m.text == payload),
                "user turn rides verbatim into the summary input"
            );
        }
    }

    #[test]
    fn context_window_reshapes_around_the_latest_compaction() {
        let events = vec![
            event(1, "user", r#"{"q":"old"}"#),
            event(
                2,
                "compaction",
                r#"{"summary":"the brief","compacted_through_seq":1,"tail_from_seq":2}"#,
            ),
            event(3, "user", r#"{"q":"after"}"#),
        ];
        let (summary, tail) = context_window(&events);
        assert_eq!(summary.as_deref(), Some("the brief"));
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].payload_json, r#"{"q":"after"}"#);
        // No compaction yet: the whole window is the tail.
        let (none, whole) = context_window(&events[..1]);
        assert_eq!(none, None);
        assert_eq!(whole.len(), 1);
    }

    #[test]
    fn policy_defaults_are_pinned_all_on() {
        const { assert!(DEFAULT_COMPACTION_POLICY.just_before_call) };
        const { assert!(DEFAULT_COMPACTION_POLICY.tool_result_clearing) };
        const { assert!(DEFAULT_COMPACTION_POLICY.selective_retention) };
        assert_eq!(RETAINED_KINDS, &["user"]);
    }
}
