//! v0.9.8 "Evidence" M2.3 — consolidation handlers.
//!
//! `POST /consolidate/propose` detects duplicate/conflict candidates without
//! mutating anything (reviewable). `POST /consolidate/apply` records the
//! operator-chosen typed links (never automatic). Both delegate to
//! `crate::consolidate`, keeping DB logic in the module (mirrors the
//! `handlers/connectors.rs` split).

use axum::extract::State;
use axum::response::Json;
use serde::Deserialize;
use serde::Serialize;
use std::sync::Arc;

use crate::consolidate;
use crate::handlers::HandlerError;
use crate::AppState;

#[derive(Debug, Serialize)]
pub struct ConsolidateProposal {
    /// Exact-duplicate chunk ids grouped by content hash.
    pub exact_duplicates: Vec<Vec<i64>>,
    /// Subject conflicts: same subject, differing content, both current.
    pub conflicts: Vec<ConflictView>,
}

#[derive(Debug, Serialize)]
pub struct ConflictView {
    pub from_chunk: i64,
    pub to_chunk: i64,
    pub subject: String,
    pub age_gap_secs: i64,
    pub authority_delta: f32,
    /// Suggested resolution: the newer/more-authoritative chunk supersedes the
    /// older one (a reviewable default, never applied without `--apply`).
    pub proposed: &'static str,
}

/// `POST /consolidate/propose` — pure detection, zero mutation.
pub async fn propose(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ConsolidateProposal>, HandlerError> {
    let pool = state.pool.clone();
    let (exact_duplicates, conflicts) =
        tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let dups = consolidate::find_exact_duplicates(&conn).map_err(|e| {
                HandlerError::internal(format!("find_exact_duplicates failed: {e}"))
            })?;
            let cf = consolidate::find_subject_conflicts(&conn).map_err(|e| {
                HandlerError::internal(format!("find_subject_conflicts failed: {e}"))
            })?;
            Ok::<_, HandlerError>((dups, cf))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    let conflicts = conflicts
        .into_iter()
        .map(|c| ConflictView {
            from_chunk: c.from_chunk,
            to_chunk: c.to_chunk,
            subject: c.subject,
            age_gap_secs: c.age_gap_secs,
            authority_delta: c.authority_delta,
            // The newer chunk (to_chunk) supersedes the older (from_chunk).
            proposed: consolidate::LINK_SUPERSEDES,
        })
        .collect();

    Ok(Json(ConsolidateProposal {
        exact_duplicates,
        conflicts,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ApplyRequest {
    /// Typed links to record. Each is (from_chunk, to_chunk, kind).
    pub links: Vec<ApplyLink>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ApplyLink {
    pub from_chunk: i64,
    pub to_chunk: i64,
    pub kind: String,
}

#[derive(Debug, Serialize)]
pub struct ApplyResponse {
    pub recorded: usize,
    pub rejected: Vec<String>,
}

/// `POST /consolidate/apply` — record operator-chosen links. Unknown kinds and
/// self-links are rejected per link (reported, not fatal). No content changes.
pub async fn apply(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ApplyRequest>,
) -> Result<Json<ApplyResponse>, HandlerError> {
    let pool = state.pool.clone();
    let links = req.links;
    let (recorded, rejected) = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let mut recorded = 0usize;
        let mut rejected: Vec<String> = Vec::new();
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(format!("tx failed: {e}")))?;
        for l in &links {
            match consolidate::link_evidence(&tx, l.from_chunk, l.to_chunk, &l.kind) {
                Ok(n) => recorded += n,
                Err(e) => rejected.push(format!(
                    "{}->{}:{} ({})",
                    l.from_chunk, l.to_chunk, l.kind, e
                )),
            }
        }
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok::<_, HandlerError>((recorded, rejected))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(ApplyResponse { recorded, rejected }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposal_serializes_empty() {
        let p = ConsolidateProposal {
            exact_duplicates: vec![],
            conflicts: vec![],
        };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, r#"{"exact_duplicates":[],"conflicts":[]}"#);
    }

    #[test]
    fn apply_link_serializes_roundtrip() {
        let l = ApplyLink {
            from_chunk: 2,
            to_chunk: 1,
            kind: "supersedes".into(),
        };
        let json = serde_json::to_string(&l).unwrap();
        let back: ApplyLink = serde_json::from_str(&json).unwrap();
        assert_eq!(back.from_chunk, 2);
        assert_eq!(back.kind, "supersedes");
    }
}
