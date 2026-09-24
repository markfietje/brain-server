//! Model-registry protocol adapters: bounded listing, one-row lookup, and
//! operator registration. Storage, lifecycle transitions, and execution
//! resolution live in [`crate::workflow::registry`]; this module only parses,
//! authorizes, delegates, and shapes responses.

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
use crate::workflow::registry::{
    self, KIND_DETERMINISTIC_RULES, KIND_LEARNED, KIND_RERANKER, Registration, RegistryError,
};

const MODEL_REGISTRY_LISTING_LIMIT_DEFAULT: usize = 20;
const MODEL_REGISTRY_LISTING_LIMIT_CAP: usize = 50;

fn registry_error(error: RegistryError) -> HandlerError {
    match error {
        RegistryError::RulesConfig(message) => {
            HandlerError::bad_request("rules_config_invalid", message)
        }
        RegistryError::IdentityDeclared => HandlerError::bad_request(
            "registry_identity_declared",
            "the deterministic rules document declares identity; do not submit identity fields",
        ),
        RegistryError::IdentityRequired => HandlerError::bad_request(
            "registry_identity_required",
            "id, version, name, and output_vocabulary are required for a declared model",
        ),
        RegistryError::VocabularyInvalid => HandlerError::bad_request(
            "registry_vocabulary_invalid",
            "output_vocabulary must be a non-empty subset of choice, score, and noul",
        ),
        RegistryError::ArtifactDigestRequired => HandlerError::bad_request(
            "artifact_digest_required",
            "a learned model registration must carry its artifact digest",
        ),
        RegistryError::ArtifactDigestInvalid => HandlerError::bad_request(
            "artifact_digest_invalid",
            "artifact_digest must be 64 lowercase hexadecimal characters",
        ),
        RegistryError::ConfigDigestInvalid => HandlerError::bad_request(
            "config_digest_invalid",
            "config_digest must be 64 lowercase hexadecimal characters",
        ),
        RegistryError::IdInvalid(message) => {
            HandlerError::bad_request("registry_id_invalid", message)
        }
        RegistryError::VersionInvalid => HandlerError::bad_request(
            "registry_version_invalid",
            "version must be 1..=64 characters",
        ),
        RegistryError::KindInvalid => HandlerError::bad_request(
            "registry_kind_invalid",
            "kind must be deterministic-rules, learned, or reranker",
        ),
        RegistryError::AlreadyRegistered { id, version } => HandlerError::conflict_with(
            "model_already_registered",
            "the model identity is already registered",
            serde_json::json!({ "id": id, "version": version }),
        ),
        RegistryError::PayloadInvalid(message) => {
            HandlerError::bad_request("registry_payload_invalid", message)
        }
        RegistryError::RowAbsent => HandlerError::bad_request(
            "registry_row_absent",
            "the referenced model registry row does not exist",
        ),
        RegistryError::RowDigestMismatch => HandlerError::bad_request(
            "registry_row_digest_mismatch",
            "the proposed row bytes do not digest to the live registry row",
        ),
        RegistryError::RowChanged => HandlerError::conflict_with(
            "registry_row_changed",
            "the registry row changed after the proposal was displayed",
            serde_json::json!([]),
        ),
        RegistryError::TransitionIllegal { action, from } => HandlerError::conflict_with(
            "registry_transition_illegal",
            "the requested lifecycle transition is not legal",
            serde_json::json!({ "action": action, "from": from }),
        ),
        RegistryError::Audit(message) => HandlerError::internal(message),
        RegistryError::Db(message) => HandlerError::internal(message.to_string()),
    }
}

fn validate_listing_limit(limit: Option<usize>) -> Result<usize, HandlerError> {
    let limit = limit.unwrap_or(MODEL_REGISTRY_LISTING_LIMIT_DEFAULT);
    if (1..=MODEL_REGISTRY_LISTING_LIMIT_CAP).contains(&limit) {
        Ok(limit)
    } else {
        Err(HandlerError::bad_request(
            "model_registry_limit_out_of_bounds",
            format!("limit must land inside 1..={MODEL_REGISTRY_LISTING_LIMIT_CAP}"),
        ))
    }
}

