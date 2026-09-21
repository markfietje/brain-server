//! The persistence half of the curated legal vocabulary: a SQLite file at
//! `BRAIN_LEGAL_DB_PATH` holding the law versions, jurisdiction rules, and
//! surveillance postures, with an FTS5 index over rule bodies and a
//! `schema_meta` head pin that mirrors the audit chain's. The server reads
//! this file per request with a READ-ONLY connection — hot-reload means the
//! next request reads the current file, nothing more; population is the
//! DPO's quarterly import (operator procedure), never a route, never
//! automated. Absent curation stays honestly empty (`effective_at = 0`,
//! `reviewed_at = NULL`, empty posture attestation) — this module never
//! invents law.
//!
//! The crate's dependency law is one new edge (rusqlite): JSON emission for
//! whole responses belongs to the server layer (the response types here are
//! plain serde structs), and the two stored list columns plus the flat head
//! pin use the tiny deterministic encoder below, pinned round-trip by test.

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

/// The `schema_meta` key holding the head pin — the structural mirror of the
/// audit chain's `audit_chain_head`: one row, written only by the seed or an
/// import that records the reviewer, verified against the rows on read.
pub const HEAD_PIN_META_KEY: &str = "law_version";

/// Posture status stamped on every seeded surveillance mechanism: the
/// operator knows which safeguard they signed; this crate records the
/// mechanism vocabulary and waits for that attestation.
pub const POSTURE_UNATTESTED: &str = "operator-attestation-required";

/// Every refusal names its reason; nothing here fails silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegalDbError {
    Io(String),
    Sqlite(String),
    NotSeeded,
    AlreadySeeded,
    UnknownLawVersion(String),
    HeadPinMismatch { pinned: String, actual: String },
}

impl std::fmt::Display for LegalDbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LegalDbError::Io(e) => write!(f, "legal db io error: {e}"),
            LegalDbError::Sqlite(e) => write!(f, "legal db sqlite error: {e}"),
            LegalDbError::NotSeeded => write!(f, "legal db is not seeded (no head pin)"),
            LegalDbError::AlreadySeeded => {
                write!(
                    f,
                    "legal db already carries seeded law versions; import, don't reseed"
                )
            }
            LegalDbError::UnknownLawVersion(v) => write!(f, "unknown law_version: {v}"),
            LegalDbError::HeadPinMismatch { pinned, actual } => write!(
                f,
                "legal db head pin mismatch: pinned {pinned} but rows are {actual}"
            ),
        }
    }
}

impl std::error::Error for LegalDbError {}

fn sql_err(e: rusqlite::Error) -> LegalDbError {
    LegalDbError::Sqlite(e.to_string())
}

/// Write a JSON array of strings — byte-deterministic, escaping the characters
/// JSON requires. The curated vocabularies stored this way are plain
/// lowercase identifiers; the escaper exists so a future import cannot
/// corrupt the file's shape.
fn json_string_array(items: &[String]) -> String {
    let mut out = String::with_capacity(2 + items.iter().map(|i| i.len() + 3).sum::<usize>());
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        for ch in item.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => out.push(c),
            }
        }
        out.push('"');
    }
    out.push(']');
    out
}

