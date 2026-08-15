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
        brain_server::register_sqlite_vec::register_sqlite_vec();
        let path = dir.path().join("brain.db");
        let mgr = r2d2_sqlite::SqliteConnectionManager::file(&path);
        let pool: crate::Pool = r2d2::Pool::builder().max_size(4).build(mgr).expect("pool");
        brain_server::migration::run_migration(
            &mut pool.get().unwrap(),
            crate::config::DB_MMAP_SIZE_MIB,
        )
        .expect("migration");
        let model: Arc<dyn brain_server::embed::Embedder> = Arc::new(
            brain_server::embed::StaticEmbedder::new(crate::config::MODEL_ID).expect("model"),
        );
        Arc::new(AppState {
            model,
            registry: DomainRegistry::new(pool.clone(), &path, true),
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
}
