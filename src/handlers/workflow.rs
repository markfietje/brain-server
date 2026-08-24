//! Governed-workflow surfaces.
//!
//! Alongside the human/operator views above, this
//! module now carries the ENGINE-facing substrate projections —
//! open/state/events/answer. These are storage projections over
//! [`crate::workflow`] primitives (WorkflowTx / outbox / cas_update), NOT
//! engine code: no decision logic lives server-side; the steward-harness
//! drives them over HTTP through the SDK `WorkflowHost` seam.

use axum::{
    Json,
    extract::{Path, State},
};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;
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
    principal: crate::handlers::auth::OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
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
    // The read seam covers EVERY stored-text surface — run state included
    // (the one seam that previously skipped it).
    let mut row = row;
    row.state_json = crate::gate::sanitize_read(&row.state_json, false, &principal);
    Ok(Json(serde_json::to_value(&row).map_err(|e| {
        HandlerError::internal(format!("serialize: {e}"))
    })?))
}

pub async fn list_steps(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
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
    // Read-seam parity with get_run: step state is stored text.
    let steps: Vec<serde_json::Value> = steps
        .into_iter()
        .map(|mut s| {
            if let Some(v) = s.get_mut("state_json")
                && let Some(raw) = v.as_str().map(str::to_string)
            {
                *v = serde_json::Value::String(crate::gate::sanitize_read(&raw, false, &principal));
            }
            s
        })
        .collect();
    Ok(Json(serde_json::json!({"steps": steps})))
}

#[derive(Deserialize)]
pub struct SteeringRequest {
    pub message: String,
}

pub async fn post_steering(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<SteeringRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
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
    // Steering text drives an engine state machine — screen it like any other
    // untrusted ingest BEFORE it can reach the outbox (prompt-injection class).
    if crate::contains_suspicious_pattern(&body.message) {
        return Err(HandlerError::bad_request(
            "steering_rejected",
            "steering message matches a blocked prompt-injection pattern",
        ));
    }
    let pool = state.pool.clone();
    // Resolve the run's domain first so the STANDARD gates apply: the shared
    // `authorize` Write check (loopback `None` = superuser is the documented
    // ambient posture for local harness processes) plus the HITL-class role
    // gate — steering shapes decisions, so a token whose roles omit the
    // approve capability may not steer, exactly as it may not approve.
    let domain: String = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        conn.query_row(
            "SELECT domain FROM workflow_runs WHERE id=?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| "workflow run not found".to_string())
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
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    super::authorize_role(&principal, &state.pool, "approve")?;

    let sanitized = crate::gate::sanitize_read(&body.message, false, &principal);
    let payload = serde_json::json!({"message": sanitized}).to_string();
    let pool = state.pool.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut conn = pool.get().map_err(|e| format!("{e}"))?;
        // The cap and the enqueue commit atomically: drop-oldest + insert in
        // one tx on one connection so the inbox bound can never race past 100.
        let tx = conn.transaction().map_err(|e| format!("{e}"))?;
        let cnt: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE run_id=?1 AND topic='steering'",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .map_err(|e| format!("{e}"))?;
        if cnt >= 100 {
            tx.execute(
                "DELETE FROM outbox WHERE id IN (SELECT id FROM outbox WHERE run_id=?1 AND topic='steering' ORDER BY id ASC LIMIT ?2)",
                rusqlite::params![id, cnt - 99],
            )
            .map_err(|e| format!("{e}"))?;
        }
        let now = chrono::Utc::now().timestamp();
        let key = format!("steering-{id}-{now}-{}", rand::random::<u32>());
        crate::workflow::outbox::enqueue(&tx, id, "steering", &payload, &key, now)
            .map_err(|e| format!("{e}"))?;
        tx.commit().map_err(|e| format!("{e}"))?;
        Ok(())
    }).await.map_err(|e| HandlerError::internal(format!("{e}")))?.map_err(HandlerError::internal)?;
    Ok(Json(serde_json::json!({"ok": true})))
}

