//! the BPO operating register (HTTP surface).
//!
//! `POST /clients` registers an operating client (name / isolation domain /
//! jurisdiction / bound profile); `GET /clients` lists the register; `GET
//! /clients/{name}` resolves one row. Every write is Admin-gated + hash-chained
//! into the audit (`AuditKind::Client`). This is the evidence/identity register
//! only — it does not gate enforcement (that is v1.27.x + v2.x).
//!
//! The storage story lives in [`crate::service::register`] — this file is the
//! protocol adapter: parse → gate → `spawn_blocking` → core call →
//! typed-error mapping → response. The domain registry (the pool authority)
//! never crosses the service boundary.

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use std::sync::Arc;

use crate::AppState;
use crate::audit::{AuditKind, AuditStatus};
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::service::register::RegisterError;

/// The handler-boundary map for the register core: the typed variants render
/// the route family's FROZEN probe-blind vocabulary — the exact pre-move
/// messages, byte for byte (404 unknown, 409 archived/duplicate, the
/// registration fences as 400s, the shared `409 legal_hold_active` envelope
/// for the termination purge's in-function backstop).
impl From<RegisterError> for HandlerError {
    fn from(e: RegisterError) -> Self {
        match e {
            RegisterError::Database(m) => HandlerError::internal(m),
            RegisterError::InvalidName => HandlerError::bad_request(
                "client_name_invalid",
                "name must be a lowercase domain-safe identifier (\u{2264} 63 chars)",
            ),
            RegisterError::InvalidDomain => HandlerError::bad_request(
                "client_domain_invalid",
                "domain must be a valid domain name",
            ),
            RegisterError::InvalidJurisdiction => HandlerError::bad_request(
                "jurisdiction_invalid",
                "jurisdiction must be a short lowercase country code",
            ),
            RegisterError::Duplicate => HandlerError::conflict("client already exists"),
            RegisterError::UnknownClient => HandlerError::not_found("client not found"),
            RegisterError::ClientArchived => HandlerError::conflict("client not active (archived)"),
            RegisterError::ArchivedReregister(name) => {
                HandlerError::conflict(format!("client {name} is archived — re-register refused"))
            }
            RegisterError::ProfileNotFound(e) => HandlerError::bad_request("profile_not_found", e),
            RegisterError::InvalidDpa(msg) => HandlerError::bad_request("dpa_field_invalid", msg),
            RegisterError::Serialize(m) => HandlerError::internal(m),
            RegisterError::LegalHold(held) => HandlerError::conflict_with(
                "legal_hold_active",
                "one or more ids are under legal hold",
                serde_json::json!({ "held": held }),
            ),
        }
    }
}

/// `POST /clients` body. `profile` is optional (the bound profile is a later
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
    crate::service::register::validate_new_client(&req.name, &req.domain, &req.jurisdiction)?;

    // Scaffold the client's domain before opening the tx — the registry's
    // register step creates + migrates the domain DB (multi-db) or touches
    // the shared pool (shim). THE POOL AUTHORITY STAYS HERE; the optional
    // profile bind + the `clients` row are the core's in-tx story.
    // Composition only, no new logic.
    let st = state.clone();
    let now = chrono::Utc::now().timestamp();
    let name_for = req.name.clone();
    let domain_for = req.domain.clone();
    let jurisdiction_for = req.jurisdiction.clone();
    let profile_for = req.profile.clone();
    tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
        st.registry
            .register(&domain_for)
            .map_err(super::map_domain_error)?;
        let mut conn = st
            .pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        crate::service::register::scaffold_and_register(
            &tx,
            &name_for,
            &domain_for,
            &jurisdiction_for,
            profile_for.as_deref(),
            now,
        )?;
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok(())
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

