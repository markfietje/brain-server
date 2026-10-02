// R55-PATH — the agreement-labelling path's pins.
//
//! ## Why this file exists separately from the module
//!
//! Each pin below asserts a property that is **invisible from inside
//! `workflow/agreement.rs`**. A module cannot check its own absence, and a
//! module that could read its own wiring would be able to argue with it. The
//! two largest claims this round makes are absences — the frozen corpus does
//! not move, and the machine's verdict does not reach a reviewer — so they are
//! asserted from OUTSIDE, by reading the tree and the fixtures as they are on
//! disk.
//!
//! ## The read-as-text idiom
//!
//! Reading source as text is the `r46_evidence_pins.rs` / `r57_census_pins.rs`
//! precedent, used deliberately: a compile-time check cannot see an ABSENCE.
//! Every scan below cuts the region it scans, because a whole-file grep passes
//! on the scanner's own literal strings — the vacuous-pass class this
//! repository treats as worse than no check at all.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The eleven frozen gold fixtures, by path. Listed explicitly rather than
/// discovered by a directory walk: a walk would silently absorb a new file,
/// and the claim is that this SET is frozen.
const FROZEN_FIXTURES: &[&str] = &[
    "crates/gold-sets/gold/qc_report.json",
    "crates/gold-sets/gold/gdl_cases/intake_is_is_not.json",
    "crates/gold-sets/gold/gdl_cases/handoff_incomplete.json",
    "crates/gold-sets/gold/gdl_cases/repeater_3_30d.json",
    "crates/gold-sets/gold/gdl_cases/skipped_verify.json",
    "crates/gold-sets/gold/gdl_cases/stale_knowledge.json",
    "crates/gold-sets/gold/admission/solve_defect_resolution.json",
    "crates/gold-sets/gold/admission/create_defect_surface.json",
    "crates/gold-sets/gold/admission/care_inquiry_clean.json",
    "crates/gold-sets/gold/admission/unfaithful_slice.json",
    "crates/gold-sets/gold/admission/vacuous_twin.json",
];

/// **The frozen corpus is untouched by this round.** Every fixture is hashed
/// from disk and the digests are compared against the values recorded when
/// this round was preregistered — so a corpus edit cannot ride in with the
/// labelling path unnoticed, which is the exact failure this programme's
/// measurement discipline exists to prevent.
#[test]
fn the_frozen_gold_corpus_has_not_moved() {
    let mut digests: Vec<(String, String)> = Vec::new();
    for rel in FROZEN_FIXTURES {
        let bytes = std::fs::read(repo_root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
        digests.push(((*rel).to_string(), hex_encode(&sha256(&bytes))));
    }
    // Recorded from `shasum -a 256` at the round's preregistration commit,
    // 2026-09-30, before the first line of the labelling path was written.
    let expected: &[(&str, &str)] = &[
        (
            "crates/gold-sets/gold/qc_report.json",
            "d2f52b3a80acb76996a0b91787e4034c511e802af762ba063c6d871c33bf0dfb",
        ),
        (
            "crates/gold-sets/gold/gdl_cases/handoff_incomplete.json",
            "d9388bbdb1ceb55606307e7ab19b1c68959f9255ec8b98e229a56a05136721b2",
        ),
        (
            "crates/gold-sets/gold/gdl_cases/intake_is_is_not.json",
            "2cbe14808476d0fd0c0e6718965dbb18f56b85f705cbc423453e1e5ebf103379",
        ),
        (
            "crates/gold-sets/gold/gdl_cases/repeater_3_30d.json",
            "3f9bc5dab4438f4f22f9649f3fc42e488392752d786a5e8c58d8e5efca83604b",
        ),
        (
            "crates/gold-sets/gold/gdl_cases/skipped_verify.json",
            "c14bc517dcfd14578929d155e61c3937774440cae17f98dc18799ced82b17dd8",
        ),
        (
            "crates/gold-sets/gold/gdl_cases/stale_knowledge.json",
            "08e9accb37f23980b7027c7b082d12c232a7da89b48717dbd18fad8463688146",
        ),
        (
            "crates/gold-sets/gold/admission/care_inquiry_clean.json",
            "b24004b6dcb59bb8272ece1b377286f0906070481c133f128558ed8722df9726",
        ),
        (
            "crates/gold-sets/gold/admission/create_defect_surface.json",
            "15a68073005da312a992a96d45d2fda1c03bd454653b17afa0918e354f6e19d5",
        ),
        (
            "crates/gold-sets/gold/admission/solve_defect_resolution.json",
            "95e13ead69c36627a505d4a75b1c65f239f06fe9c4b0a4108035136ea9b572cd",
        ),
        (
            "crates/gold-sets/gold/admission/unfaithful_slice.json",
            "97fba49a6f5b0714aa55614f803d6fec2a9024f77a1fea84bb877824aaf25d63",
        ),
        (
            "crates/gold-sets/gold/admission/vacuous_twin.json",
            "75e18d52f35a979559ffc78166c3a55e544f678144088b9807bdb49374809652",
        ),
    ];
    for (rel, want) in expected {
        let got = digests
            .iter()
            .find(|(p, _)| p == rel)
            .map(|(_, d)| d.as_str())
            .unwrap_or_else(|| panic!("{rel} is missing from the recorded set"));
        assert_eq!(got, *want, "{rel} moved — the frozen corpus is frozen");
    }
    assert_eq!(
        expected.len(),
        FROZEN_FIXTURES.len(),
        "every frozen fixture carries a recorded digest"
    );
}

/// A minimal SHA-256 over raw bytes, so the pin hashes what is ON DISK rather
/// than a re-serialization of it. Uses the crate's own digest rather than a
/// new dependency: `Cargo.lock` must stay byte-identical this round.
fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&out);
    buf
}

