use crate::workflow::{outbox, state::cas_update};
use rusqlite::Connection;

/// Persist an interview state transition (CAS) + its outbox event. Both writes
/// must succeed or the whole persist fails — a silently-dropped event would
/// break exactly-once downstream delivery. `now` stamps the row's updated_at.
pub fn persist_state(
    conn: &Connection,
    run_id: i64,
    json: &str,
    expected_rev: u64,
    now: i64,
) -> Result<(), String> {
    cas_update(conn, run_id, expected_rev as i64, json, "active", now)
        .map(|_| ())
        .map_err(|e| format!("{:?}", e))?;
    outbox::enqueue(
        conn,
        run_id,
        "interview_step",
        json,
        &format!("run-{run_id}-rev-{expected_rev}"),
        now,
    )
    .map(|_| ())
    .map_err(|e| format!("interview outbox enqueue failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_server::migration::run_migration;

    fn db() -> Connection {
        crate::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'interview', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn persist_stamps_now_and_propagates_enqueue() {
        let conn = db();
        persist_state(&conn, 1, r#"{"q":1}"#, 0, 4242).unwrap();
        let updated_at: i64 = conn
            .query_row("SELECT updated_at FROM workflow_runs WHERE id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(updated_at, 4242, "caller-supplied clock, never 0");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM outbox WHERE run_id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1, "the event landed");
    }
}
