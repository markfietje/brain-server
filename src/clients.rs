//! v1.27.1 "Clients" — the BPO operating register: one row per operating
//! client (name, isolation domain, jurisdiction, bound profile, status),
//! stored in the global DB like the `transfers` register it mirrors. This is
//! the spine of the BPO arc — every later release (onboard, DPA, DSAR, holds,
//! termination, QA) reads or extends these rows.
//!
//! Honest framing: this is the **identity/evidence** register only. It does not
//! gate recall, DSAR, or any enforcement on membership (that is v1.27.x +
//! v2.x). It records *which* clients the operator serves and under what
//! jurisdiction — so the DPA/DSAR/hold/termination workflow has a stable
//! anchor. One domain per client is the isolation seam (separate SQLite pools
//! since v1.0); `domain` here is the validated label linking a row to that
//! pool (the domain need not exist until the Onboard release scaffolds it).

use rusqlite::{Connection, Transaction};

use crate::handlers::HandlerError;

/// One client register row. `dpa_terms` is NOT surfaced yet (a v1.27.3
/// column); the struct carries only the fields this release writes + reads.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Client {
    pub name: String,
    pub domain: String,
    pub jurisdiction: String,
    pub profile: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub archived_at: Option<i64>,
}

/// Validate a `POST /clients` payload before any write. One error per field,
/// in order (name → domain → jurisdiction), matching the `transfers`
/// `validate_register` shape. Reuses the existing domain + jurisdiction
/// validators — never re-written.
pub(crate) fn validate_new_client(
    name: &str,
    domain: &str,
    jurisdiction: &str,
) -> Result<(), HandlerError> {
    if !brain_server::storage_layout::is_valid_domain(name) {
        return Err(HandlerError::bad_request(
            "client_name_invalid",
            "name must be a lowercase domain-safe identifier (\u{2264} 63 chars)",
        ));
    }
    if !brain_server::storage_layout::is_valid_domain(domain) {
        return Err(HandlerError::bad_request(
            "client_domain_invalid",
            "domain must be a valid domain name",
        ));
    }
    // Same code + message as the DSAR + transfers `jurisdiction` gate.
    if !crate::transfers::is_jurisdiction_code(jurisdiction) {
        return Err(HandlerError::bad_request(
            "jurisdiction_invalid",
            "jurisdiction must be a short lowercase country code",
        ));
    }
    Ok(())
}

/// Insert a client row inside the caller's transaction. `name` is the PK, so a
/// duplicate is caught by the `ON CONFLICT DO NOTHING` + row-count check →
/// `conflict` (POST 409). Name + jurisdiction land in their canonical lowercase
/// form (validation is case-insensitive; storage is the vocabulary).
pub(crate) fn register(
    tx: &Transaction,
    name: &str,
    domain: &str,
    jurisdiction: &str,
    profile: Option<&str>,
    now: i64,
) -> Result<(), HandlerError> {
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
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    if changed == 0 {
        return Err(HandlerError::conflict("client already exists"));
    }
    Ok(())
}

fn client_row(r: &rusqlite::Row) -> rusqlite::Result<Client> {
    Ok(Client {
        name: r.get(0)?,
        domain: r.get(1)?,
        jurisdiction: r.get(2)?,
        profile: r.get(3)?,
        status: r.get(4)?,
        created_at: r.get(5)?,
        archived_at: r.get(6)?,
    })
}

const CLIENT_SELECT: &str =
    "SELECT name, domain, jurisdiction, profile, status, created_at, archived_at FROM clients";

/// The full register, ordered by name (a small operator table — no paging seam
/// yet; the R10 dashboard reads it whole).
pub(crate) fn list(conn: &Connection) -> Result<Vec<Client>, HandlerError> {
    let mut stmt = conn
        .prepare(&format!("{CLIENT_SELECT} ORDER BY name"))
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let rows = stmt
        .query_map([], client_row)
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    Ok(rows.flatten().collect())
}

/// Resolve a single client by its PK. `None` → 404 in the handler.
pub(crate) fn by_name(conn: &Connection, name: &str) -> Result<Option<Client>, HandlerError> {
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
    .map_err(|e| HandlerError::internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE clients(
                name TEXT PRIMARY KEY,
                domain TEXT NOT NULL,
                jurisdiction TEXT NOT NULL,
                profile TEXT,
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
        assert_eq!(err.status.as_u16(), 409, "handler maps conflict → 409");
        tx.commit().unwrap();
        assert!(by_name(&conn, "no-such-client").unwrap().is_none());
    }
}
