//! II-7a — the `/verify` pins. Four of them, and the class of each is in its
//! own name, because the prompt's rule is that a pin asserting only the happy
//! path proves the fetch works.
//!
//! `#[cfg(test)]` here rather than in `tests/` for `recall.rs`'s stated reason:
//! the subject is the crate's own commands, and keeping the module private
//! stops a pin from becoming the only caller of a production path.

// The module under test, imported BY PATH rather than through `super::*`.
// `lib.rs` re-exports both commands' `authorize`, so a glob would resolve this
// file's `authorize` to `recall::authorize` and every bound assertion would
// compile against the wrong command — a type error here, but the kind that
// makes a reader stop trusting the pins.
use super::verify::*;
use crate::recall::{Refused, Unreachable, WireExchange};
use serde_json::json;

fn request(chunk_id: i64, claim: &str) -> VerifyRequest {
    VerifyRequest {
        chunk_id,
        claim: claim.to_string(),
    }
}

fn exchange(status: u16, body: serde_json::Value) -> WireExchange {
    WireExchange {
        status,
        body: body.to_string(),
    }
}

/// A supported body, exactly as `handlers/verify.rs` writes it.
fn supported_body(chunk_id: i64) -> serde_json::Value {
    json!({
        "chunk_id": chunk_id,
        "supported": true,
        "decision": "supported",
        "match_ranges": [[0, 5]]
    })
}

fn unsupported_body(chunk_id: i64) -> serde_json::Value {
    json!({
        "chunk_id": chunk_id,
        "supported": false,
        "decision": "unsupported_claim",
        "match_ranges": []
    })
}

fn doc(chunk_id: i64, claim: &str) -> VerifyDoc {
    VerifyDoc {
        chunk_id,
        claim: claim.to_string(),
    }
}

