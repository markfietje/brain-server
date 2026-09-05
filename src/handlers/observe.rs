//! observability + compliance-workflow handlers — the protocol adapters for
//! the rights surface. ALL storage for this aggregate lives in the service
//! core (`crate::service::dsar` + `crate::service::dsar::sweep` +
//! `crate::service::purge`, the Quarry move); what stays here is exactly the
//! adapter law: parse → OptPrincipal → authorize → spawn_blocking (borrow a
//! connection per pool) → core call → response.
//!
//! The recall read-event trace endpoint (`/recall/{id}/trace`) replays
//! the decision path of a recorded read event (the audit row is hash-only; the
//! non-content trace metadata lives in `recall_traces`).
//! DSAR orchestration on top of the `/export` + `/purge` primitives —
//! `POST /dsar` (locate → export → purge → certificate), `GET /tombstones`
//! (the queryable deletion registry), `GET /dsar/{id}/certificate` (chain-
//! verifiable), and the opt-in Art 19 onward-notification webhook.
//!
//! Nothing here runs autonomously: a DSAR is an explicit operator action
//! (Admin gate), and the webhook is fire-and-forget fail-soft.

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;
use crate::handlers::auth::OptPrincipal;
use crate::handlers::{HandlerError, MAX_QUERY};
use crate::service::dsar::{
    DsarError, DsarLedger, backfill_certificate, certificate_json, certificate_view,
    list_dsar_page, tombstones_page,
};

/// Map a core [`DsarError`] onto this route family's FROZEN probe-blind
/// vocabulary: storage failures → the internal-error body with the message
/// verbatim; a missing / foreign-tenant certificate row → the certificate
/// route's 404 (existence never leaks); the knowledge-purge backstop fence →
/// the shared `409 legal_hold_active` envelope every erasure route renders.
impl From<DsarError> for HandlerError {
    fn from(e: DsarError) -> Self {
        match e {
            DsarError::Database(m) => HandlerError::internal(m),
            DsarError::NotFound => HandlerError::not_found("no dsar request with this id"),
            DsarError::LegalHold(held) => HandlerError::conflict_with(
                "legal_hold_active",
                "one or more ids are under legal hold",
                serde_json::json!({ "held": held }),
            ),
        }
    }
}

/// `POST /dsar` request. `subject` is the owner/principal being actioned;
/// `action` is `export` | `purge` | `both`; `dry_run` previews the
/// footprint — locate + bundle build only, nothing is purged or written.
#[derive(Debug, Deserialize)]
pub struct DsarRequest {
    pub subject: String,
    #[serde(default = "default_dsar_action")]
    pub action: String,
    #[serde(default)]
    pub dry_run: bool,
    /// the subject's jurisdiction (country code,
    /// e.g. `eu`, `us`). Absent → the legacy generic Art 17 window/rights
    /// surface. When set, the response carries the jurisdiction's curated
    /// rights + its deadline (GDPR 1 month, CCPA 45 days, PH reasonable, ...).
    #[serde(default)]
    pub jurisdiction: Option<String>,
    /// the mechanism (e.g. `scc-eu-2021`) recorded
    /// on the deletion certificate — the per-law proof of compliance. Free-text
    /// so the operator records exactly what they signed.
    #[serde(default)]
    pub mechanism: Option<String>,
    /// Exact-subject matching for the residue sweeps (traces, proposals,
    /// workflow state): default is the erasure-safe substring sweep; an
    /// operator may narrow to exact matches to avoid over-matching short or
    /// wildcard-like subjects.
    #[serde(default)]
    pub subject_exact: bool,
}

fn default_dsar_action() -> String {
    "both".to_string()
}

// ---------------------------------------------------------------------------
// recall trace
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
        let conn = pool.get().map_err(HandlerError::db_down)?;
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

