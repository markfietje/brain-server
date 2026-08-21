use rusqlite::Connection;
pub(crate) fn record_goal_step(
    conn: &Connection,
    run_id: i64,
    content: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO workflow_steps(run_id, kind, content) VALUES (?1,'goal-state',?2)",
        rusqlite::params![run_id, content],
    )?;
    crate::workflow::audit_write(
        conn,
        run_id,
        &format!("workflow_steps/{run_id}"),
        crate::audit::AuditStatus::Ok,
        "goal-state",
    );
    Ok(())
}
