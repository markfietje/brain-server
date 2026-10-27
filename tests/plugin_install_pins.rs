//! G-4 — `brain plugin install`: the signed chain's pins, through the REAL
//! binary (the daebe7fe law: the door is the parser, and a library call
//! never parses).
//!
//! Every pin builds an exact archive in-process
//! ([`brain_server::packs::archive::write_archive`], the reader's pinned
//! opposite), signs it the MEASURED minisign way with a synthetic key
//! (the same byte-format the real-CLI probe test in
//! `src/packs/signature.rs` cross-checks), pins the key, and drives the
//! binary against a fresh migrated DB.

use brain_server::packs::archive::write_archive;
use ed25519_dalek::Signer;
use std::process::Command;

// ── the fixture kit ────────────────────────────────────────────────────────

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

fn manifest(id: &str, version: &str) -> String {
    serde_json::json!({
        "id": id, "version": version, "engine": ">=1.0.0",
        "kinds": ["domain", "roles", "rules"]
    })
    .to_string()
}

/// The support pack's data — the C1 vertical's rows as pack sections.
fn pack_entries(id: &str, version: &str) -> Vec<(String, Vec<u8>)> {
    vec![
        ("manifest.json".into(), manifest(id, version).into_bytes()),
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
    /// the pinned public half (the only key the binary will trust)
    pinned: String,
    sk: ed25519_dalek::SigningKey,
    key_id: u64,
}

fn setup(tag: &str) -> Packed {
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let dir =
        std::env::temp_dir().join(format!("brain-plugin-install-{tag}-{}", std::process::id()));
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

fn write_pack(
    packed: &Packed,
    name: &str,
    entries: &[(String, Vec<u8>)],
    signer: Option<&ed25519_dalek::SigningKey>,
    signer_id: u64,
) -> std::path::PathBuf {
    let refs: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_slice()))
        .collect();
    let archive = write_archive(&refs);
    let path = packed.dir.join(name);
    std::fs::write(&path, &archive).expect("write the pack");
    let sk = signer.unwrap_or(&packed.sk);
    std::fs::write(
        format!("{}.minisig", path.display()),
        sign_measured(sk, signer_id, &archive),
    )
    .expect("write the signature");
    path
}

fn run(bin: &str, db: &std::path::Path, pubkey_file: &str, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin)
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

fn counts(db: &std::path::Path) -> (i64, i64, i64, i64) {
    let conn = rusqlite::Connection::open(db).unwrap();
    let one = |sql: &str| -> i64 {
        conn.query_row(sql, [], |r| r.get(0))
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    };
    (
        one("SELECT COUNT(*) FROM installed_packs"),
        one("SELECT COUNT(*) FROM rules"),
        one("SELECT COUNT(*) FROM audit_events WHERE kind='plugin'"),
        one("SELECT COUNT(*) FROM roles WHERE name='support-supervisor'"),
    )
}

// ── the pins ───────────────────────────────────────────────────────────────

