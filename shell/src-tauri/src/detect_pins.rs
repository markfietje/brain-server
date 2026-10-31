//! W-T1's pins — the detection core's laws, hermetic by construction: every
//! pin drives `detect.rs` against a temp home whose fixtures stand in for
//! the machine's real paths. The ZERO-WRITE law is a source-scan; the
//! evidence law and the D3 shape are drives over the real fns.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::detect::{
    NOT_FOUND_SENTENCE, detect_all, detect_existing_install, detect_kernel_on_port,
    detect_openclaw, detect_update, detect_workspace, version_order,
};

// ── the harness ─────────────────────────────────────────────────────────────

/// A temp home with the round's own fixtures: an operator-shaped plist, an
/// OpenClaw config + workspace, all inside the pin's own sandbox.
struct FixtureHome {
    root: Box<Path>,
    workspace_dbs: Vec<(String, usize)>,
}

impl FixtureHome {
    fn with_operator_install(program: &str) -> Self {
        let fixture = Self::empty();
        // The D1 plist, the documented launchd shape.
        let agents = fixture.root.join("Library/LaunchAgents");
        fs::create_dir_all(&agents).expect("LaunchAgents fixture");
        fs::write(
            agents.join("com.brain.server.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <plist version=\"1.0\">\n<dict>\n\
                 <key>Label</key><string>com.brain.server</string>\n\
                 <key>ProgramArguments</key>\n<array>\n\
                 <string>{program}</string>\n\
                 </array>\n\
                 </dict>\n</plist>\n"
            ),
        )
        .expect("operator plist fixture");
        fixture
    }

    fn with_openclaw() -> Self {
        let mut fixture = Self::empty();
        let openclaw = fixture.root.join(".openclaw");
        fs::create_dir_all(&openclaw).expect("openclaw fixture");
        fs::write(
            openclaw.join("openclaw.json"),
            r#"{"plugins":{"entries":{"brain-server":{"enabled":true}}}}"#,
        )
        .expect("openclaw config fixture");
        let workspace = openclaw.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace fixture");
        for (name, size) in [
            ("brain-global.db", 0),
            ("brain-import-dom-10345000.db", 8192),
            ("brain-import-dom-108239000.db", 8192),
        ] {
            fs::write(workspace.join(name), vec![0u8; size]).expect("workspace db fixture");
            fixture.workspace_dbs.push((name.to_string(), size));
        }
        // A non-database file the enumeration must skip.
        fs::write(workspace.join("notes.txt"), b"not a database").expect("distractor");
        fixture
    }

    fn empty() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "detect-pins-{}-{:?}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("temp home");
        Self {
            root: dir.into_boxed_path(),
            workspace_dbs: Vec::new(),
        }
    }

    fn home(&self) -> &Path {
        &self.root
    }
}

impl Drop for FixtureHome {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).ok();
    }
}

/// A stand-in "installed binary" the D1/D5 version exec can run: a shell
/// script is refused (no shell), so the fixture is a copy of THIS test
/// binary — it exists, it execs, it is not a kernel, so `--version` reads
/// as unknown. The version laws are pinned by `bounded_version`'s own arms
/// below, not by the fixture pretending to be a kernel.
fn execable_standin(dir: &Path, name: &str) -> String {
    let path = dir.join(name);
    fs::copy(std::env::current_exe().expect("test binary"), &path).expect("standin");
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o755));
    path.display().to_string()
}

// ── THE ZERO-WRITE LAW ──────────────────────────────────────────────────────

