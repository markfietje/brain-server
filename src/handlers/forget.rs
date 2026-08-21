//! `DELETE /memory/{id}` — forget a knowledge entry. Per `API_CONTRACT.md`
//! §4. Cascades to the embedding + owned relations (FK CASCADE / SET NULL).

use axum::extract::{Path, State};
use axum::response::Json;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::auth::OptPrincipal;
use crate::handlers::{ForgetResponse, HandlerError};

/// `DELETE /memory/{id}`
pub async fn forget(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<ForgetResponse>, HandlerError> {
    // AuthZ admin gate — DELETE is Admin (destructive operator action); `None` = superuser.
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;

    let pool = state.pool.clone();

    let deleted = tokio::task::spawn_blocking(move || -> Result<bool, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(format!("transaction failed: {e}")))?;

        // A held id is frozen against every erasure; refuse in-transaction so
        // `/purge`'s 409 is the single hold-fence envelope.
        crate::legal_hold::refuse_if_held(&tx, &[id])?;

        // Capture document_id + content digest for the tombstone (the registry
        // must carry the same SHA-256 evidence as every erasure path).
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

        // vec_knowledge is a vec0 table with no FK (no cascade) — delete explicitly.
        tx.execute(
            "DELETE FROM vec_knowledge WHERE knowledge_id = ?1",
            rusqlite::params![id],
        )
        .map_err(|e| HandlerError::internal(format!("vec0 delete failed: {e}")))?;

        // Deleting the row cascades to embeddings, SET NULLs relationships,
        // and the FTS trigger removes the FTS row.
        let rows = tx
            .execute("DELETE FROM knowledge WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| HandlerError::internal(format!("delete failed: {e}")))?;

        if rows > 0 {
            // Tombstone for provenance (content gone; SHA-256 digest survives).
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
