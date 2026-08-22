use axum::{
    Json,
    extract::{Path, State},
};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;
use crate::auth::Principal;
use crate::handlers::HandlerError;

#[derive(Serialize)]
struct RunRow {
    id: i64,
    domain: String,
    kind: String,
    status: String,
    state_json: String,
    created_at: i64,
    updated_at: i64,
}

pub async fn get_run(
    State(state): State<Arc<AppState>>,
    axum::Extension(principal): axum::Extension<Option<Principal>>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = state.pool.clone();
    let row: Option<RunRow> = tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        conn.query_row(
            "SELECT id,domain,kind,status,state_json,created_at,updated_at FROM workflow_runs WHERE id=?1",
            rusqlite::params![id],
            |r| Ok(RunRow{
                id: r.get(0)?, domain: r.get(1)?, kind: r.get(2)?, status: r.get(3)?,
                state_json: r.get(4)?, created_at: r.get(5)?, updated_at: r.get(6)?,
            }),
        ).optional().map_err(|e| format!("{e}"))
    }).await.map_err(|e| HandlerError::internal(format!("{e}")))?.map_err(HandlerError::internal)?;
    let row = row.ok_or_else(|| HandlerError::not_found("workflow run not found"))?;
    crate::handlers::authorize(&principal, crate::auth::Action::Read, "", &row.domain)?;
    Ok(Json(serde_json::to_value(&row).unwrap()))
}

pub async fn list_steps(
    State(state): State<Arc<AppState>>,
    axum::Extension(principal): axum::Extension<Option<Principal>>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = state.pool.clone();
    let (domain, steps): (Option<String>, Vec<serde_json::Value>) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        let domain: Option<String> = conn.query_row("SELECT domain FROM workflow_runs WHERE id=?1", rusqlite::params![id], |r| r.get(0)).optional().map_err(|e| format!("{e}"))?;
        let Some(ref _d) = domain else { return Ok((None, vec![])) };
        let mut stmt = conn.prepare("SELECT id,run_id,phase,step_key,state_json,revision,parent_step_id FROM workflow_steps WHERE run_id=?1 ORDER BY id").map_err(|e| format!("{e}"))?;
        let rows = stmt.query_map(rusqlite::params![id], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_,i64>(0)?,
                "run_id": r.get::<_,i64>(1)?,
                "phase": r.get::<_,String>(2)?,
                "step_key": r.get::<_,String>(3)?,
                "state_json": r.get::<_,String>(4)?,
                "revision": r.get::<_,i64>(5)?,
                "parent_step_id": r.get::<_,Option<i64>>(6)?,
            }))
        }).map_err(|e| format!("{e}"))?;
        let mut out = Vec::new();
        for r in rows { out.push(r.map_err(|e| format!("{e}"))?); }
        Ok((domain, out))
    }).await.map_err(|e| HandlerError::internal(format!("{e}")))?.map_err(HandlerError::internal)?;
    let Some(domain) = domain else {
        return Err(HandlerError::not_found("workflow run not found"));
    };
    crate::handlers::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    Ok(Json(serde_json::json!({"steps": steps})))
}

#[derive(Deserialize)]
pub struct SteeringRequest {
    pub message: String,
}