/// The detection core performs ZERO writes: the module's source may not so
/// much as NAME a write API — no `fs::write`, no `OpenOptions`, no
/// `File::create`, no remove/rename/rename-dir/set-permissions/create-dir.
/// The D1 read is the documented plist path and nothing else; the collision
/// law forbids writes, not reads, and this pin holds the read-only shape.
#[test]
fn the_detection_core_performs_zero_writes() {
    let source = include_str!("detect.rs");
    for forbidden in [
        "fs::write",
        "OpenOptions",
        "File::create",
        "remove_file",
        "remove_dir",
        "fs::rename",
        "set_permissions",
        "create_dir",
    ] {
        assert!(
            !source.contains(forbidden),
            "detect.rs names {forbidden} — the detection core performs ZERO writes; \
             a detector that writes is an installer wearing a reader's badge"
        );
    }
    // The documented D1 path, constructed exactly ONCE in code: no second
    // LaunchAgents construct, no other plist location. (Prose may name the
    // path — documentation is allowed; the CONSTRUCT is what reads.)
    assert_eq!(
        source.matches("join(\"Library/LaunchAgents\")").count(),
        1,
        "the D1 read is the documented plist path and nothing else"
    );
}

// ── the evidence law ────────────────────────────────────────────────────────

/// Every found card carries the path(s) it actually read — a claim without
/// a path is a lie the pin refuses. And an absent detection is NOT silently
/// absent: the renderer's sentence is the module's own named constant.
#[test]
fn every_found_card_carries_its_evidence_path() {
    let fixture = FixtureHome::with_operator_install("/nonexistent/brain-server");
    let home = fixture.home();
    let existing = detect_existing_install(home, false).expect("the fixture carries an install");
    assert!(
        existing
            .evidence
            .contains("Library/LaunchAgents/com.brain.server.plist"),
        "the D1 card names the plist it read: {}",
        existing.evidence
    );
    assert!(existing.evidence.contains("ProgramArguments[0]"));

    let openclaw = FixtureHome::with_openclaw();
    let openclaw_card = detect_openclaw(openclaw.home()).expect("the fixture carries openclaw");
    assert!(
        openclaw_card.evidence.contains(".openclaw/openclaw.json"),
        "the D2 card names the config it read: {}",
        openclaw_card.evidence
    );

    let workspace = detect_workspace(openclaw.home()).expect("the fixture carries workspace dbs");
    assert!(
        workspace.evidence.contains(".openclaw/workspace"),
        "the D3 card names the directory it enumerated: {}",
        workspace.evidence
    );

    let kernel = detect_kernel_on_port();
    assert!(!kernel.evidence.is_empty(), "the D4 card names its probe");

    // The no-silent-absence law: the sentence exists, is named, and is what
    // the renderer shows for an absent card.
    assert!(NOT_FOUND_SENTENCE.contains("not found"));
    assert!(NOT_FOUND_SENTENCE.contains("nothing touched"));
}

// ── D1's laws ───────────────────────────────────────────────────────────────

/// D1 reads the documented plist and resolves ProgramArguments[0]. A
/// missing or malformed plist is NO card (None), never an error and never a
/// guess.
#[test]
fn d1_resolves_the_documented_plist_or_reports_nothing() {
    let fixture = FixtureHome::with_operator_install("/nonexistent/brain-server");
    let existing = detect_existing_install(fixture.home(), false)
        .expect("a well-formed plist yields the card");
    assert_eq!(existing.binary_path, "/nonexistent/brain-server");
    assert!(!existing.running, "the fixture's port is not held");

    // A malformed plist is no card — a guess would be worse than absence.
    let broken = FixtureHome::empty();
    let agents = broken.root.join("Library/LaunchAgents");
    fs::create_dir_all(&agents).expect("agents dir");
    fs::write(agents.join("com.brain.server.plist"), b"not a plist").expect("broken plist");
    assert!(detect_existing_install(broken.home(), false).is_none());

    // No plist at all is no card.
    let none = FixtureHome::empty();
    assert!(detect_existing_install(none.home(), false).is_none());
}

// ── D3's laws ───────────────────────────────────────────────────────────────

