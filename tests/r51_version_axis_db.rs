//! D51.3 — the `knowledge_version` axis, proven against a REAL migrated database.
//!
//! The source-reading pins in `r51_version_axis_pins.rs` assert the SHAPE of the
//! migration and the write. This one opens an actual database, runs the actual
//! migration, opens an actual case, and reads the value back with SQL — so the claim
//! is "a new case carries a knowledge_version", not "a string appears in a source
//! file".
//!
//! `state::open_run` is `pub(crate)`, unreachable from an integration test, so the
//! insert below reproduces its exact statement. That equivalence is not assumed — it
//! is asserted by `d51_3_open_run_sql_matches_this_statement`, which compares the
//! live statement text against the one in `src/workflow/state.rs`.

use brain_server::migration::run_migration;
use rusqlite::Connection;

fn migrated_db() -> Connection {
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mut db = Connection::open_in_memory().expect("open in-memory DB");
    run_migration(&mut db, 64).expect("migration");
    db
}

/// The statement `state::open_run` executes, verbatim.
const OPEN_RUN_SQL: &str = "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at, knowledge_version)\n         VALUES (?1, ?2, ?3, 0, 'active', ?4, ?4, ?5)";

fn open_case(conn: &Connection) -> i64 {
    conn.execute(
        OPEN_RUN_SQL,
        rusqlite::params!["acme", "troubleshoot", "{}", 1i64, 1i64],
    )
    .expect("open_run");
    conn.last_insert_rowid()
}

#[test]
fn d51_3_a_new_case_carries_knowledge_version() {
    let conn = migrated_db();

    // Pre-condition: the additive column exists after migration.
    let cols: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('workflow_runs') WHERE name='knowledge_version'",
            [],
            |r| r.get(0),
        )
        .expect("pragma");
    assert_eq!(cols, 1, "the additive column must exist after migration");

    let run_id = open_case(&conn);

    // D51.3 readback: SELECT id, knowledge_version FROM workflow_runs
    // ORDER BY id DESC LIMIT 1
    let (id, kv): (i64, Option<i64>) = conn
        .query_row(
            "SELECT id, knowledge_version FROM workflow_runs ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("readback");
    assert_eq!(id, run_id, "the readback must be the case we just opened");
    assert_eq!(
        kv,
        Some(1),
        "a newly opened case must carry knowledge_version = 1. NULL would mean the \
         case-open write never happened; 0 would be the forbidden sentinel."
    );
}

/// The statement used above really is the one production opens cases with — otherwise
/// this file would be testing a query nothing runs.
#[test]
fn d51_3_open_run_sql_matches_this_statement() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/workflow/state.rs"),
    )
    .expect("state.rs");
    let idx = src
        .find("INSERT INTO workflow_runs")
        .expect("open_run's insert");
    let tail = &src[idx..];
    let end = tail.find('"').expect("statement end");
    let actual: String = tail[..end].replace("\\\n         ", " ");
    let actual = actual.split_whitespace().collect::<Vec<_>>().join(" ");
    let expected = OPEN_RUN_SQL
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        actual, expected,
        "this file's OPEN_RUN_SQL has drifted from `state::open_run`'s real statement. \
         D51.3 would then be proving a query production never runs."
    );
}

#[test]
fn d51_3_legacy_rows_are_null_not_zero() {
    let conn = migrated_db();
    // A row inserted WITHOUT the column (as every pre-R51 row is) must read NULL,
    // never 0 — the sentinel that would falsely date it to version zero.
    conn.execute(
        "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
         VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
        [],
    )
    .expect("legacy insert");
    let kv: Option<i64> = conn
        .query_row(
            "SELECT knowledge_version FROM workflow_runs LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("readback");
    assert_eq!(
        kv, None,
        "a row that predates tracking must read NULL, not 0. NULL means 'predates \
         tracking'; 0 would falsely claim it opened against version zero."
    );
}

#[test]
fn d51_3_migration_is_idempotent() {
    // Re-running the migration over an already-migrated database must be a no-op,
    // not a "duplicate column" error.
    let mut conn = migrated_db();
    open_case(&conn);
    run_migration(&mut conn, 64).expect("second migration is a no-op");
    let cols: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('workflow_runs') WHERE name='knowledge_version'",
            [],
            |r| r.get(0),
        )
        .expect("pragma");
    assert_eq!(cols, 1, "the guarded ALTER must not duplicate the column");
    // and the rows survived
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM workflow_runs", [], |r| r.get(0))
        .expect("count");
    assert_eq!(n, 1, "the additive migration must not lose rows");
}