/// BEHAVIORAL — the claim bound, at the kernel's own thresholds.
///
/// This is the arm that would catch the mistake the module header is about, and
/// the mistake is the one a reader of `recall.rs` would make automatically:
/// `recall.rs` measures BYTES because `handlers/recall.rs` measures bytes. The
/// `/verify` handler measures `chars()` AFTER trimming, so:
#[test]
fn a_claim_is_bounded_in_chars_after_trim_at_the_kernels_thresholds() {
    // A plain claim authorizes, and the authorized body is the request.
    let authorized =
        authorize(request(7, "the wire bridge")).expect("a plain claim must authorize");
    assert_eq!(authorized.doc.chunk_id, 7);
    assert_eq!(authorized.doc.claim, "the wire bridge");
    assert_eq!(authorized.claim_chars, "the wire bridge".chars().count());

    // ---- EMPTY: refused, and refused where the kernel refuses it ----
    //
    // Whitespace-only is `claim_empty` and NOT a long claim, because the trim
    // precedes the emptiness check in `handlers/verify.rs:69-75`. An arm that
    // checked emptiness before trimming would authorize "   " and the kernel
    // would 400 it — a core that mints requests the kernel rejects.
    for blank in ["", " ", "   ", "\t", "\n", " \t\n "] {
        assert_eq!(
            authorize(request(7, blank)),
            Err(VerifyRefusal::ClaimEmpty),
            "{blank:?} is empty once trimmed, which is the kernel's claim_empty"
        );
    }

    // ---- TOO LONG: chars, and INCLUSIVE at exactly MAX_CLAIM_CHARS ----
    let at_ceiling = "c".repeat(MAX_CLAIM_CHARS);
    assert_eq!(
        authorize(request(7, &at_ceiling)).map(|a| a.claim_chars),
        Ok(MAX_CLAIM_CHARS),
        "a claim of exactly MAX_CLAIM_CHARS must authorize — the kernel refuses only ABOVE it"
    );
    let over_ceiling = "c".repeat(MAX_CLAIM_CHARS + 1);
    assert_eq!(
        authorize(request(7, &over_ceiling)),
        Err(VerifyRefusal::ClaimTooLong {
            chars: MAX_CLAIM_CHARS + 1,
            max: MAX_CLAIM_CHARS
        })
    );

    // ---- THE ARM THAT MAKES THIS NOT A BYTE BOUND IN DISGUISE ----
    //
    // 2_000 CHARACTERS of a 3-byte scalar is ~6_000 bytes and the kernel
    // SERVES it, because it counts `chars()`. A byte-measuring core copied from
    // `recall.rs` refuses this request the kernel would have answered. That is
    // over-refusal against a live route, which is the failure this arm exists.
    let multibyte = "é".repeat(MAX_CLAIM_CHARS);
    assert_eq!(
        multibyte.chars().count(),
        MAX_CLAIM_CHARS,
        "the fixture must be exactly at the ceiling in CHARS"
    );
    assert!(
        multibyte.len() > MAX_CLAIM_CHARS,
        "the fixture must EXCEED the ceiling in BYTES, or this arm proves nothing — it is \
         {} bytes vs {MAX_CLAIM_CHARS}",
        multibyte.len()
    );
    assert_eq!(
        authorize(request(7, &multibyte)).map(|a| a.claim_chars),
        Ok(MAX_CLAIM_CHARS),
        "2_000 chars of a multibyte scalar is legal: the kernel counts chars, not bytes"
    );

    // And one character more in CHARS is refused, whatever its byte length.
    assert_eq!(
        authorize(request(7, &"é".repeat(MAX_CLAIM_CHARS + 1))),
        Err(VerifyRefusal::ClaimTooLong {
            chars: MAX_CLAIM_CHARS + 1,
            max: MAX_CLAIM_CHARS
        })
    );

    // ---- TRIM PRECEDES THE MEASUREMENT, on both sides ----
    //
    // 2_000 chars of claim plus surrounding whitespace is legal, because the
    // kernel trims before counting. Counting the raw string would refuse it.
    let padded = format!("  {at_ceiling}  ");
    assert!(
        padded.chars().count() > MAX_CLAIM_CHARS,
        "the fixture must exceed the ceiling RAW, or the trim arm proves nothing"
    );
    assert_eq!(
        authorize(request(7, &padded)).map(|a| a.claim_chars),
        Ok(MAX_CLAIM_CHARS),
        "the kernel trims before counting, so padded-to-the-ceiling authorizes"
    );

    // And the authorized body carries the TRIMMED claim — the string that was
    // bounded is the string that goes out. An echo of the raw string would make
    // the bound a description rather than a fact.
    assert_eq!(
        authorize(request(7, &padded))
            .expect("authorized")
            .doc
            .claim,
        at_ceiling
    );

    // ---- CHUNK ID: the rowid bound ----
    assert_eq!(
        authorize(request(0, "k")),
        Err(VerifyRefusal::ChunkIdNotPositive { chunk_id: 0 })
    );
    for bad in [-1i64, -42, i64::MIN] {
        assert_eq!(
            authorize(request(bad, "k")),
            Err(VerifyRefusal::ChunkIdNotPositive { chunk_id: bad })
        );
    }
    assert_eq!(
        authorize(request(MIN_CHUNK_ID, "k")).map(|a| a.doc.chunk_id),
        Ok(MIN_CHUNK_ID),
        "id 1 is the first rowid the sequence can mint, so it must authorize"
    );

    // The ceiling constant is pinned to the kernel's own value. `MAX_QUERY` is
    // 2_000 (`src/handlers/mod.rs:88`), and `/verify` enforces exactly that.
    assert_eq!(
        MAX_CLAIM_CHARS, 2_000,
        "handlers/mod.rs declares MAX_QUERY = 2_000"
    );
    assert_eq!(MIN_CHUNK_ID, 1);
}