fn validate_status_filter(status: Option<&str>) -> Result<Option<String>, HandlerError> {
    let Some(status) = status else {
        return Ok(None);
    };
    if matches!(
        status,
        registry::STATUS_CANDIDATE
            | registry::STATUS_EVALUATED
            | registry::STATUS_PROMOTED
            | registry::STATUS_RETIRED
    ) {
        Ok(Some(status.to_string()))
    } else {
        Err(HandlerError::bad_request(
            "registry_status_invalid",
            "status must be candidate, evaluated, promoted, or retired",
        ))
    }
}

fn validate_kind_filter(kind: Option<&str>) -> Result<Option<String>, HandlerError> {
    let Some(kind) = kind else { return Ok(None) };
    if matches!(
        kind,
        KIND_DETERMINISTIC_RULES | KIND_LEARNED | KIND_RERANKER
    ) {
        Ok(Some(kind.to_string()))
    } else {
        Err(HandlerError::bad_request(
            "registry_kind_invalid",
            "kind must be deterministic-rules, learned, or reranker",
        ))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterModelBody {
    pub kind: String,
    #[serde(default)]
    pub rules_config: Option<serde_json::Value>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub output_vocabulary: Option<Vec<String>>,
    #[serde(default)]
    pub artifact_digest: Option<String>,
    #[serde(default)]
    pub config_digest: Option<String>,
    #[serde(default)]
    pub calibration_ref: Option<String>,
}

fn any_identity_field(body: &RegisterModelBody) -> bool {
    body.id.is_some()
        || body.version.is_some()
        || body.name.is_some()
        || body.output_vocabulary.is_some()
        || body.artifact_digest.is_some()
        || body.config_digest.is_some()
        || body.calibration_ref.is_some()
}

fn registration_from_body(body: RegisterModelBody) -> Result<Registration, HandlerError> {
    match body.kind.as_str() {
        KIND_DETERMINISTIC_RULES => {
            if any_identity_field(&body) {
                return Err(registry_error(RegistryError::IdentityDeclared));
            }
            let Some(rules_config) = body.rules_config else {
                return Err(registry_error(RegistryError::RulesConfig(
                    "rules_config is required for deterministic-rules".into(),
                )));
            };
            if !rules_config.is_object() {
                return Err(registry_error(RegistryError::RulesConfig(
                    "rules_config must be a JSON object".into(),
                )));
            }
            let rules_json = serde_json::to_string(&rules_config)
                .map_err(|error| HandlerError::internal(error.to_string()))?;
            Ok(Registration::DeterministicRules { rules_json })
        }
        KIND_LEARNED | KIND_RERANKER => {
            if body.rules_config.is_some() {
                return Err(HandlerError::bad_request(
                    "registry_payload_invalid",
                    "rules_config is only valid for deterministic-rules",
                ));
            }
            let kind = if body.kind == KIND_LEARNED {
                KIND_LEARNED
            } else {
                KIND_RERANKER
            };
            let (Some(id), Some(version), Some(name), Some(output_vocabulary)) =
                (body.id, body.version, body.name, body.output_vocabulary)
            else {
                return Err(registry_error(RegistryError::IdentityRequired));
            };
            Ok(Registration::Declared {
                kind,
                id,
                version,
                name,
                output_vocabulary,
                artifact_digest: body.artifact_digest,
                config_digest: body.config_digest,
                calibration_ref: body.calibration_ref,
            })
        }
        _ => Err(registry_error(RegistryError::KindInvalid)),
    }
}

fn parse_model_ref(raw: &str) -> Result<(String, String), HandlerError> {
    let invalid = || {
        HandlerError::bad_request(
            "model_ref_invalid",
            "model_ref must be one whole path segment shaped id@version",
        )
    };
    let (id, version) = raw.split_once('@').ok_or_else(invalid)?;
    if id.is_empty()
        || version.is_empty()
        || id.contains('@')
        || version.contains('@')
        || id
            .chars()
            .any(|c| c.is_control() || crate::strip_invisible::is_invisible(c))
        || version
            .chars()
            .any(|c| c.is_control() || crate::strip_invisible::is_invisible(c))
        || id.len() > 256
        || version.len() > 64
    {
        return Err(invalid());
    }
    Ok((id.to_string(), version.to_string()))
}

/// Register a model identity. Deterministic rules derive identity and digest
/// from the in-body document; declared learned/reranker identities carry their
/// own bounded digest references. The row always lands as `candidate`.
pub async fn post_register_model(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<RegisterModelBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), HandlerError> {
    let principal = principal.0;
    super::authorize(&principal, crate::auth::Action::Admin, "", "global")?;
    let registration = registration_from_body(body)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let actor = super::recall::principal_label(&principal);
    let now = chrono::Utc::now().timestamp();
    let row = tokio::task::spawn_blocking(move || {
        let mut conn = pool.get().map_err(HandlerError::db_down)?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|error| HandlerError::internal(error.to_string()))?;
        let row = registry::register_model(&mut tx, &registration, &actor, now)
            .map_err(registry_error)?;
        tx.commit()
            .map_err(|error| HandlerError::internal(error.to_string()))?;
        Ok::<_, HandlerError>(row)
    })
    .await
    .map_err(|error| HandlerError::internal(format!("{error}")))??;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": row.id,
            "version": row.version,
            "status": row.status,
            "config_digest": row.config_digest,
        })),
    ))
}

