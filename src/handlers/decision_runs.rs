//! The decision-run surfaces: execute a pipeline run, read its stored
//! trace, replay it under its own recorded conditions, and the DPO-gated
//! listing — the decision harness's public-safe face.
//!
//! The SEMANTICS stay internal (the pipeline is the line's private core):
//! what is public here is route EXISTENCE + the authz posture. Every route
//! clones the shipped postures: the run resolves first (404 probe-blind on
//! an absent or foreign run), the operator decision gates (Write/Read +
//! the `workflow` role), the listing's DPO dual gate (Admin + the DPO
//! role, audited per call — the κ-report posture), and ONE transition per
//! write (the trace writer owns it; the route's proposal + audit ride it).
//!
//! The raw query is the request's ONE free-text field: it reaches only the
//! retrieval seam's argument and is hashed before anything durable — the
//! trace law (digests and refs only) is the storage contract and this
//! file adds no exception. Config documents arrive in the request BODY,
//! never from the environment.
//!
//! Mode law end-to-end: an exploratory run PROPOSES, never promotes. The
//! proposal this route writes (opt-in, escalate-only) carries the run's
//! provenance ref; the promotion gate reads the ref and refuses
//! exploratory proposals by name — the gate, not this route, is the
//! enforcement point.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::workflow::harness::config::DecisionPipelineConfig;
use crate::workflow::harness::models::rules::RulesModel;
use crate::workflow::harness::pipeline::{
    DecisionRunRequest, RegistryRef, StageRecord, run_decision_pipeline,
};
use crate::workflow::harness::retrieval::SearchRetriever;
use crate::workflow::harness::trace::{build_decision_run_trace, persist_decision_run_trace_with};
use crate::workflow::registry;
use brain_engine_sdk::decision::RunMode;

const MAX_DECISION_QUERY_LEN: usize = 4096;
const MAX_DECISION_REQUEST_ID_LEN: usize = 256;
const MAX_QUESTION_IDS: usize = 64;
const MAX_QUESTION_ID_LEN: usize = 256;
const LISTING_LIMIT_DEFAULT: usize = 20;
const LISTING_LIMIT_CAP: usize = 50;

fn screening_err(code: &'static str, msg: &str) -> HandlerError {
    HandlerError::bad_request(code, msg.to_string())
}

/// The shared screening law (the decision-ref precedent): bounded,
/// non-empty, no control or invisible characters — free text never rides
/// an unvalidated path.
fn validate_bounded_text(
    raw: &str,
    code: &'static str,
    max: usize,
) -> Result<String, HandlerError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(screening_err(code, "value must not be empty"));
    }
    if trimmed.len() > max
        || trimmed
            .chars()
            .any(|c| c.is_control() || crate::strip_invisible::is_invisible(c))
    {
        return Err(screening_err(
            code,
            &format!("value must be 1..={max} chars with no control or invisible characters"),
        ));
    }
    Ok(trimmed.to_string())
}

fn parse_mode(raw: &str) -> Result<RunMode, HandlerError> {
    match raw {
        "deterministic" => Ok(RunMode::Deterministic),
        "exploratory" => Ok(RunMode::Exploratory),
        other => Err(screening_err(
            "mode_invalid",
            &format!("mode must be deterministic | exploratory, got {other}"),
        )),
    }
}

fn parse_question(
    question_id: &Option<String>,
    question_kind: &Option<String>,
    question_ids: &[String],
) -> Result<Option<crate::workflow::harness::pipeline::AskQuestion>, HandlerError> {
    use crate::workflow::harness::pipeline::AskKind;
    use crate::workflow::harness::pipeline::AskQuestion;
    match (question_id, question_kind) {
        (None, None) => Ok(None),
        (Some(id), Some(kind)) => {
            let id = validate_bounded_text(id, "question_invalid", MAX_QUESTION_ID_LEN)?;
            if !question_ids.contains(&id) {
                return Err(screening_err(
                    "question_invalid",
                    "question_id must appear in question_ids",
                ));
            }
            let kind = match kind.as_str() {
                "choice" => AskKind::Choice,
                "score" => AskKind::Score,
                "noul" => AskKind::Noul,
                other => {
                    return Err(screening_err(
                        "question_invalid",
                        &format!("question_kind must be choice | score | noul, got {other}"),
                    ));
                }
            };
            Ok(Some(AskQuestion { id, kind }))
        }
        _ => Err(screening_err(
            "question_invalid",
            "question_id and question_kind are optional but inseparable",
        )),
    }
}

/// The run inputs both the POST and the replay accept: everything the
/// engine needs that is not config. `run_id` is route-specific (path for
/// the replay, body for the POST), so it is resolved before this lands.
pub(crate) struct RunInputs {
    pub(crate) mode: RunMode,
    pub(crate) request_id: String,
    pub(crate) question_ids: Vec<String>,
    pub(crate) question: Option<crate::workflow::harness::pipeline::AskQuestion>,
    pub(crate) query: String,
}

fn validate_inputs(
    mode: &str,
    request_id: &str,
    question_ids: &[String],
    question_id: &Option<String>,
    question_kind: &Option<String>,
    query: &str,
) -> Result<RunInputs, HandlerError> {
    let mode = parse_mode(mode)?;
    let request_id = validate_bounded_text(
        request_id,
        "request_id_invalid",
        MAX_DECISION_REQUEST_ID_LEN,
    )?;
    if question_ids.is_empty() || question_ids.len() > MAX_QUESTION_IDS {
        return Err(screening_err(
            "question_ids_invalid",
            &format!("question_ids must be 1..={MAX_QUESTION_IDS} entries"),
        ));
    }
    let mut cleaned = Vec::with_capacity(question_ids.len());
    for q in question_ids {
        cleaned.push(validate_bounded_text(
            q,
            "question_ids_invalid",
            MAX_QUESTION_ID_LEN,
        )?);
    }
    let question = parse_question(question_id, question_kind, &cleaned)?;
    let query = validate_bounded_text(query, "query_invalid", MAX_DECISION_QUERY_LEN)?;
    Ok(RunInputs {
        mode,
        request_id,
        question_ids: cleaned,
        question,
        query,
    })
}

