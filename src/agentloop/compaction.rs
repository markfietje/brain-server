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

use crate::agentloop::context::{self, ContextError, ContextEvent};
use crate::agentloop::provider::ChatMessage;
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
    if split_at == 0 {
        return None;
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
            ChatMessage::User { text }
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

pub(crate) const SUMMARY_CAP: usize = 16 * 1024;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyBoundary {
    summary: String,
    compacted_through_seq: i64,
    tail_from_seq: i64,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Boundary {
    version: u8,
    summary: String,
    compacted_through_seq: i64,
    tail_from_seq: i64,
    supersedes_seq: Option<i64>,
    tail_seqs: Vec<i64>,
}

pub(crate) struct EffectiveContext {
    pub summary: Option<String>,
    pub boundary_seq: Option<i64>,
    pub tail: Vec<ContextEvent>,
}

pub(crate) fn shape_summary(text: &str) -> Result<String, ContextError> {
    if text.len() > SUMMARY_CAP {
        return Err(ContextError::SummaryLimit);
    }
    let shaped = context::shape_prose(text)?;
    if shaped.len() > SUMMARY_CAP {
        return Err(ContextError::SummaryLimit);
    }
    if shaped.trim().is_empty() {
        return Err(ContextError::CompactionBoundary);
    }
    Ok(shaped)
}

/// The manifest proves scoped continuity without treating excluded control or
/// foreign-child rows as missing conversation. Legacy gaps are ambiguous.
pub(crate) fn reconstruct(events: &[ContextEvent]) -> Result<EffectiveContext, ContextError> {
    let bad = ContextError::CompactionBoundary;
    if events.len() > context::EVENT_CAP {
        return Err(ContextError::EventLimit);
    }
    if events.iter().any(|e| e.row.seq <= 0)
        || events.windows(2).any(|w| w[0].row.seq >= w[1].row.seq)
    {
        return Err(bad);
    }
    let Some(index) = events.iter().rposition(|e| e.row.kind == "compaction") else {
        return Ok(EffectiveContext {
            summary: None,
            boundary_seq: None,
            tail: events.to_vec(),
        });
    };
    let row = &events[index].row;
    if row.payload_json.len() > context::PAYLOAD_CAP {
        return Err(bad);
    }
    let value: serde_json::Value = serde_json::from_str(&row.payload_json).map_err(|_| bad)?;
    let (summary, through, from, manifest, supersedes) = if value.get("version").is_some() {
        let b: Boundary = serde_json::from_str(&row.payload_json).map_err(|_| bad)?;
        if b.version != 2 || b.tail_seqs.is_empty() || b.tail_seqs.len() > context::EVENT_CAP {
            return Err(bad);
        }
        (
            b.summary,
            b.compacted_through_seq,
            b.tail_from_seq,
            Some(b.tail_seqs),
            b.supersedes_seq,
        )
    } else {
        let b: LegacyBoundary = serde_json::from_str(&row.payload_json).map_err(|_| bad)?;
        (
            b.summary,
            b.compacted_through_seq,
            b.tail_from_seq,
            None,
            None,
        )
    };
    if through <= 0
        || through >= from
        || from >= row.seq
        || supersedes.is_some_and(|s| s <= 0 || s >= row.seq)
    {
        return Err(bad);
    }
    let start = events.iter().position(|e| e.row.seq == from).ok_or(bad)?;
    if start >= index {
        return Err(bad);
    }
    let retained = &events[start..index];
    let actual: Vec<_> = retained
        .iter()
        .filter(|e| e.row.kind != "compaction")
        .map(|e| e.row.seq)
        .collect();
    if let Some(expected) = &manifest {
        if expected.first() != Some(&from) || *expected != actual {
            return Err(bad);
        }
        let visible_prior = events[..index]
            .iter()
            .rev()
            .find(|e| e.row.kind == "compaction");
        if visible_prior.is_some_and(|e| Some(e.row.seq) != supersedes)
            || retained
                .iter()
                .any(|e| e.row.kind == "compaction" && Some(e.row.seq) != supersedes)
        {
            return Err(bad);
        }
        if let Some(prior) = visible_prior {
            let previous: serde_json::Value =
                serde_json::from_str(&prior.row.payload_json).map_err(|_| bad)?;
            let (old_through, old_from) = if previous.get("version").is_some() {
                let b: Boundary = serde_json::from_str(&prior.row.payload_json).map_err(|_| bad)?;
                if b.version != 2 {
                    return Err(bad);
                }
                (b.compacted_through_seq, b.tail_from_seq)
            } else {
                let b: LegacyBoundary =
                    serde_json::from_str(&prior.row.payload_json).map_err(|_| bad)?;
                (b.compacted_through_seq, b.tail_from_seq)
            };
            if old_through <= 0
                || old_from <= old_through
                || old_from >= prior.row.seq
                || through < old_from
                || from <= old_from
            {
                return Err(bad);
            }
        }
    } else if retained.iter().any(|e| e.row.kind == "compaction")
        || retained
            .windows(2)
            .any(|w| w[0].row.seq.checked_add(1) != Some(w[1].row.seq))
        || retained.last().and_then(|e| e.row.seq.checked_add(1)) != Some(row.seq)
    {
        return Err(bad);
    }
    // The covered head must end exactly at `through`: any visible conversational
    // row between the covered prefix and the retained tail is a silent-loss gap.
    let covered: Vec<_> = events[..start]
        .iter()
        .filter(|e| e.row.kind != "compaction")
        .map(|e| e.row.seq)
        .collect();
    if covered.last() != Some(&through) {
        return Err(bad);
    }
    let tail = events[start..]
        .iter()
        .filter(|e| e.row.kind != "compaction")
        .cloned()
        .collect();
    Ok(EffectiveContext {
        summary: Some(shape_summary(&summary)?),
        boundary_seq: Some(row.seq),
        tail,
    })
}

pub(crate) struct Admission {
    pub head: Vec<ContextEvent>,
    pub tail: Vec<ContextEvent>,
    pub summary: Option<String>,
    pub supersedes_seq: Option<i64>,
}

pub(crate) fn message_tokens(messages: &[ChatMessage]) -> Result<usize, ContextError> {
    serde_json::to_string(messages)
        .map(|text| estimate_tokens(&text))
        .map_err(|_| ContextError::RequestEncoding)
}

/// Select only complete projected groups. Current-exchange rows never enter
/// the head, even if one exchange alone exhausts the verbatim budget.
pub(crate) fn admit(
    effective: EffectiveContext,
    current: i64,
) -> Result<Option<Admission>, ContextError> {
    let messages = context::with_summary(effective.summary.as_deref(), &effective.tail)?;
    if !brain_engine_sdk::prompt::should_compact(message_tokens(&messages)?) {
        return Ok(None);
    }
    let budget = brain_engine_sdk::prompt::KEEP_VERBATIM_TOKENS;
    let live = effective
        .tail
        .iter()
        .position(|e| e.exchange_id == Some(current))
        .unwrap_or(effective.tail.len());

    if message_tokens(&context::project(&effective.tail[live..])?)? > budget {
        return Err(ContextError::CompactionTailBudget);
    }
    // Bounded by the selected-event cap. Project candidate slices to preserve
    // exact tool-group correlation rather than estimating a raw JSON boundary.
    for split in 0..=live {
        let Ok(tail_messages) = context::project(&effective.tail[split..]) else {
            continue;
        };
        if message_tokens(&tail_messages)? > budget {
            continue;
        }
        let Ok(head_messages) = context::project(&effective.tail[..split]) else {
            continue;
        };
        if head_messages.is_empty() {
            return Ok(None);
        }
        if tail_messages.is_empty() {
            return Err(ContextError::CompactionTailBudget);
        }
        return Ok(Some(Admission {
            head: effective.tail[..split].to_vec(),
            tail: effective.tail[split..].to_vec(),
            summary: effective.summary,
            supersedes_seq: effective.boundary_seq,
        }));
    }
    Err(ContextError::CompactionTailBudget)
}

pub(crate) fn admission_json(summary: &str, split: &Admission) -> Result<String, ContextError> {
    let boundary = Boundary {
        version: 2,
        summary: shape_summary(summary)?,
        compacted_through_seq: split
            .head
            .last()
            .ok_or(ContextError::CompactionBoundary)?
            .row
            .seq,
        tail_from_seq: split
            .tail
            .first()
            .ok_or(ContextError::CompactionBoundary)?
            .row
            .seq,
        supersedes_seq: split.supersedes_seq,
        tail_seqs: split.tail.iter().map(|e| e.row.seq).collect(),
    };
    let encoded = serde_json::to_string(&boundary).map_err(|_| ContextError::RequestEncoding)?;
    if encoded.len() > context::PAYLOAD_CAP {
        return Err(ContextError::SummaryLimit);
    }
    Ok(encoded)
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
        let cleared = input[0].text();
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
                input.iter().any(|m| m.text() == payload),
                "user turn rides verbatim into the summary input"
            );
        }
    }

    #[test]
    fn context_window_reshapes_around_the_latest_compaction() {
        let events: Vec<_> = vec![
            event(1, "user", "old"),
            event(2, "user", "retained"),
            event(
                3,
                "compaction",
                r#"{"summary":"the brief","compacted_through_seq":1,"tail_from_seq":2}"#,
            ),
            event(4, "user", "after"),
        ]
        .into_iter()
        .map(|row| ContextEvent {
            row,
            exchange_id: None,
            turn: None,
        })
        .collect();
        let effective = reconstruct(&events).unwrap();
        assert_eq!(effective.summary.as_deref(), Some("the brief"));
        assert_eq!(effective.tail, vec![events[1].clone(), events[3].clone()]);
        let whole = reconstruct(&events[..1]).unwrap();
        assert!(whole.summary.is_none());
        assert_eq!(whole.tail, events[..1]);
    }

    fn context_rows(rows: Vec<SessionEventRow>) -> Vec<ContextEvent> {
        rows.into_iter()
            .map(|row| ContextEvent {
                row,
                exchange_id: None,
                turn: None,
            })
            .collect()
    }

    #[test]
    fn effective_pressure_ignores_covered_history_and_empty_head() {
        let mut rows: Vec<_> = (1..=25)
            .map(|seq| event(seq, "user", &big(4_000)))
            .collect();
        rows.push(event(26, "user", "current"));
        rows.push(event(27, "compaction", r#"{"version":2,"summary":"brief","compacted_through_seq":25,"tail_from_seq":26,"supersedes_seq":null,"tail_seqs":[26]}"#));
        assert!(window_tokens(&rows) > 20_000);
        let effective = reconstruct(&context_rows(rows)).unwrap();
        assert!(admit(effective, 99).unwrap().is_none());
        let mut rows = context_rows(vec![event(1, "user", &big(68_000))]);
        rows[0].row.payload_json = big(64_000);
        rows[0].exchange_id = Some(10);
        let effective = reconstruct(&rows).unwrap();
        assert!(admit(effective, 10).unwrap().is_none());
    }

    #[test]
    fn current_exchange_over_tail_budget_refuses() {
        let mut rows = context_rows(vec![
            event(1, "user", &big(50_000)),
            event(2, "user", &big(50_000)),
        ]);
        for e in &mut rows {
            e.exchange_id = Some(10);
        }
        assert!(matches!(
            admit(reconstruct(&rows).unwrap(), 10),
            Err(ContextError::CompactionTailBudget)
        ));
    }

    #[test]
    fn boundary_manifest_detects_missing_rows_but_accepts_scoped_gaps() {
        let rows = context_rows(vec![
            event(1, "user", "covered"),
            event(3, "user", "tail"),
            event(6, "user", "current"),
            event(
                7,
                "compaction",
                r#"{"version":2,"summary":"brief","compacted_through_seq":1,"tail_from_seq":3,"supersedes_seq":null,"tail_seqs":[3,6]}"#,
            ),
        ]);
        assert_eq!(reconstruct(&rows).unwrap().tail, rows[1..3]);
        let mut missing = rows.clone();
        missing.remove(2);
        assert!(matches!(
            reconstruct(&missing),
            Err(ContextError::CompactionBoundary)
        ));
        let mut legacy = rows.clone();
        legacy[3].row.payload_json =
            r#"{"summary":"brief","compacted_through_seq":1,"tail_from_seq":3}"#.into();
        assert!(matches!(
            reconstruct(&legacy),
            Err(ContextError::CompactionBoundary)
        ));
        assert!(matches!(
            reconstruct(&rows[2..]),
            Err(ContextError::CompactionBoundary)
        ));
    }

    #[test]
    fn supersession_cannot_move_coverage_backwards() {
        let rows = context_rows(vec![
            event(1, "user", "covered"),
            event(2, "user", "covered too"),
            event(3, "user", "tail"),
            event(
                4,
                "compaction",
                r#"{"version":2,"summary":"first","compacted_through_seq":2,"tail_from_seq":3,"supersedes_seq":null,"tail_seqs":[3]}"#,
            ),
            event(
                5,
                "compaction",
                r#"{"version":2,"summary":"second","compacted_through_seq":1,"tail_from_seq":2,"supersedes_seq":4,"tail_seqs":[2,3]}"#,
            ),
        ]);
        assert!(matches!(
            reconstruct(&rows),
            Err(ContextError::CompactionBoundary)
        ));
    }

    #[test]
    fn policy_defaults_are_pinned_all_on() {
        const { assert!(DEFAULT_COMPACTION_POLICY.just_before_call) };
        const { assert!(DEFAULT_COMPACTION_POLICY.tool_result_clearing) };
        const { assert!(DEFAULT_COMPACTION_POLICY.selective_retention) };
        assert_eq!(RETAINED_KINDS, &["user"]);
    }
}
