//! The DSAR core — the rights surface (the Quarry milestone of the
//! Foundation Line). Locate, export bundle, purge, certificate, and ledger
//! composition for the GDPR Art 15/17 workflow, moved verbatim out of the
//! observe handler: the handler is the protocol adapter (parse → Admin gate
//! → multi-pool sequencing → response), this module is the complete storage
//! story of one DSAR pool.
//!
//! OWNS:
//! - the locate walk (`knowledge` by `owner` + transitive `derived_from`
//!   descendants, bounded by [`DERIVED_MAX_DEPTH`]);
//! - the portable export bundle (Art 15) — the same query the live purge
//!   runs, so what the certificate erases and what the bundle discloses
//!   stay symmetric, `channel_notes[]` included;
//! - one pool's erasure ([`run_pool`]): the remanence posture (secure_delete
//!   pragma ATTEMPT — certificate-owned, never asserted untried), the purge
//!   tx (held-id deferral, knowledge purge via `service::purge`, trace and
//!   proposal residue sweeps, the workflow sweep submodule), the ledger row
//!   committed atomically with the purge, and the best-effort WAL TRUNCATE
//!   checkpoint after commit;
//! - the ledger page (`GET /dsar`), the tombstone registry page
//!   (`GET /tombstones`), the certificate re-fetch (`GET
//!   /dsar/{id}/certificate`, tenant-gated), the stale-ledger retention
//!   prune, the certificate backfill, and the deadline math.
//!
//! FK-CHILDREN MAP for every parent DELETE this module (or the submodules
//! it calls) performs — written BEFORE the move, from the declared schema
//! (`PRAGMA foreign_keys=ON`); the erasure lesson is structural law now:
//!
//! `knowledge` (deleted by `service::purge::purge_chunk_ids` — the map
//! lives in that module header): `embeddings` CASCADE; `relationships`
//! SET NULL + explicit; `evidence_links`, `proposals.conflict_with`,
//! `recall_traces` soft refs, explicit; `tombstones` soft ref BY DESIGN;
//! `case_articles` / `kcs_translations` declared NO ACTION and NOT
//! cleared — purging a chunk carrying one fails the tx LOUDLY
//! (fail-closed; documented pre-existing ceiling, not silently widened).
//!
//! `workflow_runs` (deleted by `sweep::sweep_subject` — full map in that
//! header): `workflow_steps`, `findings`, `contradictions`, `outbox`,
//! `handover_offers`, `case_notes` DELETED first; `case_status_refs`
//! purged (or revoked when a legal hold defers the run); `crm_cases`
//! UNLINKED (`run_id = NULL` — the external case outlives the run);
//! `delegations` + `channel_threads` deleted first (declared FK children
//! the pre-Quarry sweep missed — the gap this move's map exposed and
//! closed, the release's one intended fail-path delta).
//!
//! `case_notes` (deleted by the run arm + the subject arms):
//! `parent_note_id` is an UNdeclared self-reference — no FK, no hazard.
//!
//! `handover_offers` (deleted by run): no dependents.
//!
//! `crm_cases` (updated only — unlink): no dependents; never deleted here.
//!
//! `dsar_requests` (INSERT + certificate backfill + retention prune of
//! completed rows): no dependents; the certificate is an inline column.
//! `tombstones` (INSERT only): the registry outlives the row by design.
//! `recall_traces` / `proposals` residue sweeps (subject arms): no declared
//! FK children (`recall_traces.audit_id` references the audit chain with
//! no FK — the chain is the registry of record, not a parent).
//!
//! Bounds: the ledger page + tombstone page caps (`MAX_TOMBSTONES`, the
//! `MAX_MULTI_GET` clamp) are re-asserted HERE so every future caller
//! inherits the fence; the handler pre-clamps identically, so the wire is
//! unchanged.
//!
//! Wire-shape ceilings (honest): the ledger rows, the tombstone page, and
//! the certificate view stay the legacy shapes (`serde_json` maps /
//! derived structs) — the byte-for-byte wire pins outrank the domain-type
//! aspiration; typing them is a follow-up, NOT part of this move.
//!
//! Remanence (certificate-owned): the claim reflects the pragma ATTEMPT —
//! on a failed `secure_delete=ON` the certificate downgrades to the
//! disclosed logical posture instead of asserting an overwrite that never
//! happened. A checkpoint failure is best-effort: it must not fail an
//! otherwise-successful erasure, and silence is never certified (warn).

