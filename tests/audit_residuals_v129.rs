//! Security-audit repro tests for brain-server v1.29.11 (current main).
//!
//! These document RESIDUALS found by the 2026-10-11 audit — behaviors that
//! still hold at this version and (where marked FINDING) should be fixed.
//! Controls are included alongside each finding so the pair reads as
//! "what works / what still doesn't".

use brain_server::audit;
use rusqlite::Connection;

// ─────────────────────────────────────────────────────────────────────────
// Chain tamper-evidence at v1.29.11 (hmac256 epoch): controls + residuals
// ─────────────────────────────────────────────────────────────────────────

fn db_hmac() -> Connection {
    use std::sync::Once;
    static KEY_INIT: Once = Once::new();
    KEY_INIT.call_once(|| {
        // Env-free: init_chain_key reads (or generates + writes 0600)
        // `<dir>/audit-chain.key`. First test generates; the rest read it.
        let dir = std::env::temp_dir().join("audit-residual-v129-keydir");
        let _ = std::fs::create_dir_all(&dir);
        audit::init_chain_key(&dir).expect("install test chain key");
    });
    let db = Connection::open_in_memory().expect("db");
    db.execute_batch(
        "CREATE TABLE audit_events(
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           ts TEXT DEFAULT CURRENT_TIMESTAMP,
           kind TEXT NOT NULL,
           actor TEXT,
           target_hash TEXT,
           status TEXT,
           detail_hash TEXT,
           tenant_id TEXT NOT NULL DEFAULT 'global',
           prev_hash TEXT);
         CREATE TABLE schema_meta(key TEXT PRIMARY KEY, value TEXT);",
    )
    .expect("schema");
    // Row-less fresh DB + installed key → stamps the hmac256 epoch.
    assert!(
        audit::bootstrap_epoch(&db),
        "test DB must boot into the hmac256 epoch"
    );
    db
}

fn record_n(db: &Connection, n: usize) {
    for i in 0..n {
        audit::record_tenant(
            db,
            audit::AuditKind::Ingest,
            &format!("actor-{i}"),
            &format!("target-{i}"),
            if i % 2 == 0 {
                audit::AuditStatus::Ok
            } else {
                audit::AuditStatus::Denied
            },
            &format!("detail-{i}"),
            "tenant-a",
        )
        .expect("record");
    }
}

/// CONTROL (the v1.15.0 F2 gaps are closed at this epoch): status/detail_hash
/// tamper IS detected now — the hmac256 link commits the full 8 fields.
#[test]
fn control_status_and_detail_tamper_detected_hmac256() {
    let db = db_hmac();
    record_n(&db, 4);
    assert!(audit::verify_chain(&db), "baseline verifies");
    db.execute(
        "UPDATE audit_events SET status='ok', detail_hash='0' WHERE id=2",
        [],
    )
    .unwrap();
    assert!(
        !audit::verify_chain(&db),
        "v1.29.11: covered-field tamper IS detected under hmac256"
    );
}

/// CONTROL: tail truncation IS detected via the head pin.
#[test]
fn control_tail_truncation_detected_by_pin() {
    let db = db_hmac();
    record_n(&db, 6);
    assert!(audit::verify_chain(&db));
    db.execute("DELETE FROM audit_events WHERE id > 4", [])
        .unwrap();
    assert!(
        !audit::verify_chain(&db),
        "tail truncation trips the head pin"
    );
}

/// CONTROL: full wipe with the pin surviving IS detected.
#[test]
fn control_full_wipe_with_pin_detected() {
    let db = db_hmac();
    record_n(&db, 5);
    assert!(audit::verify_chain(&db));
    db.execute("DELETE FROM audit_events", []).unwrap();
    assert!(
        !audit::verify_chain(&db),
        "empty chain under a live pin = full-truncation detection"
    );
}