/// Lowercase hex, local to the pin: an integration test cannot reach the
/// crate's `pub(crate)` audit helpers, and duplicating six lines of hex here
/// is cheaper than opening a module's visibility for it.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// **Blinding is a type, and the type is what the pin reads.** The queue's row
/// type must have no machine-verdict field, and the module must not offer any
/// constructor that populates one. This cuts the STRUCT DEFINITION region
/// specifically, so the literals in this file and in the module's own tests
/// cannot satisfy it.
#[test]
fn the_reviewer_row_type_carries_no_machine_verdict_field() {
    let src = read("src/workflow/agreement.rs");
    let start = src
        .find("pub(crate) struct AgreementTuple {")
        .expect("the reviewer row type exists");
    let region = &src[start..];
    let end = region.find("\n}").expect("the struct is closed");
    let body = &region[..end];
    for forbidden in ["machine_verdict", "governed_truth", "machine_outcome"] {
        assert!(
            !body.contains(forbidden),
            "the reviewer row type must not carry `{forbidden}`; body was:\n{body}"
        );
    }
    // And the type IS serialized, so a future field becomes a wire-visible one
    // and the field-count pin in the module's own tests would catch it.
    assert!(
        src.contains(
            "#[derive(Debug, Clone, PartialEq, Serialize)]\npub(crate) struct AgreementTuple"
        ),
        "the reviewer row type must derive Serialize — the blindness pin serializes the STRUCT"
    );
}

/// The queue surface must not emit a machine verdict, even by accident: the
/// handler's row projection is a hand-built JSON literal, so it is pinned
/// against the field that would carry the leak.
#[test]
fn the_queue_surface_emits_no_machine_verdict() {
    let src = read("src/handlers/agreement.rs");
    let start = src
        .find("pub async fn get_agreement_queue(")
        .expect("the queue surface exists");
    // The offset returned by `find` on the SLICE is relative to `start`, so it
    // must be added back before it is used as an absolute index — slicing with
    // the raw offset produces an inverted range, and the panic that follows
    // would mask the fact that the assertion never ran. (This exact bug made
    // the pin fail for the wrong reason on its first run.)
    let end = start
        + src[start..]
            .find("pub async fn post_agreement_label(")
            .expect("the capture surface follows the queue");
    assert!(end > start, "the queue slice is non-empty");
    let body = &src[start..end];
    // `machine_verdict` may appear ONLY in the queue's non-row context — here
    // it must not appear at all, because nothing in the queue may know it.
    assert!(
        !body.contains("machine_verdict"),
        "the queue handler must not reference the machine verdict:\n{body}"
    );
}

/// The report surface is the exfiltration gate: DPO dual gate plus the
/// `calibrate` capability, and an audited row per call. Pinned structurally so
/// a later round cannot quietly drop the audit or loosen the gate.
#[test]
fn the_agreement_report_is_dpo_gated_and_audited() {
    let src = read("src/handlers/agreement.rs");
    let start = src
        .find("pub async fn get_agreement_report(")
        .expect("the report surface exists");
    let body = &src[start..];
    assert!(
        body.contains("crate::auth::Action::Admin"),
        "the report demands the Admin scope"
    );
    assert!(
        body.contains("require_dpo_role"),
        "the report demands the DPO role"
    );
    assert!(
        body.contains("\"calibrate\""),
        "the report demands the calibrate capability"
    );
    assert!(
        body.contains("record_tenant"),
        "every report call lands an audited row"
    );
}

/// The claim wording is part of the contract, not a nicety: the panel is ONE
/// member and the rater is the system's author, so the report must never call
/// itself agreement with humans or a consensus.
#[test]
fn the_agreement_report_is_worded_agreement_with_the_operator() {
    let src = read("src/handlers/agreement.rs");
    assert!(
        src.contains("agreement-with-the-operator"),
        "the report names its measure"
    );
    for forbidden in [
        "agreement-with-humans",
        "human consensus",
        "human_consensus",
    ] {
        assert!(
            !src.contains(forbidden),
            "the report must not claim `{forbidden}`"
        );
    }
}

