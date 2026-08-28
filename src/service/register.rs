//! The BPO operating register core — the `clients` table's complete storage
//! story plus the per-client delegation seams, converged onto the service
//! layer out of `src/clients.rs` (the pre-service domain module) and the
//! `handlers/clients.rs` bodies.
//!
//! OWNS (this aggregate's complete storage story):
//! - the `clients` register rows: insert ([`register`], canonical lowercase
//!   name/jurisdiction, `ON CONFLICT`-guarded), the archive flip
//!   ([`archive`] — WORM-lite, `status` is the only change), the reads
//!   ([`list`], [`by_name`]);
//! - the Art-28 DPA terms round-trip ([`set_dpa_terms`], blank/oversize
//!   fenced by [`MAX_DPA_FIELD`], stored as the row's `dpa_terms` JSON
//!   document);
//! - the registration fences re-asserted IN the core
//!   ([`validate_new_client`], [`validate_dpa_terms`]) so every future
//!   caller inherits them — the wire vocabulary is rendered at the handler
//!   from the typed variants;
//! - the per-client DELEGATION seams: [`require_active_client`] (the
//!   by-name resolve + archived-refusal every per-client route shares),
//!   [`coach_note`] (the QA-note write + its audit row INSIDE the caller's
//!   tx), [`termination_clause`] (the contract-end purge-or-return around
//!   the shared [`crate::service::purge`] / [`crate::service::dsar`]
//!   primitives), and [`dsar_context`] (the client's jurisdiction + latest
//!   transfer mechanism the DSAR certificate stamps);
//! - the auditor row filter ([`list_for_domain_grants`]): a
//!   `client-auditor`'s grant list scopes the emitted rows IN the core —
//!   row-scoping is a service duty, not call-site discipline.
//!
//! FK-children map: NONE — no parent-row DELETE lives in this aggregate.
//! The only mutation resembling a removal is [`archive`], which deletes
//! nothing (the audit-chain-preserving status flip). The termination clause
//! deletes `knowledge` rows ONLY through [`crate::service::purge::
//! purge_chunk_ids`], whose module header carries the complete FK-children
//! map (incl. the NO ACTION ceilings).
//!
//! pool authority: the domain registry stays at the handler — every fn
//! here takes `&rusqlite::Connection` or `&rusqlite::Transaction`, and
//! `register_services_receive_no_registry` (service/mod.rs) pins that at
//! the source AND the type level. The registry-scaffold step of
//! registration (which creates + migrates the domain file) therefore runs
//! at the handler, immediately before the tx is opened;
//! [`scaffold_and_register`] owns the in-tx story (archived-refusal,
//! profile bind, row insert).
//!
//! Wire-shape ceiling (honest): [`Client`] and [`DpaTerms`] keep their
//! legacy serde derives — the register row JSON and the stored `dpa_terms`
//! document ARE the wire/storage forms (byte-for-byte behavior pins outrank
//! the domain-type aspiration; the retention exemplar's ceiling).

use rusqlite::{Connection, Transaction};

/// Art 28 sub-processor terms — the evidence a controller checks before
/// authorizing the BPO. Free-text + bounded, exported as-is; CONFIG the operator fills, not a signed contract.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct DpaTerms {
    pub retention_on_termination: String,
    pub deletion_timeline: String,
    pub audit_rights: String,
    pub breach_notification_timeline: String,
    pub onward_transfer_restriction: String,
    pub sub_sub_processor_list: String,
}
pub(crate) const MAX_DPA_FIELD: usize = 2000;

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Client {
    pub name: String,
    pub domain: String,
    pub jurisdiction: String,
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dpa_terms: Option<DpaTerms>,
    pub status: String,
    pub created_at: i64,
    pub archived_at: Option<i64>,
}

/// Typed service error (the ServiceError convention: one enum per module).
/// `Database` carries the rusqlite text VERBATIM — the handler maps it onto
/// the route's frozen internal-error body byte-for-byte; every other variant
/// carries exactly the pre-move wire message.
#[derive(Debug)]
pub(crate) enum RegisterError {
    /// A query failed; the rusqlite message travels unchanged.
    Database(String),
    /// The `POST /clients` fence: a malformed name (charset/length).
    InvalidName,
    /// The `POST /clients` fence: a malformed isolation-domain label.
    InvalidDomain,
    /// The shared jurisdiction gate (same code as DSAR + transfers).
    InvalidJurisdiction,
    /// A duplicate register insert (`ON CONFLICT DO NOTHING` ate the row).
    Duplicate,
    /// The by-name resolve missed — one 404, existence never leaked wider.
    UnknownClient,
    /// The client row is archived — per-client writes refuse (409).
    ClientArchived,
    /// An ARCHIVED client must not be silently re-registered; carries the
    /// client name for the exact pre-move conflict message.
    ArchivedReregister(String),
    /// The bound profile does not exist (bind fails CLOSED pre-write).
    ProfileNotFound(String),
    /// A DPA field is blank or over [`MAX_DPA_FIELD`]; carries the exact
    /// pre-move message (field named).
    InvalidDpa(String),
    /// The `dpa_terms` JSON document failed to serialize.
    Serialize(String),
    /// The in-tx legal-hold fence of the termination purge fired (an
    /// impossible-after-deferral race); carries the held map for the shared
    /// `409 legal_hold_active` envelope.
    LegalHold(std::collections::HashMap<i64, Vec<String>>),
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegisterError::Database(e) => write!(f, "database error: {e}"),
            RegisterError::InvalidName => write!(
                f,
                "name must be a lowercase domain-safe identifier (\u{2264} 63 chars)"
            ),
            RegisterError::InvalidDomain => write!(f, "domain must be a valid domain name"),
            RegisterError::InvalidJurisdiction => {
                write!(f, "jurisdiction must be a short lowercase country code")
            }
            RegisterError::Duplicate => write!(f, "client already exists"),
            RegisterError::UnknownClient => write!(f, "client not found"),
            RegisterError::ClientArchived => write!(f, "client not active (archived)"),
            RegisterError::ArchivedReregister(name) => {
                write!(f, "client {name} is archived — re-register refused")
            }
            RegisterError::ProfileNotFound(e) => write!(f, "{e}"),
            RegisterError::InvalidDpa(msg) => write!(f, "{msg}"),
            RegisterError::Serialize(e) => write!(f, "serialize dpa_terms: {e}"),
            RegisterError::LegalHold(held) => write!(f, "legal hold active on {held:?}"),
        }
    }
}