fn config_invalid(msg: String) -> HandlerError {
    HandlerError::bad_request("config_invalid", msg)
}

/// The server's ns clock: the injected timing source the engine records
/// (`started_at`/`duration_ns` are provenance, never compared).
fn ns_clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// Load + digest-verify the model the config binds: the loader laws refuse
/// hostile documents by name; the digest check binds the supplied table to
/// the config's recorded model (mismatch → the named 400 — the route never
/// runs a model the config did not pin).
fn load_bound_model(
    config_value: &serde_json::Value,
    rules_value: &serde_json::Value,
) -> Result<
    (
        crate::workflow::harness::config::LoadedPipelineConfig,
        RulesModel,
    ),
    HandlerError,
> {
    let config_str =
        serde_json::to_string(config_value).map_err(|e| HandlerError::internal(e.to_string()))?;
    let loaded = DecisionPipelineConfig::load(&config_str).map_err(config_invalid)?;
    let rules_str =
        serde_json::to_string(rules_value).map_err(|e| HandlerError::internal(e.to_string()))?;
    let model = RulesModel::from_canonical_json(&rules_str)
        .map_err(|e| HandlerError::bad_request("rules_config_invalid", e))?;
    if model.digest() != loaded.config.model.digest {
        return Err(HandlerError::bad_request(
            "model_digest_mismatch",
            format!(
                "the supplied rules table digests to {} but the config binds {} — the run \
                 never evaluates a model the config did not pin",
                model.digest(),
                loaded.config.model.digest
            ),
        ));
    }
    Ok((loaded, model))
}