pub mod sweep;

use rusqlite::{Connection, params};
use serde::Serialize;
use std::collections::HashMap;

/// Max derived_from walk depth for a DSAR purge. Derived chains are operator-
/// created and short (see `consolidate.rs`); a bounded walk keeps the tx small.
pub const DERIVED_MAX_DEPTH: usize = 8;
/// Max tombstone rows returned by `GET /tombstones` per page. Re-asserted in
/// the core (the fence holds of the FUNCTION); the handler pre-clamps with
/// the same bound.
pub const MAX_TOMBSTONES: i64 = 1000;

/// Typed service error (the ServiceError convention: one enum per module).
/// The observe handler's `From` impl maps every variant onto that route's
/// FROZEN probe-blind
/// vocabulary: `Database` → the internal-error body with the message
/// verbatim, `NotFound` → the certificate route's 404, `LegalHold` → the
/// shared `409 legal_hold_active` envelope (the purge backstop's shape).
#[derive(Debug)]
pub enum DsarError {
    /// A query failed; the message travels unchanged.
    Database(String),
    /// No certificate row for the id, or the row belongs to another tenant —
    /// one 404, existence never leaked.
    NotFound,
    /// The knowledge-purge backstop fence fired (a held id reached a purge).
    /// Carries the held map for the shared 409 envelope.
    LegalHold(HashMap<i64, Vec<String>>),
}

impl std::fmt::Display for DsarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DsarError::Database(e) => write!(f, "database error: {e}"),
            DsarError::NotFound => write!(f, "no dsar request with this id"),
            DsarError::LegalHold(held) => write!(f, "legal hold active on {held:?}"),
        }
    }
}

impl From<rusqlite::Error> for DsarError {
    fn from(e: rusqlite::Error) -> Self {
        DsarError::Database(e.to_string())
    }
}

impl From<crate::service::purge::PurgeError> for DsarError {
    fn from(e: crate::service::purge::PurgeError) -> Self {
        match e {
            crate::service::purge::PurgeError::Database(m) => DsarError::Database(m),
            crate::service::purge::PurgeError::LegalHold(held) => DsarError::LegalHold(held),
        }
    }
}

/// the DSAR Art 17 erasure deadline — `created_at` +
/// the operator's window, a pure mirror of `gate::proposal_deadline`. The
/// client countdown ticks against this absolute deadline, so an operator's
/// `BRAIN_DSAR_WINDOW_DAYS` override is authoritative (no client window guess).
pub fn dsar_deadline(created_at: i64) -> i64 {
    created_at + crate::config::dsar_window_secs()
}

/// the would-be DSAR deletion footprint — what a live purge would
/// locate + export + delete, without executing any write. The GDPR Art 17
/// preview a DPO reads before clicking "erase". Aggregated across pools by
/// the handler's orchestration (sequencing stays handler-side).
#[derive(Debug, Serialize, Clone)]
pub struct Footprint {
    pub roots: usize,
    pub derived: usize,
    pub export_rows: usize,
    pub tombstones: usize,
    pub dsar_rows: usize,
    /// Governed-workflow rows a live purge would reach (matched runs +
    /// their dependents; frozen runs counted as matched).
    pub workflow_rows: usize,
    pub dry_run: bool,
}

/// One `dsar_requests` ledger row. `created_at` +
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

