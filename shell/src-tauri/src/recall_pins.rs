//! P1 — the core's pins. Each is behavioral: it drives the real functions and
//! compares, and the red-proofs are in the register row rather than asserted.
//!
//! `#[cfg(test)]` lives here rather than in a separate integration test
//! because the subject is the crate's own commands — a `tests/` file could
//! reach them too, but keeping the module private means a pin cannot
//! accidentally become the only caller of a production path.

use super::*;
use serde_json::json;

fn query(text: &str) -> RecallQuery {
    RecallQuery {
        query: text.to_string(),
        domain: None,
        limit: None,
    }
}

fn doc(text: &str) -> RecallDoc {
    RecallDoc {
        v: 1,
        query: text.to_string(),
        domain: None,
        limit: DEFAULT_LIMIT,
        provenance: true,
    }
}

fn exchange(status: u16, body: serde_json::Value) -> WireExchange {
    WireExchange {
        status,
        body: body.to_string(),
    }
}

/// The bounds law, at the kernel's own values. Behavioural: it drives
/// `authorize` at each boundary the kernel itself declares
/// (`handlers/recall.rs` refuses outside `MIN_LIMIT..=MAX_LIMIT`).
#[test]
fn bounds_are_enforced_in_the_core_at_the_kernels_own_thresholds() {
    // A legal request is authorized, and the authorized body is the request.
    let authorized = authorize(query("wire bridge")).expect("a plain query must authorize");
    assert_eq!(authorized.doc.query, "wire bridge");
    assert_eq!(authorized.query_bytes, "wire bridge".len());

    // Empty is refused, not sent as an empty string.
    assert_eq!(authorize(query("")), Err(RecallRefusal::QueryEmpty));

    // The query ceiling is INCLUSIVE at the kernel's 2_000 bytes: the kernel
    // refuses above it, so the core must refuse above it too — and must not
    // refuse AT it. (This arm is the red-proof: an exclusive bound here refuses
    // a request the kernel would have served.)
    let at_ceiling = "q".repeat(MAX_QUERY_BYTES);
    assert_eq!(
        authorize(query(&at_ceiling)).map(|a| a.query_bytes),
        Ok(MAX_QUERY_BYTES),
        "a query of exactly MAX_QUERY_BYTES must be authorized — the kernel serves it"
    );
    let over_ceiling = "q".repeat(MAX_QUERY_BYTES + 1);
    assert_eq!(
        authorize(query(&over_ceiling)),
        Err(RecallRefusal::QueryTooLong {
            bytes: MAX_QUERY_BYTES + 1,
            max: MAX_QUERY_BYTES
        })
    );

    // The result-count ceiling is inclusive at the kernel's MAX_LIMIT = 100.
    assert_eq!(
        authorize(RecallQuery {
            limit: Some(MAX_LIMIT),
            ..query("k")
        })
        .map(|a| a.doc.limit),
        Ok(MAX_LIMIT),
        "limit == MAX_LIMIT must be authorized — the kernel's own range is inclusive"
    );
    assert_eq!(
        authorize(RecallQuery {
            limit: Some(MAX_LIMIT + 1),
            ..query("k")
        }),
        Err(RecallRefusal::LimitOutOfRange {
            requested: MAX_LIMIT + 1,
            min: MIN_LIMIT,
            max: MAX_LIMIT
        })
    );
    assert_eq!(
        authorize(RecallQuery {
            limit: Some(0),
            ..query("k")
        }),
        Err(RecallRefusal::LimitOutOfRange {
            requested: 0,
            min: MIN_LIMIT,
            max: MAX_LIMIT
        })
    );

    // The default is the kernel's own default, not a number invented here.
    assert_eq!(
        authorize(query("d")).map(|a| a.doc.limit),
        Ok(DEFAULT_LIMIT)
    );
    assert_eq!(DEFAULT_LIMIT, 5, "the kernel's DEFAULT_RECALL_LIMIT is 5");

    // A domain is an isolation label, not prose.
    let long_domain = "d".repeat(MAX_DOMAIN_BYTES + 1);
    assert_eq!(
        authorize(RecallQuery {
            domain: Some(long_domain.clone()),
            ..query("d")
        }),
        Err(RecallRefusal::DomainTooLong {
            bytes: MAX_DOMAIN_BYTES + 1,
            max: MAX_DOMAIN_BYTES
        })
    );
    assert_eq!(
        authorize(RecallQuery {
            domain: Some("two words".to_string()),
            ..query("d")
        }),
        Err(RecallRefusal::DomainNotALabel)
    );
    assert!(matches!(
        authorize(RecallQuery { domain: Some("legal".to_string()), ..query("d") }),
        Ok(AuthorizedRecall { doc: RecallDoc { domain: Some(ref d), .. }, .. }) if d == "legal"
    ));
}

