//! The compliance family: the feature-gated compliance-pack evidence
//! router (merges EMPTY without the feature — the routes do not exist on
//! the wire at all) plus the always-on governance reads (retention, the
//! Art 30 register, snapshot status) and the DSAR/trace surfaces.

use axum::{
    Router,
    routing::{get, post},
};
use std::sync::Arc;

use crate::handlers;
use crate::server::bootstrap::AppState;

/// The always-on governance + DSAR + trace routes.
pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        // per-kind retention policy, the Art 30
        // records-of-processing register, and the snapshot self-check
        // panel. GET /retention reads; POST /retention overrides
        // (Admin + audited); /art30 and /snapshot/status are Admin read-only.
        .route("/retention", get(handlers::govern::retention_get))
        .route("/retention", post(handlers::govern::retention_post))
        .route("/retention/report", get(handlers::govern::retention_report))
        .route("/art30", get(handlers::govern::art30))
        .route("/snapshot/status", get(handlers::govern::snapshot_status))
        // read-event trace + DSAR workflow. `/recall/{id}/
        // trace` replays a recorded recall decision path; `/dsar` is the GDPR
        // Art 15/17 workflow (locate → export → purge → certificate);
        // `/tombstones` is the queryable deletion registry; `/dsar/{id}/
        // certificate` re-fetches a past deletion certificate.
        .route(
            "/recall/{trace_id}/trace",
            get(handlers::observe::get_trace),
        )
        .route("/dsar", post(handlers::observe::post_dsar))
        // the DSAR ledger list (Admin) — past requests
        // + the Art 17 window the client countdown renders.
        .route("/dsar", get(handlers::observe::list_dsar))
        .route("/tombstones", get(handlers::observe::list_tombstones))
        .route(
            "/dsar/{id}/certificate",
            get(handlers::observe::get_dsar_certificate),
        )
}

/// The feature-gated compliance-pack evidence router: merges EMPTY without
/// the feature — the routes do not exist on the wire at all.
pub(crate) fn pack_router() -> Router<Arc<AppState>> {
    {
        #[cfg(feature = "compliance-pack")]
        {
            Router::new()
                .route("/audit/export", get(handlers::compliance::export_audit))
                .route(
                    "/compliance/evaluation-record",
                    post(handlers::compliance::post_evaluation_record),
                )
                .route(
                    "/compliance/inventory",
                    get(handlers::compliance::inventory),
                )
                .route(
                    "/ropa",
                    get(handlers::compliance::list_ropa).post(handlers::compliance::create_ropa),
                )
                .route("/ropa/{id}", post(handlers::compliance::upsert_ropa))
        }
        #[cfg(not(feature = "compliance-pack"))]
        {
            Router::new()
        }
    }
}
