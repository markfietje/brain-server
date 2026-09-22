//! The disagreement-corpus export surface: the DPO's read window over the
//! after-action reflection rows the close seam records. Protocol adapter
//! only — every read lives in the domain core
//! ([`crate::workflow::reflection`], which owns the row shapes, the
//! bounds, the frozen split, and the read-seam sanitizer), and this file
//! carries no statements of its own (the no-SQL-in-handlers law).
//!
//! Gate posture: the DPO dual gate exactly like the scoreboard — the
//! Admin scope AND the DPO role — because an export is the round's
//! exfiltration surface. Every export lands a global audit row naming the
//! principal, the filter set, and the row count.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use std::sync::Arc;

use super::HandlerError;
use crate::AppState;

/// The bounded page: `limit` must land inside 1..=500 (named 400 outside),
/// `partition` is `all | train | holdout` (named 400 otherwise), `since`
/// filters on the recording timestamp.
#[derive(Debug, Deserialize)]
pub struct CorpusQuery {
    pub since: Option<i64>,
    pub limit: Option<usize>,
    pub partition: Option<String>,
}

fn validate_limit(limit: Option<usize>) -> Result<usize, HandlerError> {
    let limit = limit.unwrap_or(100);
    if (1..=500).contains(&limit) {
        return Ok(limit);
    }
    Err(HandlerError::internal_with(
        "reflection_limit_out_of_bounds",
        "limit must land inside 1..=500 — the corpus page is bounded",
        StatusCode::BAD_REQUEST,
    ))
}

fn validate_partition(
    raw: Option<&str>,
) -> Result<Option<crate::workflow::reflection::Partition>, HandlerError> {
    match raw {
        None | Some("all") => Ok(None),
        Some(p) => crate::workflow::reflection::Partition::parse(p).map_or_else(
            || {
                Err(HandlerError::internal_with(
                    "reflection_partition_unknown",
                    "partition must be one of all | train | holdout",
                    StatusCode::BAD_REQUEST,
                ))
            },
            |part| Ok(Some(part)),
        ),
    }
}

/// `GET /workflow/reflection/corpus?since=&limit=&partition=` — the
/// de-identified, frozen-split corpus page. DPO/admin dual gate; audited;
/// rows carry their partition so a train/holdout bleed is checkable.
pub async fn get_reflection_corpus(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Query(q): Query<CorpusQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Admin, "", "global")?;
    crate::handlers::breaches::require_dpo_role(&principal, &pool)?;
    let limit = validate_limit(q.limit)?;
    let partition = validate_partition(q.partition.as_deref())?;
    let since = q.since;
    let who = principal
        .as_ref()
        .map(|p| p.sub.clone())
        .unwrap_or_else(|| "loopback".into());
    let entries = tokio::task::spawn_blocking(move || -> Result<Vec<_>, String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        let entries =
            crate::workflow::reflection::reflection_corpus(&conn, since, limit, partition)
                .map_err(|e| format!("{e}"))?;
        // EVERY export lands an audit row: who, the filter set, the
        // row count. The corpus read and its audit share one
        // connection so the count describes exactly the emitted page.
        let detail = format!(
            "principal={who} since={since:?} limit={limit} partition={} rows={}",
            partition
                .map(crate::workflow::reflection::Partition::as_str)
                .unwrap_or("all"),
            entries.len()
        );
        crate::audit::record_tenant(
            &conn,
            crate::audit::AuditKind::Workflow,
            crate::workflow::ACTOR,
            "reflection_corpus_export",
            crate::audit::AuditStatus::Ok,
            &detail,
            "global",
        );
        Ok(entries)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?;
    let rows: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "run_id": e.run_id,
                "seq": e.seq,
                "partition": e.partition.as_str(),
                "record": e.record,
                "disagreements": e.disagreements,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "rows": rows })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pure validation only: the bounds and the partition vocabulary.
    /// The SQL-bearing laws live in tests/main_suite.rs (the no-SQL law
    /// counts statements in this file, assertions included).
    #[test]
    fn corpus_limit_bounds_are_pinned() {
        assert!(validate_limit(None).is_ok());
        assert_eq!(validate_limit(None).unwrap(), 100);
        assert!(validate_limit(Some(1)).is_ok());
        assert!(validate_limit(Some(500)).is_ok());
        assert!(validate_limit(Some(0)).is_err());
        assert!(validate_limit(Some(501)).is_err());
        assert!(validate_limit(Some(100_000)).is_err());
    }

    #[test]
    fn corpus_partition_vocabulary_is_closed() {
        assert!(validate_partition(None).unwrap().is_none());
        assert!(validate_partition(Some("all")).unwrap().is_none());
        assert!(validate_partition(Some("train")).unwrap().is_some());
        assert!(validate_partition(Some("holdout")).unwrap().is_some());
        assert!(validate_partition(Some("everything")).is_err());
        assert!(validate_partition(Some("")).is_err());
    }
}