/// Read back what [`json_string_array`] wrote. Strict round-trip: only the
/// writer's shape (a top-level array of double-quoted strings) is accepted;
/// anything else is a named error, never a guessed parse.
fn parse_json_string_array(s: &str) -> Result<Vec<String>, LegalDbError> {
    let bytes = s.trim();
    if !bytes.starts_with('[') || !bytes.ends_with(']') {
        return Err(LegalDbError::Io(format!("not a JSON string array: {s:?}")));
    }
    let inner = &bytes[1..bytes.len() - 1];
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut saw_any = false;
    for ch in inner.chars() {
        if in_string {
            if escaped {
                match ch {
                    '"' => cur.push('"'),
                    '\\' => cur.push('\\'),
                    'n' => cur.push('\n'),
                    'r' => cur.push('\r'),
                    't' => cur.push('\t'),
                    'u' => {
                        return Err(LegalDbError::Io(
                            "the writer never emits \\u escapes".to_string(),
                        ));
                    }
                    other => {
                        return Err(LegalDbError::Io(format!("unknown escape \\{other}")));
                    }
                }
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => {
                    in_string = false;
                    items.push(std::mem::take(&mut cur));
                }
                other => cur.push(other),
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                saw_any = true;
            }
            ' ' | ',' => {}
            other => {
                return Err(LegalDbError::Io(format!(
                    "unexpected character {other:?} outside a string"
                )));
            }
        }
    }
    if in_string || escaped || (!items.is_empty() && !saw_any) {
        return Err(LegalDbError::Io("unterminated string array".to_string()));
    }
    Ok(items)
}

/// Open the curated DB READ-ONLY. This is the server-facing law: no route
/// can write the legal DB, and a missing or corrupt file is a named error.
pub fn open_readonly(path: &std::path::Path) -> Result<Connection, LegalDbError> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(sql_err)
}

/// The curated law version table: (jurisdiction, version) plus its curation
/// metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LawVersionRow {
    pub jurisdiction: String,
    pub version: String,
    pub effective_at: i64,
    pub source_ref: String,
    pub reviewed_by: String,
    pub reviewed_at: Option<i64>,
}

/// One jurisdiction rule row — the DB normalization of the crate's [`crate::Rule`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuleRow {
    pub id: i64,
    pub jurisdiction: String,
    pub subject: String,
    pub rule_key: String,
    pub body: String,
    pub source_ref: String,
    pub law_version: String,
    pub deadline_days: Option<i64>,
    pub rights: Vec<String>,
    pub effective_at: i64,
    pub reviewed_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub revision: i64,
    pub superseded_by: Option<i64>,
}

/// One surveillance mechanism row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostureRow {
    pub mechanism: String,
    pub law_version: String,
    pub jurisdictions: Vec<String>,
    pub status: String,
    pub source_ref: String,
    pub reviewed_by: String,
    pub reviewed_at: Option<i64>,
}

/// The head pin: counts and the max rule id over the curated tables. Written
/// only by [`seed`]/[`pin_head`]; verified against the live rows by
/// [`verify_head`] exactly as the audit chain head is. The stored form is the
/// exact byte string [`head_pin_json`] produces — verification is a string
/// comparison, so the pin cannot drift in representation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeadPin {
    pub rules: i64,
    pub versions: i64,
    pub postures: i64,
    pub max_rules_id: i64,
}

fn head_pin_json(pin: &HeadPin) -> String {
    format!(
        "{{\"rules\":{},\"versions\":{},\"postures\":{},\"max_rules_id\":{}}}",
        pin.rules, pin.versions, pin.postures, pin.max_rules_id
    )
}

/// The C2 diff payload: every rule newer than the `since` version's snapshot
/// point, ordered by (jurisdiction, subject, effective_at, revision, id).
/// Field order is the serialization order — the server layer's response is
/// byte-reproducible for one DB state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffResponse {
    pub since: Option<String>,
    pub since_effective_at: Option<i64>,
    pub head: HeadPin,
    pub rules: Vec<RuleRow>,
}