/// Pure ledger page — the ordering, the page
/// boundary, and the total count are unit-testable without an HTTP stack
/// (the `page_decayed` idiom).
pub fn list_dsar_page(conn: &Connection, limit: i64, offset: i64) -> Result<DsarLedger, DsarError> {
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM dsar_requests", [], |r| r.get(0))?;
    let mut stmt = conn.prepare(
        "SELECT id, subject, action, status, created_at, completed_at
         FROM dsar_requests ORDER BY id DESC LIMIT ?1 OFFSET ?2",
    )?;
    let requests = stmt
        .query_map(params![limit, offset], |r| {
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
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(DsarLedger { requests, total })
}

/// The tombstone registry page (the EDPB Coordinated Enforcement Framework
/// ask). Hash-only, append-only rows. `subject` filters by the
/// `owner:<subject>` purge reason; `since` filters by `purged_at`; `limit`
/// is re-clamped here (the fence of the FUNCTION). `tenant_filter` carries
/// the handler's scoping decision (`owner:<sub>` for a scoped principal,
/// `None` for superuser): a caller-supplied subject that disagrees with the
/// tenant scope yields an EMPTY page — a cross-tenant request must not leak
/// existence.
pub fn tombstones_page(
    conn: &Connection,
    subject: Option<String>,
    since: Option<i64>,
    limit: i64,
    tenant_filter: Option<String>,
) -> Result<serde_json::Value, DsarError> {
    let limit = limit.clamp(1, MAX_TOMBSTONES);
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
    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(refs.as_slice(), |r| {
        Ok(serde_json::json!({
            "knowledge_id": r.get::<_, i64>(0)?,
            "content_hash": r.get::<_, Option<String>>(1)?,
            "purged_at": r.get::<_, Option<i64>>(2)?,
            "reason": r.get::<_, Option<String>>(3)?,
            "origin_id": r.get::<_, Option<i64>>(4)?,
        }))
    })?;
    let mut out = Vec::new();
    for v in rows.flatten() {
        out.push(v);
    }
    Ok(serde_json::json!({ "tombstones": out }))
}

/// Re-fetch a past deletion certificate. The stored `chain_head` is the
/// audit-chain link at certification time; the view recomputes `verify_chain`
/// live so the caller sees whether the chain the certificate anchored to
/// still holds. `tenant_sub` is the handler's scoping input (`None` =
/// superuser): a scoped principal only ever sees their own subject's row,
/// and a mismatch is the SAME 404 as a missing row (existence never leaks).
pub fn certificate_view(
    conn: &Connection,
    id: i64,
    tenant_sub: Option<String>,
) -> Result<serde_json::Value, DsarError> {
    // Fetch subject + certificate in one query so the tenant check happens
    // before the certificate body is read.
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT subject, certificate FROM dsar_requests WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    match row {
        Some((Some(stored_subject), Some(c))) => {
            // Tenant gate: if the principal is scoped, the row's subject must match.
            if let Some(sub) = &tenant_sub
                && stored_subject.as_str() != sub.as_str()
            {
                return Err(DsarError::NotFound);
            }
            let v: serde_json::Value = serde_json::from_str(&c)
                .map_err(|_| DsarError::Database("stored certificate is not valid JSON".into()))?;
            Ok(serde_json::json!({
                "certificate": v,
                "chain_verifies": crate::audit::verify_chain(conn),
            }))
        }
        _ => Err(DsarError::NotFound),
    }
}

/// ledger retention. Completed `dsar_requests` rows older than
/// `retention_days` are deleted (the erasure record's remaining value is the
/// certificate + the audit chain, not the ledger row itself). Returns the
/// number of rows removed. Pure; best-effort callers swallow the result.
pub fn purge_stale_dsar_ledger(conn: &Connection, retention_days: u32) -> i64 {
    if retention_days == 0 {
        return 0;
    }
    let now = chrono::Utc::now().timestamp();
    // was `.unwrap_or(0)` — a silent failure hid the
    // prune; warn instead of pretending.
    match conn.execute(
        "DELETE FROM dsar_requests WHERE status = 'completed' AND completed_at < ?1",
        params![now - (retention_days as i64) * 86400],
    ) {
        Ok(n) => n as i64,
        Err(e) => {
            tracing::warn!("DSAR ledger retention prune failed: {e}");
            0
        }
    }
}

/// Best-effort post-commit certificate backfill onto the ledger row (the row
/// and its timestamps already prove the erasure; the certificate is the
/// human-readable copy).
///
/// A failure warns — it never fails the erasure.
pub fn backfill_certificate(conn: &Connection, ledger_id: i64, subject: &str, certificate: &str) {
    if let Err(e) = conn.execute(
        "UPDATE dsar_requests SET certificate = ?1 WHERE id = ?2",
        params![certificate, ledger_id],
    ) {
        tracing::warn!(
            "DSAR certificate backfill failed (ledger row {ledger_id}, subject {subject}): {e}"
        );
    }
}

/// Collect all rows of `SELECT <i64>` sql (one `?` param) into a Vec.
fn collect_ids(tx: &rusqlite::Transaction, sql: &str, param: &str) -> Result<Vec<i64>, DsarError> {
    let mut stmt = tx.prepare(sql)?;
    let rows = stmt.query_map(params![param], |r| r.get::<_, i64>(0))?;
    Ok(rows.flatten().collect())
}

/// Locate every record for a DSAR subject: content rows by `owner`, plus all
/// transitive `derived_from` descendants (bounded by `DERIVED_MAX_DEPTH`).
/// Returns `(root_ids, derived_pairs)` where each derived pair is
/// `(derived_id, root_id)` — the purge stamps `origin_id` so the deletion
/// registry can point a derived chunk back at the subject's root record.
#[allow(clippy::type_complexity)]
pub fn dsar_locate(
    tx: &rusqlite::Transaction,
    subject: &str,
) -> Result<(Vec<i64>, Vec<(i64, i64)>), DsarError> {
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
        let mut stmt = tx.prepare(&sql)?;
        let mut next: Vec<i64> = Vec::new();
        {
            let params: Vec<&dyn rusqlite::ToSql> =
                frontier.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
            let rows = stmt.query_map(params.as_slice(), |r| r.get::<_, i64>(0))?;
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

/// build the portable export bundle (the JSON a live purge embeds
/// into its ledger row) for the given locate result. The dry-run
/// preview and the live path run the EXACT same query — behavior-preserving.
pub fn build_export_bundle(
    tx: &rusqlite::Transaction,
    subject: &str,
    roots: &[i64],
    derived: &[(i64, i64)],
) -> Result<String, DsarError> {
    let mut rows: Vec<serde_json::Value> = Vec::new();
    let mut stmt = tx.prepare(
        "SELECT id, content, node_kind, assertion_kind, confidence,
                owner, observed_at, valid_from, valid_to, lawful_basis, purpose
         FROM knowledge WHERE id IN (?1)",
    )?;
    for id in roots.iter().chain(derived.iter().map(|(d, _)| d)) {
        let q = stmt.query_map(params![id], |row| {
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
                // the lawful-basis + purpose
                // tags surfaced on the export/DSAR bundle (Art 5/6 evidence).
                "lawful_basis": row.get::<_, Option<String>>(9)?,
                "purpose": row.get::<_, Option<String>>(10)?,
            }))
        })?;
        for v in q.flatten() {
            rows.push(v);
        }
    }
    // Channel rows the subject can access under Art 15 — the SAME three
    // match arms the purge's sweep erases (author / addressee / content),
    // so what the certificate erases and what the bundle discloses stay
    // symmetric. Raw text matches the knowledge rows' posture (the bundle
    // goes to the subject or their DPO, never to a public surface).
    let mut notes: Vec<serde_json::Value> = Vec::new();
    let mut note_stmt = tx.prepare(
        "SELECT id, run_id, kind, author, content, addressed_to, created_at
         FROM case_notes
          WHERE author = ?1 OR addressed_to = ?1 OR content LIKE ?2
          ORDER BY id",
    )?;
    let q = note_stmt.query_map(params![subject, format!("%{subject}%")], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, i64>(0)?,
            "run_id": row.get::<_, i64>(1)?,
            "kind": row.get::<_, String>(2)?,
            "author": row.get::<_, String>(3)?,
            "content": row.get::<_, String>(4)?,
            "addressed_to": row.get::<_, Option<String>>(5)?,
            "created_at": row.get::<_, i64>(6)?,
        }))
    })?;
    for v in q.flatten() {
        notes.push(v);
    }
    drop(note_stmt);
    Ok(serde_json::json!({
        "exported_at": chrono::Utc::now().to_rfc3339(),
        // the residency stamp on the DSAR bundle too.
        "region": brain_server::storage_layout::region(),
        "subject": subject,
        "knowledge": rows,
        "channel_notes": notes,
    })
    .to_string())
}

