//! v0.9.6 "Bridge" — connector contract.
//!
//! A *connector* is an out-of-process binary that pulls documents from an
//! external source (GitHub, …) into brain-server's existing source/revision
//! model via the HTTP API. The server's job is narrow: register instances,
//! surface status, and (in M2.x) spawn connector binaries via `supervisor`.
//! All outbound HTTP to the external source happens in the connector binary,
//! never in the server process — see `bin_common/http.rs` line 4 for the
//! rationale (the server deliberately has no outbound HTTP client dep).
//!
//! M1 scope: manifest + spawn primitive + a stub binary + migration. No real
//! connector logic yet (that's M2.x). The contract documented below is what
//! `brain-connector-gh` (M2) and future connectors will implement.
//!
//! The M1 module is built but not yet wired into the server runtime — the
//! spawn loop lands with M2.x's "auto-start registered connectors on boot",
//! and the registration path lands with M3's `brain connect` CLI. Until then,
//! `#[allow(dead_code)]` keeps clippy quiet.

#![allow(dead_code)]
//!
//! ## Connector binary contract
//!
//! A connector binary MUST:
//! 1. Read its config from `<path>` (argv: `--config <path>`).
//! 2. Read its checkpoint DB path from argv: `--checkpoint <path>`.
//! 3. Authenticate to brain-server using `BRAIN_TOKEN_FILE` → `BRAIN_TOKEN` →
//!    the default file at `~/.config/brain-server/auth-token` (same resolver
//!    as the other CLIs; see `bin_common/http.rs`).
//! 4. Ingest via `POST /ingest/markdown` with `source_path` set to a stable
//!    connector-defined URI (e.g. `github://owner/repo/issues/42`).
//! 5. Reconcile via `POST /sources/reconcile` with `{kind, live_uris}`.
//! 6. Emit one JSON line per event to stdout:
//!    `{"type":"log","level":"info","msg":"…"}` |
//!    `{"type":"progress","cursor":"…","count":N}` |
//!    `{"type":"done","report":{…}}` |
//!    `{"type":"error","msg":"…","retry":bool}`.
//! 7. Exit 0 on success, non-zero on hard failure.
//!
//! Idempotent ingest (per `sources::upsert_revision`) means a restart always
//! resumes from the last committed checkpoint without duplicating.

pub mod auth;
#[cfg(feature = "connector-github")]
pub mod github;
pub mod supervisor;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// The maximum supported connector manifest schema version. Bumped when the
/// manifest format changes in a way that requires server-side handling.
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Connector manifest. Loaded from `~/.config/brain-server/connectors/*.toml`
/// in M2.x; in M1 we construct these in-process for the stub.
///
/// `binary` is the connector binary name (resolved via `$PATH` at spawn time)
/// or an absolute path. `scopes` are declared capabilities (informational; the
/// server does not enforce them — enforcement is at the external API level,
/// e.g. GitHub App installation-token scope).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectorManifest {
    /// Connector kind, e.g. `"github"`. Lowercase, matches `connectors.kind`.
    pub kind: String,
    /// Manifest schema version (independent of the connector's own version).
    pub schema_version: u32,
    /// Binary name (looked up via `$PATH`) or absolute path.
    pub binary: String,
    /// Connector-defined capability strings (informational).
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Free-form connector version (informational, surfaced in `/connectors`).
    pub version: String,
}

impl ConnectorManifest {
    /// Construct a manifest for the M1 stub connector. Used by tests and by
    /// `brain connect` (when it lands in M3) to register the stub.
    pub fn stub() -> Self {
        Self {
            kind: "stub".to_string(),
            schema_version: MANIFEST_SCHEMA_VERSION,
            binary: "brain-connector-stub".to_string(),
            scopes: Vec::new(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    /// Validate the manifest fields the server actually depends on. The
    /// connector-defined `scopes` and `version` are informational and not
    /// checked here. Returns the kind, trimmed and lowercased.
    pub fn validate(&self) -> Result<String> {
        let kind = self.kind.trim().to_lowercase();
        if kind.is_empty() {
            anyhow::bail!("manifest `kind` must not be empty");
        }
        if self.binary.trim().is_empty() {
            anyhow::bail!("manifest `binary` must not be empty");
        }
        if self.schema_version > MANIFEST_SCHEMA_VERSION {
            anyhow::bail!(
                "manifest schema_version {} is newer than supported {}",
                self.schema_version,
                MANIFEST_SCHEMA_VERSION
            );
        }
        Ok(kind)
    }
}

/// A row in the `connectors` table, as surfaced by `GET /connectors`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConnectorRow {
    pub id: i64,
    pub kind: String,
    pub instance: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// List all registered connectors of any kind. Used by `GET /connectors`.
pub fn list_connectors(conn: &Connection) -> Result<Vec<ConnectorRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, instance, state, last_sync_at, last_error \
         FROM connectors ORDER BY kind, instance",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(ConnectorRow {
            id: r.get(0)?,
            kind: r.get(1)?,
            instance: r.get(2)?,
            state: r.get(3)?,
            last_sync_at: r.get(4)?,
            last_error: r.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.context("connector row decode")?);
    }
    Ok(out)
}

