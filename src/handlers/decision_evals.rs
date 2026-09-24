//! Protocol adapters for bounded decision-evaluation records.
//!
//! The handler parses and authorizes only. Judgment-set validation, trace
//! loading, metric calculation, canonicalization, persistence, and checked
//! audit writes live in `crate::workflow::decision_eval`.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::workflow::decision_eval::{
    self, CreateEvaluationBody, DEFAULT_LIST_LIMIT, EvaluationError, MAX_LIST_LIMIT,
};

fn evaluation_error(error: EvaluationError) -> HandlerError {
    match error {
        EvaluationError::Invalid { code, message } => HandlerError::bad_request(code, message),
        EvaluationError::JudgmentSetUnavailable => HandlerError::bad_request(
            "judgment_set_unavailable",
            "a valid decision judgment set could not be proven from the supplied manifest and persisted traces",
        ),
        EvaluationError::TargetMismatch => HandlerError::bad_request(
            "evaluation_target_mismatch",
            "the evaluation target does not match the persisted trace/model binding",
        ),
        EvaluationError::ModelArtifactDigestRequired => HandlerError::bad_request(
            "model_artifact_digest_required",
            "a learned model target requires an artifact digest",
        ),
        EvaluationError::IdempotencyConflict => HandlerError::conflict_with(
            "evaluation_idempotency_conflict",
            "the idempotency key is already bound to a different evaluation request",
            serde_json::json!({}),
        ),
        EvaluationError::RecordTampered => {
            HandlerError::internal("stored evaluation record digest verification failed")
        }
        EvaluationError::Audit(message) => {
            HandlerError::internal(format!("checked evaluation audit write failed: {message}"))
        }
        EvaluationError::Db(message) => HandlerError::internal(message.to_string()),
    }
}

fn validate_evaluation_limit(limit: Option<usize>) -> Result<usize, HandlerError> {
    let limit = limit.unwrap_or(DEFAULT_LIST_LIMIT);
    if (1..=MAX_LIST_LIMIT).contains(&limit) {
        Ok(limit)
    } else {
        Err(HandlerError::bad_request(
            "evaluation_limit_out_of_bounds",
            format!("limit must land inside 1..={MAX_LIST_LIMIT}"),
        ))
    }
}

fn parse_create_body(raw: serde_json::Value) -> Result<CreateEvaluationBody, HandlerError> {
    if !matches!(raw.get("judgment_set"), Some(value) if !value.is_null()) {
        return Err(HandlerError::bad_request(
            "judgment_set_unavailable",
            "a decision judgment set is required before evaluation can run",
        ));
    }
    serde_json::from_value(raw).map_err(|_| {
        HandlerError::bad_request(
            "evaluation_request_invalid",
            "the evaluation request does not match the strict typed contract",
        )
    })
}

fn parse_evaluation_id(raw: &str) -> Result<String, HandlerError> {
    if raw.len() > 96
        || !raw.starts_with("eval_")
        || raw.len() != 37
        || !raw[5..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(HandlerError::bad_request(
            "evaluation_id_invalid",
            "evaluation id must be the stable eval_ plus 32 hexadecimal characters",
        ));
    }
    Ok(raw.to_string())
}

#[derive(Debug, Deserialize)]
pub(crate) struct DecisionEvalQuery {
    pub limit: Option<usize>,
}

/// Create one operator-declared, non-authoritative evaluation record.
pub(crate) async fn post_decision_eval(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(raw_body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Admin, "", "global")?;
    super::breaches::require_dpo_role(&principal, &pool)?;
    let body = parse_create_body(raw_body)?;
    let actor = super::recall::principal_label(&principal);
    let now = chrono::Utc::now().timestamp();
    let receipt = tokio::task::spawn_blocking(move || {
        let mut connection = pool.get().map_err(HandlerError::db_down)?;
        decision_eval::create_evaluation(&mut connection, body, &actor, now)
            .map_err(evaluation_error)
    })
    .await
    .map_err(|error| HandlerError::internal(format!("{error}")))??;
    let mut response = serde_json::to_value(receipt.record)
        .map_err(|error| HandlerError::internal(error.to_string()))?;
    super::sanitize_value_strings(&mut response);
    let status = if receipt.created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(response)))
}

