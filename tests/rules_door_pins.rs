//! G-3 — the rules door's pins (C1's `the_rules_insert_door_is_missing`).
//!
//! The library law (natural-key revisions, audit-per-insert, the C1 fixture
//! rows as fixtures) lives in `src/service/rules.rs`. This file holds the
//! pin that lesson `daebe7fe` paid for: **a pin that constructs its own
//! input cannot see the door.** The verb's own documented flags must PARSE
//! through the real binary — `VALUE_FLAGS` is a closed vocabulary, and a
//! library round-trip never reads it.

use std::process::Command;

fn run(bin: &str, db: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin)
        .args(args)
        .arg("--db")
        .arg(db)
        .output()
        .expect("the brain binary runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn migrated_db(dir: &std::path::Path) -> std::path::PathBuf {
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let db = dir.join("rules-pin.db");
    let mut conn = rusqlite::Connection::open(&db).expect("open fixture db");
    brain_server::migration::run_migration(&mut conn, 512).expect("migrate fixture db");
    db
}

/// C1's fixture rows, verbatim in data (plans repo
/// `c1-support-fixture/sql/rules-support.sql`) — the register sentence
/// honored: they are the door's first regression fixtures, landed through
/// the DOOR (the real binary), not by the direct SQL C1 had to settle for.
const C1_ROWS: [&str; 3] = ["ticket_retention", "breach_posture", "dsar_clock"];

fn insert_args(rule_key: &str, revision: &str) -> Vec<String> {
    vec![
        "rules".into(),
        "insert".into(),
        "--jurisdiction".into(),
        "ph".into(),
        "--subject".into(),
        "support-tickets".into(),
        "--rule-key".into(),
        rule_key.into(),
        "--body".into(),
        format!("the {rule_key} rule body (fixture data, see rules-support.sql)"),
        "--source-ref".into(),
        "c1-support-fixture/packs/support-escalation-pack.json".into(),
        "--revision".into(),
        revision.into(),
    ]
}

#[test]
fn the_c1_fixture_rows_land_through_the_real_binary_and_the_replay_refuses() {
    let bin = env!("CARGO_BIN_EXE_brain");
    let dir = std::env::temp_dir().join(format!("brain-rules-door-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let db = migrated_db(&dir);

    for key in C1_ROWS {
        let args = insert_args(key, "1");
        let argrefs: Vec<&str> = args.iter().map(String::as_str).collect();
        let (code, stdout, stderr) = run(bin, &db, &argrefs);
        assert_eq!(code, 0, "the fixture row {key} lands: {stderr}");
        assert!(stdout.contains("landed as row"), "{stdout}");
    }

    // the door's list carries all three
    let (code, stdout, _stderr) = run(bin, &db, &["rules", "list"]);
    assert_eq!(code, 0);
    for key in C1_ROWS {
        assert!(stdout.contains(key), "the list shows {key}: {stdout}");
    }

    // a replay of the same revision REFUSES — the C1 fixture's idempotency
    // law, enforced by the door instead of the operator's WHERE NOT EXISTS
    let args = insert_args("ticket_retention", "1");
    let argrefs: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, _stdout, stderr) = run(bin, &db, &argrefs);
    assert_eq!(code, 1, "a replay is a runtime refusal, not a usage error");
    assert!(
        stderr.contains("already present at revision 1"),
        "the refusal names the law: {stderr}"
    );

    // and the three acts are on the chain, kind-attributable
    let conn = rusqlite::Connection::open(&db).unwrap();
    let audited: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM audit_events WHERE kind='rules'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(audited, 3, "one audit row per insert act");
}

#[test]
fn a_missing_documented_flag_is_a_usage_error_naming_the_vocabulary() {
    // the daebe7fe lesson, held as a pin: if `--subject` ever leaves the
    // VALUE_FLAGS vocabulary again, THIS is the red that names it — the
    // binary's own refusal, not a library call that never parses.
    let bin = env!("CARGO_BIN_EXE_brain");
    let dir = std::env::temp_dir().join(format!("brain-rules-vocab-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let db = migrated_db(&dir);

    let out = Command::new(bin)
        .args(["rules", "insert", "--jurisdiction", "ph"])
        .arg("--db")
        .arg(&db)
        .output()
        .expect("the brain binary runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(2),
        "missing required flags are usage errors"
    );
    assert!(stderr.contains("--subject is required"), "{stderr}");
}