/// List the bounded registry projection. The DPO dual gate and audit row are
/// applied before the page is emitted; artifact digest values never ride here.
pub async fn get_model_registry(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(query): Query<ModelRegistryQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Admin, "", "global")?;
    crate::handlers::breaches::require_dpo_role(&principal, &pool)?;
    let limit = validate_listing_limit(query.limit)?;
    let status = validate_status_filter(query.status.as_deref())?;
    let kind = validate_kind_filter(query.kind.as_deref())?;
    let who = super::recall::principal_label(&principal);
    let rows = tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        let rows = registry::list_rows(&conn, status.as_deref(), kind.as_deref(), limit)
            .map_err(|error| HandlerError::internal(error.to_string()))?;
        crate::audit::record_tenant_checked(
            &conn,
            crate::audit::AuditKind::Workflow,
            crate::workflow::ACTOR,
            "model_registry:listing",
            crate::audit::AuditStatus::Ok,
            &format!(
                "principal={who} status={} kind={} count={}",
                status.as_deref().unwrap_or("all"),
                kind.as_deref().unwrap_or("all"),
                rows.len()
            ),
            "global",
        )
        .map_err(|error| HandlerError::internal(error.to_string()))?;
        Ok::<_, HandlerError>(rows)
    })
    .await
    .map_err(|error| HandlerError::internal(format!("{error}")))??;
    let rows: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "id": row.id,
                "version": row.version,
                "kind": row.kind,
                "name": row.name,
                "output_vocabulary": row.output_vocabulary,
                "artifact_digest_present": row.artifact_digest_present,
                "config_digest": row.config_digest,
                "status": row.status,
                "proposed_by": row.proposed_by,
                "created_at": row.created_at,
                "updated_at": row.updated_at,
            })
        })
        .collect();
    let count = rows.len();
    let mut response = serde_json::json!({ "rows": rows, "count": count });
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct ModelRegistryQuery {
    pub limit: Option<usize>,
    pub status: Option<String>,
    pub kind: Option<String>,
}

