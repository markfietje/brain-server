//! Crate tests for the curated legal DB: tempfile-DB, spawn-free. The
//! three-way consistency law (seed ↔ SDK table ↔ transfers register) is
//! pinned here together with the kernel's existing transfers ↔ SDK test;
//! the SDK table is the single owner and the join key. Byte-level response
//! reproducibility is pinned at the route layer (the server serializes);
//! this layer pins struct-level determinism, ordering, and the named laws.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A unique temp file per test — no tempfile dependency, no spawns.
fn temp_db(tag: &str) -> std::path::PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "legal-rules-db-{}-{}-{}.db",
        tag,
        std::process::id(),
        n
    ))
}

struct TempDb(std::path::PathBuf);

impl TempDb {
    fn new(tag: &str) -> Self {
        Self(temp_db(tag))
    }
    fn seeded(&self) -> Connection {
        let conn = Connection::open(&self.0).expect("open writable");
        seed(&conn, "dpo-test", 1_700_000_000).expect("seed");
        conn
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The stored list encoder: exact bytes for the curated identifiers, and a
/// strict round-trip for escaped content.
#[test]
fn the_list_encoder_round_trips_and_pins_its_bytes() {
    let plain = vec!["access".to_string(), "erasure".to_string()];
    assert_eq!(json_string_array(&plain), "[\"access\",\"erasure\"]");
    assert_eq!(
        parse_json_string_array("[\"access\",\"erasure\"]").unwrap(),
        plain
    );
    let tricky = vec![
        "quote\"inside".to_string(),
        "back\\slash".to_string(),
        "new\nline".to_string(),
        "tab\tstop".to_string(),
    ];
    let encoded = json_string_array(&tricky);
    assert_eq!(parse_json_string_array(&encoded).unwrap(), tricky);
    assert_eq!(parse_json_string_array("[]").unwrap(), Vec::<String>::new());
    assert!(parse_json_string_array("[not,a,string,array]").is_err());
    assert!(parse_json_string_array("[\"unterminated]").is_err());
}

/// The three-way law: every seeded law version IS an SDK label (no extras,
/// none missing), and the DSAR mirror matches the transfers register's
/// curated rows exactly (the kernel's own test pins transfers ↔ SDK).
#[test]
fn seed_matches_the_sdk_table_and_the_transfers_mirror() {
    let db = TempDb::new("three-way");
    let conn = db.seeded();
    let mut labels: Vec<(String, String)> = conn
        .prepare("SELECT jurisdiction, version FROM law_version ORDER BY jurisdiction")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut sdk: Vec<(String, String)> = brain_engine_sdk::policy::LAW_VERSIONS
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    sdk.sort();
    labels.sort();
    assert_eq!(labels, sdk, "seed ↔ SDK law-version table must be exact");

    // The DSAR mirror: (jurisdiction, body=law, deadline_days, rights) per
    // the transfers register's curated table.
    let eu = brain_engine_sdk::policy::law_version_for("eu").unwrap();
    assert_eq!(eu, "gdpr-consolidated-2021");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM jurisdiction_rules WHERE subject = 'dsar'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 7, "the transfers register's seven curated DSAR rows");
    let us_deadline: Option<i64> = conn
        .query_row(
            "SELECT deadline_days FROM jurisdiction_rules WHERE jurisdiction = 'us' AND subject = 'dsar'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        us_deadline,
        Some(45),
        "CCPA's 45-day DSAR deadline, verbatim"
    );
    let us_rights: String = conn
        .query_row(
            "SELECT rights FROM jurisdiction_rules WHERE jurisdiction = 'us' AND subject = 'dsar'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        us_rights,
        "[\"access\",\"erasure\",\"rectification\",\"opt-out-of-sale\"]"
    );
    let ph_deadline: Option<i64> = conn
        .query_row(
            "SELECT deadline_days FROM jurisdiction_rules WHERE jurisdiction = 'ph' AND subject = 'dsar'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        ph_deadline, None,
        "RA 10173 is 'reasonable' — no fixed days"
    );

    // The mechanism vocabulary: recorded, unattested, honestly empty.
    let mechanisms: Vec<String> = conn
        .prepare("SELECT mechanism FROM surveillance_postures ORDER BY mechanism")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        mechanisms,
        vec![
            "adequacy".to_string(),
            "bcr".to_string(),
            "cbpr".to_string(),
            "dpf-us".to_string(),
            "scc-eu-2021".to_string(),
            "uk-idta".to_string(),
        ]
    );
    let hk: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM surveillance_postures WHERE jurisdictions LIKE '%hk%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        hk, 0,
        "no HK posture exists in any curated table — none is invented here"
    );
    verify_head(&conn).expect("seed pins the head");
}