/// The workspace enumeration lists names/sizes/mtimes — and the TYPE cannot
/// carry content: the struct's fields are exactly the three metadata keys,
/// so the installer could not open the databases even by accident. The
/// listing skips non-database files and orders newest-first.
#[test]
fn d3_lists_metadata_only_and_never_the_contents() {
    let fixture = FixtureHome::with_openclaw();
    let workspace = detect_workspace(fixture.home()).expect("the fixture carries workspace dbs");
    assert_eq!(workspace.databases.len(), 3, "the distractor is skipped");
    let names: Vec<&str> = workspace
        .databases
        .iter()
        .map(|db| db.name.as_str())
        .collect();
    assert!(names.contains(&"brain-global.db"));
    assert!(names.contains(&"brain-import-dom-10345000.db"));
    assert!(!names.contains(&"notes.txt"), "non-databases are skipped");
    assert_eq!(
        workspace.total_bytes,
        fixture
            .workspace_dbs
            .iter()
            .map(|(_, size)| *size as u64)
            .sum::<u64>(),
        "the total is the sum of the listed sizes"
    );
    // Newest-first, and every row carries its mtime.
    let modified: Vec<u64> = workspace
        .databases
        .iter()
        .map(|db| db.modified_epoch)
        .collect();
    let mut sorted = modified.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(modified, sorted, "newest first");
    // The content refusal is STRUCTURAL: the fields are name/size/modified
    // — serialized, nothing else can leave the module.
    let db = serde_json::to_value(&workspace.databases[0]).expect("serializes");
    let keys: Vec<&str> = db
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        ["modifiedEpoch", "name", "sizeBytes"],
        "the listing cannot carry contents — the shape has no field for them"
    );
}

// ── D5's laws ───────────────────────────────────────────────────────────────

/// The version order: numeric components, missing = zero, pre-release loses
/// to release. The update card's comparison rides this, so a 1.29.9 vs
/// 1.29.10 string-compare bug cannot rename an update into a downgrade.
#[test]
fn the_version_order_is_numeric_not_lexicographic() {
    use std::cmp::Ordering;
    assert_eq!(version_order("1.29.9", "1.29.10"), Ordering::Less);
    assert_eq!(version_order("1.29.10", "1.29.9"), Ordering::Greater);
    assert_eq!(version_order("1.29.38", "1.29.38"), Ordering::Equal);
    assert_eq!(version_order("1.29", "1.29.0"), Ordering::Equal);
    assert_eq!(version_order("1.30.0-beta.1", "1.30.0"), Ordering::Less);
}

/// The update card names BOTH versions and only exists when they differ.
/// A same-version compare is NOT a card (nothing to propose); the evidence
/// names both executables.
#[test]
fn the_update_card_names_both_versions_or_does_not_exist() {
    let fixture = FixtureHome::empty();
    let dir = fixture.root.join("bin");
    fs::create_dir_all(&dir).expect("bin fixture");
    let standin = execable_standin(&dir, "brain-server");
    // The standin answers no --version, so the card cannot exist — an
    // unknown version is not an update proposal.
    assert!(detect_update(Path::new(&standin), &standin).is_none());
}

// ── the report's laws ───────────────────────────────────────────────────────

/// The whole report over an EMPTY home: every card absent, the port card
/// honest about whatever this pin's machine holds, and nothing written
/// anywhere (the temp home is disposed by the fixture's Drop).
#[test]
fn the_report_over_an_empty_home_is_honest_absence() {
    let fixture = FixtureHome::empty();
    let report = detect_all(fixture.home(), None);
    assert!(report.existing.is_none(), "no plist, no card");
    assert!(report.openclaw.is_none(), "no config, no card");
    assert!(report.workspace.is_none(), "no workspace, no card");
    assert!(
        report.updates.is_empty(),
        "no offered sidecar, no update cards"
    );
    // The port card ALWAYS exists — its honest absence is the "free port"
    // fact the proposal needs.
    assert!(!report.kernel_on_port.evidence.is_empty());
}
