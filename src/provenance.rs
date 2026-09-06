//! Provenance marks — the Art 50(2) posture for engine-generated TEXT
//! artifacts that leave a boundary (Enterprise Line M5, v1.28.62).
//!
//! Every artifact carries one machine-readable object:
//!
//! ```json
//! "provenance": {
//!     "mark": "AIGEN",
//!     "generator": "brain-server/1.28.62",
//!     "generated_at": 1790000000,
//!     "signed_by": "did:key:z…",
//!     "sig": "<ed25519 hex over the canonical body bytes>"
//! }
//! ```
//!
//! The signature is the parcels/standby convention via the SHARED
//! [`crate::ump_integrity::sign_manifest_bytes`]: Ed25519 over the lowercase
//! hex SHA-256 STRING of the canonical wrapper bytes — the wrapper binds the
//! artifact body (WITHOUT its `provenance` field) to the mark CLAIM (mark,
//! generator, generated_at, actor) in one canonical object, so flipping the
//! mark breaks verification exactly like flipping a body byte. Canonical
//! form is [`crate::ump_integrity::canonical_ump`] (the RFC 8785 flavor that
//! reproduces the UMP reference `contentHash`; the artifacts here carry
//! only ints/strings, so every canonicalization quirk is out of range). One
//! helper, one verify: tamper anything and the mark refuses.
//!
//! **Honest scope (the `ponytail:` boundary):** this is marking for TEXT
//! artifacts — JSON fields riding existing envelopes. It is NOT C2PA, not
//! media signing, and makes no manifest-store claim anywhere. Human-authored
//! artifacts mark `HUMAN` with the actor principal in `signed_by` (and a
//! signature when the operator key resolves, binding actor + bytes).
//!
//! Degradation is VISIBLE, never silent: without an operator key the mark is
//! still present (`mark`/`generator`/`generated_at` always) but `signed_by`
//! and `sig` are `null` and [`verify_artifact`] fails — an unsigned mark reads as
//! unverified, the L2 hash-only posture the UMP already models. The
//! deliverable pin (`provenance_marks_present_on_all_four_classes`) proves
//! the SIGNED form on all four classes; absence of a key is a deployment
//! choice the operator sees in every artifact.

use serde_json::Value;

/// The machine-generated mark (AI Act Art 50(2) posture).
pub const MARK_AIGEN: &str = "AIGEN";

/// The human-authored mark: `signed_by` carries the actor principal.
pub const MARK_HUMAN: &str = "HUMAN";

/// The field every marked artifact rides.
pub const FIELD: &str = "provenance";

/// The generator stamp: `brain-server/<version>` — CARGO_PKG_VERSION at
/// compile time, exactly what `/version` serves.
pub fn generator() -> String {
    format!("brain-server/{}", env!("CARGO_PKG_VERSION"))
}

/// The signed message: the canonical bytes of a wrapper binding the body to
/// its mark claim — `{"artifact": <body minus provenance>, "claim": {mark,
/// generator, generated_at, actor}}`. The claim rides INSIDE the signature,
/// so flipping the mark (or the generator stamp, or the timestamp) breaks
/// verification exactly like flipping a body byte does. `sig`/`signed_by`
/// stay outside (they ARE the signature envelope).
fn signed_message(value: &Value, claim: &Value) -> Result<Vec<u8>, String> {
    let mut body = value.clone();
    if let Some(obj) = body.as_object_mut() {
        obj.remove(FIELD);
    }
    crate::ump_integrity::canonical_ump(&serde_json::json!({
        "artifact": body,
        "claim": claim,
    }))
}

/// The claim a provenance object carries: everything the mark asserts,
/// everything the signature binds.
fn claim_of(p: &Value) -> Value {
    serde_json::json!({
        "mark": p.get("mark"),
        "generator": p.get("generator"),
        "generated_at": p.get("generated_at"),
        "actor": p.get("actor"),
    })
}