/// REGRESSION (was the head-deletion finding): deleting the OLDEST rows is
/// DETECTED by the genesis pin — the pin anchors the chain's first row, so a
/// prefix deletion changes which row is first and the pinned id disagrees.
/// Both pins carry MACs under the keyed epoch, so rewriting the genesis to
/// match the truncated chain is behind the chain key (outside the DB).
#[test]
fn head_deletion_is_detected_by_genesis_pin() {
    let db = db_hmac();
    record_n(&db, 6);
    assert!(audit::verify_chain(&db));
    db.execute("DELETE FROM audit_events WHERE id <= 3", [])
        .unwrap();
    assert!(
        !audit::verify_chain(&db),
        "oldest-rows deletion must trip the genesis pin"
    );
}

/// REGRESSION (the pin-forgery escalation of the head-deletion finding): an
/// attacker deletes the oldest rows AND rewrites the genesis pin JSON to the
/// new oldest row. Without pin MACs this blessed the truncation; with them
/// the rewrite is invalid because the MAC key lives outside the DB.
#[test]
fn head_deletion_with_genesis_rewrite_is_detected() {
    let db = db_hmac();
    record_n(&db, 6);
    assert!(audit::verify_chain(&db));
    db.execute("DELETE FROM audit_events WHERE id <= 3", [])
        .unwrap();
    // Forge: re-point the genesis pin at the new first row (id 4) with the
    // correct hash but NO valid MAC — the attacker cannot compute one.
    let forged = format!(
        "{{\"id\":4,\"hash\":\"{}\",\"epoch\":\"hmac256\",\"mac\":\"{}\"}}",
        "a".repeat(64),
        "b".repeat(64)
    );
    db.execute(
        "INSERT INTO schema_meta(key, value) VALUES ('audit_chain_genesis', ?1)
         ON CONFLICT(key) DO UPDATE SET value = ?1",
        [&forged],
    )
    .unwrap();
    assert!(
        !audit::verify_chain(&db),
        "a rewritten genesis pin without a valid MAC must fail verify"
    );
}

/// REGRESSION: deleting just the genesis PIN ROW (leaving rows intact) must
/// fail on a keyed chain — every legitimate write path keeps both pins, so
/// absence on a live chain is removal, not "never stamped".
#[test]
fn genesis_pin_row_deletion_is_detected() {
    let db = db_hmac();
    record_n(&db, 4);
    assert!(audit::verify_chain(&db));
    db.execute(
        "DELETE FROM schema_meta WHERE key = 'audit_chain_genesis'",
        [],
    )
    .unwrap();
    assert!(
        !audit::verify_chain(&db),
        "a missing genesis pin on a non-empty keyed chain must fail verify"
    );
}

/// CEILING (documented, not a defect): a full wipe that ALSO drops both pin
/// rows reads as a fresh DB — destroying every in-DB marker is
/// indistinguishable from never having had one, in any design that keeps
/// its evidence only in the guarded store. The compensating control is the
/// OFF-HOST `brain anchor` fingerprint (and the chain key file itself,
/// which the attacker needs and does not have). If this test ever fails,
/// an external witness exists — update THREAT_MODEL §2b accordingly.
#[test]
fn ceiling_full_wipe_with_all_pins_deleted_reads_as_fresh() {
    let db = db_hmac();
    record_n(&db, 5);
    assert!(audit::verify_chain(&db));
    db.execute("DELETE FROM audit_events", []).unwrap();
    db.execute("DELETE FROM schema_meta WHERE key LIKE 'audit_chain%'", [])
        .unwrap();
    assert!(
        audit::verify_chain(&db),
        "CEILING: total destruction of all in-DB state == fresh install; \
         the off-host anchor is the detection layer"
    );
}

/// CONTROL: the retention prune's anti-laundering guard refuses to prune a
/// chain that does not verify (a tampered chain is preserved for forensics).
#[test]
fn control_prune_refuses_broken_chain() {
    let db = db_hmac();
    record_n(&db, 4);
    // Age every row past a 1-day window so the prune has work to do.
    db.execute(
        "UPDATE audit_events SET ts = datetime('now', '-40 days')",
        [],
    )
    .unwrap();
    // Tamper a covered field first.
    db.execute("UPDATE audit_events SET actor='rewritten' WHERE id=2", [])
        .unwrap();
    assert!(!audit::verify_chain(&db));
    let out = audit::prune_audit_retention(&db, 1);
    assert_eq!(
        out, None,
        "prune must refuse (None) a chain that fails verify"
    );
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 4, "rows preserved verbatim for forensics");
}

