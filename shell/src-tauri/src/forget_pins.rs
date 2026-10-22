//! II-7b — the `/ump/forget` pins. The centre of this file is one pin — that
//! `hard: true` cannot be reached from the shell — and it is written as a
//! COERCION SWEEP rather than a single case, because "the shell does not pass
//! hard: true" is a statement about every way a caller might arrange it.
//!
//! Imported BY PATH, not `super::*`, for `verify_tests.rs`'s reason: `lib.rs`
//! re-exports both commands' `authorize`, and a glob would silently bind this
//! file's bounds assertions to the wrong command.

use super::forget::*;
use crate::recall::{Refused, Unreachable, WireExchange};
use serde_json::json;

fn request(id: &str) -> ForgetRequest {
    ForgetRequest {
        id: Some(id.to_string()),
        hard: None,
    }
}

fn doc(id: &str) -> ForgetDoc {
    ForgetDoc { id: id.to_string() }
}

fn exchange(status: u16, body: serde_json::Value) -> WireExchange {
    WireExchange {
        status,
        body: body.to_string(),
    }
}

fn tombstoned_body() -> serde_json::Value {
    json!({ "result": "tombstoned" })
}

/// BEHAVIORAL — the load-bearing pin of this command: `hard: true` cannot be
/// coerced from the shell, and the refusal says WHY.
///
/// Every arm below is a way a caller might arrange the destructive path, not
/// four spellings of one test. A pin that only sent `hard: true` would pass
/// against a core that refused the exact boolean and quietly forwarded every
/// other spelling — which is the shape coercion takes in a typed client where
/// one field is a boolean and the rest are not.
#[test]
fn the_forget_core_cannot_be_coerced_into_a_hard_erase() {
    // ---- THE SWEEP: every value `hard` can carry, and what happens to it ----
    //
    // `Option<Value>` is the reason this table is enumerable at all. With a
    // `bool` field, `"true"` would be a serde parse error naming nothing; with a
    // `Value`, every value the wire can carry arrives here and gets a NAMED
    // refusal.
    let coercions: Vec<(&str, serde_json::Value)> = vec![
        ("true", json!(true)),
        ("the string \"true\"", json!("true")),
        ("the string \"false\"", json!("false")),
        ("the number 1", json!(1)),
        ("the number 0", json!(0)),
        ("an object", json!({ "value": true })),
        ("an array", json!([true])),
        // A caller who believes a truthy string flips the gate. It does not,
        // and the refusal names the value so the belief is visible.
        ("the string \"yes\"", json!("yes")),
    ];
    for (name, value) in &coercions {
        let attempt = ForgetRequest {
            id: Some("41".to_string()),
            hard: Some(value.clone()),
        };
        assert_eq!(
            authorize(attempt).err(),
            Some(ForgetRefusal::HardRefused {
                requested: value.to_string()
            }),
            "hard: {name} must be refused, and the refusal must name the value that arrived"
        );
    }

    // ---- THE REFUSAL SAYS WHY, because the reason IS the deliverable ----
    //
    // "invalid request" would leave the next reader to guess whether the value
    // was refused for SHAPE or for AUTHORITY. The second reading is the one
    // that matters, so the sentence must carry all three facts: what the
    // destructive path requires, that a capability bearer cannot reach it, and
    // that the refusal is the boundary rather than an oversight.
    let refusal = authorize(ForgetRequest {
        id: Some("41".to_string()),
        hard: Some(json!(true)),
    })
    .expect_err("hard:true is refused");
    let text = refusal.to_string();
    for fact in [
        "/purge",
        "Admin scope",
        "purge role",
        "capability bearer can never reach it",
    ] {
        assert!(
            text.contains(fact),
            "the hard refusal must state `{fact}` — a refusal that says only \"invalid\" leaves \
             the reader guessing between shape and authority: {text}"
        );
    }
    // And it names the value that arrived, so the attempt is legible in a log.
    assert!(
        text.contains("true"),
        "the refusal must quote the received value: {text}"
    );

    // ---- `hard: false`, and `hard: null` on BOTH paths ----
    //
    // Both reach the same kernel behaviour as omission (`#[serde(default)] pub
    // hard: bool`), so neither needs authorizing. Accepting them rather than
    // refusing them is deliberate: refusing `false` would teach callers that the
    // field is load-bearing when it is not, and the soft path is the only path
    // this shell has.
    for soft in [json!(false), json!(null)] {
        let authorized = authorize(ForgetRequest {
            id: Some("41".to_string()),
            hard: Some(soft.clone()),
        })
        .expect("an explicitly-soft request authorizes");
        assert_eq!(authorized.doc, doc("41"));
    }
    // ABSENT, too — the ordinary path.
    assert_eq!(authorize(request("41")).expect("authorized").doc, doc("41"));

    // ---- AND THE SAME, THROUGH SERDE, which is the path that actually ships --
    //
    // The `null` case above constructs `Some(Value::Null)` in Rust, which the
    // wire cannot produce in that shape. This arm MEASURES what serde actually
    // does with the three soft spellings rather than asserting a single
    // collapse, because the three are NOT the same: only `null` collapses to
    // `None`; an explicit `false` stays `Some(Bool(false))`. The first draft of
    // this arm asserted one collapse for all three and was wrong — it would have
    // passed against a core that refused explicit-false, which is exactly the
    // over-refusal the module header argues against.
    //
    // What matters is asserted below and it is the same for all three: each
    // authorizes, and each yields the SAME doc — `{id}` alone.
    let soft_spellings = [
        (json!({ "id": "41" }), None),
        (json!({ "id": "41", "hard": null }), None),
        (
            json!({ "id": "41", "hard": false }),
            Some(serde_json::Value::Bool(false)),
        ),
    ];
    for (wire, expected_hard) in &soft_spellings {
        let parsed: ForgetRequest = serde_json::from_value(wire.clone())
            .unwrap_or_else(|e| panic!("{wire} must parse: {e}"));
        assert_eq!(
            &parsed.hard, expected_hard,
            "{wire}: the wire shape serde produces, measured rather than assumed"
        );
        assert_eq!(
            authorize(parsed)
                .expect("a soft wire request authorizes")
                .doc,
            doc("41"),
            "{wire}: all three soft spellings must authorize to the SAME doc"
        );
    }
    // And the destructive arm through serde as well, so the refusal is proven on
    // the path callers actually use.
    for (wire, expected) in [
        (
            json!({ "id": "41", "hard": true }),
            ForgetRefusal::HardRefused {
                requested: "true".to_string(),
            },
        ),
        (
            json!({ "id": "41", "hard": "true" }),
            ForgetRefusal::HardRefused {
                requested: "\"true\"".to_string(),
            },
        ),
    ] {
        let parsed: ForgetRequest = serde_json::from_value(wire.clone()).expect("must parse");
        assert_eq!(
            authorize(parsed),
            Err(expected),
            "{wire}: refused, with the value that arrived"
        );
    }

    // ---- THE PROOF THAT MATTERS: the core CANNOT EMIT `hard` AT ALL ----
    //
    // Every refusal above proves the core refuses what it is HANDED. This proves
    // the stronger thing: there is no path by which `hard` leaves, whatever was
    // handed in. The authorized body is serialized and its key set read, so a
    // future edit that adds `hard: request.hard.unwrap_or(false)` — or worse,
    // `hard: false` that someone later flips to a passthrough — fails here.
    let authorized = authorize(request("41")).expect("authorized");
    let body = serde_json::to_value(&authorized.doc).expect("the authorized body serializes");
    let object = body.as_object().expect("the authorized body is an object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["id"],
        "the authorized /ump/forget body must be {{id}} alone — the kernel's requestBody is \
         {{id, reason?, hard?}} and this core authorizes the soft path with neither optional field"
    );
    assert!(
        !object.contains_key("hard"),
        "the core must never emit `hard`: the field is structurally absent so there is no line a \
         later edit can turn into the destructive act"
    );
    assert!(
        !object.contains_key("reason"),
        "the core never emits `reason` either — see the module header for why the kernel's \
         unbounded free-text field is not exposed"
    );
    // And round-tripping the authorized doc back through the request type
    // yields no `hard` either, so the echo seam cannot re-introduce it.
    let round_trip: ForgetRequest = serde_json::from_value(body).expect("round-trips");
    assert_eq!(
        round_trip.hard, None,
        "an authorized doc must deserialize with no hard field"
    );

    // ---- THE ORDERING: `hard` is refused even when the id is ALSO malformed --
    //
    // Answering `id_malformed` to a `hard: true` request would hide the one
    // refusal that matters behind an unrelated one. Same shape as the kernel's
    // "the 403/401 leak nothing about which ids exist".
    let both_broken = authorize(ForgetRequest {
        id: Some("   ".to_string()),
        hard: Some(json!(true)),
    });
    assert_eq!(
        both_broken,
        Err(ForgetRefusal::HardRefused {
            requested: "true".to_string()
        }),
        "a destructive request must be refused AS destructive, not behind an id complaint"
    );
    // Same for a MISSING id: the hard refusal still wins.
    assert_eq!(
        authorize(ForgetRequest {
            id: None,
            hard: Some(json!(true))
        }),
        Err(ForgetRefusal::HardRefused {
            requested: "true".to_string()
        }),
        "a destructive request with no id must still be refused as destructive"
    );
}