/// The schema: five tables, `IF NOT EXISTS` everywhere, idempotent. The FTS5
/// index is external-content over `jurisdiction_rules` so the rule rows stay
/// the single storage of truth.
pub fn create_schema(conn: &Connection) -> Result<(), LegalDbError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_meta(
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS law_version(
            jurisdiction TEXT NOT NULL,
            version      TEXT NOT NULL,
            effective_at INTEGER NOT NULL DEFAULT 0,
            source_ref   TEXT NOT NULL DEFAULT '',
            reviewed_by  TEXT NOT NULL DEFAULT '',
            reviewed_at  INTEGER,
            PRIMARY KEY (jurisdiction, version)
         );
         CREATE TABLE IF NOT EXISTS jurisdiction_rules(
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            jurisdiction  TEXT NOT NULL,
            subject       TEXT NOT NULL,
            rule_key      TEXT NOT NULL,
            body          TEXT NOT NULL,
            source_ref    TEXT NOT NULL DEFAULT '',
            law_version   TEXT NOT NULL DEFAULT '',
            deadline_days INTEGER,
            rights        TEXT NOT NULL DEFAULT '[]',
            effective_at  INTEGER NOT NULL DEFAULT 0,
            reviewed_at   INTEGER,
            expires_at    INTEGER,
            revision      INTEGER NOT NULL DEFAULT 1,
            superseded_by INTEGER
         );
         CREATE TABLE IF NOT EXISTS surveillance_postures(
            mechanism    TEXT NOT NULL,
            law_version  TEXT NOT NULL DEFAULT '',
            jurisdictions TEXT NOT NULL DEFAULT '[]',
            status       TEXT NOT NULL DEFAULT '',
            source_ref   TEXT NOT NULL DEFAULT '',
            reviewed_by  TEXT NOT NULL DEFAULT '',
            reviewed_at  INTEGER,
            PRIMARY KEY (mechanism, law_version)
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS rules_fts USING fts5(
            body, source_ref,
            content='jurisdiction_rules', content_rowid='id'
         );
         CREATE INDEX IF NOT EXISTS idx_jurisdiction_rules_order
             ON jurisdiction_rules(jurisdiction, subject, effective_at, revision);",
    )
    .map_err(sql_err)
}

fn fts_sync(conn: &Connection, id: i64, body: &str, source_ref: &str) -> Result<(), LegalDbError> {
    conn.execute(
        "INSERT INTO rules_fts(rowid, body, source_ref) VALUES (?1, ?2, ?3)",
        params![id, body, source_ref],
    )
    .map_err(sql_err)?;
    Ok(())
}

/// Insert one curated law version row.
pub fn insert_law_version(conn: &Connection, row: &LawVersionRow) -> Result<(), LegalDbError> {
    conn.execute(
        "INSERT INTO law_version(jurisdiction, version, effective_at, source_ref, reviewed_by, reviewed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            row.jurisdiction,
            row.version,
            row.effective_at,
            row.source_ref,
            row.reviewed_by,
            row.reviewed_at,
        ],
    )
    .map_err(sql_err)?;
    Ok(())
}

/// Insert one jurisdiction rule and keep the FTS index in step. The rule's
/// fields are the crate's [`crate::Rule`] vocabulary (plus the transfers
/// table's `deadline_days`/`rights`), never a new shape.
pub fn insert_jurisdiction_rule(
    conn: &Connection,
    rule: &crate::Rule,
    deadline_days: Option<i64>,
    rights: &[&str],
) -> Result<i64, LegalDbError> {
    let rights_owned: Vec<String> = rights.iter().map(|s| (*s).to_string()).collect();
    conn.execute(
        "INSERT INTO jurisdiction_rules(
            jurisdiction, subject, rule_key, body, source_ref, law_version,
            deadline_days, rights, effective_at, reviewed_at, expires_at, revision, superseded_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            rule.jurisdiction,
            rule.subject,
            rule.rule_key,
            rule.body,
            rule.source_ref,
            rule.law_version,
            deadline_days,
            json_string_array(&rights_owned),
            rule.effective_at,
            rule.reviewed_at,
            rule.expires_at,
            rule.revision,
            rule.superseded_by,
        ],
    )
    .map_err(sql_err)?;
    let id = conn.last_insert_rowid();
    fts_sync(conn, id, &rule.body, &rule.source_ref)?;
    Ok(id)
}

