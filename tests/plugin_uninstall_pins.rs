//! The uninstall round's pins — `brain plugin uninstall` through the REAL
//! binary (the daebe7fe law: the door is the parser, and a library call
//! never parses). The fixture kit is `plugin_install_pins.rs`'s, extended
//! with the roster doors, because REFUSE-WHILE-LIVE is the round's teeth:
//! the witnessed operator sequence is install → grant → REFUSED (naming
//! the principal and the revoke door) → revoke → REFUSED (naming the
//! binding and the unbind door) → unbind → dry-run → --yes → the data law
//! → re-install as a fresh decision.

use brain_server::packs::archive::write_archive;
use ed25519_dalek::Signer;
use std::process::Command;

// ── the fixture kit (plugin_install_pins' measured shape) ──────────────────

fn mint_key(id: u64) -> (ed25519_dalek::SigningKey, String) {
    let sk = ed25519_dalek::SigningKey::from_bytes(&{
        let mut seed = [0u8; 32];
        seed[..8].copy_from_slice(&id.to_be_bytes());
        seed
    });
    let mut blob = Vec::new();
    blob.extend_from_slice(b"Ed");
    blob.extend_from_slice(&id.to_be_bytes());
    blob.extend_from_slice(&sk.verifying_key().to_bytes());
    let keyfile = format!("untrusted comment: pack key\n{}\n", {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(&blob)
    });
    (sk, keyfile)
}

fn sign_measured(sk: &ed25519_dalek::SigningKey, key_id: u64, content: &[u8]) -> String {
    use blake2::Digest;
    let digest = blake2::Blake2b512::digest(content);
    let primary = sk.sign(&digest);
    let trusted = "file:pack.tar.gz";
    let mut signed = Vec::new();
    signed.extend_from_slice(&primary.to_bytes());
    signed.extend_from_slice(trusted.as_bytes());
    let secondary = sk.sign(&signed);
    let mut blob = Vec::new();
    blob.extend_from_slice(b"ED");
    blob.extend_from_slice(&key_id.to_be_bytes());
    blob.extend_from_slice(&primary.to_bytes());
    let b64 = |d: &[u8]| {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(d)
    };
    format!(
        "untrusted comment: signature\n{}\ntrusted comment: {trusted}\n{}\n",
        b64(&blob),
        b64(&secondary.to_bytes())
    )
}

/// The support pack with EVERY live-state arm: a domain bound to a preset
/// profile (the profile arm), an upserted role (the roles arm), and two
/// governed rules (the data law's count).
fn pack_entries(id: &str, version: &str) -> Vec<(String, Vec<u8>)> {
    vec![
        (
            "manifest.json".into(),
            serde_json::json!({
                "id": id, "version": version, "engine": ">=1.0.0",
                "kinds": ["domain", "roles", "rules"]
            })
            .to_string()
            .into_bytes(),
        ),
        (
            "domain.json".into(),
            serde_json::json!({
                "name": "support", "label": "Support desk",
                "retention_days": 90, "profile": "call-center"
            })
            .to_string()
            .into_bytes(),
        ),
        (
            "roles.json".into(),
            serde_json::json!([{
                "name": "support-supervisor",
                "scopes": ["domain"], "owner_filter": "reports",
                "can": ["read", "write", "approve"]
            }])
            .to_string()
            .into_bytes(),
        ),
        (
            "rules.json".into(),
            serde_json::json!([
                {"jurisdiction": "ph", "subject": "support-tickets",
                 "rule_key": "ticket_retention",
                 "body": "Resolved ticket corpus is retained 90 days post-resolution.",
                 "source_ref": "pack#retention", "effective_at": 1_700_000_000,
                 "revision": 1},
                {"jurisdiction": "ph", "subject": "support-tickets",
                 "rule_key": "breach_posture",
                 "body": "A suspected breach escalates to the supervisor queue.",
                 "source_ref": "pack#breach", "effective_at": 1_700_000_000,
                 "revision": 1}
            ])
            .to_string()
            .into_bytes(),
        ),
    ]
}

struct Packed {
    dir: std::path::PathBuf,
    db: std::path::PathBuf,
    pinned: String,
    sk: ed25519_dalek::SigningKey,
    key_id: u64,
}

fn setup(tag: &str) -> Packed {
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let dir = std::env::temp_dir().join(format!(
        "brain-plugin-uninstall-{tag}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut db_path = dir.clone();
    db_path.push("probe.db");
    let mut conn = rusqlite::Connection::open(&db_path).expect("open fixture db");
    brain_server::migration::run_migration(&mut conn, 512).expect("migrate fixture db");
    drop(conn);
    let (sk, pinned) = mint_key(0x1111_2222_3333_4444);
    Packed {
        dir,
        db: db_path,
        pinned,
        sk,
        key_id: 0x1111_2222_3333_4444,
    }
}

fn write_pack(packed: &Packed, name: &str, entries: &[(String, Vec<u8>)]) -> std::path::PathBuf {
    let refs: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_slice()))
        .collect();
    let archive = write_archive(&refs);
    let path = packed.dir.join(name);
    std::fs::write(&path, &archive).expect("write the pack");
    std::fs::write(
        format!("{}.minisig", path.display()),
        sign_measured(&packed.sk, packed.key_id, &archive),
    )
    .expect("write the signature");
    path
}

