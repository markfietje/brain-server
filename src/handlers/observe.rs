//! v1.15.0 "Observe" — observability + compliance-workflow handlers.
//!
//! M1/M2: the recall read-event trace endpoint (`/recall/{id}/trace`) replays
//! the decision path of a recorded read event (the audit row is hash-only; the
//! non-content trace metadata lives in `recall_traces`).
//! M3: DSAR orchestration on top of the v1.14 `/export` + `/purge` primitives —
//! `POST /dsar` (locate → export → purge → certificate), `GET /tombstones`
//! (the queryable deletion registry), `GET /dsar/{id}/certificate` (chain-
//! verifiable), and the opt-in Art 19 onward-notification webhook.
//!
//! Nothing here runs autonomously: a DSAR is an explicit operator action
//! (Admin gate), and the webhook is fire-and-forget fail-soft.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::handlers::auth::OptPrincipal;
use crate::handlers::{HandlerError, MAX_QUERY};
use crate::AppState;

/// Max derived_from walk depth for a DSAR purge. Derived chains are operator-
/// created and short (see `consolidate.rs`); a bounded walk keeps the tx small.
const DERIVED_MAX_DEPTH: usize = 8;
/// Max tombstone rows returned by `GET /tombstones` per page.
const MAX_TOMBSTONES: i64 = 1000;

// ---------------------------------------------------------------------------
// M2 — recall trace
// ---------------------------------------------------------------------------