/// Read one evaluation by stable id. Authorization precedes lookup so an
/// absent id is probe-blind for unauthorized callers.
pub(crate) async fn get_decision_eval(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(evaluation_id): Path<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Admin, "", "global")?;
    super::breaches::require_dpo_role(&principal, &pool)?;
    let evaluation_id = parse_evaluation_id(&evaluation_id)?;
    let actor = super::recall::principal_label(&principal);
    let record = tokio::task::spawn_blocking(move || {
        let connection = pool.get().map_err(HandlerError::db_down)?;
        let record =
            decision_eval::get_evaluation(&connection, &evaluation_id).map_err(evaluation_error)?;
        if let Some(record) = record.as_ref() {
            crate::audit::record_tenant_checked(
                &connection,
                crate::audit::AuditKind::Workflow,
                crate::workflow::ACTOR,
                &record.audit_target,
                crate::audit::AuditStatus::Ok,
                &format!("principal={actor} read=decision_evaluation"),
                "global",
            )
            .map_err(|error| HandlerError::internal(error.to_string()))?;
        }
        Ok::<_, HandlerError>(record)
    })
    .await
    .map_err(|error| HandlerError::internal(format!("{error}")))??;
    let record = record.ok_or_else(|| HandlerError::not_found("evaluation record not found"))?;
    let mut response =
        serde_json::to_value(record).map_err(|error| HandlerError::internal(error.to_string()))?;
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

/// List bounded evaluation metadata. The full manifest/report never rides
/// this exfiltration surface; the DPO dual gate and audit are mandatory.
pub(crate) async fn get_decision_evals(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(query): Query<DecisionEvalQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Admin, "", "global")?;
    super::breaches::require_dpo_role(&principal, &pool)?;
    let limit = validate_evaluation_limit(query.limit)?;
    let actor = super::recall::principal_label(&principal);
    let rows = tokio::task::spawn_blocking(move || {
        let connection = pool.get().map_err(HandlerError::db_down)?;
        let rows = decision_eval::list_evaluations(&connection, limit).map_err(evaluation_error)?;
        crate::audit::record_tenant_checked(
            &connection,
            crate::audit::AuditKind::Workflow,
            crate::workflow::ACTOR,
            "decision_evaluations:listing",
            crate::audit::AuditStatus::Ok,
            &format!("principal={actor} count={} limit={limit}", rows.len()),
            "global",
        )
        .map_err(|error| HandlerError::internal(error.to_string()))?;
        Ok::<_, HandlerError>(rows)
    })
    .await
    .map_err(|error| HandlerError::internal(format!("{error}")))??;
    let values: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "evaluation_id": row.evaluation_id,
                "pipeline_version": row.pipeline_version,
                "config_hash": row.config_hash,
                "model_registry_id": row.model_registry_id,
                "model_registry_version": row.model_registry_version,
                "judgment_set_ref": row.judgment_set_ref,
                "judgment_set_digest": row.judgment_set_digest,
                "judgment_set_count": row.judgment_set_count,
                "acceptance_state": row.acceptance_state,
                "created_at": row.created_at,
            })
        })
        .collect();
    let count = values.len();
    let mut response = serde_json::json!({ "rows": values, "count": count });
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    const OP_TOKEN: &str = "r30-decision-eval-operator";

    struct Fixture {
        _dir: tempfile::TempDir,
        state: Arc<AppState>,
    }

    fn fixture() -> Fixture {
        crate::register_sqlite_vec::register_sqlite_vec();
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("brain.db");
        let manager = crate::pool::SqliteConnectionManager::file(&db_path);
        let pool: crate::Pool = r2d2::Pool::builder().max_size(4).build(manager).unwrap();
        crate::migration::run_migration(&mut pool.get().unwrap(), 0).unwrap();
        let token_path = dir.path().join("tokens");
        std::fs::write(&token_path, format!("{OP_TOKEN}\n")).unwrap();
        let token_store = crate::auth::TokenStore::from_file(Some(token_path));
        token_store.reload_parts_from(vec![OP_TOKEN.to_string()], None);
        let model: Arc<dyn crate::embed::Embedder> =
            Arc::new(crate::embed::StaticEmbedder::new(crate::config::MODEL_ID).unwrap());
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

    async fn request(
        state: &Arc<AppState>,
        method: &str,
        uri: &str,
        body: String,
        token: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let response = crate::server::router::app(state.clone())
            .oneshot(builder.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn decision_eval_routes_are_role_gated_and_dual_gated_for_listings() {
        let fixture = fixture();
        for (method, path) in [
            ("GET", "/workflow/decision-evals"),
            (
                "GET",
                "/workflow/decision-evals/eval_00000000000000000000000000000000",
            ),
            ("POST", "/workflow/decision-evals"),
        ] {
            let (status, _) = request(
                &fixture.state,
                method,
                path,
                if method == "POST" {
                    "{}".into()
                } else {
                    String::new()
                },
                None,
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
        }
        let (status, body) = request(
            &fixture.state,
            "GET",
            "/workflow/decision-evals",
            String::new(),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "opaque operator listing: {body}");
        let (status, body) = request(
            &fixture.state,
            "POST",
            "/workflow/decision-evals",
            "{}".into(),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "missing judgment set: {body}"
        );
        assert_eq!(body["error"]["code"], "judgment_set_unavailable");
        let (status, body) = request(
            &fixture.state,
            "GET",
            "/workflow/decision-evals/eval_00000000000000000000000000000000",
            String::new(),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "absent detail is probe-blind: {body}"
        );
    }
}
