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
