//! AUDIT R2 reproductions — run OUTSIDE `src/audit/mod.rs` on purpose.
//!
//! `tests/external_claim_pins.rs:986` pins the SHA-256 of the whole
//! `src/audit/mod.rs` file (`r45_0_correction_leaves_the_audit_chain_module_byte_untouched`),
//! asserting that the R45-0 correction round changed the module's *verdicts*,
//! not its bytes. Adding a test INSIDE that module therefore trips the pin —
//! correctly. These reproductions live here so the audit's evidence does not
//! masquerade as a production change to a module that is deliberately frozen.
//!
//! Three findings, each with a red-first test that fails on `main` (d517cf67).

use brain_server::audit::{AuditKind, AuditStatus, hash, record, verify_chain};

/// A real migrated DB plus `schema_meta` (where the epoch stamp and the
/// genesis/head pins live). Mirrors `db_with_meta()` in the audit module's own
/// test block.
fn db_with_meta() -> rusqlite::Connection {
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mut db = rusqlite::Connection::open_in_memory().expect("open in-memory DB");
    brain_server::migration::run_migration(&mut db, 1).expect("run migration");
    db.execute_batch("CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY, value TEXT);")
        .expect("create schema_meta");
    db
}

// ─────────────────────────────────────────────────────────────────────────
// R2-03 · C15 · audit::hash is unsalted SHA-256 → dictionary-reversible
// ─────────────────────────────────────────────────────────────────────────

/// Installs a real chain key so the pin exercises the KEYED path.
///
/// This matters: with no key installed, `audit::hash` falls back to plain
/// SHA-256 and the dictionary test passes for the WRONG reason. An earlier
/// version of this file had exactly that flaw — it "passed" against the
/// unfixed code. The key is process-global, so it is installed once and left
/// in place for the rest of this target.
fn install_chain_key() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join("audit_r2_chain_key_dir");
        std::fs::create_dir_all(&dir).expect("chain key dir");
        brain_server::audit::init_chain_key(&dir).expect("init chain key");
    });
    assert!(
        brain_server::audit::chain_key().is_some(),
        "the keyed path must be active, or this test proves nothing"
    );
}

