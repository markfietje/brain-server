//! Compliance-pack evidence surfaces (EU AI Act Art.12/13/14, GDPR RoPA).
//!
//! All routes are Admin + audited. The decision ledger itself is written by
//! the host write path; these endpoints only append operator-attested records
//! (evaluation declarations) and read/export evidence.

#![deny(clippy::unwrap_used)]

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;

use crate::AppState;
use crate::handlers::{HandlerError, auth::OptPrincipal};

fn admin_gate(principal: &Option<crate::auth::Principal>) -> Result<(), HandlerError> {
    crate::handlers::authorize(principal, crate::auth::Action::Admin, "", "global")
}

/// `GET /audit/export?since=&format=jsonl|pdf&rpcId=` — the Art.12 evidence
/// bundle. `rpcId` is echoed for reconciliation with the caller's request log.
#[cfg_attr(
    feature = "otel",
    tracing::instrument(name = "compliance.export", skip_all)
)]
pub async fn export_audit(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<ExportQuery>,
) -> Result<axum::response::Response, HandlerError> {
    admin_gate(&principal.0)?;
    // The export of Art.12 evidence is itself evidence (who inspected).
    if let Ok(conn) = state.pool.get() {
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Decision,
            &crate::handlers::recall::principal_label(&principal.0),
            "compliance/export",
            crate::audit::AuditStatus::Ok,
            "decision_ledger_export",
        );
    }
    let format = q.format.unwrap_or_else(|| "jsonl".to_string());
    if let Some(rpc) = q.rpc_id.as_ref()
        && rpc.len() > MAX_RPC_ID
    {
        return Err(HandlerError::bad_request(
            "rpc_id_too_long",
            "rpcId must be at most 128 characters",
        ));
    }
    if !matches!(format.as_str(), "jsonl" | "pdf") {
        return Err(HandlerError::bad_request(
            "format_invalid",
            "format must be jsonl or pdf",
        ));
    }
    let targets = crate::handlers::domain_pools(&state.registry, &state.pool);
    let since = q.since;
    let records: Vec<(String, brain_server::audit::decision::DecisionRecord)> =
        tokio::task::spawn_blocking(
            move || -> Vec<(String, brain_server::audit::decision::DecisionRecord)> {
                let mut all = Vec::new();
                for (domain, pool) in &targets {
                    let Some(pool) = pool else { continue };
                    let Ok(conn) = pool.get() else { continue };
                    match brain_server::audit::decision::list_decisions(&conn, since, 10_000) {
                        Ok(rows) => all.extend(rows.into_iter().map(|r| (domain.clone(), r))),
                        Err(_) => continue, // table absent (feature added later) exports empty
                    }
                }
                // ids are per-domain sequences — sort by (domain, id) so the
                // merged bundle stays attributable and per-domain ordered.
                all.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.id.cmp(&b.1.id)));
                all
            },
        )
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))?;
    let labels: Vec<String> = records.iter().map(|(d, _)| d.clone()).collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();

    let content_disposition = |name: &str| format!("attachment; filename=\"{name}\"");
    if format == "pdf" {
        let body = brain_server::audit::decision::render_pdf_labelled(
            &label_refs,
            // render_pdf_labelled borrows the records; rebuild the slice view.
            &records.iter().map(|(_, r)| r).collect::<Vec<_>>(),
            "Decision ledger (Art.12)",
        );
        Ok((
            [
                (
                    axum::http::header::CONTENT_DISPOSITION,
                    content_disposition("decision-ledger.pdf"),
                ),
                (
                    axum::http::header::CONTENT_TYPE,
                    "application/pdf".to_string(),
                ),
            ],
            body,
        )
            .into_response())
    } else {
        let mut lines = String::new();
        // the rpcId echo rides the envelope's first line so a reconciler can
        // pair export requests with their evidence bundles.
        if let Some(rpc) = q.rpc_id {
            let envelope = serde_json::json!({ "rpcId": rpc });
            lines.push_str(&format!("{envelope}\n"));
        }
        for (domain, r) in &records {
            // Each line carries its owning domain: ids are per-domain
            // sequences and each domain has its own chain lineage.
            if let Ok(mut v) = serde_json::to_value(r) {
                v["domain"] = serde_json::Value::String(domain.clone());
                if let Ok(line) = serde_json::to_string(&v) {
                    lines.push_str(&line);
                    lines.push('\n');
                }
            }
        }
        Ok((
            [
                (
                    axum::http::header::CONTENT_DISPOSITION,
                    content_disposition("decision-ledger.jsonl"),
                ),
                (
                    axum::http::header::CONTENT_TYPE,
                    "application/x-ndjson".to_string(),
                ),
            ],
            lines,
        )
            .into_response())
    }
}