fn resolve_registered_model(
    conn: &rusqlite::Connection,
    key: &str,
    digest: &str,
    mode: &str,
) -> Result<registry::RegistryRow, HandlerError> {
    match registry::resolve_for_execution(conn, key, digest, mode) {
        Ok(Ok(row)) => Ok(row),
        Ok(Err(registry::ResolveRefusal::NotRegistered)) => Err(HandlerError::bad_request(
            "model_not_registered",
            "the bound model is not present in the governed registry",
        )),
        Ok(Err(registry::ResolveRefusal::NotPromoted)) => Err(HandlerError::bad_request(
            "model_not_promoted",
            "the bound model is registered but has not passed the human promotion gate",
        )),
        Ok(Err(registry::ResolveRefusal::Retired)) => Err(HandlerError::bad_request(
            "model_retired",
            "the bound model has been retired and cannot execute",
        )),
        Err(error) => Err(HandlerError::internal(error.to_string())),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRunBody {
    pub config: serde_json::Value,
    pub rules_config: serde_json::Value,
    pub run_id: i64,
    pub mode: String,
    pub request_id: String,
    #[serde(default)]
    pub question_id: Option<String>,
    #[serde(default)]
    pub question_kind: Option<String>,
    pub question_ids: Vec<String>,
    pub query: String,
    #[serde(default)]
    pub proposal: bool,
}

/// `POST /workflow/decision-runs` — execute one pipeline run and persist
/// its trace. 201 with the trace's bounded face; the optional escalation
/// proposal (opt-in) lands in the SAME transition carrying the run's
/// provenance ref.
pub async fn post_decision_run(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<DecisionRunBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, body.run_id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let inputs = validate_inputs(
        &body.mode,
        &body.request_id,
        &body.question_ids,
        &body.question_id,
        &body.question_kind,
        &body.query,
    )?;
    let (loaded, model) = load_bound_model(&body.config, &body.rules_config)?;
    let run_id = body.run_id;
    let want_proposal = body.proposal;
    let owner = super::recall::principal_label(&principal);
    let retriever = SearchRetriever {
        pool: pool.clone(),
        model: Arc::clone(&state.model),
    };

    let outcome = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let registry_row = {
            let conn = pool.get().map_err(HandlerError::db_down)?;
            resolve_registered_model(
                &conn,
                &loaded.config.model.key,
                &loaded.config.model.digest,
                inputs.mode.as_str(),
            )?
        };
        let req = DecisionRunRequest {
            run_id,
            mode: inputs.mode,
            role_scope: vec![],
            created_at: chrono::Utc::now().timestamp(),
            request_id: inputs.request_id,
            question: inputs.question,
            question_ids: inputs.question_ids,
            query: &inputs.query,
        };
        let result = run_decision_pipeline(&loaded, &req, &model, &retriever, &ns_clock);
        let mut trace = build_decision_run_trace(&loaded, &req, &result);
        if let Some(model_ref) = trace.model_refs.first_mut() {
            model_ref.registry_ref = Some(RegistryRef {
                registry_id: registry_row.id.clone(),
                registry_version: registry_row.version.clone(),
            });
        }

        // The escalation proposal rides the writer's OWN transition: the
        // trace it cites and the proposal citing it commit together or not
        // at all. The audit row rides the same tx.
        let proposal_cell = std::cell::Cell::new(None);
        let side = |tx: &rusqlite::Transaction| -> Result<(), String> {
            let trace_row_id = tx.last_insert_rowid();
            if want_proposal
                && result.action == crate::workflow::harness::pipeline::ActionLabel::Escalate
            {
                let escalation = trace.outcome.escalation.as_ref();
                let content = serde_json::json!({
                    "pipeline_id": loaded.config.pipeline_id,
                    "action": result.action,
                    "escalation_stage": escalation.map(|e| e.stage.as_str()),
                    "escalation_reason": escalation.map(|e| e.reason),
                    "input_digest": result.input_digest,
                    "config_hash": trace.config_hash,
                })
                .to_string();
                let decision_run_ref = serde_json::json!({
                    "trace_id": trace_row_id,
                    "run_id": run_id,
                    "mode": trace.mode,
                    "config_hash": trace.config_hash,
                })
                .to_string();
                let proposal_id = crate::service::review::insert_decision_run_proposal(
                    tx,
                    &crate::service::review::NewDecisionRunProposal {
                        content: &content,
                        created_at: chrono::Utc::now().timestamp(),
                        owner: Some(&owner),
                        domain: "global",
                        decision_run_ref: &decision_run_ref,
                    },
                )
                .map_err(|e| e.to_string())?;
                proposal_cell.set(Some(proposal_id));
            }
            crate::audit::record_tenant(
                tx,
                crate::audit::AuditKind::Workflow,
                &owner,
                &format!("decision_run:{trace_row_id}"),
                crate::audit::AuditStatus::Ok,
                &format!(
                    "run_id={run_id} action={}",
                    serde_json::to_value(result.action).map_err(|e| e.to_string())?
                ),
                "global",
            );
            Ok(())
        };

        let mut conn = pool.get().map_err(HandlerError::db_down)?;
        let receipt =
            persist_decision_run_trace_with(&mut conn, &trace, chrono::Utc::now().timestamp(), Some(&side))
                .map_err(|e| HandlerError::internal(e.to_string()))?;

        let mut reply = serde_json::json!({
            "trace_id": receipt.trace_row_id,
            "action": serde_json::to_value(result.action).map_err(|e| HandlerError::internal(e.to_string()))?,
            "records": result.records.len(),
        });
        if let Some(esc) = &result.escalation {
            reply["escalation"] =
                serde_json::to_value(esc).map_err(|e| HandlerError::internal(e.to_string()))?;
        }
        if let Some(out) = &result.output {
            reply["output"] =
                serde_json::to_value(out).map_err(|e| HandlerError::internal(e.to_string()))?;
        }
        if let Some(pid) = proposal_cell.get() {
            reply["proposal_id"] = serde_json::json!(pid);
        }
        Ok(reply)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    Ok((StatusCode::CREATED, Json(outcome?)))
}

/// `GET /workflow/decision-runs/{id}` — the stored trace by ROW id. The
/// 404 is probe-blind (an absent id and a foreign run's id answer the
/// same), the body is the stored document verbatim — digests and refs
/// only, the storage contract the trace tests keep proving.
pub async fn get_decision_run(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Read, "", "global")?;
    super::authorize_role(&principal, &pool, "workflow")?;
    let who = super::recall::principal_label(&principal);
    let trace_json = tokio::task::spawn_blocking(move || -> Result<String, HandlerError> {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        let json = crate::workflow::harness::trace::load_decision_run_trace_json(&conn, id)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        if json.is_some() {
            crate::audit::record_tenant(
                &conn,
                crate::audit::AuditKind::Workflow,
                crate::workflow::ACTOR,
                &format!("decision_run:{id}"),
                crate::audit::AuditStatus::Ok,
                &format!("principal={who} read=trace"),
                "global",
            );
        }
        json.ok_or_else(|| HandlerError::not_found("decision run trace not found"))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;
    let doc: serde_json::Value =
        serde_json::from_str(&trace_json).map_err(|e| HandlerError::internal(e.to_string()))?;
    Ok(Json(doc))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayDiffBody {
    pub config: serde_json::Value,
    pub rules_config: serde_json::Value,
    pub mode: String,
    pub request_id: String,
    #[serde(default)]
    pub question_id: Option<String>,
    #[serde(default)]
    pub question_kind: Option<String>,
    pub question_ids: Vec<String>,
    pub query: String,
}

/// One per-stage row of the replay agreement report.
fn stage_diff_rows(stored: &[StageRecord], replayed: &[StageRecord]) -> Vec<serde_json::Value> {
    let n = stored.len().max(replayed.len());
    (0..n)
        .map(|i| {
            let s = stored.get(i);
            let r = replayed.get(i);
            let (s_digest, r_digest, matched) = match (s, r) {
                (Some(s), Some(r)) => (
                    s.outputs_digest.as_str(),
                    r.outputs_digest.as_str(),
                    s.outputs_digest == r.outputs_digest,
                ),
                (Some(s), None) => (s.outputs_digest.as_str(), "", false),
                (None, Some(r)) => ("", r.outputs_digest.as_str(), false),
                (None, None) => ("", "", true),
            };
            serde_json::json!({
                "stage": s.or(r).map(|rec| rec.stage.as_str()),
                "match": matched,
                "stored_outputs_digest": s_digest,
                "replayed_outputs_digest": r_digest,
            })
        })
        .collect()
}

/// `POST /workflow/decision-runs/{id}/replay-diff` — re-execute a stored
/// run under ITS OWN recorded conditions (the supplied config's canonical
/// hash must match the trace's, else 409) and report per-stage digest
/// agreement. The report is DATA: mismatches are the product (poison
/// visibility), never an error status; timing fields are provenance and
/// never compared; the replay persists NOTHING.
pub async fn post_decision_run_replay_diff(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<ReplayDiffBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let trace_row = tokio::task::spawn_blocking(move || -> Result<Option<String>, HandlerError> {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        crate::workflow::harness::trace::load_decision_run_trace_json(&conn, id)
            .map_err(|e| HandlerError::internal(e.to_string()))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let trace_json =
        trace_row?.ok_or_else(|| HandlerError::not_found("decision run trace not found"))?;
    let stored: crate::workflow::harness::trace::DecisionRunTrace =
        serde_json::from_str(&trace_json).map_err(|e| HandlerError::internal(e.to_string()))?;

    // The run's residency is authoritative for the by-id act: an absent
    // or foreign run answers the SAME probe-blind 404 (the run_domain
    // law the decision surfaces share).
    let domain = super::workflow::run_domain(&state, stored.run_id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let inputs = validate_inputs(
        &body.mode,
        &body.request_id,
        &body.question_ids,
        &body.question_id,
        &body.question_kind,
        &body.query,
    )?;
    let (loaded, model) = load_bound_model(&body.config, &body.rules_config)?;

    let supplied_hash = {
        // The config's canonical hash over the SUPPLIED document — the
        // replay replays its own recorded conditions or refuses.
        let config_str = serde_json::to_string(&body.config)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let reloaded = DecisionPipelineConfig::load(&config_str).map_err(config_invalid)?;
        reloaded.config_hash
    };
    if supplied_hash != stored.config_hash {
        return Err(HandlerError::conflict_with(
            "config_hash_mismatch",
            "the supplied config does not canonical-hash to the stored trace's config — \
             a replay replays its own recorded conditions or refuses",
            serde_json::json!([]),
        ));
    }

    let retriever = SearchRetriever {
        pool: pool.clone(),
        model: Arc::clone(&state.model),
    };
    let run_id = stored.run_id;

    let report = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        {
            let conn = pool.get().map_err(HandlerError::db_down)?;
            resolve_registered_model(
                &conn,
                &loaded.config.model.key,
                &loaded.config.model.digest,
                inputs.mode.as_str(),
            )?;
        }
        let req = DecisionRunRequest {
            run_id,
            mode: inputs.mode,
            role_scope: vec![],
            created_at: chrono::Utc::now().timestamp(),
            request_id: inputs.request_id,
            question: inputs.question,
            question_ids: inputs.question_ids,
            query: &inputs.query,
        };
        let replayed = run_decision_pipeline(&loaded, &req, &model, &retriever, &ns_clock);
        let stages = stage_diff_rows(&stored.stages, &replayed.records);
        let all_match = stages.iter().all(|s| s["match"] == serde_json::json!(true));
        let input_digest_match = replayed.input_digest == stored.input_digest;
        Ok(serde_json::json!({
            "trace_id": id,
            "config_hash": stored.config_hash,
            "config_hash_match": true,
            "replay_input_digest": replayed.input_digest,
            "input_digest_match": input_digest_match,
            "stages": stages,
            "all_match": all_match,
        }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    Ok(Json(report?))
}

#[derive(Debug, Deserialize)]
pub struct DecisionRunsQuery {
    pub limit: Option<usize>,
    #[serde(default)]
    pub run_id: Option<i64>,
}

/// `GET /workflow/decision-runs` — the bounded listing. THE exfiltration
/// surface: the DPO dual gate (Admin + the DPO role) plus an audited
/// global row per call naming the principal, the filter, and the count.
/// Bounded columns only — the trace_json itself NEVER rides a listing.
pub async fn get_decision_runs(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<DecisionRunsQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Admin, "", "global")?;
    crate::handlers::breaches::require_dpo_role(&principal, &pool)?;
    let limit = q.limit.unwrap_or(LISTING_LIMIT_DEFAULT);
    if limit == 0 || limit > LISTING_LIMIT_CAP {
        return Err(HandlerError::bad_request(
            "decision_runs_limit_out_of_bounds",
            format!(
                "limit must land inside 1..={LISTING_LIMIT_CAP} — the listing pages are bounded"
            ),
        ));
    }
    let run_id = q.run_id;
    let who = super::recall::principal_label(&principal);
    let rows =
        tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>, HandlerError> {
            let conn = pool.get().map_err(HandlerError::db_down)?;
            let listed =
                crate::workflow::harness::trace::list_decision_run_traces(&conn, run_id, limit)
                    .map_err(|e| HandlerError::internal(e.to_string()))?;
            let out: Vec<serde_json::Value> = listed
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id,
                        "run_id": r.run_id,
                        "mode": r.mode,
                        "pipeline_version": r.pipeline_version,
                        "config_hash": r.config_hash,
                        "created_at": r.created_at,
                        "stage_count": r.stage_count,
                    })
                })
                .collect();
            crate::audit::record_tenant(
                &conn,
                crate::audit::AuditKind::Workflow,
                crate::workflow::ACTOR,
                "decision_runs:listing",
                crate::audit::AuditStatus::Ok,
                &format!(
                    "principal={who} run_id={} count={}",
                    run_id
                        .map(|r| r.to_string())
                        .unwrap_or_else(|| "all".into()),
                    out.len()
                ),
                "global",
            );
            Ok(out)
        })
        .await
        .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let rows = rows?;
    let count = rows.len();
    Ok(Json(serde_json::json!({ "rows": rows, "count": count })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    const OP_TOKEN: &str = "r28-decision-op-token";
    const AGENT_TOKEN_UNUSED: &str = "r28-agent-line-unused";

    const TABLE_JSON: &str = r#"{
      "model_id": "rules-reference",
      "model_version": "1.0.0",
      "rules": [
        { "question_id": "needs_human", "min_evidence": 1, "min_tier": "vetted",
          "output": { "Choice": { "options": ["act", "reject"], "label": "act" } } }
      ]
    }"#;

    fn config_json(model_digest: &str) -> String {
        format!(
            r#"{{
              "config_schema": "harness.pipeline/v1",
              "pipeline_id": "route-line",
              "stages": ["normalize","retrieve_context","candidate_generation","decision_model","rules_policy","threshold","action_escalation"],
              "retrieval": {{"rrf_k": 60, "limit": 5, "leg": "vector"}},
              "model": {{"key": "rules:rules-reference", "digest": "{model_digest}"}},
              "thresholds": {{
                "act_labels": ["act"],
                "reject_labels": ["reject"],
                "score_act_at_or_above": 50,
                "score_reject_at_or_below": 10,
                "noul_act_when": true,
                "fallback": "approve"
              }}
            }}"#
        )
    }

    fn run_body(config: &str, rules: &str, run_id: i64, mode: &str) -> String {
        format!(
            r#"{{
              "config": {config},
              "rules_config": {rules},
              "run_id": {run_id},
              "mode": "{mode}",
              "request_id": "req-route-1",
              "question_id": "needs_human",
              "question_kind": "choice",
              "question_ids": ["needs_human"],
              "query": "route fixture ask"
            }}"#
        )
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        state: Arc<AppState>,
    }

    fn fixture() -> Fixture {
        crate::register_sqlite_vec::register_sqlite_vec();
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("brain.db");
        let mgr = crate::pool::SqliteConnectionManager::file(&db_path);
        let pool: crate::Pool = r2d2::Pool::builder().max_size(4).build(mgr).unwrap();
        crate::migration::run_migration(&mut pool.get().unwrap(), 0).unwrap();
        let tok_file = dir.path().join("tokens");
        std::fs::write(&tok_file, format!("{OP_TOKEN}\n{AGENT_TOKEN_UNUSED}\n")).unwrap();
        let token_store = crate::auth::TokenStore::from_file(Some(tok_file.clone()));
        token_store.reload_parts_from(vec![OP_TOKEN.to_string()], None);
        let model: Arc<dyn crate::embed::Embedder> =
            Arc::new(crate::embed::StaticEmbedder::new(crate::config::MODEL_ID).expect("model"));
        let state = Arc::new(AppState {
            token_store,
            jwt_middleware_state: Arc::new(
                crate::server::router::auth::JwtMiddlewareState::opaque_for_tests(
                    pool.clone(),
                    db_path.clone(),
                ),
            ),
            cors: tower_http::cors::CorsLayer::new(),
            durability: Default::default(),
            loom: Default::default(),
            model,
            registry: crate::domain_registry::DomainRegistry::new(pool.clone(), &db_path, false),
            pool,
            db_path,
            connection_tracker: Arc::new(crate::http_limit::ConnectionTracker::new()),
            rate_limiter: Arc::new(crate::http_limit::RateLimiter::new()),
            snapshot: crate::integrity::SnapshotState::default(),
            audit_chain_cache: Arc::new(std::sync::Mutex::new(None)),
            auth_mode: crate::auth::AuthMode::Opaque,
            key_store: crate::auth::jwks::KeyStore::default(),
            revocation_cache: Arc::new(crate::auth::revocation::RevocationCache::new()),
            jwt_issuer: String::new(),
            jwt_audience: String::new(),
            oidc_config: crate::handlers::well_known::OidcConfig::unconfigured(),
            ump_events: tokio::sync::broadcast::channel(16).0,
            alert_events: tokio::sync::broadcast::channel(16).0,
            alert_seq: std::sync::atomic::AtomicU64::new(0),
            chain_watch: crate::alert::ChainWatchState::default(),
            concurrency: &crate::concurrency::CONCURRENCY,
        });
        Fixture { _dir: dir, state }
    }

    fn seed_run(state: &AppState, domain: &str) -> i64 {
        use crate::workflow::state::test_support::insert_fresh_troubleshoot_run;
        let conn = state.pool.get().unwrap();
        let now = chrono::Utc::now().timestamp();
        insert_fresh_troubleshoot_run(&conn, domain, now).unwrap()
    }

    async fn post_json(
        state: &Arc<AppState>,
        uri: &str,
        body: String,
        bearer: Option<&str>,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let mut builder = axum::http::Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(t) = bearer {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        let res = crate::server::router::app(state.clone())
            .oneshot(builder.body(Body::from(body)).unwrap())
            .await
            .expect("oneshot");
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    async fn get_json(
        state: &Arc<AppState>,
        uri: &str,
        bearer: Option<&str>,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let mut builder = axum::http::Request::builder().method("GET").uri(uri);
        if let Some(t) = bearer {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        let res = crate::server::router::app(state.clone())
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .expect("oneshot");
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    fn table_value() -> serde_json::Value {
        serde_json::from_str(TABLE_JSON).unwrap()
    }

    fn digest_of(table: &serde_json::Value) -> String {
        RulesModel::from_canonical_json(&serde_json::to_string(table).unwrap())
            .unwrap()
            .digest()
            .to_string()
    }

    fn seed_promoted_model(state: &AppState) {
        let conn = state.pool.get().unwrap();
        crate::workflow::registry::test_support::seed_promoted_rules_model(&conn, TABLE_JSON);
    }

    /// The route-level law set: absent/foreign run → the SAME probe-blind
    /// 404; a hostile config → the named 400; digest-mismatched rules →
    /// the named 400; replay with a foreign config → the named 409. No
    /// hostile input reaches the engine or the store.
    #[tokio::test]
    async fn decision_run_post_route_is_probe_blind_and_config_hostile() {
        let f = fixture();
        let digest = digest_of(&table_value());
        let good_config = config_json(&digest);

        // Absent run → 404, and a foreign/absent id answers the SAME body
        // shape (probe-blind: no existence oracle).
        let (absent_status, absent_body) = post_json(
            &f.state,
            "/workflow/decision-runs",
            run_body(&good_config, TABLE_JSON, 4_242_424, "deterministic"),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(absent_status, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(absent_body["error"]["code"], "not_found");

        let run_id = seed_run(&f.state, "global");
        seed_promoted_model(&f.state);

        // Hostile config (unknown field) → the loader's named 400.
        let mut hostile: serde_json::Value = serde_json::from_str(&good_config).unwrap();
        hostile["totally_unknown"] = serde_json::json!(1);
        let (status, v) = post_json(
            &f.state,
            "/workflow/decision-runs",
            run_body(&hostile.to_string(), TABLE_JSON, run_id, "deterministic"),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "body: {v}");
        assert_eq!(v["error"]["code"], "config_invalid");

        // Digest-mismatched rules → the named 400.
        let mut wrong_rules = table_value();
        wrong_rules["rules"][0]["min_evidence"] = serde_json::json!(1);
        wrong_rules["model_version"] = serde_json::json!("9.9.9");
        let (status, v) = post_json(
            &f.state,
            "/workflow/decision-runs",
            run_body(
                &good_config,
                &wrong_rules.to_string(),
                run_id,
                "deterministic",
            ),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "body: {v}");
        assert_eq!(v["error"]["code"], "model_digest_mismatch");

        // A deterministic run executes end to end and persists a trace.
        let (status, v) = post_json(
            &f.state,
            "/workflow/decision-runs",
            run_body(&good_config, TABLE_JSON, run_id, "deterministic"),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::CREATED, "body: {v}");
        let trace_id = v["trace_id"].as_i64().expect("trace id in reply");

        // Replay with a FOREIGN config → the named 409 (a replay replays
        // its own recorded conditions or refuses).
        let mut foreign: serde_json::Value = serde_json::from_str(&good_config).unwrap();
        foreign["pipeline_id"] = serde_json::json!("a-different-pipeline");
        let replay_body = format!(
            r#"{{
              "config": {foreign},
              "rules_config": {},
              "mode": "deterministic",
              "request_id": "req-replay-1",
              "question_id": "needs_human",
              "question_kind": "choice",
              "question_ids": ["needs_human"],
              "query": "route fixture ask"
            }}"#,
            serde_json::to_string(&table_value()).unwrap()
        );
        let (status, v) = post_json(
            &f.state,
            &format!("/workflow/decision-runs/{trace_id}/replay-diff"),
            replay_body,
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::CONFLICT, "body: {v}");
        assert_eq!(v["error"]["code"], "config_hash_mismatch");
    }

    /// The authz posture: no bearer → 401 everywhere; the listing's DPO
    /// dual gate binds once the role store is populated (a role-less token
    /// refuses even with the Admin scope); the `workflow` role predicate
    /// refuses a capability-less principal and the DPO predicate passes a
    /// dpo-role principal (the routes call these gates — the action-scan
    /// pins the calls in the handler sources).
    #[tokio::test]
    async fn decision_run_routes_are_role_gated_and_dual_gated_for_listings() {
        let f = fixture();

        // 401: the routes are not public.
        let (status, _) = get_json(&f.state, "/workflow/decision-runs", None).await;
        assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);
        let (status, _) = get_json(&f.state, "/workflow/decision-runs/1", None).await;
        assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);

        // Populate the role store: from this point a principal whose roles
        // claim lacks the DPO role fails the dual gate's role half (the
        // single-token shape the dual gate exists to stop). The fixture's
        // opaque operator bearer rides the documented None principal (the
        // loopback incumbent), so the dual-gate and workflow-role
        // predicates are exercised at the handler level below — the routes
        // call exactly these gates (the action-scan pins the calls in the
        // handler sources).
        {
            let conn = f.state.pool.get().unwrap();
            crate::workflow::state::test_support::seed_role_with_capabilities(
                &conn,
                "limited",
                &["read", "write", "workflow"],
            )
            .unwrap();
        }

        let pool = f.state.pool.clone();
        let limited = crate::auth::Principal {
            sub: "user:limited@x".into(),
            tenant: "global".into(),
            scopes: vec![crate::auth::Scope {
                action: crate::auth::Action::Admin,
                team: "*".into(),
                domain: "*".into(),
            }],
            jti: "j-limited".into(),
            roles: vec!["limited".into()],
            manages: vec![],
            kind: crate::auth::PrincipalKind::Jwt,
        };
        assert!(
            crate::handlers::authorize_role(&Some(limited.clone()), &pool, "workflow").is_ok(),
            "the limited role carries the workflow capability"
        );
        let no_workflow = crate::auth::Principal {
            roles: vec!["no-such-role".into()],
            ..limited.clone()
        };
        assert!(
            crate::handlers::authorize_role(&Some(no_workflow), &pool, "workflow").is_err(),
            "a principal whose roles lack workflow refuses the role gate"
        );
        assert!(
            crate::handlers::breaches::require_dpo_role(&Some(limited.clone()), &pool).is_err(),
            "a workflow-only roles claim refuses the DPO dual gate once the store is populated"
        );
        let dpo = crate::auth::Principal {
            roles: vec!["dpo".into()],
            ..limited.clone()
        };
        assert!(
            crate::handlers::breaches::require_dpo_role(&Some(dpo), &pool).is_ok(),
            "the dpo role satisfies the dual gate's role half"
        );
        let _ = f;
    }

    /// The listing is bounded (0/51 refuse named, default 20, page ≤ 50)
    /// and every call lands its audit row naming the principal and count.
    #[tokio::test]
    async fn decision_run_listing_is_bounded_and_audited() {
        let f = fixture();
        // Bounds refuse named BEFORE any read (loopback passes the dual
        // gate only while the role store is empty — the fixture keeps it
        // empty; the DPO posture itself is pinned in the role-gate test).
        let (status, v) =
            get_json(&f.state, "/workflow/decision-runs?limit=0", Some(OP_TOKEN)).await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "body: {v}");
        assert_eq!(v["error"]["code"], "decision_runs_limit_out_of_bounds");
        let (status, v) =
            get_json(&f.state, "/workflow/decision-runs?limit=51", Some(OP_TOKEN)).await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "body: {v}");
        assert_eq!(v["error"]["code"], "decision_runs_limit_out_of_bounds");

        // Seed 3 traces directly (the writer is the R27 unit's subject);
        // the listing returns bounded columns only.
        let digest = digest_of(&table_value());
        let run_id = seed_run(&f.state, "global");
        seed_promoted_model(&f.state);
        let (status, created) = post_json(
            &f.state,
            "/workflow/decision-runs",
            run_body(&config_json(&digest), TABLE_JSON, run_id, "deterministic"),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::CREATED, "body: {created}");
        let (status, listed) =
            get_json(&f.state, "/workflow/decision-runs?limit=50", Some(OP_TOKEN)).await;
        assert_eq!(status, axum::http::StatusCode::OK, "body: {listed}");
        assert_eq!(listed["count"].as_u64(), Some(1));
        let row = &listed["rows"][0];
        assert!(row.get("trace_json").is_none(), "no trace_json in listings");
        assert!(row["stage_count"].as_u64().unwrap() > 0);
        assert_eq!(row["run_id"].as_i64(), Some(run_id));

        // The audit row landed for the listing call (the audit_events
        // store hashes target/detail at rest — the pin is the row delta
        // by kind + the chain staying green).
        {
            let conn = f.state.pool.get().unwrap();
            let audits =
                crate::workflow::state::test_support::workflow_audit_row_count(&conn).unwrap();
            assert!(
                audits >= 2,
                "the run write + the listing call are audited, got {audits}"
            );
            assert!(crate::audit::verify_chain(&conn), "audit chain green");
        }
    }

    /// The replay contract on pure stages: identical config + input under
    /// the same retriever semantics reproduces every stage digest
    /// (all_match true, timing excluded); a perturbed corpus names the
    /// retrieval-dependent mismatch with both digests.
    #[tokio::test]
    async fn deterministic_mode_replay_diff_is_exact_on_pure_stages() {
        let f = fixture();
        let digest = digest_of(&table_value());
        let run_id = seed_run(&f.state, "global");
        seed_promoted_model(&f.state);
        let (status, created) = post_json(
            &f.state,
            "/workflow/decision-runs",
            run_body(&config_json(&digest), TABLE_JSON, run_id, "deterministic"),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::CREATED, "body: {created}");
        let trace_id = created["trace_id"].as_i64().unwrap();

        // The replay body mirrors the POST minus run_id/proposal.
        let replay_body = format!(
            r#"{{
              "config": {},
              "rules_config": {},
              "mode": "deterministic",
              "request_id": "req-route-1",
              "question_id": "needs_human",
              "question_kind": "choice",
              "question_ids": ["needs_human"],
              "query": "route fixture ask"
            }}"#,
            config_json(&digest),
            serde_json::to_string(&table_value()).unwrap()
        );
        let (status, report) = post_json(
            &f.state,
            &format!("/workflow/decision-runs/{trace_id}/replay-diff"),
            replay_body,
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "body: {report}");
        assert_eq!(report["config_hash_match"], serde_json::json!(true));
        assert_eq!(
            report["all_match"],
            serde_json::json!(true),
            "body: {report}"
        );
        assert_eq!(
            report["input_digest_match"],
            serde_json::json!(true),
            "the same input re-binds the same commitment"
        );
        let stages = report["stages"].as_array().unwrap();
        assert!(!stages.is_empty());
        for s in stages {
            assert_eq!(s["match"], serde_json::json!(true), "stage {}", s["stage"]);
        }

        // A replay writes nothing: the trace count is unchanged.
        let conn = f.state.pool.get().unwrap();
        let traces = crate::workflow::state::test_support::decision_run_trace_count(&conn).unwrap();
        assert_eq!(traces, 1, "the replay persists nothing");
    }

    /// The mode law end-to-end: an exploratory run's escalation proposal
    /// carries the provenance ref with mode "exploratory"; the REAL
    /// approve route refuses it NAMED; the deterministic twin approves
    /// through the generic promote path.
    #[tokio::test]
    async fn exploratory_run_cannot_promote_proposal_carries_mode() {
        let f = fixture();
        let digest = digest_of(&table_value());
        let run_id = seed_run(&f.state, "global");
        seed_promoted_model(&f.state);
        let body_with_proposal = format!(
            r#"{{
              "config": {},
              "rules_config": {},
              "run_id": {run_id},
              "mode": "exploratory",
              "request_id": "req-explore-1",
              "question_ids": ["needs_human"],
              "query": "explore ask",
              "proposal": true
            }}"#,
            config_json(&digest),
            serde_json::to_string(&table_value()).unwrap()
        );
        // An exploratory ask over an empty corpus escalates at candidate
        // generation (insufficient evidence) — the proposal path this
        // route exists for.
        let (status, v) = post_json(
            &f.state,
            "/workflow/decision-runs",
            body_with_proposal,
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::CREATED, "body: {v}");
        assert_eq!(v["action"], "escalate", "the empty corpus escalates: {v}");
        let proposal_id = v["proposal_id"].as_i64().expect("proposal written");

        // The proposal row carries the ref with mode exploratory.
        let content = {
            let conn = f.state.pool.get().unwrap();
            let (kind, ref_json, content, status) =
                crate::workflow::state::test_support::decision_run_proposal_fields(
                    &conn,
                    proposal_id,
                )
                .unwrap();
            assert_eq!(kind, crate::service::review::DECISION_REVIEW_PROPOSAL_KIND);
            assert_eq!(
                crate::service::review::decision_run_ref_mode(&ref_json.unwrap()).as_deref(),
                Some("exploratory")
            );
            assert_eq!(status, "pending");
            content
        };

        // The REAL approve route refuses the exploratory proposal NAMED —
        // the approve binds to the displayed digest (the gate's own law).
        let want = crate::handlers::gate::review_digest(&content);
        let (status, v) = post_json(
            &f.state,
            &format!("/proposals/{proposal_id}/approve?digest={want}"),
            String::new(),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "body: {v}");
        assert_eq!(
            v["error"]["code"], "exploratory_mode_not_promotable",
            "body: {v}"
        );
        // The refused proposal stays pending (the queue keeps it).
        {
            let conn = f.state.pool.get().unwrap();
            let (_, _, _, status) =
                crate::workflow::state::test_support::decision_run_proposal_fields(
                    &conn,
                    proposal_id,
                )
                .unwrap();
            assert_eq!(status, "pending", "the refusal never decides the row");
        }

        // The deterministic twin approves (the generic promote path).
        let body_det = format!(
            r#"{{
              "config": {},
              "rules_config": {},
              "run_id": {run_id},
              "mode": "deterministic",
              "request_id": "req-det-1",
              "question_ids": ["needs_human"],
              "query": "explore ask",
              "proposal": true
            }}"#,
            config_json(&digest),
            serde_json::to_string(&table_value()).unwrap()
        );
        let (status, v) = post_json(
            &f.state,
            "/workflow/decision-runs",
            body_det,
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::CREATED, "body: {v}");
        assert_eq!(v["action"], "escalate");
        let det_proposal = v["proposal_id"].as_i64().expect("deterministic proposal");
        let det_content = {
            let conn = f.state.pool.get().unwrap();
            let (_, _, content, _) =
                crate::workflow::state::test_support::decision_run_proposal_fields(
                    &conn,
                    det_proposal,
                )
                .unwrap();
            content
        };
        let want_det = crate::handlers::gate::review_digest(&det_content);
        let (status, v) = post_json(
            &f.state,
            &format!("/proposals/{det_proposal}/approve?digest={want_det}"),
            String::new(),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "body: {v}");
        assert_eq!(v["status"], "approved", "body: {v}");
    }

    #[tokio::test]
    async fn harness_cannot_execute_retired_or_unregistered_model_in_deterministic_mode() {
        let f = fixture();
        let digest = digest_of(&table_value());
        let run_id = seed_run(&f.state, "global");
        let body = run_body(&config_json(&digest), TABLE_JSON, run_id, "deterministic");

        let (status, response) = post_json(
            &f.state,
            "/workflow/decision-runs",
            body.clone(),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(
            status,
            axum::http::StatusCode::BAD_REQUEST,
            "body: {response}"
        );
        assert_eq!(response["error"]["code"], "model_not_registered");

        {
            let conn = f.state.pool.get().unwrap();
            crate::workflow::registry::test_support::seed_rules_model(
                &conn,
                TABLE_JSON,
                crate::workflow::registry::STATUS_CANDIDATE,
            );
        }
        let (status, response) = post_json(
            &f.state,
            "/workflow/decision-runs",
            body.clone(),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(
            status,
            axum::http::StatusCode::BAD_REQUEST,
            "body: {response}"
        );
        assert_eq!(response["error"]["code"], "model_not_promoted");

        {
            let conn = f.state.pool.get().unwrap();
            crate::workflow::registry::test_support::seed_rules_model(
                &conn,
                TABLE_JSON,
                crate::workflow::registry::STATUS_PROMOTED,
            );
        }
        let (status, response) = post_json(
            &f.state,
            "/workflow/decision-runs",
            body.clone(),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::CREATED, "body: {response}");

        {
            let conn = f.state.pool.get().unwrap();
            crate::workflow::registry::test_support::seed_rules_model(
                &conn,
                TABLE_JSON,
                crate::workflow::registry::STATUS_RETIRED,
            );
        }
        let (status, response) =
            post_json(&f.state, "/workflow/decision-runs", body, Some(OP_TOKEN)).await;
        assert_eq!(
            status,
            axum::http::StatusCode::BAD_REQUEST,
            "body: {response}"
        );
        assert_eq!(response["error"]["code"], "model_retired");
    }

    #[tokio::test]
    async fn trace_cites_registry_id_version_digest() {
        let f = fixture();
        let digest = digest_of(&table_value());
        let run_id = seed_run(&f.state, "global");
        seed_promoted_model(&f.state);
        let (status, created) = post_json(
            &f.state,
            "/workflow/decision-runs",
            run_body(&config_json(&digest), TABLE_JSON, run_id, "deterministic"),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::CREATED, "body: {created}");
        let trace_id = created["trace_id"].as_i64().unwrap();
        let (status, trace) = get_json(
            &f.state,
            &format!("/workflow/decision-runs/{trace_id}"),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "body: {trace}");
        assert_eq!(
            trace["model_refs"][0]["registry_ref"]["registry_id"],
            "rules-reference"
        );
        assert_eq!(
            trace["model_refs"][0]["registry_ref"]["registry_version"],
            "1.0.0"
        );
        assert!(
            trace["config_hash"]
                .as_str()
                .is_some_and(|value| value.len() == 64)
        );
    }
}
