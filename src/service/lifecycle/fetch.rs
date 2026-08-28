//! The fetch core — the by-id/batch read projections of the lifecycle
//! family, moved verbatim out of the gate handler (the shared knowledge-row
//! projection) and the router file (the `/get/{id}` + `/multi-get` row
//! loads) into the service layer.
//!
//! OWNS (this aggregate's complete storage story):
//! - [`KNOWLEDGE_ROW_COLS`] + [`knowledge_row_to_json`] +
//!   [`load_knowledge_row`] — the shared knowledge column list and the row →
//!   JSON shape the record engine (`emit_record`) renders from. One source
//!   of truth so the record engine never misses a column the export
//!   carries; consumers are the `/ump/*` record paths and `/export`;
//! - [`chunk_in_domain`] — the `/get/{id}` row load: the domain predicate
//!   binds the header-resolved label so an id can never cross domains in
//!   shim mode (multi-db pools are territory-scoped already);
//! - [`chunks_in_domain`] — the `/multi-get` batch load: one
//!   `WHERE id IN (...)` query with positional binds, bounded by
//!   [`crate::config::MAX_MULTI_GET`], the same domain predicate.
//!
//! Read seam: STAYS at the handler emission boundary — these cores return
//! STORED forms (raw text + the row's pii/domain/owner/scope) and the
//! handler runs `sanitize_read*` on every emitted field, re-authorizes
//! against the row's OWN domain (`can_read_domain`), and applies the
//! composite record gate. A batch read filters like recall searches, keeping
//! id-probing of foreign rows blind rather than loud — that filter is
//! AUTHZ and stays handler-side by law.
//!
//! FK-children map: NONE — read-only aggregate (no DELETE, no parent row).
//!
//! Bounds: the batch id list refuses beyond [`crate::config::MAX_MULTI_GET`]
//! HERE (the route's identical 400 fence stays in front, so the wire
//! vocabulary is unchanged — this is the inherited fence for future
//! callers); an empty id list short-circuits to an empty page (the `IN ()`
//! SQL would be invalid).
//!
//! Wire-shape ceiling (honest): the record projection stays the legacy
//! `serde_json::Value` map with the exact pre-move shape — the byte-for-byte
//! wire pins outrank the domain-type aspiration.

use rusqlite::Connection;

/// the shared `knowledge` column list for row rendering
/// (export + the `/ump/*` record paths) — one source of truth so the record
/// engine never misses a column the export carries.
pub(crate) const KNOWLEDGE_ROW_COLS: &str =
    "id, content, node_kind, source, origin, authority, assertion_kind, confidence,
        access_scope, owner, observed_at, valid_from, valid_to,
        content_hash, title, expires_at, created_at, ump_meta, ump_id, region";

/// Row → the JSON shape the record engine (`emit_record`) renders from.
pub(crate) fn knowledge_row_to_json(r: &rusqlite::Row) -> rusqlite::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "id": r.get::<_, i64>(0)?,
        "content": r.get::<_, String>(1)?,
        "memory_kind": r.get::<_, String>(2)?,
        "source": r.get::<_, String>(3)?,
        "origin": r.get::<_, String>(4)?,
        "authority": r.get::<_, Option<f32>>(5)?,
        "assertion_kind": r.get::<_, String>(6)?,
        "confidence": r.get::<_, f32>(7)?,
        "access_scope": r.get::<_, String>(8)?,
        "owner": r.get::<_, Option<String>>(9)?,
        "observed_at": r.get::<_, Option<String>>(10)?,
        "valid_from": r.get::<_, Option<String>>(11)?,
        "valid_to": r.get::<_, Option<String>>(12)?,
        "content_hash": r.get::<_, Option<String>>(13)?,
        "title": r.get::<_, Option<String>>(14)?,
        "expires_at": r.get::<_, Option<i64>>(15)?,
        "created_at": r.get::<_, Option<String>>(16)?
            .map(|s| crate::consolidate::observed_secs(&Some(s)))
            .filter(|&ts| ts != 0),
        "ump_meta": r.get::<_, Option<String>>(17)?,
        "ump_id": r.get::<_, Option<String>>(18)?,
        // the residency stamp on every chunk (data residency).
        "region": r.get::<_, Option<String>>(19)?,
    }))
}

/// One knowledge row by id (same columns as the export) — the `/ump/*`
/// record paths resolve rows through this.
pub(crate) fn load_knowledge_row(
    conn: &rusqlite::Connection,
    id: i64,
) -> Result<Option<serde_json::Value>, rusqlite::Error> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {KNOWLEDGE_ROW_COLS} FROM knowledge WHERE id = ?1"
    ))?;
    let mut rows = stmt.query_map(rusqlite::params![id], knowledge_row_to_json)?;
    rows.next().transpose()
}