/// BEHAVIORAL — the id bounds: the two refusals `resolve_row_id` cannot serve,
/// and no invented rule beside them.
#[test]
fn the_forget_id_is_bounded_by_what_the_kernel_can_resolve() {
    // Both addressable forms authorize: the numeric row id and the
    // `urn:ump:…` content id (`openapi.yaml`'s `{id}` description).
    for id in ["41", "urn:ump:brain:abc123", "brain:7"] {
        assert_eq!(
            authorize(request(id))
                .expect("an addressable id authorizes")
                .doc,
            doc(id),
            "{id} is one of resolve_row_id's shapes and must authorize"
        );
    }
    // `brain:7` above is deliberate: `resolve_row_id`'s third rule takes any
    // `:`-suffix that parses as an i64, so narrowing the core to the two
    // DOCUMENTED forms would refuse an id the kernel serves. The core refuses
    // only what cannot resolve under ANY of the three.

    // Absent is `IdMissing`; present-but-blank is `IdMalformed`. Different
    // operator mistakes, so different arms.
    assert_eq!(
        authorize(ForgetRequest {
            id: None,
            hard: None
        }),
        Err(ForgetRefusal::IdMissing)
    );
    for blank in ["", " ", "   ", "\t\n "] {
        assert_eq!(
            authorize(request(blank)),
            Err(ForgetRefusal::IdMalformed {
                id: blank.to_string()
            }),
            "{blank:?} is blank, which no resolve_row_id rule can resolve"
        );
    }
    // A blank id is refused WHATEVER was handed for it besides the id — a soft
    // request with a blank id is still a blank id.
    assert_eq!(
        authorize(ForgetRequest {
            id: Some(" ".to_string()),
            hard: Some(json!(false))
        }),
        Err(ForgetRefusal::IdMalformed {
            id: " ".to_string()
        })
    );

    // The authorized id is TRIMMED, so the string that goes out is the one that
    // resolved — no whitespace-padded id reaches `resolve_row_id`.
    assert_eq!(
        authorize(request("  41  ")).expect("authorized").doc,
        doc("41"),
        "the authorized id is the trimmed one, so no padded id reaches the resolver"
    );

    // And each refusal names what fired rather than tagging itself.
    for (refusal, needle) in [
        (ForgetRefusal::IdMissing, "no id"),
        (
            ForgetRefusal::IdMalformed {
                id: "  ".to_string(),
            },
            "blank",
        ),
    ] {
        let text = refusal.to_string();
        assert!(
            text.contains(needle),
            "the refusal must name its bound ({needle}): {text}"
        );
    }

    // Anti-vacuity: the ordinary path still authorizes.
    assert!(authorize(request("41")).is_ok());
}

