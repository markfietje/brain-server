use super::state::cas_update;
use rusqlite::Connection;

pub fn persist_stage(
    conn: &Connection,
    run_id: i64,
    stage_json: &str,
    rev: i64,
    now: i64,
) -> Result<(), String> {
    cas_update(conn, run_id, rev, stage_json, "active", now)
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}

pub fn set_pending_approval(
    conn: &Connection,
    run_id: i64,
    rev: i64,
    now: i64,
) -> Result<(), String> {
    conn.execute("INSERT INTO workflow_steps(run_id, kind, status, content_json, created_at) VALUES (?1,'stage','pending_approval','{}',?2)", rusqlite::params![run_id, now]).map_err(|e| e.to_string())?;
    super::audit_write(
        conn,
        run_id,
        &format!("run:{run_id}"),
        crate::audit::AuditStatus::Ok,
        "pending_approval",
    );
    cas_update(conn, run_id, rev, "{}", "pending_approval", now)
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}