/// Attach a signed `AIGEN` mark to `value` in place. Returns `true` when the
/// operator key resolved and the mark carries a signature; `false` when the
/// mark is present but UNSIGNED (no key configured — the visible
/// degradation documented in the module doc). Idempotent: any pre-existing
/// provenance field is replaced (the mark always describes the FINAL body).
pub fn attach_aigen(value: &mut Value, now: i64) -> bool {
    attach_marked(value, MARK_AIGEN, None, now)
}

/// Attach a `HUMAN` mark naming the actor principal. Same signature rule as
/// [`attach_aigen`]: signed when the operator key resolves, visibly unsigned
/// otherwise.
pub fn attach_human(value: &mut Value, actor: &str, now: i64) -> bool {
    attach_marked(value, MARK_HUMAN, Some(actor), now)
}

fn attach_marked(value: &mut Value, mark: &str, actor: Option<&str>, now: i64) -> bool {
    let claim = serde_json::json!({
        "mark": mark,
        "generator": generator(),
        "generated_at": now,
        "actor": actor,
    });
    let Some((_, sk)) = crate::handlers::ump::operator_signing_key() else {
        value[FIELD] = unsigned_mark(mark, actor, now);
        return false;
    };
    let message = match signed_message(value, &claim) {
        Ok(m) => m,
        // A non-canonicalizable body (non-finite float) gets an unsigned
        // mark — the artifact still declares itself, honestly unverified.
        Err(_) => {
            value[FIELD] = unsigned_mark(mark, actor, now);
            return false;
        }
    };
    let (sig_hex, signed_by) = crate::ump_integrity::sign_manifest_bytes(&sk, &message);
    value[FIELD] = serde_json::json!({
        "mark": mark,
        "generator": generator(),
        "generated_at": now,
        "signed_by": signed_by,
        "sig": sig_hex,
        "actor": actor,
    });
    true
}

fn unsigned_mark(mark: &str, actor: Option<&str>, now: i64) -> Value {
    serde_json::json!({
        "mark": mark,
        "generator": generator(),
        "generated_at": now,
        "signed_by": actor,
        "sig": Value::Null,
    })
}