pub async fn get_suggestions(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Path(id): Path<i64>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
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
        let reader = principal.clone();
        // simple FTS search; fallback to empty
        tokio::task::spawn_blocking(move || -> Vec<serde_json::Value> {
            let Ok(conn) = pool2.get() else { return vec![] };
            let mut out = Vec::new();
            // Quarantine + decay posture (P0-1 read seam): flagged content is
            // never retrievable through a side door, and expired rows stay
            // retired. The LIKE pattern escapes `%`/`_`/`\` so the run's `q`
            // cannot inject wildcards (data-shape class).
            let mut stmt = match conn.prepare(
                "SELECT id,title,content FROM knowledge \
                 WHERE domain=?1 AND flagged=0 \
                   AND (expires_at IS NULL OR expires_at >= ?3) \
                   AND content LIKE ?2 ESCAPE '\\' LIMIT 5",
            ) {
                Ok(s) => s,
                Err(_) => return vec![],
            };
            let q_take: String = q2.chars().take(50).collect();
            let pat = format!(
                "%{}%",
                q_take
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            );
            let now = chrono::Utc::now().timestamp();
            let rows = match stmt.query_map(rusqlite::params![domain2, pat, now], |r| {
                let id: i64 = r.get(0)?;
                let title: Option<String> = r.get(1)?;
                let content: String = r.get(2)?;
                Ok((id, title, content))
            }) {
                Ok(r) => r,
                Err(_) => return vec![],
            };
            for (id, title, content) in rows.flatten() {
                // Read-seam parity with get_run/list_steps: suggestion output
                // feeds engine context, so title + snippet go through the
                // same PII-redact → markdown-ref → invisible-strip chain.
                let snippet: String = crate::gate::sanitize_read(&content, false, &reader)
                    .chars()
                    .take(200)
                    .collect();
                out.push(serde_json::json!({
                    "id": id,
                    "title": title
                        .as_deref()
                        .map(|t| crate::gate::sanitize_read(t, false, &reader)),
                    "snippet": snippet,
                }));
            }
            out
        })
        .await
        .unwrap_or_default()
    };
    if hits.is_empty() {
        let pool3 = st.pool.clone();
        tokio::task::spawn_blocking(move || {
            if let Ok(mut conn) = pool3.get() {
                let now = chrono::Utc::now().timestamp();
                // Never certify silence: a failed abstention record is
                // announced, not swallowed (the silent-write sweep convention).
                if let Err(e) = conn.execute(
                    "INSERT INTO findings(run_id,claim,evidence,source,confidence,ts) VALUES (?1,'abstention','no hits','copilot',0,?2)",
                    rusqlite::params![id, now],
                ) {
                    tracing::warn!("abstention finding write failed for run {id}: {e}");
                }
                // The documented KCS signal for a zero-hit reuse search.
                let n = crate::workflow::kcs::record_sir_not_found(&mut conn, id, now);
                if n == 0 && crate::workflow::kcs::case_ref_for_run(&conn, id).is_some() {
                    tracing::warn!("sir not-found record failed for run {id}");
                }
            }
        }).await.ok();
        return Ok(Json(serde_json::json!({"suggestions": []})));
    }
    // The Reuse practice: cited hits (the `used` id list the engine sends
    // back once it actually cites them in step evidence) land `searched_found`
    // SIR rows. Best-effort; a failed record reads as a gap, never a fork.
    if let Some(used) = params.get("used").cloned() {
        let hit_ids: std::collections::HashSet<i64> = hits
            .iter()
            .filter_map(|h| h.get("id").and_then(|v| v.as_i64()))
            .collect();
        let cited: Vec<i64> = used
            .split(',')
            .filter_map(|s| s.trim().parse::<i64>().ok())
            .filter(|i| hit_ids.contains(i))
            .take(64)
            .collect();
        if !cited.is_empty() {
            let pool4 = st.pool.clone();
            tokio::task::spawn_blocking(move || {
                if let Ok(conn) = pool4.get() {
                    let now = chrono::Utc::now().timestamp();
                    crate::workflow::kcs::record_sir_found(&conn, id, &cited, now);
                }
            })
            .await
            .ok();
        }
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
    principal: crate::handlers::auth::OptPrincipal,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Admin, "", "global")?;
    crate::handlers::breaches::require_dpo_role(&principal, &pool)?;
    let pool_cal = pool.clone();
    let runs: Vec<brain_engine_sdk::scoreboard::RunArtifacts> =
        tokio::task::spawn_blocking(move || -> Result<_, String> {
            let conn = pool.get().map_err(|e| format!("{e}"))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, status, state_json FROM workflow_runs ORDER BY id DESC LIMIT 1000",
                )
                .map_err(|e| format!("{e}"))?;
            let mut rows = Vec::new();
            for row in stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| format!("{e}"))?
            {
                rows.push(row.map_err(|e| format!("{e}"))?);
            }
            let audited = audited_run_ids(&conn, rows.iter().map(|(id, _, _)| *id));
            Ok(derive_artifacts(&rows, &audited))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("{e}")))?
        .map_err(HandlerError::internal)?;
    let sb = brain_engine_sdk::scoreboard::build(&runs);
    // Pair efficiency with correctness, then honor the weekly REPORT cadence:
    // when due, a machine-generated CalibrationRecord lands on the workflow
    // audit chain (best-effort; a missed report is re-due next read).
    let score_units = if runs.is_empty() {
        0
    } else {
        runs.iter()
            .map(|r| brain_engine_sdk::pure::qa_score::score_run(r).total_units)
            .sum::<i32>()
            / runs.len() as i32
    };
    let emitted = tokio::task::spawn_blocking(
        move || -> Result<(bool, crate::workflow::kcs::KcsMeasures), String> {
            let conn = pool_cal.get().map_err(|e| format!("{e}"))?;
            let now = chrono::Utc::now().timestamp();
            // The Evolve loop's measures ride the weekly report: linkage,
            // reuse (SIR), and freshness, computed from the same tables the
            // scoreboard serves — the report and the board measure the same
            // numbers.
            let kcs = crate::workflow::kcs::kcs_measures(&conn, now).map_err(|e| format!("{e}"))?;
            if !crate::workflow::calibration::report_due(&conn, now) {
                return Ok((false, kcs));
            }
            let summary = format!(
                "kcs_linkage_rate:{} reuse_rate:{} freshness_median_age_secs:{}",
                kcs.linkage_rate_units,
                kcs.searched_found_rate_units,
                kcs.article_freshness_median_age_secs
            );
            crate::workflow::calibration::record_report(&conn, score_units, now, &summary)
                .map_err(|e| format!("{e}"))?;
            Ok((true, kcs))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?;
    let (emitted, kcs) = emitted;
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
        "calibration_report_emitted": emitted,
        // Evolve: the KCS performance measures (additive).
        "kcs_linkage_rate_units": kcs.linkage_rate_units,
        "searched_found_rate_units": kcs.searched_found_rate_units,
        "article_freshness_median_age_secs": kcs.article_freshness_median_age_secs,
    })))
}