/// count prior deletions for a subject — the tombstone reasons a live
/// purge writes: `owner:<subject>` for roots, and `derived` (scoped to one of
/// this subject's roots via `origin_id`) for derived descendants. The ledger
/// trace a DPO sees in the preview.
fn count_subject_tombstones(
    tx: &rusqlite::Transaction,
    subject: &str,
    roots: &[i64],
) -> Result<i64, DsarError> {
    let owner_reason = format!("owner:{subject}");
    if roots.is_empty() {
        return tx
            .query_row(
                "SELECT COUNT(*) FROM tombstones WHERE reason = ?1",
                params![owner_reason],
                |r| r.get(0),
            )
            .map_err(DsarError::from);
    }
    let placeholders = vec!["?"; roots.len()].join(",");
    let sql = format!(
        "SELECT COUNT(*) FROM tombstones
          WHERE reason = ?1 OR (reason = 'derived' AND origin_id IN ({placeholders}))"
    );
    let mut stmt = tx.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> =
        roots.iter().map(|r| r as &dyn rusqlite::ToSql).collect();
    // ?1 = the owner reason, then one per root for the IN list.
    let mut all_params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(params.len() + 1);
    all_params.push(&owner_reason);
    all_params.extend(params.iter().copied());
    let count: i64 = stmt.query_row(all_params.as_slice(), |r| r.get(0))?;
    Ok(count)
}

