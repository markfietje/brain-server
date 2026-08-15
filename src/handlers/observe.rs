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
/// `action` is `export` | `purge` | `both`; `dry_run` (v1.20.21) previews the
/// footprint — locate + bundle build only, nothing is purged or written.
#[derive(Debug, Deserialize)]
pub struct DsarRequest {
    pub subject: String,
    #[serde(default = "default_dsar_action")]
    pub action: String,
    #[serde(default)]
    pub dry_run: bool,
}

fn default_dsar_action() -> String {
    "both".to_string()
}

/// v1.20.22 "Clocks" M1.1: the DSAR Art 17 erasure deadline — `created_at` +
/// the operator's window, a pure mirror of `gate::proposal_deadline`. The
/// client countdown ticks against this absolute deadline, so an operator's
/// `BRAIN_DSAR_WINDOW_DAYS` override is authoritative (no client window guess).
pub fn dsar_deadline(created_at: i64) -> i64 {
    created_at + crate::config::dsar_window_secs()
}

/// `POST /dsar` response: the workflow row id + the deletion certificate. In
/// dry-run mode the certificate is `None` and `footprint` carries the would-be
/// deletion footprint instead.
#[derive(Debug, Serialize)]
pub struct DsarResponse {
    pub id: i64,
    pub subject: String,
    pub status: &'static str,
    /// v1.20.22 "Clocks" M1.1: when the request was created (ledger `created_at`).
    pub created_at: i64,
    /// v1.20.22 "Clocks" M1.1: the computed Art 17 erasure deadline
    /// (`created_at + window`) — the client clock's source of truth. `0` in a
    /// dry-run preview (no ledger row, no deadline).
    pub deadline: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footprint: Option<Footprint>,
}

/// v1.20.21: the would-be DSAR deletion footprint — what a live purge would
/// locate + export + delete, without executing any write. The GDPR Art 17
/// preview a DPO reads before clicking "erase".
#[derive(Debug, Serialize, Clone)]
pub struct Footprint {
    pub roots: usize,
    pub derived: usize,
    pub export_rows: usize,
    pub tombstones: usize,
    pub dsar_rows: usize,
    pub dry_run: bool,
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
    let action = req.action.clone();
    let action_did_purge = matches!(action.as_str(), "purge" | "both");

    // v1.20.24 "Sweep" (cross-domain erasure): a DSAR must cover every
    // registered domain, not just the global pool. In multi-db mode each
    // `brain-<domain>.db` runs its own locate + purge tx; in shim mode the
    // list is exactly the global pool (whose owner query already covers every
    // row of the one shared DB — byte-identical behavior to v1.20.23).
    // ponytail: per-domain txs, not one cross-file tx — a crash mid-run can
    // leave some domains purged with the ledger written last (erasure-safe
    // direction; the ledger under-reports rather than over-reports); the
    // ledger row + certificate are the global DB's registry of record.
    let domains: Vec<String> = if state.registry.is_multi_db() {
        state.registry.known_domains()
    } else {
        vec!["global".to_string()]
    };
    let mut pools: Vec<(String, crate::Pool)> = Vec::with_capacity(domains.len());
    for d in &domains {
        pools.push((
            d.clone(),
            super::resolve_domain_pool(&state.registry, Some(d))?,
        ));
    }
    // Global runs LAST so its ledger row can carry the cross-domain digest.
    pools.sort_by(|a, b| match (a.0 == "global", b.0 == "global") {
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        _ => std::cmp::Ordering::Equal,
    });

