//! v0.9.6 Bridge — connector registry HTTP handlers.
//!
//! `GET /connectors` lists every registered connector instance across all
//! kinds. The handler is a thin read of `crate::connector::list_connectors`;
//! all DB logic stays in the connector module (mirrors `handlers/sources.rs`'s
//! split from `sources.rs`).

use axum::extract::State;
use axum::response::Json;
use serde::Serialize;
use std::sync::Arc;

use crate::connector::ConnectorRow;
use crate::handlers::auth::OptPrincipal;
use crate::handlers::HandlerError;
use crate::AppState;

#[derive(Debug, Serialize)]
pub struct ListConnectorsResponse {
    pub connectors: Vec<ConnectorRow>,
}

/// `GET /connectors` — list every registered connector instance, ordered by
/// `(kind, instance)`. Empty list if none registered.
pub async fn list(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<ListConnectorsResponse>, HandlerError> {
    // v1.12.1 "Harden": AuthZ read gate. `None` (no JWT) = superuser.
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
