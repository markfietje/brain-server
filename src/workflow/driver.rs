use super::state::{CasError, cas_update};
use rusqlite::{Connection, params};
use serde_json::Value;

#[derive(Debug, PartialEq, Clone)]
pub enum Decision {
    AskHuman { question: String },
    RunStep { step: String },
    Advance { next_state: String },
    Done,
}

pub fn decide(state: &Value) -> Decision {
    let status = state
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("active");
    if status == "done" || status == "complete" {
        return Decision::Done;
    }
    if let Some(q) = state.get("pending_question").and_then(|v| v.as_str()) {
        return Decision::AskHuman {
            question: q.to_string(),
        };
    }
    if let Some(s) = state.get("next_step").and_then(|v| v.as_str()) {
        return Decision::RunStep {
            step: s.to_string(),
        };
    }
    if let Some(n) = state.get("next_state").and_then(|v| v.as_str()) {
        return Decision::Advance {
            next_state: n.to_string(),
        };
    }
    Decision::Done
}

pub fn load_state(conn: &Connection, run_id: i64) -> Option<(Value, i64)> {
    let (js, rev): (String, i64) = conn
        .query_row(
            "SELECT state_json, state_revision FROM workflow_runs WHERE id=?1",
            params![run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    Some((serde_json::from_str(&js).unwrap_or(Value::Null), rev))
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