/// The body of a monthly human-signed calibration. `human_agreement_kappa_units`
/// is the reviewer's recorded scorer-vs-human κ (`-1` sentinel = none yet).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationSignBody {
    pub(crate) reviewer_id: String,
    pub(crate) human_agreement_kappa_units: i32,
}

/// `POST /workflow/calibration/sign` — the monthly human-signed calibration
/// gate (DPO/admin). One signature per calendar month; the record rides the
/// workflow audit chain and re-anchors the baseline delta.
pub async fn post_calibration_sign(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Json(body): Json<CalibrationSignBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    super::authorize(&principal, crate::auth::Action::Admin, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    crate::handlers::breaches::require_dpo_role(&principal, &pool)?;
    let pool_sign = pool.clone();
    let reviewer = body.reviewer_id.trim().to_string();
    if reviewer.is_empty() || reviewer.len() > 128 {
        return Err(HandlerError::bad_request(
            "reviewer_invalid",
            "reviewer_id must be 1..=128 characters",
        ));
    }
    let kappa = body.human_agreement_kappa_units;
    if kappa != -1 && !(0..=brain_engine_sdk::pure::qa_score::SCALE).contains(&kappa) {
        return Err(HandlerError::bad_request(
            "kappa_invalid",
            "human_agreement_kappa_units must be -1 or in 0..=10000",
        ));
    }
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let mut conn = pool_sign
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        // Gate + write in ONE immediate transaction: the monthly gate is
        // check-then-write, so two concurrent signs must serialize here.
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let now = chrono::Utc::now().timestamp();
        if crate::workflow::calibration::signature_blocked(&tx, now) {
            return Err(HandlerError::conflict_with(
                "already_signed_this_month",
                "a human-signed calibration already exists for this month",
                serde_json::json!([]),
            ));
        }
        crate::workflow::calibration::record_signed(
            &tx,
            kappa,
            score_units_now(&tx),
            &reviewer,
            now,
        )
        .map_err(|e| HandlerError::internal(format!("{e}")))?;
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok(serde_json::json!({"signed": true, "month": now}))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map(Json)
}

/// Reconstruct the set of run ids that a workflow audit row references.
///
/// `audit_events` stores only SHA-256 target hashes — there is no plain-text
/// `target` column to cast. Every run-bound substrate write (open, CAS
/// transition, answer, state_read) targets the canonical `run:{id}` string,
/// so a run is audit-linked iff `hash("run:{id}")` appears among the
/// workflow-kind rows. Anything else (outbox/calibration rows) targets other
/// strings and must never light up a run.
fn audited_run_ids(
    conn: &rusqlite::Connection,
    run_ids: impl Iterator<Item = i64>,
) -> std::collections::HashSet<i64> {
    let Ok(mut stmt) =
        conn.prepare("SELECT DISTINCT target_hash FROM audit_events WHERE kind = 'workflow'")
    else {
        return Default::default();
    };
    let targets: std::collections::HashSet<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default();
    run_ids
        .filter(|id| targets.contains(&crate::audit::hash(&format!("run:{id}"))))
        .collect()
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

/// The current mean per-run score, derived exactly as the scoreboard derives
/// it (same queries, same fail-closed audit linkage) — the signed gate and
/// the weekly report must measure the same number.
fn score_units_now(conn: &rusqlite::Connection) -> i32 {
    let mut stmt = match conn
        .prepare("SELECT id, status, state_json FROM workflow_runs ORDER BY id DESC LIMIT 1000")
    {
        Ok(s) => s,
        Err(_) => return 0,
    };
    let rows: Vec<(i64, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default();
    let audited = audited_run_ids(conn, rows.iter().map(|(id, _, _)| *id));
    let runs = derive_artifacts(&rows, &audited);
    if runs.is_empty() {
        0
    } else {
        runs.iter()
            .map(|r| brain_engine_sdk::pure::qa_score::score_run(r).total_units)
            .sum::<i32>()
            / runs.len() as i32
    }
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

    /// Regression pin: `audit_events` has no plain-text `target` column (the
    /// old `CAST(target AS INTEGER)` query 500s). The audited set must
    /// reconstruct via `hash("run:{id}")` membership and stay fail-closed.
    #[test]
    fn audited_run_ids_reconstructs_hashed_targets() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE audit_events(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts TEXT DEFAULT CURRENT_TIMESTAMP,
                kind TEXT NOT NULL,
                actor TEXT,
                target_hash TEXT,
                status TEXT,
                detail_hash TEXT,
                tenant_id TEXT NOT NULL DEFAULT 'global',
                prev_hash TEXT);",
        )
        .unwrap();
        for (target, kind) in [
            ("run:7", "workflow"),     // canonical run-bound row → links run 7
            ("outbox:k1", "workflow"), // substrate row bound to no run id
            ("run:9", "client"),       // wrong kind must never link
        ] {
            conn.execute(
                "INSERT INTO audit_events(kind, actor, target_hash, status)
                 VALUES (?1, 'workflow', ?2, 'ok')",
                rusqlite::params![kind, crate::audit::hash(target)],
            )
            .unwrap();
        }
        let audited = audited_run_ids(&conn, [7i64, 8, 9].into_iter());
        assert!(audited.contains(&7), "hash(run:7) linkage must reconstruct");
        assert!(!audited.contains(&8), "absence never counts green");
        assert!(
            !audited.contains(&9),
            "non-workflow kinds must not satisfy workflow audit linkage"
        );
    }
}

// ── plugin mount evidence (Art. 12 record-keeping) ─────────────────────────

#[derive(Debug, Deserialize)]
pub struct PluginMountRequest {
    /// Plugin identity (`ui-shell`, `ui-chat`, `ui-control-panel`, …).
    pub plugin: String,
    /// `mount` (default) or `unmount`.
    #[serde(default)]
    pub action: Option<String>,
    /// The slot-registry revision the client composed at.
    #[serde(default)]
    pub revision: Option<u64>,
    /// SHA-256 of the bundle the plugin shipped in, when the boot manifest
    /// carries one. Since the Gateweld closure the digest is SERVER-VERIFIED
    /// against the live `boot_manifest()` before any audit row is written —
    /// a mismatch or unknown digest is a `409` (the Art. 12 row can no longer
    /// certify bytes that were never served).
    #[serde(default)]
    pub bundle_sha256: Option<String>,
    /// The manifest bundle path the digest claims to cover (`pkg/app.js`).
    /// Optional; when omitted any manifest bundle with a matching digest
    /// satisfies the check.
    #[serde(default)]
    pub bundle_path: Option<String>,
}

fn valid_plugin_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Pure: does `sha` (with optional pinned `path`) match a bundle in the boot
/// manifest? The mount-evidence gate — a digest for bytes the server never
/// served can never be certified.
pub(crate) fn mount_digest_matches(
    manifest: &serde_json::Value,
    sha: &str,
    path: Option<&str>,
) -> bool {
    manifest["bundles"].as_array().is_some_and(|bundles| {
        bundles
            .iter()
            .any(|b| b["sha256"] == *sha && path.is_none_or(|want| b["path"] == want))
    })
}

/// `POST /workflow/plugins/mount` — record UI-plugin mount/unmount evidence on
/// the audit chain. Mount evidence is Art. 12 record-keeping: WHO ran WHICH
/// plugin composition, WHEN, against which bundle digest. Metadata only; the
/// write is audited in the same tx it lands.
pub async fn post_plugin_mount(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Json(body): Json<PluginMountRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    if !valid_plugin_name(&body.plugin) {
        return Err(HandlerError::bad_request(
            "plugin_invalid",
            "plugin name must be 1..=64 lowercase alnum/hyphen",
        ));
    }
    let action = match body.action.as_deref() {
        None | Some("mount") => "mount",
        Some("unmount") => "unmount",
        Some(_) => {
            return Err(HandlerError::bad_request(
                "action_invalid",
                "action must be mount or unmount",
            ));
        }
    };
    if let Some(sha) = &body.bundle_sha256
        && (sha.len() != 64 || !sha.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return Err(HandlerError::bad_request(
            "sha_invalid",
            "bundle_sha256 must be 64 hex chars",
        ));
    }
    // Server-verify the digest against the LIVE boot manifest BEFORE the audit
    // row — mount evidence is Art. 12 record-keeping, so a digest for bytes
    // that were never served must never reach the chain. Fail before write.
    if let Some(sha) = &body.bundle_sha256 {
        let manifest = super::frontend::boot_manifest(super::frontend::dist_dir());
        if !mount_digest_matches(&manifest, sha, body.bundle_path.as_deref()) {
            return Err(HandlerError::conflict(
                "bundle_unverified: bundle_sha256 does not match the served boot manifest",
            ));
        }
    }
    // Write gate on the shared pool: recording evidence is a write, and an
    // unauthenticated caller has no composition worth evidencing.
    super::authorize(&principal, crate::auth::Action::Write, "", "global")?;
    let actor = super::recall::principal_label(&principal);
    let pool = state.pool.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        // Audit row + evidence commit atomically via the chain's own tx path.
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Workflow,
            actor.trim(),
            &format!("plugin:{}", body.plugin),
            crate::audit::AuditStatus::Ok,
            &match (&body.revision, &body.bundle_sha256) {
                (Some(r), Some(s)) => format!("{action} revision:{r} bundle:{s}"),
                (Some(r), None) => format!("{action} revision:{r}"),
                (None, Some(s)) => format!("{action} bundle:{s}"),
                (None, None) => action.to_string(),
            },
        );
        Ok(())
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?;
    Ok(Json(serde_json::json!({"ok": true})))
}