#[test]
fn the_full_chain_lands_every_declared_row_dry_run_prints_count_for_count() {
    let bin = env!("CARGO_BIN_EXE_brain");
    let packed = setup("happy");
    let entries = pack_entries("support-pack", "0.1.0");
    let path = write_pack(&packed, "pack.tar.gz", &entries, None, packed.key_id);
    let path_str = path.display().to_string();
    let pinned_file = packed.dir.join("pack.pub");
    std::fs::write(&pinned_file, &packed.pinned).expect("pin the pack key");

    // THE DEFAULT SURFACE: no --yes → dry-run, nothing written
    let (code, stdout, stderr) = run(
        bin,
        &packed.db,
        &pinned_file.display().to_string(),
        &["plugin", "install", &path_str],
    );
    assert_eq!(code, 0, "{stderr}");
    // every row that WOULD land is printed, by name
    assert!(stdout.contains("would land: domain support (profile call-center)"));
    assert!(stdout.contains("would land: role support-supervisor"));
    assert!(stdout.contains("would land: rule ph:support-tickets:ticket_retention @r1"));
    assert!(stdout.contains("would land: rule ph:support-tickets:breach_posture @r1"));
    assert!(stdout.contains("nothing written — pass --yes"));
    // counts BEFORE == the untouched counts: dry-run wrote NOTHING
    let before = counts(&packed.db);
    assert_eq!(before.0, 0, "no register row");
    assert_eq!(before.1, 0, "no rule rows");

    // --yes imports: the landed rows == the printed rows, count for count
    let (code, stdout, stderr) = run(
        bin,
        &packed.db,
        &pinned_file.display().to_string(),
        &["plugin", "install", &path_str, "--yes"],
    );
    assert_eq!(code, 0, "{stderr}");
    let printed_rows = 4; // domain + role + 2 rules, as printed above
    assert!(
        stdout.contains(&format!("{printed_rows} rows landed")),
        "the import receipts its row count: {stdout}"
    );
    let after = counts(&packed.db);
    assert_eq!(after.0, 1, "the register row");
    assert_eq!(after.1, 2, "the two rule rows");
    assert_eq!(after.2, 1, "the import act's audit row");
    assert_eq!(after.3, 1, "the role landed through the roles store");
    // the dry-run's printed count == the import's landed count (4 == 4)

    // plugin list carries the pack: id, version, kinds
    let (code, stdout, stderr) = run(
        bin,
        &packed.db,
        &pinned_file.display().to_string(),
        &["plugin", "list"],
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("support-pack"), "{stdout}");
    assert!(stdout.contains("0.1.0"));
    assert!(stdout.contains("domain,roles,rules"));
}

#[test]
fn a_tampered_archive_is_refused_post_signature_and_a_foreign_key_is_refused_by_name() {
    let bin = env!("CARGO_BIN_EXE_brain");
    let packed = setup("tamper");
    let pinned_file = packed.dir.join("pack.pub");
    std::fs::write(&pinned_file, &packed.pinned).expect("pin the pack key");
    let pinned_str = pinned_file.display().to_string();

    // THE tamper arm: sign the true bytes, flip one byte, install → refused
    let entries = pack_entries("support-pack", "0.1.0");
    let path = write_pack(&packed, "pack.tar.gz", &entries, None, packed.key_id);
    let mut archive = std::fs::read(&path).unwrap();
    let last = archive.len() - 1;
    archive[last] ^= 0x01;
    std::fs::write(&path, &archive).unwrap(); // bytes changed, signature stale
    let path_str = path.display().to_string();
    let (code, _stdout, stderr) = run(
        bin,
        &packed.db,
        &pinned_str,
        &["plugin", "install", &path_str, "--yes"],
    );
    assert_ne!(code, 0, "a tampered pack installs NOTHING");
    assert!(stderr.contains("signature refused"), "{stderr}");
    assert_eq!(counts(&packed.db).0, 0, "the tamper landed nothing");

    // THE foreign-key arm: the UPDATER-style second key signs, the pack
    // key's pin verifies → refused naming BOTH keynums
    let (other_sk, _other_pub) = mint_key(0x9999_8888_7777_6666);
    let true_entries = pack_entries("support-pack", "0.1.0");
    let path2 = write_pack(
        &packed,
        "pack2.tar.gz",
        &true_entries,
        Some(&other_sk),
        0x9999_8888_7777_6666,
    );
    let (code, _stdout, stderr) = run(
        bin,
        &packed.db,
        &pinned_str,
        &["plugin", "install", &path2.display().to_string(), "--yes"],
    );
    assert_ne!(code, 0);
    assert!(
        stderr.contains("9999888877776666") && stderr.contains("1111222233334444"),
        "the refusal names both keynums: {stderr}"
    );

    // a pack carrying EXECUTABLE content is refused naming the file
    let mut evil = pack_entries("evil-pack", "0.1.0");
    evil.push((
        "payload.wasm".into(),
        vec![0x00, 0x61, 0x73, 0x6D, 1, 0, 0, 0],
    ));
    let path3 = write_pack(&packed, "evil.tar.gz", &evil, None, packed.key_id);
    let (code, _stdout, stderr) = run(
        bin,
        &packed.db,
        &pinned_str,
        &["plugin", "install", &path3.display().to_string(), "--yes"],
    );
    assert_ne!(code, 0);
    assert!(
        stderr.contains("payload.wasm") && stderr.contains("executable"),
        "the refusal names the file: {stderr}"
    );
}