    let outcome = tokio::task::spawn_blocking(move || -> Result<DsarOutcome, HandlerError> {
        let now = chrono::Utc::now().timestamp();
        let dry_run = req.dry_run;
        let mut runs: Vec<DsarPoolRun> = Vec::with_capacity(pools.len());

        // 1+2. Non-global pools first: each locates + purges in its own tx and
        //      returns its bundle + purged ids for the aggregate.
        let mut cross_bundle: Vec<(String, String)> = Vec::new();
        let mut cross_ids: Vec<i64> = Vec::new();
        let global_idx = pools
            .iter()
            .position(|(name, _)| name == "global")
            .ok_or_else(|| HandlerError::internal("global pool missing".to_string()))?;
        for (idx, (name, pool)) in pools.iter().enumerate() {
            if idx == global_idx {
                continue;
            }
            let run = run_dsar_pool(pool, &subject, &action, dry_run, now, false, None)?;
            if !dry_run {
                cross_ids.extend(run.purged_ids.iter().copied());
                if let Some(b) = &run.bundle {
                    cross_bundle.push((name.clone(), b.clone()));
                }
            }
            runs.push(run);
        }

        // 3. Aggregate digest for the global ledger row: in shim mode this is
        //    the single local bundle (byte-identical hash to v1.20.23); in
        //    multi-db mode it is SHA-256 over the joined per-domain bundles.
        let aggregate_hash = if !dry_run && pools.len() > 1 && !cross_bundle.is_empty() {
            Some(crate::handlers::gate::sha256_hex(
                &serde_json::json!({ "subject": subject, "domains": cross_bundle }).to_string(),
            ))
        } else {
            None // shim mode: the global run digests its own bundle
        };

        // 4. The global pool: locate + purge + the ledger row (atomic in its
        //    own tx, as v1.20.17 M5 requires).
        let global_run = run_dsar_pool(
            &pools[global_idx].1,
            &subject,
            &action,
            dry_run,
            now,
            true,
            aggregate_hash.as_deref(),
        )?;
        runs.push(global_run);

        // 5. Dry-run preview: aggregate the read-only footprint across pools.
        if dry_run {
            let mut fp = Footprint {
                roots: 0,
                derived: 0,
                export_rows: 0,
                tombstones: 0,
                dsar_rows: 0,
                dry_run: true,
            };
            for r in &runs {
                fp.roots += r.roots;
                fp.derived += r.derived;
                fp.export_rows += r.export_rows;
                fp.tombstones += r.tombstones;
                fp.dsar_rows += r.dsar_rows;
            }
            return Ok(DsarOutcome::Footprint(fp));
        }

        // 6. Post-commit: audit + certificate on the global pool (the audit
        //    chain is the registry of record), then backfill the ledger row.
        let global_conn = pools[global_idx]
            .1
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        crate::audit::record(
            &global_conn,
            crate::audit::AuditKind::Reconcile,
            "api",
            &format!("dsar:{subject}"),
            crate::audit::AuditStatus::Ok,
            "dsar",
        );
        let chain_head = crate::audit::chain_head(&global_conn);
        let certified_at = chrono::Utc::now().to_rfc3339();
        let mut purged_ids = cross_ids;
        let mut found_count: usize = 0;
        let mut held: Vec<serde_json::Value> = Vec::new();
        let mut tombstone_root: Option<i64> = None;
        let mut ledger_id: Option<i64> = None;
        for r in &runs {
            found_count += r.roots + r.derived;
            purged_ids.extend(r.purged_ids.iter().copied());
            held.extend(r.held.iter().cloned());
            // Prefer the ledger-bearing (global) run's root as the certificate
            // anchor — the registry of record — falling back to any domain.
            if r.ledger_id.is_some() || tombstone_root.is_none() {
                tombstone_root = r.tombstone_root;
            }
            ledger_id = r.ledger_id.or(ledger_id);
        }
        let certificate = serde_json::json!({
            "subject": subject,
            "action": action,
            "found_count": found_count,
            "purged_ids": purged_ids,
            // v1.22.0 M1/M3: held ids (deferred erasure, with reasons) + the
            // residency stamp that proves where the data lived.
            "region": brain_server::storage_layout::region(),
            "held_ids": held,
            "tombstone_root": tombstone_root,
            "certified_at": certified_at,
            "chain_head": chain_head,
        })
        .to_string();
        let ledger_id =
            ledger_id.ok_or_else(|| HandlerError::internal("no ledger row written".to_string()))?;

        // v1.20.17 M5: backfill the certificate onto the ledger row committed
        // with the purge (best-effort — the row + times already prove it).
        let _ = global_conn.execute(
            "UPDATE dsar_requests SET certificate = ?1 WHERE id = ?2",
            rusqlite::params![certificate, ledger_id],
        );

        Ok(DsarOutcome::Completed {
            id: ledger_id,
            subject,
            certificate,
            certified_at,
            created_at: now,
        })
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    // A dry-run preview never purged anything: no art-19 notification, no
    // certificate; return the footprint as the whole answer.
    if let DsarOutcome::Footprint(fp) = &outcome {
        return Ok(Json(DsarResponse {
            id: 0,
            subject: String::new(),
            status: "preview",
            created_at: 0,
            deadline: 0,
            certificate: None,
            footprint: Some(fp.clone()),
        }));
    }
    let DsarOutcome::Completed {
        id,
        subject,
        certificate,
        certified_at,
        created_at,
    } = outcome
    else {
        unreachable!("footprint already returned above")
    };

    // 6. Art 19 onward-notification: opt-in, fire-and-forget, fail-soft.
    if action_did_purge {
        notify_art19(subject.clone(), id, certified_at.clone());
    }

    let cert: serde_json::Value = serde_json::from_str(&certificate)
        .map_err(|_| HandlerError::internal("stored certificate is not valid JSON"))?;
    Ok(Json(DsarResponse {
        id,
        subject,
        status: "completed",
        created_at,
        deadline: dsar_deadline(created_at),
        certificate: Some(cert),
        footprint: None,
    }))
}

enum DsarOutcome {
    Completed {
        id: i64,
        subject: String,
        certificate: String,
        certified_at: String,
        created_at: i64,
    },
    Footprint(Footprint),
}

// ---------------------------------------------------------------------------
// M3.1 — DSAR ledger list
// ---------------------------------------------------------------------------

/// `GET /dsar?limit=&offset=` query.
#[derive(Debug, Default, Deserialize)]
pub struct DsarLedgerQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// One `dsar_requests` ledger row (v1.20.22 "Clocks" M1.2). `created_at` +
/// `completed_at` are the clock inputs; `deadline` is the server-computed Art
/// 17 erasure window so the client ticks against the SAME number the `POST`
/// response carries — no client mirror of `BRAIN_DSAR_WINDOW_DAYS`. The
/// subject is the operator's operand (Admin surface; no redaction).
#[derive(Debug, Serialize)]
pub struct DsarLedgerRow {
    pub id: i64,
    pub subject: String,
    pub action: String,
    pub status: String,
    pub created_at: Option<i64>,
    pub deadline: Option<i64>,
    pub completed_at: Option<i64>,
}

/// `GET /dsar` response: a bounded, newest-first page + the total row count.
#[derive(Debug, Serialize)]
pub struct DsarLedger {
    pub requests: Vec<DsarLedgerRow>,
    pub total: i64,
}

/// `GET /dsar` — the v1.20.22 "Clocks" ledger list (Admin). Past DSAR requests
/// were only visible via the `/audit` side-channel; this is the first-class
/// registry the client countdown renders. Bounded (default 100, clamped to
/// `MAX_MULTI_GET`), newest-first (`ORDER BY id DESC`), the audit pagination
/// idiom.
pub async fn list_dsar(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<DsarLedgerQuery>,
) -> Result<Json<DsarLedger>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, Some("global"))?;
    let limit = q
        .limit
        .map(|l| l.clamp(1, crate::config::MAX_MULTI_GET as i64))
        .unwrap_or(100);
    let offset = q.offset.unwrap_or(0).max(0);
    let body = tokio::task::spawn_blocking(move || -> Result<DsarLedger, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        list_dsar_page(&conn, limit, offset)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(body))
}