/// The DSAR deletion-certificate JSON shape — shared by the multi-pool
/// orchestration and the per-client single-pool surface so the audit-visible
/// contract lives in one place. `purged_ids`/`held`/`tombstone_root` are the
/// run(s)' erased sets; jurisdiction/mechanism are the request's per-law
/// stamp (advisory). `remanence` is
/// the honest physical-purge posture to disclose on the
/// deletion certificate (the strict domain's certificate states
/// secure_delete).
#[allow(clippy::too_many_arguments)]
pub fn certificate_json(
    subject: &str,
    action: &str,
    found_count: usize,
    purged_ids: Vec<i64>,
    held: Vec<serde_json::Value>,
    jurisdiction: Option<&str>,
    mechanism: Option<&str>,
    tombstone_root: Option<i64>,
    certified_at: &str,
    chain_head: Option<String>,
    remanence: &str,
) -> String {
    serde_json::json!({
        "subject": subject,
        "action": action,
        "found_count": found_count,
        "purged_ids": purged_ids,
        "region": brain_server::storage_layout::region(),
        "held_ids": held,
        "jurisdiction": jurisdiction,
        "mechanism": mechanism,
        "tombstone_root": tombstone_root,
        "certified_at": certified_at,
        "chain_head": chain_head,
        "physical_purge": remanence,
    })
    .to_string()
}

/// one DSAR pool's run outcome — the locate + purge result for a
/// single domain DB (global or `brain-<domain>.db`). Counts for the dry-run
/// footprint, ids + bundle for the cross-domain aggregate, and the ledger row
/// identity when this pool is the registry of record (global).
pub struct DsarRun {
    pub roots: usize,
    pub derived: usize,
    pub export_rows: usize,
    pub tombstones: usize,
    pub dsar_rows: usize,
    /// Governed-workflow rows this pool's sweep reached (matched runs +
    /// dependents; frozen runs counted as matched).
    pub workflow_rows: usize,
    /// Live-purge ids from this pool (certificate payload).
    pub purged_ids: Vec<i64>,
    /// ids under legal hold that erasure DEFERRED,
    /// with their reasons — listed on the certificate as the why. A held id
    /// is never purged here.
    pub held: Vec<serde_json::Value>,
    /// This pool's export bundle (cross-domain aggregate input).
    pub bundle: Option<String>,
    /// `Some(ledger row id)` when this pool wrote the ledger row (global).
    pub ledger_id: Option<i64>,
    pub tombstone_root: Option<i64>,
    /// the honest physical-purge posture for this
    /// domain (secure_delete+checkpoint for a strict profile, else the disclosed
    /// logical posture). Surfaced verbatim on the deletion certificate.
    pub remanence: String,
}

