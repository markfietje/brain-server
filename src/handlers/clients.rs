//! v1.27.1 "Clients" — the BPO operating register (HTTP surface).
//!
//! `POST /clients` registers an operating client (name / isolation domain /
//! jurisdiction / bound profile); `GET /clients` lists the register; `GET
//! /clients/{name}` resolves one row. Every write is Admin-gated + hash-chained
//! into the audit (`AuditKind::Client`). This is the evidence/identity register
//! only — it does not gate enforcement (that is v1.27.x + v2.x).

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

use crate::audit::{AuditKind, AuditStatus};
use crate::handlers::auth::OptPrincipal;
use crate::handlers::HandlerError;
use crate::AppState;

/// `POST /clients` body. `profile` is optional (the bound profile is an R2+
/// concern; here it is recorded verbatim when supplied).
#[derive(Debug, Deserialize)]
pub struct CreateClientRequest {
    pub name: String,
    pub domain: String,
    pub jurisdiction: String,
    #[serde(default)]
    pub profile: Option<String>,
}

/// `POST /clients` — register an operating client. Admin + audited.
pub async fn register_client(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<CreateClientRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    crate::clients::validate_new_client(&req.name, &req.domain, &req.jurisdiction)?;

    // Scaffold the client's domain before writing the row — `pool_for` creates
    // + migrates the domain DB (multi-db) or touches the shared pool (shim).
    // The optional profile bind is the v1.21 seam; `register` (via the compose
    // fn) makes the `clients` row. Composition only, no new logic.
    let st = state.clone();
    let now = chrono::Utc::now().timestamp();
    let name_for = req.name.clone();
    let domain_for = req.domain.clone();
    let jurisdiction_for = req.jurisdiction.clone();
    let profile_for = req.profile.clone();
    tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
        crate::clients::scaffold_and_register(
            &st.registry,
            &st.pool,
            &name_for,
            &domain_for,
            &jurisdiction_for,
            profile_for.as_deref(),
            now,
        )
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    if let Ok(conn) = state.pool.get() {
        crate::audit::record(
            &conn,
            AuditKind::Client,
            "api",
            &format!("client:{}", req.name.trim().to_ascii_lowercase()),
            AuditStatus::Ok,
            &format!(
                "register:{}:{}",
                req.jurisdiction.trim().to_ascii_lowercase(),
                req.domain.trim().to_ascii_lowercase()
            ),
        );
    }
    Ok(Json(serde_json::json!({ "name": req.name })))
}

/// `GET /clients` — the full register, ordered by name. Admin read.
pub async fn list_clients(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool_for = pool.clone();
    let rows =
        tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>, HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            Ok(crate::clients::list(&conn)?
                .into_iter()
                .map(|c| serde_json::to_value(c).unwrap_or_default())
                .collect())
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(serde_json::json!({ "clients": rows })))
}

/// `GET /clients/{name}` — resolve one client. Admin read; 404 when absent.
pub async fn get_client(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool_for = pool.clone();
    let value =
        tokio::task::spawn_blocking(move || -> Result<Option<serde_json::Value>, HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            Ok(crate::clients::by_name(&conn, &name)?
                .map(|c| serde_json::to_value(c).unwrap_or_default()))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(value.ok_or_else(|| {
        HandlerError::not_found("client not found")
    })?))
}

/// `POST /clients/{name}/dsar` body. `action` is the shared DSAR vocab
/// (`purge|export|both`); `dry_run` previews the footprint write-free.
#[derive(Debug, Deserialize)]
pub struct ClientDsarRequest {
    pub subject: String,
    #[serde(default = "default_dsar_action")]
    pub action: String,
    #[serde(default)]
    pub dry_run: bool,
}

fn default_dsar_action() -> String {
    "purge".to_string()
}