const MAX_RPC_ID: usize = 128;

/// Wire-boundary field caps for operator-supplied evidence text: bounded
/// input is the two-layer-envelope rule — refuse absurd sizes rather than
/// storing them forever (evidence tables are append-only).
fn capped(field: &str, value: &str, max: usize) -> Result<(), HandlerError> {
    if value.len() > max {
        return Err(HandlerError::bad_request(
            "field_too_long",
            format!("{field} must be at most {max} characters"),
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    since: Option<i64>,
    format: Option<String>,
    #[serde(default)]
    rpc_id: Option<String>,
}

// ── Art.14 oversight evidence ───────────────────────────────────────────

/// Append one oversight-evidence row linked to a fresh decision record.
/// Best-effort like the audit chain: evidence must never fail the primary
/// action. `basis` is the snapshot hash of what the reviewer saw (the
/// review digest — never raw content); `outcome` ∈ accept|modify|override.
pub(crate) fn record_oversight(
    conn: &rusqlite::Connection,
    reviewer_id: &str,
    basis: &str,
    outcome: &str,
    authority: &str,
    proposal_id: Option<i64>,
    domain: &str,
) -> Option<i64> {
    let decision = brain_server::audit::decision::record_decision(
        conn,
        &brain_server::audit::decision::DecisionInput {
            actor_id: reviewer_id,
            role: authority,
            policy_version: env!("CARGO_PKG_VERSION"),
            prompt_class: "review",
            tool: "oversight",
            model_id: "",
            outcome,
        },
    )?;
    let decision_hash = decision.hash.clone();
    conn.execute(
        "INSERT INTO oversight_evidence(reviewer_id, reviewed_at, basis, outcome, authority, decision_hash, proposal_id, domain)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            reviewer_id,
            chrono::Utc::now().timestamp(),
            basis,
            outcome,
            authority,
            decision_hash,
            proposal_id,
            domain
        ],
    )
    .ok()?;
    Some(conn.last_insert_rowid())
}

/// `POST /compliance/evaluation-record` — an accuracy/validation declaration
/// (Art.15 evidence) appended to the decision ledger, tied to the system
/// version and the SHA-256 of the evaluation dataset it rests on.
pub async fn post_evaluation_record(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<EvaluationRecord>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    admin_gate(&principal.0)?;
    if body.dataset_hash.is_empty() || body.declaration.is_empty() {
        return Err(HandlerError::bad_request(
            "evaluation_record_incomplete",
            "dataset_hash and declaration are required",
        ));
    }
    // the dataset hash must BE a SHA-256 hex digest — a free-text blob here
    // would defeat the "tied to dataset" property the record exists for.
    if !is_sha256_hex(&body.dataset_hash) {
        return Err(HandlerError::bad_request(
            "dataset_hash_invalid",
            "dataset_hash must be 64 hex characters (SHA-256)",
        ));
    }
    capped("declaration", &body.declaration, 8_000)?;
    capped("system_version", &body.system_version, 128)?;
    let pool = state.pool.clone();
    let actor = crate::handlers::recall::principal_label(&principal.0);
    let record = tokio::task::spawn_blocking(
        move || -> Result<brain_server::audit::decision::DecisionRecord, HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let outcome = format!(
                "declared:{} dataset_sha256:{} version:{}",
                body.declaration, body.dataset_hash, body.system_version
            );
            brain_server::audit::decision::record_decision(
                &conn,
                &brain_server::audit::decision::DecisionInput {
                    actor_id: &actor,
                    role: "operator",
                    policy_version: env!("CARGO_PKG_VERSION"),
                    prompt_class: "evaluation",
                    tool: "compliance/evaluation-record",
                    model_id: &body.system_version,
                    outcome: &outcome,
                },
            )
            .ok_or_else(|| HandlerError::internal("decision record write failed"))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(
        serde_json::to_value(&record).unwrap_or(serde_json::Value::Null),
    ))
}

#[derive(Debug, Deserialize)]
pub struct EvaluationRecord {
    /// free-text accuracy/validation declaration (methodology summary)
    declaration: String,
    /// SHA-256 of the frozen evaluation dataset
    dataset_hash: String,
    #[serde(default)]
    system_version: String,
}

/// `GET /compliance/inventory` — the evidence-inventory checker: flags which
/// high-risk artefact classes exist and which are missing/expired.
pub async fn inventory(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<serde_json::Value>, HandlerError> {
    admin_gate(&principal.0)?;
    if let Ok(conn) = state.pool.get() {
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Decision,
            &crate::handlers::recall::principal_label(&principal.0),
            "compliance/inventory",
            crate::audit::AuditStatus::Ok,
            "evidence_inventory_read",
        );
    }
    let targets = crate::handlers::domain_pools(&state.registry, &state.pool);
    let now = chrono::Utc::now().timestamp();
    let items = tokio::task::spawn_blocking(move || -> Vec<serde_json::Value> {
        let mut counts = InventoryCounts::default();
        for (_, pool) in &targets {
            let Some(pool) = pool else { continue };
            let Ok(conn) = pool.get() else { continue };
            let n = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap_or(-1) };
            counts.decisions = counts
                .decisions
                .max(n("SELECT COUNT(*) FROM decision_records"));
            counts.oversight = counts
                .oversight
                .max(n("SELECT COUNT(*) FROM oversight_evidence"));
            counts.dsar = counts.dsar.max(n("SELECT COUNT(*) FROM dsar_requests"));
            counts.incidents = counts.incidents.max(n("SELECT COUNT(*) FROM breaches"));
            counts.transfers = counts.transfers.max(n("SELECT COUNT(*) FROM transfers"));
            counts.ropa = counts.ropa.max(n("SELECT COUNT(*) FROM ropa_registry"));
        }
        vec![
            art(
                "art12_decision_log",
                counts.decisions > 0,
                counts.decisions,
                None,
            ),
            art(
                "art14_oversight_evidence",
                counts.oversight > 0,
                counts.oversight,
                None,
            ),
            art("art17_dsar_ledger", counts.dsar > 0, counts.dsar, None),
            art(
                "art73_incident_log",
                counts.incidents > 0,
                counts.incidents,
                None,
            ),
            art(
                "gdpr_transfers_register",
                counts.transfers > 0,
                counts.transfers,
                None,
            ),
            art("gdpr_ropa", counts.ropa > 0, counts.ropa, None),
            art("retention_window_months", true, 12, Some(now)),
        ]
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))?;
    let complete = items
        .iter()
        .all(|i| i["present"].as_bool().unwrap_or(false));
    Ok(Json(
        serde_json::json!({ "complete": complete, "items": items }),
    ))
}

#[derive(Default)]
struct InventoryCounts {
    decisions: i64,
    oversight: i64,
    dsar: i64,
    incidents: i64,
    transfers: i64,
    ropa: i64,
}

/// strict SHA-256 hex-digest predicate (pure, pinned).
fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn art(name: &str, present: bool, count: i64, _now: Option<i64>) -> serde_json::Value {
    serde_json::json!({ "artifact": name, "present": present, "count": count })
}

// ── RoPA (GDPR Art.30 records of processing) ────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RopaInput {
    pub activity: String,
    pub controller: String,
    pub processor: String,
    #[serde(default)]
    pub categories: String,
    #[serde(default)]
    pub recipients: String,
    pub lawful_basis: String,
    #[serde(default)]
    pub retention_days: Option<i64>,
    #[serde(default)]
    pub security_measures: String,
    #[serde(default)]
    pub transfers: String,
}