/// `POST /dsar` request. `subject` is the owner/principal being actioned;
/// `POST /dsar` response: the workflow row id + the deletion certificate. In
/// dry-run mode the certificate is `None` and `footprint` carries the would-be
/// deletion footprint instead. The footprint type is the core's
/// (`crate::service::dsar::Footprint`) — the preview is the core's answer.
#[derive(Debug, Serialize)]
pub struct DsarResponse {
    pub id: i64,
    pub subject: String,
    pub status: &'static str,
    /// when the request was created (ledger `created_at`).
    pub created_at: i64,
    /// the computed Art 17 erasure deadline
    /// (`created_at + window`) — the client clock's source of truth. `0` in a
    /// dry-run preview (no ledger row, no deadline).
    pub deadline: i64,
    /// the subject's jurisdiction (when provided).
    /// Absent for the generic surface / a dry-run preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<String>,
    /// the jurisdiction's applicable subject rights
    /// (the DSAR response lists them so the operator acts per the subject's
    /// law). Empty when no jurisdiction was given.
    pub rights: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footprint: Option<crate::service::dsar::Footprint>,
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
    let subject = normalize_dsar_subject(&req.subject, &req.action)?;
    // Erasure is irreversible: Admin.
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let action = req.action.clone();
    let action_did_purge = matches!(action.as_str(), "purge" | "both");

    // normalize the subject's jurisdiction.
    let jurisdiction = match req.jurisdiction.clone() {
        Some(j) => {
            let j = j.trim().to_ascii_lowercase();
            if !crate::transfers::is_jurisdiction_code(&j) {
                return Err(HandlerError::bad_request(
                    "jurisdiction_invalid",
                    "jurisdiction must be a short lowercase country code",
                ));
            }
            Some(j)
        }
        None => None,
    };
    let rights: Vec<&'static str> = jurisdiction
        .as_deref()
        .and_then(crate::transfers::jurisdiction_rule)
        .map(|r| r.rights.to_vec())
        .unwrap_or_default();
    // The mechanism stays free-text (the operator records exactly what they
    // signed) — whitespace-trimmed only, like the jurisdiction above, so the
    // certificate carries the label the operator typed.
    let mechanism = req.mechanism.clone().map(|m| m.trim().to_string());
    let jur_for_cert = jurisdiction.clone();
    let mech_for_cert = mechanism.clone();

    // a DSAR must cover every
    // registered domain, not just the global pool. In multi-db mode each
    // `brain-<domain>.db` runs its own locate + purge tx; in shim mode the
    // list is exactly the global pool (whose owner query already covers every
    // row of the one shared DB — byte-identical to the legacy shim behavior).
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
    // a DSAR (export|purge) is a capability-gated action on
    // the data surface; a roles-principal needs `dsar_export` in the allowlist.
    super::authorize_role(&principal.0, &pools[0].1, "dsar_export")?;
    // Global runs LAST so its ledger row can carry the cross-domain digest.
    pools.sort_by(|a, b| match (a.0 == "global", b.0 == "global") {
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        _ => std::cmp::Ordering::Equal,
    });

