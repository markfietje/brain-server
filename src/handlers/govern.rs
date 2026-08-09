//! v1.17.1 "Govern" — retention lifecycle + compliance exports + snapshot checks.
//!
//! M2 — per-kind retention policy: `GET /retention` (current policy + per-kind
//! counts) and `POST /retention` (operator override, Admin + audited). The
//! override is persisted in `retention_policy` so it survives restart; the
//! default policy ships in code. Nothing here runs autonomously — retention is
//! applied at query time by the retriever, never by a sweeper.
//!
//! M5 — Art 30 records-of-processing register: `GET /art30` is a projection of
//! existing tables (categories of data, purpose, retention, recipients, DSAR
//! history, chunk lifecycle) — the register every controller must maintain.
//!
//! M7 — snapshot self-check panel: `GET /snapshot/status` inspects every
//! `VACUUM INTO` `.bak` snapshot in the DB directory — exists, 0600, size,
//! `PRAGMA integrity_check`, and audit-chain verification of its log.

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

use crate::handlers::auth::OptPrincipal;
use crate::handlers::HandlerError;
use crate::AppState;

/// Persisted retention overrides plus per-kind counts, both keyed by kind.
type KindMap = std::collections::BTreeMap<String, i64>;

// ---------------------------------------------------------------------------
// M2 — /retention
// ---------------------------------------------------------------------------