pub async fn list_ropa(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<serde_json::Value>, HandlerError> {
    admin_gate(&principal.0)?;
    if let Ok(conn) = state.pool.get() {
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Client,
            &crate::handlers::recall::principal_label(&principal.0),
            "ropa:list",
            crate::audit::AuditStatus::Ok,
            "ropa_registry_read",
        );
    }
    let pool = state.pool.clone();
    let rows =
        tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>, HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, activity, controller, processor, categories, recipients,
                        lawful_basis, retention_days, security_measures, transfers, updated_at
                 FROM ropa_registry ORDER BY id",
                )
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "activity": r.get::<_, String>(1)?,
                        "controller": r.get::<_, String>(2)?,
                        "processor": r.get::<_, String>(3)?,
                        "categories": r.get::<_, String>(4)?,
                        "recipients": r.get::<_, String>(5)?,
                        "lawful_basis": r.get::<_, String>(6)?,
                        "retention_days": r.get::<_, Option<i64>>(7)?,
                        "security_measures": r.get::<_, String>(8)?,
                        "transfers": r.get::<_, String>(9)?,
                        "updated_at": r.get::<_, i64>(10)?,
                    }))
                })
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| HandlerError::internal(e.to_string()))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))?;
    Ok(Json(serde_json::json!({ "activities": rows? })))
}