/// Insert one surveillance mechanism row. The mechanism vocabulary is
/// operator-set (the transfers module's own posture): this crate records the
/// label and waits for the operator's attestation — it never invents posture
/// text or jurisdiction claims.
pub fn insert_posture(conn: &Connection, posture: &PostureRow) -> Result<(), LegalDbError> {
    conn.execute(
        "INSERT INTO surveillance_postures(mechanism, law_version, jurisdictions, status, source_ref, reviewed_by, reviewed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            posture.mechanism,
            posture.law_version,
            json_string_array(&posture.jurisdictions),
            posture.status,
            posture.source_ref,
            posture.reviewed_by,
            posture.reviewed_at,
        ],
    )
    .map_err(sql_err)?;
    Ok(())
}

/// One curated DSAR row: (jurisdiction, law, law_version, deadline_days,
/// rights).
type DsarSeedRow = (
    &'static str,
    &'static str,
    &'static str,
    Option<i64>,
    &'static [&'static str],
);

/// The transfers register's curated DSAR rules, mirrored here as the initial
/// DB content. The single-owner join is the SDK's law-version table: the
/// kernel's own test pins transfers ↔ SDK, the crate test pins seed ↔ SDK,
/// and the mirror below is byte-pinned at seed time by the three-way test.
const DSAR_SEED: &[DsarSeedRow] = &[
    (
        "eu",
        "GDPR (Art 12/15/17)",
        "gdpr-consolidated-2021",
        Some(30),
        &[
            "access",
            "erasure",
            "portability",
            "rectification",
            "objection",
            "automated-decision",
        ],
    ),
    (
        "uk",
        "UK GDPR (Art 12/15/17)",
        "uk-gdpr-idta-2021",
        Some(30),
        &[
            "access",
            "erasure",
            "portability",
            "rectification",
            "objection",
        ],
    ),
    (
        "us",
        "CCPA/CPRA (Cal. Civ. Code 1798)",
        "ccpa-cpra-2023-amended",
        Some(45),
        &["access", "erasure", "rectification", "opt-out-of-sale"],
    ),
    (
        "au",
        "Privacy Act 1988 (APPs)",
        "privacy-act-apps-2019",
        Some(30),
        &["access", "erasure", "rectification"],
    ),
    (
        "sg",
        "PDPA 2012",
        "pdpa-2020-amended",
        Some(30),
        &["access", "erasure", "rectification"],
    ),
    (
        "ca",
        "PIPEDA (SC 2000 c.5)",
        "pipeda-2019-amended",
        Some(30),
        &["access", "erasure", "rectification"],
    ),
    (
        "ph",
        "RA 10173 (DPA 2012)",
        "npc-advisory-2024-04",
        None,
        &["access", "erasure", "rectification"],
    ),
];

/// The transfers register's curated transfer-mechanism vocabulary.
const MECHANISM_SEED: &[&str] = &[
    "scc-eu-2021",
    "uk-idta",
    "dpf-us",
    "cbpr",
    "bcr",
    "adequacy",
];

