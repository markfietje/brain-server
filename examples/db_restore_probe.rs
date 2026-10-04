//! Build a REAL migrated database in-process and probe the R58b join.
//! Read-only with respect to the operator's live store: nothing here opens
//! ~/.openclaw.
use rusqlite::Connection;

fn main() {
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mut db = Connection::open_in_memory().expect("open");
    brain_server::migration::run_migration(&mut db, 512).expect("migration");

    let cols = |t: &str| -> Vec<String> {
        let mut s = db
            .prepare(&format!("SELECT name FROM pragma_table_info('{t}')"))
            .expect("pragma");
        s.query_map([], |r| r.get::<_, String>(0))
            .expect("rows")
            .map(|x| x.expect("col"))
            .collect()
    };
    let dt = cols("delivery_traces");
    println!(
        "delivery_traces.model_registry_id present: {}",
        dt.iter().any(|c| c == "model_registry_id")
    );
    println!(
        "delivery_traces.model_registry_version present: {}",
        dt.iter().any(|c| c == "model_registry_version")
    );

    // THE join R58b needs: trace -> registry with no translation layer.
    let sql = "SELECT COUNT(*) FROM delivery_traces t \
               JOIN decision_model_registry r \
                 ON r.id = t.model_registry_id AND r.version = t.model_registry_version";
    match db.query_row(sql, [], |r| r.get::<_, i64>(0)) {
        Ok(n) => println!("R58b join EXPRESSES (rows on an empty DB: {n})"),
        Err(e) => println!("R58b join does NOT express: {e}"),
    }
    let stamp: String = db
        .query_row(
            "SELECT value FROM schema_meta WHERE key='schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| format!("<{e}>"));
    println!("schema_version: {stamp}");
}