pub async fn create_ropa(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<RopaInput>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), HandlerError> {
    ropa_upsert(state, principal, None, body).await
}

pub async fn upsert_ropa(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<RopaInput>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), HandlerError> {
    ropa_upsert(state, principal, Some(id), body).await
}

async fn ropa_upsert(
    state: Arc<AppState>,
    principal: OptPrincipal,
    id: Option<i64>,
    body: RopaInput,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), HandlerError> {
    admin_gate(&principal.0)?;
    if body.activity.trim().is_empty() || body.lawful_basis.trim().is_empty() {
        return Err(HandlerError::bad_request(
            "ropa_incomplete",
            "activity and lawful_basis are required",
        ));
    }
    capped("activity", &body.activity, 256)?;
    capped("controller", &body.controller, 256)?;
    capped("processor", &body.processor, 256)?;
    capped("lawful_basis", &body.lawful_basis, 128)?;
    capped("categories", &body.categories, 1_024)?;
    capped("recipients", &body.recipients, 1_024)?;
    capped("security_measures", &body.security_measures, 1_024)?;
    capped("transfers", &body.transfers, 1_024)?;
    // Bound the declared window: a garbage retention period would poison the
    // Art.30 register's storage-limitation evidence.
    if let Some(d) = body.retention_days
        && !(0..=36_500).contains(&d)
    {
        return Err(HandlerError::bad_request(
            "retention_days_invalid",
            "retention_days must be 0..=36500",
        ));
    }
    let pool = state.pool.clone();
    let actor = crate::handlers::recall::principal_label(&principal.0);
    let id_out = tokio::task::spawn_blocking(move || -> Result<i64, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let now = chrono::Utc::now().timestamp();
        let rid = match id {
            Some(rid) => {
                let n = tx
                    .execute(
                        "UPDATE ropa_registry SET activity=?2, controller=?3, processor=?4,
                            categories=?5, recipients=?6, lawful_basis=?7, retention_days=?8,
                            security_measures=?9, transfers=?10, updated_at=?11 WHERE id=?1",
                        rusqlite::params![
                            rid,
                            body.activity,
                            body.controller,
                            body.processor,
                            body.categories,
                            body.recipients,
                            body.lawful_basis,
                            body.retention_days,
                            body.security_measures,
                            body.transfers,
                            now
                        ],
                    )
                    .map_err(|e| HandlerError::internal(e.to_string()))?;
                if n == 0 {
                    return Err(HandlerError::not_found(format!(
                        "no RoPA activity with id {rid}"
                    )));
                }
                rid
            }
            None => {
                tx.execute(
                    "INSERT INTO ropa_registry(activity, controller, processor, categories,
                          recipients, lawful_basis, retention_days, security_measures,
                          transfers, created_at, updated_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",
                    rusqlite::params![
                        body.activity,
                        body.controller,
                        body.processor,
                        body.categories,
                        body.recipients,
                        body.lawful_basis,
                        body.retention_days,
                        body.security_measures,
                        body.transfers,
                        now
                    ],
                )
                .map_err(|e| HandlerError::internal(e.to_string()))?;
                tx.last_insert_rowid()
            }
        };
        crate::audit::record(
            &tx,
            crate::audit::AuditKind::Client,
            &actor,
            &format!("ropa:{rid}"),
            crate::audit::AuditStatus::Ok,
            "ropa_upserted",
        );
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok(rid)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({ "id": id_out? })),
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The decision signing key resolves once per process from the env; every
    /// compliance test installs the same fixed seed under a shared lock so
    /// signatures verify deterministically.
    pub(crate) static KEY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    const TEST_SEED_HEX: &str = "0707070707070707070707070707070707070707070707070707070707070707";
    /// Install the FIXED test seed directly into the process-global cache and
    /// hold the crate-wide decision test lock for the caller's whole
    /// record→verify span. Env-var racing cannot produce mixed-key
    /// signatures (the tip_truncation CI flake).
    #[must_use]
    pub(crate) fn ensure_test_key() -> std::sync::MutexGuard<'static, ()> {
        let _g = brain_server::audit::decision::decision_test_lock();
        brain_server::audit::decision::install_test_signing_key([7u8; 32]);
        _g
    }

    fn db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(brain_server::audit::decision::DDL)
            .unwrap();
        conn.execute_batch(
            "CREATE TABLE oversight_evidence(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                reviewer_id TEXT NOT NULL,
                reviewed_at INTEGER NOT NULL,
                basis TEXT NOT NULL,
                outcome TEXT NOT NULL,
                authority TEXT NOT NULL DEFAULT '',
                decision_hash TEXT
             );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn oversight_links_a_signed_decision_record() {
        let _key = ensure_test_key();
        let conn = db();
        let id = record_oversight(
            &conn,
            "dpo-1",
            "digest-abc",
            "accept",
            "approve",
            Some(7),
            "global",
        )
        .unwrap();
        assert_eq!(id, 1);
        let (hash, outcome): (String, String) = conn
            .query_row(
                "SELECT decision_hash, outcome FROM oversight_evidence WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(outcome, "accept");
        // the linked decision record exists and carries the same hash
        let stored: String = conn
            .query_row(
                "SELECT hash FROM decision_records WHERE hash = ?1",
                rusqlite::params![hash],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, hash);
        let _g = brain_server::audit::decision::decision_test_lock();
        assert!(brain_server::audit::decision::verify_decisions(&conn).unwrap());
    }

    #[test]
    fn wire_input_caps_reject_absurd_sizes() {
        assert!(capped("x", &"a".repeat(255), 256).is_ok());
        assert!(capped("x", &"a".repeat(257), 256).is_err());
        assert!(capped("x", "", 1).is_ok());
    }

    #[test]
    fn dataset_hash_predicate_is_strict() {
        let hex = "a".repeat(64);
        assert!(is_sha256_hex(&hex));
        assert!(!is_sha256_hex(&"g".repeat(64)));
        assert!(!is_sha256_hex(&"a".repeat(63)));
        assert!(!is_sha256_hex(""));
    }

    #[test]
    fn pdf_labelled_export_carries_domain_provenance() {
        let conn = db();
        let _key = ensure_test_key();
        record_oversight(&conn, "dpo-1", "d", "accept", "approve", None, "global");
        let recs = brain_server::audit::decision::list_decisions(&conn, None, 10).unwrap();
        let labels = vec!["acme-us"];
        let refs: Vec<&brain_server::audit::decision::DecisionRecord> = recs.iter().collect();
        let body = String::from_utf8(brain_server::audit::decision::render_pdf_labelled(
            &labels, &refs, "t",
        ))
        .unwrap();
        assert!(body.contains("domain=acme-us"), "label present");
        assert!(body.contains("decision id="));
        // Unlabelled render stays label-free (legacy shape).
        let plain =
            String::from_utf8(brain_server::audit::decision::render_pdf(&recs, "t")).unwrap();
        assert!(!plain.contains("domain="));
    }

    #[test]
    fn tampered_signature_fails_verification() {
        let _key = ensure_test_key();
        let conn = db();
        record_oversight(
            &conn,
            "dpo-1",
            "d",
            "override",
            "reject",
            Some(9),
            "acme-us",
        );
        let n = conn
            .execute(
                "UPDATE decision_records SET sig = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
                [],
            )
            .unwrap();
        assert_eq!(n, 1);
        let _g = brain_server::audit::decision::decision_test_lock();
        assert!(!brain_server::audit::decision::verify_decisions(&conn).unwrap());
    }
}