/// BEHAVIORAL — the authz half, on the first WRITE surface in the shell.
///
/// Same law as the two read commands, and it matters more here: `/ump/forget`
/// writes, so a field through which a principal or a scope could arrive would
/// not be a read leak but a write performed under someone else's authority.
#[test]
fn the_forget_core_takes_no_principal_and_refuses_one_smuggled_in() {
    for field in [
        "authorization",
        "token",
        "principal",
        "bearer",
        "tenant",
        "role",
        "capability",
        "admin",
        "purge",
    ] {
        // `tenant`, `role`, `capability`, `admin` and `purge` are the ones that
        // matter most: they are the NAMES of the kernel's own gates
        // (`authorize_role(p, pool, "purge")`, `cap_gate(cap, "admin")`), so a
        // caller could reasonably believe the shell forwards them. It must not,
        // and the refusal names the field.
        let attempt = json!({ "id": "41", field: "granted" });
        let parsed = serde_json::from_value::<ForgetRequest>(attempt.clone());
        assert!(
            parsed.is_err(),
            "a forget request carrying `{field}` must be refused, not parsed: {attempt}"
        );
        let error = parsed
            .expect_err("the refusal is the assertion")
            .to_string();
        assert!(
            error.contains(field),
            "the refusal must NAME the field it refused, so the bypass is legible: {error}"
        );
    }

    // `reason` is refused too, and that is a DECISION rather than an omission:
    // the kernel writes it verbatim into `tombstones.reason` with no ceiling of
    // its own, so exposing it would mean either letting a caller author
    // unbounded text into a persisted row or choosing a ceiling the kernel does
    // not state. Both are refused; see the module header.
    let reason_attempt = json!({ "id": "41", "reason": "because" });
    assert!(
        serde_json::from_value::<ForgetRequest>(reason_attempt.clone()).is_err(),
        "`reason` is deliberately not exposed: {reason_attempt}"
    );

    // Anti-vacuity: the field the core DOES own still parses.
    assert!(serde_json::from_value::<ForgetRequest>(json!({ "id": "41" })).is_ok());
}