/// The head-pin law: the stored bytes are exactly this crate's deterministic
/// form; unpinned drift is a NAMED mismatch; re-pinning after an import
/// heals it.
#[test]
fn head_pin_detects_drift_and_repin_heals() {
    let db = TempDb::new("head-pin");
    let conn = db.seeded();
    let pinned = stored_head(&conn).unwrap().unwrap();
    let snap = head_snapshot(&conn).unwrap();
    assert_eq!(
        pinned,
        format!(
            "{{\"rules\":{},\"versions\":{},\"postures\":{},\"max_rules_id\":{}}}",
            snap.rules, snap.versions, snap.postures, snap.max_rules_id
        )
    );
    // An import: one new rule arrives with a real effective date...
    let rule = crate::Rule {
        id: 0,
        law_version: brain_engine_sdk::policy::law_version_for("eu")
            .unwrap()
            .to_string(),
        jurisdiction: "eu".to_string(),
        subject: "vat".to_string(),
        rule_key: "eu_vat_digital".to_string(),
        body: "Digital services VAT".to_string(),
        source_ref: "EU VAT Directive".to_string(),
        effective_at: 1_800_000_000,
        reviewed_at: Some(1_800_000_000),
        expires_at: None,
        revision: 1,
        superseded_by: None,
        created_at: 1_800_000_000,
    };
    insert_jurisdiction_rule(&conn, &rule, None, &[]).unwrap();
    // ...and a reviewer who forgot to re-pin: named mismatch, never silence.
    match verify_head(&conn) {
        Err(LegalDbError::HeadPinMismatch { .. }) => {}
        other => panic!("expected a named head-pin mismatch, got {other:?}"),
    }
    // The import path re-pins and the file is consistent again.
    pin_head(&conn).unwrap();
    verify_head(&conn).expect("re-pin heals the mismatch");
}

/// Reseeding an already-seeded file is a named refusal — imports add rows and
/// re-pin; they never silently duplicate the seed.
#[test]
fn reseeding_an_imported_file_is_a_named_refusal() {
    let db = TempDb::new("reseed");
    {
        let conn = Connection::open(&db.0).unwrap();
        seed(&conn, "dpo-test", 1_700_000_000).unwrap();
    }
    let conn = Connection::open(&db.0).unwrap();
    assert_eq!(
        seed(&conn, "dpo-test", 1_700_000_100),
        Err(LegalDbError::AlreadySeeded)
    );
}