#[test]
fn a_verbatim_reinstall_is_refused_and_an_upgrade_moves_the_register() {
    let bin = env!("CARGO_BIN_EXE_brain");
    let packed = setup("reinstall");
    let pinned_file = packed.dir.join("pack.pub");
    std::fs::write(&pinned_file, &packed.pinned).expect("pin the pack key");
    let pinned_str = pinned_file.display().to_string();

    let entries = pack_entries("support-pack", "0.1.0");
    let path = write_pack(&packed, "pack.tar.gz", &entries, None, packed.key_id);
    let path_str = path.display().to_string();
    let (code, _s, stderr) = run(
        bin,
        &packed.db,
        &pinned_str,
        &["plugin", "install", &path_str, "--yes"],
    );
    assert_eq!(code, 0, "{stderr}");

    // the re-install law (CHOSEN, recorded): same id+version REFUSED —
    // idempotent success is how a tampered re-offer would ride in
    let (code, _stdout, stderr) = run(
        bin,
        &packed.db,
        &pinned_str,
        &["plugin", "install", &path_str, "--yes"],
    );
    assert_ne!(code, 0);
    assert!(stderr.contains("ALREADY INSTALLED"), "{stderr}");
    assert_eq!(counts(&packed.db).0, 1, "still one register row");

    // a higher version upgrades the same register row
    let entries2 = pack_entries("support-pack", "0.2.0");
    let path2 = write_pack(&packed, "pack2.tar.gz", &entries2, None, packed.key_id);
    let (code, stdout, stderr) = run(
        bin,
        &packed.db,
        &pinned_str,
        &["plugin", "install", &path2.display().to_string(), "--yes"],
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("rows landed"), "{stdout}");
    let (register, rules, _plugin, _role) = counts(&packed.db);
    assert_eq!(register, 1, "one living row per pack");
    assert_eq!(rules, 2, "the unchanged rules replay-skip (the C1 law)");
}

#[test]
fn the_two_key_law_holds_at_the_pin_location_too() {
    // the pack key's pin is BRAIN_PACK_PUBKEY (or pack.pub beside the data
    // root) — never the updater's BRAIN_RELEASE_PUBKEY post. The env var
    // the binary reads is the PACK one; a release-pubkey env alone leaves
    // the install refused with the unpinned-pack-key message.
    let bin = env!("CARGO_BIN_EXE_brain");
    let packed = setup("pinlaw");
    let entries = pack_entries("support-pack", "0.1.0");
    let path = write_pack(&packed, "pack.tar.gz", &entries, None, packed.key_id);
    let out = Command::new(bin)
        .args([
            "plugin",
            "install",
            path.display().to_string().as_str(),
            "--yes",
        ])
        .arg("--db")
        .arg(&packed.db)
        .env("BRAIN_RELEASE_PUBKEY", packed.dir.join("release.pub")) // the UPDATER's var, set
        .env_remove("BRAIN_PACK_PUBKEY") // the PACK pin, absent
        .output()
        .expect("the brain binary runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_ne!(out.status.code(), Some(0));
    assert!(
        stderr.contains("not pinned") || stderr.contains("pack key"),
        "the updater's env var does not pin a pack key: {stderr}"
    );
}
