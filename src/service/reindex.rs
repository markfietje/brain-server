//! Reindex candidate selection — the ONE query feeding every re-embed
//! loop (`POST /reindex`, the profile-switch path in `bootstrap`).
//! Quarantined rows are excluded: they hold no vector by the ingest gate,
//! and re-embedding them would resurrect the shadowing the gate removed
//! (re-approval regenerates through the edit path).

use rusqlite::Connection;

/// Ids + content eligible for (re-)embedding, in row-id order.
pub fn reindex_candidate_ids(conn: &Connection) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut stmt =
        conn.prepare("SELECT id, content FROM knowledge WHERE flagged = 0 ORDER BY id")?;
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flagged_db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE knowledge(id INTEGER PRIMARY KEY, content TEXT, flagged INTEGER NOT NULL DEFAULT 0);",
        )
        .unwrap();
        c.execute(
            "INSERT INTO knowledge(id, content, flagged) VALUES (1, 'clean', 0), (2, 'plant', 1)",
            [],
        )
        .unwrap();
        c
    }

    #[test]
    fn reindex_candidates_exclude_quarantined() {
        let ids = reindex_candidate_ids(&flagged_db()).unwrap();
        assert_eq!(ids, vec![(1, "clean".to_string())]);
    }
}
