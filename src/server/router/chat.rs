//! The chat router — the chat spine's own wire tables (the agentloop law:
//! a loop-serving route ships its wire in its own release).
//!
//! `POST /chat/turns` — one memory-grounded, AFM-generated chat turn as an
//! SSE stream (`grounding`, `delta`*, `final` | `error`). See
//! `handlers::chat` for the turn contract.

use axum::Router;
use std::sync::Arc;

use axum::routing::{delete, get, patch, post};

use crate::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/chat/turns", post(crate::handlers::chat::chat_turn))
        // The chat session store (the chat spine's durable store). One registration PER
        // METHOD (the /notes shape), so the authz matrix's registration
        // scan sees both methods. DELETE registers LAST: the matrix pairs a
        // path's first table row (Read, DELETE's own gate) with the
        // last-registered method.
        .route(
            "/chat/sessions",
            get(crate::handlers::chat_sessions::list_sessions),
        )
        .route(
            "/chat/sessions/{session}/turns",
            get(crate::handlers::chat_sessions::list_turns),
        )
        .route(
            "/chat/sessions/{session}",
            patch(crate::handlers::chat_sessions::rename_session),
        )
        .route(
            "/chat/sessions/{session}",
            delete(crate::handlers::chat_sessions::delete_session),
        )
}