/// Typed service error (the ServiceError convention: one enum per module).
/// `Database` carries the rusqlite text VERBATIM — the handler maps it onto
/// the route's frozen internal-error body. `TooManyIds` is the storage
/// boundary re-assertion of the route's `MAX_MULTI_GET` fence.
#[derive(Debug)]
pub(crate) enum FetchError {
    /// A query failed; the rusqlite message travels unchanged.
    Database(String),
    /// The id list exceeds [`crate::config::MAX_MULTI_GET`] (unreachable over
    /// HTTP today; a future caller inherits the fence).
    TooManyIds,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Database(e) => write!(f, "database error: {e}"),
            FetchError::TooManyIds => write!(
                f,
                "batch accepts at most {} ids",
                crate::config::MAX_MULTI_GET
            ),
        }
    }
}

impl From<rusqlite::Error> for FetchError {
    fn from(e: rusqlite::Error) -> Self {
        FetchError::Database(e.to_string())
    }
}

/// A stored chunk row — the raw form the read paths load. The handler keeps
/// the read seam (PII redaction + invisible-Unicode strip + markdown-ref
/// strip per field) and the authz filters; this is the stored truth.
#[derive(Debug, Clone)]
pub(crate) struct ChunkRecord {
    pub id: i64,
    pub title: Option<String>,
    pub content: String,
    pub document_id: Option<String>,
    pub chunk_index: Option<i64>,
    pub heading_path: Option<String>,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub source_uri: Option<String>,
    pub revision_id: Option<i64>,
    pub pii: bool,
    /// The row's OWN domain — the handler re-authorizes against it.
    pub domain: String,
    pub owner: Option<String>,
    pub access_scope: Option<String>,
    /// the ingest kind (`k.source`) — by-id path only (the batch projection
    /// never carried it).
    pub source: Option<String>,
    /// the raw `created_at` TEXT — by-id path only.
    pub created_at: Option<String>,
}

/// The `/get/{id}` row load: the domain predicate binds the
/// header-resolved label so an id can never cross domains in shim mode
/// (multi-db pools are territory-scoped already). `Ok(None)` = no such row
/// in that domain (the route's frozen 404; existence never leaks across
/// domains).
pub(crate) fn chunk_in_domain(
    conn: &Connection,
    id: i64,
    domain_label: &str,
) -> Result<Option<ChunkRecord>, FetchError> {
    let r = conn.query_row(
        "SELECT k.id, k.title, k.content, k.source, k.document_id, k.chunk_index,
                k.heading_path, k.line_start, k.line_end, k.created_at,
                s.uri, sr.id, k.pii, k.domain, k.owner, k.access_scope
         FROM knowledge k
         LEFT JOIN sources s ON k.source_id = s.id
         LEFT JOIN source_revisions sr ON k.revision_id = sr.id
         WHERE k.id = ?1 AND k.domain = ?2",
        rusqlite::params![id, domain_label],
        |row| {
            Ok(ChunkRecord {
                id: row.get(0)?,
                title: row.get(1)?,
                content: row.get(2)?,
                source: row.get(3)?,
                document_id: row.get(4)?,
                chunk_index: row.get(5)?,
                heading_path: row.get(6)?,
                line_start: row.get(7)?,
                line_end: row.get(8)?,
                created_at: row.get(9)?,
                source_uri: row.get(10)?,
                revision_id: row.get(11)?,
                pii: row.get::<_, i64>(12)? != 0,
                domain: row.get(13)?,
                owner: row.get(14)?,
                access_scope: row.get(15)?,
            })
        },
    );
    match r {
        Ok(rec) => Ok(Some(rec)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(FetchError::from(e)),
    }
}

/// The `/multi-get` batch load: a single `WHERE id IN (...)` query instead
/// of N round-trips, safe parameterization (placeholders from the ids
/// length, each id bound by position), bounded by
/// [`crate::config::MAX_MULTI_GET`], the same domain predicate as
/// [`chunk_in_domain`].
pub(crate) fn chunks_in_domain(
    conn: &Connection,
    ids: &[i64],
    domain_label: &str,
) -> Result<Vec<ChunkRecord>, FetchError> {
    // single `WHERE id IN (...)` query instead of N round-trips.
    // Safe parameterization: build placeholders from the ids length, bind
    // each id by position. Bounded by MAX_MULTI_GET (1000).
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    if ids.len() > crate::config::MAX_MULTI_GET {
        return Err(FetchError::TooManyIds);
    }
    // the domain predicate binds the
    // header-resolved label, so ids cannot cross domains in shim mode.
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let label_ph = ids.len() + 1;
    let sql = format!(
        "SELECT k.id, k.title, k.content, k.document_id, k.chunk_index,\
                k.heading_path, k.line_start, k.line_end, s.uri, sr.id, k.pii,\
                k.domain, k.owner, k.access_scope \
         FROM knowledge k \
         LEFT JOIN sources s ON k.source_id = s.id \
         LEFT JOIN source_revisions sr ON k.revision_id = sr.id \
         WHERE k.id IN ({}) AND k.domain = ?{label_ph}",
        placeholders.join(",")
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = ids
        .iter()
        .map(|i| Box::new(*i) as Box<dyn rusqlite::ToSql>)
        .collect();
    params.push(Box::new(domain_label.to_string()));
    let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok(ChunkRecord {
            id: row.get(0)?,
            title: row.get(1)?,
            content: row.get(2)?,
            document_id: row.get(3)?,
            chunk_index: row.get(4)?,
            heading_path: row.get(5)?,
            line_start: row.get(6)?,
            line_end: row.get(7)?,
            source_uri: row.get(8)?,
            revision_id: row.get(9)?,
            pii: row.get::<_, i64>(10)? != 0,
            domain: row.get(11)?,
            owner: row.get(12)?,
            access_scope: row.get(13)?,
            // the batch projection never carried these two columns.
            source: None,
            created_at: None,
        })
    })?;
    let mut out = Vec::with_capacity(ids.len());
    for v in rows {
        out.push(v?);
    }
    Ok(out)
}