// ── engine-facing substrate projections ───────────────────────────────────

/// State bodies are bounded to match the SDK harness `MAX_BODY_BYTES`
/// envelope — a run state is a working set, not a document store.
pub(crate) const MAX_STATE_BYTES: usize = 256 * 1024;
/// The AskHuman answer bound (same as steering).
pub(crate) const MAX_ANSWER_BYTES: usize = 4000;

fn valid_domain_label(d: &str) -> bool {
    !d.is_empty()
        && d.len() <= 63
        && d.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_kind(k: &str) -> bool {
    !k.is_empty() && k.len() <= 64 && k.bytes().all(|b| b.is_ascii_graphic() && b != b'<')
}

fn valid_state_body(s: &str) -> Result<serde_json::Value, HandlerError> {
    if s.len() > MAX_STATE_BYTES {
        return Err(HandlerError::bad_request(
            "state_too_large",
            format!("state_json exceeds {} bytes", MAX_STATE_BYTES),
        ));
    }
    serde_json::from_str::<serde_json::Value>(s)
        .map_err(|_| HandlerError::bad_request("state_invalid", "state_json must be valid JSON"))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenRunRequest {
    pub domain: String,
    pub kind: String,
    pub state_json: String,
}

/// `POST /workflow/runs` — open a governed run. Substrate projection: an
/// INSERT under [`crate::workflow::tx::WorkflowTx`] with the audit row in the
/// SAME transaction. Gated Write + the `workflow` role.
pub async fn post_run(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Json(body): Json<OpenRunRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    if !valid_domain_label(&body.domain) {
        return Err(HandlerError::bad_request(
            "domain_invalid",
            "domain must be 1..=63 lowercase alnum/hyphen",
        ));
    }
    if !valid_kind(&body.kind) {
        return Err(HandlerError::bad_request(
            "kind_invalid",
            "kind must be 1..=64 printable characters",
        ));
    }
    let parsed = valid_state_body(&body.state_json)?;
    if parsed.is_null() {
        return Err(HandlerError::bad_request(
            "state_invalid",
            "state_json must not be null",
        ));
    }
    let _ = parsed;
    super::authorize(&principal, crate::auth::Action::Write, "", &body.domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    crate::handlers::authorize_role(&principal, &pool, "workflow")?;
    let (domain, kind, state_json) = (body.domain, body.kind, body.state_json);
    let (run_id, _): (i64, i64) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let mut conn = pool.get().map_err(|e| format!("{e}"))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn).map_err(|e| format!("{e}"))?;
        let now = chrono::Utc::now().timestamp();
        tx.tx()
            .execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 0, 'active', ?4, ?4)",
                rusqlite::params![domain, kind, state_json, now],
            )
            .map_err(|e| format!("{e}"))?;
        let run_id = tx.tx().last_insert_rowid();
        crate::workflow::audit_write(
            tx.tx(),
            run_id,
            &format!("run:{run_id}"),
            crate::audit::AuditStatus::Ok,
            "open",
        );
        tx.commit().map_err(|e| format!("{e}"))?;
        Ok((run_id, 0i64))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?;
    Ok(Json(serde_json::json!({"run_id": run_id, "revision": 0})))
}