/// `GET /clients` — the registered clients, ordered by name. Admin read for
/// every principal EXCEPT a `client-auditor`, whose view is a row-level filter
/// to exactly its granted client-domain(s) (parent verification #7) — the
/// server enforces `authorize` on the path too (defense-in-depth). The row
/// filter itself lives IN the core (`list_for_domain_grants`).
pub async fn list_clients(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let granted = crate::auth::client_authorized_domains(&principal.0);
    match &granted {
        Some(g) if !g.is_empty() => {
            super::authorize(&principal.0, crate::auth::Action::Read, "", &g[0])?
        }
        None => super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?,
        // An auditor token with an EMPTY grant set
        // previously skipped the gate entirely (falling through to the row
        // filter, which yields nothing). "Some([]) denies all" — deny at the
        // gate too, not just at the rows: the surface is closed to this
        // principal, loudly.
        Some(_) => {
            return Err(HandlerError::forbidden(
                crate::auth::Action::Read,
                "",
                "clients",
            ));
        }
    }
    let pool_for = pool.clone();
    let rows = tokio::task::spawn_blocking(
        move || -> Result<Vec<crate::service::register::Client>, HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            Ok(crate::service::register::list_for_domain_grants(
                &conn,
                granted.as_deref(),
            )?)
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    let rows: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|c| serde_json::to_value(c).unwrap_or_default())
        .collect();
    Ok(Json(serde_json::json!({ "clients": rows })))
}