/// `POST /clients/{name}/dsar` — a subject erasure scoped to a single client's
/// domain, stamped with that client's jurisdiction, deadline, rights, and
/// transfer mechanism (the "erase Client Beta's data on contract end" building
/// block). Admin + audited. Resolves the client's `domain` + `jurisdiction`
/// from the register and delegates to the shared DSAR run — no new purge
/// logic. 404 unknown client, 409 archived, before any pool work.
pub async fn client_dsar(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
    Json(req): Json<ClientDsarRequest>,
) -> Result<Json<crate::handlers::observe::DsarResponse>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let pool_for = pool.clone();
    let key = name.trim().to_ascii_lowercase();
    let key_for = key.clone();
    let (domain, jurisdiction, status, mechanism) = tokio::task::spawn_blocking(
        move || -> Result<(String, String, String, Option<String>), HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let c = crate::clients::by_name(&conn, &key_for)?
                .ok_or_else(|| HandlerError::not_found("client not found"))?;
            let mech = crate::transfers::list(&conn, 1, None, Some(&c.jurisdiction), None)?
                .first()
                .map(|t| t.mechanism.clone());
            Ok((c.domain, c.jurisdiction, c.status, mech))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    if status != "active" {
        return Err(HandlerError::conflict("client not active (archived)"));
    }
    let domain_pool = super::resolve_domain_pool(&state.registry, Some(&domain))?;
    let now = chrono::Utc::now().timestamp();
    let resp = crate::handlers::observe::run_dsar_subject(
        state,
        principal,
        domain_pool,
        &req.subject,
        &req.action,
        req.dry_run,
        Some(jurisdiction),
        mechanism,
        now,
    )
    .await?;
    Ok(Json(resp))
}

/// `POST /clients/{name}/hold` body: the ids to freeze in the client's domain
/// and the human citation (`reason`). `reason` is required non-blank (the
/// shared hold validator enforces it); the field defaults so a body without it
/// still deserializes and the validator returns the precise `reason_empty`
/// error.
#[derive(Debug, Deserialize)]
pub struct ClientHoldRequest {
    pub ids: Vec<i64>,
    #[serde(default)]
    pub reason: String,
}

/// `POST /clients/{name}/hold` — place a legal hold on ids in THAT client's
/// domain, never another's. Composes the shared per-domain hold write
/// (`handlers::holds::post_legal_hold_for_domain`) — no second hold
/// implementation. Admin + audited. 404 unknown client, 409 archived, before
/// any pool work.
pub async fn client_hold(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
    Json(req): Json<ClientHoldRequest>,
) -> Result<Json<super::holds::HoldResponse>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let pool_for = pool.clone();
    let key = name.trim().to_ascii_lowercase();
    let (domain, status) =
        tokio::task::spawn_blocking(move || -> Result<(String, String), HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let c = crate::clients::by_name(&conn, &key)?
                .ok_or_else(|| HandlerError::not_found("client not found"))?;
            Ok((c.domain, c.status))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    if status != "active" {
        return Err(HandlerError::conflict("client not active (archived)"));
    }
    super::holds::post_legal_hold_for_domain(state, principal, &domain, req.ids, req.reason).await
}

/// `POST /clients/{name}/proposals/{id}/coach` body. `note` is the supervisor's
/// coaching note; `flagged` is an advisory flag. Both optional.
#[derive(Debug, Deserialize)]
pub struct CoachRequest {
    #[serde(default)]
    pub flagged: bool,
    #[serde(default)]
    pub note: Option<String>,
}