    let outcome = tokio::task::spawn_blocking(move || -> Result<DsarOutcome, HandlerError> {
        let now = chrono::Utc::now().timestamp();
        let dry_run = req.dry_run;
        let mut runs: Vec<crate::service::dsar::DsarRun> = Vec::with_capacity(pools.len());

        // 1+2. Non-global pools first: each locates + purges in its own tx and
        //      returns its bundle + purged ids for the aggregate. The core
        //      borrows one connection per pool; the pool handles stay here.
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
            let run = run_dsar_pool(
                pool,
                name,
                &subject,
                &action,
                dry_run,
                now,
                false,
                None,
                req.subject_exact,
            )?;
            if !dry_run {
                cross_ids.extend(run.purged_ids.iter().copied());
                if let Some(b) = &run.bundle {
                    cross_bundle.push((name.clone(), b.clone()));
                }
            }
            runs.push(run);
        }

        // 3. Aggregate digest for the global ledger row: in shim mode this is
        //    the single local bundle (byte-identical to the legacy hash); in
        //    multi-db mode it is SHA-256 over the joined per-domain bundles.
        let aggregate_hash = if !dry_run && pools.len() > 1 && !cross_bundle.is_empty() {
            Some(crate::handlers::gate::sha256_hex(
                &serde_json::json!({ "subject": subject, "domains": cross_bundle }).to_string(),
            ))
        } else {
            None // shim mode: the global run digests its own bundle
        };

        // 4. The global pool: locate + purge + the ledger row (atomic in its
        //    own tx, as the ledger-atomicity invariant requires).
        let global_run = run_dsar_pool(
            &pools[global_idx].1,
            "global",
            &subject,
            &action,
            dry_run,
            now,
            true,
            aggregate_hash.as_deref(),
            req.subject_exact,
        )?;
        runs.push(global_run);

        // 5. Dry-run preview: aggregate the read-only footprint across pools.
        if dry_run {
            let mut fp = crate::service::dsar::Footprint {
                roots: 0,
                derived: 0,
                export_rows: 0,
                tombstones: 0,
                dsar_rows: 0,
                workflow_rows: runs.iter().map(|r| r.workflow_rows).sum(),
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
        let global_conn = pools[global_idx].1.get().map_err(HandlerError::db_down)?;
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
        let mut found_count = 0;
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
        let certificate = certificate_json(
            &subject,
            &action,
            found_count,
            purged_ids,
            held,
            jur_for_cert.as_deref(),
            mech_for_cert.as_deref(),
            tombstone_root,
            &certified_at,
            chain_head,
            runs.last()
                .map(|r| r.remanence.as_str())
                .unwrap_or("logical (secure_delete off; WAL/freelist/backup copies may persist)"),
        );
        let ledger_id =
            ledger_id.ok_or_else(|| HandlerError::internal("no ledger row written".to_string()))?;

        // backfill the certificate onto the ledger row committed
        // with the purge (best-effort — the row + times already prove it).
        backfill_certificate(&global_conn, ledger_id, &subject, &certificate);

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
            jurisdiction: None,
            rights: Vec::new(),
            certificate: None,
            footprint: Some(fp.clone()),
        }));
    }
    let (id, subject, certificate, certified_at, created_at) = match outcome {
        DsarOutcome::Completed {
            id,
            subject,
            certificate,
            certified_at,
            created_at,
        } => (id, subject, certificate, certified_at, created_at),
        DsarOutcome::Footprint(_) => {
            return Err(HandlerError::internal(
                "dry_run returned no completion state".to_string(),
            ));
        }
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
        deadline: jurisdiction
            .as_deref()
            .map(|j| crate::transfers::dsar_deadline_for(created_at, j))
            .unwrap_or_else(|| crate::service::dsar::dsar_deadline(created_at)),
        jurisdiction,
        rights,
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
    Footprint(crate::service::dsar::Footprint),
}

/// Trim + validate a DSAR subject + action, shared by every DSAR surface so the
/// trust-boundary checks live in exactly one place (`subject_empty`,
/// `subject_too_long`, `invalid_action`).
fn normalize_dsar_subject(subject: &str, action: &str) -> Result<String, HandlerError> {
    let subject = subject.trim().to_string();
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
    if !matches!(action, "export" | "purge" | "both") {
        return Err(HandlerError::bad_request(
            "invalid_action",
            "dsar action must be export|purge|both",
        ));
    }
    Ok(subject)
}