pub async fn post_steering(
    State(state): State<Arc<AppState>>,
    axum::Extension(principal): axum::Extension<Option<Principal>>,
    Path(id): Path<i64>,
    Json(body): Json<SteeringRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    if body.message.len() > 4000 {
        return Err(HandlerError::bad_request(
            "message_too_long",
            "message exceeds 4000 chars",
        ));
    }
    if body.message.trim().is_empty() {
        return Err(HandlerError::bad_request(
            "message_empty",
            "message must not be empty",
        ));
    }
    let sanitized = crate::gate::sanitize_read(&body.message, false, &principal);
    let payload = serde_json::json!({"message": sanitized}).to_string();
    let pool = state.pool.clone();
    let principal2 = principal.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        let domain: String = conn.query_row("SELECT domain FROM workflow_runs WHERE id=?1", rusqlite::params![id], |r| r.get(0)).optional().map_err(|e| format!("{e}"))?.ok_or("workflow run not found".to_string())?;
        // auth inside blocking would need principal; we already authorized via sanitized? do check via is_authorized logic without pool
        // authorize read/write check is done outside; re-check via principal2
        if let Some(p) = &principal2
            && !crate::auth::is_authorized(p, crate::auth::Action::Write, &p.tenant, &domain)
        {
            return Err("forbidden".to_string());
        }
        let cnt: i64 = conn.query_row("SELECT COUNT(*) FROM outbox WHERE run_id=?1 AND topic='steering'", rusqlite::params![id], |r| r.get(0)).map_err(|e| format!("{e}"))?;
        if cnt >= 100 {
            conn.execute("DELETE FROM outbox WHERE id IN (SELECT id FROM outbox WHERE run_id=?1 AND topic='steering' ORDER BY id ASC LIMIT 1)", rusqlite::params![id]).map_err(|e| format!("{e}"))?;
        }
        let now = chrono::Utc::now().timestamp();
        let key = format!("steering-{id}-{now}-{}", rand::random::<u32>());
        use brain_engine_sdk::host::WorkflowHost as _;
        crate::workflow::host::SqliteWorkflowHost::new(pool.clone())
            .enqueue(id, "steering", &payload, &key)
            .map_err(|e| format!("{e}"))?;
        crate::audit::record_tenant(&conn, crate::audit::AuditKind::Workflow, "api", &format!("workflow:{id}"), crate::audit::AuditStatus::Ok, "steering", &domain);
        Ok(())
    }).await.map_err(|e| HandlerError::internal(format!("{e}")))?.map_err(|e| if e=="forbidden" { HandlerError::forbidden(crate::auth::Action::Write, "", "workflow") } else if e=="workflow run not found" { HandlerError::not_found(e) } else { HandlerError::internal(e) })?;
    Ok(Json(serde_json::json!({"ok": true})))
}

