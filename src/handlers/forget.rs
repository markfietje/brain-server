//! `DELETE /memory/{id}` — forget a knowledge entry.
//!
//! Per `API_CONTRACT.md` §4. Cascades to the entry's embedding and owned
//! relations (via the existing `ON DELETE CASCADE` / `SET NULL` FKs).

use axum::extract::{Path, State};
use axum::response::Json;
use std::sync::Arc;

use crate::handlers::auth::OptPrincipal;
use crate::handlers::{ForgetResponse, HandlerError};
use crate::AppState;

/// `DELETE /memory/{id}`
pub async fn forget(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<ForgetResponse>, HandlerError> {
    // v1.2.0 M3 AuthZ: write gate at handler entry. `None` (no JWT) = superuser.
    super::authorize(&principal.0, crate::auth::Action::Write, "", "global")?;
    // `id` is extracted as i64 by axum; non-numeric already → 400 via extractor.

    let pool = state.pool.clone();

    let deleted = tokio::task::spawn_blocking(move || -> Result<bool, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(format!("transaction failed: {e}")))?;

        // Capture the document_id for the tombstone before deleting.
        let doc_id: Option<String> = tx
            .query_row(
                "SELECT document_id FROM knowledge WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .ok()
            .flatten();

        // vec_knowledge is a vec0 virtual table with NO foreign key, so it does
        // not cascade — clean it up explicitly so the index has no orphans.
        let _ = tx.execute(
            "DELETE FROM vec_knowledge WHERE knowledge_id = ?1",
            rusqlite::params![id],
        );

        // Deleting the knowledge row cascades to embeddings (FK CASCADE) and
        // relationships (FK SET NULL), and the FTS trigger removes the FTS row.
        let rows = tx
            .execute("DELETE FROM knowledge WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| HandlerError::internal(format!("delete failed: {e}")))?;

        if rows > 0 {
            // Record a tombstone for provenance (content is already gone).
            let _ = tx.execute(
                "INSERT INTO tombstones (knowledge_id, document_id) VALUES (?1, ?2)",
                rusqlite::params![id, doc_id],
            );
        }

        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok(rows > 0)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    if !deleted {
        return Err(HandlerError::not_found(format!("no memory with id {id}")));
    }

    Ok(Json(ForgetResponse { deleted: true }))
}
