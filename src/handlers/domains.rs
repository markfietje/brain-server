//! `GET /domains` — registry listing (ops/debug + the `brain` CLI).
//!
//! Per `API_CONTRACT.md` §4. Not on the recall hot path, but useful for
//! surfacing `knownDomains` in `domain_unknown` errors and for admin tooling.
//!
//! Implementation status:
//!   - Response serde ✅
//!   - Counts: ✅ against the legacy single-DB (treated as `global`) until
//!     v1.0.0 splits per-domain files; v1.0.0 returns the real per-domain
//!     entries/entities/relations and `hasCentroid`.

use axum::extract::State;
use axum::response::Json;
use serde::Serialize;
use std::sync::Arc;

use crate::handlers::HandlerError;
use crate::AppState;

#[derive(Debug, Serialize)]
pub struct DomainInfo {
    pub name: String,
    pub entries: i64,
    pub entities: i64,
    pub relations: i64,
    pub has_centroid: bool,
}

#[derive(Debug, Serialize)]
pub struct DomainsResponse {
    pub domains: Vec<DomainInfo>,
}

/// `GET /domains`
pub async fn domains(
    State(state): State<Arc<AppState>>,
) -> Result<Json<DomainsResponse>, HandlerError> {
    let pool = state.pool.clone();

    let info = tokio::task::spawn_blocking(move || -> Result<DomainsResponse, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;

        // Real per-domain listing from the `domain` column (single-DB tagged
        // model). has_centroid is false until the per-domain centroid layer
        // (computed mean vectors) ships; today routing is by explicit domain.
        let mut stmt = conn
            .prepare("SELECT domain, COUNT(*) FROM knowledge GROUP BY domain ORDER BY domain")
            .map_err(|e| HandlerError::internal(format!("prepare domains failed: {e}")))?;
        let rows: Vec<(String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| HandlerError::internal(format!("query domains failed: {e}")))?
            .filter_map(|r| r.ok())
            .collect();

        // Entity/relation totals (domain-scoped KG lands with per-domain DBs).
        let entities: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
            .unwrap_or(0);
        let relations: i64 = conn
            .query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))
            .unwrap_or(0);

        let domains = rows
            .into_iter()
            .map(|(name, entries)| DomainInfo {
                name,
                entries,
                entities,
                relations,
                has_centroid: false,
            })
            .collect();

        Ok(DomainsResponse { domains })
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(info))
}