/// `GET /recall/{trace_id}/trace` — replay the decision path of a recorded
/// recall read event: the exact chunks injected (id, score, assertion_kind,
/// source, relevance, decayed), the abstention decision, the access-scope
/// filter applied, the principal, the query, and the domains searched. Pure
/// read; no audit row of its own (avoid recursion).
pub async fn get_trace(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(trace_id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    // The decision-path viewer is operator surface: Admin. The trace holds
    // chunk ids + scores + the query — sensitive, never content.
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, Some("global"))?;
    let trace = tokio::task::spawn_blocking(move || -> Result<Option<String>, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        Ok(crate::audit::read_trace(&conn, trace_id))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    match trace {
        Some(t) => {
            let v: serde_json::Value = serde_json::from_str(&t)
                .map_err(|_| HandlerError::internal("stored trace is not valid JSON"))?;
            Ok(Json(v))
        }
        None => Err(HandlerError::not_found("no trace for this id")),
    }
}

// ---------------------------------------------------------------------------
// M3 — DSAR orchestration
// ---------------------------------------------------------------------------

/// `POST /dsar` request. `subject` is the owner/principal being actioned;
/// `action` is `export` | `purge` | `both`.
#[derive(Debug, Deserialize)]
pub struct DsarRequest {
    pub subject: String,
    #[serde(default = "default_dsar_action")]
    pub action: String,
}

fn default_dsar_action() -> String {
    "both".to_string()
}

/// `POST /dsar` response: the workflow row id + the deletion certificate.
#[derive(Debug, Serialize)]
pub struct DsarResponse {
    pub id: i64,
    pub subject: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate: Option<serde_json::Value>,
}

/// `POST /dsar` — the GDPR Art 15/17 workflow: locate every record for the
/// subject (content rows by `owner` + `derived_from` descendants), export the
/// bundle (portable JSON), purge (hard, reaching vec0 + graph + pointers),
/// tombstone, and return a chain-verifiable deletion certificate. One atomic
/// workflow: best-effort export, all-or-nothing purge tx.
pub async fn post_dsar(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<DsarRequest>,
) -> Result<Json<DsarResponse>, HandlerError> {
    let subject = req.subject.trim().to_string();
    if subject.is_empty() {
        return Err(HandlerError::bad_request(
            "subject_empty",
            "dsar subject must not be empty",
        ));
    }
    if subject.len() > MAX_QUERY {
        return Err(HandlerError::bad_request(
            "subject_too_long",
            format!("subject exceeds {MAX_QUERY} characters"),
        ));
    }
    if !matches!(req.action.as_str(), "export" | "purge" | "both") {
        return Err(HandlerError::bad_request(
            "invalid_action",
            "dsar action must be export|purge|both",
        ));
    }
    // Erasure is irreversible: Admin.
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, Some("global"))?;
    let action = req.action.clone();
    let action_did_purge = matches!(action.as_str(), "purge" | "both");

    let outcome = tokio::task::spawn_blocking(move || -> Result<DsarOutcome, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let now = chrono::Utc::now().timestamp();

        // 1. Locate: owner rows + transitive derived_from descendants.
        let (roots, derived) = dsar_locate(&tx, &subject)?;
        let found_count = roots.len() + derived.len();

        // 2. Export bundle (portable JSON; no pii_map — that is /export's job).
        let export_bundle = if matches!(action.as_str(), "export" | "both") {
            let mut rows: Vec<serde_json::Value> = Vec::new();
            let mut stmt = tx
                .prepare(
                    "SELECT id, content, node_kind, assertion_kind, confidence,
                            owner, observed_at, valid_from, valid_to
                     FROM knowledge WHERE id IN (?1)",
                )
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            for id in roots.iter().chain(derived.iter().map(|(d, _)| d)) {
                let q = stmt
                    .query_map(rusqlite::params![id], |row| {
                        Ok(serde_json::json!({
                            "id": row.get::<_, i64>(0)?,
                            "content": row.get::<_, String>(1)?,
                            "memory_kind": row.get::<_, String>(2)?,
                            "assertion_kind": row.get::<_, String>(3)?,
                            "confidence": row.get::<_, f32>(4)?,
                            "owner": row.get::<_, Option<String>>(5)?,
                            "observed_at": row.get::<_, Option<String>>(6)?,
                            "valid_from": row.get::<_, Option<String>>(7)?,
                            "valid_to": row.get::<_, Option<String>>(8)?,
                        }))
                    })
                    .map_err(|e| HandlerError::internal(e.to_string()))?;
                for v in q.flatten() {
                    rows.push(v);
                }
            }
            Some(
                serde_json::json!({
                    "exported_at": chrono::Utc::now().to_rfc3339(),
                    "subject": subject,
                    "knowledge": rows,
                })
                .to_string(),
            )
        } else {
            None
        };

        // 3. Purge (all-or-nothing with the export, same tx): roots with the
        //    owner reason, derived descendants with `derived` + origin id.
        let mut purged_ids: Vec<i64> = Vec::new();
        if matches!(action.as_str(), "purge" | "both") {
            for root in &roots {
                let closure: Vec<i64> = derived
                    .iter()
                    .filter(|(_, r)| r == root)
                    .map(|(d, _)| *d)
                    .collect();
                if !closure.is_empty() {
                    purged_ids.extend(closure.iter().copied());
                }
            }
            let _ = crate::handlers::gate::purge_chunk_ids(
                &tx,
                &roots,
                now,
                &format!("owner:{subject}"),
                None,
            )?;
            for root in &roots {
                let closure: Vec<i64> = derived
                    .iter()
                    .filter(|(_, r)| r == root)
                    .map(|(d, _)| *d)
                    .collect();
                if !closure.is_empty() {
                    let _ = crate::handlers::gate::purge_chunk_ids(
                        &tx,
                        &closure,
                        now,
                        "derived",
                        Some(*root),
                    )?;
                }
            }
            purged_ids.extend(roots.iter().copied());
        }

        // v1.16.1: DSAR-specific trace residue sweep. Beyond the hit-id
        // cascade in purge_chunk_ids, traces whose *query text* mentions the
        // subject (e.g. an email/name typed into recall) are personal data of
        // that subject too — the trace JSON stores the raw query. Sweep them
        // in the same tx, best-effort (short common subjects over-match
        // slightly; that is the erasure-safe direction).
        if matches!(action.as_str(), "purge" | "both") && !subject.is_empty() {
            let _ = tx.execute(
                "DELETE FROM recall_traces WHERE trace_json LIKE ?1",
                rusqlite::params![format!("%{}%", subject)],
            );
        }
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;

        // 4. Audit the workflow (hash-only), then capture the chain head so the
        //    certificate carries tamper-evidence at certification time.
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Reconcile,
            "api",
            &format!("dsar:{subject}"),
            crate::audit::AuditStatus::Ok,
            "dsar",
        );
        let chain_head = crate::audit::chain_head(&conn);
        let certified_at = chrono::Utc::now().to_rfc3339();
        let certificate = serde_json::json!({
            "subject": subject,
            "action": action,
            "found_count": found_count,
            "purged_ids": purged_ids,
            "tombstone_root": roots.first().copied(),
            "certified_at": certified_at,
            "chain_head": chain_head,
        })
        .to_string();

        // 5. Ledger row.
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO dsar_requests(subject, action, status, export_bundle, certificate, created_at, completed_at)
             VALUES (?1, ?2, 'completed', ?3, ?4, ?5, ?5)",
            rusqlite::params![subject, action, export_bundle, certificate, now],
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
        let id = conn.last_insert_rowid();

        Ok(DsarOutcome {
            id,
            subject,
            certificate,
            certified_at,
        })
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    // 6. Art 19 onward-notification: opt-in, fire-and-forget, fail-soft.
    if action_did_purge {
        notify_art19(
            outcome.subject.clone(),
            outcome.id,
            outcome.certified_at.clone(),
        );
    }

    let cert: serde_json::Value = serde_json::from_str(&outcome.certificate)
        .map_err(|_| HandlerError::internal("stored certificate is not valid JSON"))?;
    Ok(Json(DsarResponse {
        id: outcome.id,
        subject: outcome.subject,
        status: "completed",
        certificate: Some(cert),
    }))
}

