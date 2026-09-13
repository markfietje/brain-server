//! The single-chunk erasure aggregate: `DELETE /memory/{id}`.
//!
//! OWNS the forget family's storage story: the document-id +
//! content-digest capture, the explicit vec0 delete (vec_knowledge is a vec0
//! table with no FK — no cascade), the knowledge row delete (whose FK
//! CASCADE/SET NULLs drop embeddings, NULL relationships, and clear the FTS
//! trigger row), and the tombstone write (content gone; SHA-256 digest +
//! document_id survive for the registry). The legal-hold FENCE stays at the
//! handler seam (refuse_if_held rides HandlerError today); this core
//! documents it as the caller's in-tx obligation, immediately before the
//! call.
//!
//! FK-children map: `knowledge` is the parent — deleting it cascades
//! `embeddings` and SET-NULLs `relationships`; `evidence_links` and
//! `case_articles` reference it by soft ref (registry outlives row). The
//! tombstone's `knowledge_id` is a soft ref BY DESIGN. Chunk-keyed residue
//! the FKs do NOT cover is deleted explicitly, mirroring the purge core
//! (`service/purge.rs`): `suggest_feedback` rows addressed by `chunk_id`
//! (a certified erasure must not leave feedback queryable against a
//! tombstoned id). DELIBERATELY RETAINED (documented, not deleted):
//! `relationships` rows whose `knowledge_id` SET-NULLs to an orphan edge
//! (graph archaeology — the edge's other endpoint still names it), and
//! `recall_traces` rows naming the chunk (read-event evidence — the audit
//! chain's replay artifact outlives the content by design, same as the
//! audit rows themselves). This aggregate has no delete path for graph
//! edges — the purge core owns that family's ordering; forget is the
//! single-chunk immediate erasure.
//!
//! The rows-affected check is the certified-silence inverse: a tombstone
//! is written ONLY when the row actually deleted (never for a row that
//! wasn't there). Error Display carries the exact pre-move message text.

use std::fmt;

use rusqlite::params;

/// A storage failure. `Display` carries the exact pre-move message; the
/// handler wraps it in `HandlerError::internal` unchanged.
#[derive(Debug)]
pub(crate) enum ForgetError {
    Database(String),
}

impl fmt::Display for ForgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ForgetError::Database(m) => f.write_str(m),
        }
    }
}

impl From<rusqlite::Error> for ForgetError {
    fn from(e: rusqlite::Error) -> Self {
        ForgetError::Database(e.to_string())
    }
}

