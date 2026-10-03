// The disproof sweep's pins, asserted from OUTSIDE the module.
//
// ## The claim this file exists to make
//
// `evaluate_disproof` shipped complete and **unreachable**: every call in the
// tree was a test. A claim could carry a disproof condition and nothing would
// ever read it back against the text it was written about — so the loop's
// falsification half existed on paper.
//
// `brain disproof` is the caller. These pins hold two properties that must not
// trade away to become useful: that it is genuinely reachable, and that it
// reports what it found honestly.
//
// ## Why the reachability pin is shaped the way it is
//
// R9's own pin was wrong exactly once: it searched for a call site and found
// an **orphan** — `fn cmd_x` complete with its call, but no longer named in the
// dispatch table, so no user could ever invoke it. A "does the string appear"
// walk passes on that every time.
//
// So this one requires the call to sit inside a function the `SUBCOMMANDS` table
// actually dispatches. Deleting the dispatch entry must fail it; that is the
// red-proof, and it is the same plant that caught R9's.

mod common;

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The functions the `brain` binary's dispatch table can invoke.
///
/// Parsed from the table itself rather than a restatement of it, so a command
/// that stops being dispatchable stops counting as reachable.
fn dispatched_runners() -> Vec<String> {
    let src = common::code_only(&read("src/bin/brain.rs"));
    let table = src
        .split("const SUBCOMMANDS")
        .nth(1)
        .expect("SUBCOMMANDS table missing from brain.rs — the dispatch law is broken");
    let table = &table[..table.find("\n];").expect("SUBCOMMANDS table never closes")];
    table
        .lines()
        .filter_map(|line| {
            let idx = line.find("run: ")?;
            let rest = &line[idx + "run: ".len()..];
            let end = rest.find(',')?;
            Some(rest[..end].to_string())
        })
        .collect()
}

/// The sweep has a REACHABLE production caller.
///
/// Not "a call site exists" — a call site inside a function the binary's
/// dispatch table names. An orphan satisfies the first and fails this.
#[test]
fn the_disproof_sweep_has_a_reachable_production_caller() {
    let dispatched = dispatched_runners();
    assert!(
        dispatched.len() >= 40,
        "the dispatch table yielded {} run targets — the extractor is broken and this pin \
         would pass or fail for the wrong reason",
        dispatched.len()
    );
    assert!(
        dispatched.iter().any(|f| f == "cmd_disproof"),
        "no dispatch entry runs cmd_disproof — `brain disproof` is unreachable, and the \
         disproof core is unreachable again behind it. Call sites in the file do not count: \
         an undispatched fn is dead code that happens to compile."
    );

    // And the call is inside THAT function, not merely somewhere in the file.
    let src = common::code_only(&read("src/bin/brain.rs"));
    let start = src
        .find("fn cmd_disproof")
        .expect("cmd_disproof must exist in brain.rs");
    let body = &src[start..];
    let end = body
        .find("\nfn ")
        .expect("cmd_disproof must close before the next top-level fn");
    let body = &body[..end];
    assert!(
        body.contains("sweep_disproofs"),
        "cmd_disproof is dispatched but does not call the sweep — the dispatch entry is a \
         facade over something else, and the disproof core is still unreachable"
    );
}

/// The sweep WRITES NOTHING.
///
/// The posture is what lets this ship while promotion stays a compile-time
/// `false`: a sweep that demoted or ratified its own findings would be a
/// promotion path. Asserted against the source rather than promised, because
/// "it only reads" is exactly the kind of claim that decays.
#[test]
fn the_sweep_writes_nothing() {
    let src = common::code_only(&read("src/service/create.rs"));
    let start = src
        .find("pub fn sweep_disproofs")
        .expect("sweep_disproofs must exist");
    // The public entry plus the inner it delegates to.
    let slice = &src[start..];
    let end = slice.find("\n/// ").unwrap_or(slice.len().min(4000));
    let body = &slice[..end];
    let upper = body.to_ascii_uppercase();
    for verb in [
        "UPDATE ",
        "INSERT INTO",
        "DELETE FROM",
        "CREATE TABLE",
        "DROP ",
    ] {
        assert!(
            !upper.contains(verb),
            "the disproof sweep contains `{verb}` — it is supposed to be a READ. A sweep that \
             writes is a promotion path, and promotion is a compile-time false."
        );
    }
}