/// `GET /clients/{name}` — resolve one client. Admin read; 404 when absent.
/// For a `client-auditor`, the specific client is denied (404, no existence
/// leak) unless its domain is in the principal's granted set.
pub async fn get_client(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let granted = crate::auth::client_authorized_domains(&principal.0);
    if granted.is_none() {
        super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    }
    let pool_for = pool.clone();
    let key = name.trim().to_ascii_lowercase();
    let value = tokio::task::spawn_blocking(
        move || -> Result<Option<crate::service::register::Client>, HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            Ok(crate::service::register::by_name(&conn, &key)?)
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    let client = value.ok_or_else(|| HandlerError::not_found("client not found"))?;
    if let Some(g) = granted {
        if !g.iter().any(|d| d == &client.domain) {
            return Err(HandlerError::not_found("client not found"));
        }
        super::authorize(&principal.0, crate::auth::Action::Read, "", &client.domain)?;
    }
    Ok(Json(serde_json::to_value(client).unwrap_or_default()))
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
    /// Exact-subject matching for the residue sweeps (default: the
    /// erasure-safe substring sweep).
    #[serde(default)]
    pub subject_exact: bool, // subject_exact defaults false via serde; test literals use ..Default::default()-free explicit form
}

fn default_dsar_action() -> String {
    "purge".to_string()
}

/// `POST /clients/{name}/dsar` — a subject erasure scoped to a single client's
/// domain, stamped with that client's jurisdiction, deadline, rights, and
/// transfer mechanism (the "erase Client Beta's data on contract end" building
/// block). Admin + audited. The core's `require_active_client` seam resolves
/// the client (404 unknown, 409 archived, before any domain-pool work); the
/// shared DSAR run does the rest — no new purge logic.
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
    let (domain, jurisdiction, mechanism) = tokio::task::spawn_blocking(
        move || -> Result<(String, String, Option<String>), HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let c = crate::service::register::require_active_client(&conn, &key)?;
            let mech = crate::transfers::list(&conn, 1, None, Some(&c.jurisdiction), None)?
                .first()
                .map(|t| t.mechanism.clone());
            Ok((c.domain, c.jurisdiction, mech))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    let domain_pool = super::resolve_domain_pool(&state.registry, Some(&domain))?;
    let now = chrono::Utc::now().timestamp();
    let resp = crate::handlers::observe::run_dsar_subject(
        state,
        principal,
        domain_pool,
        &domain,
        &req.subject,
        &req.action,
        req.dry_run,
        Some(jurisdiction),
        mechanism,
        req.subject_exact,
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
/// domain, never another's. The core's `require_active_client` seam resolves
/// the client's domain (404 unknown, 409 archived, before any pool work);
/// the write composes the shared per-domain hold write
/// (`handlers::holds::post_legal_hold_for_domain`) — no second hold
/// implementation. Admin + audited.
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
    let domain = tokio::task::spawn_blocking(move || -> Result<String, HandlerError> {
        let conn = pool_for
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        Ok(crate::service::register::require_active_client(&conn, &key)?.domain)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
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
/// queue carries it) and emit the audit row INSIDE the same tx (the core's
/// `coach_note`). Never gates approval — it is a flag + note a human decides
/// on. Admin + audited. 404 unknown client or proposal; 409 archived.
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
    let key_lookup = key.clone();
    let domain = tokio::task::spawn_blocking(move || -> Result<String, HandlerError> {
        let conn = pool_for
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        Ok(crate::service::register::require_active_client(&conn, &key_lookup)?.domain)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    let domain_pool = super::resolve_domain_pool(&state.registry, Some(&domain))?;
    let note = req.note;
    let flagged = req.flagged;
    let key_for = key.clone();
    let updated = tokio::task::spawn_blocking(move || -> Result<usize, HandlerError> {
        let mut conn = domain_pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let n = crate::service::register::coach_note(&tx, &key_for, id, note, flagged)?;
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
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
/// `manages` set (empty manages = the whole queue), each with its
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
    let domain = tokio::task::spawn_blocking(move || -> Result<String, HandlerError> {
        let conn = pool_for
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        Ok(crate::service::register::require_active_client(&conn, &key)?.domain)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
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
/// the client + domain (the audit chain is never deleted). The DATA phase is
/// the core's `termination_clause` (around the shared purge/export
/// primitives); the archive flip + its audit row are the core's, in the
/// caller's global-DB tx. One audit row. Admin + audited. 404 unknown client,
/// 409 archived.
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
    let (domain, jurisdiction, dpa_purge) =
        tokio::task::spawn_blocking(move || -> Result<(String, String, bool), HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let c = crate::service::register::require_active_client(&conn, &key_for)?;
            let dpa_purge = c
                .dpa_terms
                .as_ref()
                .map(|t| t.retention_on_termination.trim() == "purge")
                .unwrap_or(false);
            Ok((c.domain, c.jurisdiction, dpa_purge))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
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
            let outcome = {
                let mut conn = domain_pool
                    .get()
                    .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
                let tx = conn
                    .transaction()
                    .map_err(|e| HandlerError::internal(e.to_string()))?;
                let out = crate::service::register::termination_clause(
                    &tx,
                    &key_for2,
                    purge,
                    now,
                    &dataset_for,
                )?;
                tx.commit()
                    .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
                out
            };
            // Archive + audit on the GLOBAL pool after the domain conn is dropped
            // (shim mode shares one r2d2 pool — a held domain conn could deadlock a
            // `max_size(1)` pool). Re-running on an already-archived client is safe:
            // `archive` no-ops after the first flip, so a crash post-purge recovers.
            // The audit row rides INSIDE the archive tx (SAVEPOINT-nested) —
            // the transition and its evidence commit together.
            let mut g = state_for
                .pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let tx = g
                .transaction()
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            if !crate::service::register::archive(&tx, &key_for2, now)? {
                return Err(HandlerError::internal(
                    "client row status changed concurrently".to_string(),
                ));
            }
            crate::audit::record(
                &tx,
                crate::audit::AuditKind::Client,
                "api",
                &format!("client:{key_for2}"),
                crate::audit::AuditStatus::Ok,
                &format!("termination:{policy_for}:{dataset_for}"),
            );
            tx.commit()
                .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
            let chain_head = crate::audit::chain_head(&g);
            Ok(TerminationCertificate {
                client: key_for2,
                domain,
                jurisdiction,
                policy: policy_for,
                purged_chunk_count: outcome.purged_chunk_count,
                held_ids: outcome.held_ids,
                exported_bundle: outcome.exported_bundle,
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
    Json(req): Json<crate::service::register::DpaTerms>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    crate::service::register::validate_dpa_terms(&req)?;
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
        let n = crate::service::register::set_dpa_terms(&tx, &key_for, &terms)?;
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
        let c = crate::service::register::by_name(&conn, &key)?
            .ok_or_else(|| HandlerError::not_found("client not found"))?;
        Ok(c.dpa_terms
            .map(|t| serde_json::to_value(t).unwrap_or_default())
            .unwrap_or(serde_json::Value::Null))
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

    // Fixtures for the legal-hold fence pins below (the erasure surfaces
    // these pins exercise — `DELETE /memory/{id}`, the source sweeps, ump
    // forget, hold release — are OTHER aggregates; their pins ride with
    // those surfaces' own extractions, not this one).

    fn seed_subject(state: &AppState, domain: &str, owner: &str) {
        // registered-only; `register` is idempotent.
        let pool = state.registry.register(domain).expect("domain pool");
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
            .register(domain)
            .unwrap()
            .get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .unwrap_or(0)
    }

    /// Register-fixture for the DSAR-certificate pin (the composition seam:
    /// registry scaffold + the core's in-tx story).
    fn register_client_fixture(state: &AppState, name: &str, domain: &str, jurisdiction: &str) {
        state.registry.register(domain).expect("domain scaffold");
        let mut conn = state.pool.get().expect("global conn");
        let tx = conn.transaction().unwrap();
        crate::service::register::scaffold_and_register(
            &tx,
            name,
            domain,
            jurisdiction,
            None,
            1_000,
        )
        .expect("register client");
        tx.commit().unwrap();
    }

    #[tokio::test]
    async fn client_auditor_can_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let auditor = OptPrincipal(Some(crate::auth::Principal {
            sub: "compliance@acme".to_string(),
            tenant: "ops".to_string(),
            scopes: vec![crate::auth::Scope::parse("admin:ops/acme-us").unwrap()],
            jti: "a".to_string(),
            roles: vec!["client-auditor".to_string()],
            manages: vec![],
        }));
        let err = crate::handlers::authorize_role(&auditor.0, &state.pool, "admin")
            .expect_err("client-auditor cannot admin");
        assert_eq!(err.status, StatusCode::FORBIDDEN);
        assert!(crate::handlers::authorize_role(&auditor.0, &state.pool, "read").is_ok());
    }

    // ── the universal legal-hold fence ────────
    // Every erasure path — `/purge`, DSAR, `DELETE /memory/{id}`, the source
    // sweeps (single delete + reconcile), quarantine delete, domain delete —
    // runs one `refuse_if_held` inside its write tx and answers `409
    // legal_hold_active`. One test per bypass path (plan M1.2) + the tombstone
    // digest parity (M1.3).

    fn seed_global_chunk(state: &AppState, content: &str) -> i64 {
        let conn = state.pool.get().unwrap();
        conn.execute(
            "INSERT INTO knowledge(content, content_hash, owner, document_id)
             VALUES (?1, ?2, 'o', 'doc-1')",
            rusqlite::params![content, format!("h{content}")],
        )
        .expect("seed global chunk");
        conn.last_insert_rowid()
    }

    fn hold(domain_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>, ids: &[i64]) {
        let mut conn = domain_pool.get().unwrap();
        let tx = conn.transaction().unwrap();
        crate::legal_hold::insert_holds(&tx, ids, "litigation 2026-118", Some("dpo"), 60).unwrap();
        tx.commit().unwrap();
    }

    fn seed_source_chunk(state: &AppState, uri: &str, kind: &str, content: &str) -> (i64, i64) {
        let conn = state.pool.get().unwrap();
        conn.execute(
            "INSERT INTO sources(uri, kind) VALUES (?1, ?2)",
            rusqlite::params![uri, kind],
        )
        .unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO source_revisions(source_id, revision, content_hash, chunk_count,
                                           byte_size, state)
             VALUES (?1, 'r1', ?2, 1, ?3, 'active')",
            rusqlite::params![sid, format!("h{content}"), content.len() as i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO knowledge(content, content_hash, owner, source_id, revision_id)
             VALUES (?1, ?2, 'o', ?3, ?4)",
            rusqlite::params![
                content,
                format!("h{content}"),
                sid,
                conn.last_insert_rowid()
            ],
        )
        .unwrap();
        (sid, conn.last_insert_rowid())
    }

    #[tokio::test]
    async fn delete_memory_refuses_held_id() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let id = seed_global_chunk(&state, "evidence under litigation");
        hold(&state.pool, &[id]);

        let err =
            crate::handlers::forget::forget(State(state.clone()), OptPrincipal(None), Path(id))
                .await
                .expect_err("a held id must refuse DELETE /memory/{id}");
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.inner.code, "legal_hold_active");

        let free: i64 = state
            .pool
            .get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM knowledge WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(free, 1, "the held row survives");

        // Release the hold → the same path erases it (the fence is per-id).
        let mut conn = state.pool.get().unwrap();
        let tx = conn.transaction().unwrap();
        crate::legal_hold::release(&tx, 1, 61).unwrap();
        tx.commit().unwrap();
        let resp =
            crate::handlers::forget::forget(State(state.clone()), OptPrincipal(None), Path(id))
                .await
                .expect("a released id deletes normally");
        assert!(resp.deleted);
    }

    #[tokio::test]
    async fn delete_memory_tombstone_carries_content_digest() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let id = seed_global_chunk(&state, "the deleted subject's evidence");

        let _ = crate::handlers::forget::forget(State(state.clone()), OptPrincipal(None), Path(id))
            .await
            .expect("forget runs");

        let (hash, doc): (Option<String>, Option<String>) = state
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT content_hash, document_id FROM tombstones WHERE knowledge_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            hash.as_deref(),
            Some(crate::handlers::gate::sha256_hex("the deleted subject's evidence").as_str()),
            "the tombstone carries the same SHA-256 evidence /purge writes"
        );
        assert_eq!(doc.as_deref(), Some("doc-1"));
    }

    #[tokio::test]
    async fn source_delete_refuses_held_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let (sid, cid) = seed_source_chunk(&state, "/v/legal.md", "vault", "held under hold");
        hold(&state.pool, &[cid]);

        let err = crate::handlers::sources::delete_source(
            State(state.clone()),
            OptPrincipal(None),
            Path(sid),
        )
        .await
        .expect_err("a source with a held chunk must refuse DELETE /sources/{id}");
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.inner.code, "legal_hold_active");

        let (chunk, sstate): (i64, String) = state
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT (SELECT COUNT(*) FROM knowledge WHERE id = ?1),
                        (SELECT state FROM sources WHERE id = ?2)",
                rusqlite::params![cid, sid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((chunk, sstate.as_str()), (1, "active"), "nothing erased");
    }

    #[tokio::test]
    async fn ump_hard_forget_refuses_held_chunk() {
        // S2-03 (CRITICAL): `POST /ump/forget {"hard":true}` reaches
        // `purge_chunk_ids` — the legal-hold fence that guards every other
        // erasure path must guard this one too (it is MCP-reachable at Write
        // scope via the `ump.forget` tool).
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let (_sid, cid) = seed_source_chunk(&state, "/v/hold.md", "vault", "litigation evidence");
        hold(&state.pool, &[cid]);
        let cid_s = cid.to_string();

        let err = crate::handlers::ump_ops::forget(
            State(state.clone()),
            OptPrincipal(None),
            crate::handlers::auth::OptCapability(None),
            Json(crate::handlers::ump_ops::ForgetRequest {
                id: cid_s.clone(),
                reason: Some("ump_forget".to_string()),
                hard: true,
            }),
        )
        .await
        .expect_err("a hard forget of a held chunk must 409");
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.inner.code, "legal_hold_active");

        let chunks: i64 = state
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM knowledge WHERE id = ?1",
                rusqlite::params![cid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(chunks, 1, "held chunk survives the hard-forget attempt");

        // Release → the same request erases.
        let mut conn = state.pool.get().unwrap();
        let tx = conn.transaction().unwrap();
        crate::legal_hold::release(&tx, 1, 63).unwrap();
        tx.commit().unwrap();
        drop(conn);
        let _erased = crate::handlers::ump_ops::forget(
            State(state.clone()),
            OptPrincipal(None),
            crate::handlers::auth::OptCapability(None),
            Json(crate::handlers::ump_ops::ForgetRequest {
                id: cid_s,
                reason: Some("ump_forget".to_string()),
                hard: true,
            }),
        )
        .await
        .expect("after release the hard forget completes");
        let chunks: i64 = state
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM knowledge WHERE id = ?1",
                rusqlite::params![cid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(chunks, 0, "released chunk is erased");
    }

    #[tokio::test]
    async fn ump_forget_soft_flags_but_not_held_chunks() {
        // S2-03 complement (v1.27.27 M2): the SOFT branch is not an erasure —
        // it flags (quarantine-style) + tombstones, so a held chunk may be
        // soft-forgotten (still retrievable with include_flagged) but is
        // NEVER purged by it. The hold freezes erasure, not flagging.
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let (_sid, cid) = seed_source_chunk(&state, "/v/hold.md", "vault", "litigation evidence");
        hold(&state.pool, &[cid]);
        let cid_s = cid.to_string();

        let resp = crate::handlers::ump_ops::forget(
            State(state.clone()),
            OptPrincipal(None),
            crate::handlers::auth::OptCapability(None),
            Json(crate::handlers::ump_ops::ForgetRequest {
                id: cid_s,
                reason: Some("ump_forget".to_string()),
                hard: false,
            }),
        )
        .await
        .expect("soft forget of a held chunk proceeds (it is not an erasure)");
        assert_eq!(resp.0["result"], "tombstoned");

        let (chunks, flagged): (i64, i64) = state
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*), MAX(flagged) FROM knowledge WHERE id = ?1",
                rusqlite::params![cid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (chunks, flagged),
            (1, 1),
            "the held chunk survives soft forget (no purge) and is flagged"
        );
    }

    #[tokio::test]
    async fn empty_live_set_requires_explicit_allow_empty() {
        // S2/N1: an empty live_uris retires EVERY active source of the kind —
        // indistinguishable on the wire from a caller whose listing failed, so
        // it must be an explicit decision, not a default.
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let (sid, _cid) = seed_source_chunk(&state, "/v/a.md", "vault", "content");

        let err = crate::handlers::sources::reconcile(
            State(state.clone()),
            OptPrincipal(None),
            Json(crate::handlers::sources::ReconcileRequest {
                kind: "vault".to_string(),
                live_uris: vec![],
                allow_empty: false,
            }),
        )
        .await
        .expect_err("an unconfirmed empty live set must be refused");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.inner.code, "live_set_empty");

        let sstate: String = state
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT state FROM sources WHERE id = ?1",
                rusqlite::params![sid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sstate, "active", "nothing retired by the refused request");

        // The explicit confirmation retires it.
        let resp = crate::handlers::sources::reconcile(
            State(state.clone()),
            OptPrincipal(None),
            Json(crate::handlers::sources::ReconcileRequest {
                kind: "vault".to_string(),
                live_uris: vec![],
                allow_empty: true,
            }),
        )
        .await
        .expect("confirmed empty set proceeds");
        assert_eq!(resp.deleted_sources, 1);
    }

    #[tokio::test]
    async fn source_reconcile_refuses_held_chunks() {
        // §F-02 adversarial replay: hold a chunk, then reconcile with an empty
        // live set — the sweep must refuse (409) and the chunk must survive.
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let (sid, cid) = seed_source_chunk(&state, "/v/legal.md", "vault", "held under hold");
        hold(&state.pool, &[cid]);

        let err = crate::handlers::sources::reconcile(
            State(state.clone()),
            OptPrincipal(None),
            Json(crate::handlers::sources::ReconcileRequest {
                kind: "vault".to_string(),
                live_uris: vec![],
                allow_empty: true,
            }),
        )
        .await
        .expect_err("a reconcile that would erase a held chunk must 409");
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.inner.code, "legal_hold_active");

        let (chunk, sstate): (i64, String) = state
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT (SELECT COUNT(*) FROM knowledge WHERE id = ?1),
                        (SELECT state FROM sources WHERE id = ?2)",
                rusqlite::params![cid, sid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((chunk, sstate.as_str()), (1, "active"), "erasure deferred");

        // Release the hold → the same reconcile retires the source.
        let mut conn = state.pool.get().unwrap();
        let tx = conn.transaction().unwrap();
        crate::legal_hold::release(&tx, 1, 62).unwrap();
        tx.commit().unwrap();
        let resp = crate::handlers::sources::reconcile(
            State(state.clone()),
            OptPrincipal(None),
            Json(crate::handlers::sources::ReconcileRequest {
                kind: "vault".to_string(),
                live_uris: vec![],
                allow_empty: true,
            }),
        )
        .await
        .expect("after release the reconcile completes");
        assert_eq!(resp.deleted_sources, 1);
        assert_eq!(resp.deleted_chunks, 1);
    }

    // ── the deletion certificate discloses the
    // honest physical-purge posture — secure_delete+checkpoint for a
    // strict-posture domain, the disclosed logical posture otherwise.

    #[tokio::test]
    async fn dsar_certificate_states_remanence_posture() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Strict-posture domain: bind a pii_mode=strict profile.
        register_client_fixture(&state, "beta", "beta-eu", "eu");
        seed_subject(&state, "beta-eu", "alice@beta");
        {
            let conn = state.registry.register("beta-eu").unwrap().get().unwrap();
            let p = brain_server::profile::Profile {
                name: "strict-holdall".into(),
                pii_mode: Some("strict".into()),
                ..Default::default()
            };
            brain_server::profile::upsert(&conn, &p).unwrap();
            brain_server::profile::bind(&conn, "beta-eu", Some("strict-holdall")).unwrap();
        }
        // Default-posture domain (no bound profile).
        register_client_fixture(&state, "acme", "acme-us", "us");
        seed_subject(&state, "acme-us", "bob@acme");

        let strict = crate::handlers::observe::run_dsar_subject(
            state.clone(),
            OptPrincipal(None),
            state.registry.register("beta-eu").unwrap(),
            "beta-eu",
            "alice@beta",
            "purge",
            false,
            Some("eu".to_string()),
            None,
            false,
            now,
        )
        .await
        .expect("strict dsar runs");
        assert_eq!(
            strict.certificate.as_ref().unwrap()["physical_purge"],
            "secure_delete+checkpoint (backup files excepted)",
            "a strict domain's certificate states the strict posture"
        );

        let logical = crate::handlers::observe::run_dsar_subject(
            state.clone(),
            OptPrincipal(None),
            state.registry.register("acme-us").unwrap(),
            "acme-us",
            "bob@acme",
            "purge",
            false,
            Some("us".to_string()),
            None,
            false,
            now + 1,
        )
        .await
        .expect("default dsar runs");
        assert_eq!(
            logical.certificate.as_ref().unwrap()["physical_purge"],
            "logical (secure_delete off; WAL/freelist/backup copies may persist)",
            "an unbound domain honestly discloses the logical posture"
        );

        // The strict domain's purge actually happened; the erasure completed.
        assert_eq!(count_knowledge(&state, "beta-eu"), 0);
        assert_eq!(count_knowledge(&state, "acme-us"), 0);
    }

    // ── releasing a legal hold unfreezes erasure
    // mid-litigation — the same DPO/admin dual gate a breach close carries.
    // A `dpo`-role principal releases; a role bundle without the dpo role OR
    // an admin capability is refused even when its SCOPES grant admin.

    fn gated_principal(roles: &[&str]) -> OptPrincipal {
        OptPrincipal(Some(crate::auth::Principal {
            sub: format!("{}@ops", roles.join("-")),
            tenant: "ops".to_string(),
            scopes: vec![crate::auth::Scope::parse("admin:ops/*").unwrap()],
            jti: "m3".to_string(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
            manages: vec![],
        }))
    }

    #[tokio::test]
    async fn hold_release_requires_dpo() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let id = seed_global_chunk(&state, "evidence in two holds");
        hold(&state.pool, &[id]);

        let release = |pid: i64, principal: OptPrincipal| {
            crate::handlers::holds::release_legal_hold(
                State(state.clone()),
                principal,
                Path(pid),
                axum::extract::Query(crate::handlers::holds::ReleaseQuery { domain: None }),
            )
        };

        let _ = release(1, gated_principal(&["dpo"]))
            .await
            .expect("a dpo-role principal releases the hold");
        let freed: i64 = state
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM legal_holds WHERE id = 1 AND released_at IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(freed, 1, "the dpo release sticks");

        hold(&state.pool, &[id]);
        let err = release(2, gated_principal(&["qa"]))
            .await
            .expect_err("a qa-role principal cannot unfreeze erasure");
        assert_eq!(err.status, StatusCode::FORBIDDEN);
        let still_held: i64 = state
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM legal_holds WHERE id = 2 AND released_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(still_held, 1, "the refused hold stays frozen");
    }

    #[tokio::test]
    async fn non_dpo_admin_cannot_release_hold() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let id = seed_global_chunk(&state, "the controller's contested evidence");
        hold(&state.pool, &[id]);

        // An admin-SCOPED principal whose resolved role (`controller`) has
        // purge but not the admin capability is refused by the dual gate —
        // scope grant alone is not a release.
        let err = crate::handlers::holds::release_legal_hold(
            State(state.clone()),
            gated_principal(&["controller"]),
            Path(1),
            axum::extract::Query(crate::handlers::holds::ReleaseQuery { domain: None }),
        )
        .await
        .expect_err("an admin-scoped controller cannot release a hold");
        assert_eq!(err.status, StatusCode::FORBIDDEN);

        // The `admin` role (its can list carries the admin capability) passes
        // the same gate — breach parity, verified by name.
        let resp = crate::handlers::holds::release_legal_hold(
            State(state.clone()),
            gated_principal(&["admin"]),
            Path(1),
            axum::extract::Query(crate::handlers::holds::ReleaseQuery { domain: None }),
        )
        .await
        .expect("the admin-capability role releases through the dual gate");
        assert!(resp["released"].as_bool().unwrap_or(false));
    }

    // ── the Art-30 register row + its audit row
    // are ONE transaction — a crash between commit and audit cannot leave an
    // unmirrored register entry (and a rollback erases both).

    #[tokio::test]
    async fn transfer_registration_audited_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);

        // Positive: the handler commits the register row AND its audit row
        // in one transaction.
        let resp = crate::handlers::transfers::register_transfer(
            State(state.clone()),
            OptPrincipal(None),
            Json(crate::handlers::transfers::TransferRequest {
                dataset: "hr".into(),
                origin_jurisdiction: "eu".into(),
                destination_jurisdiction: "us".into(),
                mechanism: "scc-eu-2021".into(),
                counterparty: "acme-us".into(),
                lawful_basis: Some("contract".into()),
                purpose: "payroll".into(),
                signed_at: None,
                expires_at: None,
            }),
        )
        .await
        .expect("transfer registers");
        let id = resp["id"].as_i64().unwrap();
        let (t, a): (i64, i64) = state
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT (SELECT COUNT(*) FROM transfers WHERE id = ?1),
                        (SELECT COUNT(*) FROM audit_events
                          WHERE target_hash = ?2)",
                rusqlite::params![id, crate::audit::hash(&format!("transfer_register:{id}"))],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((t, a), (1, 1), "register row + audit row commit together");

        // Crash-injection fixture: the SAME two writes inside one rolled-back
        // transaction → neither survives the rollback.
        {
            let mut conn = state.pool.get().unwrap();
            let tx = conn.transaction().unwrap();
            let id2 = crate::transfers::register(
                &tx,
                "hr",
                "eu",
                "us",
                "scc-eu-2021",
                "acme-us",
                Some("contract"),
                "payroll",
                None,
                None,
            )
            .unwrap();
            let _ = crate::audit::record(
                &tx,
                crate::audit::AuditKind::Transfer,
                "api",
                &format!("transfer_register:{id2}"),
                crate::audit::AuditStatus::Ok,
                "hr:eu->us:scc-eu-2021",
            );
            tx.rollback().unwrap();
            let (t2, a2): (i64, i64) = state
                .pool
                .get()
                .unwrap()
                .query_row(
                    "SELECT (SELECT COUNT(*) FROM transfers WHERE id = ?1),
                            (SELECT COUNT(*) FROM audit_events
                              WHERE target_hash = ?2)",
                    rusqlite::params![id2, crate::audit::hash(&format!("transfer_register:{id2}"))],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(
                (t2, a2),
                (0, 0),
                "a rollback erases BOTH the register row and its audit row"
            );
        }
    }
}
