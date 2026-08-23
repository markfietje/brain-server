use super::state::{CasError, cas_update};
use rusqlite::{Connection, params};
use serde_json::Value;

// The state-key routing contract moved home to the SDK: the four keys are
// the engine ABI, so `Decision`/`decide` live in the SDK
// (`brain_engine_sdk::workflow_state`) and the server re-exports them.
// `load_state`/`advance` stay here — they are rusqlite-bound, not ABI.
#[allow(unused_imports)] // consumed by this module's #[cfg(test)] pins
pub use brain_engine_sdk::workflow_state::{Decision, decide};

pub fn load_state(conn: &Connection, run_id: i64) -> Option<(Value, i64)> {
    let (js, rev): (String, i64) = conn
        .query_row(
            "SELECT state_json, state_revision FROM workflow_runs WHERE id=?1",
            params![run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    match serde_json::from_str::<Value>(&js) {
        Ok(v) => Some((v, rev)),
        // Corrupt state is integrity-visible: the driver refuses to treat it
        // as a terminal run (the old `unwrap_or(Null)` fell through to
        // `Decision::Done`), so a poisoned row surfaces instead of silently
        // closing the run.
        Err(e) => {
            tracing::warn!("run {run_id}: corrupt state_json refused: {e}");
            None
        }
    }
}

pub fn advance(
    conn: &Connection,
    run_id: i64,
    expected_rev: i64,
    next_state: &str,
    now: i64,
) -> Result<(), CasError> {
    let v: Value =
        serde_json::from_str(next_state).unwrap_or(Value::String(next_state.to_string()));
    let s = v.to_string();
    cas_update(conn, run_id, expected_rev, &s, "active", now).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;
    use rusqlite::Connection;
    use serde_json::json;

    fn seed_db(js: &str) -> Connection {
        register_sqlite_vec();
        let mut c = Connection::open_in_memory().unwrap();
        run_migration(&mut c, 1).unwrap();
        c.execute(
            "INSERT INTO workflow_runs(domain,kind,state_json,state_revision,status,created_at,updated_at) VALUES ('global','interview',?1,0,'active',1,1)",
            params![js],
        ).unwrap();
        c
    }

    #[test]
    fn corrupt_state_refused_not_done() {
        let conn = seed_db("{not json");
        // P3-9: a poisoned state_json used to fall through to Decision::Done.
        assert!(load_state(&conn, 1).is_none(), "corrupt state refused");
    }

    #[test]
    fn decision_table_routes() {
        assert!(matches!(decide(&json!({"status":"done"})), Decision::Done));
        assert!(matches!(
            decide(&json!({"pending_question":"hi"})),
            Decision::AskHuman { .. }
        ));
        assert!(matches!(
            decide(&json!({"next_step":"inventory"})),
            Decision::RunStep { .. }
        ));
        assert!(matches!(
            decide(&json!({"next_state":"{}"})),
            Decision::Advance { .. }
        ));
    }

    #[test]
    fn orchestration_replays_after_crash_exactly_once() {
        let conn = seed_db(r#"{"next_state":"{\"status\":\"active\"}"}"#);
        let (state, rev) = load_state(&conn, 1).unwrap();
        let d = decide(&state);
        assert!(matches!(d, Decision::Advance { .. }));
        if let Decision::Advance { next_state } = d {
            advance(&conn, 1, rev, &next_state, 2).unwrap();
        }
        // replay: revision now 1, stale advance fails
        assert!(matches!(
            advance(&conn, 1, 0, "{}", 3),
            Err(CasError::Stale { .. })
        ));
        let (s2, r2) = load_state(&conn, 1).unwrap();
        assert_eq!(r2, 1);
        assert_eq!(s2["status"], "active");
    }
}