/// Read one identity by its exact `id@version` citation. Authorization happens
/// before lookup, and absence is a probe-blind not-found response.
pub async fn get_model(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(model_ref): Path<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    super::authorize(&principal, crate::auth::Action::Read, "", "global")?;
    let (id, version) = parse_model_ref(&model_ref)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let who = super::recall::principal_label(&principal);
    let row = tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        let row = registry::row_by_ref(&conn, &id, &version).map_err(registry_error)?;
        if row.is_some() {
            crate::audit::record_tenant_checked(
                &conn,
                crate::audit::AuditKind::Workflow,
                crate::workflow::ACTOR,
                &format!("registry:{id}@{version}"),
                crate::audit::AuditStatus::Ok,
                &format!("principal={who} read=model_registry"),
                "global",
            )
            .map_err(|error| HandlerError::internal(error.to_string()))?;
        }
        Ok::<_, HandlerError>(row)
    })
    .await
    .map_err(|error| HandlerError::internal(format!("{error}")))??;
    let row = row.ok_or_else(|| HandlerError::not_found("model registry row not found"))?;
    let mut response =
        serde_json::to_value(row).map_err(|error| HandlerError::internal(error.to_string()))?;
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::registry::PROP_KIND_REGISTRY_LIFECYCLE;
    use axum::body::Body;
    use tower::ServiceExt;

    const OP_TOKEN: &str = "model-registry-operator";
    const RULES: &str = r#"{
      "model_id": "rules-reference",
      "model_version": "1.0.0",
      "rules": [
        { "question_id": "needs_human", "min_evidence": 1, "min_tier": "vetted",
          "output": { "Choice": { "options": ["act", "reject"], "label": "act" } } }
      ]
    }"#;

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

    async fn request_json(
        state: &Arc<AppState>,
        method: &str,
        uri: &str,
        body: String,
        bearer: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(token) = bearer {
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

    async fn post_json(
        state: &Arc<AppState>,
        uri: &str,
        body: String,
        bearer: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        request_json(state, "POST", uri, body, bearer).await
    }

    async fn get_json(
        state: &Arc<AppState>,
        uri: &str,
        bearer: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        request_json(state, "GET", uri, String::new(), bearer).await
    }

    fn rules_value() -> serde_json::Value {
        serde_json::from_str(RULES).unwrap()
    }

    async fn register_reference(state: &Arc<AppState>) -> serde_json::Value {
        let body = serde_json::json!({
            "kind": KIND_DETERMINISTIC_RULES,
            "rules_config": rules_value(),
        });
        let (status, response) = post_json(
            state,
            "/workflow/model-registry/register",
            body.to_string(),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "body: {response}");
        response
    }

    async fn promote_reference(state: &Arc<AppState>) -> (String, String) {
        let registered = register_reference(state).await;
        let id = registered["id"].as_str().unwrap().to_string();
        let version = registered["version"].as_str().unwrap().to_string();
        let row = {
            let conn = state.pool.get().unwrap();
            registry::row_by_ref(&conn, &id, &version).unwrap().unwrap()
        };
        let digest = registry::row_digest(&row).unwrap();
        let payload = serde_json::json!({
            "action": "promote",
            "id": id,
            "version": version,
            "row_digest": digest,
            "row": row,
        });
        let payload_text = payload.to_string();
        let proposal_body = serde_json::json!({
            "content": payload_text,
            "kind": PROP_KIND_REGISTRY_LIFECYCLE,
        });
        let (status, proposal) = post_json(
            state,
            "/ingest/proposal",
            proposal_body.to_string(),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "proposal body: {proposal}");
        let proposal_id = proposal["id"].as_i64().unwrap();
        let want = crate::handlers::gate::review_digest(&payload_text);
        let (status, approved) = post_json(
            state,
            &format!("/proposals/{proposal_id}/approve?digest={want}"),
            String::new(),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "approval body: {approved}");
        assert_eq!(approved["status"], "approved");
        (id, version)
    }

    #[tokio::test]
    async fn registry_promotion_requires_gate_approval_no_direct_status_route() {
        let source = include_str!("../server/router/workflow.rs");
        assert_eq!(source.matches("\"/workflow/model-registry").count(), 3);
        assert!(!source.contains("/workflow/model-registry/{model_ref}/status"));

        let f = fixture();
        let (id, version) = promote_reference(&f.state).await;
        let path = format!("/workflow/model-registry/{id}@{version}");
        for method in ["POST", "PUT"] {
            let (status, _) =
                request_json(&f.state, method, &path, String::new(), Some(OP_TOKEN)).await;
            assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
        }
        let conn = f.state.pool.get().unwrap();
        assert_eq!(
            crate::workflow::registry::test_support::registry_row_status(&conn, &id, &version)
                .as_deref(),
            Some(registry::STATUS_PROMOTED)
        );
        let (knowledge, vectors) =
            crate::workflow::state::test_support::knowledge_and_vec_counts(&conn).unwrap();
        assert_eq!(
            (knowledge, vectors),
            (0, 0),
            "lifecycle approval creates no knowledge row"
        );
    }

    #[tokio::test]
    async fn learned_model_without_artifact_digest_is_rejected() {
        let f = fixture();
        let before = {
            let conn = f.state.pool.get().unwrap();
            registry::test_support::registry_row_count(&conn)
        };
        let body = serde_json::json!({
            "kind": KIND_LEARNED,
            "id": "learned-without-digest",
            "version": "1.0.0",
            "name": "learned-without-digest",
            "output_vocabulary": ["choice"],
        });
        let (status, response) = post_json(
            &f.state,
            "/workflow/model-registry/register",
            body.to_string(),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body: {response}");
        assert_eq!(response["error"]["code"], "artifact_digest_required");
        let after = {
            let conn = f.state.pool.get().unwrap();
            registry::test_support::registry_row_count(&conn)
        };
        assert_eq!(before, after);
    }

    #[tokio::test]
    async fn registry_rows_are_audit_chained() {
        let f = fixture();
        let (id, version) = promote_reference(&f.state).await;
        let conn = f.state.pool.get().unwrap();
        let target = format!("registry:{id}@{version}");
        assert!(registry::test_support::audit_rows_for_target(&conn, &target) >= 2);
        assert!(crate::audit::verify_chain(&conn));
    }

    #[tokio::test]
    async fn model_registry_routes_are_role_gated_and_dual_gated_for_listings() {
        let f = fixture();
        let (status, _) = get_json(&f.state, "/workflow/model-registry", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = get_json(
            &f.state,
            "/workflow/model-registry/rules-reference@1.0.0",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = post_json(
            &f.state,
            "/workflow/model-registry/register",
            serde_json::json!({ "kind": KIND_DETERMINISTIC_RULES }).to_string(),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        {
            let conn = f.state.pool.get().unwrap();
            crate::workflow::state::test_support::seed_role_with_capabilities(
                &conn,
                "limited",
                &["read", "write"],
            )
            .unwrap();
            crate::workflow::state::test_support::seed_role_with_capabilities(
                &conn,
                "dpo",
                &["admin"],
            )
            .unwrap();
        }
        let limited = crate::auth::Principal {
            sub: "registry-limited".into(),
            tenant: "global".into(),
            scopes: vec![crate::auth::Scope {
                action: crate::auth::Action::Read,
                team: "*".into(),
                domain: "*".into(),
            }],
            jti: "registry-limited".into(),
            roles: vec!["limited".into()],
            manages: vec![],
            kind: crate::auth::PrincipalKind::Jwt,
        };
        assert!(
            crate::handlers::authorize(
                &Some(limited.clone()),
                crate::auth::Action::Admin,
                "",
                "global"
            )
            .is_err()
        );
        assert!(
            crate::handlers::breaches::require_dpo_role(&Some(limited.clone()), &f.state.pool)
                .is_err()
        );
        let dpo = crate::auth::Principal {
            scopes: vec![crate::auth::Scope {
                action: crate::auth::Action::Admin,
                team: "*".into(),
                domain: "*".into(),
            }],
            roles: vec!["dpo".into()],
            ..limited
        };
        assert!(crate::handlers::breaches::require_dpo_role(&Some(dpo), &f.state.pool).is_ok());
    }

    #[tokio::test]
    async fn evaluation_refs_must_resolve_to_signed_records() {
        let f = fixture();
        let registered = register_reference(&f.state).await;
        let id = registered["id"].as_str().unwrap();
        let version = registered["version"].as_str().unwrap();
        let mut row = {
            let conn = f.state.pool.get().unwrap();
            registry::row_by_ref(&conn, id, version).unwrap().unwrap()
        };
        row.evaluation_refs = vec!["unsigned-record".into()];
        let digest = registry::row_digest(&row).unwrap();
        let payload = serde_json::json!({
            "action": "promote",
            "id": id,
            "version": version,
            "row_digest": digest,
            "row": row,
        });
        let (status, response) = post_json(
            &f.state,
            "/ingest/proposal",
            serde_json::json!({
                "content": payload.to_string(),
                "kind": PROP_KIND_REGISTRY_LIFECYCLE,
            })
            .to_string(),
            Some(OP_TOKEN),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body: {response}");
        assert_eq!(response["error"]["code"], "registry_payload_invalid");

        let (status, _) = post_json(
            &f.state,
            "/workflow/model-registry/register",
            serde_json::json!({
                "kind": KIND_LEARNED,
                "id": "unknown-field",
                "version": "1.0.0",
                "name": "unknown-field",
                "output_vocabulary": ["choice"],
                "evaluation_refs": ["unsigned"],
            })
            .to_string(),
            Some(OP_TOKEN),
        )
        .await;
        assert!(matches!(
            status,
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY
        ));
    }
}
