use crate::workflow::{outbox, state::cas_update};
use rusqlite::Connection;

pub fn persist_state(
    conn: &Connection,
    run_id: i64,
    json: &str,
    expected_rev: u64,
) -> Result<(), String> {
    cas_update(conn, run_id, expected_rev as i64, json, "active", 0)
        .map(|_| ())
        .map_err(|e| format!("{:?}", e))?;
    let _ = outbox::enqueue(
        conn,
        run_id,
        "interview_step",
        json,
        &format!("run-{run_id}-rev-{expected_rev}"),
        0,
    );
    Ok(())
}