struct DsarOutcome {
    id: i64,
    subject: String,
    certificate: String,
    certified_at: String,
}

/// `GET /tombstones?subject=&since=&limit=` — the queryable deletion registry
/// (the EDPB Coordinated Enforcement Framework ask). Hash-only, append-only
/// rows. `subject` filters by the `owner:<subject>` purge reason; `since`
/// filters by `purged_at`; `limit` caps the page (default 100, clamped to
/// `MAX_TOMBSTONES`). `?principal=` is reserved (tombstones are tenant-global).
#[derive(Debug, Default, Deserialize)]
pub struct TombstonesQuery {
    pub subject: Option<String>,
    pub since: Option<i64>,
    pub limit: Option<i64>,
}

pub async fn list_tombstones(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<TombstonesQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, Some("global"))?;
    let subject = q.subject.clone();
    let since = q.since;
    let limit = q.limit.map(|l| l.clamp(1, MAX_TOMBSTONES)).unwrap_or(100);
    let body = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let mut sql = String::from(
            "SELECT knowledge_id, content_hash, purged_at, reason, origin_id \
               FROM tombstones",
        );
        let mut clauses: Vec<String> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(s) = &subject {
            clauses.push("reason = ?".to_string());
            params.push(Box::new(format!("owner:{s}")));
        }
        if let Some(t) = since {
            clauses.push("purged_at >= ?".to_string());
            params.push(Box::new(t));
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY purged_at DESC LIMIT ?");
        params.push(Box::new(limit));
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), |r| {
                Ok(serde_json::json!({
                    "knowledge_id": r.get::<_, i64>(0)?,
                    "content_hash": r.get::<_, Option<String>>(1)?,
                    "purged_at": r.get::<_, Option<i64>>(2)?,
                    "reason": r.get::<_, Option<String>>(3)?,
                    "origin_id": r.get::<_, Option<i64>>(4)?,
                }))
            })
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let mut out = Vec::new();
        for v in rows.flatten() {
            out.push(v);
        }
        Ok(serde_json::json!({ "tombstones": out }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(body))
}

/// `GET /dsar/{id}/certificate` — re-fetch a past deletion certificate. The
/// stored `chain_head` is the audit-chain link at certification time; the
/// response recomputes `verify_chain` live so the caller sees whether the
/// chain the certificate anchored to still holds.
pub async fn get_dsar_certificate(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, Some("global"))?;
    let body = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let cert: Option<String> = conn
            .query_row(
                "SELECT certificate FROM dsar_requests WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .ok();
        match cert {
            Some(c) => {
                let v: serde_json::Value = serde_json::from_str(&c)
                    .map_err(|_| HandlerError::internal("stored certificate is not valid JSON"))?;
                Ok(serde_json::json!({
                    "certificate": v,
                    "chain_verifies": crate::audit::verify_chain(&conn),
                }))
            }
            None => Err(HandlerError::not_found("no dsar request with this id")),
        }
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(body))
}