/// Seed the DB with ONLY the existing curated data: the SDK's law-version
/// table (read through the SDK constant, never copied), the crate's own VAT
/// rules, the transfers register's DSAR rules, and the transfer-mechanism
/// vocabulary. The transfers tables carry no effective dates, so those rows
/// carry `effective_at = 0` — "in force, effective date not yet curated" —
/// and the DPO import curates them. Seeding pins the head. Re-seeding an
/// already-seeded file is a NAMED refusal (import, don't reseed).
pub fn seed(conn: &Connection, reviewed_by: &str, reviewed_at: i64) -> Result<(), LegalDbError> {
    create_schema(conn)?;
    let versions: i64 = conn
        .query_row("SELECT COUNT(*) FROM law_version", [], |r| r.get(0))
        .map_err(sql_err)?;
    if versions > 0 {
        return Err(LegalDbError::AlreadySeeded);
    }
    // The SDK table is the single owner of the law-version vocabulary.
    for (code, label) in brain_engine_sdk::policy::LAW_VERSIONS {
        insert_law_version(
            conn,
            &LawVersionRow {
                jurisdiction: (*code).to_string(),
                version: (*label).to_string(),
                effective_at: 0,
                source_ref: "curated snapshot; effective date not yet curated".to_string(),
                reviewed_by: reviewed_by.to_string(),
                reviewed_at: Some(reviewed_at),
            },
        )?;
    }
    // The crate's own curated VAT rules (with their real dates).
    let (vat_rules, _) = crate::seed_rules();
    for rule in &vat_rules {
        insert_jurisdiction_rule(conn, rule, None, &[])?;
    }
    // The transfers register's DSAR rules. Effective date not yet curated → 0.
    for (code, law, label, deadline, rights) in DSAR_SEED {
        let rule = crate::Rule {
            id: 0,
            law_version: (*label).to_string(),
            jurisdiction: (*code).to_string(),
            subject: "dsar".to_string(),
            rule_key: format!("{code}_dsar_curated"),
            body: (*law).to_string(),
            source_ref: (*law).to_string(),
            effective_at: 0,
            reviewed_at: Some(reviewed_at),
            expires_at: None,
            revision: 1,
            superseded_by: None,
            created_at: reviewed_at,
        };
        insert_jurisdiction_rule(conn, &rule, *deadline, rights)?;
    }
    // The transfer-mechanism vocabulary: recorded, unattested, empty. No
    // jurisdiction claims, no posture text — the operator attests.
    for mechanism in MECHANISM_SEED {
        insert_posture(
            conn,
            &PostureRow {
                mechanism: (*mechanism).to_string(),
                law_version: String::new(),
                jurisdictions: Vec::new(),
                status: POSTURE_UNATTESTED.to_string(),
                source_ref: "transfers register mechanism vocabulary".to_string(),
                reviewed_by: reviewed_by.to_string(),
                reviewed_at: Some(reviewed_at),
            },
        )?;
    }
    pin_head(conn)?;
    Ok(())
}

fn head_snapshot(conn: &Connection) -> Result<HeadPin, LegalDbError> {
    let one = |q: &str| -> Result<i64, LegalDbError> {
        conn.query_row(q, [], |r| r.get(0)).map_err(sql_err)
    };
    Ok(HeadPin {
        rules: one("SELECT COUNT(*) FROM jurisdiction_rules")?,
        versions: one("SELECT COUNT(*) FROM law_version")?,
        postures: one("SELECT COUNT(*) FROM surveillance_postures")?,
        max_rules_id: one("SELECT COALESCE(MAX(id), 0) FROM jurisdiction_rules")?,
    })
}

/// Write (or re-write after an import) the head pin. Returns the exact bytes
/// stored.
pub fn pin_head(conn: &Connection) -> Result<String, LegalDbError> {
    let json = head_pin_json(&head_snapshot(conn)?);
    conn.execute(
        "INSERT INTO schema_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = ?2",
        params![HEAD_PIN_META_KEY, json],
    )
    .map_err(sql_err)?;
    Ok(json)
}

/// The stored head-pin bytes, when the file carries one. The report route
/// surfaces this verbatim so a reader can compare it against any pin.
pub fn stored_head(conn: &Connection) -> Result<Option<String>, LegalDbError> {
    conn.query_row(
        "SELECT value FROM schema_meta WHERE key = ?1",
        params![HEAD_PIN_META_KEY],
        |r| r.get(0),
    )
    .optional()
    .map_err(sql_err)
}

/// The curated DB's CURRENT law version for the jurisdiction of `version` —
/// the newest `effective_at` (ties broken by the greater version label).
/// `None` when the named version is unknown to the DB. This is the
/// comparable "head" for the report route's mismatch advisory: a pinned
/// version versus the newest version known for the same law.
pub fn current_version_for(
    conn: &Connection,
    version: &str,
) -> Result<Option<String>, LegalDbError> {
    conn.query_row(
        "SELECT current.version FROM law_version AS pinned
         JOIN law_version AS current ON current.jurisdiction = pinned.jurisdiction
         WHERE pinned.version = ?1
         ORDER BY current.effective_at DESC, current.version DESC LIMIT 1",
        params![version],
        |r| r.get(0),
    )
    .optional()
    .map_err(sql_err)
}