/// All three verdict states survive into the receipt.
///
/// **This is the load-bearing pin.** A sweep that counted `NoVerdict` as a pass
/// would be green, quiet, and wrong — and every other pin here would still
/// pass. The polarity is asserted in both directions because
/// `DisproofVerdict`'s own doc names it as the easy thing to invert.
#[test]
fn the_three_verdict_states_are_distinct_and_never_collapsed() {
    use brain_server::service::create::{DisproofVerdict, sweep_disproofs};

    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mut db = rusqlite::Connection::open_in_memory().expect("open");
    brain_server::migration::run_migration(&mut db, 512).expect("migration");

    db.execute(
        "INSERT INTO claim_schemas(id, domain, version, authored_by, body, body_digest, created_at)
         VALUES (1, 'global', 1, 'human', '{}', ?, 0)",
        ["d".repeat(64)],
    )
    .expect("schema row");

    let plant = |id: &str, subject: &str, form: &str, citation: Option<&str>| {
        let audit = if form == "audited" {
            Some("aud-1")
        } else {
            None
        };
        db.execute(
            "INSERT INTO claims(claim_id, schema_ref, subject, predicate, object,
                                evidence_digest, audit_target_hash, created_by, created_at,
                                disproof_form, disproof_body, disproof_op, disproof_citation,
                                disproof_scope, disproof_coverage, disproof_audit_ref)
             VALUES (?1, 1, ?2, 'p', 'o', ?3, ?4, 'human', 0, ?5, 'body', 'contains', ?6,
                     'global', ?7, ?8)",
            rusqlite::params![
                id,
                subject,
                "e".repeat(64),
                "a".repeat(64),
                form,
                citation,
                r#"["claims.subject"]"#,
                audit
            ],
        )
        .expect("plant");
    };

    // The disproof citation IS present in the subject → the claim does NOT stand.
    plant(
        "clm_r",
        "the payer is payer-verified",
        "evaluated",
        Some("payer-verified"),
    );
    // The citation is ABSENT → the disproof was not observed → the claim stands.
    plant(
        "clm_s",
        "the payer is self-serve",
        "evaluated",
        Some("never-appears"),
    );
    // A prose condition has no mechanical reading at all.
    plant("clm_p", "anything", "audited", None);

    let readings = sweep_disproofs(&db).expect("sweep");
    assert_eq!(readings.len(), 3, "every planted claim must be read");

    let by_id = |id: &str| {
        readings
            .iter()
            .find(|r| r.claim_id == id)
            .unwrap_or_else(|| panic!("{id} missing from the sweep"))
            .verdict
            .clone()
    };

    // Polarity, asserted in BOTH directions. Naming what would refute the claim
    // and then observing it means the claim is refuted — not fine.
    assert_eq!(by_id("clm_r"), DisproofVerdict::Refuted);
    assert_eq!(by_id("clm_s"), DisproofVerdict::Satisfied);

    // A prose condition is NEVER green, and is not the same value as either.
    let prose = by_id("clm_p");
    assert!(
        matches!(prose, DisproofVerdict::NoVerdict { .. }),
        "a prose condition must report NoVerdict, got {prose:?}"
    );
    assert!(!prose.is_green(), "a prose condition reported GREEN");
    assert!(!prose.is_refuted(), "a prose condition reported refuted");

    // And the counts partition the read — no claim is in two buckets, none in
    // none. A receipt that double-counts or drops a state is the exact collapse
    // this pin exists to prevent.
    let satisfied = readings.iter().filter(|r| r.verdict.is_satisfied()).count();
    let refuted = readings.iter().filter(|r| r.verdict.is_refuted()).count();
    let no_verdict = readings.len() - satisfied - refuted;
    assert_eq!((satisfied, refuted, no_verdict), (1, 1, 1));
}

/// The verb reports all three states, through the REAL binary.
///
/// A source-reading pin cannot tell whether a receipt collapses `NoVerdict` into
/// a pass — only running it can. This is the same lesson the routing seam taught
/// at cost: *reachable* is not *working*.
#[test]
fn the_verb_reports_the_three_states_on_a_real_database() {
    let bin = env!("CARGO_BIN_EXE_brain");
    let dir = std::env::temp_dir().join(format!("brain-disproof-pin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let db = dir.join("brain.db");

    // Build a real migrated DB with one claim of each state.
    {
        brain_server::register_sqlite_vec::register_sqlite_vec();
        let mut c = rusqlite::Connection::open(&db).expect("open");
        brain_server::migration::run_migration(&mut c, 512).expect("migration");
        c.execute(
            "INSERT INTO claim_schemas(id, domain, version, authored_by, body, body_digest, created_at)
             VALUES (1, 'global', 1, 'human', '{}', ?, 0)",
            ["d".repeat(64)],
        )
        .expect("schema row");
        let plant = |id: &str, subject: &str, form: &str, citation: Option<&str>| {
            let audit = if form == "audited" {
                Some("aud-1")
            } else {
                None
            };
            c.execute(
                "INSERT INTO claims(claim_id, schema_ref, subject, predicate, object,
                                    evidence_digest, audit_target_hash, created_by, created_at,
                                    disproof_form, disproof_body, disproof_op, disproof_citation,
                                    disproof_scope, disproof_coverage, disproof_audit_ref)
                 VALUES (?1, 1, ?2, 'p', 'o', ?3, ?4, 'human', 0, ?5, 'body', 'contains', ?6,
                         'global', ?7, ?8)",
                rusqlite::params![
                    id,
                    subject,
                    "e".repeat(64),
                    "a".repeat(64),
                    form,
                    citation,
                    r#"["claims.subject"]"#,
                    audit
                ],
            )
            .expect("plant");
        };
        plant(
            "clm_r",
            "the payer is payer-verified",
            "evaluated",
            Some("payer-verified"),
        );
        plant(
            "clm_s",
            "the payer is self-serve",
            "evaluated",
            Some("never-appears"),
        );
        plant("clm_p", "anything", "audited", None);
    }

    let out = std::process::Command::new(bin)
        .args(["--json", "disproof", "--db"])
        .arg(&db)
        .output()
        .expect("the brain binary must run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "brain disproof failed: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        stdout
    );
    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("{e}: {stdout}"));
    let d = &v["data"];
    assert_eq!(d["claims_read"], 3, "every claim must be counted: {stdout}");
    assert_eq!(d["satisfied"], 1, "{stdout}");
    assert_eq!(d["refuted"], 1, "{stdout}");
    assert_eq!(
        d["no_verdict"], 1,
        "the prose condition must be reported as its own state, never folded in: {stdout}"
    );
    assert_eq!(
        d["wrote_anything"], false,
        "the receipt must state the read-only posture: {stdout}"
    );

    // The refuted claim is NAMED — a count alone does not tell an operator which
    // claim stopped standing.
    let ids = d["refuted_claim_ids"].as_array().expect("ids array");
    assert_eq!(ids.len(), 1, "{stdout}");
    assert_eq!(
        ids[0], "clm_r",
        "the wrong claim was named refuted: {stdout}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