/// Run a DSAR against ONE domain pool and produce the full `DsarResponse`
/// (certificate or dry-run footprint), jurisdiction-stamped. The shared seam
/// for the per-client surface — the real locate/purge/export/certificate +
/// legal-hold deferral all live in the core (`crate::service::dsar::run_pool`);
/// this only composes a single run with the caller's resolved pool +
/// jurisdiction + mechanism (no new purge path). The audit + chain anchor stay
/// on the global pool (the hash chain is the server's single registry of
/// record), while the ledger row lives in the run's domain pool.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_dsar_subject(
    state: Arc<AppState>,
    principal: OptPrincipal,
    pool: crate::Pool,
    domain: &str,
    subject: &str,
    action: &str,
    dry_run: bool,
    jurisdiction: Option<String>,
    mechanism: Option<String>,
    subject_exact: bool,
    now: i64,
) -> Result<DsarResponse, HandlerError> {
    let subject = subject.to_string();
    let action = action.to_string();
    let mech_for_cert = mechanism.map(|m| m.trim().to_string());
    let domain_for = domain.to_string();
    let state_for = state.clone();
    tokio::task::spawn_blocking(move || -> Result<DsarResponse, HandlerError> {
        let subject = normalize_dsar_subject(&subject, &action)?;
        super::authorize_role(&principal.0, &pool, "dsar_export")?;
        let rights: Vec<&'static str> = jurisdiction
            .as_deref()
            .and_then(crate::transfers::jurisdiction_rule)
            .map(|r| r.rights.to_vec())
            .unwrap_or_default();
        let run = run_dsar_pool(
            &pool,
            &domain_for,
            &subject,
            &action,
            dry_run,
            now,
            true,
            None,
            subject_exact,
        )?;
        if dry_run {
            return Ok(DsarResponse {
                id: 0,
                subject: String::new(),
                status: "preview",
                created_at: 0,
                deadline: 0,
                jurisdiction: None,
                rights: Vec::new(),
                certificate: None,
                footprint: Some(crate::service::dsar::Footprint {
                    roots: run.roots,
                    derived: run.derived,
                    export_rows: run.export_rows,
                    tombstones: run.tombstones,
                    dsar_rows: run.dsar_rows,
                    workflow_rows: run.workflow_rows,
                    dry_run: true,
                }),
            });
        }
        let ledger_id = run
            .ledger_id
            .ok_or_else(|| HandlerError::internal("no ledger row written".to_string()))?;
        // Audit + chain anchor on the global pool (the registry of record).
        // Scoped so the conn is dropped before the domain conn is acquired
        // below — in shim mode both resolve to the SAME r2d2 pool, so holding
        // both at once could exceed a `max_size(1)` pool and deadlock.
        let chain_head = {
            let g = state_for.pool.get().map_err(HandlerError::db_down)?;
            crate::audit::record(
                &g,
                crate::audit::AuditKind::Client,
                "api",
                &format!("client-dsar:{subject}"),
                crate::audit::AuditStatus::Ok,
                "dsar",
            );
            crate::audit::chain_head(&g)
        };
        let certified_at = chrono::Utc::now().to_rfc3339();
        let certificate = certificate_json(
            &subject,
            &action,
            run.roots + run.derived,
            run.purged_ids,
            run.held,
            jurisdiction.as_deref(),
            mech_for_cert.as_deref(),
            run.tombstone_root,
            &certified_at,
            chain_head,
            &run.remanence,
        );
        let conn = pool.get().map_err(HandlerError::db_down)?;
        backfill_certificate(&conn, ledger_id, &subject, &certificate);
        let deadline = jurisdiction
            .as_deref()
            .map(|j| crate::transfers::dsar_deadline_for(now, j))
            .unwrap_or_else(|| crate::service::dsar::dsar_deadline(now));
        let cert: serde_json::Value = serde_json::from_str(&certificate)
            .map_err(|_| HandlerError::internal("stored certificate is not valid JSON"))?;
        Ok(DsarResponse {
            id: ledger_id,
            subject,
            status: "completed",
            created_at: now,
            deadline,
            jurisdiction,
            rights,
            certificate: Some(cert),
            footprint: None,
        })
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))?
}

// ---------------------------------------------------------------------------
// DSAR ledger list
// ---------------------------------------------------------------------------