/// `POST /clients/{name}/proposals/{id}/coach` — supervisor coaching on a QA
/// review item in THAT client's domain: set/clear the `qa_note` (the review
/// queue carries it). Never gates approval — it is a flag + note a human
/// decides on. Admin + audited. 404 unknown client or proposal; 409 archived.
pub async fn coach_proposal(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path((name, id)): Path<(String, i64)>,
    Json(req): Json<CoachRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let pool_for = pool.clone();
    let key = name.trim().to_ascii_lowercase();
    let key_for = key.clone();
    let (domain, status) =
        tokio::task::spawn_blocking(move || -> Result<(String, String), HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let c = crate::clients::by_name(&conn, &key_for)?
                .ok_or_else(|| HandlerError::not_found("client not found"))?;
            Ok((c.domain, c.status))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    if status != "active" {
        return Err(HandlerError::conflict("client not active (archived)"));
    }
    let domain_pool = super::resolve_domain_pool(&state.registry, Some(&domain))?;
    let note = req.note;
    let flagged = req.flagged;
    let key_for2 = key.clone();
    let updated = tokio::task::spawn_blocking(move || -> Result<usize, HandlerError> {
        let conn = domain_pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let n = conn
            .execute(
                "UPDATE proposals SET qa_note = ?1 WHERE id = ?2",
                rusqlite::params![note, id],
            )
            .map_err(|e| HandlerError::internal(format!("coach update failed: {e}")))?;
        if n > 0 {
            // The note content is never stored raw in the audit — only the id +
            // flagged flag (the note may contain feedback).
            crate::audit::record(
                &conn,
                AuditKind::Client,
                "api",
                &format!("client:{key_for2}:coach:{id}:{flagged}"),
                AuditStatus::Ok,
                "coach",
            );
        }
        Ok(n)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    if updated == 0 {
        return Err(HandlerError::not_found(format!(
            "no proposal with id {id} in client {name}"
        )));
    }
    Ok(Json(serde_json::json!({
        "proposal_id": id,
        "client": key,
        "flagged": flagged,
        "status": "coached",
    })))
}

/// `GET /clients/{name}/proposals` — the supervisor QA queue for a client:
/// the client's pending review items, owner-scoped to the supervisor's
/// `manages` set (R1 role; empty manages = the whole queue), each with its
/// `qa_score`. Admin. 404 unknown client; 409 archived.
pub async fn client_proposals(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
) -> Result<Json<Vec<crate::handlers::gate::ProposalView>>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let pool_for = pool.clone();
    let key = name.trim().to_ascii_lowercase();
    let key_for = key.clone();
    let (domain, status) =
        tokio::task::spawn_blocking(move || -> Result<(String, String), HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let c = crate::clients::by_name(&conn, &key_for)?
                .ok_or_else(|| HandlerError::not_found("client not found"))?;
            Ok((c.domain, c.status))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    if status != "active" {
        return Err(HandlerError::conflict("client not active (archived)"));
    }
    let domain_pool = super::resolve_domain_pool(&state.registry, Some(&domain))?;
    let manages = principal
        .0
        .as_ref()
        .map(|p| p.manages.clone())
        .unwrap_or_default();
    let rows = tokio::task::spawn_blocking(
        move || -> Result<Vec<crate::handlers::gate::ProposalView>, HandlerError> {
            let conn = domain_pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let page = crate::handlers::gate::list_proposals_page(
                &conn,
                "pending",
                crate::handlers::gate::MAX_PROPOSALS,
                None,
            )?;
            Ok(crate::handlers::gate::owner_in_filtered(page, &manages))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(rows))
}

/// `POST /clients/{name}/end` body. `purge` (optional) overrides the DPA
/// `retention_on_termination` (absent → follow the terms); `dataset` is the
/// tombstone/audit reason tag.
#[derive(Debug, Deserialize)]
pub struct ClientEndRequest {
    #[serde(default, rename = "purge")]
    pub purge_opt: Option<bool>,
    #[serde(default = "default_dataset")]
    pub dataset: String,
}

fn default_dataset() -> String {
    "termination".to_string()
}

/// The contract-end certificate (the operator's durable record; the register
/// archive + audit row survive in the DB). `held_ids` are deferred (never
/// purged); `exported_bundle` is the return-path export; `chain_head` anchors
/// the certificate to the audit tip.
#[derive(Debug, serde::Serialize)]
pub struct TerminationCertificate {
    pub client: String,
    pub domain: String,
    pub jurisdiction: String,
    pub policy: String,
    pub purged_chunk_count: i64,
    pub held_ids: Vec<i64>,
    pub exported_bundle: Option<String>,
    pub archived_at: i64,
    pub chain_head: Option<String>,
}

/// `POST /clients/{name}/end` — run the per-client termination clause: purge or
/// return per the DPA's `retention_on_termination` (overridable), then archive
/// the client + domain (the audit chain is never deleted). Reuses the shared
/// primitives (`purge_chunk_ids`, the DSAR export builder, holds); one audit
/// row. Admin + audited. 404 unknown client, 409 archived.
pub async fn client_end(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
    Json(req): Json<ClientEndRequest>,
) -> Result<Json<TerminationCertificate>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let pool_for = pool.clone();
    let key = name.trim().to_ascii_lowercase();
    let key_for = key.clone();
    let (domain, jurisdiction, status, dpa_purge) = tokio::task::spawn_blocking(
        move || -> Result<(String, String, String, bool), HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let c = crate::clients::by_name(&conn, &key_for)?
                .ok_or_else(|| HandlerError::not_found("client not found"))?;
            let dpa_purge = c
                .dpa_terms
                .as_ref()
                .map(|t| t.retention_on_termination.trim() == "purge")
                .unwrap_or(false);
            Ok((c.domain, c.jurisdiction, c.status, dpa_purge))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    if status != "active" {
        return Err(HandlerError::conflict("client not active (archived)"));
    }
    let purge = req.purge_opt.unwrap_or(dpa_purge);
    let policy = if purge { "purge" } else { "return" }.to_string();
    let dataset = if req.dataset.trim().is_empty() {
        "termination".to_string()
    } else {
        req.dataset.trim().to_string()
    };
    let now = chrono::Utc::now().timestamp();
    let domain_pool = super::resolve_domain_pool(&state.registry, Some(&domain))?;

    let state_for = state.clone();
    let key_for2 = key.clone();
    let policy_for = policy.clone();
    let dataset_for = dataset.clone();
    let cert =
        tokio::task::spawn_blocking(move || -> Result<TerminationCertificate, HandlerError> {
            // Domain first, then global archive + audit: the domain conn is
            // dropped before the global pool conn (shim shares one r2d2 pool,
            // so holding both could deadlock a `max_size(1)`). `archive`
            // no-ops after the first flip, so a crash post-purge recovers by
            // re-running `end`.
            let (purged_chunk_count, held_ids, exported_bundle) = {
                let mut conn = domain_pool
                    .get()
                    .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
                let tx = conn
                    .transaction()
                    .map_err(|e| HandlerError::internal(e.to_string()))?;
                let active: Vec<i64> = {
                    let mut stmt = tx
                        .prepare("SELECT id FROM knowledge")
                        .map_err(|e| HandlerError::internal(e.to_string()))?;
                    let rows = stmt
                        .query_map([], |r| r.get::<_, i64>(0))
                        .map_err(|e| HandlerError::internal(e.to_string()))?;
                    rows.flatten().collect()
                };
                let held_set = crate::legal_hold::active_hold_ids(&tx)?;
                let held_ids: Vec<i64> = active
                    .iter()
                    .filter(|id| held_set.contains(id))
                    .copied()
                    .collect();
                let free: Vec<i64> = active
                    .iter()
                    .filter(|id| !held_set.contains(id))
                    .copied()
                    .collect();
                let (n, bundle) = if purge {
                    let n = crate::handlers::gate::purge_chunk_ids(
                        &tx,
                        &free,
                        now,
                        &dataset_for,
                        None,
                    )?;
                    (n, None)
                } else {
                    (
                        0,
                        Some(crate::handlers::observe::build_export_bundle(
                            &tx,
                            &key_for2,
                            &active,
                            &[],
                        )?),
                    )
                };
                tx.commit()
                    .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
                (n, held_ids, bundle)
            };
            // Archive + audit on the GLOBAL pool after the domain conn is dropped
            // (shim mode shares one r2d2 pool — a held domain conn could deadlock a
            // `max_size(1)` pool). Re-running on an already-archived client is safe:
            // `archive` no-ops after the first flip, so a crash post-purge recovers.
            let mut g = state_for
                .pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let tx = g
                .transaction()
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            if !crate::clients::archive(&tx, &key_for2, now)? {
                return Err(HandlerError::internal(
                    "client row status changed concurrently".to_string(),
                ));
            }
            tx.commit()
                .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
            crate::audit::record(
                &g,
                crate::audit::AuditKind::Client,
                "api",
                &format!("client:{key_for2}"),
                crate::audit::AuditStatus::Ok,
                &format!("termination:{policy_for}:{dataset_for}"),
            );
            let chain_head = crate::audit::chain_head(&g);
            Ok(TerminationCertificate {
                client: key_for2,
                domain,
                jurisdiction,
                policy: policy_for,
                purged_chunk_count,
                held_ids,
                exported_bundle,
                archived_at: now,
                chain_head,
            })
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(cert))
}

/// `POST /clients/{name}/dpa` — set Art 28 sub-processor terms (the evidence a
/// client's controller checks). Admin + audited. 404 when the client is
/// unknown (via the update's affected-row count, no second query).
pub async fn set_client_dpa(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
    Json(req): Json<crate::clients::DpaTerms>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    crate::clients::validate_dpa_terms(&req)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let pool_for = pool.clone();
    let key = name.trim().to_ascii_lowercase();
    let key_for = key.clone();
    let terms = req;
    let changed = tokio::task::spawn_blocking(move || -> Result<usize, HandlerError> {
        let mut conn = pool_for
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let n = crate::clients::set_dpa_terms(&tx, &key_for, &terms)?;
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok(n)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    if changed == 0 {
        return Err(HandlerError::not_found("client not found"));
    }
    if let Ok(conn) = state.pool.get() {
        crate::audit::record(
            &conn,
            AuditKind::Client,
            "api",
            &format!("client:{key}"),
            AuditStatus::Ok,
            "dpa_terms_set",
        );
    }
    Ok(Json(serde_json::json!({ "name": key })))
}

/// `GET /clients/{name}/dpa` — read the stored terms; `null` when set never.
/// Admin read. 404 when the client is unknown.
pub async fn get_client_dpa(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool_for = pool.clone();
    let key = name.trim().to_ascii_lowercase();
    let value = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool_for
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        match crate::clients::by_name(&conn, &key)? {
            None => Err(HandlerError::not_found("client not found")),
            Some(c) => Ok(c
                .dpa_terms
                .map(|t| serde_json::to_value(t).unwrap_or_default())
                .unwrap_or(serde_json::Value::Null)),
        }
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))?;
    Ok(Json(value?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alert::ChainWatchState;
    use crate::auth::jwks::KeyStore;
    use crate::domain_registry::DomainRegistry;
    use crate::integrity::SnapshotState;
    use crate::{AppState, ConnectionTracker, RateLimiter};
    use axum::http::StatusCode;

    fn app_state(dir: &tempfile::TempDir) -> Arc<AppState> {
        app_state_with(dir, true, 4)
    }

    // The shared static embedder is loaded once and reused across tests: many
    // parallel tests each building a fresh model2vec instance raced on huggingface's
    // file-based cache lock ("Lock acquisition failed") under a cold CI cache.
    static TEST_EMBEDDER: std::sync::OnceLock<Arc<dyn brain_server::embed::Embedder>> =
        std::sync::OnceLock::new();

    fn app_state_with(dir: &tempfile::TempDir, multi_db: bool, max_size: u32) -> Arc<AppState> {
        brain_server::register_sqlite_vec::register_sqlite_vec();
        let path = dir.path().join("brain.db");
        let mgr = r2d2_sqlite::SqliteConnectionManager::file(&path);
        let pool: crate::Pool = r2d2::Pool::builder()
            .max_size(max_size)
            .build(mgr)
            .expect("pool");
        brain_server::migration::run_migration(
            &mut pool.get().unwrap(),
            crate::config::DB_MMAP_SIZE_MIB,
        )
        .expect("migration");
        let model: Arc<dyn brain_server::embed::Embedder> = TEST_EMBEDDER
            .get_or_init(|| {
                Arc::new(
                    brain_server::embed::StaticEmbedder::new(crate::config::MODEL_ID)
                        .expect("model"),
                )
            })
            .clone();
        Arc::new(AppState {
            model,
            registry: DomainRegistry::new(pool.clone(), &path, multi_db),
            pool,
            db_path: path.clone(),
            connection_tracker: Arc::new(ConnectionTracker::new()),
            rate_limiter: Arc::new(RateLimiter::new()),
            snapshot: SnapshotState::default(),
            audit_chain_cache: Arc::new(std::sync::Mutex::new(None)),
            auth_mode: crate::auth::AuthMode::Opaque,
            key_store: KeyStore::load(std::path::Path::new("/nonexistent"))
                .expect("empty key store"),
            revocation_cache: Arc::new(crate::auth::revocation::RevocationCache::new()),
            jwt_issuer: String::new(),
            jwt_audience: String::new(),
            oidc_config: crate::handlers::well_known::OidcConfig::unconfigured(),
            ump_events: tokio::sync::broadcast::channel(crate::config::UMP_EVENT_BUFFER).0,
            alert_events: tokio::sync::broadcast::channel(crate::config::ALERT_EVENT_BUFFER).0,
            alert_seq: std::sync::atomic::AtomicU64::new(0),
            chain_watch: ChainWatchState::default(),
        })
    }

    fn register_client(state: &AppState, name: &str, domain: &str, jurisdiction: &str) {
        crate::clients::scaffold_and_register(
            &state.registry,
            &state.pool,
            name,
            domain,
            jurisdiction,
            None,
            1_000,
        )
        .expect("register client");
    }

    fn seed_subject(state: &AppState, domain: &str, owner: &str) {
        let pool = state.registry.pool_for(domain).expect("domain pool");
        pool.get()
            .unwrap()
            .execute(
                "INSERT INTO knowledge(content, content_hash, owner) VALUES ('data', 'h', ?1)",
                rusqlite::params![owner],
            )
            .expect("seed subject row");
    }

    fn count_knowledge(state: &AppState, domain: &str) -> i64 {
        state
            .registry
            .pool_for(domain)
            .unwrap()
            .get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .unwrap_or(0)
    }

    #[tokio::test]
    async fn per_client_dsar_scoped_to_domain() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "beta", "beta-eu", "eu");
        register_client(&state, "acme", "acme-us", "us");
        seed_subject(&state, "beta-eu", "alice@beta");
        seed_subject(&state, "acme-us", "alice@beta");

        let resp = client_dsar(
            State(state.clone()),
            OptPrincipal(None),
            Path("beta".to_string()),
            Json(ClientDsarRequest {
                subject: "alice@beta".to_string(),
                action: "purge".to_string(),
                dry_run: false,
            }),
        )
        .await
        .expect("dsar runs");
        assert_eq!(resp.status, "completed");
        assert_eq!(resp.jurisdiction.as_deref(), Some("eu"));
        assert_eq!(resp.deadline, resp.created_at + 30 * 86400);
        assert!(resp.rights.contains(&"objection"));
        assert!(resp.certificate.is_some(), "certificate present");
        assert_eq!(
            count_knowledge(&state, "beta-eu"),
            0,
            "beta-eu fully purged"
        );
        assert_eq!(
            count_knowledge(&state, "acme-us"),
            1,
            "acme-us untouched (domain isolation)"
        );
    }

    #[tokio::test]
    async fn per_client_dsar_unknown_or_archived_client_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let req = Json(ClientDsarRequest {
            subject: "s".to_string(),
            action: "purge".to_string(),
            dry_run: true,
        });
        let err = client_dsar(
            State(state.clone()),
            OptPrincipal(None),
            Path("nope".to_string()),
            req,
        )
        .await
        .expect_err("unknown client 404s");
        assert_eq!(err.status, StatusCode::NOT_FOUND);

        register_client(&state, "beta", "beta-eu", "eu");
        state
            .pool
            .get()
            .unwrap()
            .execute("UPDATE clients SET status='archived' WHERE name='beta'", [])
            .expect("archive");
        let err = client_dsar(
            State(state.clone()),
            OptPrincipal(None),
            Path("beta".to_string()),
            Json(ClientDsarRequest {
                subject: "s".to_string(),
                action: "purge".to_string(),
                dry_run: true,
            }),
        )
        .await
        .expect_err("archived client 409s");
        assert_eq!(err.status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn legal_hold_per_client_isolates_domains() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "beta", "beta-eu", "eu");
        register_client(&state, "acme", "acme-us", "us");
        let id_beta = {
            let pool = state.registry.pool_for("beta-eu").unwrap();
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO knowledge(content, content_hash, owner) VALUES ('data','h','alice')",
                [],
            )
            .expect("seed beta row");
            conn.query_row("SELECT MAX(id) FROM knowledge", [], |r| r.get(0))
                .unwrap()
        };
        let id_acme = {
            let pool = state.registry.pool_for("acme-us").unwrap();
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO knowledge(content, content_hash, owner) VALUES ('data','h','alice')",
                [],
            )
            .expect("seed acme row");
            conn.query_row("SELECT MAX(id) FROM knowledge", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(
            id_beta, id_acme,
            "identical autoincrement ids across domains"
        );

        let resp = client_hold(
            State(state.clone()),
            OptPrincipal(None),
            Path("acme".to_string()),
            Json(ClientHoldRequest {
                ids: vec![id_acme],
                reason: "case 2026-118".to_string(),
            }),
        )
        .await
        .expect("hold lands on acme's domain");
        assert_eq!(resp.held, 1);

        let acme_held = {
            let conn = state.registry.pool_for("acme-us").unwrap().get().unwrap();
            crate::legal_hold::active_hold_ids(&conn).unwrap()
        };
        assert!(acme_held.contains(&id_acme), "acme's id is held in acme-us");
        let beta_held = {
            let conn = state.registry.pool_for("beta-eu").unwrap().get().unwrap();
            crate::legal_hold::active_hold_ids(&conn).unwrap()
        };
        assert!(
            !beta_held.contains(&id_beta),
            "beta's identical-id row is NOT held (isolation)"
        );
        assert!(
            acme_held != beta_held,
            "the held sets must differ across domains"
        );
    }

    #[tokio::test]
    async fn client_hold_unknown_or_archived_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let body = Json(ClientHoldRequest {
            ids: vec![1],
            reason: "case".to_string(),
        });
        let err = client_hold(
            State(state.clone()),
            OptPrincipal(None),
            Path("nope".to_string()),
            body,
        )
        .await
        .expect_err("unknown client 404s");
        assert_eq!(err.status, StatusCode::NOT_FOUND);

        register_client(&state, "beta", "beta-eu", "eu");
        state
            .pool
            .get()
            .unwrap()
            .execute("UPDATE clients SET status='archived' WHERE name='beta'", [])
            .expect("archive");
        let err = client_hold(
            State(state.clone()),
            OptPrincipal(None),
            Path("beta".to_string()),
            Json(ClientHoldRequest {
                ids: vec![1],
                reason: "case".to_string(),
            }),
        )
        .await
        .expect_err("archived client 409s");
        assert_eq!(err.status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn per_client_dsar_shim_single_pool_no_deadlock() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state_with(&dir, false, 1);
        register_client(&state, "beta", "beta", "eu");
        seed_subject(&state, "global", "alice@beta");

        let resp = client_dsar(
            State(state.clone()),
            OptPrincipal(None),
            Path("beta".to_string()),
            Json(ClientDsarRequest {
                subject: "alice@beta".to_string(),
                action: "purge".to_string(),
                dry_run: false,
            }),
        )
        .await
        .expect("shim dsar completes (no pool deadlock)");
        assert_eq!(resp.status, "completed");
        assert!(resp.certificate.is_some());
        assert_eq!(count_knowledge(&state, "global"), 0, "shim subject purged");
        let cert: Option<String> = state
            .pool
            .get()
            .unwrap()
            .query_row("SELECT certificate FROM dsar_requests LIMIT 1", [], |r| {
                r.get(0)
            })
            .expect("ledger row");
        assert!(
            cert.is_some() && cert.unwrap().contains("\"jurisdiction\":\"eu\""),
            "certificate backfilled with the client's jurisdiction"
        );
    }

    fn seed_rows(state: &AppState, domain: &str, n: i64) -> Vec<i64> {
        let pool = state.registry.pool_for(domain).unwrap();
        let conn = pool.get().unwrap();
        let mut ids = Vec::new();
        for i in 0..n {
            conn.execute(
                "INSERT INTO knowledge(content, content_hash, owner) VALUES (?1, ?2, 'o')",
                rusqlite::params![format!("data-{i}"), format!("h{i}")],
            )
            .expect("seed row");
            ids.push(conn.last_insert_rowid());
        }
        ids
    }

    fn set_client_dpa_direct(state: &AppState, name: &str, retention: &str) {
        let terms = crate::clients::DpaTerms {
            retention_on_termination: retention.into(),
            deletion_timeline: "30d".into(),
            audit_rights: "annual".into(),
            breach_notification_timeline: "72h".into(),
            onward_transfer_restriction: "none".into(),
            sub_sub_processor_list: "none".into(),
        };
        let mut conn = state.pool.get().unwrap();
        let tx = conn.transaction().unwrap();
        crate::clients::set_dpa_terms(&tx, name, &terms).unwrap();
        tx.commit().unwrap();
    }

    #[tokio::test]
    async fn client_end_runs_termination_clause() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "acme", "acme-us", "us");
        set_client_dpa_direct(&state, "acme", "purge");
        seed_rows(&state, "acme-us", 2);

        let resp = client_end(
            State(state.clone()),
            OptPrincipal(None),
            Path("acme".to_string()),
            Json(ClientEndRequest {
                purge_opt: None,
                dataset: "termination".to_string(),
            }),
        )
        .await
        .expect("end follows the DPA purge policy");
        assert_eq!(resp.policy, "purge");
        assert_eq!(resp.purged_chunk_count, 2);
        assert!(resp.exported_bundle.is_none());
        assert!(resp.chain_head.is_some());
        assert_eq!(count_knowledge(&state, "acme-us"), 0);
        let c = crate::clients::by_name(&state.pool.get().unwrap(), "acme")
            .unwrap()
            .unwrap();
        assert_eq!(c.status, "archived");
        assert_eq!(c.archived_at, Some(resp.archived_at));
    }

    #[tokio::test]
    async fn client_end_return_exports_and_archives_no_purge() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "acme", "acme-us", "us");
        set_client_dpa_direct(&state, "acme", "return");
        seed_rows(&state, "acme-us", 2);

        let resp = client_end(
            State(state.clone()),
            OptPrincipal(None),
            Path("acme".to_string()),
            Json(ClientEndRequest {
                purge_opt: Some(false),
                dataset: "termination".to_string(),
            }),
        )
        .await
        .expect("return policy exports without purging");
        assert_eq!(resp.policy, "return");
        assert_eq!(resp.purged_chunk_count, 0);
        let bundle = resp.exported_bundle.as_deref().expect("bundle present");
        let v: serde_json::Value = serde_json::from_str(bundle).unwrap();
        assert_eq!(v["subject"].as_str(), Some("acme"));
        assert_eq!(v["knowledge"].as_array().unwrap().len(), 2);
        assert_eq!(count_knowledge(&state, "acme-us"), 2, "no purge on return");
        assert_eq!(
            crate::clients::by_name(&state.pool.get().unwrap(), "acme")
                .unwrap()
                .unwrap()
                .status,
            "archived"
        );
    }

    #[tokio::test]
    async fn client_end_defers_held_ids_and_archive_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "acme", "acme-us", "us");
        let ids = seed_rows(&state, "acme-us", 2);
        let pool = state.registry.pool_for("acme-us").unwrap();
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO legal_holds(knowledge_id, reason, held_by, held_at)
             VALUES (?1, 'case-42', 'test', 1)",
            rusqlite::params![ids[1]],
        )
        .expect("hold the second row");

        let resp = client_end(
            State(state.clone()),
            OptPrincipal(None),
            Path("acme".to_string()),
            Json(ClientEndRequest {
                purge_opt: Some(true),
                dataset: "termination".to_string(),
            }),
        )
        .await
        .expect("purge terminates");
        assert_eq!(resp.purged_chunk_count, 1, "only the free row purged");
        assert_eq!(
            resp.held_ids,
            vec![ids[1]],
            "held id deferred on the certificate, never purged"
        );
        assert_eq!(count_knowledge(&state, "acme-us"), 1, "held row survives");

        let err = client_end(
            State(state.clone()),
            OptPrincipal(None),
            Path("acme".to_string()),
            Json(ClientEndRequest {
                purge_opt: Some(true),
                dataset: "termination".to_string(),
            }),
        )
        .await
        .expect_err("already-archived client 409s");
        assert_eq!(err.status, StatusCode::CONFLICT);
    }

    #[test]
    fn client_end_unknown_client_404s_before_pool_work() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(client_end(
            State(state.clone()),
            OptPrincipal(None),
            Path("nope".to_string()),
            Json(ClientEndRequest {
                purge_opt: None,
                dataset: "termination".to_string(),
            }),
        ));
        let err = err.expect_err("unknown client 404s");
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn coach_attaches_note_and_audits() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "beta", "beta-eu", "eu");
        let did: i64 = state
            .registry
            .pool_for("beta-eu")
            .unwrap()
            .get()
            .unwrap()
            .query_row(
                "INSERT INTO proposals(kind, content, novelty, salience, created_at, owner)
                 VALUES ('fact', 'body', 0.9, 0.5, 0, 'agent-1') RETURNING id",
                [],
                |r| r.get(0),
            )
            .unwrap();

        let resp = coach_proposal(
            State(state.clone()),
            OptPrincipal(None),
            Path(("beta".to_string(), did)),
            Json(CoachRequest {
                flagged: true,
                note: Some("follow up".to_string()),
            }),
        )
        .await
        .expect("coach runs");
        assert_eq!(resp["flagged"], true);
        assert_eq!(resp["status"], "coached");

        let note: Option<String> = state
            .registry
            .pool_for("beta-eu")
            .unwrap()
            .get()
            .unwrap()
            .query_row("SELECT qa_note FROM proposals WHERE id = ?1", [did], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(note.as_deref(), Some("follow up"));

        let audited: i64 = state
            .registry
            .pool_for("beta-eu")
            .unwrap()
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'client'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(audited >= 1, "coach is audited");

        let err = coach_proposal(
            State(state.clone()),
            OptPrincipal(None),
            Path(("beta".to_string(), 99_999)),
            Json(CoachRequest {
                flagged: false,
                note: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND, "unknown proposal 404s");
    }

    #[tokio::test]
    async fn qa_review_queue_surfaces_agent_interactions() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "beta", "beta-eu", "eu");
        let conn = state.registry.pool_for("beta-eu").unwrap().get().unwrap();
        for (i, owner) in ["agent-1", "other-agent"].iter().enumerate() {
            conn.execute(
                "INSERT INTO proposals(kind, content, novelty, salience, created_at, owner)
                 VALUES ('fact', ?1, 0.9, 0.5, ?2, ?3)",
                rusqlite::params![format!("body {i}"), i as i64, owner],
            )
            .unwrap();
        }
        let sup = crate::auth::Principal {
            sub: "super@beta".to_string(),
            tenant: "beta".to_string(),
            scopes: vec![crate::auth::Scope::parse("admin:beta/*").unwrap()],
            jti: "t".to_string(),
            roles: vec![],
            manages: vec!["agent-1".to_string()],
        };
        let resp = client_proposals(
            State(state.clone()),
            OptPrincipal(Some(sup)),
            Path("beta".to_string()),
        )
        .await
        .expect("qa list runs");
        assert_eq!(resp.len(), 1, "only the managed agent's proposal surfaces");
        assert_eq!(resp[0].owner.as_deref(), Some("agent-1"));
        assert!(resp[0].qa_score > 0, "qa list carries a score");
    }
}