/// The round registers THREE routes, not the preregistration's two: the
/// report endpoint is the exfiltration-surface answer and is load-bearing for
/// the round's purpose. Pinned so the count cannot drift without the guard
/// tables and openapi drifting with it.
#[test]
fn the_agreement_routes_are_registered_in_every_wire_surface() {
    let routes = [
        "/workflow/agreement/queue",
        "/workflow/agreement/labels",
        "/workflow/agreement/report",
    ];
    let router = read("src/server/router/workflow.rs");
    let guards = read("src/server/router/route_guards.rs");
    let openapi = read("openapi.yaml");
    let api = read("docs/api.md");
    for route in routes {
        assert!(
            router.contains(route),
            "{route} is not registered in the router"
        );
        assert!(
            guards.contains(&format!("\"{route}\",")),
            "{route} has no OPENAPI_ROUTES row"
        );
        assert!(
            guards.contains(&format!("(\"{route}\",")),
            "{route} has no AUTHZ_GATES row"
        );
        assert!(openapi.contains(route), "{route} has no openapi.yaml path");
        assert!(api.contains(route), "{route} has no docs/api.md row");
    }
    // The write surfaces demand Write; the report demands Admin.
    for (route, action) in [
        ("/workflow/agreement/queue", "Write"),
        ("/workflow/agreement/labels", "Write"),
        ("/workflow/agreement/report", "Admin"),
    ] {
        assert!(
            guards.contains(&format!("(\"{route}\", \"{action}\")")),
            "{route} must demand {action}"
        );
    }
}

/// The round's substrate law: the additive kind needs no migration, so the
/// schema is untouched and the reviewer id is a PAYLOAD field. A reviewer id
/// as a COLUMN would mean a second rater needed a migration, which would make
/// the "no schema change" claim a fiction.
#[test]
fn the_reviewer_id_is_a_payload_field_and_not_a_column() {
    // No migration touches the schema.
    let migration = read("src/migration.rs");
    assert!(
        !migration.contains("agreement_label"),
        "the additive kind must not require a migration"
    );
    let layout = read("src/storage_layout.rs");
    assert!(
        layout.contains("LATEST_KNOWN_SCHEMA: &str = SCHEMA_VERSION_V1_32_24"),
        "the schema version must be unmoved by this round — the ceiling tracks the newest \
         schema round, and this round ships no migration of its own"
    );
    // And the reviewer rides the parsed payload, keyed with the rest.
    let src = read("src/workflow/agreement.rs");
    let start = src
        .find("pub(crate) struct AgreementLabelPayload {")
        .expect("the payload type exists");
    let end = src[start..].find("\n}").expect("the payload is closed");
    assert!(
        src[start..start + end].contains("reviewer_id"),
        "the reviewer id is a payload field"
    );
}

/// **A machine rater is a TYPE change, not a string.** The reviewer-kind
/// vocabulary is closed to one member, the writer refuses anything outside it,
/// and the report pairs only same-kind raters. A model rater joining as data
/// would push `distinct_reviewers` to 2 and have a reader conclude an
/// inter-rater reliability that nobody measured.
#[test]
fn the_reviewer_kind_vocabulary_is_closed_and_enforced() {
    let src = read("src/workflow/agreement.rs");
    // The vocabulary is declared closed and holds exactly one member.
    assert!(
        src.contains("pub(crate) const RATER_KINDS: &[&str] = &[\"operator\"];"),
        "the rater-kind vocabulary must be closed to `operator`"
    );
    assert!(
        src.contains("pub(crate) fn validate_reviewer_kind("),
        "the kind is validated, not accepted as a bare string"
    );
    // The WRITER stamps the kind from a core constant, never from the request.
    let writer = src
        .find("pub(crate) fn write_agreement_label(")
        .expect("the writer exists");
    let writer_body = &src[writer..writer + 2000];
    assert!(
        writer_body.contains("REVIEWER_KIND_OPERATOR"),
        "the writer stamps the ratified kind constant"
    );
    assert!(
        !writer_body.contains("body.reviewer_kind")
            && !writer_body.contains("reviewer_kind: reviewer_kind"),
        "the writer must not take the kind from the request"
    );
    // The pair report gates on same-kind.
    assert!(
        src.contains("if kind_a != kind_b {"),
        "the pair report must skip cross-kind pairs"
    );
    // The report reader is LENIENT about the kind (a stored row is a fact), so
    // a future kind cannot break the read of history — but the pairing rule
    // still reads the stored kind.
    assert!(
        src.contains("fn parse_stored_label_for_report("),
        "the report uses a kind-lenient reader"
    );
}

/// The handler must not let a client name a rater kind, and the report must
/// surface the kind it measured.
#[test]
fn the_handler_never_accepts_a_client_named_reviewer_kind() {
    let src = read("src/handlers/agreement.rs");
    // The body carries no kind field.
    let body = src
        .find("pub struct AgreementLabelBody {")
        .expect("the body struct exists");
    let body_end = src[body..].find("\n}").expect("the body is closed");
    assert!(
        !src[body..body + body_end].contains("reviewer_kind"),
        "the request body must not carry a reviewer kind"
    );
    // And the report emits it.
    assert!(
        src.contains("\"reviewer_kind\": c.reviewer_kind"),
        "the report surfaces the kind"
    );
    assert!(
        src.contains("\"rater_kind\": p.rater_kind"),
        "the pair cell surfaces the kind"
    );
}