impl From<rusqlite::Error> for RegisterError {
    fn from(e: rusqlite::Error) -> Self {
        RegisterError::Database(e.to_string())
    }
}

/// Validate a `POST /clients` payload before any write. One error per field in order
/// (name → domain → jurisdiction), matching `transfers::validate_register`; reuses the existing validators.
pub(crate) fn validate_new_client(
    name: &str,
    domain: &str,
    jurisdiction: &str,
) -> Result<(), RegisterError> {
    if !brain_server::storage_layout::is_valid_domain(name) {
        return Err(RegisterError::InvalidName);
    }
    if !brain_server::storage_layout::is_valid_domain(domain) {
        return Err(RegisterError::InvalidDomain);
    }
    // Same code + message as the DSAR + transfers `jurisdiction` gate.
    if !crate::transfers::is_jurisdiction_code(jurisdiction) {
        return Err(RegisterError::InvalidJurisdiction);
    }
    Ok(())
}

/// Insert a client row in the caller's tx. `name` is the PK, so a duplicate is caught
/// by `ON CONFLICT DO NOTHING` + row-count → [`RegisterError::Duplicate`] (409). Name + jurisdiction
/// land in canonical lowercase (validation is case-insensitive; storage is the vocabulary).
pub(crate) fn register(
    tx: &Transaction,
    name: &str,
    domain: &str,
    jurisdiction: &str,
    profile: Option<&str>,
    now: i64,
) -> Result<(), RegisterError> {
    let changed = tx
        .execute(
            "INSERT OR IGNORE INTO clients(name, domain, jurisdiction, profile, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                name.trim().to_ascii_lowercase(),
                domain.trim().to_ascii_lowercase(),
                jurisdiction.trim().to_ascii_lowercase(),
                profile.map(str::trim),
                now
            ],
        )
        .map_err(RegisterError::from)?;
    if changed == 0 {
        return Err(RegisterError::Duplicate);
    }
    Ok(())
}

/// Trust boundary: DPA terms ride out unredacted (Art 28 evidence), so nothing goes
/// out blank or oversized. Deterministic order; one error per field. The typed
/// [`RegisterError::InvalidDpa`] carries the exact pre-move message; the handler
/// renders the shared `dpa_field_invalid` code.
pub(crate) fn validate_dpa_terms(t: &DpaTerms) -> Result<(), RegisterError> {
    let fields: [(&str, &str); 6] = [
        ("retention_on_termination", &t.retention_on_termination),
        ("deletion_timeline", &t.deletion_timeline),
        ("audit_rights", &t.audit_rights),
        (
            "breach_notification_timeline",
            &t.breach_notification_timeline,
        ),
        (
            "onward_transfer_restriction",
            &t.onward_transfer_restriction,
        ),
        ("sub_sub_processor_list", &t.sub_sub_processor_list),
    ];
    for (name, value) in fields {
        let v = value.trim();
        if v.is_empty() {
            return Err(RegisterError::InvalidDpa(format!(
                "{name} must not be blank"
            )));
        }
        if v.len() > MAX_DPA_FIELD {
            return Err(RegisterError::InvalidDpa(format!(
                "{name} must be at most {MAX_DPA_FIELD} chars"
            )));
        }
    }
    Ok(())
}

/// Write the terms to the client row in the caller's tx (`WHERE name = ?`), returning
/// the affected row count so the handler 404s an unknown client without a second query.
/// The blank/oversize fence is re-asserted here (the fence holds of the FUNCTION).
pub(crate) fn set_dpa_terms(
    tx: &Transaction,
    name: &str,
    terms: &DpaTerms,
) -> Result<usize, RegisterError> {
    validate_dpa_terms(terms)?;
    let json = serde_json::to_string(terms).map_err(|e| RegisterError::Serialize(e.to_string()))?;
    tx.execute(
        "UPDATE clients SET dpa_terms = ?1 WHERE name = ?2",
        rusqlite::params![json, name.trim().to_ascii_lowercase()],
    )
    .map_err(RegisterError::from)
}

/// Audit-chain-preserving termination: flip the client row to `archived` (the
/// WORM-lite posture — nothing is DELETEd, `status` is the only change).
/// `Ok(false)` = already archived (handler maps to 409) or unknown (404).
pub(crate) fn archive(tx: &Transaction, name: &str, now: i64) -> Result<bool, RegisterError> {
    let n = tx
        .execute(
            "UPDATE clients SET status='archived', archived_at=?1 WHERE name=?2 AND status<>'archived'",
            rusqlite::params![now, name.trim().to_ascii_lowercase()],
        )
        .map_err(RegisterError::from)?;
    Ok(n > 0)
}

/// Parse stored DPA-term JSON, `None`-preserving: a client with no terms →
/// `None`, never a panic or a zeroed struct.
fn dpa_terms_of(json: Option<&str>) -> Option<DpaTerms> {
    json.and_then(|s| serde_json::from_str(s).ok())
}

fn client_row(r: &rusqlite::Row) -> rusqlite::Result<Client> {
    let dpa_terms: Option<String> = r.get(4)?;
    Ok(Client {
        name: r.get(0)?,
        domain: r.get(1)?,
        jurisdiction: r.get(2)?,
        profile: r.get(3)?,
        dpa_terms: dpa_terms_of(dpa_terms.as_deref()),
        status: r.get(5)?,
        created_at: r.get(6)?,
        archived_at: r.get(7)?,
    })
}

const CLIENT_SELECT: &str = "SELECT name, domain, jurisdiction, profile, dpa_terms, status, created_at, archived_at FROM clients";