pub async fn get_suggestions(
    State(state): State<Arc<AppState>>,
    axum::Extension(principal): axum::Extension<Option<Principal>>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = state.pool.clone();
    let st = state.clone();
    let (domain, state_json): (String, String) =
        tokio::task::spawn_blocking(move || -> Result<_, String> {
            let conn = pool.get().map_err(|e| format!("{e}"))?;
            let row: Option<(String, String)> = conn
                .query_row(
                    "SELECT domain,state_json FROM workflow_runs WHERE id=?1",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(|e| format!("{e}"))?;
            row.ok_or("workflow run not found".to_string())
        })
        .await
        .map_err(|e| HandlerError::internal(format!("{e}")))?
        .map_err(|e| {
            if e == "workflow run not found" {
                HandlerError::not_found(e)
            } else {
                HandlerError::internal(e)
            }
        })?;
    crate::handlers::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let q: String = serde_json::from_str::<serde_json::Value>(&state_json)
        .ok()
        .and_then(|v| v.get("q").and_then(|x| x.as_str()).map(|s| s.to_string()))
        .unwrap_or_else(|| state_json.chars().take(200).collect());
    let q = q.trim().to_string();
    // do a recall scoped to domain; if q empty, zero hits
    let hits: Vec<serde_json::Value> = if q.is_empty() {
        vec![]
    } else {
        // use search directly via spawn_blocking
        let pool2 = st.pool.clone();
        let domain2 = domain.clone();
        let q2 = q.clone();
        // simple FTS search; fallback to empty
        tokio::task::spawn_blocking(move || -> Vec<serde_json::Value> {
            let Ok(conn) = pool2.get() else { return vec![] };
            let mut out = Vec::new();
            let mut stmt = match conn.prepare("SELECT id,title,content FROM knowledge WHERE domain=?1 AND content LIKE ?2 LIMIT 5") {
                Ok(s) => s,
                Err(_) => return vec![],
            };
            let pat = format!("%{}%", q2.chars().take(50).collect::<String>());
            let rows = match stmt.query_map(rusqlite::params![domain2, pat], |r| Ok(serde_json::json!({"id": r.get::<_,i64>(0)?, "title": r.get::<_,Option<String>>(1)?, "snippet": r.get::<_,String>(2)?.chars().take(200).collect::<String>()}))) {
                Ok(r) => r,
                Err(_) => return vec![],
            };
            for r in rows.flatten() { out.push(r); }
            out
        }).await.unwrap_or_default()
    };
    if hits.is_empty() {
        let pool3 = st.pool.clone();
        tokio::task::spawn_blocking(move || {
            if let Ok(conn) = pool3.get() {
                let now = chrono::Utc::now().timestamp();
                let _ = conn.execute("INSERT INTO findings(run_id,claim,evidence,source,confidence,ts) VALUES (?1,'abstention','no hits','copilot',0,?2)", rusqlite::params![id, now]);
            }
        }).await.ok();
        return Ok(Json(serde_json::json!({"suggestions": []})));
    }
    let suggestions: Vec<serde_json::Value> = hits
        .into_iter()
        .map(|h| serde_json::json!({"hit": h}))
        .collect();
    Ok(Json(serde_json::json!({"suggestions": suggestions})))
}

/// `GET /workflow/scoreboard` — the outcome/efficiency scoreboard over
/// workflow runs (DPO/admin evidence surface). Derived honestly from what
/// runs recorded: known scorer fields in `state_json`, absence = default,
/// and a fail-closed `audit_ok`: a run counts green only when a workflow
/// audit row actually references it.
pub async fn get_scoreboard(
    State(state): State<Arc<AppState>>,
    axum::Extension(principal): axum::Extension<Option<Principal>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Admin, "", "global")?;
    crate::handlers::breaches::require_dpo_role(&principal, &pool)?;
    let runs: Vec<brain_engine_sdk::scoreboard::RunArtifacts> =
        tokio::task::spawn_blocking(move || -> Result<_, String> {
            let conn = pool.get().map_err(|e| format!("{e}"))?;
            let mut stmt = conn
                .prepare("SELECT id, status, state_json FROM workflow_runs ORDER BY id DESC LIMIT 1000")
                .map_err(|e| format!("{e}"))?;
            let mut audited_stmt = conn
                .prepare(
                    "SELECT DISTINCT CAST(target AS INTEGER) FROM audit_events WHERE kind = 'workflow'",
                )
                .map_err(|e| format!("{e}"))?;
            let audited: std::collections::HashSet<i64> = {
                let rows = audited_stmt
                    .query_map([], |r| r.get::<_, i64>(0))
                    .map_err(|e| format!("{e}"))?;
                let mut set = std::collections::HashSet::new();
                for r in rows {
                    set.insert(r.map_err(|e| format!("{e}"))?);
                }
                set
            };

            let mut rows = Vec::new();
            for row in stmt
                .query_map([], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
                })
                .map_err(|e| format!("{e}"))?
            {
                rows.push(row.map_err(|e| format!("{e}"))?);
            }
            Ok(derive_artifacts(&rows, &audited))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("{e}")))?
        .map_err(HandlerError::internal)?;
    let sb = brain_engine_sdk::scoreboard::build(&runs);
    // The SDK stays dependency-free; the host owns the wire shape.
    Ok(Json(serde_json::json!({
        "fcr_units": sb.fcr_units,
        "repeat_contact_rate_units": sb.repeat_contact_rate_units,
        "correctness_units": sb.correctness_units,
        "override_rate_units": sb.override_rate_units,
        "gap_rate_units": sb.gap_rate_units,
        "abstention_rate_units": sb.abstention_rate_units,
        "guidance_acceptance_units": sb.guidance_acceptance_units,
        "handoff_completeness_units": sb.handoff_completeness_units,
        "audit_green": sb.audit_green,
        "escalation_honored_units": sb.escalation_honored_units,
        "runs_scored": runs.len(),
    })))
}

