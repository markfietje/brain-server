//! steward-harness RPC — line-delimited JSON over stdin/stdout.
//!
//! Commands (superset of the 0.1 stub):
//!   open-run    {domain, seed?}          -> {ok, run_id}
//!   crank       {run_id, max_steps?}     -> {ok, stopped_at, steps_executed, revision?}
//!   ask-human   {run_id, answer, digest} -> {ok}   (POST .../answer)
//!   step-result {run_id, expected_rev, state_json} -> {ok, revision}  (PUT state)
//!   advance     {run_id, next_state}     -> {ok, revision}               (PUT state)
//!
//! Every response carries `{ok, run_id?, stopped_at?, revision?}`.

#![forbid(unsafe_code)]

use std::io::{BufRead, BufReader};
use std::sync::Arc;

use serde_json::{Value, json};
use steward_harness::engine;
use steward_harness::remote_host::{RemoteWorkflowHost, resolve_token};

fn main() {
    let base = std::env::var("BRAIN_URL").unwrap_or_default();
    let token = resolve_token();
    let host = match RemoteWorkflowHost::new(base, token) {
        Ok(h) => Arc::new(h),
        Err(e) => {
            eprintln!("harness: {e}");
            std::process::exit(2);
        }
    };
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("harness: runtime: {e}");
            std::process::exit(2);
        }
    };
    let stdin = std::io::stdin();
    for line in BufReader::new(stdin).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        let resp = rt.block_on(handle_rpc(&host, &v));
        println!("{resp}");
    }
}

async fn handle_rpc(host: &Arc<RemoteWorkflowHost>, v: &Value) -> Value {
    use brain_engine_sdk::host::WorkflowHost as _;
    let cmd = v.get("cmd").and_then(|x| x.as_str()).unwrap_or("");
    match cmd {
        "open-run" => {
            let domain = v.get("domain").and_then(|x| x.as_str()).unwrap_or("global");
            let state_json = v
                .get("seed")
                .and_then(|x| x.as_str())
                .unwrap_or("{}")
                .to_string();
            match host
                .open_run(domain.to_string(), "troubleshoot".to_string(), state_json)
                .await
            {
                Ok(run_id) => json!({"ok": true, "run_id": run_id}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            }
        }
        "crank" => {
            let Some(run_id) = v.get("run_id").and_then(|x| x.as_i64()) else {
                return json!({"ok": false, "error": "missing run_id"});
            };
            let env_max = std::env::var("BRAIN_MAX_STEPS")
                .ok()
                .and_then(|s| s.parse::<u32>().ok());
            let max_steps = engine::resolve_budget(env_max);
            match engine::crank_with_steering(host.clone(), Some(host.clone()), run_id, max_steps)
                .await
            {
                Ok(report) => json!({
                    "ok": true,
                    "run_id": run_id,
                    "stopped_at": report.stopped_at.as_str(),
                    "steps_executed": report.steps_executed,
                    "warn_threshold_fired": report.warn_threshold_fired,
                }),
                Err(e) => json!({"ok": false, "run_id": run_id, "error": e.to_string()}),
            }
        }
        "ask-human" => {
            let run_id = v.get("run_id").and_then(|x| x.as_i64()).unwrap_or(0);
            let body = json!({
                "answer": v.get("answer").and_then(|x| x.as_str()).unwrap_or(""),
                "question_digest": v.get("digest").and_then(|x| x.as_str()).unwrap_or(""),
            });
            match host.answer(run_id, body).await {
                Ok(revision) => json!({"ok": true, "run_id": run_id, "revision": revision}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            }
        }
        // step-result / advance both map to the CAS PUT.
        "step-result" | "advance" => {
            let run_id = v.get("run_id").and_then(|x| x.as_i64()).unwrap_or(0);
            let expected_rev = v.get("expected_rev").and_then(|x| x.as_i64()).unwrap_or(0);
            let state_json = v
                .get("state_json")
                .or_else(|| v.get("next_state"))
                .and_then(|x| x.as_str())
                .unwrap_or("{}")
                .to_string();
            match host.cas(run_id, expected_rev, &state_json) {
                Ok(()) => match host.load_state(run_id) {
                    Ok(Some((_, rev))) => {
                        json!({"ok": true, "run_id": run_id, "revision": rev})
                    }
                    _ => json!({"ok": true, "run_id": run_id}),
                },
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            }
        }
        _ => json!({"ok": false, "error": "unknown cmd"}),
    }
}