// ---------------------------------------------------------------------------
// Art 19 webhook (opt-in) + shared helpers
// ---------------------------------------------------------------------------

/// v1.15.0 "Observe" M3: Art 19 onward-notification. When
/// `BRAIN_DSAR_WEBHOOK_URL` is set, a completed DSAR purge POSTs
/// `{subject, certified_at, certificate_id}` to the URL, HMAC-SHA256-signed
/// (`X-Brain-Signature-256: sha256=<hex>`) when `BRAIN_DSAR_WEBHOOK_SECRET`
/// is set. Fail-soft: bounded retries, then a logged warning — a webhook
/// failure NEVER rolls back the purge.
pub(crate) fn notify_art19(subject: String, certificate_id: i64, certified_at: String) {
    let Some(url) = crate::config::dsar_webhook_url() else {
        return;
    };
    let payload = serde_json::json!({
        "subject": subject,
        "certified_at": certified_at,
        "certificate_id": certificate_id,
    })
    .to_string();
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut last_err: Option<String> = None;
        for attempt in 0..3u32 {
            let mut req = client.post(&url).header("content-type", "application/json");
            if let Some(secret) = crate::config::dsar_webhook_secret() {
                let sig = hmac_hex(secret.as_bytes(), payload.as_bytes());
                req = req.header("x-brain-signature-256", format!("sha256={sig}"));
            }
            match req.body(payload.clone()).send().await {
                Ok(r) if r.status().is_success() => return,
                Ok(r) => last_err = Some(format!("http {}", r.status())),
                Err(e) => last_err = Some(e.to_string()),
            }
            tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt))).await;
        }
        tracing::warn!("DSAR Art 19 webhook failed after retries: {last_err:?}");
    });
}

/// HMAC-SHA256 hex signature (the same scheme `webhook.rs` verifies for
/// inbound GitHub webhooks — the DSAR webhook is the outbound mirror).
fn hmac_hex(secret: &[u8], body: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key is any length");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Collect all rows of `SELECT <i64>` sql (one `?` param) into a Vec.
fn collect_ids(
    tx: &rusqlite::Transaction,
    sql: &str,
    param: &str,
) -> Result<Vec<i64>, HandlerError> {
    let mut stmt = tx
        .prepare(sql)
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params![param], |r| r.get::<_, i64>(0))
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    Ok(rows.flatten().collect())
}

/// Locate every record for a DSAR subject: content rows by `owner`, plus all
/// transitive `derived_from` descendants (bounded by `DERIVED_MAX_DEPTH`).
/// Returns `(root_ids, derived_pairs)` where each derived pair is
/// `(derived_id, root_id)` — the purge stamps `origin_id` so the deletion
/// registry can point a derived chunk back at the subject's root record.
#[allow(clippy::type_complexity)]
pub(crate) fn dsar_locate(
    tx: &rusqlite::Transaction,
    subject: &str,
) -> Result<(Vec<i64>, Vec<(i64, i64)>), HandlerError> {
    let roots: Vec<i64> = collect_ids(tx, "SELECT id FROM knowledge WHERE owner = ?1", subject)?;
    let mut derived: Vec<(i64, i64)> = Vec::new(); // (derived_id, root_id)
    let mut seen: std::collections::HashSet<i64> = roots.iter().copied().collect();
    let mut frontier: Vec<i64> = roots.clone();
    for _ in 0..DERIVED_MAX_DEPTH {
        if frontier.is_empty() {
            break;
        }
        let placeholders = vec!["?"; frontier.len()].join(",");
        let sql = format!(
            "SELECT el.to_chunk FROM evidence_links el
             WHERE el.kind = 'derived_from' AND el.from_chunk IN ({placeholders})"
        );
        let mut stmt = tx
            .prepare(&sql)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let mut next: Vec<i64> = Vec::new();
        {
            let params: Vec<&dyn rusqlite::ToSql> =
                frontier.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
            let rows = stmt
                .query_map(params.as_slice(), |r| r.get::<_, i64>(0))
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            for v in rows.flatten() {
                if seen.insert(v) {
                    next.push(v);
                    derived.push((v, roots[0]));
                }
            }
        }
        frontier = next;
    }
    Ok((roots, derived))
}
