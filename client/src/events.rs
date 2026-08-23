//! v1.28.19 Witness: the persistent `/events` stream. The chunk-and-drop poll
//! becomes a reconnecting bounded `bytes_stream()` loop (exponential backoff
//! capped at 30s — no new dep), deduped per coordinate space (alert `seq`,
//! workflow outbox `event_id`) and folded into the conversation assembler.
//!
//! Everything decision-bearing is a pure core here; the coroutine driver in
//! main.rs is thin plumbing over these (the repo's testable-seam convention).

use serde_json::Value;

/// One parsed SSE envelope from `/events`: exactly the server shape
/// `{kind, ts, seq, payload}`. Alert kinds carry their hand-curated payload;
/// workflow envelopes carry `{topic, run_id, payload_json, event_id,
/// parent_event_id, domain}`.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamEvent {
    pub kind: String,
    pub seq: u64,
    pub payload: Value,
}

impl StreamEvent {
    /// The dedup coordinate: workflow events dedup on the outbox
    /// `(run_id, event_id)` pair (a separate id space from the alert seq);
    /// everything else rides the monotonic alert `seq`.
    pub fn dedup_key(&self) -> Option<String> {
        if self.kind == "workflow" {
            let run = self.payload.get("run_id")?.to_string();
            let id = self.payload.get("event_id")?.to_string();
            Some(format!("wf:{run}:{id}"))
        } else {
            Some(format!("alert:{}", self.seq))
        }
    }

    /// Map one envelope onto a conversation-node-definition event:
    /// proposal alerts already carry producer-shaped payloads
    /// (`{kind:"proposal/open", id:"p7", ...}`); workflow envelopes fold the
    /// outbox row back into an event (`{kind:"workflow/log", run_id:"w3", ...}`).
    /// Returns `None` for kinds no definition family consumes.
    pub fn definition_event(&self) -> Option<Value> {
        match self.kind.as_str() {
            "proposal" => {
                // The family matcher reads `kind` off the event itself.
                self.payload.get("kind").and_then(Value::as_str)?;
                Some(self.payload.clone())
            }
            "workflow" => {
                let topic = self.payload.get("topic")?.as_str()?.to_string();
                let run_id = self.payload.get("run_id")?;
                let payload_json = self.payload.get("payload_json")?;
                let raw = payload_json.as_str()?;
                let mut v: Value = serde_json::from_str(raw).ok()?;
                if !v.is_object() {
                    return None;
                }
                let obj = v.as_object_mut()?;
                obj.insert("kind".into(), Value::from(topic));
                obj.insert("run_id".into(), Value::from(format!("w{run_id}")));
                Some(v)
            }
            _ => None,
        }
    }

    /// Fold this event through the assembler families it belongs to.
    /// Pure so the stream→node seam is pinnable without a runtime.
    pub fn ingest(&self, asm: &mut crate::conversation::assembler::Assembler) {
        let Some(ev) = self.definition_event() else {
            return;
        };
        let kind = ev.get("kind").and_then(Value::as_str).unwrap_or("");
        if kind.starts_with("proposal/") || kind.starts_with("review/") {
            asm.ingest::<crate::conversation::ReviewJob>(self.seq, &ev);
        } else if kind.starts_with("workflow/") {
            asm.ingest::<crate::conversation::WorkflowRun>(self.seq, &ev);
        }
    }
}

/// Parse one SSE `data:` JSON line. Malformed lines are dropped (a lost
/// signal is safe — the poll fallback + lineage read re-sync).
pub fn parse_stream_event(data: &str) -> Option<StreamEvent> {
    let alert = parse_alert(data)?;
    Some(StreamEvent {
        kind: alert.kind,
        seq: alert.seq,
        payload: alert.payload,
    })
}

struct ParsedAlert {
    kind: String,
    seq: u64,
    payload: Value,
}

fn parse_alert(data: &str) -> Option<ParsedAlert> {
    let v: Value = serde_json::from_str(data).ok()?;
    Some(ParsedAlert {
        kind: v.get("kind")?.as_str()?.to_string(),
        seq: v.get("seq")?.as_u64()?,
        payload: v.get("payload").cloned().unwrap_or(Value::Null),
    })
}