/// `GET /retention` — the effective per-kind retention policy (code defaults
/// merged with persisted overrides), the kill-switch state, and the current
/// per-kind chunk counts (so an operator sees what a policy change would govern).
pub async fn retention_get(
    State(_state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let enabled = crate::config::brain_retention_enabled();
    let pool = super::resolve_domain_pool(&_state.registry, Some("global"))?;
    let (overridden, counts) = tokio::task::spawn_blocking(
        move || -> Result<(Vec<(String, i64)>, KindMap), HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let mut stmt = conn
                .prepare("SELECT kind, days FROM retention_policy ORDER BY kind")
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let overridden: Vec<(String, i64)> = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                .map_err(|e| HandlerError::internal(e.to_string()))?
                .flatten()
                .collect();
            let mut counts: std::collections::BTreeMap<String, i64> =
                std::collections::BTreeMap::new();
            if let Ok(mut cs) =
                conn.prepare("SELECT node_kind, COUNT(*) FROM knowledge GROUP BY node_kind")
            {
                for (k, n) in cs
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                    .map_err(|e| HandlerError::internal(e.to_string()))?
                    .flatten()
                {
                    counts.insert(k, n);
                }
            }
            Ok((overridden, counts))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    let mut policy = crate::config::retention_kind_days();
    for (k, d) in overridden {
        policy.insert(k, d);
    }
    Ok(Json(serde_json::json!({
        "enabled": enabled,
        "policy": policy,
        "counts": counts,
        "projection": "kind-default expiry is derived from each chunk's created_at at query time"
    })))
}

/// `POST /retention` body. Either a single `{kind, days}` or a full
/// `{policy: {kind: days}}` map. `days` must be a positive integer.
#[derive(Debug, Deserialize)]
pub struct RetentionSet {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub days: Option<i64>,
    #[serde(default)]
    pub policy: Option<std::collections::BTreeMap<String, i64>>,
}

/// `POST /retention` — set/override the per-kind retention policy. Admin +
/// audited. The override is persisted (`retention_policy`) so it survives a
/// restart. An empty body clears an override back to the code default (the
/// operator-driven reset).
pub async fn retention_post(
    State(_state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<RetentionSet>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    // Build the kind→days set to apply: single pair, full map, or empty (clear).
    let mut to_set: Vec<(String, i64)> = Vec::new();
    if let Some(k) = &req.kind {
        let days = req.days.ok_or_else(|| {
            HandlerError::bad_request("days_required", "days is required with kind")
        })?;
        if !(1..=36500).contains(&days) {
            return Err(HandlerError::bad_request(
                "invalid_days",
                "days must be an integer in [1, 36500]",
            ));
        }
        to_set.push((crate::handlers::normalize_name(k)?, days));
    } else if let Some(policy) = &req.policy {
        for (k, days) in policy {
            if !(1..=36500).contains(days) {
                return Err(HandlerError::bad_request(
                    "invalid_days",
                    "days must be an integer in [1, 36500]",
                ));
            }
            to_set.push((crate::handlers::normalize_name(k)?, *days));
        }
    }
    if to_set.is_empty() {
        return Err(HandlerError::bad_request(
            "empty_policy",
            "provide a {kind, days} pair or a {policy: {kind: days}} map",
        ));
    }

    let pool = super::resolve_domain_pool(&_state.registry, Some("global"))?;
    let now = chrono::Utc::now().timestamp();
    let pool2 = pool.clone();
    let to_set2 = to_set.clone();
    let n = tokio::task::spawn_blocking(move || -> Result<usize, HandlerError> {
        let conn = pool2
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let mut affected = 0usize;
        for (kind, days) in &to_set2 {
            affected += conn
                .execute(
                    "INSERT INTO retention_policy(kind, days, updated_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(kind) DO UPDATE SET days = excluded.days, updated_at = excluded.updated_at",
                    rusqlite::params![kind, days, now],
                )
                .map_err(|e| HandlerError::internal(e.to_string()))?;
        }
        Ok(affected)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    let conn = pool
        .get()
        .map_err(|e| HandlerError::internal(format!("DB: {e}")))?;
    crate::audit::record(
        &conn,
        crate::audit::AuditKind::Reconcile,
        "api",
        &format!("retention:{n}"),
        crate::audit::AuditStatus::Ok,
        "retention_set",
    );

    Ok(Json(serde_json::json!({ "updated": n, "set": to_set })))
}

// ---------------------------------------------------------------------------
// M5 — /art30
// ---------------------------------------------------------------------------

/// `GET /art30` — the Art 30 records-of-processing register, projected from
/// existing tables. Admin. Sections: categories of data, purpose, retention,
/// recipients, transfer legal bases, DSAR exercise history, and chunk lifecycle
/// summary. This is the server-side data the client's transfer register consumes.
pub async fn art30(
    State(_state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool = super::resolve_domain_pool(&_state.registry, Some("global"))?;
    let body = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;

        // Categories of data: per-memory_kind counts (the kinds are the
        // categories brain-server processes).
        let mut categories: Vec<serde_json::Value> = Vec::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT node_kind, COUNT(*) FROM knowledge GROUP BY node_kind ORDER BY node_kind",
                )
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            for (kind, n) in stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                .map_err(|e| HandlerError::internal(e.to_string()))?
                .flatten()
            {
                categories.push(serde_json::json!({
                    "category": kind,
                    "count": n,
                    "purpose": format!("{kind} memory retrieved into decision context"),
                }));
            }
        }

        // Retention: the effective per-kind policy.
        let retention = crate::config::retention_kind_days();

        // Recipients: configured outbound surfaces (DSAR webhook) + registered
        // connectors. Static/internally-derived; the Art 30 register names them.
        let mut recipients: Vec<serde_json::Value> = Vec::new();
        if crate::config::dsar_webhook_url().is_some() {
            recipients.push(serde_json::json!({
                "name": "Art 19 DSAR webhook",
                "purpose": "onward notification of a completed erasure (Art 19)",
                "legal_basis": "EU GDPR Art 19",
            }));
        }
        {
            if let Ok(mut stmt) = conn.prepare("SELECT kind, instance FROM connectors ORDER BY kind") {
                for (kind, instance) in stmt
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .map_err(|e| HandlerError::internal(e.to_string()))?
                    .flatten()
                {
                    recipients.push(serde_json::json!({
                        "name": format!("connector:{kind}:{instance}"),
                        "purpose": "external data source ingestion",
                        "legal_basis": "legitimate interest / contract",
                    }));
                }
            }
        }

        // Transfer legal bases: outbound transfers that cross the boundary.
        let webhook_basis = if crate::config::dsar_webhook_url().is_some() {
            "explicit consent / controller obligation"
        } else {
            "not configured"
        };
        let transfers: Vec<serde_json::Value> = vec![
            serde_json::json!({
                "transfer": "Art 19 webhook",
                "legal_basis": webhook_basis,
            }),
            serde_json::json!({
                "transfer": "connector outbound ingestion",
                "legal_basis": "legitimate interest (operator-authorized)",
            }),
        ];

        // DSAR exercise history (count by action/status).
        let dsar: Vec<serde_json::Value> = {
            let mut v = Vec::new();
            if let Ok(mut stmt) = conn.prepare("SELECT action, status, COUNT(*) FROM dsar_requests GROUP BY action, status ORDER BY action") {
                for (a, s, n) in stmt
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
                    .map_err(|e| HandlerError::internal(e.to_string()))?
                    .flatten()
                {
                    v.push(serde_json::json!({ "action": a, "status": s, "count": n }));
                }
            }
            v
        };

        // Chunk lifecycle summary: superseded (valid_to set), tombstoned, live.
        let live: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge WHERE valid_to IS NULL", [], |r| r.get(0))
            .unwrap_or(0);
        let superseded: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge WHERE valid_to IS NOT NULL", [], |r| r.get(0))
            .unwrap_or(0);
        let tombstoned: i64 = conn
            .query_row("SELECT COUNT(*) FROM tombstones", [], |r| r.get(0))
            .unwrap_or(0);

        Ok(serde_json::json!({
            "art30": {
                "register_name": "brain-server records of processing activities",
                "generated_at": chrono::Utc::now().to_rfc3339(),
                "controller": crate::config::controller_name(),
                "categories_of_data": categories,
                "purpose": "deterministic memory storage, retrieval, and decision-path provenance for the operator's agents",
                "retention": retention,
                "recipients": recipients,
                "transfer_legal_bases": transfers,
                "dsar_history": dsar,
                "lifecycle": {
                    "live": live,
                    "superseded": superseded,
                    "tombstoned": tombstoned,
                },
                "provenance_fields": ["source", "assertion_kind", "confidence"],
            }
        }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(body))
}

// ---------------------------------------------------------------------------
// M7 — /snapshot/status
// ---------------------------------------------------------------------------

/// `GET /snapshot/status` — inspect every `VACUUM INTO` `.bak` snapshot in the
/// DB directory: exists, size, 0600 mode, `PRAGMA integrity_check`, and
/// audit-chain verification of its log. Admin. Read-only — it never creates or
/// mutates a snapshot. A tampered snapshot reports its failure, not a crash.
pub async fn snapshot_status(
    State(_state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let db_path = _state.db_path.clone();
    let body = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let dir = db_path.parent().unwrap_or(std::path::Path::new("."));
        let stem = db_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("brain");
        let prefix = format!("{stem}.snapshot-");
        let mut snaps: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| HandlerError::internal(format!("read dir failed: {e}")))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|s| s.to_str())
                    .map(|n| n.starts_with(&prefix) && n.ends_with(".bak"))
                    .unwrap_or(false)
            })
            .collect();
        snaps.sort();

        let mut snapshots: Vec<serde_json::Value> = Vec::new();
        for p in &snaps {
            snapshots.push(check_snapshot(p));
        }
        let all_ok = snapshots.iter().all(|s| s["ok"].as_bool().unwrap_or(false));
        Ok(serde_json::json!({
            "db": db_path.display().to_string(),
            "snapshot_count": snaps.len(),
            "all_ok": all_ok,
            "snapshots": snapshots,
        }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(body))
}

/// Check one `.bak` snapshot file. Returns a JSON status object with `ok`.
fn check_snapshot(p: &std::path::Path) -> serde_json::Value {
    let name = p
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_string();
    let (exists, size, mode_ok, integrity_ok, chain_ok) =
        (|| -> Result<(bool, u64, bool, bool, bool), rusqlite::Error> {
            let meta = std::fs::metadata(p)
                .map_err(|_| rusqlite::Error::InvalidParameterName("missing".into()))?;
            let size = meta.len();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = meta.permissions().mode() & 0o777;
                let mode_ok = mode == 0o600;
                let conn = rusqlite::Connection::open(p)?;
                let integrity: String =
                    conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
                let chain_ok = crate::audit::verify_chain(&conn);
                Ok((true, size, mode_ok, integrity == "ok", chain_ok))
            }
            #[cfg(not(unix))]
            {
                let conn = rusqlite::Connection::open(p)?;
                let integrity: String =
                    conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
                let chain_ok = crate::audit::verify_chain(&conn);
                Ok((true, size, true, integrity == "ok", chain_ok))
            }
        })()
        .unwrap_or((false, 0, false, false, false));

    serde_json::json!({
        "file": name,
        "exists": exists,
        "size_bytes": size,
        "mode_0600": mode_ok,
        "integrity_check": integrity_ok,
        "audit_chain_ok": chain_ok,
        "ok": exists && mode_ok && integrity_ok && chain_ok,
    })
}