/// The full register, ordered by name (a small operator table — no paging seam
/// yet; the dashboard reads it whole).
pub(crate) fn list(conn: &Connection) -> Result<Vec<Client>, RegisterError> {
    let mut stmt = conn
        .prepare(&format!("{CLIENT_SELECT} ORDER BY name"))
        .map_err(RegisterError::from)?;
    let rows = stmt
        .query_map([], client_row)
        .map_err(RegisterError::from)?;
    Ok(rows.flatten().collect())
}

/// The register as a principal may see it: an unfiltered list for a global
/// admin, or the row-level filter to exactly the granted client-domain(s)
/// (parent verification #7) for a `client-auditor`. The scoping predicate
/// lives HERE — defensive row-scoping in the core, not call-site discipline;
/// the handler's gate (403 on an empty grant set) stays in front of it.
pub(crate) fn list_for_domain_grants(
    conn: &Connection,
    granted: Option<&[String]>,
) -> Result<Vec<Client>, RegisterError> {
    let rows = list(conn)?;
    if let Some(g) = granted {
        Ok(rows
            .into_iter()
            .filter(|c| g.iter().any(|d| d == &c.domain))
            .collect())
    } else {
        Ok(rows)
    }
}

/// Resolve a single client by its PK. `None` → 404 in the handler.
pub(crate) fn by_name(conn: &Connection, name: &str) -> Result<Option<Client>, RegisterError> {
    conn.query_row(
        &format!("{CLIENT_SELECT} WHERE name = ?1"),
        rusqlite::params![name.to_ascii_lowercase()],
        client_row,
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
    .map_err(RegisterError::from)
}

/// The per-client delegation seam: by-name resolve + the archived refusal
/// every per-client write path (DSAR, hold, coach, QA queue, termination)
/// shares — 404 unknown, 409 archived, before any domain-pool work. The
/// handler renders both from the typed variants, byte-identical to the
/// pre-move vocabulary.
pub(crate) fn require_active_client(
    conn: &Connection,
    name: &str,
) -> Result<Client, RegisterError> {
    let c = by_name(conn, name)?.ok_or(RegisterError::UnknownClient)?;
    if c.status != "active" {
        return Err(RegisterError::ClientArchived);
    }
    Ok(c)
}

/// Composition seam, shared by the handler + CLI.
///
/// 1. The client's domain is made real BEFORE the tx opens
///    (the registry's register step at the handler — the pool authority
///    never crosses this boundary).
/// 2. THIS fn: the archived-refusal, the optional profile bind (fails CLOSED
///    before any client-row write) and the row insert — one caller-owned tx,
///    so a failed bind never leaves a `clients` row or `domain_profiles`
///    bind (atomicity).
///
/// `pool_for`'s note: the GLOBAL connection holding the `clients` (and
/// `domain_profiles`) tables is the caller's borrow.
pub(crate) fn scaffold_and_register(
    tx: &Transaction,
    name: &str,
    domain: &str,
    jurisdiction: &str,
    profile: Option<&str>,
    now: i64,
) -> Result<(), RegisterError> {
    validate_new_client(name, domain, jurisdiction)?;
    if let Some(existing) = by_name(tx, name)? {
        if existing.status != "active" {
            // An ARCHIVED client must not be silently re-registered: the
            // archive is a termination record. Re-activation is an
            // explicit operator action, not a register side effect.
            return Err(RegisterError::ArchivedReregister(name.to_string()));
        }
        return Ok(());
    }
    if let Some(p) = profile {
        brain_server::profile::bind(tx, domain, Some(p)).map_err(RegisterError::ProfileNotFound)?;
    }
    register(tx, name, domain, jurisdiction, profile, now)
}

/// The supervisor-coach write in the caller's tx: set/clear the `qa_note`
/// and — only when a row matched — emit its audit row INSIDE the same tx
/// (the audit-per-write law; the note content is never stored raw in the
/// audit — only the id + flagged flag). Returns the affected-row count so
/// the handler 404s an unknown proposal.
pub(crate) fn coach_note(
    tx: &Transaction,
    client_key: &str,
    proposal_id: i64,
    note: Option<String>,
    flagged: bool,
) -> Result<usize, RegisterError> {
    let n = tx
        .execute(
            "UPDATE proposals SET qa_note = ?1 WHERE id = ?2",
            rusqlite::params![note, proposal_id],
        )
        .map_err(|e| RegisterError::Database(format!("coach update failed: {e}")))?;
    if n > 0 {
        crate::audit::record(
            tx,
            crate::audit::AuditKind::Client,
            "api",
            &format!("client:{client_key}:coach:{proposal_id}:{flagged}"),
            crate::audit::AuditStatus::Ok,
            "coach",
        );
    }
    Ok(n)
}

/// The contract-end outcome: what the certificate reports about the data
/// phase of the termination (the archive + audit happen separately in the
/// caller's global-DB tx).
#[derive(Debug)]
pub(crate) struct TerminationOutcome {
    pub purged_chunk_count: i64,
    pub held_ids: Vec<i64>,
    pub exported_bundle: Option<String>,
}

/// The per-client termination clause's DATA phase, inside the caller's
/// DOMAIN-DB tx: the live id set, the hold split (held ids are DEFERRED —
/// never purged, reported on the certificate), then purge
/// ([`crate::service::purge::purge_chunk_ids`], the `dataset` tag riding as
/// the reason) or the return-path export
/// ([`crate::service::dsar::build_export_bundle`]). The archive flip +
/// audit row are the caller's global-DB story (two-pool sequencing stays at
/// the handler — shim mode shares one r2d2 pool, so the domain conn is
/// dropped before the global one is taken).
pub(crate) fn termination_clause(
    tx: &Transaction,
    subject: &str,
    purge: bool,
    now: i64,
    dataset: &str,
) -> Result<TerminationOutcome, RegisterError> {
    let active: Vec<i64> = {
        let mut stmt = tx.prepare("SELECT id FROM knowledge")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        rows.flatten().collect()
    };
    let held_set = crate::legal_hold::active_hold_ids(tx)?;
    let held_ids: Vec<i64> = active
        .iter()
        .filter(|id| held_set.contains(id))
        .copied()
        .collect();
    let free: Vec<i64> = active
        .iter()
        .filter(|id| !held_set.contains(id))
        .copied()
        .collect();
    let (n, bundle) = if purge {
        let n =
            crate::service::purge::purge_chunk_ids(tx, &free, now, dataset, None).map_err(|e| {
                match e {
                    crate::service::purge::PurgeError::Database(m) => RegisterError::Database(m),
                    crate::service::purge::PurgeError::LegalHold(held) => {
                        RegisterError::LegalHold(held)
                    }
                }
            })?;
        (n, None)
    } else {
        (
            0,
            Some(
                crate::service::dsar::build_export_bundle(tx, subject, &active, &[]).map_err(
                    |e| match e {
                        crate::service::dsar::DsarError::Database(m) => RegisterError::Database(m),
                        crate::service::dsar::DsarError::LegalHold(held) => {
                            RegisterError::LegalHold(held)
                        }
                        other => RegisterError::Database(other.to_string()),
                    },
                )?,
            ),
        )
    };
    Ok(TerminationOutcome {
        purged_chunk_count: n,
        held_ids,
        exported_bundle: bundle,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alert::ChainWatchState;
    use crate::auth::jwks::KeyStore;
    use crate::domain_registry::DomainRegistry;
    use crate::handlers::clients::{
        ClientDsarRequest, ClientEndRequest, ClientHoldRequest, CoachRequest, client_dsar,
        client_end, client_hold, client_proposals, coach_proposal, get_client, list_clients,
    };
    use crate::integrity::SnapshotState;
    use crate::{AppState, ConnectionTracker, RateLimiter};
    use axum::Json;
    use axum::extract::{Path, State};
    use axum::http::StatusCode;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE clients(
                name TEXT PRIMARY KEY,
                domain TEXT NOT NULL,
                jurisdiction TEXT NOT NULL,
                profile TEXT,
                dpa_terms TEXT,
                status TEXT NOT NULL DEFAULT 'active',
                created_at INTEGER NOT NULL,
                archived_at INTEGER);",
        )
        .unwrap();
        conn
    }

    fn add_one(conn: &mut Connection) {
        let tx = conn.transaction().unwrap();
        register(&tx, "Acme Corp", "acme", "us", None, 1_000).unwrap();
        tx.commit().unwrap();
    }

    #[test]
    fn client_register_round_trips_and_lists() {
        let mut conn = db();
        add_one(&mut conn);
        let rows = list(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        // Canonical lowercase name + jurisdiction stored, per the plan.
        assert_eq!(rows[0].name, "acme corp");
        assert_eq!(rows[0].status, "active");
        assert_eq!(rows[0].created_at, 1_000);
        assert!(rows[0].archived_at.is_none());
        let one = by_name(&conn, "ACME CORP").unwrap().unwrap();
        assert_eq!(one.domain, "acme");
        assert_eq!(one.jurisdiction, "us");
    }

    #[test]
    fn validate_new_client_rejects_bad_name_and_jurisdiction() {
        assert!(validate_new_client("acme", "acme", "us").is_ok());
        assert!(validate_new_client("", "acme", "us").is_err());
        assert!(
            validate_new_client("ACME", "acme", "us").is_err(),
            "uppercase"
        );
        assert!(
            validate_new_client("..", "acme", "us").is_err(),
            "path-safe"
        );
        assert!(validate_new_client("acme", "bad domain", "us").is_err());
        assert!(
            validate_new_client("acme", "acme", "US").is_ok(),
            "case-insensitive code (like transfers)"
        );
        assert!(validate_new_client("acme", "acme", "not a code").is_err());
        assert!(validate_new_client("acme", "acme", "").is_err());
    }

    #[test]
    fn register_conflicts_on_duplicate_and_by_name_absent_is_none() {
        let mut conn = db();
        add_one(&mut conn);
        let tx = conn.transaction().unwrap();
        let dup = register(&tx, "acme corp", "acme", "us", None, 2_000);
        let err = dup.expect_err("duplicate must conflict");
        assert!(
            matches!(err, RegisterError::Duplicate),
            "handler maps conflict → 409, got: {err:?}"
        );
        tx.commit().unwrap();
        assert!(by_name(&conn, "no-such-client").unwrap().is_none());
    }

    /// Build a real multi-db registry (temp dir, migration-seeded global DB)
    /// so the scaffold seam exercises the actual `pool_for` creation step
    /// (the registry half that stays at the caller).
    fn registry() -> (
        tempfile::TempDir,
        crate::Pool,
        crate::domain_registry::DomainRegistry,
    ) {
        brain_server::register_sqlite_vec::register_sqlite_vec();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brain.db");
        let mgr = r2d2_sqlite::SqliteConnectionManager::file(&path);
        let global: crate::Pool = r2d2::Pool::builder().build(mgr).expect("global pool");
        brain_server::migration::run_migration(
            &mut global.get().unwrap(),
            crate::config::DB_MMAP_SIZE_MIB,
        )
        .expect("migration seeds profiles + clients");
        let reg = crate::domain_registry::DomainRegistry::new(global.clone(), &path, true);
        (dir, global, reg)
    }

    /// The split composition: the registry scaffold (pool authority — stays
    /// at the caller) + the core's in-tx story.
    fn scaffold_via(
        reg: &DomainRegistry,
        global: &crate::Pool,
        name: &str,
        domain: &str,
        jurisdiction: &str,
        profile: Option<&str>,
        now: i64,
    ) -> Result<(), RegisterError> {
        reg.register(domain)
            .map_err(|e| RegisterError::Database(e.to_string()))?;
        let mut conn = global.get().unwrap();
        let tx = conn.transaction().unwrap();
        let out = scaffold_and_register(&tx, name, domain, jurisdiction, profile, now);
        if out.is_ok() {
            tx.commit().unwrap();
        }
        out
    }

    #[test]
    fn create_domain_scaffolding_is_idempotent_and_binds_profile() {
        brain_server::register_sqlite_vec::register_sqlite_vec();
        let (_dir, global, reg) = registry();
        scaffold_via(
            &reg,
            &global,
            "acme",
            "acme",
            "us",
            Some("health-hipaa"),
            1_000,
        )
        .unwrap();
        scaffold_via(
            &reg,
            &global,
            "acme",
            "acme",
            "us",
            Some("health-hipaa"),
            2_000,
        )
        .expect("second compose (same domain) must not error");
        let conn = global.get().unwrap();
        let rows = list(&conn).unwrap();
        assert_eq!(rows.len(), 1, "one client row across two composes");
        let bound: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM domain_profiles WHERE domain = 'acme'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bound, 1, "exactly one profile bind for the domain");
        drop(conn);
        let domain_conn = reg.register("acme").unwrap().get().unwrap();
        let knowledge: i64 = domain_conn
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(knowledge, 0, "client domain DB was scaffolded by register");
    }

    #[test]
    fn create_domain_bad_profile_fails_closed_no_client_row() {
        brain_server::register_sqlite_vec::register_sqlite_vec();
        let (_dir, global, reg) = registry();
        let err = scaffold_via(
            &reg,
            &global,
            "acme",
            "acme",
            "us",
            Some("no-such-profile"),
            1_000,
        )
        .unwrap_err();
        assert!(
            matches!(err, RegisterError::ProfileNotFound(_)),
            "unknown profile closes the write, got: {err:?}"
        );
        let conn = global.get().unwrap();
        assert!(
            by_name(&conn, "acme").unwrap().is_none(),
            "tx atomicity: no clients row after a failed bind"
        );
        let bound: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM domain_profiles WHERE domain = 'acme'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bound, 0, "no domain_profiles bind persists on failure");
    }

    #[test]
    fn archive_is_idempotent_and_unknown_is_false() {
        let mut conn = db();
        add_one(&mut conn);
        let tx = conn.transaction().unwrap();
        assert!(
            archive(&tx, "acme corp", 5_000).unwrap(),
            "active → archived"
        );
        assert!(
            !archive(&tx, "acme corp", 6_000).unwrap(),
            "second archive is a no-op (idempotent)"
        );
        assert!(
            !archive(&tx, "no-such-client", 6_000).unwrap(),
            "unknown client affects zero rows"
        );
        tx.commit().unwrap();
        let one = by_name(&conn, "acme corp").unwrap().unwrap();
        assert_eq!(one.status, "archived");
        assert_eq!(one.archived_at, Some(5_000), "first archive stamps 5000");
    }

    #[test]
    fn dpa_terms_round_trip_and_list() {
        let mut conn = db();
        add_one(&mut conn);
        let terms = DpaTerms {
            retention_on_termination: "purge".into(),
            deletion_timeline: "within 30 days of end".into(),
            audit_rights: "annual + on request".into(),
            breach_notification_timeline: "within 72h".into(),
            onward_transfer_restriction: "no onward transfer".into(),
            sub_sub_processor_list: "none".into(),
        };
        let tx = conn.transaction().unwrap();
        let n = set_dpa_terms(&tx, "acme corp", &terms).unwrap();
        assert_eq!(n, 1, "known client updates one row");
        tx.commit().unwrap();
        let one = by_name(&conn, "acme corp").unwrap().unwrap();
        assert_eq!(one.dpa_terms.as_ref().unwrap(), &terms);
        let listed = list(&conn).unwrap();
        assert_eq!(
            listed[0].dpa_terms.as_ref().unwrap(),
            &terms,
            "list carries the parsed terms"
        );
        assert_eq!(one.status, "active");
    }

    #[test]
    fn validate_dpa_terms_rejects_blank_and_too_long() {
        let good = DpaTerms {
            retention_on_termination: "purge".into(),
            deletion_timeline: "within 30 days".into(),
            audit_rights: "annual".into(),
            breach_notification_timeline: "72h".into(),
            onward_transfer_restriction: "none".into(),
            sub_sub_processor_list: "none".into(),
        };
        assert!(validate_dpa_terms(&good).is_ok());
        let mut blank = good.clone();
        blank.audit_rights = "  ".into();
        let err = validate_dpa_terms(&blank).unwrap_err();
        assert!(matches!(err, RegisterError::InvalidDpa(_)));
        assert!(
            err.to_string().contains("audit_rights"),
            "error names the offending field"
        );
        let mut long = good.clone();
        long.onward_transfer_restriction = "x".repeat(MAX_DPA_FIELD + 1);
        assert!(validate_dpa_terms(&long).is_err());
    }

    #[test]
    fn set_dpa_terms_unknown_client_returns_zero() {
        let mut conn = db();
        add_one(&mut conn);
        let terms = DpaTerms {
            retention_on_termination: "purge".into(),
            deletion_timeline: "30d".into(),
            audit_rights: "annual".into(),
            breach_notification_timeline: "72h".into(),
            onward_transfer_restriction: "none".into(),
            sub_sub_processor_list: "none".into(),
        };
        let tx = conn.transaction().unwrap();
        let n = set_dpa_terms(&tx, "no-such-client", &terms).unwrap();
        assert_eq!(n, 0, "unknown client affects zero rows (handler 404s)");
        tx.commit().unwrap();
        assert!(by_name(&conn, "no-such-client").unwrap().is_none());
    }

    // ── the per-client delegation seams, driven through the handler fns ──
    // (the pre-move handlers/clients.rs pins, repointed verbatim — the
    // handler fns are the same call surface the routes bind.)

    fn app_state(dir: &tempfile::TempDir) -> std::sync::Arc<AppState> {
        app_state_with(dir, true, 4)
    }

    // The shared static embedder is loaded once and reused across tests: many
    // parallel tests each building a fresh model2vec instance raced on huggingface's
    // file-based cache lock ("Lock acquisition failed") under a cold CI cache.
    static TEST_EMBEDDER: std::sync::OnceLock<std::sync::Arc<dyn brain_server::embed::Embedder>> =
        std::sync::OnceLock::new();

    fn app_state_with(
        dir: &tempfile::TempDir,
        multi_db: bool,
        max_size: u32,
    ) -> std::sync::Arc<AppState> {
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
        let model: std::sync::Arc<dyn brain_server::embed::Embedder> = TEST_EMBEDDER
            .get_or_init(|| {
                std::sync::Arc::new(
                    brain_server::embed::StaticEmbedder::new(crate::config::MODEL_ID)
                        .expect("model"),
                )
            })
            .clone();
        std::sync::Arc::new(AppState {
            model,
            registry: DomainRegistry::new(pool.clone(), &path, multi_db),
            pool,
            db_path: path.clone(),
            connection_tracker: std::sync::Arc::new(ConnectionTracker::new()),
            rate_limiter: std::sync::Arc::new(RateLimiter::new()),
            snapshot: SnapshotState::default(),
            audit_chain_cache: std::sync::Arc::new(std::sync::Mutex::new(None)),
            auth_mode: crate::auth::AuthMode::Opaque,
            key_store: KeyStore::load(std::path::Path::new("/nonexistent"))
                .expect("empty key store"),
            revocation_cache: std::sync::Arc::new(crate::auth::revocation::RevocationCache::new()),
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
        // The split composition: registry scaffold (pool authority) + the
        // core's in-tx story.
        state.registry.register(domain).expect("domain scaffold");
        let mut conn = state.pool.get().expect("global conn");
        let tx = conn.transaction().unwrap();
        scaffold_and_register(&tx, name, domain, jurisdiction, None, 1_000)
            .expect("register client");
        tx.commit().unwrap();
    }

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
            crate::handlers::auth::OptPrincipal(None),
            Path("beta".to_string()),
            Json(ClientDsarRequest {
                subject: "alice@beta".to_string(),
                action: "purge".to_string(),
                dry_run: false,
                subject_exact: false,
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
            subject_exact: false,
        });
        let err = client_dsar(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
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
            crate::handlers::auth::OptPrincipal(None),
            Path("beta".to_string()),
            Json(ClientDsarRequest {
                subject: "s".to_string(),
                action: "purge".to_string(),
                dry_run: true,
                subject_exact: false,
            }),
        )
        .await
        .expect_err("archived client 409s");
        assert_eq!(err.status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn legal_hold_per_client_isolates_domains() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "beta", "beta-eu", "eu");
        register_client(&state, "acme", "acme-us", "us");
        let id_beta = {
            let pool = state.registry.register("beta-eu").unwrap();
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO knowledge(content, content_hash, owner) VALUES ('data','h','alice')",
                [],
            )
            .expect("seed beta row");
            conn.query_row("SELECT MAX(id) FROM knowledge", [], |r| r.get(0))
                .unwrap()
        };
        let id_acme = {
            let pool = state.registry.register("acme-us").unwrap();
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO knowledge(content, content_hash, owner) VALUES ('data','h','alice')",
                [],
            )
            .expect("seed acme row");
            conn.query_row("SELECT MAX(id) FROM knowledge", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(
            id_beta, id_acme,
            "identical autoincrement ids across domains"
        );

        let resp = client_hold(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            Path("acme".to_string()),
            Json(ClientHoldRequest {
                ids: vec![id_acme],
                reason: "case 2026-118".to_string(),
            }),
        )
        .await
        .expect("hold lands on acme's domain");
        assert_eq!(resp.held, 1);

        let acme_held = {
            let conn = state.registry.register("acme-us").unwrap().get().unwrap();
            crate::legal_hold::active_hold_ids(&conn).unwrap()
        };
        assert!(acme_held.contains(&id_acme), "acme's id is held in acme-us");
        let beta_held = {
            let conn = state.registry.register("beta-eu").unwrap().get().unwrap();
            crate::legal_hold::active_hold_ids(&conn).unwrap()
        };
        assert!(
            !beta_held.contains(&id_beta),
            "beta's identical-id row is NOT held (isolation)"
        );
        assert!(
            acme_held != beta_held,
            "the held sets must differ across domains"
        );
    }

    #[tokio::test]
    async fn client_hold_unknown_or_archived_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let body = Json(ClientHoldRequest {
            ids: vec![1],
            reason: "case".to_string(),
        });
        let err = client_hold(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            Path("nope".to_string()),
            body,
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
        let err = client_hold(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            Path("beta".to_string()),
            Json(ClientHoldRequest {
                ids: vec![1],
                reason: "case".to_string(),
            }),
        )
        .await
        .expect_err("archived client 409s");
        assert_eq!(err.status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn per_client_dsar_shim_single_pool_no_deadlock() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state_with(&dir, false, 1);
        register_client(&state, "beta", "beta", "eu");
        seed_subject(&state, "global", "alice@beta");

        let resp = client_dsar(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            Path("beta".to_string()),
            Json(ClientDsarRequest {
                subject: "alice@beta".to_string(),
                action: "purge".to_string(),
                dry_run: false,
                subject_exact: false,
            }),
        )
        .await
        .expect("shim dsar completes (no pool deadlock)");
        assert_eq!(resp.status, "completed");
        assert!(resp.certificate.is_some());
        assert_eq!(count_knowledge(&state, "global"), 0, "shim subject purged");
        let cert: Option<String> = state
            .pool
            .get()
            .unwrap()
            .query_row("SELECT certificate FROM dsar_requests LIMIT 1", [], |r| {
                r.get(0)
            })
            .expect("ledger row");
        assert!(
            cert.is_some() && cert.unwrap().contains("\"jurisdiction\":\"eu\""),
            "certificate backfilled with the client's jurisdiction"
        );
    }

    fn seed_rows(state: &AppState, domain: &str, n: i64) -> Vec<i64> {
        let pool = state.registry.register(domain).unwrap();
        let conn = pool.get().unwrap();
        let mut ids = Vec::new();
        for i in 0..n {
            conn.execute(
                "INSERT INTO knowledge(content, content_hash, owner) VALUES (?1, ?2, 'o')",
                rusqlite::params![format!("data-{i}"), format!("h{i}")],
            )
            .expect("seed row");
            ids.push(conn.last_insert_rowid());
        }
        ids
    }

    fn set_client_dpa_direct(state: &AppState, name: &str, retention: &str) {
        let terms = DpaTerms {
            retention_on_termination: retention.into(),
            deletion_timeline: "30d".into(),
            audit_rights: "annual".into(),
            breach_notification_timeline: "72h".into(),
            onward_transfer_restriction: "none".into(),
            sub_sub_processor_list: "none".into(),
        };
        let mut conn = state.pool.get().unwrap();
        let tx = conn.transaction().unwrap();
        set_dpa_terms(&tx, name, &terms).unwrap();
        tx.commit().unwrap();
    }

    #[tokio::test]
    async fn client_end_runs_termination_clause() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "acme", "acme-us", "us");
        set_client_dpa_direct(&state, "acme", "purge");
        seed_rows(&state, "acme-us", 2);

        let resp = client_end(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            Path("acme".to_string()),
            Json(ClientEndRequest {
                purge_opt: None,
                dataset: "termination".to_string(),
            }),
        )
        .await
        .expect("end follows the DPA purge policy");
        assert_eq!(resp.policy, "purge");
        assert_eq!(resp.purged_chunk_count, 2);
        assert!(resp.exported_bundle.is_none());
        assert!(resp.chain_head.is_some());
        assert_eq!(count_knowledge(&state, "acme-us"), 0);
        let c = by_name(&state.pool.get().unwrap(), "acme")
            .unwrap()
            .unwrap();
        assert_eq!(c.status, "archived");
        assert_eq!(c.archived_at, Some(resp.archived_at));
    }

    #[tokio::test]
    async fn client_end_return_exports_and_archives_no_purge() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "acme", "acme-us", "us");
        set_client_dpa_direct(&state, "acme", "return");
        seed_rows(&state, "acme-us", 2);

        let resp = client_end(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            Path("acme".to_string()),
            Json(ClientEndRequest {
                purge_opt: Some(false),
                dataset: "termination".to_string(),
            }),
        )
        .await
        .expect("return policy exports without purging");
        assert_eq!(resp.policy, "return");
        assert_eq!(resp.purged_chunk_count, 0);
        let bundle = resp.exported_bundle.as_deref().expect("bundle present");
        let v: serde_json::Value = serde_json::from_str(bundle).unwrap();
        assert_eq!(v["subject"].as_str(), Some("acme"));
        assert_eq!(v["knowledge"].as_array().unwrap().len(), 2);
        assert_eq!(count_knowledge(&state, "acme-us"), 2, "no purge on return");
        assert_eq!(
            by_name(&state.pool.get().unwrap(), "acme")
                .unwrap()
                .unwrap()
                .status,
            "archived"
        );
    }

    #[tokio::test]
    async fn client_end_defers_held_ids_and_archive_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "acme", "acme-us", "us");
        let ids = seed_rows(&state, "acme-us", 2);
        let pool = state.registry.register("acme-us").unwrap();
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO legal_holds(knowledge_id, reason, held_by, held_at)
             VALUES (?1, 'case-42', 'test', 1)",
            rusqlite::params![ids[1]],
        )
        .expect("hold the second row");

        let resp = client_end(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            Path("acme".to_string()),
            Json(ClientEndRequest {
                purge_opt: Some(true),
                dataset: "termination".to_string(),
            }),
        )
        .await
        .expect("purge terminates");
        assert_eq!(resp.purged_chunk_count, 1, "only the free row purged");
        assert_eq!(
            resp.held_ids,
            vec![ids[1]],
            "held id deferred on the certificate, never purged"
        );
        assert_eq!(count_knowledge(&state, "acme-us"), 1, "held row survives");

        let err = client_end(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            Path("acme".to_string()),
            Json(ClientEndRequest {
                purge_opt: Some(true),
                dataset: "termination".to_string(),
            }),
        )
        .await
        .expect_err("already-archived client 409s");
        assert_eq!(err.status, StatusCode::CONFLICT);
    }

    #[test]
    fn client_end_unknown_client_404s_before_pool_work() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(client_end(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            Path("nope".to_string()),
            Json(ClientEndRequest {
                purge_opt: None,
                dataset: "termination".to_string(),
            }),
        ));
        let err = err.expect_err("unknown client 404s");
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn coach_attaches_note_and_audits() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "beta", "beta-eu", "eu");
        let did: i64 = state
            .registry
            .register("beta-eu")
            .unwrap()
            .get()
            .unwrap()
            .query_row(
                "INSERT INTO proposals(kind, content, novelty, salience, created_at, owner)
                 VALUES ('fact', 'body', 0.9, 0.5, 0, 'agent-1') RETURNING id",
                [],
                |r| r.get(0),
            )
            .unwrap();

        let resp = coach_proposal(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            Path(("beta".to_string(), did)),
            Json(CoachRequest {
                flagged: true,
                note: Some("follow up".to_string()),
            }),
        )
        .await
        .expect("coach runs");
        assert_eq!(resp["flagged"], true);
        assert_eq!(resp["status"], "coached");

        let note: Option<String> = state
            .registry
            .register("beta-eu")
            .unwrap()
            .get()
            .unwrap()
            .query_row("SELECT qa_note FROM proposals WHERE id = ?1", [did], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(note.as_deref(), Some("follow up"));

        let audited: i64 = state
            .registry
            .register("beta-eu")
            .unwrap()
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'client'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(audited >= 1, "coach is audited");

        let err = coach_proposal(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            Path(("beta".to_string(), 99_999)),
            Json(CoachRequest {
                flagged: false,
                note: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND, "unknown proposal 404s");
    }

    #[tokio::test]
    async fn coach_audits_inside_the_tx() {
        // The audit-per-write law, twin of the retention pin: the note write
        // and its audit row run inside ONE caller tx — a rollback erases
        // both, a commit lands both.
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "beta", "beta-eu", "eu");
        let did: i64 = state
            .registry
            .register("beta-eu")
            .unwrap()
            .get()
            .unwrap()
            .query_row(
                "INSERT INTO proposals(kind, content, novelty, salience, created_at, owner)
                 VALUES ('fact', 'body', 0.9, 0.5, 0, 'agent-1') RETURNING id",
                [],
                |r| r.get(0),
            )
            .unwrap();

        let mut conn = state.registry.register("beta-eu").unwrap().get().unwrap();
        let tx = conn.transaction().unwrap();
        let n = coach_note(&tx, "beta", did, Some("rolled back".to_string()), true).unwrap();
        assert_eq!(n, 1);
        tx.rollback().unwrap();

        let conn = state.registry.register("beta-eu").unwrap().get().unwrap();
        let note: Option<String> = conn
            .query_row("SELECT qa_note FROM proposals WHERE id = ?1", [did], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(note.is_none(), "the rolled-back note never lands");
        let audited: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE detail_hash = ?1",
                [crate::audit::hash("coach")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audited, 0, "the audit row rolled back WITH the note write");
    }

    #[tokio::test]
    async fn qa_review_queue_surfaces_agent_interactions() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "beta", "beta-eu", "eu");
        let conn = state.registry.register("beta-eu").unwrap().get().unwrap();
        for (i, owner) in ["agent-1", "other-agent"].iter().enumerate() {
            conn.execute(
                "INSERT INTO proposals(kind, content, novelty, salience, created_at, owner)
                 VALUES ('fact', ?1, 0.9, 0.5, ?2, ?3)",
                rusqlite::params![format!("body {i}"), i as i64, owner],
            )
            .unwrap();
        }
        let sup = crate::auth::Principal {
            sub: "super@beta".to_string(),
            tenant: "beta".to_string(),
            scopes: vec![crate::auth::Scope::parse("admin:beta/*").unwrap()],
            jti: "t".to_string(),
            roles: vec![],
            manages: vec!["agent-1".to_string()],
        };
        let resp = client_proposals(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(Some(sup)),
            Path("beta".to_string()),
        )
        .await
        .expect("qa list runs");
        assert_eq!(resp.len(), 1, "only the managed agent's proposal surfaces");
        assert_eq!(resp[0].owner.as_deref(), Some("agent-1"));
        assert!(resp[0].qa_score > 0, "qa list carries a score");
    }

    #[tokio::test]
    async fn client_auditor_sees_only_their_domain() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "acme", "acme-us", "us");
        register_client(&state, "beta", "beta-eu", "eu");

        let auditor = || {
            crate::handlers::auth::OptPrincipal(Some(crate::auth::Principal {
                sub: "compliance@acme".to_string(),
                tenant: "ops".to_string(),
                scopes: vec![crate::auth::Scope::parse("admin:ops/acme-us").unwrap()],
                jti: "a".to_string(),
                roles: vec!["client-auditor".to_string()],
                manages: vec![],
            }))
        };
        let list = list_clients(State(state.clone()), auditor())
            .await
            .expect("auditor list runs");
        let names: Vec<&str> = list["clients"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["name"].as_str())
            .collect();
        assert_eq!(
            names,
            vec!["acme"],
            "only the granted client-domain surfaces"
        );

        let denied = get_client(State(state.clone()), auditor(), Path("beta".to_string()))
            .await
            .expect_err("beta is not the auditor's client");
        assert_eq!(denied.status, StatusCode::NOT_FOUND);

        let ops = crate::handlers::auth::OptPrincipal(Some(crate::auth::Principal {
            sub: "ops".to_string(),
            tenant: "ops".to_string(),
            scopes: vec![crate::auth::Scope::parse("admin:ops/*").unwrap()],
            jti: "b".to_string(),
            roles: vec!["bpo-ops".to_string()],
            manages: vec![],
        }));
        let all = list_clients(State(state.clone()), ops)
            .await
            .expect("bpo-ops list runs");
        let names: Vec<&str> = all["clients"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["name"].as_str())
            .collect();
        assert_eq!(names, vec!["acme", "beta"], "bpo-ops sees every client");
    }

    #[tokio::test]
    async fn client_auditor_with_no_granted_domain_sees_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        register_client(&state, "acme", "acme-us", "us");
        let auditor = || {
            crate::handlers::auth::OptPrincipal(Some(crate::auth::Principal {
                sub: "c".to_string(),
                tenant: "ops".to_string(),
                scopes: vec![],
                jti: "a".to_string(),
                roles: vec!["client-auditor".to_string()],
                manages: vec![],
            }))
        };
        let list = list_clients(State(state.clone()), auditor()).await;
        // v1.27.25 (S2-15): "Some([]) denies all" now denies at the GATE too —
        // an empty-grant auditor gets 403, not a silent 200-empty (the
        // row-filter equivalent was probe-food; the surface is closed).
        let denied_list = list.expect_err("empty-grant auditor must be denied at the gate");
        assert_eq!(denied_list.status, StatusCode::FORBIDDEN);
        let denied = get_client(State(state.clone()), auditor(), Path("acme".to_string()))
            .await
            .expect_err("no granted domain denies the lookup");
        assert_eq!(denied.status, StatusCode::NOT_FOUND);
    }

    /// The core-level half of the auditor isolation: the row filter runs in
    /// the service — a grant list scopes the emitted rows even when a future
    /// caller forgets to check at the gate.
    #[test]
    fn list_for_domain_grants_scopes_rows_in_the_core() {
        let mut conn = db();
        add_one(&mut conn);
        let tx = conn.transaction().unwrap();
        register(&tx, "beta", "beta-eu", "eu", None, 1_000).unwrap();
        tx.commit().unwrap();

        let grants = vec!["acme".to_string()];
        let rows = list_for_domain_grants(&conn, Some(&grants)).unwrap();
        assert_eq!(rows.len(), 1, "only the granted domain's row surfaces");
        assert_eq!(rows[0].domain, "acme");

        let empty: Vec<String> = Vec::new();
        assert!(
            list_for_domain_grants(&conn, Some(&empty))
                .unwrap()
                .is_empty(),
            "an empty grant set yields no rows (the handler adds the 403)"
        );
        assert_eq!(
            list_for_domain_grants(&conn, None).unwrap().len(),
            2,
            "an admin (no grant list) sees the whole register"
        );
    }
}
