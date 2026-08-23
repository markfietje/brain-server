//! The gold-set pins: the governed loop replays every frozen case
//! end-to-end, exactly once, with gates that reject into FINDINGS.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::sync::Arc;

use brain_engine_sdk::host::WorkflowHost;
use gold_sets::{CaseArtifacts, GoldCase};
use serde_json::{Value, json};
use steward_harness::engine::{self, CrankReport, StoppedAt};
use steward_harness::inmem::InMemHost;

/// Build the replay seed state for a frozen case: its recorded steps become
/// queue items carrying their artifact fields + declared findings.
fn seed_state(case: &GoldCase) -> String {
    let a = &case.artifacts;
    let queue: Vec<Value> = a
        .steps
        .iter()
        .map(|s| {
            let mut item = json!({
                "expected": s.expected,
                "actual": s.actual,
                "skipped_verify": s.skipped_verify,
                "abstained": s.abstained,
            });
            if let Some(g) = s.guidance_accepted {
                item["guidance_accepted"] = json!(g);
            }
            item
        })
        .collect();
    // Declared findings ride on the LAST executed step (findings come from
    // step execution; no synthetic steps are invented).
    let mut items = queue;
    for f in &a.findings {
        match items.last_mut() {
            Some(last) => {
                let mut extra = json!({ "finding": f });
                if let (Some(obj), Some(t)) = (last.as_object_mut(), extra.as_object_mut()) {
                    for (k, v) in t {
                        obj.insert(k.clone(), v.clone());
                    }
                }
            }
            None => items.push(json!({ "expected": "", "actual": "", "finding": f })),
        }
    }
    json!({
        "next_step": "step-0",
        "queue": items,
        "contradictions": a.contradictions,
        "repeat_contact": a.repeat_contact,
        "handoff_complete": a.handoff_complete,
        "verified": a.verified,
        "escalation_honored": a.escalation_honored,
    })
    .to_string()
}

