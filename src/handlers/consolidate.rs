//! consolidation handlers.
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

use crate::AppState;
use crate::consolidate;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;

#[derive(Debug, Serialize)]
pub struct ConsolidateProposal {
    /// Exact-duplicate chunk ids grouped by content hash.
    pub exact_duplicates: Vec<Vec<i64>>,
    /// Subject conflicts: same subject, differing content, both current.
    pub conflicts: Vec<ConflictView>,
    /// `contradicts` links that have no paired `supersedes` resolution.
    /// Each entry is `(from_chunk, to_chunk)`. Operator-actionable: a
    /// contradiction was flagged but nobody picked a winner.
    pub unresolved_contradictions: Vec<(i64, i64)>,
    /// vault sources whose backing file no longer exists on disk.
    /// Operator reviews: re-ingest if moved, or `DELETE /sources/{id}` to retire.
    pub stale_sources: Vec<StaleSourceView>,
    /// near-duplicate chunk pairs (cosine > threshold, different hash).
    /// Propose `supersedes`; operator picks the winner via `brain resolve`.
    pub near_duplicates: Vec<NearDupView>,
}

#[derive(Debug, Serialize)]
pub struct StaleSourceView {
    pub source_id: i64,
    pub uri: String,
    pub kind: String,
    pub chunk_count: i64,
}

#[derive(Debug, Serialize)]
pub struct NearDupView {
    pub chunk_a: i64,
    pub chunk_b: i64,
    pub similarity: f32,
    pub proposed: &'static str,
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
    principal: OptPrincipal,
) -> Result<Json<ConsolidateProposal>, HandlerError> {
    // AuthZ read gate (detection surface, zero mutation).
    // `None` (no JWT) = superuser.
    // Layout-dependent gate — the five detection scans are
    // corpus-wide by nature (cross-chunk comparisons). In multi-db the pool
    // IS the domain (scoped by construction); in shim mode the shared pool
    // is every tenant's corpus, so the surface requires Admin there.
    let action = if state.registry.is_multi_db() {
        crate::auth::Action::Read
    } else {
        crate::auth::Action::Admin
    };
    super::authorize(&principal.0, action, "", "global")?;
    let pool = state.pool.clone();
    let (exact_duplicates, conflicts, unresolved, stale, near_dups) =
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
            let uc = consolidate::find_unresolved_contradictions(&conn).map_err(|e| {
                HandlerError::internal(format!("find_unresolved_contradictions failed: {e}"))
            })?;
            // stale-source + near-duplicate proposals.
            let stale = consolidate::find_stale_sources(&conn)
                .map_err(|e| HandlerError::internal(format!("find_stale_sources failed: {e}")))?;
            // ponytail: near-dup KNN scan is O(n) per chunk; bounded by
            // MAX_NEAR_DUP_PAIRS so the propose endpoint stays cheap.
            let near = consolidate::find_near_duplicates(&conn, 0.95, 50)
                .map_err(|e| HandlerError::internal(format!("find_near_duplicates failed: {e}")))?;
            Ok::<_, HandlerError>((dups, cf, uc, stale, near))
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
    let stale_sources = stale
        .into_iter()
        .map(|s| StaleSourceView {
            source_id: s.source_id,
            uri: s.uri,
            kind: s.kind,
            chunk_count: s.chunk_count,
        })
        .collect();
    let near_duplicates = near_dups
        .into_iter()
        .map(|n| NearDupView {
            chunk_a: n.chunk_a,
            chunk_b: n.chunk_b,
            similarity: n.similarity,
            proposed: consolidate::LINK_SUPERSEDES,
        })
        .collect();

    Ok(Json(ConsolidateProposal {
        exact_duplicates,
        conflicts,
        unresolved_contradictions: unresolved,
        stale_sources,
        near_duplicates,
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
    principal: OptPrincipal,
    Json(req): Json<ApplyRequest>,
) -> Result<Json<ApplyResponse>, HandlerError> {
    // write gate. `None` (no JWT) = superuser.
    super::authorize(&principal.0, crate::auth::Action::Write, "", "global")?;
    let pool = state.pool.clone();
    let links = req.links;
    let now_utc = chrono::Utc::now().to_rfc3339();
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
            // `supersedes` is the only kind that expires
            // the prior fact (the mandatory Carry-forward). Other kinds
            // (contradicts/supports/references/derived_from) just record the
            // link — they don't change retrieval state. Routing on kind keeps
            // the handler generic while making supersession atomic.
            let n = if l.kind == consolidate::LINK_SUPERSEDES {
                match consolidate::resolve_supersession(&tx, l.from_chunk, l.to_chunk, &now_utc) {
                    Ok(n) => n,
                    Err(e) => {
                        rejected.push(format!(
                            "{}->{}:{} ({})",
                            l.from_chunk, l.to_chunk, l.kind, e
                        ));
                        continue;
                    }
                }
            } else {
                match consolidate::link_evidence(&tx, l.from_chunk, l.to_chunk, &l.kind) {
                    Ok(n) => n,
                    Err(e) => {
                        rejected.push(format!(
                            "{}->{}:{} ({})",
                            l.from_chunk, l.to_chunk, l.kind, e
                        ));
                        continue;
                    }
                }
            };
            recorded += n;
        }
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok::<_, HandlerError>((recorded, rejected))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(ApplyResponse { recorded, rejected }))
}

#[derive(Debug, Deserialize)]
pub struct UndoRequest {
    /// Chunk ids to un-resolve. Each was previously the 'old' (loser) chunk
    /// in a `resolve_supersession` call. Undo restores them to current recall
    /// by clearing valid_to + removing the supersedes link.
    pub old_chunks: Vec<i64>,
}

#[derive(Debug, Serialize)]
pub struct UndoResponse {
    pub undone: usize,
    pub rejected: Vec<String>,
}

/// `POST /consolidate/undo` — reverse prior supersession resolutions.
/// Roadmap exit criterion: "reject or undo them without retrieval regression."
/// For each `old_chunk`, clears `valid_to` back to NULL + removes the
/// `supersedes` evidence_link, atomically in one tx. Audited. Idempotent.
pub async fn undo(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<UndoRequest>,
) -> Result<Json<UndoResponse>, HandlerError> {
    // write gate. `None` (no JWT) = superuser.
    super::authorize(&principal.0, crate::auth::Action::Write, "", "global")?;
    let pool = state.pool.clone();
    let chunks = req.old_chunks;
    let (undone, rejected) = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let mut undone = 0usize;
        let mut rejected: Vec<String> = Vec::new();
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(format!("tx failed: {e}")))?;
        for cid in &chunks {
            match consolidate::undo_supersession(&tx, *cid) {
                Ok(n) => undone += n,
                Err(e) => rejected.push(format!("chunk {cid}: {e}")),
            }
        }
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok::<_, HandlerError>((undone, rejected))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(UndoResponse { undone, rejected }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposal_serializes_empty() {
        let p = ConsolidateProposal {
            exact_duplicates: vec![],
            conflicts: vec![],
            unresolved_contradictions: vec![],
            stale_sources: vec![],
            near_duplicates: vec![],
        };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(
            json,
            r#"{"exact_duplicates":[],"conflicts":[],"unresolved_contradictions":[],"stale_sources":[],"near_duplicates":[]}"#
        );
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