/// BEHAVIORAL — the authz half, same law as `RecallQuery`'s, on a NEW surface.
///
/// `/verify` is a content-confirmation oracle, so a field through which a
/// principal could arrive would be the whole attack. There is none, and a
/// smuggling attempt is REFUSED rather than ignored.
#[test]
fn the_verify_core_takes_no_principal_and_refuses_one_smuggled_in() {
    for field in [
        "authorization",
        "token",
        "principal",
        "bearer",
        "domain",
        "owner",
        "scope",
    ] {
        // `domain` and `owner` are the interesting two here: they are REAL
        // fields on other kernel routes, so a caller could reasonably assume
        // this one accepts them. It does not — the kernel's `/verify` body is
        // `{chunk_id, claim}` and the row's domain comes from a HEADER the
        // core does not mint.
        let attempt = json!({ "chunk_id": 1, "claim": "k", field: "someone-elses" });
        let parsed = serde_json::from_value::<VerifyRequest>(attempt.clone());
        assert!(
            parsed.is_err(),
            "a verify request carrying `{field}` must be refused, not parsed: {attempt}"
        );
        let error = parsed
            .expect_err("the refusal is the assertion")
            .to_string();
        assert!(
            error.contains(field),
            "the refusal must NAME the field it refused, so the bypass is legible: {error}"
        );
    }

    // The authorized body is the kernel's two-field body and NOTHING else.
    let authorized = authorize(request(1, "k")).expect("a plain claim must authorize");
    let body = serde_json::to_value(&authorized.doc).expect("the authorized body serializes");
    let object = body.as_object().expect("the authorized body is an object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["chunk_id", "claim"],
        "the authorized /verify body must be exactly openapi.yaml's requestBody: chunk_id, claim"
    );
    for forbidden in [
        "authorization",
        "token",
        "principal",
        "bearer",
        "hard",
        "domain",
    ] {
        assert!(
            !object.contains_key(forbidden),
            "the core must never emit `{forbidden}`; it has no principal and no scope to emit"
        );
    }

    // Anti-vacuity: the two fields the core DOES own still parse, so the
    // refusals above are about unknown fields and not about a parser that
    // refuses everything.
    assert!(
        serde_json::from_value::<VerifyRequest>(json!({ "chunk_id": 3, "claim": "k" })).is_ok()
    );
}

/// EXHAUSTIVE — the refusal enum is TOTAL over its own bounds: every arm is
/// reachable from a bound, and no arm is decoration.
///
/// The two halves matter differently. "Every arm is REACHABLE" catches an arm
/// no input can trigger — decoration wearing a taxonomy. "Every arm is NAMED"
/// catches the opposite: a variant that exists and is exercised by the pin but
/// that no reader can identify, which is an enum arm the webview's `switch`
/// cannot reach.
#[test]
fn the_verify_refusal_enum_is_total_over_its_bounds() {
    // ---- HALF 1, ENFORCED BY THE COMPILER: every variant has a name ----
    //
    // There is deliberately no `_ =>` arm. Adding a fourth variant to
    // `VerifyRefusal` is then a COMPILE ERROR here rather than a silently
    // unnamed arm, so "total" here is a property the compiler holds rather than
    // a count a test maintains. This is the strongest form available without
    // reflection, and reflection is not worth a dependency for a three-arm
    // enum.
    fn arm_name(refusal: &VerifyRefusal) -> &'static str {
        match refusal {
            VerifyRefusal::ClaimEmpty => "claim_empty",
            VerifyRefusal::ClaimTooLong { .. } => "claim_too_long",
            VerifyRefusal::ChunkIdNotPositive { .. } => "chunk_id_not_positive",
        }
    }

    // The declared table, and the pins that read it.
    const DECLARED: [&str; 3] = ["claim_empty", "claim_too_long", "chunk_id_not_positive"];

    // ---- HALF 2: an input exists for every declared arm ----
    //
    // Each entry is a BOUND and the result it produces, not a fixture
    // hand-built to hit an arm. This list IS the bound table: if a bound is
    // added without a line here, the coverage assertion below fails.
    //
    // The results are kept as `Result` rather than unwrapped to a refusal,
    // deliberately: a bound that stopped refusing would then fail the pin's
    // OWN named assertion — "bound `chunk_id < 1` refused nothing, so the arm it
    // covers is decoration" — instead of dying on a generic
    // `unwrap_err()` panic that names neither the bound nor the arm.
    let bounds: Vec<(&str, Result<AuthorizedVerify, VerifyRefusal>)> = vec![
        ("chunk_id < 1", authorize(request(0, "k"))),
        ("blank claim", authorize(request(1, "   "))),
        (
            "claim over the ceiling",
            authorize(request(1, &"c".repeat(MAX_CLAIM_CHARS + 1))),
        ),
    ];

    // Every declared bound must refuse. A bound that authorizes is a bound that
    // is not enforcing, which is the defect this half exists to name.
    let mut reachable: Vec<(&str, VerifyRefusal)> = Vec::new();
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
        "the bounds reach {} distinct arms but the enum declares {}: every arm must have a \
         bound that produces it, and no arm may be decoration. produced {produced:?}",
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
    // And no bound collapses onto an arm another bound already reached, which
    // would mean a real bound is folded into an unrelated arm's name.
    assert_eq!(
        produced.len(),
        distinct.len(),
        "two different bounds produced the same arm, so a real bound is unnamed: {produced:?}"
    );

    // ---- HALF 3: the arm name IS the wire tag, and each refusal says what fired --
    //
    // Tying the hand-written name to the serializer's actual tag is the teeth:
    // renaming the serde attribute without renaming this function fails here, so
    // a webview `switch` can rely on both spellings agreeing.
    for (bound, refusal) in &reachable {
        let name = arm_name(refusal);
        let json = serde_json::to_value(refusal).expect("a refusal serializes");
        let tag = json
            .get("refusal")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("a refusal must serialize with a `refusal` tag: {json}"));
        assert_eq!(
            tag, name,
            "bound `{bound}`: the wire tag drifted from the arm name"
        );
        // Each arm's Display names its bound, so the refusal is legible to the
        // person who triggered it rather than being a bare tag.
        let text = refusal.to_string();
        assert!(
            text.len() > 10,
            "bound `{bound}`: a refusal must say what fired, not tag itself"
        );
    }

    // Anti-vacuity on the whole enum: the legal request still authorizes, so
    // the three refusals above are bounds and not a command that refuses all.
    assert!(authorize(request(1, "k")).is_ok());
}