#[cfg(test)]
mod pins {
    use super::*;

    fn fresh_conn() -> rusqlite::Connection {
        crate::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().expect("db");
        brain_server::migration::run_migration(&mut conn, 1).expect("migration");
        conn
    }

    /// the by-id record projection round-trips every column the record
    /// engine renders, including the TEXT `created_at` → unix-secs mapping
    /// (the mapper `knowledge_row_to_json`, which maps the TEXT column) and
    /// the region stamp. (Companion to the export mapper pin that stayed
    /// with the export surface.)
    #[test]
    fn load_knowledge_row_projects_every_rendered_column() {
        let conn = fresh_conn();
        conn.execute(
            "INSERT INTO knowledge (title, content, source, content_hash, domain, owner) \
             VALUES ('t', 'c', 'manual', 'h', 'global', 'alice')",
            [],
        )
        .unwrap();
        let id: i64 = conn
            .query_row("SELECT id FROM knowledge", [], |r| r.get(0))
            .unwrap();
        let row = load_knowledge_row(&conn, id)
            .expect("query ok")
            .expect("row found");
        assert_eq!(row["id"], id);
        assert_eq!(row["content"], "c");
        assert_eq!(row["memory_kind"], "fact");
        assert_eq!(row["origin"], "imported");
        assert_eq!(row["owner"], "alice");
        assert!(row.get("region").is_some(), "the residency stamp renders");
        assert!(
            load_knowledge_row(&conn, id + 999)
                .expect("query ok")
                .is_none(),
            "a missing id is None, not an error"
        );
    }

    /// the `/get/{id}` + `/multi-get` loads are domain-scoped: an id lives
    /// in exactly one domain label, and a fetch under another label is the
    /// frozen None/empty (existence never leaks across domains in shim
    /// mode).
    #[test]
    fn fetch_projections_are_domain_scoped() {
        let conn = fresh_conn();
        conn.execute(
            "INSERT INTO knowledge (content, source, content_hash, domain) \
             VALUES ('in-support', 'manual', 'h1', 'support')",
            [],
        )
        .unwrap();
        let id: i64 = conn
            .query_row("SELECT id FROM knowledge", [], |r| r.get(0))
            .unwrap();
        assert!(
            chunk_in_domain(&conn, id, "support").unwrap().is_some(),
            "the row resolves under its own domain label"
        );
        assert!(
            chunk_in_domain(&conn, id, "global").unwrap().is_none(),
            "a foreign label must not resolve the row"
        );
        assert!(chunks_in_domain(&conn, &[id], "global").unwrap().is_empty());
        let batch = chunks_in_domain(&conn, &[id, id + 1], "support").unwrap();
        assert_eq!(batch.len(), 1, "missing ids drop silently from the batch");
        assert_eq!(batch[0].id, id);
        assert_eq!(batch[0].domain, "support");
    }

    /// the batch fence: an empty id list short-circuits to an empty page
    /// (the `IN ()` SQL would be invalid) and an oversized list refuses at
    /// the storage boundary — the route's identical 400 stays in front.
    #[test]
    fn chunks_in_domain_reasserts_the_bounds_fence() {
        let conn = fresh_conn();
        assert!(
            chunks_in_domain(&conn, &[], "global").unwrap().is_empty(),
            "empty batch short-circuits"
        );
        let oversized: Vec<i64> = (0..=crate::config::MAX_MULTI_GET as i64).collect();
        assert!(
            matches!(
                chunks_in_domain(&conn, &oversized, "global"),
                Err(FetchError::TooManyIds)
            ),
            "oversized batch refuses at the storage boundary"
        );
    }
}