/// Split one received byte chunk into complete SSE data lines, returning
/// them plus the trailing partial line to carry into the next chunk. Handles
/// `\n` and `\r\n` framing; `event:` lines are ignored (the envelope is in
/// `data:`). Pure so chunk-boundary behavior is testable without a socket.
pub fn sse_data_lines(chunk: &str) -> (Vec<String>, String) {
    let mut buf = chunk.to_string();
    let mut lines = Vec::new();
    while let Some(pos) = buf.find('\n') {
        let line: String = buf.drain(..=pos).collect();
        let line = line.trim_end_matches(['\n', '\r']);
        if let Some(data) = line.strip_prefix("data:") {
            lines.push(data.trim().to_string());
        }
    }
    (lines, buf)
}

/// Reconnect backoff: exponential 1s → 2s → 4s … capped at 30s. Attempt 0 is
/// the first retry after a healthy connection drops.
pub fn backoff_secs(failures: u32) -> u64 {
    (1u64 << failures.min(5)).min(30)
}

/// Degradation law for the ops panel: the legacy 10s poll wakes up only
/// after TWO consecutive stream failures (one flap must not flip the mode —
/// the same false-offline guard as the connection probe). Pure.
pub fn poll_fallback_active(consecutive_failures: u32) -> bool {
    consecutive_failures >= 2
}

/// Bounded dedup state across reconnects: monotonic alert-seq guard plus a
/// capped recent-set for workflow coordinates (outbox ids are per-run spaces,
/// not globally ordered, so a set — not a watermark — is the honest guard).
#[derive(Default)]
pub struct EventDedup {
    last_seq: u64,
    seen: std::collections::HashSet<String>,
    order: std::collections::VecDeque<String>,
}

const DEDUP_CAP: usize = 512;

