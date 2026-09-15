//! The operator's off-host tamper witness (the "Notary" verb pair).
//!
//! The seventh-pass live drill demonstrated the X-C5 ceiling's exact
//! shape: a host-level actor can `UPDATE knowledge SET content=…` behind the audit
//! chain and every in-tree verifier stays green — `/ump/audit/verify`
//! censuses UMP evidence rows (not business rows) and `/verify` checks
//! claims against CURRENT bytes (there is no approved-bytes memory). The
//! chain proves ITS OWN rows; nothing binds today's business bytes to it.
//!
//! This module is the minimal honest closure: a deterministic state
//! fingerprint the operator records OFF-HOST (paper, password manager,
//! second machine) and later recomputes for comparison. Detection, not
//! prevention — the host can still forge everything on it; it cannot forge
//! the copy in the operator's pocket. Periodic, not continuous: the
//! operator picks the cadence (no background worker, ever — mantra 2).
//!
//! Semantics: the anchor is a WHOLE-STATE fingerprint. ANY legitimate write
//! between two anchor events also trips `--verify` (chain head + row counts
//! move on every audited write). That is by design: verify answers "did
//! anything change since I recorded this?", and when it trips, the audit
//! chain explains what — if the chain verifies clean but the knowledge
//! census moved, that is exactly the behind-the-chain tamper class nothing
//! else detects.
//!
//! ponytail: what this does NOT do —
//!  * no writing: `brain anchor` records NOTHING in the DB (an anchor audit
//!    row would move the chain head it just fingerprinted — the off-host
//!    line IS the evidence);
//!  * no signatures or key material: the anchor's authority is that only
//!    the operator's copy holds it (hashing is SHA-256; printing is safe);
//!  * no per-row census beyond `knowledge` (procedures ride the same table
//!    via `node_kind`): `proposals`/`workflow_runs`/`dsar_requests` are
//!    censused by COUNT — bulk-tamper canaries, disclosed here rather than
//!    silently implied;
//!  * no page-layout fields: `VACUUM`/`shred` re-lay pages without changing
//!    logical state, so page_count/freelist are deliberately absent
//!    (`anchor_ignores_page_layout_vacuum` pins that).
//!
//! The line format is versioned (`v1`) so fields can be added later without
//! misreading an old recorded anchor.

use rusqlite::Connection;
use sha2::{Digest, Sha256};

/// The v1 anchor: every field deterministic, timestamp-free, and stable
/// across reopen + VACUUM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorV1 {
    /// `audit::chain_head` at record time (`None` on an empty chain — an
    /// anchor of a chain-less DB is legal; it fingerprints the business
    /// state and the absence of evidence alike).
    pub chain_head: Option<String>,
    pub audit_rows: i64,
    /// Rolling SHA-256 over `{id}\x1f{sha256(content)}\x1e` lines ordered
    /// by id — the business-row binding the drill showed missing.
    pub knowledge_root: String,
    pub knowledge_rows: i64,
    pub proposals: i64,
    pub workflow_runs: i64,
    pub dsar_requests: i64,
    /// `schema_meta.schema_version` (cosmetic context; not a trust claim).
    pub schema_version: Option<String>,
}

fn count(conn: &Connection, table: &str) -> Result<i64, String> {
    // Table names are compile-time literals from this module only — never
    // user input — so the interpolation is not an injection surface.
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .map_err(|e| format!("count {table}: {e}"))
}

fn scalar_meta(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM schema_meta WHERE key = ?1",
        rusqlite::params![key],
        |r| r.get(0),
    )
    .ok()
}

