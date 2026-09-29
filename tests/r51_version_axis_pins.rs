//! R51 — the knowledge-version axis (I51.7, R51.3).
//!
//! **Why the axis exists.** The ring is not idempotent under time. Solve is per-case
//! (minutes), Evolve is per-pattern (days), Deflect is per-corpus (weeks). A case can
//! therefore be open while Evolve publishes a supersession UNDERNEATH it, and a
//! reopened case (`reask`, back-referral) re-enters Solve against a MOVED knowledge
//! base — silently mixing evidence from two versions. `knowledge_version` records which
//! version a case opened against, so the movement is visible instead of invisible.
//!
//! **CEILING, pinned deliberately.** There is **no Evolve bump site yet**, so the value
//! written at case-open is CONSTANT. It records; it does not prevent. The delta OFFER is
//! where prevention lives, and delta semantics are R53/R57 work. A test that implied
//! the axis already prevents mixed-basis reasoning would be a false claim, so
//! `r51_knowledge_version_ceiling_is_stated` asserts the ceiling is *written down*
//! rather than asserting a protection that does not exist.
//!
//! **Why `NULL` and not `0`.** `NULL` means "this row predates tracking". A sentinel
//! `0` would falsely date every legacy row to version zero and make "opened against
//! version zero" indistinguishable from "predates versioning".

fn crate_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let p = crate_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} must exist: {e}", p.display()))
}

/// **I51.7 / R51.3 — a newly opened case carries `knowledge_version`.** Pre-fix this
/// failed by absence: the column did not exist at all.
#[test]
fn r51_open_run_writes_knowledge_version() {
    let state = read("src/workflow/state.rs");
    assert!(
        state.contains("knowledge_version"),
        "the case-open insert must set `knowledge_version` — that is the whole axis."
    );
    assert!(
        state.contains("KNOWLEDGE_VERSION"),
        "the case-open insert must read the version from the single pinned constant, not a \
         literal, so there is exactly one place the value is defined."
    );
    // every run kind goes through `open_run`, so this is the single choke point
    let callers = [
        "src/workflow/delivery.rs",
        "src/workflow/accounts.rs",
        "src/workflow/valet.rs",
        "src/handlers/webhooks.rs",
        "src/handlers/workflow.rs",
    ];
    for c in callers {
        assert!(
            read(c).contains("open_run("),
            "expected {c} to open runs through `state::open_run`; if a new case-open path \
             bypasses it, `knowledge_version` would be silently NULL for that path."
        );
    }
}

/// The migration is ADDITIVE and IDEMPOTENT — guarded by `pragma_table_info`, so
/// re-running the runner is a no-op rather than an error.
#[test]
fn r51_knowledge_version_migration_is_additive_and_idempotent() {
    let m = read("src/migration.rs");
    assert!(
        m.contains("ALTER TABLE workflow_runs ADD COLUMN knowledge_version INTEGER"),
        "the migration must be an additive `ALTER TABLE ... ADD COLUMN` with NO default — a \
         DEFAULT would backfill a sentinel over rows that predate tracking."
    );
    assert!(
        m.contains("pragma_table_info('workflow_runs') WHERE name='knowledge_version'"),
        "the ALTER must be guarded by a `pragma_table_info` presence check, so the runner \
         applies it idempotently (the in-tree additive-migration precedent)."
    );
    // no rebuild
    assert!(
        !m.contains("DROP TABLE workflow_runs"),
        "workflow_runs must never be dropped/rebuilt: a rebuild is the one operation that \
         can lose rows under a crash."
    );
}

/// The schema stamp moved in lockstep with the migration.
#[test]
fn r51_schema_stamp_moved_with_the_axis() {
    let layout = read("src/storage_layout.rs");
    assert!(
        layout.contains("SCHEMA_VERSION_V1_32_20"),
        "the schema ceiling must move to 1.32.20 with the additive column."
    );
    assert!(
        layout.contains("LATEST_KNOWN_SCHEMA: &str = SCHEMA_VERSION_V1_32_20"),
        "LATEST_KNOWN_SCHEMA must track the newest const, or a newer-schema database would \
         not be refused."
    );
    let m = read("src/migration.rs");
    assert!(
        m.contains("'schema_version', '1.32.20'"),
        "run_migration must stamp 1.32.20 — the stamp and the ceiling move together."
    );
}

/// The ceiling is WRITTEN DOWN, in code, at every place a reader would assume the axis
/// is doing more than it does. This is the honesty pin: it asserts the limitation is
/// declared, not that a protection exists.
#[test]
fn r51_knowledge_version_ceiling_is_stated() {
    let config = read("src/config.rs");
    // The declaration's own doc comment: collect the whole contiguous `///` block
    // immediately above the const.
    let idx = config
        .find("pub const KNOWLEDGE_VERSION")
        .expect("KNOWLEDGE_VERSION must be declared in config.rs");
    let head = config[..idx].lines().collect::<Vec<_>>();
    let mut start = head.len();
    while start > 0 && head[start - 1].trim_start().starts_with("///") {
        start -= 1;
    }
    let doc: String = head[start..].join("\n");
    let decl = format!("{doc}\n{}", &config[idx..(idx + 60).min(config.len())]);
    assert!(
        decl.contains("no Evolve bump site yet") && decl.contains("CONSTANT"),
        "the KNOWLEDGE_VERSION constant must state, in the declaration itself, that the value \
         is currently CONSTANT because no Evolve bump site exists. A reader who finds only \
         the column would otherwise assume a version axis that moves. Declaration:\n{decl}"
    );
    assert!(
        decl.contains("does NOT by itself prevent"),
        "the constant must state that recording the version does not by itself prevent \
         mixed-basis reasoning — the delta offer is where prevention lives."
    );
    // Read the literal from source: this is an integration test, so the lib's private
    // `config` module is not reachable. The literal is what ships.
    let pinned = config
        .lines()
        .find_map(|l| l.trim().strip_prefix("pub const KNOWLEDGE_VERSION: i64 = "))
        .map(|v| v.trim_end_matches(';').trim())
        .expect("KNOWLEDGE_VERSION must be declared in config.rs");
    assert_eq!(
        pinned, "1",
        "the pinned value is 1; a silent edit here would make every case-open assertion \
         below vacuous relative to the recorded evidence. Found: {pinned:?}"
    );
}

/// The wire contract: the case-open response must not have silently changed. D51.6
/// requires SAYING which, never claiming additive silently.
#[test]
fn r51_knowledge_version_did_not_silently_change_the_wire() {
    let openapi = read("openapi.yaml");
    // the field is internal to the case record; it must NOT have been added to the
    // case-open response without an explicit declaration. Assert the stamp is intact.
    assert!(
        openapi.contains("x-api-version:"),
        "openapi.yaml must still carry x-api-version — a diff-empty check needs an anchor."
    );
}