fn run(db: &std::path::Path, pubkey_file: &str, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_brain"))
        .args(args)
        .arg("--db")
        .arg(db)
        .env("BRAIN_PACK_PUBKEY", pubkey_file)
        .output()
        .expect("the brain binary runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn counts(db: &std::path::Path) -> (i64, i64, i64, i64, Option<i64>) {
    let conn = rusqlite::Connection::open(db).unwrap();
    let one = |sql: &str| -> i64 {
        conn.query_row(sql, [], |r| r.get(0))
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    };
    let removed_at: Option<i64> = conn
        .query_row(
            "SELECT removed_at FROM installed_packs WHERE pack_id = 'support-pack'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    (
        one("SELECT COUNT(*) FROM installed_packs"),
        one("SELECT COUNT(*) FROM rules"),
        one("SELECT COUNT(*) FROM audit_events WHERE kind='plugin'"),
        one("SELECT COUNT(*) FROM roles WHERE name='support-supervisor'"),
        removed_at,
    )
}

// ── the pins ───────────────────────────────────────────────────────────────

/// THE WITNESSED OPERATOR SEQUENCE, every act through the real binary:
/// install → grant → uninstall REFUSES naming the live principal and the
/// revoke door → revoke → uninstall STILL REFUSES naming the binding and
/// the unbind door → unbind → the dry-run default → --yes marks the
/// register and moves nothing else → the already-removed refusal → the
/// register reads removed → the SAME version re-installs (a fresh
/// decision).
#[test]
fn the_witnessed_sequence_refuses_while_live_then_marks_then_reinstalls() {
    let packed = setup("witness");
    let pinned_file = packed.dir.join("pack.pub");
    std::fs::write(&pinned_file, &packed.pinned).expect("pin the pack key");
    let pinned_str = pinned_file.display().to_string();

    // install --yes
    let path = write_pack(
        &packed,
        "pack.tar.gz",
        &pack_entries("support-pack", "0.1.0"),
    );
    let (code, _stdout, stderr) = run(
        &packed.db,
        &pinned_str,
        &["plugin", "install", &path.display().to_string(), "--yes"],
    );
    assert_eq!(code, 0, "{stderr}");
    let (register, rules, plugin_rows, roles, removed_at) = counts(&packed.db);
    assert_eq!((register, rules, plugin_rows, roles), (1, 2, 1, 1));
    assert_eq!(removed_at, None, "a fresh install is a live registration");

    // the roster door grants a principal on the pack-upserted role
    let token_out = packed.dir.join("support-agent.token");
    let (code, _stdout, stderr) = run(
        &packed.db,
        &pinned_str,
        &[
            "principal",
            "grant",
            "--name",
            "support-agent",
            "--role",
            "support-supervisor",
            "--domain",
            "support",
            "--out",
            &token_out.display().to_string(),
        ],
    );
    assert_eq!(code, 0, "{stderr}");

    // REFUSE-WHILE-LIVE, the ROLES arm — the refusal NAMES the principal,
    // the role, and the door that clears it. Dry-run or --yes alike: a
    // live pack refuses before anything writes.
    for extra in [Vec::<&str>::new(), vec!["--yes"]] {
        let mut args: Vec<&str> = vec!["plugin", "uninstall", "support-pack"];
        args.extend_from_slice(&extra);
        let (code, _stdout, stderr) = run(&packed.db, &pinned_str, &args);
        assert_ne!(code, 0, "a live pack refuses (extra: {extra:?})");
        assert!(stderr.contains("uninstall refused"), "{stderr}");
        assert!(stderr.contains("support-agent"), "{stderr}");
        assert!(stderr.contains("support-supervisor"), "{stderr}");
        assert!(
            stderr.contains("brain principal revoke --name support-agent"),
            "the refusal names the clearing door: {stderr}"
        );
        assert_eq!(counts(&packed.db).4, None, "a refusal writes nothing");
    }

    // the revoke door (the real CLI act)
    let (code, _stdout, stderr) = run(
        &packed.db,
        &pinned_str,
        &[
            "principal",
            "revoke",
            "--name",
            "support-agent",
            "--reason",
            "vertical retired",
        ],
    );
    assert_eq!(code, 0, "{stderr}");

    // REFUSE-WHILE-LIVE, the PROFILE arm — after the revoke the binding
    // still stands, and the refusal names profile + domain + the unbind.
    let (code, _stdout, stderr) = run(
        &packed.db,
        &pinned_str,
        &["plugin", "uninstall", "support-pack", "--yes"],
    );
    assert_ne!(code, 0, "the binding keeps the pack live");
    assert!(stderr.contains("uninstall refused"), "{stderr}");
    assert!(stderr.contains("call-center"), "{stderr}");
    assert!(stderr.contains("support"), "{stderr}");
    assert!(stderr.contains("unbind"), "{stderr}");
    assert_eq!(counts(&packed.db).4, None, "a refusal writes nothing");

    // THE UNBIND DOOR: `profile::bind(domain, None)` — the exact function
    // the HTTP door and the installer share (the CLI ships no unbind verb;
    // the lib pins witness the door's internals, this pin witnesses the
    // operator's path through it).
    {
        let conn = rusqlite::Connection::open(&packed.db).unwrap();
        brain_server::profile::bind(&conn, "support", None).expect("the unbind lands");
    }

    // THE DRY-RUN IS THE DEFAULT SURFACE: nothing written without --yes,
    // the mark printed as a would.
    let (code, stdout, stderr) = run(
        &packed.db,
        &pinned_str,
        &["plugin", "uninstall", "support-pack"],
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("would mark"), "{stdout}");
    assert!(stdout.contains("nothing written — pass --yes"), "{stdout}");
    assert!(
        stdout.contains("the imported DATA stays"),
        "the dry-run says the erasure boundary out loud: {stdout}"
    );
    let (register, rules, plugin_rows, roles, removed_at) = counts(&packed.db);
    assert_eq!(
        (register, rules, plugin_rows, roles, removed_at),
        (1, 2, 1, 1, None),
        "a dry-run writes NOTHING"
    );

    // --yes is the only writer: the mark lands, ONE audit row lands, and
    // the DATA LAW holds — zero content rows change.
    let (code, stdout, stderr) = run(
        &packed.db,
        &pinned_str,
        &["plugin", "uninstall", "support-pack", "--yes"],
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("marked removed"), "{stdout}");
    assert!(stdout.contains("the imported DATA stays"), "{stdout}");
    let (register, rules, plugin_rows, roles, removed_at) = counts(&packed.db);
    assert_eq!(register, 1, "the row is marked, never deleted");
    assert_eq!(rules, 2, "the data law: zero rule rows change");
    assert_eq!(roles, 1, "the data law: zero role rows change");
    assert_eq!(plugin_rows, 2, "import + uninstall, on the chain");
    assert!(removed_at.is_some(), "the mark is set");

    // an already-removed registration refuses: history, not a live
    // registration
    let (code, _stdout, stderr) = run(
        &packed.db,
        &pinned_str,
        &["plugin", "uninstall", "support-pack", "--yes"],
    );
    assert_ne!(code, 0);
    assert!(stderr.contains("already removed"), "{stderr}");
    assert!(stderr.contains("fresh decision"), "{stderr}");

    // the register reads as removed — history, not a live registration
    let (code, stdout, stderr) = run(&packed.db, &pinned_str, &["plugin", "list"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("support-pack"), "{stdout}");
    assert!(stdout.contains("removed"), "{stdout}");

    // THE RE-INSTALL LAW RESETS: the SAME version installs again — a fresh
    // decision, the mark lifts, one row stays one row.
    let (code, _stdout, stderr) = run(
        &packed.db,
        &pinned_str,
        &["plugin", "install", &path.display().to_string(), "--yes"],
    );
    assert_eq!(code, 0, "the re-offer is a fresh decision: {stderr}");
    let (register, rules, plugin_rows, _roles, removed_at) = counts(&packed.db);
    assert_eq!(register, 1, "one row stays one row");
    assert_eq!(removed_at, None, "the re-install lifts the mark");
    assert_eq!(rules, 2, "the unchanged rules replay-skip (the C1 law)");
    assert_eq!(
        plugin_rows, 3,
        "import + uninstall + re-import, on the chain"
    );

    // an unknown pack-id refuses naming the register
    let (code, _stdout, stderr) = run(
        &packed.db,
        &pinned_str,
        &["plugin", "uninstall", "no-such-pack", "--yes"],
    );
    assert_ne!(code, 0);
    assert!(
        stderr.contains("no live registration") || stderr.contains("no installed pack named"),
        "{stderr}"
    );
}

/// THE THIRD REFUSAL POSTURE, A FACT ABOUT THE TREE: no uninstall HTTP
/// route exists. The act is a LOCAL operator act on the DB the operator
/// names (the mint refusal's posture, the erasure boundary beside it).
/// The scan reads the router sources with the anti-vacuity arms proving
/// the scan sees real registrations.
#[test]
fn no_uninstall_http_route_exists() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/server/router");
    let mut corpus = String::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("router dir")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    entries.sort();
    assert!(
        entries.len() >= 5,
        "the router tree moved — re-derive this scan (found {} files)",
        entries.len()
    );
    for p in &entries {
        corpus.push_str(&std::fs::read_to_string(p).unwrap_or_default());
    }
    // ANTI-VACUITY: the scan must see real registrations.
    for real in [
        ".route(\"/audit\"",
        ".route(\"/roles\"",
        ".route(\"/recall\"",
    ] {
        assert!(
            corpus.contains(real),
            "the scan corpus lost `{real}` — the scanner is broken, not the tree"
        );
    }
    assert!(
        !corpus.contains(".route(\"/plugin"),
        "a plugin HTTP route appeared — pack lifecycle acts are LOCAL operator acts; \
         the refusal is a NAMED decision: revert it or amend the law in the open, \
         never by drift"
    );
}
