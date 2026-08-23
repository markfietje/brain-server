//! Workflow lineage surfaces ("Lineage"): the events read, the
//! rewind-as-branch write, and the I-PASS handoff packet. Storage projections
//! over [`crate::workflow::outbox`] primitives — no engine logic here; the
//! state stays engine-opaque and is restored verbatim from checkpoint rows.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use std::{collections::HashMap, sync::Arc};

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::workflow::run_domain;
use rusqlite::OptionalExtension;

/// `GET /workflow/runs/{id}/events?branch=` — the lineage read: ordered
/// events with parent links (the UI timeline input). `branch=<event_id>`
/// narrows to that event's ancestor chain, root-first.
pub async fn get_run_events(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Path(id): Path<i64>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = run_domain(&state, id).await?;
    crate::handlers::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let branch: Option<i64> = match q.get("branch") {
        None => None,
        Some(s) => s.parse().map(Some).map_err(|_| {
            HandlerError::bad_request("branch_invalid", "branch must be an event id")
        })?,
    };
    let pool = state.pool.clone();
    let rows: Vec<(i64, Option<i64>, String, String, String)> =
        tokio::task::spawn_blocking(move || -> Result<_, String> {
            let conn = pool.get().map_err(|e| format!("{e}"))?;
            let chain: Option<Vec<i64>> = match branch {
                Some(b) => Some(
                    crate::workflow::outbox::branch_chain(&conn, id, b)
                        .map_err(|e| format!("{e}"))?,
                ),
                None => None,
            };
            let mut stmt = conn
                .prepare(
                    "SELECT id, parent_id, topic, payload_json, status FROM outbox
                      WHERE run_id = ?1 ORDER BY id ASC LIMIT 1000",
                )
                .map_err(|e| format!("{e}"))?;
            let it = stmt
                .query_map(rusqlite::params![id], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })
                .map_err(|e| format!("{e}"))?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.map_err(|e| format!("{e}"))?);
            }
            if let Some(chain) = chain {
                out.retain(|(eid, ..)| chain.contains(eid));
            }
            Ok(out)
        })
        .await
        .map_err(|e| HandlerError::internal(format!("{e}")))?
        .map_err(HandlerError::internal)?;
    let events: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(eid, parent, topic, payload, status)| {
            serde_json::json!({
                "event_id": eid,
                "parent_id": parent,
                "topic": topic,
                // The read seam covers EVERY emitted stored-text field.
                "payload_json": crate::gate::sanitize_read(&payload, false, &principal),
                "status": status,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "events": events })))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewindRequest {
    pub to_event_id: i64,
    /// Why the branch forks (screened like steering; ≤4000 chars). Recorded in
    /// the run state's `branches[]` marker.
    pub reason: String,
}

enum RewindErr {
    Target(String),
    Corrupt(String),
    Gone,
}