/// Register (or reactivate) a connector instance. Returns its id.
/// Idempotent: re-registering the same `(kind, instance)` reactivates the row
/// (state ← `'registered'`) rather than inserting a duplicate.
pub fn upsert_connector(
    conn: &Connection,
    kind: &str,
    instance: &str,
    config_json: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO connectors (kind, instance, config_json, state) \
         VALUES (?1, ?2, ?3, 'registered') \
         ON CONFLICT(kind, instance) DO UPDATE SET \
             config_json = excluded.config_json, \
             state = 'registered', \
             updated_at = CURRENT_TIMESTAMP",
        params![kind, instance, config_json],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM connectors WHERE kind = ?1 AND instance = ?2",
        params![kind, instance],
        |r| r.get(0),
    )?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let db = Connection::open_in_memory().expect("open in-memory DB");
        db.execute_batch(
            "CREATE TABLE connectors(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                instance TEXT NOT NULL,
                config_json TEXT NOT NULL DEFAULT '{}',
                state TEXT NOT NULL DEFAULT 'registered',
                last_sync_at TEXT,
                last_error TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(kind, instance));
             CREATE TABLE connector_checkpoints(
                connector_id INTEGER NOT NULL REFERENCES connectors(id) ON DELETE CASCADE,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (connector_id, key));",
        )
        .expect("create schema");
        db
    }

    #[test]
    fn test_manifest_parses_minimal_github() {
        // The shape we expect M2.x's github.toml to take. Parsing is via serde
        // (TOML files are deserialized in M2; here we just exercise the struct).
        let json = r#"{
            "kind": "github",
            "schema_version": 1,
            "binary": "brain-connector-gh",
            "scopes": ["repo:issues", "repo:pulls"],
            "version": "0.9.6"
        }"#;
        let m: ConnectorManifest =
            serde_json::from_str(json).expect("deserializes a real GitHub manifest");
        assert_eq!(m.validate().unwrap(), "github");
        assert_eq!(m.scopes.len(), 2);
    }

    #[test]
    fn test_manifest_rejects_empty_kind() {
        let m = ConnectorManifest {
            kind: "   ".to_string(),
            schema_version: 1,
            binary: "x".to_string(),
            scopes: vec![],
            version: "0".to_string(),
        };
        assert!(m.validate().is_err());
    }

    #[test]
    fn test_manifest_rejects_future_schema_version() {
        let m = ConnectorManifest {
            kind: "github".to_string(),
            schema_version: u32::MAX,
            binary: "x".to_string(),
            scopes: vec![],
            version: "0".to_string(),
        };
        let err = m.validate().unwrap_err().to_string();
        assert!(
            err.contains("newer than supported"),
            "error should mention 'newer than supported', got: {err}"
        );
    }

    #[test]
    fn test_upsert_connector_is_idempotent_and_reactivates() {
        let db = db();
        let id1 = upsert_connector(&db, "stub", "default", "{}").unwrap();
        // Soft-delete the row (simulating a disconnect).
        db.execute(
            "UPDATE connectors SET state = 'stopped' WHERE id = ?1",
            params![id1],
        )
        .unwrap();
        // Re-registering should reactivate, not insert a duplicate.
        let id2 = upsert_connector(&db, "stub", "default", "{}").unwrap();
        assert_eq!(id1, id2);
        let state: String = db
            .query_row(
                "SELECT state FROM connectors WHERE id = ?1",
                params![id2],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "registered");
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM connectors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "re-registration should not duplicate the row");
    }

    #[test]
    fn test_list_connectors_orders_by_kind_then_instance() {
        let db = db();
        upsert_connector(&db, "github", "b", "{}").unwrap();
        upsert_connector(&db, "stub", "a", "{}").unwrap();
        upsert_connector(&db, "github", "a", "{}").unwrap();
        let rows = list_connectors(&db).unwrap();
        let keys: Vec<(String, String)> = rows.into_iter().map(|r| (r.kind, r.instance)).collect();
        assert_eq!(
            keys,
            vec![
                ("github".to_string(), "a".to_string()),
                ("github".to_string(), "b".to_string()),
                ("stub".to_string(), "a".to_string()),
            ]
        );
    }
}