/// The authz half. There is no field through which a principal can arrive, and
/// a caller who smuggles one is REFUSED rather than ignored — a silently
/// dropped field is how a synthesized principal reaches production looking
/// like the operator's.
#[test]
fn the_core_takes_no_principal_and_refuses_one_smuggled_in() {
    for field in [
        "authorization",
        "token",
        "principal",
        "bearer",
        "Authorization",
    ] {
        let attempt = json!({ "query": "q", field: "Bearer forged" });
        let parsed = serde_json::from_value::<RecallQuery>(attempt.clone());
        assert!(
            parsed.is_err(),
            "a recall request carrying `{field}` must be refused, not parsed: {attempt}"
        );
        let error = parsed
            .expect_err("the refusal is the assertion")
            .to_string();
        assert!(
            error.contains(field),
            "the refusal must NAME the field it refused, so the bypass is legible: {error}"
        );
    }

    // And the authorized body carries no auth material at all: the caller's own
    // bearer rides the webview's middleware, so the core has nothing to add.
    // Compared as a SET, not as an ordered list: JSON object key order is not
    // semantic, and a pin asserting insertion order would fail for a reason
    // that has nothing to do with authz.
    let authorized = authorize(query("q")).expect("a plain query must authorize");
    let body = serde_json::to_value(&authorized.doc).expect("the authorized body serializes");
    let object = body.as_object().expect("the authorized body is an object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, vec!["limit", "provenance", "query", "v"]);
    for forbidden in ["authorization", "token", "principal", "bearer"] {
        assert!(
            !object.contains_key(forbidden),
            "the core must never emit `{forbidden}`; it has no principal to emit"
        );
    }

    // Anti-vacuity: a field the core DOES own still round-trips, so the
    // refusal above is about unknown fields and not about a parser that
    // refuses everything.
    assert!(serde_json::from_value::<RecallQuery>(json!({ "query": "q", "limit": 7 })).is_ok());
}

/// `low_confidence` is an ANSWER. It is never an error path, and it never
/// arrives without a decision.
#[test]
fn low_confidence_is_an_answer_and_never_an_error() {
    let outcome = shape_recall(
        exchange(
            200,
            json!({ "hits": [], "decision": "low_confidence", "telemetry": { "confidence": 0.11 } }),
        ),
        &doc("q"),
    );

    // Named as an abstention, not as an error and not as an absence.
    assert!(
        matches!(&outcome, RecallOutcome::Abstained(answer) if answer.decision == Decision::LowConfidence),
        "an abstention must be Abstained, got {outcome:?}"
    );

    // The decision is present ON the abstention — that is the arm.
    assert_eq!(outcome.decision(), Some(Decision::LowConfidence));

    // And the confidence the kernel sent is carried, which is only possible
    // because the core asks for provenance.
    match &outcome {
        RecallOutcome::Abstained(answer) => {
            assert_eq!(answer.confidence, Some(0.11));
            assert!(answer.passages.is_empty());
        }
        other => panic!("expected an abstention, got {other:?}"),
    }

    // The serialized form speaks the WIRE's vocabulary, not Plan 04 §2's.
    // Plan 04 §2 writes `decision: "answered" | "low_confidence"`; the
    // published enum is `[ok, low_confidence]` and the core renames nothing.
    //
    // The outcome serializes FLAT — serde's internal tag puts `state` beside
    // the answer's own fields rather than nesting them under the variant name
    // — so a renderer reads `state` and then `decision`/`passages` directly.
    let wire = serde_json::to_value(&outcome).expect("the outcome serializes");
    assert_eq!(wire["state"], "abstained");
    assert_eq!(wire["decision"], "low_confidence");

    // An answered response carries `ok` — never "answered".
    let answered = shape_recall(
        exchange(
            200,
            json!({ "hits": [{ "id": 1, "content": "t", "score": 0.9 }], "decision": "ok" }),
        ),
        &doc("q"),
    );
    assert_eq!(
        serde_json::to_value(&answered).expect("serializes")["decision"],
        "ok"
    );
}

/// Every response carries a decision, and a value outside the published enum
/// is a contract violation rather than a third client state.
#[test]
fn every_outcome_carries_a_decision_and_the_enum_is_the_kernels() {
    // A 200 with NO decision is a violation, not a best-effort answer.
    let missing = shape_recall(exchange(200, json!({ "hits": [] })), &doc("q"));
    assert!(
        matches!(&missing, RecallOutcome::ContractViolation(_)),
        "a body with no decision must be a ContractViolation, got {missing:?}"
    );

    // A third value is a violation too. This is the arm that keeps a client
    // from inventing `answered` and calling it a state.
    let third = shape_recall(
        exchange(200, json!({ "hits": [], "decision": "answered" })),
        &doc("q"),
    );
    assert!(
        matches!(&third, RecallOutcome::ContractViolation(_)),
        "a decision outside the published enum must be a ContractViolation, got {third:?}"
    );

    // Unparseable JSON is a violation, and — the arm that matters — it is NOT
    // an absence. A body that cannot be read must never render as "no results".
    let garbage = shape_recall(exchange(200, json!("not an object")), &doc("q"));
    assert!(matches!(&garbage, RecallOutcome::ContractViolation(_)));
    assert!(!garbage.has_passages());

    // Both real decisions classify, and both carry one.
    for (body, expected) in [
        (json!({ "hits": [], "decision": "ok" }), Decision::Ok),
        (
            json!({ "hits": [], "decision": "low_confidence" }),
            Decision::LowConfidence,
        ),
    ] {
        let outcome = shape_recall(exchange(200, body), &doc("q"));
        assert_eq!(outcome.decision(), Some(expected));
    }
}

/// A refusal is not an absence. Collapsing the two is a confidentiality leak,
/// so the distinction is structural: a different state, a different shape, and
/// no passages on either side of the wall for a caller to confuse.
#[test]
fn a_refusal_is_structurally_distinguishable_from_an_absence() {
    let requested = doc("q");

    // The absence: a 200 the kernel served, with nothing in it.
    let absence = shape_recall(
        exchange(200, json!({ "hits": [], "decision": "ok" })),
        &requested,
    );
    assert!(
        matches!(&absence, RecallOutcome::Absent(answer) if answer.decision == Decision::Ok),
        "expected Absent, got {absence:?}"
    );
    assert_eq!(absence.decision(), Some(Decision::Ok));
    assert!(!absence.has_passages());
    assert!(!absence.is_refusal());

    // The refusals, both statuses the kernel can return for absent authority.
    for status in [401u16, 403u16] {
        let refused = shape_recall(
            WireExchange {
                status,
                body: r#"{"code":"unauthorized"}"#.to_string(),
            },
            &requested,
        );
        assert!(
            refused.is_refusal(),
            "a {status} must be a refusal, got {refused:?}"
        );

        // Structurally different: a refusal carries a status and a scope, and
        // carries NO decision and NO passages — so a renderer that branches on
        // "did I get passages?" cannot silently draw it as an empty answer.
        match &refused {
            RecallOutcome::Refused(r) => {
                assert_eq!(r.status, status);
                assert_eq!(r.scope, "recall");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
        let wire = serde_json::to_value(&refused).expect("serializes");
        assert_eq!(wire["state"], "refused");
        assert_eq!(wire["status"], status);
        // THE arm, and it is stronger than a nesting check: on a refusal the
        // `decision` and `passages` keys are ABSENT from the object entirely,
        // where an absence carries both (passages empty, decision `ok`). So a
        // renderer branching on `passages.length` cannot reach a refusal at
        // all, and one branching on `decision === "ok"` reads it as neither
        // answered nor abstained.
        assert!(
            wire.get("passages").is_none(),
            "a refusal must not carry a passages key at all: {wire}"
        );
        assert!(
            wire.get("decision").is_none(),
            "a refusal must not carry a decision key at all: {wire}"
        );

        // The absence, for the contrast: same keys present, passages empty.
        let absence_wire = serde_json::to_value(&absence).expect("serializes");
        assert_eq!(absence_wire["state"], "absent");
        assert_eq!(absence_wire["decision"], "ok");
        assert_eq!(absence_wire["passages"].as_array().map(Vec::len), Some(0));
        assert!(absence_wire.get("passages").is_some());

        // And the two serialized shapes are not interchangeable.
        assert_ne!(
            wire, absence_wire,
            "a refusal and an absence must not share a rendered shape"
        );

        // And the refusal does not become an absence when the kernel's error
        // body happens to parse as a recall body.
        let sneaky = shape_recall(
            WireExchange {
                status,
                body: json!({ "hits": [], "decision": "ok" }).to_string(),
            },
            &requested,
        );
        assert!(
            sneaky.is_refusal(),
            "a {status} whose body looks like a recall answer must still be a refusal"
        );
    }

    // A 401/403 is a refusal even when the body is absent entirely.
    let empty = shape_recall(
        WireExchange {
            status: 403,
            body: String::new(),
        },
        &requested,
    );
    assert!(empty.is_refusal());
}

/// Plan 04 §5: kernel unreachable is degraded, and NEVER a silent empty result.
#[test]
fn an_unreachable_kernel_is_never_an_absence() {
    for status in [0u16, 404, 422, 500, 503] {
        let outcome = shape_recall(exchange(status, json!({})), &doc("q"));
        match &outcome {
            RecallOutcome::Unreachable(f) => assert_eq!(f.status, status),
            other => panic!("a {status} must be Unreachable, got {other:?}"),
        }
        assert!(!outcome.has_passages());
        assert!(!outcome.is_refusal());
        // Not an absence: the discriminant differs from Absent's.
        assert_eq!(
            serde_json::to_value(&outcome).expect("serializes")["state"],
            "unreachable"
        );
    }
}

/// The projector reads the wire as the kernel writes it — including validity,
/// which is the field whose absence would let a stale fact be presented as
/// current (Plan 06 §2), and the cross-domain disclosure, which the spec says
/// must be "visible, never silent".
#[test]
fn the_projection_carries_validity_and_the_cross_domain_disclosure() {
    let body = json!({
        "hits": [{
            "id": 42,
            "title": "t",
            "content": "the passage text",
            "snippet": "the passage",
            "score": 0.87,
            "domain": "legal",
            "evidence": { "valid_from": "2024-01-01T00:00:00Z", "valid_to": "2025-01-01T00:00:00Z" },
            "flagged": true,
            "ingest_kind": "markdown",
            "memory_kind": "fact",
            "authority": 0.6
        }],
        "decision": "ok",
        "domains_searched": ["legal", "global"],
        "included_global": true,
        "telemetry": { "confidence": 0.93 }
    });
    let requested = RecallDoc {
        v: 1,
        query: "q".to_string(),
        domain: Some("legal".to_string()),
        limit: 5,
        provenance: true,
    };
    let outcome = shape_recall(exchange(200, body), &requested);
    let RecallOutcome::Answered(answer) = &outcome else {
        panic!("expected Answered, got {outcome:?}");
    };

    // Positional: the passage is asserted field by field, in order, because a
    // count would not notice the fields arriving empty.
    let passage = answer.passages.first().expect("one passage");
    assert_eq!(passage.id, 42);
    assert_eq!(passage.text, "the passage text");
    assert_eq!(passage.score, 0.87);
    assert_eq!(passage.domain.as_deref(), Some("legal"));
    assert_eq!(passage.valid_from.as_deref(), Some("2024-01-01T00:00:00Z"));
    assert_eq!(passage.valid_to.as_deref(), Some("2025-01-01T00:00:00Z"));
    assert!(passage.flagged);
    assert_eq!(passage.ingest_kind.as_deref(), Some("markdown"));
    assert_eq!(passage.memory_kind.as_deref(), Some("fact"));
    assert_eq!(answer.confidence, Some(0.93));

    // The searched echo is the AUTHORIZED request, not the caller's original:
    // that is what makes a clamped or refused bound visible.
    assert_eq!(answer.searched.query, "q");
    assert_eq!(answer.searched.domain.as_deref(), Some("legal"));
    assert_eq!(answer.searched.limit, 5);
    assert_eq!(answer.searched.domains_searched, vec!["legal", "global"]);
    assert_eq!(answer.searched.included_global, Some(true));

    // Plan 04 §2 named two fields the wire does not carry. They are ABSENT,
    // and this is the arm: the core does not invent an `owner` (an authz fact)
    // or a `chain_head` (a provenance fact).
    let wire = serde_json::to_value(&outcome).expect("serializes");
    let passage_wire = &wire["passages"][0];
    assert!(
        passage_wire.get("owner").is_none(),
        "owner is not on RecallHit and must not be invented"
    );
    assert!(
        passage_wire.get("chain_head").is_none(),
        "chain_head is not on RecallHit and must not be invented"
    );

    // Additive tolerance: a kernel that grows a field must not break the shell,
    // so an unknown key is ignored rather than refused.
    let grown = shape_recall(
        exchange(
            200,
            json!({ "hits": [{ "id": 1, "content": "t", "some_future_field": 42 }], "decision": "ok", "future_top": true }),
        ),
        &requested,
    );
    assert!(
        grown.has_passages(),
        "an additive kernel field must not break the projection"
    );
}

/// The `searched` echo reports what was really sent, so the doc the caller
/// hands BACK is re-authorized against the same bounds. Without this the echo
/// is attacker-controlled: a caller could report a query and a domain the core
/// never authorized, and the audit trail of what was asked for would be a lie.
#[test]
fn the_echoed_request_must_be_one_the_core_actually_authorized() {
    let ok_body = json!({ "hits": [], "decision": "ok" }).to_string();

    // A doc inside the bounds is accepted and echoed as sent.
    let good = shape_recall_command(
        WireExchange {
            status: 200,
            body: ok_body.clone(),
        },
        doc("q"),
    )
    .expect("a doc inside the bounds must be accepted");
    assert!(matches!(&good, RecallOutcome::Absent(answer) if answer.searched.query == "q"));

    // Every bound refuses here too — not just at the gate. An over-long query,
    // an out-of-range limit and a prose domain are all refused, so the echo can
    // never carry a request the gate would have rejected.
    let over_long = RecallDoc {
        query: "q".repeat(MAX_QUERY_BYTES + 1),
        ..doc("q")
    };
    assert_eq!(
        shape_recall_command(
            WireExchange {
                status: 200,
                body: ok_body.clone()
            },
            over_long
        ),
        Err(RecallRefusal::QueryTooLong {
            bytes: MAX_QUERY_BYTES + 1,
            max: MAX_QUERY_BYTES
        })
    );

    let over_limit = RecallDoc {
        limit: MAX_LIMIT + 1,
        ..doc("q")
    };
    assert_eq!(
        shape_recall_command(
            WireExchange {
                status: 200,
                body: ok_body.clone()
            },
            over_limit
        ),
        Err(RecallRefusal::LimitOutOfRange {
            requested: MAX_LIMIT + 1,
            min: MIN_LIMIT,
            max: MAX_LIMIT
        })
    );

    let prose_domain = RecallDoc {
        domain: Some("two words".to_string()),
        ..doc("q")
    };
    assert_eq!(
        shape_recall_command(
            WireExchange {
                status: 200,
                body: ok_body
            },
            prose_domain
        ),
        Err(RecallRefusal::DomainNotALabel)
    );

    // An empty query has no echo either: it names nothing.
    assert_eq!(
        shape_recall_command(
            WireExchange {
                status: 200,
                body: "{}".to_string()
            },
            RecallDoc {
                query: String::new(),
                ..doc("q")
            },
        ),
        Err(RecallRefusal::QueryEmpty)
    );
}
