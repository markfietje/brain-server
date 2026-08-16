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
    // v1.12.1 "Harden": AuthZ admin gate — the v1.2 matrix puts DELETE
    // /memory/{id} on the Admin surface (destructive operator action).
    // `None` (no JWT) = superuser.
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    // `id` is extracted as i64 by axum; non-numeric already → 400 via extractor.

    let pool = state.pool.clone();

    let deleted = tokio::task::spawn_blocking(move || -> Result<bool, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(format!("transaction failed: {e}")))?;

        // v1.28.1 "Holdall" M1 (F-02): a held id is frozen against EVERY erasure
        // path. Refuse this one in-transaction so `/purge`'s 409 shape is the
        // single hold-fence envelope.
        crate::legal_hold::refuse_if_held(&tx, &[id])?;

        // Capture the document_id + content digest for the tombstone before
        // deleting (v1.28.1 M1.3: the deletion registry must carry the same
        // SHA-256 evidence every other erasure path records).
        let doc_id: Option<String> = tx
            .query_row(
                "SELECT document_id FROM knowledge WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        let content_digest: Option<String> = tx
            .query_row(
                "SELECT content FROM knowledge WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .map(|c| crate::handlers::gate::sha256_hex(&c));

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
            // Record a tombstone for provenance (content is already gone;
            // the SHA-256 digest survives so the registry has the same
            // deletion evidence as `/purge`).
            tx.execute(
                "INSERT INTO tombstones (knowledge_id, document_id, content_hash)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![id, doc_id, content_digest],
            )
            .map_err(|e| HandlerError::internal(format!("tombstone failed: {e}")))?;
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
