//! R48 — the clean-cycle install: the STRUCTURAL, SCOPE and LAW half of the
//! battery, red-first.
//!
//! **Why this file is half a battery, and where the other half is.** A pin that
//! *calls* the new `deploy/` code cannot exist at the RED commit: the unit and
//! scripts do not exist yet, so the file would not COMPILE, and a battery that
//! does not compile is not a red battery. R46 and R47 hit the same wall and split
//! the same way. So:
//!
//!   * **structural / scope / law pins** (this file) read the tree as TEXT, run
//!     against the pre-R48 tree, and are RED here;
//!   * **behavioural pins** — the journal-mode refusal, the boot verification, the
//!     ship verb — land in `#[cfg(test)]` modules inside `src/` with the
//!     implementation, where they call the real code.
//!
//! **RED-first, and loudly.** A pin that reads a file R48 creates panics naming
//! the path — never a silent pass. The pins that assert an ABSENCE are green from
//! this commit and must stay green; that is their job.
//!
//! **Every pin here must be DEMONSTRATED TO BITE before the round is called
//! done.** That is not a platitude: R47 produced a real vacuous pin that passed
//! with its own bug reverted, and R48's own reference install shipped a stop
//! script that reported `clean stop` while the process was still running. A
//! finding recorded as "the pin would catch this" is worth nothing.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_repo(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Read a file R48 CREATES. A missing path is a loud, named failure — the pin
/// cannot pass vacuously on a tree where the round has not landed.
fn read_r48(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "R48 must ship {} — it is absent at this tree ({e}). This pin reads the \
             round's own artifacts as text so the RED battery compiles against a tree \
             that does not have them yet; a silent pass here would be a guard that \
             covers nothing.",
            path.display()
        )
    })
}

fn exists(rel: &str) -> bool {
    repo_root().join(rel).exists()
}

/// The production region: everything before the first test module.
///
/// **Scoped to LINE-LEADING attributes on purpose.** An earlier draft split on
/// the bare substring, and a COMMENT in this very file that quoted a test
/// attribute silently truncated the region to nothing — which turned two pins
/// green-to-red for the wrong reason. The same trap is waiting for any source
/// file whose prose mentions a test attribute, so the boundary is matched where
/// it actually lives: at the start of a line.
fn production_region(src: &str) -> &str {
    let mut cut = src.len();
    for (i, line) in src.split_inclusive('\n').enumerate() {
        let _ = i;
        if line.trim_start().starts_with("#[cfg(test)]") {
            cut = line.as_ptr() as usize - src.as_ptr() as usize;
            break;
        }
    }
    &src[..cut]
}