/// Fail-closed verification of a marked artifact: the provenance object must
/// be present and fully shaped, and its signature must verify over the
/// canonical bytes of the claim-bound wrapper (body minus the mark + the
/// claim itself — see [`signed_message`]). Any malformation, missing field,
/// flipped mark, or one flipped byte anywhere in the body → `false` (never
/// errors).
pub fn verify_artifact(value: &Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    let Some(p) = obj.get(FIELD) else {
        return false;
    };
    let Some(mark) = p.get("mark").and_then(|v| v.as_str()) else {
        return false;
    };
    if mark != MARK_AIGEN && mark != MARK_HUMAN {
        return false;
    }
    let (Some(signed_by), Some(sig)) = (
        p.get("signed_by").and_then(|v| v.as_str()),
        p.get("sig").and_then(|v| v.as_str()),
    ) else {
        return false; // unsigned marks do not verify — by design
    };
    let Ok(message) = signed_message(value, &claim_of(p)) else {
        return false;
    };
    crate::ump_integrity::verify_manifest_bytes(signed_by, sig, &message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::lock_env;

    /// Point the operator key at a temp dir holding a fresh 0600 seed — the
    /// mesh fixture, verbatim (same env law: THE shared lock, drop-restore).
    struct OperatorKey(#[allow(dead_code)] tempfile::TempDir);
    impl OperatorKey {
        fn new() -> OperatorKey {
            let dir = tempfile::TempDir::new().unwrap();
            std::fs::write(dir.path().join("operator.key"), [9u8; 32]).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    dir.path().join("operator.key"),
                    std::fs::Permissions::from_mode(0o600),
                )
                .unwrap();
            }
            // SAFETY: single-threaded under ENV_LOCK — the documented env-mutation posture.
            unsafe { std::env::set_var("BRAIN_UMP_KEY_DIR", dir.path()) };
            OperatorKey(dir)
        }
    }
    impl Drop for OperatorKey {
        fn drop(&mut self) {
            // SAFETY: single-threaded under ENV_LOCK.
            unsafe { std::env::remove_var("BRAIN_UMP_KEY_DIR") };
        }
    }

    fn artifact() -> Value {
        serde_json::json!({
            "run_id": 1,
            "lifecycle_state": "acknowledged",
            "remedy_history": [{"proposal_id": 7, "amount_cents": 2500}],
            "adr_body": "Schlichtungsstelle für den Verbraucherstreit",
            "odr_note": "the ODR platform was discontinued (Reg. 2024/3228)",
        })
    }

    /// provenance_round_trip_and_tamper — the sign→verify→tamper triangle on
    /// the exact artifact shape an ADR packet carries, plus the unsigned
    /// degradation and the HUMAN variant.
    #[test]
    fn provenance_round_trip_and_tamper() {
        let _guard = lock_env();
        let _key = OperatorKey::new();

        let mut a = artifact();
        assert!(attach_aigen(&mut a, 1790000000), "key present → signed");
        let p = &a[FIELD];
        assert_eq!(p["mark"], MARK_AIGEN);
        assert_eq!(
            p["generator"],
            format!("brain-server/{}", env!("CARGO_PKG_VERSION"))
        );
        assert!(p["signed_by"].as_str().unwrap().starts_with("did:key:z"));
        assert_eq!(p["sig"].as_str().unwrap().len(), 128, "ed25519 hex");
        assert!(verify_artifact(&a), "the honest artifact verifies");

        // One flipped body byte → refuse. (Tamper the BODY, not the sig —
        // the sig then mismatches the canonical bytes.)
        let mut b = a.clone();
        b["run_id"] = serde_json::json!(2);
        assert!(!verify_artifact(&b), "tampered body fails verify");

        // Tamper the MARK itself → refuse (the executed drill flips the sig).
        let mut c = a.clone();
        c[FIELD]["mark"] = serde_json::json!("HUMAN");
        assert!(!verify_artifact(&c), "a flipped mark fails verify");
        let mut d = a.clone();
        d[FIELD]["sig"] =
            serde_json::json!(&format!("00{}", &d[FIELD]["sig"].as_str().unwrap()[2..]));
        assert!(!verify_artifact(&d), "a flipped signature fails verify");

        // Remarking is idempotent and re-binds: sign, mutate, re-attach —
        // the new mark describes the new body and verifies again.
        let mut e = a.clone();
        e["adr_body"] = serde_json::json!("other body");
        assert!(attach_aigen(&mut e, 1790000001));
        assert!(verify_artifact(&e));
    }

    /// unsigned_mark_present_but_unverifiable — no operator key: the mark is
    /// STILL present (mark/generator/generated_at) with null sig, and
    /// verify() refuses — the visible degradation, never a silent one.
    /// (Point BRAIN_UMP_KEY_DIR at an EMPTY dir rather than unsetting it:
    /// unset falls back to the default key dir, which a real deployment
    /// resolves — the test must exercise the no-key path, not the happy one.)
    #[test]
    fn unsigned_mark_present_but_unverifiable() {
        let _guard = lock_env();
        let empty = tempfile::TempDir::new().unwrap();
        // SAFETY: single-threaded under ENV_LOCK — the documented env-mutation posture.
        unsafe { std::env::set_var("BRAIN_UMP_KEY_DIR", empty.path()) };
        let mut a = artifact();
        assert!(!attach_aigen(&mut a, 1790000000), "no key → unsigned");
        let p = &a[FIELD];
        assert_eq!(p["mark"], MARK_AIGEN, "the mark is still present");
        assert!(p["sig"].is_null());
        assert!(!verify_artifact(&a), "unsigned marks do not verify");
        // SAFETY: single-threaded under ENV_LOCK.
        unsafe { std::env::remove_var("BRAIN_UMP_KEY_DIR") };
    }

    /// human_mark_names_the_actor — HUMAN carries the actor principal in
    /// signed_by (and signs when the key resolves).
    #[test]
    fn human_mark_names_the_actor() {
        let _guard = lock_env();
        let _key = OperatorKey::new();
        let mut h = serde_json::json!({"content": "operator-written note"});
        assert!(attach_human(&mut h, "user:maria", 1790000000));
        assert_eq!(h[FIELD]["mark"], MARK_HUMAN);
        assert_eq!(h[FIELD]["actor"], "user:maria");
        assert!(verify_artifact(&h));
    }

    /// provenance_marks_present_on_all_four_classes — THE deliverable pin
    /// (the reg_watch ai_act flip anchors here): every engine-generated
    /// TEXT artifact class that leaves a boundary carries a VALID,
    /// verifiable provenance mark — complaint remedy drafts, ADR packets,
    /// outreach export packets, KB build manifests — through their REAL
    /// emission shapes (the handler seal fns + the real KB writer), never a
    /// parallel test fixture.
    #[test]
    fn provenance_marks_present_on_all_four_classes() {
        let _guard = lock_env();
        let _key = OperatorKey::new();
        let now = 1_790_000_000;
        let viewer: Option<crate::auth::Principal> = None;

        // ── 1. the complaint remedy draft (the POST response shape) ──────
        let mut conn = complaint_fixture();
        let draft = crate::workflow::complaint::RemedyDraft {
            run_id: 1,
            kind: brain_engine_sdk::pure::complaint::RemedyKind::ExplanationOnly,
            amount_cents: 0,
            code_clause_id: "",
            tier: 2,
            proposed_by: "agent",
        };
        let proposal = {
            let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn).unwrap();
            let p = crate::workflow::complaint::propose_remedy(tx.tx(), &draft, now)
                .expect("remedy proposed");
            tx.commit().unwrap();
            p
        };
        let remedy = crate::handlers::workflow::remedy_response(&proposal, now);
        assert_eq!(remedy[FIELD]["mark"], MARK_AIGEN);
        assert!(verify_artifact(&remedy), "the remedy draft's mark verifies");

        // ── 2. the ADR packet (the GET response shape, post read-seam) ───
        conn.execute(
            "INSERT INTO knowledge(title, content, source) VALUES ('DE', 'Schlichtungsstelle für den Verbraucherstreit', 'adr_body')",
            [],
        )
        .unwrap();
        let packet = crate::workflow::complaint::adr_packet(&conn, 1, "DE").expect("packet");
        let adr = crate::handlers::workflow::seal_adr_packet(packet, &viewer, now);
        assert_eq!(adr[FIELD]["mark"], MARK_AIGEN);
        assert!(verify_artifact(&adr), "the ADR packet's mark verifies");

        // ── 3. the outreach export packet (approved campaign) ────────────
        conn.execute(
            "INSERT INTO proposals(kind, content, source, novelty, salience, status, created_at)
             VALUES ('outreach_campaign', ?1, 'agent', 1.0, 0.5, 'approved', ?2)",
            rusqlite::params![
                r#"{"channel":"email","purpose":"care_followup","template_id":"followup-1","recipients":[],"excluded":{},"execution":"export-for-crm-only"}"#,
                now
            ],
        )
        .unwrap();
        let campaign_id = conn.last_insert_rowid();
        let campaign =
            crate::workflow::outreach::campaign_packet(&conn, campaign_id).expect("campaign");
        let sealed = crate::handlers::workflow::seal_campaign_packet(campaign, &viewer, now);
        assert_eq!(sealed[FIELD]["mark"], MARK_AIGEN);
        assert!(verify_artifact(&sealed), "the export packet's mark verifies");

        // ── 4. the KB build manifest (the real writer, disk round-trip) ──
        let mut files = std::collections::BTreeMap::new();
        files.insert(
            "articles/art.html".to_string(),
            "<html>art</html>".to_string(),
        );
        files.insert("index.html".to_string(), "<html>index</html>".to_string());
        let dir = tempfile::tempdir().unwrap();
        let n = crate::kb::write_artifact_at(dir.path(), &files, now).expect("written");
        assert_eq!(
            n,
            files.len() + 1,
            "manifest rides WITH the files, not as a page"
        );
        let written = std::fs::read_to_string(dir.path().join(crate::kb::MANIFEST_NAME)).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(manifest[FIELD]["mark"], MARK_AIGEN);
        assert!(
            verify_artifact(&manifest),
            "the build manifest's seal verifies over the digests body"
        );
        // The digests the operator verifies are byte-unchanged by the seal.
        let digests: serde_json::Value =
            serde_json::from_str(&crate::kb::manifest_json(&files)).unwrap();
        assert_eq!(
            manifest["files"], digests["files"],
            "the seal adds a field, never moves a digest"
        );
    }

    /// tampered_provenance_fails_verify — the verify path rejects tampered
    /// marks on EVERY class: flip one hex char of the signature (or one byte
    /// of the body, or the mark itself) and the artifact refuses.
    #[test]
    fn tampered_provenance_fails_verify() {
        let _guard = lock_env();
        let _key = OperatorKey::new();
        let now = 1_790_000_000;
        let viewer: Option<crate::auth::Principal> = None;

        let mut conn = complaint_fixture();
        let draft = crate::workflow::complaint::RemedyDraft {
            run_id: 1,
            kind: brain_engine_sdk::pure::complaint::RemedyKind::ExplanationOnly,
            amount_cents: 0,
            code_clause_id: "",
            tier: 2,
            proposed_by: "agent",
        };
        let proposal = {
            let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn).unwrap();
            let p = crate::workflow::complaint::propose_remedy(tx.tx(), &draft, now).unwrap();
            tx.commit().unwrap();
            p
        };
        let remedy = crate::handlers::workflow::remedy_response(&proposal, now);

        conn.execute(
            "INSERT INTO knowledge(title, content, source) VALUES ('DE', 'Schlichtungsstelle', 'adr_body')",
            [],
        )
        .unwrap();
        let adr = crate::handlers::workflow::seal_adr_packet(
            crate::workflow::complaint::adr_packet(&conn, 1, "DE").unwrap(),
            &viewer,
            now,
        );

        conn.execute(
            "INSERT INTO proposals(kind, content, source, novelty, salience, status, created_at)
             VALUES ('outreach_campaign', ?1, 'agent', 1.0, 0.5, 'approved', ?2)",
            rusqlite::params![
                r#"{"channel":"email","purpose":"care_followup","template_id":"t","recipients":[],"excluded":{},"execution":"export"}"#,
                now
            ],
        )
        .unwrap();
        let campaign_id = conn.last_insert_rowid();
        let campaign = crate::handlers::workflow::seal_campaign_packet(
            crate::workflow::outreach::campaign_packet(&conn, campaign_id).unwrap(),
            &viewer,
            now,
        );

        let mut files = std::collections::BTreeMap::new();
        files.insert(
            "articles/art.html".to_string(),
            "<html>art</html>".to_string(),
        );
        let dir = tempfile::tempdir().unwrap();
        crate::kb::write_artifact_at(dir.path(), &files, now).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(crate::kb::MANIFEST_NAME)).unwrap(),
        )
        .unwrap();

        for (class, artifact) in [
            ("remedy", remedy),
            ("adr-packet", adr),
            ("campaign-packet", campaign),
            ("kb-manifest", manifest),
        ] {
            // Flip the first hex char of the signature: a different message
            // digest, same shape — verify must refuse.
            let mut tampered = artifact.clone();
            let sig = tampered[FIELD]["sig"].as_str().unwrap().to_string();
            let flipped = if sig.starts_with('0') { "1" } else { "0" };
            tampered[FIELD]["sig"] = serde_json::json!(format!("{flipped}{}", &sig[1..]));
            assert!(!verify_artifact(&tampered), "{class}: flipped sig must refuse");

            // Flip the mark: same signature, wrong claim.
            let mut tampered = artifact.clone();
            let other = if tampered[FIELD]["mark"] == MARK_AIGEN {
                MARK_HUMAN
            } else {
                MARK_AIGEN
            };
            tampered[FIELD]["mark"] = serde_json::json!(other);
            assert!(!verify_artifact(&tampered), "{class}: flipped mark must refuse");

            // And the honest form verifies (the control).
            assert!(verify_artifact(&artifact), "{class}: honest artifact verifies");
        }
    }

    /// A complaint run (id 1) + migrated schema — the minimal fixture the
    /// remedy/ADR/campaign paths share (the complaint.rs db() shape).
    fn complaint_fixture() -> rusqlite::Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::migration::run_migration(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'complaint', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn
    }
}