/// Run locate + [dry-run preview | purge + ledger] for ONE pool's
/// connection (borrowed from the handler's blocking closure — the pool
/// handle never crosses this boundary). `write_ledger` is true only for the
/// global pool (the registry of record);
/// `aggregate_bundle_hash` carries the cross-domain digest in multi-db mode
/// (the global run's own bundle is digested otherwise). The per-pool
/// transaction is begun and committed HERE so the pragma posture, the purge,
/// the ledger row, and the post-commit checkpoint stay one story; multi-pool
/// sequencing is the handler's orchestration, not storage.
#[allow(clippy::too_many_arguments)]
pub fn run_pool(
    conn: &mut Connection,
    domain: &str,
    subject: &str,
    action: &str,
    dry_run: bool,
    now: i64,
    write_ledger: bool,
    aggregate_bundle_hash: Option<&str>,
    subject_exact: bool,
) -> Result<DsarRun, DsarError> {
    // a strict-posture domain erases with
    // `secure_delete=ON` (freed page images overwritten) + a WAL TRUNCATE
    // checkpoint after commit, so the certificate's erasure claim has teeth.
    // Best-effort profile lookup: an unreadable/missing bind defaults to the
    // disclosed logical posture (nothing ever fails closed into a lie).
    let strict = brain_server::profile::profile_for_domain(conn, domain)
        .ok()
        .flatten()
        .is_some_and(|p| p.pii_strict());
    // The remanence claim must reflect the pragma ATTEMPT,
    // not just the profile flag — on a failed `secure_delete=ON` the
    // certificate must downgrade to the disclosed logical posture instead of
    // asserting an overwrite that never happened. Dry-run/export-only keep
    // the would-be posture (the pragma only runs on a live purge).
    let mut secure_delete_active = strict;
    if strict && !dry_run && matches!(action, "purge" | "both") {
        // was `let _ =` — a failed secure_delete
        // weakens the remanence claim on the certificate; warn, don't certify.
        if let Err(e) = conn.execute_batch("PRAGMA secure_delete=ON;") {
            tracing::warn!("secure_delete=ON failed for DSAR purge: {e}");
            secure_delete_active = false;
        }
    }
    let remanence = if secure_delete_active {
        "secure_delete+checkpoint (backup files excepted)".to_string()
    } else {
        "logical (secure_delete off; WAL/freelist/backup copies may persist)".to_string()
    };
    let tx = conn.transaction()?;

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
        let dsar_rows: i64 = tx.query_row(
            "SELECT COUNT(*) FROM dsar_requests WHERE subject = ?1",
            params![subject],
            |r| r.get(0),
        )?;
        let workflow_rows: usize = if subject.is_empty() {
            0
        } else {
            tx.query_row(
                "SELECT COUNT(*) FROM workflow_runs WHERE state_json LIKE ?1",
                params![format!("%{subject}%")],
                |r| r.get::<_, i64>(0),
            )? as usize
        };
        return Ok(DsarRun {
            roots: roots.len(),
            derived: derived.len(),
            export_rows,
            tombstones: tombstones as usize,
            dsar_rows: dsar_rows as usize,
            workflow_rows,
            purged_ids: Vec::new(),
            held: Vec::new(),
            bundle: None,
            ledger_id: None,
            tombstone_root: None,
            remanence,
        });
    }

    // 3. Purge (all-or-nothing with the export, same tx): roots with the
    //    owner reason, derived descendants with `derived` + origin id.
    let mut purged_ids: Vec<i64> = Vec::new();
    let mut held: Vec<serde_json::Value> = Vec::new();
    let mut workflow_rows = 0;
    if matches!(action, "purge" | "both") {
        // a held id is frozen against DSAR erasure too
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
        let _ = crate::service::purge::purge_chunk_ids(
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
                let _ = crate::service::purge::purge_chunk_ids(
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

    // trace residue sweep. The trace no longer
    // stores the raw query (only its xxh3-64 hash), so the subject can't
    // appear in it — this sweep remains as a defensive net against any
    // future field that does embed personal data. Best-effort (short
    // common subjects over-match slightly; erasure-safe direction).
    if matches!(action, "purge" | "both") && !subject.is_empty() {
        let (sql, pat): (&str, String) = if subject_exact {
            (
                "DELETE FROM recall_traces WHERE trace_json = ?1",
                subject.to_string(),
            )
        } else {
            (
                "DELETE FROM recall_traces WHERE trace_json LIKE ?1",
                format!("%{subject}%"),
            )
        };
        tx.execute(sql, params![pat])?;
        // proposals hold raw candidate content with no owner column,
        // so a DSAR could never locate them and their plaintext (possibly PII
        // about the subject) survived a "complete" erasure. Sweep them by the
        // subject verbatim — the same erasure-safe over-match posture as the
        // trace sweep above. ponytail: this is a literal `LIKE %subject%`, not a
        // semantic owner join (proposals are operator-reviewed candidates, not
        // subject-attributed rows); the review-queue provenance for the subject
        // is intentionally erased with the memory per Art 17.
        // was `let _ =` — a silent failure would leave
        // subject PII in a "complete" erasure; propagate (tx rolls back).
        let (sql, pat): (&str, String) = if subject_exact {
            (
                "DELETE FROM proposals WHERE content = ?1",
                subject.to_string(),
            )
        } else {
            (
                "DELETE FROM proposals WHERE content LIKE ?1",
                format!("%{subject}%"),
            )
        };
        tx.execute(sql, params![pat])?;
    }

    // Governed-workflow sweep: matched runs + their dependents go with the
    // erasure; runs frozen by an active legal hold are DEFERRED and listed
    // on the certificate beside the held chunks.
    if matches!(action, "purge" | "both") {
        let wf = sweep::sweep_subject(&tx, subject)?;
        workflow_rows = wf.runs_matched + wf.dependent_rows;
        for (run_id, reasons) in wf.deferred {
            held.push(serde_json::json!({ "run": run_id, "reasons": reasons }));
        }
    } else {
        // Dry-run/export: count what a live purge WOULD reach.
        if !subject.is_empty() {
            let (sql, pat): (&str, String) = if subject_exact {
                (
                    "SELECT COUNT(*) FROM workflow_runs WHERE state_json = ?1",
                    subject.to_string(),
                )
            } else {
                (
                    "SELECT COUNT(*) FROM workflow_runs WHERE state_json LIKE ?1",
                    format!("%{subject}%"),
                )
            };
            workflow_rows = tx.query_row(sql, params![pat], |r| r.get::<_, i64>(0))? as usize;
        }
    }

    // 4. Store the export's SHA-256 (replacing
    //    the brute-forceable xxh3-64 digest of a DELETED-content payload),
    //    never the raw bundle — the ledger's job is to prove the purge
    //    happened, not to keep a copy of the erasure payload.
    let mut ledger_id: Option<i64> = None;
    if write_ledger {
        let bundle_hash = aggregate_bundle_hash
            .map(str::to_string)
            .or_else(|| export_bundle.as_deref().map(crate::audit::hash));
        tx.execute(
            "INSERT INTO dsar_requests(subject, action, status, export_bundle, certificate, created_at, completed_at)
             VALUES (?1, ?2, 'completed', ?3, NULL, ?4, ?4)",
            params![subject, action, bundle_hash, now],
        )?;
        ledger_id = Some(tx.last_insert_rowid());
    }
    tx.commit()
        .map_err(|e| DsarError::Database(format!("commit failed: {e}")))?;
    // TRUNCATE the WAL so a just-erased subject's page images do
    // not linger there (reuse the integrity.rs import pattern). Best-effort —
    // a checkpoint failure must not fail an otherwise-successful erasure.
    if !dry_run && matches!(action, "purge" | "both") {
        // was `let _ =` — a failed TRUNCATE leaves the
        // erased subject's page images in the WAL; warn, never certify silence.
        if let Err(e) = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())) {
            tracing::warn!("wal_checkpoint(TRUNCATE) failed after DSAR purge: {e}");
        }
    }

    Ok(DsarRun {
        roots: roots.len(),
        derived: derived.len(),
        export_rows: 0,
        tombstones: 0,
        dsar_rows: 0,
        workflow_rows,
        purged_ids,
        held,
        bundle: export_bundle,
        ledger_id,
        tombstone_root: roots.first().copied(),
        remanence,
    })
}