/// EXHAUSTIVE — the refusal enum is TOTAL over its own bounds, every arm
/// reachable from a bound, and every arm named by the compiler.
#[test]
fn the_forget_refusal_enum_is_total_over_its_bounds() {
    // No `_` arm: a fourth `ForgetRefusal` variant is a COMPILE ERROR here
    // rather than an unnamed arm, so "total" is a property the compiler holds
    // rather than a count a test maintains.
    fn arm_name(refusal: &ForgetRefusal) -> &'static str {
        match refusal {
            ForgetRefusal::IdMissing => "id_missing",
            ForgetRefusal::IdMalformed { .. } => "id_malformed",
            ForgetRefusal::HardRefused { .. } => "hard_refused",
        }
    }
    const DECLARED: [&str; 3] = ["id_missing", "id_malformed", "hard_refused"];

    // The bounds, held as `Result` so a bound that stopped refusing fails the
    // pin's OWN assertion naming the bound, not a generic unwrap panic.
    let bounds: Vec<(&str, Result<AuthorizedForget, ForgetRefusal>)> = vec![
        (
            "no id",
            authorize(ForgetRequest {
                id: None,
                hard: None,
            }),
        ),
        ("blank id", authorize(request("   "))),
        (
            "hard:true",
            authorize(ForgetRequest {
                id: Some("41".to_string()),
                hard: Some(json!(true)),
            }),
        ),
    ];

    let mut reachable: Vec<(&str, ForgetRefusal)> = Vec::new();
    for (bound, result) in &bounds {
        match result {
            Ok(authorized) => panic!(
                "bound `{bound}` AUTHORIZED {authorized:?} — it is not enforcing, so whatever arm \
                 it was written to cover is decoration"
            ),
            Err(refusal) => reachable.push((bound, refusal.clone())),
        }
    }

    let produced: Vec<&str> = reachable.iter().map(|(_, r)| arm_name(r)).collect();
    let mut distinct: Vec<&str> = produced.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        DECLARED.len(),
        "the bounds reach {} distinct arms but the enum declares {}: produced {produced:?}",
        distinct.len(),
        DECLARED.len()
    );
    for declared in DECLARED {
        assert!(
            produced.contains(&declared),
            "declared arm `{declared}` has no bound that produces it — it is unreachable, so it \
             is decoration. produced {produced:?}"
        );
    }

    // The arm name IS the wire tag, tying the hand-written table to the
    // serializer rather than restating it.
    for (bound, refusal) in &reachable {
        let json = serde_json::to_value(refusal).expect("a refusal serializes");
        let tag = json
            .get("refusal")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("a refusal must serialize with a `refusal` tag: {json}"));
        assert_eq!(
            tag,
            arm_name(refusal),
            "bound `{bound}`: the wire tag drifted from the arm name"
        );
    }

    // Anti-vacuity.
    assert!(authorize(request("41")).is_ok());
}

