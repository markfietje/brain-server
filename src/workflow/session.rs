//! The server's one `SessionSource`/`SessionSanitizer` implementation pair:
//! governed-workflow state reads through the SDK session seam return the
//! `sanitize_read` view — PII redact + invisible-strip + markdown-ref strip —
//! so raw state_json never leaves the host on a session read.

use brain_engine_sdk::session::{SanitizedSession, SessionSanitizer, SessionSource};
use rusqlite::Connection;

/// Reads workflow-run state rows by run id (the session key space).
pub(crate) struct RunStateSource<'a> {
    pub conn: &'a Connection,
}

impl SessionSource for RunStateSource<'_> {
    fn read_raw(&self, key: &str) -> Result<String, String> {
        let run_id: i64 = key.parse().map_err(|_| "invalid run key".to_string())?;
        self.conn
            .query_row(
                "SELECT state_json FROM workflow_runs WHERE id = ?1",
                rusqlite::params![run_id],
                |r| r.get::<_, String>(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => "run not found".to_string(),
                other => other.to_string(),
            })
    }
}

/// The production sanitizer: the same read-canonical form every API read seam
/// applies. PII stays masked for non-admin readers by construction of the
/// caller-supplied principal; here the extension-facing view is always the
/// most-protected form (`pii: false` principal-less redaction).
pub(crate) struct ReadViewSanitizer;

impl SessionSanitizer for ReadViewSanitizer {
    fn sanitize_view(&self, raw: &str) -> String {
        // A synthetic least-privileged principal: extensions never inherit a
        // caller's PII-read power (admin/loopback bypass), so masking is
        // unconditional on this seam.
        let unprivileged = Some(crate::auth::Principal {
            sub: "workflow-extension".into(),
            tenant: "global".into(),
            scopes: Vec::new(),
            jti: String::new(),
            roles: Vec::new(),
            manages: Vec::new(),
        });
        crate::gate::sanitize_read(raw, true, &unprivileged)
    }
}

/// Build the one sanitized handle extensions use.
pub(crate) fn session_for(
    conn: &Connection,
) -> SanitizedSession<RunStateSource<'_>, ReadViewSanitizer> {
    SanitizedSession::new(RunStateSource { conn }, ReadViewSanitizer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;

    #[test]
    fn session_reads_are_sanitized_never_raw() {
        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = r2d2_sqlite::SqliteConnectionManager::file(tmp.path());
        let pool = r2d2::Pool::builder().max_size(1).build(mgr).unwrap();
        let mut conn = pool.get().unwrap();
        run_migration(&mut conn, config::DB_MMAP_SIZE_MIB).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, status, created_at, updated_at)
             VALUES ('acme', 'interview', '{\"note\":\"reach jane@example.com\"}', 'active', 1, 1)",
            [],
        )
        .unwrap();
        let run_id = conn.last_insert_rowid();

        let session = session_for(&conn);
        let view = session.read(&run_id.to_string()).unwrap();
        assert!(
            !view.contains("jane@example.com"),
            "raw PII must not cross the session seam"
        );
        assert!(view.contains("[redacted:"), "masked placeholder present");

        // Unknown and malformed keys fail loudly.
        assert_eq!(session.read("99999").unwrap_err(), "run not found");
        assert_eq!(session.read("not-a-number").unwrap_err(), "invalid run key");
    }
}