/// CONTROL (C15 fix): audit::hash is now keyed SHA-256 — no, plain SHA-256
/// over the input — unsalted but full-strength (256-bit, not the v1.15.0
/// 64-bit xxh3). A dictionary attack against a short query is still
/// possible offline (no per-deployment salt), so the residual is the same
/// one SHA-256 password hashing carries without a salt; the trivial
/// 2^64 brute force of v1.15.0 is gone.
#[test]
fn control_audit_hash_is_sha256_not_xxh3() {
    let h = audit::hash("what is alice's salary band");
    assert_eq!(h.len(), 64, "256-bit digest rendered as 64 hex chars");
    assert_eq!(h, audit::hash("what is alice's salary band"));
    assert_ne!(h, audit::hash("what is alice's salary bands"));
}

// ─────────────────────────────────────────────────────────────────────────
// dsar_locate multi-root attribution (carried v1.15.0 finding F11)
// ─────────────────────────────────────────────────────────────────────────

/// REGRESSION (was the multi-root attribution finding): every `derived_from`
/// descendant is attributed to the ROOT IT DESCENDS FROM — a subject owning
/// rows 10 and 20 has 11 attributed to 10 and 21 attributed to 20, so the
/// tombstone `origin_id` in the deletion registry points at the true origin.
#[test]
fn dsar_locate_attributes_each_derived_to_its_true_root() {
    let mut db = Connection::open_in_memory().expect("db");
    db.execute_batch(
        "CREATE TABLE knowledge(id INTEGER PRIMARY KEY, owner TEXT);
         CREATE TABLE evidence_links(
           id INTEGER PRIMARY KEY,
           kind TEXT,
           from_chunk INTEGER,
           to_chunk INTEGER);",
    )
    .expect("schema");
    // Two roots owned by the subject: 10 and 20. 11 derives from 10;
    // 21 derives from 20.
    db.execute_batch(
        "INSERT INTO knowledge(id, owner) VALUES (10,'s'), (20,'s');
         INSERT INTO evidence_links(kind, from_chunk, to_chunk)
           VALUES ('derived_from', 10, 11), ('derived_from', 20, 21),
                  ('derived_from', 11, 12);",
    )
    .expect("seed");
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let (roots, derived) = brain_server::service::dsar::dsar_locate(&tx, "s").unwrap();
    tx.commit().unwrap();
    assert_eq!(roots, vec![10, 20], "both roots located");
    assert!(
        derived.contains(&(11, 10)) && derived.contains(&(21, 20)),
        "each descendant attributed to its true root — got {derived:?}"
    );
    assert!(
        !derived.contains(&(21, 10)),
        "no cross-root misattribution — got {derived:?}"
    );
    // Transitive depth: 12 derives from 11 (depth 2 from root 10) — seeded
    // below — must carry root 10, not its intermediate parent 11.
    assert!(
        derived.contains(&(12, 10)),
        "transitive descendants carry the ROOT, not the intermediate hop — got {derived:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Record read gate (the F3 fix) — pin the owner binding
// ─────────────────────────────────────────────────────────────────────────

// NOTE: `record_read_gate` lives in the server binary's handlers (not the
// lib), so its owner binding is pinned by the live drill transcripts
// instead (scratch/drill_v129-transcript.txt: userA denied on every surface
// — /recall, /search, /suggest empty; /get + /verify 404 probe-blind).
#[test]
fn control_audit_kind_vocabulary_exists() {
    // Keeps this file honest: the lib-facing audit surface this file exercises
    // compiles and the kinds used above are real variants.
    let k = audit::AuditKind::Ingest;
    assert!(format!("{k:?}").contains("Ingest"));
}