/// Resolve a run's domain or 404 (probe-blind on missing runs).
pub(crate) async fn run_domain(state: &Arc<AppState>, id: i64) -> Result<String, HandlerError> {
    let pool = state.pool.clone();
    tokio::task::spawn_blocking(move || -> Result<Option<String>, String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        conn.query_row(
            "SELECT domain FROM workflow_runs WHERE id=?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| format!("{e}"))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?
    .ok_or_else(|| HandlerError::not_found("workflow run not found"))
}

/// `GET /workflow/runs/{id}/state` — the ENGINE-exact view
/// `{state_json, revision}`. Machine round-trip: deliberately NOT routed
/// through `sanitize_read` (the human `GET /workflow/runs/{id}` is); engines
/// CAS against the exact stored bytes, and a redacted echo would poison every
/// subsequent write. Documented ceiling: this surface requires the same Read
/// grant as the human one plus the `workflow` engine role.
pub async fn get_run_state(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    crate::handlers::authorize_role(&principal, &pool, "workflow")?;
    let row: Option<(String, i64)> = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        let row = conn
            .query_row(
                "SELECT state_json, state_revision FROM workflow_runs WHERE id=?1",
                rusqlite::params![id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|e| format!("{e}"))?;
        if row.is_some() {
            // Audited read: an engine pulling run state is evidence too.
            crate::workflow::audit_write(
                &conn,
                id,
                &format!("run:{id}"),
                crate::audit::AuditStatus::Ok,
                "state_read",
            );
        }
        Ok(row)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?;
    let (state_json, revision) =
        row.ok_or_else(|| HandlerError::not_found("workflow run not found"))?;
    Ok(Json(
        serde_json::json!({"state_json": state_json, "revision": revision}),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PutStateRequest {
    pub expected_rev: i64,
    pub state_json: String,
    #[serde(default)]
    pub status: Option<String>,
}

/// `PUT /workflow/runs/{id}/state` — CAS state advance
/// (`200 {revision}` | `409 {actual_revision}`). The engine's persist step.
pub async fn put_run_state(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<PutStateRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    crate::handlers::authorize_role(&principal, &pool, "workflow")?;
    let status = body.status.clone().unwrap_or_else(|| "active".to_string());
    if !status.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') || status.len() > 24 {
        return Err(HandlerError::bad_request(
            "status_invalid",
            "status must be lowercase, ≤24 chars",
        ));
    }
    valid_state_body(&body.state_json)?;
    let new_state = body.state_json;
    let expected_rev = body.expected_rev;
    let completed_status = status.clone();
    let flag_state = new_state.clone();
    let outcome: Result<i64, crate::workflow::state::CasError> = tokio::task::spawn_blocking(
        move || -> Result<Result<i64, crate::workflow::state::CasError>, String> {
            let conn = pool.get().map_err(|e| format!("{e}"))?;
            let now = chrono::Utc::now().timestamp();
            Ok(crate::workflow::state::cas_update(
                &conn,
                id,
                expected_rev,
                &new_state,
                &status,
                now,
            ))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?;
    match outcome {
        Ok(revision) => {
            // The Improve practice hook: a run completing with contradictions
            // or skipped verification flags its cited articles (content-health
            // input; never an edit). Best-effort, announced on failure.
            if completed_status == "completed" {
                let pool_flag = super::resolve_domain_pool(&state.registry, None)?;
                let state_json = flag_state;
                tokio::task::spawn_blocking(move || {
                    if let Ok(mut conn) = pool_flag.get() {
                        let now = chrono::Utc::now().timestamp();
                        let parsed: serde_json::Value =
                            serde_json::from_str(&state_json).unwrap_or(serde_json::Value::Null);
                        let flagged = crate::workflow::kcs::flag_contradicted_articles(
                            &mut conn, id, &parsed, now,
                        );
                        if !flagged.is_empty() {
                            tracing::info!(run = id, ?flagged, "kcs improve flags emitted");
                        }
                    }
                })
                .await
                .ok();
            }
            Ok(Json(serde_json::json!({"revision": revision})))
        }
        Err(crate::workflow::state::CasError::Stale { actual_revision }) => {
            Err(HandlerError::conflict_with(
                "cas_stale",
                "state was advanced concurrently",
                serde_json::json!({ "actual_revision": actual_revision }),
            ))
        }
        Err(crate::workflow::state::CasError::Gone) => {
            Err(HandlerError::not_found("workflow run not found"))
        }
        Err(crate::workflow::state::CasError::Database(m)) => Err(HandlerError::internal(m)),
        Err(other) => Err(HandlerError::internal(other.to_string())),
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostEventRequest {
    pub topic: String,
    pub payload_json: String,
    pub idempotency_key: String,
    /// Optional ancestry link: the outbox id this event follows.
    #[serde(default)]
    pub parent_event_id: Option<i64>,
}

/// `POST /workflow/runs/{id}/events` — outbox enqueue, idempotent by UNIQUE
/// key → `{first, event_id}`. The exactly-once receipt engines replay
/// against; `event_id` resolves even on replay (the surviving row's id).
pub async fn post_event(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<PostEventRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    crate::handlers::authorize_role(&principal, &pool, "workflow")?;
    let topic_ok = !body.topic.is_empty()
        && body.topic.len() <= 64
        && body
            .topic
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'/' || b == b'-');
    if !topic_ok {
        return Err(HandlerError::bad_request(
            "topic_invalid",
            "topic must be 1..=64 lowercase alnum///-",
        ));
    }
    if body.payload_json.len() > MAX_STATE_BYTES {
        return Err(HandlerError::bad_request(
            "payload_too_large",
            "payload_json exceeds the state bound",
        ));
    }
    serde_json::from_str::<serde_json::Value>(&body.payload_json).map_err(|_| {
        HandlerError::bad_request("payload_invalid", "payload_json must be valid JSON")
    })?;
    if body.idempotency_key.is_empty() || body.idempotency_key.len() > 128 {
        return Err(HandlerError::bad_request(
            "key_invalid",
            "idempotency_key must be 1..=128 chars",
        ));
    }
    if body.parent_event_id.is_some_and(|p| p <= 0) {
        return Err(HandlerError::bad_request(
            "parent_invalid",
            "parent_event_id must be a positive outbox id",
        ));
    }
    let topic = body.topic.clone();
    let topic_check = topic.clone();
    let payload_json = body.payload_json.clone();
    let idempotency_key = body.idempotency_key.clone();
    let parent_event_id = body.parent_event_id;
    let outcome: (bool, i64) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let mut conn = pool.get().map_err(|e| format!("{e}"))?;
        let mut tx =
            crate::workflow::tx::WorkflowTx::begin(&mut conn).map_err(|e| format!("{e}"))?;
        let now = chrono::Utc::now().timestamp();
        let outcome = crate::workflow::outbox::enqueue_child(
            tx.tx(),
            id,
            parent_event_id,
            &topic,
            &payload_json,
            &idempotency_key,
            now,
        )
        .map_err(|e| format!("{e}"))?;
        tx.commit().map_err(|e| format!("{e}"))?;
        Ok(outcome)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?;
    // Evolve trigger: a FIRST `crm/case/closed` event fires the deterministic
    // KCS capture generator (exactly-once via the outbox marker). Best-effort:
    // a failed capture is announced; the case-close itself already committed.
    if topic_check == crate::connector::crm::TOPIC_CASE_CLOSED && outcome.0 {
        let pool_cap = super::resolve_domain_pool(&state.registry, None)?;
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let mut conn = pool_cap.get().map_err(|e| format!("{e}"))?;
            let now = chrono::Utc::now().timestamp();
            match crate::workflow::kcs::capture_on_case_close(&mut conn, id, now) {
                Ok(outcome) => {
                    tracing::info!(?outcome, run = id, "kcs capture pass");
                    Ok(())
                }
                Err(e) => {
                    tracing::warn!("kcs capture failed for run {id}: {e}");
                    Err(e.to_string())
                }
            }
        })
        .await
        .ok();
    }
    Ok(Json(
        serde_json::json!({"first": outcome.0, "event_id": outcome.1}),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerRequest {
    pub answer: String,
    /// SHA-256 hex of the exact `pending_question` bytes the answerer saw —
    /// the approval binds to the question it answered (the ReviewArmour
    /// digest-binding posture, at question grain).
    pub question_digest: String,
}

/// `POST /workflow/runs/{id}/answer` — THE AskHuman closer. In ONE
/// `WorkflowTx`: re-read state, verify the digest binds to the live
/// `pending_question`, append `answers[]`, clear `pending_question`, CAS.
/// Role gate: `approve` (the steering gate — answering shapes decisions).
pub async fn post_answer(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<AnswerRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    if body.answer.len() > MAX_ANSWER_BYTES {
        return Err(HandlerError::bad_request(
            "answer_too_long",
            "answer exceeds 4000 chars",
        ));
    }
    if body.answer.trim().is_empty() {
        return Err(HandlerError::bad_request(
            "answer_empty",
            "answer must not be empty",
        ));
    }
    if crate::contains_suspicious_pattern(&body.answer) {
        return Err(HandlerError::bad_request(
            "answer_rejected",
            "answer matches a blocked prompt-injection pattern",
        ));
    }
    let want = body.question_digest.trim().to_ascii_lowercase();
    if want.len() != 64 || !want.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(HandlerError::bad_request(
            "digest_invalid",
            "question_digest must be 64 hex chars",
        ));
    }
    let domain = run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    crate::handlers::authorize_role(&principal, &pool, "approve")?;
    let answer = body.answer.clone();
    let result: Result<i64, String> = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let mut conn = pool.get().map_err(|e| format!("{e}"))?;
        let mut tx =
            crate::workflow::tx::WorkflowTx::begin(&mut conn).map_err(|e| format!("{e}"))?;
        let (js, rev): (String, i64) = tx
            .tx()
            .query_row(
                "SELECT state_json, state_revision FROM workflow_runs WHERE id=?1",
                rusqlite::params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| format!("{e}"))?;
        let mut st: serde_json::Value =
            serde_json::from_str(&js).map_err(|_| "corrupt state_json".to_string())?;
        let Some(question) = st
            .get("pending_question")
            .and_then(|q| q.as_str())
            .map(str::to_string)
        else {
            return Err("__no_pending__".to_string());
        };
        let got = crate::audit::hash(&question);
        if got != want {
            return Err(format!("__digest__:{got}"));
        }
        let answers = st
            .as_object_mut()
            .ok_or("corrupt state_json".to_string())?
            .entry("answers".to_string())
            .or_insert_with(|| serde_json::Value::Array(vec![]));
        if let Some(arr) = answers.as_array_mut() {
            arr.push(serde_json::json!({
                "answer": answer,
                "question_digest": want,
                "ts": chrono::Utc::now().timestamp(),
            }));
        }
        st.as_object_mut()
            .ok_or("corrupt state_json".to_string())?
            .remove("pending_question");
        let now = chrono::Utc::now().timestamp();
        let new_json = serde_json::to_string(&st).map_err(|e| format!("{e}"))?;
        crate::workflow::state::cas_update(tx.tx(), id, rev, &new_json, "active", now)
            .map_err(|e| format!("{e:?}"))?;
        crate::workflow::audit_write(
            tx.tx(),
            id,
            &format!("run:{id}"),
            crate::audit::AuditStatus::Ok,
            "answer",
        );
        tx.commit().map_err(|e| format!("{e}"))?;
        Ok(rev + 1)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    match result {
        Ok(revision) => Ok(Json(serde_json::json!({"ok": true, "revision": revision}))),
        Err(e) if e == "__no_pending__" => Err(HandlerError::conflict(
            "no_pending_question: the run has no pending_question to answer",
        )),
        Err(e) if e.starts_with("__digest__:") => Err(HandlerError::conflict_with(
            "question_digest_mismatch",
            "question_digest does not bind to the live pending_question",
            serde_json::json!({}),
        )),
        Err(e) => Err(HandlerError::internal(e)),
    }
}

/// `GET /workflow/runs/{id}/steering?since=` — drain the steering outbox.
/// The read half of the inbox: post_steering enqueues, the engine drains at
/// the step boundary via this surface. Advisory only — never autonomous.
pub async fn get_steering(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Path(id): Path<i64>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let since: i64 = q.get("since").and_then(|s| s.parse().ok()).unwrap_or(0);
    let pool = state.pool.clone();
    let rows: Vec<(i64, String)> = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, payload_json FROM outbox
                 WHERE run_id=?1 AND topic='steering' AND id > ?2 ORDER BY id ASC LIMIT 100",
            )
            .map_err(|e| format!("{e}"))?;
        let it = stmt
            .query_map(rusqlite::params![id, since], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| format!("{e}"))?;
        Ok(it.filter_map(Result::ok).collect())
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?;
    let messages: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(oid, payload)| {
            serde_json::json!({"outbox_id": oid, "message": sanitize_steering_payload(&payload)})
        })
        .collect();
    Ok(Json(serde_json::json!({"messages": messages})))
}

/// The outbox stores the sanitized message verbatim (post_steering screens +
/// sanitizes before enqueue), so the read echoes the payload's `message`.
fn sanitize_steering_payload(payload: &str) -> String {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| {
            v.get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default()
}