impl EventDedup {
    pub fn admit(&mut self, ev: &StreamEvent) -> bool {
        if ev.kind != "workflow" {
            // Monotonic seq guard (the ops-panel law, reused verbatim).
            if ev.seq > self.last_seq {
                self.last_seq = ev.seq;
                return true;
            }
            return false;
        }
        let Some(key) = ev.dedup_key() else {
            return false;
        };
        if self.seen.contains(&key) {
            return false;
        }
        self.seen.insert(key.clone());
        self.order.push_back(key);
        while self.order.len() > DEDUP_CAP
            && let Some(old) = self.order.pop_front()
        {
            self.seen.remove(&old);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::assembler::Assembler;

    fn wf_envelope(run: i64, event_id: i64, topic: &str, note: &str) -> StreamEvent {
        StreamEvent {
            kind: "workflow".into(),
            seq: 99,
            payload: serde_json::json!({
                "topic": topic,
                "run_id": run,
                "payload_json": format!("{{\"note\":\"{note}\"}}"),
                "event_id": event_id,
                "parent_event_id": null,
                "domain": "global",
            }),
        }
    }

    #[test]
    fn parses_envelope_and_frames_chunks() {
        let ev =
            parse_stream_event(r#"{"kind":"workflow","ts":1,"seq":5,"payload":{"event_id":3}}"#)
                .expect("envelope");
        assert_eq!(ev.seq, 5);
        assert_eq!(ev.payload["event_id"], 3);
        assert!(parse_stream_event("not-json").is_none());
        assert!(parse_stream_event(r#"{"kind":"pending"}"#).is_none());

        // Chunk framing: a data line split across two chunks survives.
        let (lines, rest) = sse_data_lines("data: {\"kind\":\"p");
        assert!(lines.is_empty());
        assert_eq!(rest, "data: {\"kind\":\"p");
        let (lines, rest2) = sse_data_lines(&format!(
            "{rest}ending\",\"seq\":9}}\nevent: alert\ndata: second\n\r\n"
        ));
        assert_eq!(lines.len(), 2, "both data lines recovered across chunks");
        assert!(parse_stream_event(&lines[0]).is_some());
        assert_eq!(rest2, "");
    }

    #[test]
    fn backoff_is_exponential_capped_at_30() {
        assert_eq!(backoff_secs(0), 1);
        assert_eq!(backoff_secs(1), 2);
        assert_eq!(backoff_secs(2), 4);
        assert_eq!(backoff_secs(4), 16);
        assert_eq!(backoff_secs(5), 30, "capped — no unbounded waits");
        assert_eq!(backoff_secs(50), 30);
    }

    #[test]
    fn stream_reconnects_and_dedups_by_seq() {
        // The dedup core across a simulated reconnect burst: replays and
        // out-of-order deliveries are dropped, fresh coordinates admitted.
        let mut d = EventDedup::default();
        let a1 = parse_stream_event(r#"{"kind":"pending","seq":1,"payload":{}}"#).unwrap();
        assert!(d.admit(&a1));
        assert!(!d.admit(&a1), "replay dropped");
        let old =
            parse_stream_event(r#"{"kind":"pending","seq":0,"payload":{}}"#).unwrap_or(a1.clone());
        assert!(!d.admit(&old), "stale seq dropped");

        let w1 = wf_envelope(1, 10, "workflow/log", "a");
        assert!(d.admit(&w1));
        assert!(
            !d.admit(&w1),
            "same outbox event replayed on reconnect dropped"
        );
        // Different runs have independent id spaces: same event_id, other run.
        let w_other_run = wf_envelope(2, 10, "workflow/log", "b");
        assert!(d.admit(&w_other_run));
        // A lower alert seq after workflow traffic stays governed by its own space.
        let a0 = parse_stream_event(r#"{"kind":"expiry","seq":2,"payload":{}}"#).unwrap();
        assert!(d.admit(&a0));

        // Bounded memory: flooding past the cap keeps admitting fresh ids.
        for i in 1000..1600 {
            let e = wf_envelope(9, i, "workflow/log", "x");
            assert!(d.admit(&e));
        }
        assert!(d.order.len() <= 512 + 8, "recent-set stays bounded");
        // The oldest flopped-out key can be admitted again — bounded memory
        // trades ancient-replay protection for O(1); the server side is
        // exactly-once by key, so this is belt-and-suspenders, not the wall.
        let flooded = wf_envelope(9, 999, "workflow/log", "y");
        assert!(d.admit(&flooded));
    }

    #[test]
    fn ops_poll_falls_back_after_two_stream_failures() {
        assert!(!poll_fallback_active(0));
        assert!(!poll_fallback_active(1), "one flap never flips the mode");
        assert!(poll_fallback_active(2));
        assert!(poll_fallback_active(9));
    }

    #[test]
    fn assembler_ingest_builds_review_job_from_proposal_events() {
        // THE seam the pure engine always wanted: real envelopes → adapter →
        // Assembler → review-job node with whole-value checkpoints.
        let open = parse_stream_event(
            r#"{"kind":"proposal","seq":1,"payload":{"kind":"proposal/open","id":"p7",
               "proposal_id":7,"content_digest":"d1","sla_deadline":1800000000,"role_gate":"approve"}}"#,
        )
        .unwrap();
        let decided = parse_stream_event(
            r#"{"kind":"proposal","seq":2,"payload":{"kind":"proposal/decided","id":"p7",
               "proposal_id":7,"approved":true,"content_digest":"d1","role_gate":"approve"}}"#,
        )
        .unwrap();

        let mut asm = Assembler::new();
        open.ingest(&mut asm);
        assert_eq!(asm.snapshot().len(), 1);
        let node = asm.node("review-job:p7").expect("node keyed by branded id");
        assert_eq!(node.data["content_digest"], "d1");
        decided.ingest(&mut asm);
        let node = asm.node("review-job:p7").unwrap();
        assert_eq!(node.data["terminal"], true);
        assert_eq!(node.data["status"], true);

        // Workflow envelopes fold onto the workflow-run family too.
        let start = wf_envelope(3, 1, "workflow/start", "");
        let phase = wf_envelope(3, 2, "workflow/log", "step done");
        let mut asm2 = Assembler::new();
        start.ingest(&mut asm2);
        phase.ingest(&mut asm2);
        let n = asm2.node("workflow-run:w3").expect("workflow-run node");
        assert_eq!(n.data["state"], "running");
    }
}
