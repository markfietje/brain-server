//! The UMP 1.0 HTTP ops binding + the SSE event buses
//! (`/ump/subscribe`, `/events`) + `/.well-known/ump.json`. Handlers:
//! `handlers::ump_ops` + `alert::events`.

use axum::{
    Router,
    routing::{get, post},
};
use std::sync::Arc;

use crate::{alert, handlers, server::bootstrap::AppState};

pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/ump/capabilities", get(handlers::ump_ops::capabilities))
        .route("/ump/remember", post(handlers::ump_ops::remember))
        .route("/ump/memory/{id}", get(handlers::ump_ops::get_memory))
        .route("/ump/recall", post(handlers::ump_ops::recall))
        .route("/ump/revise", post(handlers::ump_ops::revise))
        .route("/ump/forget", post(handlers::ump_ops::forget))
        .route("/ump/feedback", post(handlers::ump_ops::feedback))
        .route("/ump/subscribe", get(handlers::ump_ops::subscribe))
        .route("/events", get(alert::events))
        .route("/ump/audit", post(handlers::ump_ops::audit))
        .route("/ump/audit/verify", get(handlers::ump_ops::audit_verify))
        .route(
            "/.well-known/ump.json",
            get(handlers::ump_ops::capabilities),
        )
}