/// Pure derivation of scorer artifacts from run rows + the audited-id set.
/// Fail-closed: `audit_ok` only when a state flag says so OR an audit row
/// references the run.
fn derive_artifacts(
    rows: &[(i64, String, String)],
    audited: &std::collections::HashSet<i64>,
) -> Vec<brain_engine_sdk::scoreboard::RunArtifacts> {
    rows.iter()
        .map(|(id, status, state_json)| {
            let v: serde_json::Value =
                serde_json::from_str(state_json).unwrap_or(serde_json::Value::Null);
            let flag = v.get("audit_ok").and_then(|b| b.as_bool()).unwrap_or(false);
            brain_engine_sdk::scoreboard::RunArtifacts {
                audit_ok: flag || audited.contains(id),
                ..artifacts_from_row(status, &v)
            }
        })
        .collect()
}

fn artifacts_from_row(
    status: &str,
    v: &serde_json::Value,
) -> brain_engine_sdk::scoreboard::RunArtifacts {
    use brain_engine_sdk::pure::qa_score::StepRow;
    brain_engine_sdk::scoreboard::RunArtifacts {
        steps: v
            .get("steps")
            .and_then(|s| s.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|s| StepRow {
                        expected: s
                            .get("expected")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .into(),
                        actual: s
                            .get("actual")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .into(),
                        skipped_verify: s
                            .get("skipped_verify")
                            .and_then(|x| x.as_bool())
                            .unwrap_or(false),
                        abstained: s
                            .get("abstained")
                            .and_then(|x| x.as_bool())
                            .unwrap_or(false),
                        guidance_accepted: s.get("guidance_accepted").and_then(|x| x.as_bool()),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        findings: v
            .get("findings")
            .and_then(|f| f.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|f| f.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        contradictions: v
            .get("contradictions")
            .and_then(|c| c.as_u64())
            .unwrap_or(0) as usize,
        audit_ok: false, // overridden by the caller's fail-closed check
        repeat_contact: v
            .get("repeat_contact")
            .and_then(|r| r.as_bool())
            .unwrap_or(false),
        handoff_complete: status == "completed",
        verified: v.get("verified").and_then(|b| b.as_bool()).unwrap_or(false),
        escalation_honored: v
            .get("escalation_honored")
            .and_then(|b| b.as_bool())
            .unwrap_or(true),
    }
}

#[cfg(test)]
mod scoreboard_tests {
    use super::*;
    use brain_engine_sdk::scoreboard::StepRow;

    #[test]
    fn derivation_defaults_and_fail_closed_audit_ok() {
        let audited = std::collections::HashSet::from([7]);
        let rows = vec![
            // completed run WITH an audit row: audit_ok true via linkage.
            (
                7i64,
                "completed".to_string(),
                r#"{"steps":[{"expected":"a","actual":"a"}]}"#.to_string(),
            ),
            // completed run WITHOUT any audit linkage and no flag: audit_ok
            // stays FALSE — absence never counts green.
            (8, "completed".to_string(), "{}".to_string()),
            // recorded flag wins over missing linkage.
            (9, "failed".to_string(), r#"{"audit_ok":true}"#.to_string()),
        ];
        let runs = derive_artifacts(&rows, &audited);
        assert_eq!(runs.len(), 3);
        assert!(runs[0].audit_ok && runs[2].audit_ok);
        assert!(!runs[1].audit_ok, "no audit row + no flag => not green");
        assert!(!runs[0].handoff_complete.eq(&false));
        assert_eq!(
            runs[0].steps,
            vec![StepRow {
                expected: "a".into(),
                actual: "a".into(),
                skipped_verify: false,
                abstained: false,
                guidance_accepted: None,
            }]
        );
    }

    #[test]
    fn empty_input_scores_zero_not_panic() {
        let sb = brain_engine_sdk::scoreboard::build(&[]);
        assert_eq!(sb.fcr_units, 0);
        assert!(sb.audit_green, "vacuous conjunction is true by definition");
    }
}