/// `GET /dsar?limit=&offset=` query.
#[derive(Debug, Default, Deserialize)]
pub struct DsarLedgerQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// `GET /dsar` — the DSAR ledger list (Admin). Past DSAR requests
/// were only visible via the `/audit` side-channel; this is the first-class
/// registry the client countdown renders. Bounded (default 100, clamped to
/// `MAX_MULTI_GET`), newest-first (`ORDER BY id DESC`), the audit pagination
/// idiom. Storage lives in the core's `list_dsar_page`.
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
        let conn = pool.get().map_err(HandlerError::db_down)?;
        list_dsar_page(&conn, limit, offset).map_err(HandlerError::from)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(body))
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
    let limit = q
        .limit
        .map(|l| l.clamp(1, crate::service::dsar::MAX_TOMBSTONES))
        .unwrap_or(100);
    // tenant scoping. A non-superuser admin only sees tombstones
    // whose `reason` (owner:<subject>) matches their own `sub`. Superuser
    // (`None` principal — opaque/loopback) is unconstrained. The query
    // caller-filter takes precedence if it's narrower than the principal scope.
    let tenant_filter: Option<String> = principal.0.as_ref().map(|p| format!("owner:{}", p.sub));
    let body = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        tombstones_page(&conn, subject, since, limit, tenant_filter).map_err(HandlerError::from)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(body))
}

/// `GET /dsar/{id}/certificate` — re-fetch a past deletion certificate. The
/// stored `chain_head` is the audit-chain link at certification time; the
/// response recomputes `verify_chain` live so the caller sees whether the
/// chain the certificate anchored to still holds. Row fetch + tenant gate +
/// chain verification live in the core's `certificate_view`.
pub async fn get_dsar_certificate(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, Some("global"))?;
    // tenant scoping. A non-superuser admin can only fetch
    // certificates for their own subject. The stored `dsar_requests.subject`
    // is checked against the principal's `sub`; a mismatch → 404 (don't leak
    // existence of another tenant's certificate).
    let tenant_sub: Option<String> = principal.0.as_ref().map(|p| p.sub.clone());
    let body = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        certificate_view(&conn, id, tenant_sub).map_err(HandlerError::from)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(body))
}

// ---------------------------------------------------------------------------
// Art 19 webhook (opt-in, handler-owned egress) + the per-pool seam
// ---------------------------------------------------------------------------

/// Art 19 onward-notification. When
/// `BRAIN_DSAR_WEBHOOK_URL` is set, a completed DSAR purge POSTs
/// `{subject, certified_at, certificate_id}` to the URL, HMAC-SHA256-signed
/// (`X-Brain-Signature-256: sha256=<hex>`) when `BRAIN_DSAR_WEBHOOK_SECRET`
/// is set. Fail-soft: bounded retries, then a logged warning — a webhook
/// failure NEVER rolls back the purge.
pub fn notify_art19(subject: String, certificate_id: i64, certified_at: String) {
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
            if let Some(secret) = crate::config::dsar_webhook_secret()
                && let Some(sig) = hmac_hex(secret.as_bytes(), payload.as_bytes())
            {
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
/// `None` on an invalid key length — the caller signs with no header
/// (fail-soft, matching `notify_art19`'s never-rolls-back posture).
fn hmac_hex(secret: &[u8], body: &[u8]) -> Option<String> {
    use hmac::{Hmac, KeyInit, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    // HMAC accepts any key length — construction cannot fail.
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts keys of any length");
    mac.update(body);
    Some(hex::encode(mac.finalize().into_bytes()))
}

/// Run one pool's DSAR through the core. The THIN per-pool seam: borrow a
/// connection from the caller's resolved pool and hand it to
/// `crate::service::dsar::run_pool` — the pool handle never crosses into the
/// core, and every statement (locate / export / purge / sweeps / ledger /
/// pragma posture / checkpoint) lives there now.
#[allow(clippy::too_many_arguments)] // 9 run fields; a struct would add ceremony to the single-erasure path
fn run_dsar_pool(
    pool: &crate::Pool,
    domain: &str,
    subject: &str,
    action: &str,
    dry_run: bool,
    now: i64,
    write_ledger: bool,
    aggregate_bundle_hash: Option<&str>,
    subject_exact: bool,
) -> Result<crate::service::dsar::DsarRun, HandlerError> {
    let mut conn = pool.get().map_err(HandlerError::db_down)?;
    crate::service::dsar::run_pool(
        &mut conn,
        domain,
        subject,
        action,
        dry_run,
        now,
        write_ledger,
        aggregate_bundle_hash,
        subject_exact,
    )
    .map_err(HandlerError::from)
}