/// The head↔rows consistency law, the audit pin's mirror: the stored pin
/// must describe the live tables exactly. Drift (an import that forgot to
/// re-pin, a truncated file) is a NAMED mismatch, never a silent pass.
pub fn verify_head(conn: &Connection) -> Result<(), LegalDbError> {
    let pinned = stored_head(conn)?.ok_or(LegalDbError::NotSeeded)?;
    let actual = head_pin_json(&head_snapshot(conn)?);
    if pinned != actual {
        return Err(LegalDbError::HeadPinMismatch { pinned, actual });
    }
    Ok(())
}

/// The C2 diff: rules newer than the `since` version's snapshot point (its
/// `effective_at`), ordered by (jurisdiction, subject, effective_at,
/// revision, id). `None` since = the full ordered snapshot. An unknown
/// `since` version is a NAMED refusal, never an empty diff.
pub fn diff_since(conn: &Connection, since: Option<&str>) -> Result<DiffResponse, LegalDbError> {
    let since_effective_at: Option<i64> = match since {
        Some(v) => {
            let e: Option<i64> = conn
                .query_row(
                    "SELECT effective_at FROM law_version WHERE version = ?1
                     ORDER BY jurisdiction LIMIT 1",
                    params![v],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql_err)?;
            Some(e.ok_or_else(|| LegalDbError::UnknownLawVersion(v.to_string()))?)
        }
        None => None,
    };
    let lower = since_effective_at.unwrap_or(i64::MIN);
    let mut stmt = conn
        .prepare(
            "SELECT id, jurisdiction, subject, rule_key, body, source_ref, law_version,
                    deadline_days, rights, effective_at, reviewed_at, expires_at, revision, superseded_by
               FROM jurisdiction_rules
              WHERE effective_at > ?1
              ORDER BY jurisdiction, subject, effective_at, revision, id",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map(params![lower], |r| {
            let rights_json: String = r.get(8)?;
            Ok((
                rights_json,
                RuleRow {
                    id: r.get(0)?,
                    jurisdiction: r.get(1)?,
                    subject: r.get(2)?,
                    rule_key: r.get(3)?,
                    body: r.get(4)?,
                    source_ref: r.get(5)?,
                    law_version: r.get(6)?,
                    deadline_days: r.get(7)?,
                    rights: Vec::new(),
                    effective_at: r.get(9)?,
                    reviewed_at: r.get(10)?,
                    expires_at: r.get(11)?,
                    revision: r.get(12)?,
                    superseded_by: r.get(13)?,
                },
            ))
        })
        .map_err(sql_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_err)?;
    let rules = rows
        .into_iter()
        .map(|(rights_json, mut row)| {
            row.rights = parse_json_string_array(&rights_json)?;
            Ok(row)
        })
        .collect::<Result<Vec<_>, LegalDbError>>()?;
    Ok(DiffResponse {
        since: since.map(str::to_string),
        since_effective_at,
        head: head_snapshot(conn)?,
        rules,
    })
}

/// Full-text search over rule bodies + source refs; returns rule ids in id
/// order. The index proves the FTS5 half of the contract in-tree.
pub fn search_rules(conn: &Connection, query: &str) -> Result<Vec<i64>, LegalDbError> {
    let mut stmt = conn
        .prepare("SELECT rowid FROM rules_fts WHERE rules_fts MATCH ?1 ORDER BY rowid")
        .map_err(sql_err)?;
    let rows = stmt
        .query_map(params![query], |r| r.get(0))
        .map_err(sql_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_err)?;
    Ok(rows)
}