/// The single-chunk forget, inside the CALLER'S tx.
///
/// The legal-hold fence is the CALLER'S obligation immediately before this
/// call (legal_hold::refuse_if_held rides HandlerError — a pre-existing
/// handler-coupled seam this move does not change; in-tx, so the 409
/// envelope matches `/purge`).
///
/// Order, verbatim: document_id capture → content digest → explicit vec0
/// delete → knowledge row delete (FK CASCADE/SET NULL) → chunk-keyed
/// residue delete (`suggest_feedback`, the purge mirror) → tombstone ONLY
/// when a row actually deleted → in-tx erasure audit row (audit-per-write:
/// the erasure and its evidence commit or roll back together). Returns
/// whether a row was deleted (the caller owns the 404).
pub(crate) fn forget_one(
    tx: &rusqlite::Transaction<'_>,
    id: i64,
    actor: &str,
) -> Result<bool, ForgetError> {
    // Capture document_id + content digest for the tombstone (the registry
    // must carry the same SHA-256 evidence as every erasure path).
    let doc_id: Option<String> = tx
        .query_row(
            "SELECT document_id FROM knowledge WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let content_digest: Option<String> = tx
        .query_row(
            "SELECT content FROM knowledge WHERE id = ?1",
            params![id],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .map(|c| crate::handlers::gate::sha256_hex(&c));

    // vec_knowledge is a vec0 table with no FK (no cascade) — delete explicitly.
    tx.execute(
        "DELETE FROM vec_knowledge WHERE knowledge_id = ?1",
        params![id],
    )
    .map_err(|e| ForgetError::Database(format!("vec0 delete failed: {e}")))?;

    // Deleting the row cascades to embeddings, SET NULLs relationships,
    // and the FTS trigger removes the FTS row.
    let rows = tx
        .execute("DELETE FROM knowledge WHERE id = ?1", params![id])
        .map_err(|e| ForgetError::Database(format!("delete failed: {e}")))?;

    if rows > 0 {
        // Chunk-keyed residue the FKs cannot reach (the purge mirror,
        // `service/purge.rs`): feedback addressed by chunk_id must not
        // survive against a tombstoned id.
        tx.execute(
            "DELETE FROM suggest_feedback WHERE chunk_id = ?1",
            params![id],
        )
        .map_err(|e| ForgetError::Database(format!("feedback residue delete failed: {e}")))?;
        // Tombstone for provenance (content gone; SHA-256 digest survives).
        tx.execute(
            "INSERT INTO tombstones (knowledge_id, document_id, content_hash)
             VALUES (?1, ?2, ?3)",
            params![id, doc_id, content_digest],
        )
        .map_err(|e| ForgetError::Database(format!("tombstone failed: {e}")))?;
        // The erasure's own evidence, in the SAME tx (audit-per-write —
        // the one mutation family that had none). Best-effort for the
        // write, never silent for the operator: a dropped row warns loudly.
        if crate::audit::record(
            tx,
            crate::audit::AuditKind::Forget,
            actor,
            &format!("chunk:{id}"),
            crate::audit::AuditStatus::Ok,
            "forget",
        )
        .is_none()
        {
            tracing::warn!("forget audit record dropped (chunk {id}) — evidence gap");
        }
    }

    Ok(rows > 0)
}

/// The stored content for a chunk, pre-delete (None when the row is absent).
/// The forget response correlates retained proposal
/// copies by exact content — the approve path stores the proposal's content
/// verbatim as the chunk, so identical bytes ARE the promoted-copy link.
pub(crate) fn chunk_content(tx: &rusqlite::Transaction<'_>, id: i64) -> Option<String> {
    tx.query_row(
        "SELECT content FROM knowledge WHERE id = ?1",
        params![id],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

/// Proposal rows still carrying `content` verbatim — the HITL decision
/// records that survive a chunk forget. Retention of the decision
/// record is legitimate (approval evidence); SILENCE about the surviving
/// content copy was not — the forget response now names every retained row.
///
/// Correlation is EXACT bytes (the approve path stores the proposal's
/// content verbatim as the chunk, so identical bytes ARE the
/// promoted-copy link) and BOUNDED: at most [`RETAINED_COPIES_CAP`] rows
/// materialize, with the overflow reported as `truncated` — the response
/// never grows without bound no matter how many proposals share one
/// content string. Edited/whitespace-variant copies are NOT correlated
/// (documented at the API seam, not silently missed).
pub(crate) const RETAINED_COPIES_CAP: i64 = 500;

pub(crate) fn retained_proposal_copies(
    tx: &rusqlite::Transaction<'_>,
    content: &str,
) -> Result<(Vec<(i64, String)>, bool), ForgetError> {
    let mut stmt = tx
        .prepare("SELECT id, status FROM proposals WHERE content = ?1 LIMIT ?2")
        .map_err(|e| ForgetError::Database(e.to_string()))?;
    let rows = stmt
        .query_map(params![content, RETAINED_COPIES_CAP + 1], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(|e| ForgetError::Database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ForgetError::Database(e.to_string()))?;
    let truncated = rows.len() as i64 > RETAINED_COPIES_CAP;
    let rows = rows
        .into_iter()
        .take(RETAINED_COPIES_CAP as usize)
        .collect();
    Ok((rows, truncated))
}

/// Replace the retained copies' content with the scrub marker (the
/// operator opt-in: `?scrub_proposals=1`). The decision record survives — id, status,
/// digests, timestamps — but the erased content does not. One audit row per
/// scrubbed proposal, inside the caller's tx (audit-per-write).
pub(crate) fn scrub_proposal_content(
    tx: &rusqlite::Transaction<'_>,
    ids: &[i64],
    now: i64,
    actor: &str,
) -> Result<usize, ForgetError> {
    let mut n = 0usize;
    for id in ids {
        let rows = tx
            .execute(
                "UPDATE proposals SET content = ?2 WHERE id = ?1 AND content <> ?2",
                params![
                    id,
                    format!("[content scrubbed: source chunk forgotten at {now}]")
                ],
            )
            .map_err(|e| ForgetError::Database(e.to_string()))?;
        if rows > 0 {
            crate::audit::record(
                tx,
                crate::audit::AuditKind::Forget,
                actor,
                &format!("proposal:{id}"),
                crate::audit::AuditStatus::Ok,
                "content_scrubbed_on_forget",
            );
            n += rows;
        }
    }
    Ok(n)
}