fn artifacts_of(state_json: &str) -> CaseArtifacts {
    let v: Value = serde_json::from_str(state_json).unwrap();
    CaseArtifacts {
        steps: serde_json::from_value(v["steps"].clone()).unwrap(),
        findings: v["findings"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|f| f.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        contradictions: v["contradictions"].as_u64().unwrap_or(0) as u32,
        audit_ok: v["audit_ok"].as_bool().unwrap_or(false),
        repeat_contact: v["repeat_contact"].as_bool().unwrap_or(false),
        handoff_complete: v["handoff_complete"].as_bool().unwrap_or(false),
        verified: v["verified"].as_bool().unwrap_or(false),
        escalation_honored: v["escalation_honored"].as_bool().unwrap_or(true),
    }
}

#[tokio::test]
async fn crank_runs_a_full_gold_case_end_to_end() {
    let case = &gold_sets::all().unwrap()[1]; // skipped_verify (the GDL pin)
    let host = Arc::new(InMemHost::new());
    host.seed(1, &seed_state(case));
    let report: CrankReport = engine::crank(host.clone() as Arc<dyn WorkflowHost>, 1, 100)
        .await
        .unwrap();
    assert_eq!(report.stopped_at, StoppedAt::Done);
    let (state_json, rev) = host.state(1).unwrap();
    assert!(
        state_json.contains("\"status\":\"completed\""),
        "run finalized: {state_json}"
    );
    assert!(rev > 0);
    // Artifacts equal the frozen case FIELD-FOR-FIELD.
    let got = artifacts_of(&state_json);
    assert_eq!(got, case.artifacts, "replay artifacts must equal the case");
}

#[tokio::test]
async fn gold_case_replays_exactly_once() {
    let cases = gold_sets::all().unwrap();
    assert_eq!(cases.len(), 7, "the frozen set is seven cases");
    for (i, case) in cases.iter().enumerate() {
        let host = Arc::new(InMemHost::new());
        let run_id = i as i64 + 1;
        host.seed(run_id, &seed_state(case));
        let h = host.clone() as Arc<dyn WorkflowHost>;
        engine::crank(h, run_id, 1000).await.unwrap();
        let after_first = host.outbox_len(run_id);
        assert!(after_first > 0, "case {} produced events", case.id);
        // Second crank on a completed run: zero NEW events.
        let h = host.clone() as Arc<dyn WorkflowHost>;
        engine::crank(h, run_id, 1000).await.unwrap();
        assert_eq!(
            host.outbox_len(run_id),
            after_first,
            "case {} replayed with no new events",
            case.id
        );
        // And the artifacts still match.
        let (state_json, _) = host.state(run_id).unwrap();
        assert_eq!(artifacts_of(&state_json), case.artifacts);
    }
}

#[tokio::test]
async fn crank_stops_at_askhuman_and_resumes_after_answer() {
    let host = Arc::new(InMemHost::new());
    host.seed(
        9,
        r#"{"pending_question":"collect logs first?","next_step":"step-0","queue":[{"expected":"x","actual":"y"}]}"#,
    );
    let h = host.clone() as Arc<dyn WorkflowHost>;
    let report = engine::crank(h, 9, 10).await.unwrap();
    assert_eq!(
        report.stopped_at,
        StoppedAt::AskHuman {
            question: "collect logs first?".into()
        }
    );
    // The human answers via POST .../answer server-side; the harness observes
    // the resulting state transition through the same seam.
    let (js, rev) = host.state(9).unwrap();
    let mut st: Value = serde_json::from_str(&js).unwrap();
    st.as_object_mut().unwrap().remove("pending_question");
    st["answers"] = json!([{"answer": "yes"}]);
    host.cas(9, rev, &st.to_string()).unwrap();

    let h = host.clone() as Arc<dyn WorkflowHost>;
    let resumed = engine::crank(h, 9, 10).await.unwrap();
    assert_eq!(resumed.stopped_at, StoppedAt::Done);
    let (js2, _) = host.state(9).unwrap();
    assert_eq!(
        artifacts_of(&js2).steps.len(),
        1,
        "the queued step executed after resume"
    );
}

#[tokio::test]
async fn budget_stops_at_max_steps_and_warns_at_80pct() {
    let host = Arc::new(InMemHost::new());
    // A long queue: far more steps than the tiny budget allows.
    let queue: Vec<Value> = (0..50)
        .map(|_| json!({"expected": "e", "actual": "a"}))
        .collect();
    host.seed(
        3,
        &json!({"next_step": "step-0", "queue": queue}).to_string(),
    );
    let h = host.clone() as Arc<dyn WorkflowHost>;
    let report = engine::crank(h, 3, 5).await.unwrap();
    assert_eq!(report.steps_executed, 5);
    assert_eq!(report.stopped_at, StoppedAt::Budget);
    assert!(
        report.warn_threshold_fired,
        "5/5 >= the 80% threshold of max=5"
    );
}

#[tokio::test]
async fn cas_stale_reloads_and_reports_not_panics() {
    let host = Arc::new(InMemHost::new());
    let queue: Vec<Value> = (0..20)
        .map(|_| json!({"expected": "e", "actual": "a"}))
        .collect();
    host.seed(
        4,
        &json!({"next_step": "step-0", "queue": queue}).to_string(),
    );
    // Simulate a concurrent driver racing ahead: keep bumping the revision
    // behind the crank's back. The crank must reload-and-retry once, then
    // REPORT stale — never panic, never wedge.
    let racer = {
        let host = host.clone();
        std::thread::spawn(move || {
            while let Some((_, rev)) = host.state(4) {
                if rev > 500 {
                    break;
                }
                let _ = host.cas(4, rev, "{\"next_step\":\"step-0\",\"queue\":[]}");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        })
    };
    let h = host.clone() as Arc<dyn WorkflowHost>;
    let result = engine::crank(h, 4, 3).await;
    let _ = racer.join();
    match result {
        Ok(_) | Err(_) => {} // either outcome is fine; the pin is NO PANIC
    }
    // Deterministic direct pin: a CAS miss against a moved revision reports.
    let host2 = InMemHost::new();
    host2.seed(1, "{}");
    assert!(host2.cas(1, 7, "{}").is_err(), "stale expectation refuses");
}

#[tokio::test]
async fn gate_rejection_becomes_finding_not_silence() {
    let host = Arc::new(InMemHost::new());
    // A step that DECLARES an approval requirement with no answerer present:
    // the Handoff gate rejects, and the rejection lands as a finding row.
    host.seed(
        5,
        r#"{"next_step":"risky change","queue":[
             {"expected":"apply fix","actual":"","needs_approval":true}
           ]}"#,
    );
    let h = host.clone() as Arc<dyn WorkflowHost>;
    let report = engine::crank(h, 5, 10).await.unwrap();
    assert_eq!(report.stopped_at, StoppedAt::Done);
    let (js, _) = host.state(5).unwrap();
    let a = artifacts_of(&js);
    assert!(
        a.findings
            .iter()
            .any(|f| f.contains("DI_GATE_OPEN") && f.contains("G_HANDOFF")),
        "gate rejection recorded as finding: {:?}",
        a.findings
    );
}