/// Pure ledger query (v1.20.22 M1.2) — extracted so the ordering, the page
/// boundary, and the total count are unit-testable without an HTTP stack
/// (the `page_decayed` idiom).
pub(crate) fn list_dsar_page(
    conn: &rusqlite::Connection,
    limit: i64,
    offset: i64,
) -> Result<DsarLedger, HandlerError> {
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM dsar_requests", [], |r| r.get(0))
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let mut stmt = conn
        .prepare(
            "SELECT id, subject, action, status, created_at, completed_at
             FROM dsar_requests ORDER BY id DESC LIMIT ?1 OFFSET ?2",
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let requests = stmt
        .query_map(rusqlite::params![limit, offset], |r| {
            let created_at = r.get::<_, Option<i64>>(4)?;
            Ok(DsarLedgerRow {
                id: r.get(0)?,
                subject: r.get(1)?,
                action: r.get(2)?,
                status: r.get(3)?,
                created_at,
                deadline: created_at.map(dsar_deadline),
                completed_at: r.get(5)?,
            })
        })
        .map_err(|e| HandlerError::internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(DsarLedger { requests, total })
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
    // v1.20.2 E1: tenant scoping. A non-superuser admin only sees tombstones
    // whose `reason` (owner:<subject>) matches their own `sub`. Superuser
    // (`None` principal — opaque/loopback) is unconstrained. The query
    // caller-filter takes precedence if it's narrower than the principal scope.
    let tenant_filter: Option<String> = principal.0.as_ref().map(|p| format!("owner:{}", p.sub));
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
        // Tenant scoping: a non-superuser admin is restricted to their own
        // subject's tombstones, regardless of the `subject` query param.
        if let Some(owner_reason) = &tenant_filter {
            // Caller-supplied `subject` (if any) must agree with the principal's
            // own sub; a cross-tenant request is rejected here at the SQL layer.
            if subject.is_none() {
                clauses.push("reason = ?".to_string());
                params.push(Box::new(owner_reason.clone()));
            } else if subject.as_deref() != Some(owner_reason.trim_start_matches("owner:")) {
                // Cross-tenant request → empty result (don't leak existence).
                return Ok(serde_json::json!({ "tombstones": [] }));
            }
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
    // v1.20.2 E1: tenant scoping. A non-superuser admin can only fetch
    // certificates for their own subject. The stored `dsar_requests.subject`
    // is checked against the principal's `sub`; a mismatch → 404 (don't leak
    // existence of another tenant's certificate).
    let tenant_sub: Option<String> = principal.0.as_ref().map(|p| p.sub.clone());
    let body = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        // Fetch subject + certificate in one query so the tenant check happens
        // before the certificate body is read.
        let row: Option<(Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT subject, certificate FROM dsar_requests WHERE id = ?1",
                rusqlite::params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        match row {
            Some((Some(stored_subject), Some(c))) => {
                // Tenant gate: if the principal is scoped, the row's subject must match.
                if let Some(sub) = &tenant_sub {
                    if stored_subject.as_str() != sub.as_str() {
                        return Err(HandlerError::not_found("no dsar request with this id"));
                    }
                }
                let v: serde_json::Value = serde_json::from_str(&c)
                    .map_err(|_| HandlerError::internal("stored certificate is not valid JSON"))?;
                Ok(serde_json::json!({
                    "certificate": v,
                    "chain_verifies": crate::audit::verify_chain(&conn),
                }))
            }
            _ => Err(HandlerError::not_found("no dsar request with this id")),
        }
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(body))
}

// ---------------------------------------------------------------------------
// Art 19 webhook (opt-in) + shared helpers
// ---------------------------------------------------------------------------

/// v1.20.17 M1: ledger retention. Completed `dsar_requests` rows older than
/// `retention_days` are deleted (the erasure record's remaining value is the
/// certificate + the audit chain, not the ledger row itself). Returns the
/// number of rows removed. Pure; best-effort callers swallow the result.
pub(crate) fn purge_stale_dsar_ledger(conn: &rusqlite::Connection, retention_days: u32) -> i64 {
    if retention_days == 0 {
        return 0;
    }
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "DELETE FROM dsar_requests WHERE status = 'completed' AND completed_at < ?1",
        rusqlite::params![now - (retention_days as i64) * 86400],
    )
    .unwrap_or(0) as i64
}

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
        let client = crate::webhook::egress_client();
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

/// v1.20.21: build the portable export bundle (the JSON a live purge embeds
/// into its ledger row) for the given locate result. Extracted so the dry-run
/// preview and the live path run the EXACT same query — behavior-preserving.
fn build_export_bundle(
    tx: &rusqlite::Transaction,
    subject: &str,
    roots: &[i64],
    derived: &[(i64, i64)],
) -> Result<String, HandlerError> {
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
    Ok(serde_json::json!({
        "exported_at": chrono::Utc::now().to_rfc3339(),
        // v1.22.0 M3: the residency stamp on the DSAR bundle too.
        "region": brain_server::storage_layout::region(),
        "subject": subject,
        "knowledge": rows,
    })
    .to_string())
}

/// v1.20.21: count prior deletions for a subject — the tombstone reasons a live
/// purge writes: `owner:<subject>` for roots, and `derived` (scoped to one of
/// this subject's roots via `origin_id`) for derived descendants. The ledger
/// trace a DPO sees in the preview.
fn count_subject_tombstones(
    tx: &rusqlite::Transaction,
    subject: &str,
    roots: &[i64],
) -> Result<i64, HandlerError> {
    let owner_reason = format!("owner:{subject}");
    if roots.is_empty() {
        return tx
            .query_row(
                "SELECT COUNT(*) FROM tombstones WHERE reason = ?1",
                rusqlite::params![owner_reason],
                |r| r.get(0),
            )
            .map_err(|e| HandlerError::internal(e.to_string()));
    }
    let placeholders = vec!["?"; roots.len()].join(",");
    let sql = format!(
        "SELECT COUNT(*) FROM tombstones
          WHERE reason = ?1 OR (reason = 'derived' AND origin_id IN ({placeholders}))"
    );
    let mut stmt = tx
        .prepare(&sql)
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let params: Vec<&dyn rusqlite::ToSql> =
        roots.iter().map(|r| r as &dyn rusqlite::ToSql).collect();
    // ?1 = the owner reason, then one per root for the IN list.
    let mut all_params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(params.len() + 1);
    all_params.push(&owner_reason);
    all_params.extend(params.iter().copied());
    let count: i64 = stmt
        .query_row(all_params.as_slice(), |r| r.get(0))
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    Ok(count)
}

/// v1.20.24 "Sweep": one DSAR pool's outcome — the locate + purge result for a
/// single domain DB (global or `brain-<domain>.db`). Counts for the dry-run
/// footprint, ids + bundle for the cross-domain aggregate, and the ledger row
/// identity when this pool is the registry of record (global).
struct DsarPoolRun {
    roots: usize,
    derived: usize,
    export_rows: usize,
    tombstones: usize,
    dsar_rows: usize,
    /// Live-purge ids from this pool (certificate payload).
    purged_ids: Vec<i64>,
    /// v1.22.0 "Regulated" M1: ids under legal hold that erasure DEFERRED,
    /// with their reasons — listed on the certificate as the why. A held id
    /// is never purged here.
    held: Vec<serde_json::Value>,
    /// This pool's export bundle (cross-domain aggregate input).
    bundle: Option<String>,
    /// `Some(ledger row id)` when this pool wrote the ledger row (global).
    ledger_id: Option<i64>,
    tombstone_root: Option<i64>,
}

/// Run locate + [dry-run preview | purge + ledger] for ONE domain pool.
/// `write_ledger` is true only for the global pool (the registry of record);
/// `aggregate_bundle_hash` carries the cross-domain SHA-256 in multi-db mode
/// (the global run's own bundle is digested in shim mode — byte-identical to
/// v1.20.23). All-or-nothing per pool: the purge + ledger row commit in the
/// same tx (v1.20.17 M5).
fn run_dsar_pool(
    pool: &crate::Pool,
    subject: &str,
    action: &str,
    dry_run: bool,
    now: i64,
    write_ledger: bool,
    aggregate_bundle_hash: Option<&str>,
) -> Result<DsarPoolRun, HandlerError> {
    let mut conn = pool
        .get()
        .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
    let tx = conn
        .transaction()
        .map_err(|e| HandlerError::internal(e.to_string()))?;

    // 1. Locate: owner rows + transitive derived_from descendants.
    let (roots, derived) = dsar_locate(&tx, subject)?;

    // 2. Export bundle (portable JSON; raw PII is never included). Shared by
    //    the live purge path and the dry-run preview — the same SELECT.
    let export_bundle = if matches!(action, "export" | "both") {
        Some(build_export_bundle(&tx, subject, &roots, &derived)?)
    } else {
        None
    };

    // 2a. Dry-run: a read-only footprint preview. Locate + bundle already ran;
    //     count what a live purge WOULD delete, then drop the tx untouched.
    if dry_run {
        let export_rows = match &export_bundle {
            Some(b) => {
                // The bundle always carries `{exported_at, subject, knowledge}`.
                serde_json::from_str::<serde_json::Value>(b)
                    .ok()
                    .and_then(|v| {
                        v.get("knowledge")
                            .and_then(|k| k.as_array())
                            .map(|a| a.len())
                    })
                    .unwrap_or(0)
            }
            None => 0, // `action == "purge"` builds no bundle; nothing exported
        };
        let tombstones = count_subject_tombstones(&tx, subject, &roots)?;
        let dsar_rows: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM dsar_requests WHERE subject = ?1",
                rusqlite::params![subject],
                |r| r.get(0),
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        return Ok(DsarPoolRun {
            roots: roots.len(),
            derived: derived.len(),
            export_rows,
            tombstones: tombstones as usize,
            dsar_rows: dsar_rows as usize,
            purged_ids: Vec::new(),
            held: Vec::new(),
            bundle: None,
            ledger_id: None,
            tombstone_root: None,
        });
    }

    // 3. Purge (all-or-nothing with the export, same tx): roots with the
    //    owner reason, derived descendants with `derived` + origin id.
    let mut purged_ids: Vec<i64> = Vec::new();
    let mut held: Vec<serde_json::Value> = Vec::new();
    if matches!(action, "purge" | "both") {
        // v1.22.0 "Regulated" M1: a held id is frozen against DSAR erasure too
        // (the WORM-lite posture). The subject's located set that is under an
        // active legal hold is DEFERRED — not purged — and listed (+ reasons)
        // on the certificate so the subject is told *why* erasure is deferred.
        let all_targets: Vec<i64> = roots
            .iter()
            .copied()
            .chain(derived.iter().map(|(d, _)| *d))
            .collect();
        let held_map = crate::legal_hold::active_reasons(&tx, &all_targets)?;
        let deferred: std::collections::HashSet<i64> = held_map.keys().copied().collect();
        for (kid, reasons) in &held_map {
            held.push(serde_json::json!({ "id": kid, "reasons": reasons }));
        }
        let free = |ids: &[i64]| {
            ids.iter()
                .filter(|i| !deferred.contains(i))
                .copied()
                .collect::<Vec<_>>()
        };
        for root in &roots {
            let closure: Vec<i64> = derived
                .iter()
                .filter(|(_, r)| r == root)
                .map(|(d, _)| *d)
                .collect();
            if !closure.is_empty() {
                purged_ids.extend(free(&closure).iter().copied());
            }
        }
        let _ = crate::handlers::gate::purge_chunk_ids(
            &tx,
            &free(&roots),
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
                    &free(&closure),
                    now,
                    "derived",
                    Some(*root),
                )?;
            }
        }
        purged_ids.extend(free(&roots).iter().copied());
    }

    // v1.16.1: trace residue sweep. Since v1.20.17 M3 the trace no longer
    // stores the raw query (only its xxh3-64 hash), so the subject can't
    // appear in it — this sweep remains as a defensive net against any
    // future field that does embed personal data. Best-effort (short
    // common subjects over-match slightly; erasure-safe direction).
    if matches!(action, "purge" | "both") && !subject.is_empty() {
        let _ = tx.execute(
            "DELETE FROM recall_traces WHERE trace_json LIKE ?1",
            rusqlite::params![format!("%{subject}%")],
        );
        // v1.20.25: proposals hold raw candidate content with no owner column,
        // so a DSAR could never locate them and their plaintext (possibly PII
        // about the subject) survived a "complete" erasure. Sweep them by the
        // subject verbatim — the same erasure-safe over-match posture as the
        // trace sweep above. ponytail: this is a literal `LIKE %subject%`, not a
        // semantic owner join (proposals are operator-reviewed candidates, not
        // subject-attributed rows); the review-queue provenance for the subject
        // is intentionally erased with the memory per Art 17.
        let _ = tx.execute(
            "DELETE FROM proposals WHERE content LIKE ?1",
            rusqlite::params![format!("%{subject}%")],
        );
    }

    // 4. v1.20.17 M1: store the export's SHA-256 (v1.20.24 "Sweep": replacing
    //    the brute-forceable xxh3-64 digest of a DELETED-content payload),
    //    never the raw bundle — the ledger's job is to prove the purge
    //    happened, not to keep a copy of the erasure payload.
    let mut ledger_id: Option<i64> = None;
    if write_ledger {
        let bundle_hash = aggregate_bundle_hash.map(str::to_string).or_else(|| {
            export_bundle
                .as_deref()
                .map(crate::handlers::gate::sha256_hex)
        });
        tx.execute(
            "INSERT INTO dsar_requests(subject, action, status, export_bundle, certificate, created_at, completed_at)
             VALUES (?1, ?2, 'completed', ?3, NULL, ?4, ?4)",
            rusqlite::params![subject, action, bundle_hash, now],
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
        ledger_id = Some(tx.last_insert_rowid());
    }
    tx.commit()
        .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;

    Ok(DsarPoolRun {
        roots: roots.len(),
        derived: derived.len(),
        export_rows: 0,
        tombstones: 0,
        dsar_rows: 0,
        purged_ids,
        held,
        bundle: export_bundle,
        ledger_id,
        tombstone_root: roots.first().copied(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE dsar_requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                subject TEXT NOT NULL,
                action TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                export_bundle TEXT,
                certificate TEXT,
                created_at INTEGER NOT NULL,
                completed_at INTEGER
             );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn dsar_ledger_stores_hash_not_raw_bundle() {
        let conn = fresh_conn();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO dsar_requests(subject, action, status, export_bundle, created_at, completed_at)
             VALUES ('alice', 'both', 'completed', ?1, ?2, ?2)",
            rusqlite::params![crate::handlers::gate::sha256_hex("personal export payload"), now],
        )
        .unwrap();
        let stored: String = conn
            .query_row("SELECT export_bundle FROM dsar_requests", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            stored,
            crate::handlers::gate::sha256_hex("personal export payload")
        );
        assert_ne!(stored, "personal export payload");
        // The hash is a bounded non-reversible digest, never the content.
        assert_eq!(stored.len(), 64);
    }

    #[test]
    fn purge_deletes_only_old_completed_rows() {
        let conn = fresh_conn();
        let now = chrono::Utc::now().timestamp();
        let insert = |subject: &str, status: &str, completed: i64| {
            conn.execute(
                "INSERT INTO dsar_requests(subject, action, status, export_bundle, created_at, completed_at)
                 VALUES (?1, 'purge', ?2, NULL, 0, ?3)",
                rusqlite::params![subject, status, completed],
            )
            .unwrap();
        };
        let thirty_one_days_ago = now - 31 * 86400;
        let one_day_ago = now - 86400;
        insert("old_completed", "completed", thirty_one_days_ago);
        insert("fresh_completed", "completed", one_day_ago);
        insert("pending", "pending", thirty_one_days_ago); // never purged
        let deleted = purge_stale_dsar_ledger(&conn, 30);
        assert_eq!(deleted, 1);
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM dsar_requests", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 2);
        // The pending erasure record survives regardless of age.
        let subjects: Vec<String> = conn
            .prepare("SELECT subject FROM dsar_requests ORDER BY subject")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(subjects, vec!["fresh_completed", "pending"]);
    }

    #[test]
    fn purge_zero_retention_is_a_noop() {
        let conn = fresh_conn();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO dsar_requests(subject, action, status, export_bundle, created_at, completed_at)
             VALUES ('x', 'purge', 'completed', NULL, 0, ?1)",
            rusqlite::params![now - 400 * 86400],
        )
        .unwrap();
        assert_eq!(purge_stale_dsar_ledger(&conn, 0), 0);
    }

    #[test]
    fn ledger_row_is_committed_atomically_with_purge_tx_commit() {
        // v1.20.17 M5 regression: the ledger insert used to happen AFTER the
        // tx.commit() — a crash between the two lost the erasure record. Now
        // the insert rides in the SAME tx as the purge; prove the row exists
        // the moment the tx commits by simulating the handler's sequence.
        let mut conn = fresh_conn();
        let tx = conn.transaction().unwrap();
        let now = chrono::Utc::now().timestamp();
        tx.execute(
            "INSERT INTO dsar_requests(subject, action, status, export_bundle, certificate, created_at, completed_at)
             VALUES ('alice', 'both', 'completed', NULL, NULL, ?1, ?1)",
            rusqlite::params![now],
        )
        .unwrap();
        let id = tx.last_insert_rowid();
        tx.commit().unwrap();
        let (subj, status): (String, String) = conn
            .query_row(
                "SELECT subject, status FROM dsar_requests WHERE id = ?1",
                rusqlite::params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((subj.as_str(), status.as_str()), ("alice", "completed"));
        // Certificate is backfilled post-commit (best-effort).
        let _ = conn.execute(
            "UPDATE dsar_requests SET certificate = ?1 WHERE id = ?2",
            rusqlite::params!["cert", id],
        );
        let cert: String = conn
            .query_row(
                "SELECT certificate FROM dsar_requests WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cert, "cert");
    }

    /// A fresh connection with the tables the M1 helpers touch: `knowledge`
    /// (owner + export columns), `evidence_links` (derived walk), `tombstones`
    /// (deletion registry), `dsar_requests` (ledger history).
    fn helper_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE knowledge (
                id INTEGER PRIMARY KEY,
                content TEXT,
                content_hash TEXT,
                node_kind TEXT DEFAULT 'chunk',
                assertion_kind TEXT DEFAULT 'stated',
                confidence REAL DEFAULT 0.5,
                owner TEXT,
                observed_at TEXT,
                valid_from TEXT,
                valid_to TEXT
             );
             CREATE TABLE evidence_links (
                kind TEXT,
                from_chunk INTEGER,
                to_chunk INTEGER
             );
             CREATE TABLE tombstones (
                id INTEGER PRIMARY KEY,
                reason TEXT,
                origin_id INTEGER
             );
             CREATE TABLE dsar_requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                subject TEXT NOT NULL,
                action TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                export_bundle TEXT,
                certificate TEXT,
                created_at INTEGER NOT NULL,
                completed_at INTEGER
             );",
        )
        .unwrap();
        conn
    }

    /// v1.20.21 M1.1: a dry-run footprint reports the exact would-be counts and
    /// writes NOTHING — the knowledge rows survive, no ledger row, no new
    /// tombstone. The preview is a pure read.
    #[test]
    fn dsar_dry_run_footprint_counts_and_writes_nothing() {
        let mut conn = helper_conn();
        conn.execute_batch(
            "INSERT INTO knowledge(id, content, content_hash, owner) VALUES
                 (1, 'alice root', 'h1', 'alice@example.com'),
                 (2, 'alice derived', 'h2', NULL),
                 (3, 'bob chunk', 'h3', 'bob@example.com');
             INSERT INTO evidence_links(kind, from_chunk, to_chunk)
                 VALUES ('derived_from', 1, 2);
             INSERT INTO tombstones(reason, origin_id) VALUES
                 ('owner:alice@example.com', NULL),
                 ('derived', 1);
             INSERT INTO dsar_requests(subject, action, status, created_at, completed_at)
                 VALUES ('alice@example.com', 'both', 'completed', 0, 0);",
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        let (roots, derived) = dsar_locate(&tx, "alice@example.com").unwrap();
        assert_eq!(roots, vec![1]);
        assert_eq!(derived, vec![(2, 1)]);
        let bundle = build_export_bundle(&tx, "alice@example.com", &roots, &derived).unwrap();
        let export_rows: usize = serde_json::from_str::<serde_json::Value>(&bundle)
            .unwrap()
            .get("knowledge")
            .unwrap()
            .as_array()
            .unwrap()
            .len();
        assert_eq!(export_rows, 2, "bundle carries both root + derived");
        let tombstones = count_subject_tombstones(&tx, "alice@example.com", &roots).unwrap();
        let dsar_rows: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM dsar_requests WHERE subject = ?1",
                rusqlite::params!["alice@example.com"],
                |r| r.get(0),
            )
            .unwrap();
        // Footprint assembly mirrors the handler's dry-run branch.
        let fp = Footprint {
            roots: roots.len(),
            derived: derived.len(),
            export_rows,
            tombstones: tombstones as usize,
            dsar_rows: dsar_rows as usize,
            dry_run: true,
        };
        assert_eq!(fp.roots, 1);
        assert_eq!(fp.derived, 1);
        assert_eq!(fp.export_rows, 2);
        assert_eq!(fp.tombstones, 2, "owner reason + derived-scoped row");
        assert_eq!(fp.dsar_rows, 1, "ledger history counted");
        // Nothing written by the read-only helpers. Drop the tx (a read-only
        // tx the handler would drop untouched) before reading the conn.
        drop(tx);
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 3, "no knowledge deleted");
        let toms: i64 = conn
            .query_row("SELECT COUNT(*) FROM tombstones", [], |r| r.get(0))
            .unwrap();
        assert_eq!(toms, 2, "no new tombstone");
        let led: i64 = conn
            .query_row("SELECT COUNT(*) FROM dsar_requests", [], |r| r.get(0))
            .unwrap();
        assert_eq!(led, 1, "no ledger row written");
    }

    /// v1.20.21 M1.1: `build_export_bundle` is behavior-preserving — the
    /// extracted builder produces the same JSON the live purge path embeds.
    #[test]
    fn dsar_export_bundle_builder_matches_live_shape() {
        let mut conn = helper_conn();
        conn.execute_batch(
            "INSERT INTO knowledge(id, content, content_hash, owner) VALUES
                 (1, 'alice root', 'h1', 'alice@example.com'),
                 (2, 'alice derived', 'h2', NULL);
             INSERT INTO evidence_links(kind, from_chunk, to_chunk)
                 VALUES ('derived_from', 1, 2);",
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        let (roots, derived) = dsar_locate(&tx, "alice@example.com").unwrap();
        let bundle = build_export_bundle(&tx, "alice@example.com", &roots, &derived).unwrap();
        let v: serde_json::Value = serde_json::from_str(&bundle).unwrap();
        assert_eq!(v["subject"], "alice@example.com");
        let k = v["knowledge"].as_array().unwrap();
        assert_eq!(k.len(), 2);
        let ids: Vec<i64> = k.iter().map(|r| r["id"].as_i64().unwrap()).collect();
        assert_eq!(ids, vec![1, 2]);
        // Same per-row shape the live handler relies on.
        assert!(k[0].get("content").is_some());
        assert!(k[0].get("memory_kind").is_some());
    }

    /// v1.20.24 "Sweep" (G4): a multi-domain DSAR purges the subject in EVERY
    /// pool but writes the ledger row + aggregate hash only on the global pool
    /// (the registry of record) — mirroring the handler's run order (non-global
    /// first, global last). Each pool commits its own transaction, so a crash
    /// between pools erases-but-under-reports (erasure-safe direction).
    #[test]
    fn cross_domain_dsar_purges_all_pools_and_ledgers_once() {
        use r2d2_sqlite::SqliteConnectionManager;

        crate::register_sqlite_vec();
        let mk_pool = || {
            let mgr = SqliteConnectionManager::memory();
            let pool: crate::Pool = r2d2::Pool::builder()
                .max_size(1)
                .build(mgr)
                .expect("build pool");
            let mut conn = pool.get().unwrap();
            brain_server::migration::run_migration(&mut conn, 1).expect("migration");
            drop(conn);
            pool
        };
        let global = mk_pool();
        let health = mk_pool();
        let now = chrono::Utc::now().timestamp();
        let subject = "alice@example.com";

        for pool in [&global, &health] {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO knowledge (content, content_hash, owner) VALUES
                     ('alice root in this db', 'h1', ?1)",
                rusqlite::params![subject],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO knowledge (content, content_hash) VALUES ('alice derived here', 'h2')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO evidence_links(kind, from_chunk, to_chunk) VALUES ('derived_from', 1, 2)",
                [],
            )
            .unwrap();
        }

        // Handler order: non-global pools first (local txs, no ledger)...
        let health_run = run_dsar_pool(&health, subject, "both", false, now, false, None).unwrap();
        assert!(!health_run.purged_ids.is_empty(), "health pool erased");
        assert_eq!(health_run.ledger_id, None, "non-global pool never ledgers");
        // ...then global, with the cross-domain aggregate hash.
        let aggregate = crate::handlers::gate::sha256_hex(
            &serde_json::json!({"subject": subject, "domains": ["health"]}).to_string(),
        );
        let global_run =
            run_dsar_pool(&global, subject, "both", false, now, true, Some(&aggregate)).unwrap();
        assert!(!global_run.purged_ids.is_empty(), "global pool erased");
        assert!(
            global_run.ledger_id.is_some(),
            "global pool owns the ledger row"
        );

        for (name, pool) in [("global", &global), ("health", &health)] {
            let conn = pool.get().unwrap();
            let remaining: i64 = conn
                .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
                .unwrap_or(0);
            assert_eq!(remaining, 0, "{name} knowledge fully purged");
            let toms: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM tombstones WHERE reason = ?1",
                    rusqlite::params![format!("owner:{subject}")],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            assert_eq!(toms, 1, "{name} tombstoned the root");
        }
        // Exactly one ledger row, on global, carrying the aggregate digest —
        // never a pool-local bundle.
        let conn = global.get().unwrap();
        let (count, stored): (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(MAX(export_bundle), '') FROM dsar_requests",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1, "one ledger row across all pools");
        assert_eq!(stored, aggregate, "ledger stores the cross-domain digest");
        let health_led: i64 = health
            .get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM dsar_requests", [], |r| r.get(0))
            .unwrap_or(0);
        assert_eq!(health_led, 0, "non-global pool has no ledger rows");
    }

    /// v1.20.25: a DSAR purge must erase the review-queue residue and the graph
    /// residue that v1.20.24 left behind — proposals have no owner column so
    /// their raw candidate content (possible PII about the subject) survived a
    /// "complete" erasure, and the entity-scoped relationship delete referenced
    /// a non-existent `entities.knowledge_id` so relationships + PII-named
    /// entity nodes survived every purge. Both must now go, while shared
    /// entities survive.
    #[test]
    fn dsar_purge_erases_proposals_and_orphaned_entities() {
        use r2d2_sqlite::SqliteConnectionManager;
        crate::register_sqlite_vec();
        let mgr = SqliteConnectionManager::memory();
        let pool: crate::Pool = r2d2::Pool::builder().max_size(1).build(mgr).expect("pool");
        let mut conn = pool.get().unwrap();
        brain_server::migration::run_migration(&mut conn, 1).expect("migration");
        let subject = "alice@example.com";
        // Root knowledge owned by the subject (will be purged).
        conn.execute(
            "INSERT INTO knowledge(id, content, content_hash, owner) VALUES (1, 'alice root', 'h1', ?1)",
            rusqlite::params![subject],
        )
        .unwrap();
        // A proposal whose raw content mentions the subject (PII in the queue).
        conn.execute(
            "INSERT INTO proposals(id, kind, content, novelty, salience, status, created_at)
             VALUES (1, 'fact', 'contact alice@example.com re: x', 1.0, 0.5, 'pending', 1)",
            [],
        )
        .unwrap();
        // Two entities: 10 is PII-named + only in the purged relationship;
        // 11 is shared with a surviving chunk.
        conn.execute(
            "INSERT INTO entities(id, name) VALUES (10, 'alice@example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO entities(id, name) VALUES (11, 'shared-concept')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO entities(id, name) VALUES (12, 'survivor')", [])
            .unwrap();
        // A surviving chunk (no owner) holding the shared entity's relationship.
        conn.execute(
            "INSERT INTO knowledge(id, content, content_hash) VALUES (2, 'survivor chunk', 'h2')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO relationships(from_entity_id, to_entity_id, relation_type, knowledge_id)
             VALUES (10, 11, 'relates_to', 1), (11, 12, 'relates_to', 2)",
            [],
        )
        .unwrap();
        drop(conn);

        let now = chrono::Utc::now().timestamp();
        let run = run_dsar_pool(&pool, subject, "both", false, now, true, None).unwrap();
        assert!(!run.purged_ids.is_empty(), "subject root erased");

        let conn = pool.get().unwrap();
        let proposals: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM proposals WHERE content LIKE ?1",
                rusqlite::params![format!("%{subject}%")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(proposals, 0, "proposal PII erased with the memory");
        let e10: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities WHERE id=10", [], |r| {
                r.get(0)
            })
            .unwrap();
        let e11: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities WHERE id=11", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(e10, 0, "orphaned PII-named entity erased");
        assert_eq!(e11, 1, "shared entity survives");
        let rels: i64 = conn
            .query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rels, 1, "only the surviving chunk's relationship remains");
    }
}
