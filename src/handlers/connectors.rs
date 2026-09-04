//! connector registry HTTP handlers.
//!
//! `GET /connectors` lists every registered connector instance across all
//! kinds. The handler is a thin read of `crate::connector::list_connectors`;
//! all DB logic stays in the connector module (mirrors `handlers/sources.rs`'s
//! split from `sources.rs`).

use axum::Json as JsonBody;
use axum::extract::State;
use axum::response::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;
use crate::connector::ConnectorRow;
use crate::connector::kind;
use crate::handlers::auth::OptPrincipal;
use crate::handlers::{ApiError, HandlerError};

#[derive(Debug, Serialize)]
pub struct ListConnectorsResponse {
    pub connectors: Vec<ConnectorRow>,
}

/// register (or reactivate) a connector instance for a
/// domain, gated by the domain's bound profile `connectors_allowed`. The
/// default domain is `global`. `config_json` is stored verbatim (the connector
/// reads it back); it is never logged.
#[derive(Debug, Deserialize)]
pub struct RegisterConnectorRequest {
    pub kind: String,
    pub instance: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub config_json: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisterConnectorResponse {
    pub id: i64,
    pub kind: String,
    pub instance: String,
    pub domain: String,
}

/// `POST /connectors/register` — Admin + audited. Validates the kind against
/// the shipped vocabulary, then enforces the domain's bound profile: a kind
/// its `connectors_allowed` does not grant is refused with
/// `403 connector_not_in_profile`. A domain with no bound profile is the
/// back-compat "no constraint" posture (registration allowed).
pub async fn register(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    JsonBody(req): JsonBody<RegisterConnectorRequest>,
) -> Result<Json<RegisterConnectorResponse>, HandlerError> {
    let domain = super::normalize_domain(req.domain.as_deref().unwrap_or("global"))?;

    // AuthZ. Admin for registration (a connector is a write
    // surface) — the same Admin gate sibling registrar routes use.
    super::authorize(&principal.0, crate::auth::Action::Admin, "", &domain)?;

    if !kind::is_connector_kind(&req.kind) {
        return Err(HandlerError::unprocessable(
            "connector_kind_invalid",
            format!("'{}' is not a shipped connector kind", req.kind),
        ));
    }
    let instance = req.instance.trim().to_string();
    if instance.is_empty() || instance.len() > 128 {
        return Err(HandlerError::bad_request(
            "connector_instance_invalid",
            "instance must be 1..=128 non-empty characters",
        ));
    }
    let config_json = req.config_json.unwrap_or_else(|| "{}".to_string());

    let kind = req.kind.trim().to_lowercase();
    let actor = super::recall::principal_label(&principal.0);
    let pool = state.pool.clone();

    let out =
        tokio::task::spawn_blocking(move || -> Result<RegisterConnectorResponse, HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            // Profile gate: a bound profile's `connectors_allowed` is the vertical
            // configuration lever. Unbound domain → no constraint → allowed.
            if let Some(profile) =
                crate::profile::profile_for_domain(&conn, &domain).map_err(map_err)?
                && !profile.connector_allowed(&kind)
            {
                return Err(HandlerError {
                    status: axum::http::StatusCode::FORBIDDEN,
                    inner: ApiError::new(
                        "connector_not_in_profile",
                        format!(
                            "'{}' is not permitted by the '{}' profile bound to '{}'",
                            kind, profile.name, domain
                        ),
                    ),
                });
            }
            let id = crate::connector::upsert_connector(&conn, &kind, &instance, &config_json)
                .map_err(map_err)?;
            crate::audit::record(
                &conn,
                crate::audit::AuditKind::Connector,
                &actor,
                &format!("{kind}/{instance}@{domain}"),
                crate::audit::AuditStatus::Ok,
                "connector registered",
            );
            Ok(RegisterConnectorResponse {
                id,
                kind,
                instance,
                domain,
            })
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(out))
}

fn map_err(e: impl std::fmt::Display) -> HandlerError {
    HandlerError::internal(format!("{e}"))
}

/// `GET /connectors` — list every registered connector instance, ordered by
/// `(kind, instance)`. Empty list if none registered.
pub async fn list(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<ListConnectorsResponse>, HandlerError> {
    // AuthZ read gate. `None` (no JWT) = superuser.
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let rows = crate::connector::list_connectors(&conn)
            .map_err(|e| HandlerError::internal(format!("list_connectors failed: {e}")))?;
        Ok::<_, HandlerError>(rows)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(ListConnectorsResponse { connectors: rows }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke: the response shape is what clients will see — `{"connectors":[...]}`.
    /// The full handler integration (axum router + DB pool) is exercised by
    /// `test_openapi_covers_routes` and the M2.x `brain connect` smoke test.
    #[test]
    fn test_list_connectors_response_serializes_empty() {
        let resp = ListConnectorsResponse { connectors: vec![] };
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(json, r#"{"connectors":[]}"#);
    }

    /// Sanity: `ConnectorRow` serializes with the documented fields and skips
    /// the `Option::None` fields (per `skip_serializing_if`).
    #[test]
    fn test_connector_row_serializes_documented_fields() {
        let row = ConnectorRow {
            id: 7,
            kind: "github".to_string(),
            instance: "markfietje/brain-server".to_string(),
            state: "running".to_string(),
            last_sync_at: None,
            last_error: None,
        };
        let json = serde_json::to_string(&row).unwrap();
        assert!(
            !json.contains("last_sync_at"),
            "None fields should be skipped, got: {json}"
        );
        assert!(
            !json.contains("last_error"),
            "None fields should be skipped, got: {json}"
        );
        assert!(json.contains("\"kind\":\"github\""));
        assert!(json.contains("\"state\":\"running\""));
    }
}