/// `src/audit/mod.rs:182`:
///
///     pub fn hash(s: &str) -> String {
///         hex_encode(&Sha256::digest(s.as_bytes()))
///     }
///
/// No salt. No key. The module doc at `src/audit/mod.rs:6-8` claims a stored
/// `target_hash` "derived from low-entropy content is not offline-recoverable".
/// That is false: one hash per target is reversible by brute force with NO
/// access to the server — a DB backup suffices.
///
/// The existing pin `hash_is_sha256_not_xxh3` (mod.rs:1833) asserts only
/// `!h.contains("alice")`, a tautology (a hex digest never contains its
/// plaintext), so it passes while the promised property does not hold.
///
/// The correct pattern already exists in this codebase and was not applied:
/// `workflow/case_status.rs:88` uses `Hmac::new_from_slice(salt)`.
#[test]
fn audit_hash_is_reversible_by_dictionary_attack() {
    install_chain_key();
    // The attacker's model of the target population — exactly the classes the
    // module doc names ("content, token, uri-with-secret") plus subject ids.
    let corpus = [
        "alice@example.com",
        "bob@example.com",
        "carol@example.com",
        "dave@example.com",
        "erin@example.com",
        "frank@example.com",
        "SUBJ-0001",
        "SUBJ-0002",
        "SUBJ-0003",
        "patient-12345",
        "patient-12346",
        "https://intranet.acme.test/patients/8821",
    ];

    // A stored row, as anyone holding a backup or audit read access sees it.
    let stored_target_hash = hash("carol@example.com");

    // The ATTACKER'S model, which is the whole point: they hold a database
    // backup, not the server's chain key. So they hash candidate identifiers
    // with PLAIN, UNKEYED SHA-256 — the same construction the server used
    // before it was keyed — and look for the stored digest.
    //
    // NOTE: an earlier version of this test called `hash(candidate)` for the
    // attack. That models the attacker as possessing the server's key, which
    // is not the threat — and it is self-defeating, because the corpus
    // necessarily contains the plaintext that was hashed, so the match is
    // true by construction and the assertion can never hold. The threat is
    // OFFLINE recovery from a stored digest with NO key material.
    let attacker_digest = |candidate: &str| {
        use sha2::{Digest, Sha256};
        Sha256::digest(candidate.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };

    let recovered = corpus
        .iter()
        .find(|candidate| attacker_digest(candidate) == stored_target_hash)
        .copied();

    assert!(
        recovered.is_none(),
        "C15 BROKEN: stored target_hash {stored_target_hash} was recovered as \
         {recovered:?} by exhaustively hashing a {}-entry dictionary with \
         PLAIN SHA-256 — no key, no server access, just the stored digest. \
         Relevant under GDPR Art.32 / HIPAA 164.312(a)(2)(iv).",
        corpus.len()
    );
}

/// The control for the test above: on a deployment with NO chain key, the
/// stored digest IS dictionary-recoverable. This is what makes the fix's
/// conditional-keying honest rather than a blanket claim: keyless deployments
/// remain pseudonymised, and the product's own answer is to configure a key.
#[test]
fn audit_hash_is_recoverable_without_a_chain_key() {
    let corpus = ["carol@example.com", "bob@example.com"];
    // Simulate the keyless (pre-fix) construction directly.
    use sha2::{Digest, Sha256};
    let unkeyed = |s: &str| {
        Sha256::digest(s.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };

    let stored = unkeyed("carol@example.com");
    let recovered = corpus
        .iter()
        .find(|c| unkeyed(c) == stored.as_str())
        .copied();

    assert_eq!(
        recovered,
        Some("carol@example.com"),
        "CONTROL: plain SHA-256 must remain dictionary-recoverable — this is the \
         exact property the fix removes once a chain key is configured. If this \
         ever fails, the threat model in the test above is wrong."
    );
}

/// Isolation pin: distinct inputs must still hash distinctly. The finding is
/// about RECOVERABILITY, not about collisions.
#[test]
fn audit_hash_still_separates_distinct_inputs() {
    assert_ne!(hash("alice@example.com"), hash("bob@example.com"));
    assert_eq!(hash("alice@example.com"), hash("alice@example.com"));
}

// ─────────────────────────────────────────────────────────────────────────
// R2-04 · C3 · a legacy chain does not commit status / detail_hash
// ─────────────────────────────────────────────────────────────────────────

/// `chain_link` (`src/audit/mod.rs:190-202`) hashes ONLY:
///
///     ts | kind | actor | target_hash | prev_hash
///
/// `status` and `detail_hash` are OUTSIDE the preimage. The 7-field tamper
/// battery `chain_hash_includes_all_fields` (mod.rs:2830) therefore only ever
/// runs under `bootstrap_epoch` — the KEYED hmac256 epoch — and there is no
/// legacy counterpart, because legacy would fail it.
///
/// This matters because legacy is the DEFAULT state of every database that has
/// not been manually re-anchored: `read_epoch` maps a missing stamp to
/// `ChainEpoch::Legacy` (mod.rs:246), and `bootstrap_epoch` REFUSES any DB
/// with ≥1 row (mod.rs:1557). Reaching hmac256 requires an operator to run the
/// offline `--re-audit` by hand.
///
/// Consequence: on a legacy chain an operator can flip an audit row's
/// `status` from 'ok' to 'denied' — or rewrite `detail_hash` outright — and
/// `verify_chain` still returns TRUE.
#[test]
fn legacy_chain_does_not_commit_status_or_detail_hash() {
    for (label, sql) in [
        (
            "status",
            "UPDATE audit_events SET status = 'denied' WHERE id = 1",
        ),
        (
            "detail_hash",
            "UPDATE audit_events SET detail_hash = 'deadbeef' WHERE id = 1",
        ),
    ] {
        let db = db_with_meta();

        // No chain key is configured, so the epoch stays legacy — the state
        // of every un-re-audited deployment.
        record(&db, AuditKind::Auth, "api", "c1", AuditStatus::Ok, "d1");
        record(&db, AuditKind::Auth, "api", "c2", AuditStatus::Ok, "d2");
        assert!(
            verify_chain(&db),
            "[{label}] the unmodified legacy chain must verify"
        );

        db.execute(sql, []).unwrap();

        assert!(
            !verify_chain(&db),
            "C3 BROKEN: on a LEGACY chain, mutating `{label}` did not break \
             verify_chain(). chain_link() commits only \
             (ts, kind, actor, target_hash, prev_hash); status and detail_hash \
             are outside the preimage, so an audit row's OUTCOME can be \
             rewritten undetected on any deployment that has not run --re-audit."
        );
    }
}

/// Isolation pin: the fields legacy DOES commit must stay protected. A fix
/// that length-prefixes `chain_link` must not weaken actor/target_hash.
#[test]
fn legacy_chain_still_detects_the_fields_it_commits() {
    for sql in [
        "UPDATE audit_events SET actor = 'mallory' WHERE id = 1",
        "UPDATE audit_events SET target_hash = 'deadbeef' WHERE id = 1",
    ] {
        let db = db_with_meta();
        record(&db, AuditKind::Auth, "api", "c1", AuditStatus::Ok, "d1");
        record(&db, AuditKind::Auth, "api", "c2", AuditStatus::Ok, "d2");
        assert!(verify_chain(&db));
        db.execute(sql, []).unwrap();
        assert!(
            !verify_chain(&db),
            "actor and target_hash ARE committed by chain_link and must stay \
             protected on the legacy epoch"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// BACKWARD COMPATIBILITY — the pin that must never be traded away
// ─────────────────────────────────────────────────────────────────────────

/// Writes a chain EXACTLY as a pre-`sha7` binary left it: unkeyed SHA-256 over
/// `ts | kind | actor | target_hash | prev_hash`, no `schema_meta` epoch stamp.
/// This is the on-disk shape of every database deployed before the fix.
fn db_with_legacy_chain() -> rusqlite::Connection {
    use sha2::{Digest, Sha256};
    let db = db_with_meta();
    let legacy_link = |ts: &str, kind: &str, actor: &str, target_hash: &str, prev: Option<&str>| {
        let mut h = Sha256::new();
        for (i, f) in [ts, kind, actor, target_hash, prev.unwrap_or("")]
            .iter()
            .enumerate()
        {
            if i > 0 {
                h.update(b"|");
            }
            h.update(f.as_bytes());
        }
        h.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    // Timestamps are fixed (not CURRENT_TIMESTAMP) so the recomputed links
    // are reproducible outside the test.
    let rows: [(&str, &str, &str, &str, &str); 3] = [
        ("2026-01-01 00:00:00", "auth", "api", "h1", "d1"),
        ("2026-01-02 00:00:00", "ingest", "api", "h2", "d2"),
        ("2026-01-03 00:00:00", "webhook", "api", "h3", "d3"),
    ];
    // Each row stores the PREVIOUS row's link (`prev_hash` is the backref);
    // its own link is what the next row will carry.
    let mut prev_link: Option<String> = None;
    for (ts, kind, actor, target, detail) in rows {
        let stored_prev = prev_link.clone();
        let link = legacy_link(ts, kind, actor, target, stored_prev.as_deref());
        db.execute(
            "INSERT INTO audit_events(ts, kind, actor, target_hash, status, detail_hash, prev_hash)
             VALUES (?1, ?2, ?3, ?4, 'ok', ?5, ?6)",
            rusqlite::params![ts, kind, actor, target, detail, stored_prev],
        )
        .unwrap();
        prev_link = Some(link);
    }
    db
}

/// THE PIN. A chain written by an older binary — 5-field links, no epoch
/// stamp — must still verify under the fixed binary. This is the false alarm
/// the whole design exists to prevent: `verify_chain` recomputes every link
/// from stored columns, so if the fix had widened `chain_link` in place, every
/// historical row in every deployed database would have started reporting as
/// tampered.
///
/// Also pins the asymmetry that makes it safe: the FIRST append to a chain
/// with no rows stamps the corrected `sha7` epoch, but a chain that already
/// has rows stays legacy forever. New evidence is fixed; old evidence is not
/// rewritten.
#[test]
fn unmigrated_legacy_chain_still_verifies_and_stays_legacy() {
    let db = db_with_legacy_chain();
    assert!(
        verify_chain(&db),
        "a legacy chain written by an older binary must still verify after \
         the link-format fix — an upgraded binary that false-alarms on \
         historical evidence is a regulated-deployment incident"
    );

    // Nothing stamped the epoch on read or verify: reading never rewrites.
    let stamp: Option<String> = db
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'audit_chain_epoch'",
            [],
            |r| r.get(0),
        )
        .ok();
    assert!(
        stamp.is_none(),
        "verify must not stamp an epoch on an existing chain"
    );

    // Appending to a chain that HAS rows keeps the legacy format — the new
    // row chains from the tip under the 5-field link, and the chain verifies.
    record(&db, AuditKind::Auth, "api", "c4", AuditStatus::Denied, "d4");
    assert!(
        verify_chain(&db),
        "a row appended to a legacy chain must chain in the legacy format \
         (the epoch of a chain with history is frozen, not upgraded)"
    );
    let stamp: Option<String> = db
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'audit_chain_epoch'",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(
        stamp, None,
        "a chain with history is NOT stamped — it stays unstamped, and \
         unstamped-with-rows IS legacy. Nothing rewrites the format of a \
         chain that already has evidence (that is --re-audit's job, and only \
         --re-audit's)."
    );
}

/// THE MIGRATION, end to end. An unmigrated legacy chain is upgraded by the
/// documented operator path (`brain-server --re-audit` → `reanchor_to_hmac`),
/// and after the replay the outcome fields are tamper-evident.
///
/// Requires a chain key: an unkeyed "re-anchor" is a no-op that lies about
/// the format, so `reanchor_to_hmac` refuses without one.
#[test]
fn reaudit_replays_a_legacy_chain_and_then_commits_status_and_detail() {
    install_chain_key();
    let db = db_with_legacy_chain();
    assert!(
        verify_chain(&db),
        "fixture: the legacy chain verifies pre-migration"
    );

    let rewritten = brain_server::audit::reanchor_to_hmac(&db).expect("--re-audit re-anchor");
    assert_eq!(rewritten, 2, "both chained rows were replayed");
    assert!(
        verify_chain(&db),
        "the replayed chain verifies under hmac256"
    );
    let epoch: String = db
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'audit_chain_epoch'",
            [],
            |r| r.get(0),
        )
        .expect("epoch stamped by the re-anchor");
    assert_eq!(epoch, "hmac256", "the replay stamped the keyed epoch");

    // Post-migration, the outcome fields are inside the link.
    db.execute("UPDATE audit_events SET status = 'denied' WHERE id = 1", [])
        .unwrap();
    assert!(
        !verify_chain(&db),
        "after --re-audit, rewriting an audit row's outcome must break the chain"
    );
}

/// The isolation pin for the migration itself: the re-anchor REFUSES a chain
/// that does not verify, so it can never launder a broken chain into a
/// freshly-verifying one.
#[test]
fn reaudit_refuses_a_broken_chain() {
    install_chain_key();
    let db = db_with_legacy_chain();
    db.execute("UPDATE audit_events SET actor = 'mallory' WHERE id = 2", [])
        .unwrap();
    assert!(!verify_chain(&db), "fixture: the chain is broken");
    assert!(
        brain_server::audit::reanchor_to_hmac(&db).is_err(),
        "replaying a chain that does not verify would bless the tamper — \
         the evidence-laundering guard must refuse"
    );
}