/// Production minus `//`, `///` and `//!` lines.
fn code_region(src: &str) -> String {
    production_region(src)
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("//") || t.starts_with("#[doc"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn walk_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_rs_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// Walk ALL files, not just `.rs`.
///
/// Separate from `walk_rs_files` because the round's artifacts are a `.service`
/// file and three `.sh` scripts — and the first draft of the non-vacuity pin
/// used the Rust walker over `deploy/`, found **zero files**, and the pin
/// reported it correctly. That is the self-pin working: the census it drives
/// was reading nothing.
fn walk_all_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_all_files(&p, out);
        } else {
            out.push(p);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// the gap (E2) — the round's primary deliverable
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn r48_the_linux_install_artifacts_exist() {
    for f in [
        "deploy/systemd/brain-server.service",
        "deploy/install.sh",
        "deploy/uninstall.sh",
        "deploy/clean-cycle-check.sh",
    ] {
        assert!(
            exists(f),
            "R48 must ship {f} — it is the round's primary deliverable"
        );
    }
}

/// E2: the unit hardens the pod. Each directive is a separate assertion so a
/// failure names the one that is missing.
#[test]
fn r48_the_systemd_unit_hardens_the_service() {
    let unit = read_r48("deploy/systemd/brain-server.service");
    for directive in [
        "NoNewPrivileges=true",
        "PrivateTmp=true",
        "ProtectHome=true",
        "ProtectSystem=strict",
        "ReadWritePaths=",
        "Restart=on-failure",
    ] {
        assert!(
            unit.contains(directive),
            "the unit must set `{directive}` — E2's least-privilege set, mirrored from the \
             measured docker-compose posture"
        );
    }
    assert!(
        !unit.contains("User=root") && !unit.contains("User= root"),
        "the unit must not run as root; it runs as a dedicated unprivileged user"
    );
}

/// E2: `TimeoutStopSec` must be EXPLICIT. The systemd default is 90s, and the
/// whole point of E2 is that the number is chosen from a measurement and stated
/// in the runbook, not inherited.
#[test]
fn r48_the_unit_has_an_explicit_timeout_stop_sec() {
    let unit = read_r48("deploy/systemd/brain-server.service");
    assert!(
        unit.contains("TimeoutStopSec="),
        "the unit must set TimeoutStopSec EXPLICITLY. E2 is that the stop budget is a \
         measured number in the file, not the systemd default nobody chose."
    );
    // and the measured value must be traceable to the runbook
    let runbook = read_r48("docs/clean-cycle.md");
    assert!(
        runbook.contains("TimeoutStopSec"),
        "the runbook must state the measured stop budget — a number in a unit file with no \
         stated derivation is the guess E2 and KILL 4 exist to prevent"
    );
}

/// E5: the stop is SURGICAL. It matches an absolute BINARY path — never a
/// database path, never a port. This is the bug the reference install shipped.
#[test]
fn r48_the_stop_targets_the_absolute_binary_path() {
    let stop = read_r48("deploy/clean-cycle-check.sh");
    let install = read_r48("deploy/install.sh");
    // the stop must not key on a DB path (it lives in the ENVIRONMENT, not argv)
    for f in [stop.as_str(), install.as_str()] {
        assert!(
            !f.contains("pkill -f 'brain-demo/active") && !f.contains("pkill -TERM -f 'brain"),
            "the stop must not match on a database path. BRAIN_DB_PATH lives in the \
             ENVIRONMENT, not in argv, so `pkill -f` on it matches NOTHING and the script \
             reports a clean stop while the process is still running. That is the exact \
             defect the R48 reference install shipped (E5)."
        );
    }
    assert!(
        stop.contains("/home/") || stop.contains("/usr/local/bin") || stop.contains("BIN="),
        "the stop must target the service by an absolute binary path, so it cannot match a \
         sibling install of the same binary name"
    );
}

/// E1: uninstall must NEVER destroy state. A rollback that deletes the database
/// is a data-loss incident wearing a maintenance script.
#[test]
fn r48_uninstall_never_deletes_the_data_directory() {
    let text = read_r48("deploy/uninstall.sh");
    for forbidden in [
        "rm -rf $DATA",
        "rm -rf ${DATA",
        "rm -rf \"$DATA",
        "rm -fr $DATA",
    ] {
        assert!(
            !text.contains(forbidden),
            "uninstall.sh contains `{forbidden}`. The data directory is the customer's; \
             uninstall removes the unit, not the state. A maintenance script that deletes \
             the database is a data-loss incident with a friendly name."
        );
    }
    assert!(
        text.contains("systemctl") || text.contains("disable"),
        "uninstall must at least stop and disable the unit"
    );
}

/// E1: install must REFUSE to clobber an existing store rather than overwrite it.
#[test]
fn r48_install_refuses_to_clobber_existing_data() {
    let text = read_r48("deploy/install.sh");
    assert!(
        text.contains("-e")
            || text.contains("refus")
            || text.contains("already exists")
            || text.contains("EXIT"),
        "install.sh must refuse to overwrite an existing data directory. A silent \
         overwrite of a customer's database during an upgrade is unrecoverable."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// the KILL gate (E4) — journal_mode is fail-closed
// ─────────────────────────────────────────────────────────────────────────────

/// E4: after migration, READ BACK the journal mode and refuse unless it is
/// `wal`. This is the round's KILL condition 3.
#[test]
fn r48_the_journal_mode_readback_refuses_when_not_wal() {
    let migration = read_repo("src/migration.rs");
    let code = code_region(&migration);
    assert!(
        code.contains("journal_mode"),
        "src/migration.rs must read back the journal mode — the pragma is set at :57 inside \
         an execute_batch that SUCCEEDS even when the mode silently stays 'delete'"
    );
    assert!(
        code.contains("query_row") || code.contains("query_one") || code.contains("pragma_update"),
        "the readback must actually QUERY the mode; setting it is not checking it"
    );
    // THE CONJUNCT THAT MATTERS. The first draft of this pin checked each
    // condition INDEPENDENTLY and passed vacuously on the pre-R48 tree:
    // `journal_mode` is satisfied by the SETTING at :57, `query_row` and `Err`
    // are satisfied by unrelated code elsewhere in the file. The distinguishing
    // feature of a real readback is the COMPARISON LITERAL — `"wal"`, lowercase and
    // quoted, which the `PRAGMA journal_mode=WAL` string does not contain. Measured
    // absent from migration.rs's production region today.
    assert!(
        code.contains("\"wal\"") || code.contains("'wal'"),
        "the readback must COMPARE the queried mode against the literal \"wal\". \
         `PRAGMA journal_mode=WAL` sets it; it does not check it. Without a \
         comparison there is no readback, only a hope."
    );
    assert!(
        code.contains("Err") || code.contains("bail") || code.contains("anyhow"),
        "a mode that is not `wal` must be an Err — the boot refuses, loudly, naming the cause"
    );
}

/// E4's premise, checked the right way round. The round's FIRST draft of this
/// pin asserted the readback was ABSENT — which is a pin that goes GREEN the
/// moment the round lands, i.e. a pin R48's own implementation falsifies.
/// Corrected: the property is that the readback lives in the **production**
/// region, which is the thing that is currently false.
#[test]
fn r48_the_journal_mode_readback_lives_in_the_production_region() {
    let migration = read_repo("src/migration.rs");
    let prod = production_region(&migration);
    let checks_mode = prod.lines().any(|l| {
        l.contains("journal_mode") && (l.contains("query_row") || l.contains("query_one"))
    });
    assert!(
        checks_mode,
        "the journal-mode readback must be in src/migration.rs's PRODUCTION region. \
         Today the ONLY assertion on the mode is src/workflow/host.rs:765, which sits \
         inside that file's #[cfg(test)] block — a test proves the code works, it does \
         not make the server refuse anything (E4)."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// the runbook's hard-won facts (E7, E8, and §23(b))
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn r48_the_runbook_never_says_copy_only_the_db_file() {
    let text = read_r48("docs/clean-cycle.md");
    assert!(
        !text.contains("cp brain.db /") && !text.contains("rsync brain.db "),
        "the runbook must never instruct copying brain.db ALONE. Separating the database \
         from its -wal file 'might [lose committed transactions], or the database file \
         might become corrupted' (sqlite.org/wal.html §4). Any generic 'just copy the .db' \
         advice is a data-loss bug wearing a backup's clothing (E7)."
    );
    assert!(
        text.contains("-wal") && text.contains("-shm"),
        "the runbook must name BOTH sidecar files travelling with the database (E7)"
    );
    assert!(
        text.to_lowercase().contains("quiesce") || text.to_lowercase().contains("stop the service"),
        "the runbook must state the QUIESCE-BEFORE-COPY rule: a copy taken while the \
         service is running is not consistent (E7)"
    );
}

#[test]
fn r48_the_runbook_states_the_network_filesystem_prohibition() {
    let text = read_r48("docs/clean-cycle.md");
    let low = text.to_lowercase();
    assert!(
        low.contains("network filesystem") || low.contains("nfs"),
        "the runbook must state the network-filesystem prohibition (E8)"
    );
    assert!(
        low.contains("locking") || low.contains("ext4") || low.contains("xfs"),
        "the runbook should give the reason (advisory locking / a local block filesystem), \
         not just the rule — an operator who does not know WHY will mount the NAS anyway"
    );
}

/// §23(b) of RA 10173 governs the off-site vault: transporting sensitive PI off
/// government property needs the agency head's approval.
#[test]
fn r48_the_runbook_names_the_offsite_approval_requirement() {
    let text = read_r48("docs/clean-cycle.md");
    assert!(
        text.contains("23(b)") || text.contains("agency head") || text.contains("approv"),
        "the runbook must name the §23(b) off-site approval requirement. An off-site \
         vault of citizen records is a documented, signed exception — not a free choice."
    );
    assert!(
        !text.to_lowercase().contains("is compliant") && !text.contains("RA 10173 compliance"),
        "the runbook must not state a COMPLIANCE conclusion. The instruments say what they \
         say; a Philippine counsel confirms scope."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// the scope + the laws (E7, E11, E12, KILL 1, KILL 5)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn r48_adds_no_table_no_stamp_no_dependency() {
    let layout = read_repo("src/storage_layout.rs");
    assert!(
        layout.contains("LATEST_KNOWN_SCHEMA: &str = SCHEMA_VERSION_V1_32_24;"),
        "the schema stamp is untouched: R48 adds no table and no stamp (the ceiling itself is \
         re-pinned by each schema round — R57b moved it, for the trace citation, R60 moved it \
         for the disproof condition's six columns on `claims`, the scope round moved it for the \
         seventh, and the per-domain axis round moved it last, for `knowledge_domain_versions`)"
    );
    let manifest = read_repo("Cargo.toml");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .and_then(|rest| rest.split("\n[").next())
        .expect("a [dependencies] section");
    let names: Vec<&str> = deps
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && l.contains('='))
        .filter_map(|l| l.split('=').next())
        .map(str::trim)
        .collect();
    assert_eq!(
        names.len(),
        52,
        "the dependency count is 52 — R48 adds no crate (found {}: {names:?})",
        names.len()
    );
    // the round's completeness half: the guard must not pass on an empty round
    assert!(
        exists("deploy/systemd/brain-server.service"),
        "the round's primary deliverable is part of the change set"
    );
}

/// KILL 1's receipt as a SOURCE property: R48 adds no route, so both guard
/// tables must still carry exactly the rows they carried at the round's open.
/// (The byte-untouched proof proper is the `git diff` in §6; this is the cheap
/// in-suite version, and it is deliberately a COUNT rather than a grep for a
/// marker R48 was never going to add.)
#[test]
fn r48_the_route_tables_are_unchanged() {
    // This pin froze the tables at R48's close and was an exact `assert_eq!`.
    // It has been RETIRED TWICE by a round that adds routes, deliberately and
    // with the numbers recorded rather than deleted: the create loop added six
    // surfaces across five distinct paths (coverage +5, authz +6), and the
    // agreement-labelling path added three distinct paths (coverage +3, authz
    // +3), taking the tables to 217 and 203.
    //
    // A pin that says "still exactly N" cannot survive a round that is allowed
    // to add routes, and a round that quietly edited the constant instead would
    // have turned R48's scope proof into a rubber stamp. The successor is the
    // create-loop suite's `route_tables_count_six_more_rows_than_the_prior_freeze`,
    // which states the same property positively and cannot be satisfied by
    // deleting rows.
    const OPEN_ROUTES: usize = 217;
    const OPEN_GATES: usize = 203;

    let guards = read_repo("src/server/router/route_guards.rs");
    let between = |start: &str| -> String {
        let tail = guards
            .split(start)
            .nth(1)
            .unwrap_or_else(|| panic!("{start} must still be declared"));
        tail.split("];").next().unwrap_or("").to_string()
    };
    let routes = between("pub const OPENAPI_ROUTES")
        .lines()
        .filter(|l| l.trim_start().starts_with('"'))
        .count();
    let gates = between("pub const AUTHZ_GATES")
        .lines()
        .filter(|l| l.trim_start().starts_with('('))
        .count();
    assert_eq!(
        routes, OPEN_ROUTES,
        "OPENAPI_ROUTES holds {routes} rows; R48 adds no route, so it must still hold {OPEN_ROUTES}"
    );
    assert_eq!(
        gates, OPEN_GATES,
        "AUTHZ_GATES holds {gates} rows; R48 adds no route, so it must still hold {OPEN_GATES}"
    );

    let role = read_repo("src/role.rs");
    let presets = role
        .split("pub const PRESETS_RAW")
        .nth(1)
        .and_then(|r| r.split("];").next())
        .expect("PRESETS_RAW must still be declared");
    assert_eq!(
        presets.matches("\"name\":").count(),
        13,
        "the role vocabulary is FROZEN (13 presets); R48 must not touch it"
    );
}

/// E12: the up-only floor cannot catch a same-round deletion. This pin can.
#[test]
fn r48_the_rounds_own_test_count_did_not_drop() {
    const ROUND_OPEN_FLOOR: usize = 2_376;
    let spire = read_repo("src/spire_inventory.rs");
    let floor_line = spire
        .lines()
        .find(|l| l.contains("const CRATE_TEST_FLOOR: usize"))
        .expect("CRATE_TEST_FLOOR must still be declared");
    let floor: usize = floor_line
        .split('=')
        .nth(1)
        .and_then(|s| s.trim().trim_end_matches(';').replace('_', "").parse().ok())
        .expect("CRATE_TEST_FLOOR must be a usize literal");
    assert!(
        floor >= ROUND_OPEN_FLOOR,
        "CRATE_TEST_FLOOR fell to {floor}; it is UP ONLY"
    );
    let mut files = Vec::new();
    walk_rs_files(&repo_root().join("src"), &mut files);
    walk_rs_files(&repo_root().join("tests"), &mut files);
    let measured: usize = files
        .iter()
        .map(|p| {
            std::fs::read_to_string(p)
                .unwrap_or_default()
                .matches("#[test]")
                .count()
        })
        .sum();
    assert!(
        measured >= floor,
        "the walk measures {measured} but the floor is {floor}"
    );
    // and the REDUCTION the up-only floor cannot see: the round must not have
    // lost pins relative to its own base.
    assert!(
        measured >= 2_380,
        "the walk measures {measured}; at R48's open it was 2,380 (2,376 floor + the four \
         pins R47's wire fix added). A same-round deletion still clears the up-only floor — \
         that is E12, and this is the assertion the floor cannot make."
    );
}

/// KILL 5: the singularity law.
#[test]
fn r48_single_authorize_decision_still_holds() {
    let mut files = Vec::new();
    walk_rs_files(&repo_root().join("src"), &mut files);
    let mut sites: Vec<String> = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        if code_region(&text)
            .lines()
            .any(|l| l.trim_start().starts_with("pub fn authorize("))
        {
            sites.push(f.display().to_string());
        }
    }
    assert_eq!(
        sites.len(),
        1,
        "exactly ONE `pub fn authorize` may exist in src/ (found {sites:?})"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// the non-vacuity self-pin
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn r48_the_battery_scan_is_not_vacuous() {
    let mut files = Vec::new();
    walk_all_files(&repo_root().join("deploy"), &mut files);
    assert!(
        !files.is_empty(),
        "the deploy/ walk found no files; a guard that scans nothing must not smile. \
         NOTE the walker must be the ALL-FILES one: a .rs-only walker finds nothing \
         in a tree made of a .service file and shell scripts."
    );
    let total: usize = files
        .iter()
        .map(|f| std::fs::read_to_string(f).map(|t| t.len()).unwrap_or(0))
        .sum();
    assert!(
        total > 500,
        "the deploy tree totals {total} bytes; too small to be the round"
    );

    // IN-BAND red-proof: the same walker over a non-existent dir must be empty.
    let mut empty: Vec<PathBuf> = Vec::new();
    walk_all_files(&repo_root().join("deploy/definitely-not-here"), &mut empty);
    assert!(
        empty.is_empty(),
        "the red-proof walk must find nothing; if it finds files the walker's join is wrong"
    );

    // and the census the pins drive is the REAL one
    let unit = read_r48("deploy/systemd/brain-server.service");
    assert!(
        unit.contains("NoNewPrivileges"),
        "the census the pins drive is reading something other than the round's unit"
    );
}