/// EXHAUSTIVE — the OUTCOME enum is total over the statuses the kernel can
/// return, and every arm is reachable from a real status or body.
///
/// The refusal taxonomy is the prompt's deliverable, and that is true of the
/// PROJECTOR as well as the gate: a `/verify` command whose only shapes are
/// `Supported` and everything-else has no authz, because it cannot tell a
/// permission decision from a verdict.
#[test]
fn the_verify_outcome_enum_is_total_and_every_arm_is_reachable() {
    let asked = doc(7, "the claim");

    // ---- the two ANSWER arms, each from the kernel's own invariant ----
    let supported = shape_verify(exchange(200, supported_body(7)), &asked);
    assert_eq!(
        supported,
        VerifyOutcome::Supported(Verdict {
            chunk_id: 7,
            decision: VerifyDecision::Supported,
            supported: true,
            match_ranges: vec![[0, 5]],
            claim: "the claim".to_string(),
        })
    );
    assert!(supported.is_supported());
    assert!(!supported.is_refusal());
    assert_eq!(supported.decision(), Some(VerifyDecision::Supported));

    // An UNSUPPORTED verdict is the route WORKING. It is an answer, it is not
    // an error, and it must not be an absence of an answer: the whole product
    // claim of `[correct]` is that this state is reachable and useful.
    let unsupported = shape_verify(exchange(200, unsupported_body(7)), &asked);
    assert_eq!(
        unsupported,
        VerifyOutcome::Unsupported(Verdict {
            chunk_id: 7,
            decision: VerifyDecision::UnsupportedClaim,
            supported: false,
            match_ranges: vec![],
            claim: "the claim".to_string(),
        })
    );
    assert!(!unsupported.is_supported());
    // THE distinction the whole route turns on: unsupported is NOT a refusal,
    // and a refusal is NOT unsupported.
    assert!(!unsupported.is_refusal());

    // ---- the four NON-answer arms ----
    for status in [401, 403] {
        let refused = shape_verify(exchange(status, json!({})), &asked);
        assert_eq!(
            refused,
            VerifyOutcome::Refused(Refused {
                status,
                scope: "verify"
            }),
            "a {status} is a permission decision, not a verdict"
        );
        assert!(refused.is_refusal());
        assert!(!refused.is_supported());
        assert_eq!(refused.decision(), None, "a refusal carries no verdict");
    }

    // 404 is the kernel's DELIBERATE blur of "no such row" and "a row your
    // record gate does not admit" (`handlers/verify.rs:110-116` returns
    // `Ok(None)` for both). This arm must not un-blur it.
    let missing = shape_verify(exchange(404, json!({})), &asked);
    assert_eq!(
        missing,
        VerifyOutcome::NoSuchChunk(NoSuchChunk {
            chunk_id: 7,
            status: 404
        })
    );
    assert!(missing.is_refusal(), "a 404 is a refusal-shaped state");
    assert!(!missing.is_supported());
    // And it carries no field from which "it does not exist" could be drawn.
    let missing_json = serde_json::to_value(&missing).expect("serializes");
    assert_eq!(
        missing_json.get("state").and_then(|v| v.as_str()),
        Some("no_such_chunk")
    );
    assert!(
        missing_json.get("exists").is_none() && missing_json.get("found").is_none(),
        "the 404 arm must not carry an existence verdict: the kernel declines to give one"
    );

    // Every other non-2xx is `Unreachable` — a degraded banner, never a verdict.
    for status in [400u16, 409, 418, 500, 503] {
        let unreachable = shape_verify(exchange(status, json!({})), &asked);
        assert_eq!(
            unreachable,
            VerifyOutcome::Unreachable(Unreachable {
                status,
                detail: "the kernel answered with a non-success status".to_string()
            }),
            "status {status} is a transport failure, not a verdict"
        );
        assert!(!unreachable.is_refusal());
        assert!(!unreachable.is_supported());
        assert_eq!(unreachable.decision(), None);
    }

    // ---- `ContractViolation`, from each of its three independent triggers ----
    //
    // Each is reachable from a real defect, and each is separate: a kernel that
    // grew a range cap, a kernel that answered for the wrong row, and a kernel
    // that contradicted itself are three different bugs.
    let not_json = shape_verify(exchange(200, json!({ "supported": true })), &asked);
    assert!(
        matches!(not_json, VerifyOutcome::ContractViolation(_)),
        "a body missing `decision` and `chunk_id` cannot be read as a verdict"
    );
    assert!(
        not_json_detail(&not_json).contains("not the published contract"),
        "the violation names that the body was unreadable: {}",
        not_json_detail(&not_json)
    );

    let other_row = shape_verify(exchange(200, supported_body(8)), &asked);
    assert_eq!(
        other_row,
        VerifyOutcome::ContractViolation(Unreachable {
            status: 200,
            detail: "the kernel answered for chunk 8 but 7 was requested".to_string()
        }),
        "a body for a different chunk would attach its ranges to the wrong citation"
    );

    let contradictory = shape_verify(
        exchange(
            200,
            json!({
                "chunk_id": 7,
                "supported": true,
                "decision": "unsupported_claim",
                "match_ranges": [[0, 5]]
            }),
        ),
        &asked,
    );
    assert!(
        matches!(contradictory, VerifyOutcome::ContractViolation(_)),
        "supported=true with decision=unsupported_claim is a contradiction, not a verdict"
    );
    assert!(contradictory_detail(&contradictory).contains("contradicts itself"));

    // And the cap: the kernel stops collecting at MAX_MATCH_RANGES, so more is
    // not this kernel's body.
    let over_cap = shape_verify(
        exchange(
            200,
            json!({
                "chunk_id": 7,
                "supported": true,
                "decision": "supported",
                "match_ranges": vec![[0usize, 1usize]; MAX_MATCH_RANGES + 1]
            }),
        ),
        &asked,
    );
    assert!(over_cap_detail(&over_cap).contains("above the kernel's 100"));

    // ---- the enum is TOTAL: six arms, six reachable, six distinct ----
    let every_arm = [
        supported.clone(),
        unsupported.clone(),
        VerifyOutcome::Refused(Refused {
            status: 403,
            scope: "verify",
        }),
        VerifyOutcome::NoSuchChunk(NoSuchChunk {
            chunk_id: 7,
            status: 404,
        }),
        VerifyOutcome::Unreachable(Unreachable {
            status: 500,
            detail: String::new(),
        }),
        VerifyOutcome::ContractViolation(Unreachable {
            status: 200,
            detail: String::new(),
        }),
    ];
    let states: Vec<&str> = every_arm.iter().map(VerifyOutcome::state_token).collect();
    let mut distinct: Vec<&str> = states.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        states.len(),
        6,
        "the verify taxonomy must have six arms; a reader enumerating them finds {}",
        states.len()
    );
    assert_eq!(
        distinct.len(),
        states.len(),
        "two arms share a wire token, so a client switch cannot tell them: {states:?}"
    );

    // And the table is not a second copy that nobody diffs: each arm's
    // SERIALIZED tag must equal the token `state_token()` claims. Drift in
    // either direction — a renamed enum variant or a hand-edited token — fails
    // here, which is the whole reason the helper returns a table at all.
    for (arm, token) in every_arm.iter().zip(states.iter()) {
        let serialized = serde_json::to_value(arm).expect("every arm serializes");
        assert_eq!(
            serialized.get("state").and_then(|v| v.as_str()),
            Some(*token),
            "the serialized tag drifted from the taxonomy table for {token}"
        );
    }

    // Anti-vacuity: the arms a renderer must not conflate are distinguished at
    // the TYPE level, so a `switch` cannot fall through.
    let rendered = serde_json::to_value(&unsupported).expect("serializes");
    assert_eq!(
        rendered.get("state").and_then(|v| v.as_str()),
        Some("unsupported"),
        "the unsupported verdict's wire token names the verdict, not an error"
    );
    assert_eq!(
        rendered.get("supported").and_then(|v| v.as_bool()),
        Some(false)
    );
    // No ranges on the unsupported arm: an empty claim of support is the one
    // thing that would let a renderer print "found at …" beside a denial.
    assert!(
        rendered.get("match_ranges").is_none(),
        "the unsupported verdict carries no ranges: the kernel reported none"
    );
}