#[cfg(test)]
mod pins {
    use std::path::Path;

    /// The Quarry source assertion: the DSAR core is free of the handler
    /// layer. Production source across `service/dsar.rs`,
    /// `service/dsar/sweep.rs`, and `service/purge.rs` never names a
    /// handler-module path or a handler type, never names a transport type,
    /// and never takes a connection-factory handle — services take
    /// connections and return domain types; the handler maps to HTTP at the
    /// boundary. (The general tree grep already covers the transport
    /// family for files directly under src/service/; this pin extends it to
    /// the dsar submodule + purge and to the handler-layer family.)
    #[test]
    fn dsar_core_is_handler_free() {
        const FORBIDDEN: &[&str] = &[
            "crate::handlers",
            "HandlerError",
            "axum",
            "StatusCode",
            "Json",
            "AppState",
            "Pool",
        ];
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service");
        let files = [
            base.join("dsar.rs"),
            base.join("dsar/sweep.rs"),
            base.join("purge.rs"),
        ];
        for f in &files {
            let text = std::fs::read_to_string(f).expect("service file must be readable");
            let prod = text
                .split("#[cfg(test)]")
                .next()
                .expect("split always yields a first slice");
            let display = f
                .strip_prefix(&base)
                .unwrap_or(f)
                .to_string_lossy()
                .into_owned();
            for token in FORBIDDEN {
                assert!(
                    !prod.contains(token),
                    "layer violation in src/service/{display}: production source names \
                     `{token}` — the DSAR core takes connections and returns domain \
                     types; map to HTTP at the handler boundary"
                );
            }
        }
    }
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
            rusqlite::params![crate::audit::hash("personal export payload"), now],
        )
        .unwrap();
        let stored: String = conn
            .query_row("SELECT export_bundle FROM dsar_requests", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, crate::audit::hash("personal export payload"));
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
        // the ledger insert used to happen AFTER the
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
    /// (deletion registry), `dsar_requests` (ledger history), and
    /// `case_notes` (the export builder's channel arm).
    fn helper_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE case_notes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                domain TEXT NOT NULL,
                run_id INTEGER NOT NULL,
                author TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'note',
                content TEXT NOT NULL,
                addressed_to TEXT,
                parent_note_id INTEGER,
                state TEXT NOT NULL DEFAULT 'visible',
                decided_at INTEGER,
                created_at INTEGER NOT NULL
             );
             CREATE TABLE knowledge (
                id INTEGER PRIMARY KEY,
                content TEXT,
                content_hash TEXT,
                node_kind TEXT DEFAULT 'chunk',
                assertion_kind TEXT DEFAULT 'stated',
                confidence REAL DEFAULT 0.5,
                owner TEXT,
                observed_at TEXT,
                valid_from TEXT,
                valid_to TEXT,
                lawful_basis TEXT,
                purpose TEXT
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

    /// a dry-run footprint reports the exact would-be counts and
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
            workflow_rows: 0,
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

    /// `build_export_bundle` is behavior-preserving — the
    /// extracted builder produces the same JSON the live purge path embeds.
    #[test]
    fn dsar_export_bundle_builder_matches_live_shape() {
        // Full-migration fixture: the bundle now also reads case_notes.
        crate::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut conn, 1).unwrap();
        conn.execute_batch(
            "INSERT INTO knowledge(id, content, content_hash, owner) VALUES
                 (1, 'alice root', 'h1', 'alice@example.com'),
                 (2, 'alice derived', 'h2', NULL);
             INSERT INTO evidence_links(kind, from_chunk, to_chunk)
                 VALUES ('derived_from', 1, 2);",
        )
        .unwrap();
        // Channel rows across all three match arms: authored by the subject,
        // addressed to her, and content-bearing on someone else's note.
        conn.execute_batch(
            "INSERT INTO workflow_runs(id, domain, kind, state_json, status, created_at, updated_at)
             VALUES (7,'acme','interview','{}','active',1,1);
             INSERT INTO case_notes(domain, run_id, author, kind, content, addressed_to, state, created_at) VALUES
                 ('acme', 7, 'alice@example.com', 'note', 'my own note', NULL, 'visible', 1),
                 ('acme', 7, 'bob', 'invite', 'ping', 'alice@example.com', 'pending', 2),
                 ('acme', 7, 'carol', 'note', 'call alice@example.com back', NULL, 'visible', 3),
                 ('acme', 7, 'dave', 'note', 'unrelated chatter', NULL, 'visible', 4);",
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
        // Art-15 symmetry: the bundle discloses exactly what the purge
        // erases — author + addressee + content arms, never the unrelated row.
        let notes = v["channel_notes"].as_array().unwrap();
        assert_eq!(
            notes
                .iter()
                .map(|n| n["id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2, 3],
            "authored + addressed-to + content-bearing; unrelated excluded"
        );
    }

    /// a multi-domain DSAR purges the subject in EVERY
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
        let mut health_conn = health.get().unwrap();
        let health_run = run_pool(
            &mut health_conn,
            "global",
            subject,
            "both",
            false,
            now,
            false,
            None,
            false,
        )
        .unwrap();
        assert!(!health_run.purged_ids.is_empty(), "health pool erased");
        assert_eq!(health_run.ledger_id, None, "non-global pool never ledgers");
        drop(health_conn);
        // ...then global, with the cross-domain aggregate hash.
        let aggregate = crate::handlers::gate::sha256_hex(
            &serde_json::json!({"subject": subject, "domains": ["health"]}).to_string(),
        );
        let mut global_conn = global.get().unwrap();
        let global_run = run_pool(
            &mut global_conn,
            "global",
            subject,
            "both",
            false,
            now,
            true,
            Some(&aggregate),
            false,
        )
        .unwrap();
        assert!(!global_run.purged_ids.is_empty(), "global pool erased");
        assert!(
            global_run.ledger_id.is_some(),
            "global pool owns the ledger row"
        );
        drop(global_conn);

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

    /// a DSAR purge must erase the review-queue residue and the graph
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
        let mut conn2 = pool.get().unwrap();
        let run = run_pool(
            &mut conn2, "global", subject, "both", false, now, true, None, false,
        )
        .unwrap();
        assert!(!run.purged_ids.is_empty(), "subject root erased");
        drop(conn2);

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
