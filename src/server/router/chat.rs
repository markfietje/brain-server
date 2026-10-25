//! The chat router — the chat spine's own wire tables (the agentloop law:
//! a loop-serving route ships its wire in its own release).
//!
//! `POST /chat/turns` — one memory-grounded, AFM-generated chat turn as an
//! SSE stream (`grounding`, `delta`*, `final` | `error`). See
//! `handlers::chat` for the turn contract.

use axum::Router;
use std::sync::Arc;

use axum::routing::post;

use crate::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/chat/turns", post(crate::handlers::chat::chat_turn))
}