/// EXHAUSTIVE — the outcome enum is total, every arm reachable, and the token
/// table tied to the serialization.
#[test]
fn the_forget_outcome_enum_is_total_and_every_arm_is_reachable() {
    let asked = doc("41");

    // ---- the one success arm ----
    let done = shape_forget(exchange(200, tombstoned_body()), &asked);
    assert_eq!(
        done,
        ForgetOutcome::Tombstoned(Tombstoned {
            id: "41".to_string(),
            result: ForgetResult::Tombstoned,
            dsar_certificate: None,
        })
    );
    assert!(done.is_tombstoned());
    assert!(!done.is_refusal());
    assert_eq!(done.result(), Some(ForgetResult::Tombstoned));

    // THE LAW `/ump/forget` DOES NOT CARRY, as a VALUE rather than an omission:
    // the outcome carries `dsar_certificate: None`, so a receipt cannot be built
    // from this outcome by forgetting a field. `/ump/forget` returns exactly
    // `{"result":"tombstoned"}`; certificates belong to the Admin-only,
    // subject-addressed `/dsar` workflow.
    //
    // The key must be PRESENT and null. Asserted as a `Value` rather than via
    // `as_str()`: the earlier form of this assertion asked a null for its
    // string and failed on a body that was correct, which is a pin that reports
    // a green field as broken — worse than no pin.
    let done_json = serde_json::to_value(&done).expect("serializes");
    assert_eq!(
        done_json.get("dsar_certificate"),
        Some(&serde_json::Value::Null),
        "the certificate field must be PRESENT and null, so its absence is a decision"
    );
    assert!(
        done_json
            .as_object()
            .expect("an object")
            .contains_key("dsar_certificate"),
        "the key must exist: a skipped field and a null field look identical to a reader who is \
         checking for the key"
    );

    // ---- the refusal arm, 401/403 ----
    for status in [401u16, 403] {
        let refused = shape_forget(exchange(status, json!({})), &asked);
        assert_eq!(
            refused,
            ForgetOutcome::Refused(Refused {
                status,
                scope: "ump.forget"
            }),
            "a {status} is a permission decision, not a tombstone"
        );
        assert!(refused.is_refusal());
        assert!(!refused.is_tombstoned());
        assert_eq!(refused.result(), None);
    }

    // ---- 404: the kernel's blur of absent and not-admitted ----
    let missing = shape_forget(exchange(404, json!({})), &asked);
    assert_eq!(
        missing,
        ForgetOutcome::NoSuchRow(NoSuchRow {
            id: "41".to_string(),
            status: 404
        })
    );
    assert!(missing.is_refusal(), "a 404 is refusal-shaped");
    assert!(!missing.is_tombstoned());
    // No field from which existence could be read. `openapi.yaml` states this
    // 404 covers "missing id, or a target row the caller's record gate does not
    // admit" — one status for both, deliberately.
    let missing_json = serde_json::to_value(&missing).expect("serializes");
    for forbidden in ["exists", "found", "forbidden", "owned_by_other"] {
        assert!(
            missing_json.get(forbidden).is_none(),
            "the 404 arm must not carry `{forbidden}`: the kernel declines to distinguish absent \
             from not-admitted, so neither may this"
        );
    }

    // ---- the degraded arm, including the legal hold's 409 ----
    for status in [400u16, 409, 500, 503] {
        let degraded = shape_forget(exchange(status, json!({})), &asked);
        assert_eq!(
            degraded,
            ForgetOutcome::Unavailable(Unreachable {
                status,
                detail: "the kernel answered with a non-success status".to_string()
            }),
            "status {status} is degraded, never a tombstone"
        );
        assert!(
            !degraded.is_refusal(),
            "a 409 legal hold is the kernel's machine declining"
        );
        assert!(!degraded.is_tombstoned());
    }

    // ---- `ContractViolation`, and THE BACKSTOP is its own arm ----
    //
    // The first: a body that is not the published contract. Note that
    // `deny_unknown_fields` is what makes a FABRICATED certificate a violation
    // rather than a receipt source.
    let fabricated = shape_forget(
        exchange(
            200,
            json!({ "result": "tombstoned", "dsar_certificate": "dsar-2026-10-18-0042" }),
        ),
        &asked,
    );
    assert!(
        matches!(fabricated, ForgetOutcome::ContractViolation(_)),
        "a body carrying a certificate is not the published contract, and the fabricated \
         reference is the exact failure forget.ts exists to prevent"
    );

    // THE BACKSTOP, and the most important arm in this file. A `hard: true` that
    // somehow reached the kernel comes back `erased`. Presenting that as a
    // tombstone would tell the operator "flagged, still retrievable" about a
    // destroyed row.
    let erased = shape_forget(exchange(200, json!({ "result": "erased" })), &asked);
    assert_eq!(
        erased,
        ForgetOutcome::ContractViolation(Unreachable {
            status: 200,
            detail: erased_detail(&erased)
        }),
        "an `erased` reply to a soft-authorized request must be refused, never presented as a \
         tombstone"
    );
    let detail = erased_detail(&erased);
    assert!(
        detail.contains("/purge") && detail.contains("do not report a tombstone"),
        "the backstop must name the act it caught and the action it forbids: {detail}"
    );
    assert!(
        !erased.is_tombstoned(),
        "the backstop is the whole module's confidentiality property"
    );
    assert_eq!(erased.result(), None, "a violation reports no result");

    // ---- the enum is TOTAL: five arms, five reachable, five distinct ----
    let every_arm = [
        done.clone(),
        ForgetOutcome::Refused(Refused {
            status: 403,
            scope: "ump.forget",
        }),
        ForgetOutcome::NoSuchRow(NoSuchRow {
            id: "41".to_string(),
            status: 404,
        }),
        ForgetOutcome::Unavailable(Unreachable {
            status: 500,
            detail: String::new(),
        }),
        ForgetOutcome::ContractViolation(Unreachable {
            status: 200,
            detail: String::new(),
        }),
    ];
    let states: Vec<&str> = every_arm.iter().map(ForgetOutcome::state_token).collect();
    let mut distinct: Vec<&str> = states.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        states.len(),
        5,
        "the forget taxonomy must have five arms, found {}",
        states.len()
    );
    assert_eq!(
        distinct.len(),
        states.len(),
        "two arms share a wire token, so a client switch cannot tell them: {states:?}"
    );
    for (arm, token) in every_arm.iter().zip(states.iter()) {
        let serialized = serde_json::to_value(arm).expect("every arm serializes");
        assert_eq!(
            serialized.get("state").and_then(|v| v.as_str()),
            Some(*token),
            "the serialized tag drifted from the taxonomy table for {token}"
        );
    }
}