/// Compute the anchor for the connection's CURRENT state. Read-only: no
/// transaction is opened, nothing is written, no pragma is flipped. Fails
/// CLOSED on any unreadable census input — a witness that silently skips
/// rows it could not read would certify a census it never took.
pub fn fingerprint(conn: &Connection) -> Result<AnchorV1, String> {
    // Knowledge census: streaming, memory-bounded — the live DB's content
    // column is the whole point, so it is hashed row-by-row (audit::hash's
    // SHA-256, re-used verbatim) and rolled into one root. Ordering by id
    // makes the walk deterministic; quarantined rows are business rows too
    // and are included.
    let mut root = Sha256::new();
    let mut stmt = conn
        .prepare("SELECT id, content FROM knowledge ORDER BY id")
        .map_err(|e| format!("census prepare: {e}"))?;
    let mut rows = stmt.query([]).map_err(|e| format!("census query: {e}"))?;
    let mut knowledge_rows = 0i64;
    loop {
        let next = rows
            .next()
            .map_err(|e| format!("census read at row {knowledge_rows}: {e}"))?;
        let Some(row) = next else { break };
        let id: i64 = row.get(0).map_err(|e| format!("census id: {e}"))?;
        let content: String = row.get(1).map_err(|e| format!("census content: {e}"))?;
        knowledge_rows += 1;
        root.update(id.to_string().as_bytes());
        root.update(b"\x1f");
        root.update(crate::audit::hash(&content).as_bytes());
        root.update(b"\x1e");
    }
    Ok(AnchorV1 {
        chain_head: crate::audit::chain_head(conn),
        audit_rows: count(conn, "audit_events")?,
        knowledge_root: crate::audit::hex_encode(&root.finalize()),
        knowledge_rows,
        proposals: count(conn, "proposals")?,
        workflow_runs: count(conn, "workflow_runs")?,
        dsar_requests: count(conn, "dsar_requests")?,
        schema_version: scalar_meta(conn, "schema_version"),
    })
}

/// Render the single recorded line. Field order is part of the v1 format.
pub fn render(a: &AnchorV1) -> String {
    format!(
        "brain-anchor v1 chain={} audit_rows={} knowledge={} knowledge_rows={} proposals={} workflow_runs={} dsar_requests={} schema={}",
        a.chain_head.as_deref().unwrap_or("none"),
        a.audit_rows,
        a.knowledge_root,
        a.knowledge_rows,
        a.proposals,
        a.workflow_runs,
        a.dsar_requests,
        a.schema_version.as_deref().unwrap_or("?"),
    )
}