/// `POST /workflow/runs/{id}/rewind` — REWIND = BRANCH, NEVER DELETE. In one
/// `WorkflowTx`: verify the target is a `workflow/checkpoint` event (or the
/// run root) of THIS run, CAS-write the checkpoint's state snapshot back over
/// the live state appending a `branches[]` marker, audit `rewind`. Nothing is
/// deleted; the abandoned branch stays fully queryable via `/events`.
/// Role gate: `approve` (the steering/answer gate — rewinding shapes
/// decisions).
pub async fn post_rewind(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<RewindRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    if body.reason.len() > 4000 || body.reason.trim().is_empty() {
        return Err(HandlerError::bad_request(
            "reason_invalid",
            "reason must be 1..=4000 characters",
        ));
    }
    if crate::contains_suspicious_pattern(&body.reason) {
        return Err(HandlerError::bad_request(
            "reason_rejected",
            "reason matches a blocked prompt-injection pattern",
        ));
    }
    let reason = body.reason.clone();
    let to_event = body.to_event_id;
    let domain = run_domain(&state, id).await?;
    crate::handlers::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = crate::handlers::resolve_domain_pool(&state.registry, None)?;
    crate::handlers::authorize_role(&principal, &pool, "approve")?;

    let result: Result<i64, RewindErr> =
        tokio::task::spawn_blocking(move || -> Result<_, RewindErr> {
            let mut conn = pool.get().map_err(|e| RewindErr::Corrupt(format!("{e}")))?;
            let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
                .map_err(|e| RewindErr::Corrupt(format!("{e}")))?;
            let (js, rev): (String, i64) = tx
                .tx()
                .query_row(
                    "SELECT state_json, state_revision FROM workflow_runs WHERE id=?1",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|_| RewindErr::Gone)?;
            let _ = js;
            let (topic, payload): (String, String) = tx
                .tx()
                .query_row(
                    "SELECT topic, payload_json FROM outbox WHERE id=?1 AND run_id=?2",
                    rusqlite::params![to_event, id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|_| RewindErr::Target("event not found on this run".to_string()))?;
            let root: i64 = tx
                .tx()
                .query_row(
                    "SELECT COALESCE(MIN(id), 0) FROM outbox WHERE run_id=?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
                .map_err(|e| RewindErr::Corrupt(format!("{e}")))?;
            // A rewind target is always a checkpoint event — or the run root
            // itself, whose snapshot is the fresh-run state ({}).
            let snapshot: serde_json::Value = if topic == "workflow/checkpoint" {
                serde_json::from_str(&payload)
                    .map_err(|e| RewindErr::Target(format!("corrupt checkpoint payload: {e}")))?
            } else if to_event == root && root != 0 {
                serde_json::json!({})
            } else {
                return Err(RewindErr::Target(
                    "target must be a workflow/checkpoint event or the run root".to_string(),
                ));
            };
            // The state is opaque to the server; the branch MARKER is appended
            // verbatim so the engine knows where its next emission parents.
            let mut st = snapshot;
            if let Some(obj) = st.as_object_mut() {
                let branches = obj
                    .entry("branches".to_string())
                    .or_insert_with(|| serde_json::Value::Array(vec![]));
                if let Some(arr) = branches.as_array_mut() {
                    arr.push(serde_json::json!({
                        "from_event": to_event,
                        "reason": reason,
                        "at": chrono::Utc::now().timestamp(),
                    }));
                }
            }
            let now = chrono::Utc::now().timestamp();
            let new_json = serde_json::to_string(&st)
                .map_err(|e| RewindErr::Corrupt(format!("serialize: {e}")))?;
            crate::workflow::state::cas_update(tx.tx(), id, rev, &new_json, "active", now)
                .map_err(|_| RewindErr::Corrupt("cas conflict during rewind".to_string()))?;
            crate::workflow::audit_write(
                tx.tx(),
                id,
                &format!("run:{id}"),
                crate::audit::AuditStatus::Ok,
                "rewind",
            );
            tx.commit()
                .map_err(|e| RewindErr::Corrupt(format!("{e}")))?;
            Ok(rev + 1)
        })
        .await
        .map_err(|e| HandlerError::internal(format!("{e}")))?;
    match result {
        Ok(revision) => Ok(Json(serde_json::json!({
            "ok": true,
            "revision": revision,
            "branched_from": to_event,
        }))),
        Err(RewindErr::Target(m)) => Err(HandlerError::bad_request("rewind_target_invalid", m)),
        Err(RewindErr::Gone) => Err(HandlerError::not_found("workflow run not found")),
        Err(RewindErr::Corrupt(m)) => Err(HandlerError::internal(m)),
    }
}

/// Assemble the wire packet from gathered facts + the SDK pure builder.
fn build_handoff_packet(
    id: i64,
    domain: &str,
    facts: &brain_engine_sdk::pure::handoff::HandoffFacts,
) -> serde_json::Value {
    use brain_engine_sdk::pure::handoff::{IpassSection, assemble};
    let packet = assemble(facts);
    let section = |s: &IpassSection| serde_json::json!({ "title": s.title(), "lines": s.lines() });
    serde_json::json!({
        "run_id": id,
        "domain": domain,
        "illness": section(&packet.illness),
        "patient": section(&packet.patient),
        "action": section(&packet.action),
        "situation": section(&packet.situation),
        "safety": section(&packet.safety),
        "handoff_complete": packet.complete,
    })
}

/// `GET /workflow/runs/{id}/handoff` — the I-PASS handoff packet assembled
/// from what exists: the run's opening event + steps + step events + latest
/// checkpoint digest + SLA/legal-hold safety envelope. Read-gated on the
/// run's domain; every emitted text rides the read seam.
pub async fn get_handoff(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = run_domain(&state, id).await?;
    crate::handlers::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let reader = principal.clone();
    let pool = state.pool.clone();
    let packet: serde_json::Value = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        let (kind, status, created_at, state_json): (String, String, i64, String) = conn
            .query_row(
                "SELECT kind, status, created_at, state_json FROM workflow_runs WHERE id=?1",
                rusqlite::params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(|e| format!("{e}"))?
            .ok_or("workflow run not found".to_string())?;
        let opening: Option<String> = conn
            .query_row(
                "SELECT payload_json FROM outbox WHERE run_id=?1 ORDER BY id ASC LIMIT 1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("{e}"))?;
        let steps: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT step_key || ':' || phase FROM workflow_steps
                      WHERE run_id=?1 ORDER BY id LIMIT 200",
                )
                .map_err(|e| format!("{e}"))?;
            let it = stmt
                .query_map(rusqlite::params![id], |r| r.get::<_, String>(0))
                .map_err(|e| format!("{e}"))?;
            it.filter_map(Result::ok).collect()
        };
        let step_events: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT topic FROM outbox
                      WHERE run_id=?1 AND topic != 'workflow/checkpoint' ORDER BY id LIMIT 200",
                )
                .map_err(|e| format!("{e}"))?;
            let it = stmt
                .query_map(rusqlite::params![id], |r| r.get::<_, String>(0))
                .map_err(|e| format!("{e}"))?;
            it.filter_map(Result::ok).collect()
        };
        let latest_checkpoint: Option<String> = conn
            .query_row(
                "SELECT payload_json FROM outbox
                  WHERE run_id=?1 AND topic='workflow/checkpoint' ORDER BY id DESC LIMIT 1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("{e}"))?;
        let held: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM legal_holds WHERE released_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let st: serde_json::Value =
            serde_json::from_str(&state_json).unwrap_or(serde_json::Value::Null);
        let now = chrono::Utc::now().timestamp();
        // SLA envelope: the recorded deadline when state carries one, else the
        // policy stamp over P3 at run-open time (documented default posture).
        let sla_deadline = st
            .get("sla_deadline")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| created_at + brain_engine_sdk::policy::Priority::P3.ttl_secs());
        let facts = brain_engine_sdk::pure::handoff::HandoffFacts {
            intent: st
                .get("intent")
                .and_then(|v| v.as_str())
                .unwrap_or(kind.as_str())
                .to_string(),
            is_seed: st
                .get("is_seed")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            is_not_seed: st
                .get("is_not_seed")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            opening_event: opening.map(|o| crate::gate::sanitize_read(&o, false, &reader)),
            domain: domain.clone(),
            // The Patient section already carries the domain header; rows add
            // subject-level references only.
            patient_rows: vec![format!("run:{id}")],
            action_steps: steps,
            action_events: step_events,
            checkpoint_digest: latest_checkpoint.map(|c| crate::audit::hash(&c)),
            pending_question: st
                .get("pending_question")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            sla_deadline: Some(sla_deadline),
            now,
            legal_hold_active: held > 0,
            escalation_honored: st
                .get("escalation_honored")
                .and_then(|b| b.as_bool())
                .unwrap_or(true),
            run_status: status,
        };
        Ok(build_handoff_packet(id, &domain, &facts))
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
    Ok(Json(packet))
}