/// The IPC seam re-authorizes the echoed doc, exactly as `recall.rs` does: a
/// caller that hands back a doc the core never authorized must be REFUSED, not
/// echoed, or the receipt would report a request that was never made.
#[test]
fn the_echoed_verification_request_must_be_one_the_core_authorized() {
    // A doc the gate would never have minted: an over-ceiling claim.
    assert_eq!(
        shape_verify_command(
            exchange(200, supported_body(1)),
            doc(1, &"c".repeat(MAX_CLAIM_CHARS + 1))
        ),
        Err(VerifyRefusal::ClaimTooLong {
            chars: MAX_CLAIM_CHARS + 1,
            max: MAX_CLAIM_CHARS
        })
    );
    // A non-resolvable id, same law.
    assert_eq!(
        shape_verify_command(exchange(200, supported_body(0)), doc(0, "k")),
        Err(VerifyRefusal::ChunkIdNotPositive { chunk_id: 0 })
    );
    // An empty-after-trim claim, same law.
    assert_eq!(
        shape_verify_command(exchange(200, supported_body(1)), doc(1, "  ")),
        Err(VerifyRefusal::ClaimEmpty)
    );

    // Anti-vacuity: the legal doc passes the seam and is classified, so the
    // re-authorization is a bound and not a blanket refusal.
    assert!(shape_verify_command(exchange(200, supported_body(1)), doc(1, "k")).is_ok());
}

/// Small helpers so the exhaustiveness pin reads as the taxonomy it is checking.
fn not_json_detail(outcome: &VerifyOutcome) -> String {
    match outcome {
        VerifyOutcome::ContractViolation(Unreachable { detail, .. }) => detail.clone(),
        other => panic!("expected a contract violation, got {other:?}"),
    }
}

fn contradictory_detail(outcome: &VerifyOutcome) -> &str {
    match outcome {
        VerifyOutcome::ContractViolation(Unreachable { detail, .. }) => detail.as_str(),
        other => panic!("expected a contract violation, got {other:?}"),
    }
}

fn over_cap_detail(outcome: &VerifyOutcome) -> &str {
    match outcome {
        VerifyOutcome::ContractViolation(Unreachable { detail, .. }) => detail.as_str(),
        other => panic!("expected a contract violation, got {other:?}"),
    }
}