/// Parse a recorded line back. Returns `None` on anything that is not a v1
/// anchor line — a mangled operator record must refuse, not half-verify.
pub fn parse(line: &str) -> Option<AnchorV1> {
    let line = line.trim();
    if !line.starts_with("brain-anchor v1 ") {
        return None;
    }
    let mut out = AnchorV1 {
        chain_head: None,
        audit_rows: 0,
        knowledge_root: String::new(),
        knowledge_rows: 0,
        proposals: 0,
        workflow_runs: 0,
        dsar_requests: 0,
        schema_version: None,
    };
    for field in line.split_whitespace().skip(2) {
        let (key, value) = field.split_once('=')?;
        match key {
            "chain" => {
                out.chain_head = if value == "none" {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "audit_rows" => out.audit_rows = value.parse().ok()?,
            "knowledge" => out.knowledge_root = value.to_string(),
            "knowledge_rows" => out.knowledge_rows = value.parse().ok()?,
            "proposals" => out.proposals = value.parse().ok()?,
            "workflow_runs" => out.workflow_runs = value.parse().ok()?,
            "dsar_requests" => out.dsar_requests = value.parse().ok()?,
            "schema" => {
                out.schema_version = if value == "?" {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            // Unknown keys refuse the whole line: the format is versioned
            // exactly so a future field never silently degrades a verify.
            _ => return None,
        }
    }
    // All required fields present (schema_version may be absent/`?`).
    if out.knowledge_root.is_empty() {
        return None;
    }
    Some(out)
}

/// Human-diff two anchors. Empty vec = identical state. The diffs name the
/// COMPONENT that moved (never content — no crib for an attacker reading
/// the terminal transcript).
pub fn compare(recorded: &AnchorV1, current: &AnchorV1) -> Vec<String> {
    let mut out = Vec::new();
    if recorded.chain_head != current.chain_head {
        out.push(format!(
            "chain_head: {} -> {}",
            recorded.chain_head.as_deref().unwrap_or("none"),
            current.chain_head.as_deref().unwrap_or("none"),
        ));
    }
    if recorded.audit_rows != current.audit_rows {
        out.push(format!(
            "audit_rows: {} -> {} (audited writes or row removal since the anchor)",
            recorded.audit_rows, current.audit_rows,
        ));
    }
    if recorded.knowledge_root != current.knowledge_root {
        out.push(
            "knowledge_census: MOVED — business-row content changed since the anchor. If the \
             audit chain still verifies, this is the behind-the-chain tamper class (no audited \
             write explains it): compare against your off-host record and investigate."
                .to_string(),
        );
    }
    if recorded.knowledge_rows != current.knowledge_rows {
        out.push(format!(
            "knowledge_rows: {} -> {}",
            recorded.knowledge_rows, current.knowledge_rows,
        ));
    }
    if recorded.proposals != current.proposals {
        out.push(format!(
            "proposals: {} -> {}",
            recorded.proposals, current.proposals
        ));
    }
    if recorded.workflow_runs != current.workflow_runs {
        out.push(format!(
            "workflow_runs: {} -> {}",
            recorded.workflow_runs, current.workflow_runs,
        ));
    }
    if recorded.dsar_requests != current.dsar_requests {
        out.push(format!(
            "dsar_requests: {} -> {}",
            recorded.dsar_requests, current.dsar_requests,
        ));
    }
    // schema_version deliberately NOT compared: it is context, not state —
    // a migration between anchors is visible in audit_rows/chain_head.
    out
}

#[cfg(test)]
mod pins {
    use super::*;

    /// Fresh real-schema DB on a unique temp file (run_migration creates
    /// every table this module reads + WAL mode — the same shape the live
    /// DB has). sqlite-vec registers process-wide first so the VACUUM pin
    /// below can run over the vec0 table the migration creates.
    fn anchored_db(tag: &str) -> (std::path::PathBuf, Connection) {
        crate::register_sqlite_vec::register_sqlite_vec();
        let path =
            std::env::temp_dir().join(format!("brain-anchor-{}-{}.db", tag, std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut conn = Connection::open(&path).expect("open temp db");
        crate::migration::run_migration(&mut conn, 0).expect("migrate temp db");
        (path, conn)
    }

    #[test]
    fn anchor_is_deterministic_across_reopen() {
        let (path, conn) = anchored_db("stable");
        conn.execute(
            "INSERT INTO knowledge (title, content, content_hash, domain) \
             VALUES ('t', 'stable body', 'x', 'global')",
            [],
        )
        .expect("insert row");
        let _ = crate::audit::record(
            &conn,
            crate::audit::AuditKind::Ingest,
            "operator",
            "test:stable",
            crate::audit::AuditStatus::Ok,
            "determinism fixture",
        );
        let first = render(&fingerprint(&conn).expect("fingerprint"));
        drop(conn);
        let reopened = Connection::open(&path).expect("reopen");
        assert_eq!(first, render(&fingerprint(&reopened).expect("fingerprint")));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    /// THE R7-08 closure pin: business-row tamper behind the audit chain
    /// moves the knowledge census while the chain itself stays green —
    /// the exact class every in-tree verifier missed before this module.
    #[test]
    fn anchor_detects_business_row_tamper() {
        let (path, conn) = anchored_db("tamper");
        conn.execute(
            "INSERT INTO knowledge (title, content, content_hash, domain) \
             VALUES ('t', 'approved body', 'x', 'global')",
            [],
        )
        .expect("insert row");
        let _ = crate::audit::record(
            &conn,
            crate::audit::AuditKind::Ingest,
            "operator",
            "test:tamper",
            crate::audit::AuditStatus::Ok,
            "pre-tamper state",
        );
        let recorded = fingerprint(&conn).expect("fingerprint");
        // The demonstrated attack: direct SQL content rewrite, behind the
        // chain, exactly as R7-08 executed it.
        conn.execute(
            "UPDATE knowledge SET content = 'rewritten by host actor' WHERE 1=1",
            [],
        )
        .expect("tamper");
        let current = fingerprint(&conn).expect("fingerprint");
        assert!(
            crate::audit::verify_chain(&conn),
            "fixture must hold: the chain itself stays verifiable (that is the point)"
        );
        let diffs = compare(&recorded, &current);
        assert!(
            diffs.iter().any(|d| d.starts_with("knowledge_census")),
            "the knowledge census must be named in {diffs:?}"
        );
        assert!(
            !diffs.iter().any(|d| d.starts_with("chain_head")),
            "the chain did not move — only the business bytes did"
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn anchor_detects_chain_truncation() {
        let (path, conn) = anchored_db("truncate");
        let _ = crate::audit::record(
            &conn,
            crate::audit::AuditKind::Auth,
            "operator",
            "test:truncate",
            crate::audit::AuditStatus::Ok,
            "row one",
        );
        let _ = crate::audit::record(
            &conn,
            crate::audit::AuditKind::Auth,
            "operator",
            "test:truncate",
            crate::audit::AuditStatus::Ok,
            "row two",
        );
        let recorded = fingerprint(&conn).expect("fingerprint");
        conn.execute(
            "DELETE FROM audit_events WHERE id = (SELECT MAX(id) FROM audit_events)",
            [],
        )
        .expect("truncate tip");
        let diffs = compare(&recorded, &fingerprint(&conn).expect("fingerprint"));
        assert!(
            diffs
                .iter()
                .any(|d| d.starts_with("chain_head") || d.starts_with("audit_rows")),
            "chain truncation must be named in {diffs:?}"
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    /// Page layout is not state: a VACUUM (and therefore the shred verb,
    /// which wraps one) must never trip the anchor.
    #[test]
    fn anchor_ignores_page_layout_vacuum() {
        let (path, conn) = anchored_db("vacuum");
        conn.execute(
            "INSERT INTO knowledge (title, content, content_hash, domain) \
             VALUES ('t', 'vacuum body', 'x', 'global')",
            [],
        )
        .expect("insert row");
        conn.execute("DELETE FROM knowledge WHERE 1=1", [])
            .expect("delete row");
        let before = render(&fingerprint(&conn).expect("fingerprint"));
        conn.execute_batch("VACUUM;").expect("vacuum");
        assert_eq!(before, render(&fingerprint(&conn).expect("fingerprint")));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    /// The recorded line must round-trip, and anything mangled must refuse
    /// (a half-parsed anchor would verify nothing while appearing to).
    #[test]
    fn anchor_line_round_trips_and_refuses_garbage() {
        let (path, conn) = anchored_db("roundtrip");
        let line = render(&fingerprint(&conn).expect("fingerprint"));
        let parsed = parse(&line).expect("round-trip");
        assert_eq!(render(&parsed), line);
        assert!(parse("brain-anchor v2 chain=…").is_none());
        assert!(parse("").is_none());
        assert!(parse("not an anchor at all").is_none());
        // A line missing a required field (no knowledge root) refuses.
        let mut broken = line.clone();
        if let Some(idx) = broken.find(" knowledge=") {
            broken.replace_range(idx..idx + " knowledge=".len(), " droppedfield=");
        }
        // `droppedfield=` hits the unknown-key arm -> None. (If the replace
        // above missed, the assert below still holds via knowledge_root
        // being empty -> None.)
        assert!(parse(&broken).is_none());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