/// The FTS index answers body/source-ref MATCH queries over the seeded rules.
#[test]
fn fts_index_matches_seeded_rule_bodies() {
    let db = TempDb::new("fts");
    let conn = db.seeded();
    let hits = search_rules(&conn, "GDPR").expect("fts query");
    assert!(!hits.is_empty(), "the EU/UK rule bodies cite GDPR");
    let eu_id: i64 = conn
        .query_row(
            "SELECT id FROM jurisdiction_rules WHERE jurisdiction = 'eu' AND subject = 'dsar'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(hits.contains(&eu_id));
    let hits = search_rules(&conn, "CCPA").expect("fts query");
    assert!(!hits.is_empty(), "the US rule body cites CCPA/CPRA");
}

/// The C2 contract, pinned at the pure layer: same DB state → struct-identical
/// diff, twice; ordered by (jurisdiction, subject, effective_at, revision,
/// id); unknown `since` refuses NAMED. (Byte-level response reproducibility
/// is the route layer's pin — the server serializes.)
#[test]
fn law_version_diff_is_reproducible() {
    let db = TempDb::new("diff");
    let conn = db.seeded();
    // A newer revision exists to be diffed into view.
    let rule = crate::Rule {
        id: 0,
        law_version: brain_engine_sdk::policy::law_version_for("nl")
            .unwrap()
            .to_string(),
        jurisdiction: "nl".to_string(),
        subject: "vat".to_string(),
        rule_key: "nl_vat_standard".to_string(),
        body: "Standard VAT rate, revised".to_string(),
        source_ref: "Wet OB 1968".to_string(),
        effective_at: 1_706_745_600,
        reviewed_at: Some(1_706_745_600),
        expires_at: None,
        revision: 2,
        superseded_by: None,
        created_at: 1_706_745_600,
    };
    insert_jurisdiction_rule(&conn, &rule, None, &[]).unwrap();

    let a = diff_since(&conn, Some("wet-ob-1968-rev2024")).expect("diff");
    let b = diff_since(&conn, Some("wet-ob-1968-rev2024")).expect("diff");
    assert_eq!(a, b, "same since, identical response value, twice");
    assert_eq!(a.since.as_deref(), Some("wet-ob-1968-rev2024"));
    assert_eq!(
        a.since_effective_at,
        Some(0),
        "the seeded SDK rows carry no curated date yet"
    );
    // Newer than the snapshot point (0): the crate's two dated VAT rules AND
    // the revised NL rule. The zero-date DSAR rows stay out (0 > 0 is false).
    assert_eq!(a.rules.len(), 3);
    assert!(
        a.rules.iter().all(|r| r.subject == "vat"),
        "the zero-date DSAR snapshot rows are not 'newer' than a zero-date pin"
    );
    let revised: Vec<&RuleRow> = a.rules.iter().filter(|r| r.revision == 2).collect();
    assert_eq!(revised.len(), 1);
    assert_eq!(revised[0].rule_key, "nl_vat_standard");
    assert_eq!(revised[0].effective_at, 1_706_745_600);
    assert_eq!(
        a.head,
        head_snapshot(&conn).unwrap(),
        "the diff carries the live head"
    );

    // Unknown since → named refusal.
    assert_eq!(
        diff_since(&conn, Some("not-a-version")).unwrap_err(),
        LegalDbError::UnknownLawVersion("not-a-version".to_string())
    );

    // Absent since → the full ordered snapshot; deterministic too.
    let full_a = diff_since(&conn, None).unwrap();
    let full_b = diff_since(&conn, None).unwrap();
    assert_eq!(full_a, full_b);
    let keys: Vec<(String, String, i64, i64)> = full_a
        .rules
        .iter()
        .map(|r| {
            (
                r.jurisdiction.clone(),
                r.subject.clone(),
                r.effective_at,
                r.revision,
            )
        })
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "the full snapshot is (jurisdiction, subject, effective_at, revision)-ordered"
    );
}

/// The read-by-server law: a read-only connection cannot write the file.
#[test]
fn readonly_connection_refuses_writes() {
    let db = TempDb::new("readonly");
    {
        let conn = Connection::open(&db.0).unwrap();
        seed(&conn, "dpo-test", 1_700_000_000).unwrap();
    }
    let ro = open_readonly(&db.0).expect("read-only open");
    let write = ro.execute(
        "INSERT INTO law_version(jurisdiction, version) VALUES ('zz', 'invented')",
        [],
    );
    assert!(write.is_err(), "the server-facing connection is read-only");
    verify_head(&ro).expect("reads and verification work read-only");
}
