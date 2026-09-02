//! the profile API.
//!
//! `GET /profiles` (list, seeded presets + operator clones), `GET
//! /profiles/{name}`, `POST /profiles/{name}` (clone/edit, Admin + audited),
//! and the domain binding `GET|POST /domains/{name}/profile` (bind, Admin +
//! audited; `{"profile": null}` unbinds — the back-compat escape hatch).
//!
//! Reads are `Read` at global scope (profiles are server config, not domain
//! data). The Health panel / wizard consume the binding view; `brain setup`
//! consumes the list.

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;

fn map_err(e: String) -> HandlerError {
    HandlerError::internal(format!("profile store: {e}"))
}

/// `GET /profiles` — every profile, name-ordered (the wizard's pick list).
pub async fn list_profiles(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let pool = state.pool.clone();
    let profiles = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        brain_server::profile::list(&conn).map_err(map_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(serde_json::json!({ "profiles": profiles })))
}

/// `GET /profiles/{name}` — one profile (404 when unknown).
pub async fn get_profile(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    if !brain_server::profile::is_valid_profile_name(&name) {
        return Err(HandlerError::bad_request(
            "profile_invalid",
            "profile name must be lowercase alnum + hyphen (max 63)",
        ));
    }
    let pool = state.pool.clone();
    let name_for_err = name.clone();
    let p = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        brain_server::profile::load(&conn, &name).map_err(map_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    match p {
        Some(p) => Ok(Json(serde_json::to_value(p).unwrap_or_default())),
        None => Err(HandlerError::not_found(format!(
            "no profile named '{name_for_err}'"
        ))),
    }
}

/// `POST /profiles/{name}` body — a Profile minus the name (the path is the
/// name; a `name` field in the body, if present, is ignored). Absent fields
/// mean "don't set that knob".
#[derive(Debug, Default, Deserialize)]
pub struct ProfileUpsertRequest {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default_access_scope: Option<String>,
    #[serde(default)]
    pub pii_mode: Option<String>,
    #[serde(default)]
    pub retention: Option<std::collections::BTreeMap<String, Option<i64>>>,
    #[serde(default)]
    pub audit_level: Option<String>,
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
    #[serde(default)]
    pub connectors_allowed: Option<Vec<String>>,
    #[serde(default)]
    pub legal_hold_default: Option<bool>,
}

/// `POST /profiles/{name}` — create or edit a profile (Admin + audited).
/// Editing a seeded preset by name overwrites it (they are starting points,
/// not locked) and the edit survives re-migrations (seeded via upsert).
pub async fn upsert_profile(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
    Json(req): Json<ProfileUpsertRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let p = brain_server::profile::Profile {
        name,
        description: req.description,
        default_access_scope: req.default_access_scope,
        pii_mode: req.pii_mode,
        retention: req.retention,
        audit_level: req.audit_level,
        kinds: req.kinds,
        connectors_allowed: req.connectors_allowed,
        legal_hold_default: req.legal_hold_default,
    };
    p.validate()
        .map_err(|e| HandlerError::bad_request("profile_invalid", e))?;
    let actor = super::recall::principal_label(&principal.0);
    let pool = state.pool.clone();
    let stored = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        brain_server::profile::upsert(&conn, &p).map_err(map_err)?;
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Reconcile,
            &actor,
            &format!("profile:{}", p.name),
            crate::audit::AuditStatus::Ok,
            "profile_upserted",
        );
        brain_server::profile::load(&conn, &p.name).map_err(map_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    match stored {
        Some(p) => Ok(Json(serde_json::to_value(p).unwrap_or_default())),
        None => Err(HandlerError::internal("profile vanished after upsert")),
    }
}

/// `GET /domains/{name}/profile` — the binding + the effective knobs the
/// Health panel renders. `profile: null` + `knobs: null` when unbound (the
/// server defaults apply — the back-compat posture).
pub async fn domain_profile_get(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let domain = super::normalize_domain(&name)?;
    let domain_for_resp = domain.clone();
    let pool = state.pool.clone();
    let bound = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        brain_server::profile::profile_for_domain(&conn, &domain).map_err(map_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    let resp = match bound {
        Some(p) => serde_json::json!({
            "domain": domain_for_resp,
            "profile": p.name,
            "knobs": {
                "default_access_scope": p.default_access_scope,
                "pii_mode": p.pii_mode,
                "retention": p.retention,
                "audit_level": p.audit_level,
                "kinds": p.kinds,
                "connectors_allowed": p.connectors_allowed,
                "legal_hold_default": p.legal_hold_default,
            },
            "effective": {
                // What retrieval will actually apply for this domain:
                // the profile's retention map (absent block = the server-wide
                // policy governs, reported as null to stay honest).
                "retention_days": p.retention_map(),
            },
        }),
        None => serde_json::json!({
            "domain": domain_for_resp,
            "profile": serde_json::Value::Null,
            "knobs": serde_json::Value::Null,
        }),
    };
    Ok(Json(resp))
}

/// `POST /domains/{name}/profile` body. `{"profile": "health-hipaa"}` binds;
/// `{"profile": null}` (or `{}`) unbinds — the back-compat escape hatch.
#[derive(Debug, Default, Deserialize)]
pub struct BindProfileRequest {
    #[serde(default)]
    pub profile: Option<String>,
}

/// `POST /domains/{name}/profile` — bind a domain to a profile (Admin +
/// audited). Takes effect at the next request (no restart, no re-ingest).
pub async fn domain_profile_bind(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
    Json(req): Json<BindProfileRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let domain = super::normalize_domain(&name)?;
    let profile = match req.profile.as_deref() {
        None => None,
        Some(p) if p.trim().is_empty() => None,
        Some(p) => Some(p.trim().to_string()),
    };
    if let Some(p) = &profile
        && !brain_server::profile::is_valid_profile_name(p)
    {
        return Err(HandlerError::bad_request(
            "profile_invalid",
            "profile name must be lowercase alnum + hyphen (max 63)",
        ));
    }
    let actor = super::recall::principal_label(&principal.0);
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        // Friendly 404 for an unknown profile (the FK would also refuse).
        if let Some(p) = &profile
            && brain_server::profile::load(&conn, p)
                .map_err(map_err)?
                .is_none()
        {
            return Err(HandlerError::not_found(format!("no profile named '{p}'")));
        }
        brain_server::profile::bind(&conn, &domain, profile.as_deref()).map_err(map_err)?;
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Reconcile,
            &actor,
            &format!("domain-profile:{domain}"),
            crate::audit::AuditStatus::Ok,
            &format!("domain_bound:{}", profile.as_deref().unwrap_or("(none)")),
        );
        Ok(serde_json::json!({
            "domain": domain,
            "profile": profile,
            "status": "ok",
        }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_request_is_profile_shaped() {
        // The body maps 1:1 onto the Profile knobs (minus name); serde must
        // accept the plan's health-hipaa example verbatim.
        let req: ProfileUpsertRequest = serde_json::from_str(
            r#"{"default_access_scope":"private","pii_mode":"strict",
                "retention":{"fact":null,"episodic":90,"procedure":null},
                "audit_level":"verbose","kinds":["fact","episodic","procedure","decision"],
                "connectors_allowed":["ehr-readonly"],"legal_hold_default":false}"#,
        )
        .expect("plan example parses");
        assert_eq!(req.pii_mode.as_deref(), Some("strict"));
        assert_eq!(
            req.retention.as_ref().unwrap().get("fact"),
            Some(&None),
            "explicit null retention = no decay"
        );
    }
}