/// The IPC seam re-authorizes the echoed doc. For this command that is also the
/// `hard` backstop at the seam: a caller cannot hand back a doc it was never
/// granted, because the granted doc type has no `hard` field to hand.
#[test]
fn the_echoed_forget_request_must_be_one_the_core_authorized() {
    assert_eq!(
        shape_forget_command(exchange(200, tombstoned_body()), doc("   ")),
        Err(ForgetRefusal::IdMalformed {
            id: "   ".to_string()
        })
    );
    assert_eq!(
        shape_forget_command(exchange(200, tombstoned_body()), doc("")),
        Err(ForgetRefusal::IdMalformed { id: String::new() })
    );
    // Anti-vacuity: the legal doc passes the seam and is classified.
    assert!(shape_forget_command(exchange(200, tombstoned_body()), doc("41")).is_ok());

    // And the seam's re-authorization cannot be talked into `hard` — the doc it
    // is handed has no such field, so there is nothing to pass. This arm exists
    // because the `hard` backstop is worth holding at BOTH places it could leak.
    let as_request: ForgetRequest =
        serde_json::from_value(serde_json::to_value(doc("41")).expect("serializes"))
            .expect("an authorized doc deserializes as a request");
    assert_eq!(as_request.hard, None);
    assert!(authorize(as_request).is_ok());
}

fn erased_detail(outcome: &ForgetOutcome) -> String {
    match outcome {
        ForgetOutcome::ContractViolation(Unreachable { detail, .. }) => detail.clone(),
        other => panic!("expected a contract violation, got {other:?}"),
    }
}
