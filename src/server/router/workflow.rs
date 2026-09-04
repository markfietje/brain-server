//! The workflow family: runs + events + lineage + steering, the valet
//! cranks, the complaint lifecycle, outreach, plugins, KCS, shifts/crew,
//! workload, the breach/transfer/client registers, webhooks, parcels,
//! mesh cards/delegations, and the ops boards. All handlers live in
//! `src/handlers/*` + `src/service/*` — this file only registers.

use axum::{
    Router,
    routing::{delete, get, post, put},
};
use std::sync::Arc;

use crate::handlers;
use crate::server::bootstrap::AppState;

pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/breach", post(handlers::breaches::post_breach))
        .route(
            "/breach/{id}/event",
            post(handlers::breaches::post_breach_event),
        )
        .route("/breach/{id}/close", post(handlers::breaches::close_breach))
        .route("/breaches", get(handlers::breaches::list_breaches))
        .route("/breaches/{id}", get(handlers::breaches::get_breach))
        // the cross-border transfer register +
        // the TIA/DPA evidence artifacts. Writes are Admin + audited; the
        // register + templates are the Art 30/46 + Schrems II evidence a
        // client's regulator asks for (a human DPO/legal reviews + signs them).
        .route("/transfers", post(handlers::transfers::register_transfer))
        .route("/transfers", get(handlers::transfers::list_transfers))
        .route("/transfers/{id}/tia", get(handlers::transfers::get_tia))
        .route("/transfers/{id}/dpa", get(handlers::transfers::get_dpa))
        // the BPO operating register — the spine every later
        // BPO release (onboard/dpa/dsar/holds/termination) reads. Writes are
        // Admin + audited (AuditKind::Client); the identity/evidence surface
        // only (no enforcement gate).
        .route("/clients", post(handlers::clients::register_client))
        .route("/clients", get(handlers::clients::list_clients))
        .route("/clients/{name}", get(handlers::clients::get_client))
        .route(
            "/clients/{name}/dpa",
            post(handlers::clients::set_client_dpa),
        )
        .route(
            "/clients/{name}/dpa",
            get(handlers::clients::get_client_dpa),
        )
        .route("/clients/{name}/dsar", post(handlers::clients::client_dsar))
        .route("/clients/{name}/hold", post(handlers::clients::client_hold))
        .route("/clients/{name}/end", post(handlers::clients::client_end))
        // the supervisor QA surface — owner-scoped queue
        // list + audited coaching (read + write are Admin like every client op).
        .route(
            "/clients/{name}/proposals",
            get(handlers::clients::client_proposals),
        )
        .route(
            "/clients/{name}/proposals/{id}/coach",
            post(handlers::clients::coach_proposal),
        )
        // source lifecycle. `reconcile` retires active sources
        // of a kind whose URI is no longer in the live set (a vault delete or
        // rename); `delete /sources/{id}` retires a single source explicitly.
        .route("/sources/reconcile", post(handlers::sources::reconcile))
        .route("/sources/{id}", delete(handlers::sources::delete_source))
        // connector registry. `GET /connectors` lists every
        // registered connector instance across all kinds.
        .route("/connectors", get(handlers::connectors::list))
        // register a connector instance, gated by the
        // domain's bound profile `connectors_allowed` (Admin, audited).
        .route("/connectors/register", post(handlers::connectors::register))
        // deterministic span verification. Given a
        // claim + chunk_id, returns whether the claim is supported by the
        // chunk's text. Pure lexical match — no embeddings, no LLM.
        .route("/verify", post(handlers::verify::verify))
        // opt-in, non-interrupting anticipation. `/suggest`
        // is an explicit pull (caller asks "what else might be relevant?");
        // `/suggest/feedback` records accept/dismiss; `/suggest/metrics` is
        // the false-positive rate (roadmap exit criterion). All three are
        // gated by BRAIN_SUGGEST_ENABLED and return 501 when disabled — the
        // roadmap's "otherwise the feature is removed" kill switch.
        .route("/suggest", post(handlers::suggest::suggest))
        .route("/suggest/feedback", post(handlers::suggest::feedback))
        .route("/suggest/metrics", get(handlers::suggest::metrics))
        // procedural memory + deterministic categorization
        // + decision evaluation. `POST /procedure` ingests an ordered runbook;
        // `GET /procedure/{id}/steps` returns the ordered chain; `POST /classify`
        // categorizes text deterministically (Mem0's premium, free); `POST
        // /decision/{id}/evaluate` runs a stored decision rule against input vars.
        // All deterministic — no LLM, no cloud, no tokens.
        .route("/procedure", post(handlers::procedure::create))
        .route("/procedure/{id}/steps", get(handlers::procedure::steps))
        .route("/classify", post(handlers::procedure::classify))
        .route(
            "/decision/{id}/evaluate",
            post(handlers::procedure::evaluate),
        )
        // reviewable consolidation. `propose` is pure
        // detection (no mutation); `apply` records operator-chosen typed links.
        .route("/consolidate/propose", post(handlers::consolidate::propose))
        .route("/consolidate/apply", post(handlers::consolidate::apply))
        // reverse prior supersession resolutions. The undo
        // arm of the roadmap exit criterion ("reject or undo them without
        // retrieval regression"). Clears valid_to + removes the supersedes link.
        .route("/consolidate/undo", post(handlers::consolidate::undo))
        // write-back gate — proposals queue + human review.
        // No auto-promote: a candidate becomes memory only by explicit approval.
        .route("/ingest/proposal", post(handlers::gate::ingest_proposal))
        .route("/proposals", get(handlers::gate::list_proposals))
        .route(
            "/proposals/{id}/approve",
            post(handlers::gate::approve_proposal),
        )
        .route(
            "/proposals/{id}/reject",
            post(handlers::gate::reject_proposal),
        )
        .route("/proposals/{id}/edit", post(handlers::gate::edit_proposal))
        // decay + GDPR lifecycle. `/export` is portable JSON
        // (interchange); `/purge` is hard, explicit, audited deletion; `/decayed`
        // is the operator review list. Nothing is deleted autonomously.
        .route("/decayed", get(handlers::gate::list_decayed))
        .route("/export", get(handlers::gate::export))
        .route("/purge", post(handlers::gate::purge))
        // per-kind retention policy, the Art 30
        // records-of-processing register, and the snapshot self-check
        // panel. GET /retention reads; POST /retention overrides
        // (Admin + audited); /art30 and /snapshot/status are Admin read-only.
        // verified webhook ingestion. The handler only verifies
        // the HMAC + enqueues; the drain worker (spawned in main) does the rest.
        .route("/webhooks/{kind}", post(handlers::webhooks::receive))
        .route(
            "/webhooks/channel/{kind}",
            post(handlers::channel_webhook::receive_channel),
        )
        .route(
            "/webhooks/channel/{kind}/drain",
            post(handlers::channel_webhook::drain_channel),
        )
        .route(
            "/webhooks/channel/{kind}/console",
            post(handlers::channel_webhook::post_console),
        )
        // The console annex is HMAC self-authenticating like its sibling
        // channel seams: the bridge holds no bearer, ever.
        // OIDC discovery + JWKS + auth endpoints. These are
        // PUBLIC routes (no auth_middleware) except `/auth/revoke` (admin)
        // and `/auth/logout` (the
        .route("/workflow/runs/{id}", get(handlers::workflow::get_run))
        .route(
            "/workflow/runs/{id}/state",
            get(handlers::workflow::get_run_state),
        )
        .route(
            "/workflow/runs/{id}/state",
            put(handlers::workflow::put_run_state),
        )
        .route("/workflow/runs", post(handlers::workflow::post_run))
        .route(
            "/workflow/runs/{id}/events",
            get(handlers::workflow_lineage::get_run_events),
        )
        .route(
            "/workflow/runs/{id}/events",
            post(handlers::workflow::post_event),
        )
        .route(
            "/workflow/runs/{id}/rewind",
            post(handlers::workflow_lineage::post_rewind),
        )
        .route(
            "/workflow/runs/{id}/handoff",
            get(handlers::workflow_lineage::get_handoff),
        )
        .route(
            "/workflow/runs/{id}/handover/offer",
            post(handlers::relay::post_handover_offer),
        )
        .route(
            "/workflow/runs/{id}/handover/{offer_id}/accept",
            post(handlers::relay::post_handover_accept),
        )
        .route(
            "/workflow/runs/{id}/handover/{offer_id}/decline",
            post(handlers::relay::post_handover_decline),
        )
        .route(
            "/workflow/runs/{id}/notes",
            post(handlers::channel::post_notes),
        )
        .route(
            "/workflow/runs/{id}/notes",
            get(handlers::channel::get_notes),
        )
        .route(
            "/workflow/runs/{id}/notes/{invite_id}/accept",
            post(handlers::channel::post_invite_accept),
        )
        .route(
            "/workflow/channel/user-map",
            post(handlers::channel::post_user_map_proposal),
        )
        // Mesh: agents as named colleagues — signed cards + delegation.
        .route("/ops/agents/cards", post(handlers::mesh::post_card))
        .route("/ops/agents/cards", get(handlers::mesh::get_cards))
        .route(
            "/workflow/runs/{id}/delegations",
            post(handlers::mesh::post_delegation),
        )
        .route(
            "/workflow/runs/{id}/delegations",
            get(handlers::mesh::get_delegations),
        )
        .route(
            "/workflow/runs/{id}/delegations/{delegation_id}/result",
            post(handlers::mesh::post_delegation_result),
        )
        // Parcels: signed site-to-site knowledge — export, import, ledger.
        .route("/parcels/export", post(handlers::parcels::post_export))
        .route("/parcels/import", post(handlers::parcels::post_import))
        .route("/parcels", get(handlers::parcels::get_ledger))
        .route("/ops/handovers", get(handlers::relay::get_ops_handovers))
        .route(
            "/workflow/runs/{id}/context",
            get(handlers::workflow_lineage::get_run_context),
        )
        .route(
            "/workflow/runs/{id}/answer",
            post(handlers::workflow::post_answer),
        )
        .route(
            "/workflow/runs/{id}/steering",
            get(handlers::workflow::get_steering),
        )
        .route(
            "/workflow/runs/{id}/steps",
            get(handlers::workflow::list_steps),
        )
        .route(
            "/workflow/runs/{id}/steering",
            post(handlers::workflow::post_steering),
        )
        .route(
            "/workflow/runs/{id}/suggestions",
            get(handlers::workflow::get_suggestions),
        )
        // The personal assistant's cranks + views.
        // due is the cron-cranked scheduler (no daemon); brief is today's
        // derived context; consent is the one-subject Outreach-lite registry.
        .route("/workflow/valet/due", post(handlers::valet::post_due))
        .route("/workflow/valet/brief", get(handlers::valet::get_brief))
        .route("/workflow/valet/consent", put(handlers::valet::put_consent))
        .route(
            "/workflow/runs/{id}/complaint/lifecycle",
            post(handlers::workflow::post_complaint_lifecycle),
        )
        .route(
            "/workflow/runs/{id}/complaint/remedy",
            post(handlers::workflow::post_complaint_remedy),
        )
        .route(
            "/workflow/runs/{id}/complaint/adr-packet",
            get(handlers::workflow::get_complaint_adr_packet),
        )
        .route(
            "/workflow/runs/{id}/complaint/ack",
            post(handlers::workflow::post_complaint_ack),
        )
        .route(
            "/workflow/complaints/ack-sweep",
            post(handlers::workflow::post_complaint_ack_sweep),
        )
        .route(
            "/workflow/outreach/campaign",
            post(handlers::workflow::post_outreach_campaign),
        )
        .route(
            "/workflow/outreach/campaign/{id}",
            get(handlers::workflow::get_outreach_campaign),
        )
        .route(
            "/workflow/outreach/consent",
            get(handlers::workflow::get_outreach_consent),
        )
        .route(
            "/workflow/runs/{id}/outreach/followup",
            post(handlers::workflow::post_outreach_followup),
        )
        .route(
            "/workflow/runs/{id}/status-ref",
            post(handlers::workflow::post_status_ref),
        )
        .route(
            "/workflow/scoreboard",
            get(handlers::workflow::get_scoreboard),
        )
        .route(
            "/workflow/calibration/sign",
            post(handlers::workflow::post_calibration_sign),
        )
        .route(
            "/workflow/plugins/mount",
            post(handlers::workflow::post_plugin_mount),
        )
        .route(
            "/kcs/articles/{id}/approve",
            post(handlers::kcs::post_kcs_article_approve),
        )
        .route("/kcs/articles", get(handlers::kcs::get_kcs_articles))
        .route("/kcs/translate", post(handlers::kcs::post_kcs_translate))
        .route(
            "/kcs/articles/{id}/publish",
            post(handlers::kcs::post_kcs_article_publish),
        )
        .route(
            "/kcs/articles/{id}/preview",
            get(handlers::kcs::get_kcs_article_preview),
        )
        .route("/ops/shifts", get(handlers::shifts::get_ops_shifts))
        .route("/ops/shifts", post(handlers::shifts::post_ops_shift))
        .route("/ops/crew", get(handlers::crew::get_ops_crew))
        .route("/ops/skills", get(handlers::crew::get_ops_skills))
        .route("/ops/skills", post(handlers::crew::post_ops_skills))
        .route(
            "/ops/crew/config",
            post(handlers::crew::post_ops_crew_config),
        )
        // Workload visibility: lineage-only
        // reads; fatigue alerts the scheduling human, never reassigns.
        .route("/ops/workload", get(handlers::workload::get_ops_workload))
        .route("/ops/coverage", get(handlers::workload::get_ops_coverage))
}
