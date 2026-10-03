// The routing seam, asserted from OUTSIDE the module.
//
// ## The claim this file exists to make
//
// `workflow/routing.rs` shipped as a pure core with **zero production callers** —
// a decision nothing could reach, so an unreadable file and a load-bearing one
// looked identical. `service/routing.rs` is its first consumer. The pin below
// makes that consumer **visible to a reader of `tests/`**, and holds two properties
// that the wiring must not trade away to become useful.
//
// ## Why the two pins that matter are shaped the way they are
//
// 1. `the_routing_core_has_a_production_caller` drives the **real** service core.
//    R7's lesson: a pin that tests a reimplementation proves nothing, so this calls
//    `service::routing::route_case` and asserts on its verdict.
// 2. `the_wiring_holds_no_domain_to_queue_table` scans the seam
//    **comment-stripped**, through the one consolidated stripper in
//    `tests/common/mod.rs`. A raw scan passes on a comment that merely names the
//    thing it forbids — the false-pass class this repository treats as worse than
//    no check. This is the anti-invention pin: it must stay green while the table
//    is absent, and go red the moment one is planted.
//
// ## What "no table" means precisely
//
// Not "the word technology never appears" — the classifier's own vocabulary does,
// legitimately, in `RoutingClass`. The claim is that **no class is mapped to a
// queue**: there is no arm, match, table, or lookup anywhere in the seam that
// turns a `RoutingClass` into a queue id. The vocabulary arrives from the declared
// proposal surface or the case escalates.

mod common;

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The routing core now has a production caller.
///
/// **This is the pin that makes "no caller" false.** It drives the real service
/// seam, so a routing decision here is a decision the running system can reach.
#[test]
fn the_routing_core_has_a_production_caller() {
    use brain_server::service::routing::{HUMAN_ESCALATION_QUEUE, route_case};
    use brain_server::workflow::routing::RoutingClass;

    let conn = rusqlite::Connection::open_in_memory().expect("in-memory");
    conn.execute_batch(
        "CREATE TABLE proposals(
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             domain TEXT NOT NULL, kind TEXT NOT NULL, status TEXT NOT NULL,
             payload_json TEXT NOT NULL DEFAULT '{}');",
    )
    .expect("schema");
    conn.execute(
        "INSERT INTO proposals(domain, kind, status, payload_json)
         VALUES ('care', 'queue_declaration', 'approved', json_object('queue', 'Q-CARE-INTAKE'))",
        [],
    )
    .expect("seed");

    // A declared queue routes: the seam is reachable and decisive.
    let routed = route_case(
        &conn,
        "care",
        RoutingClass::BusinessProcess,
        Some("Q-CARE-INTAKE"),
        None,
    )
    .expect("the production seam routed");
    assert_eq!(routed.queue(), "Q-CARE-INTAKE");

    // And an undeclared one escalates to a human rather than inventing a queue.
    let escalated = route_case(
        &conn,
        "care",
        RoutingClass::BusinessProcess,
        Some("Q-NOT-DECLARED"),
        None,
    )
    .expect("the production seam escalated");
    assert_eq!(escalated.queue(), HUMAN_ESCALATION_QUEUE);
    assert!(escalated.is_escalated());
}

/// The seam holds no class→queue table.
///
/// **The anti-invention pin.** Comment-stripped, so a comment naming what it
/// forbids cannot pass it. Scans for the two shapes a hand-typed map takes in
/// Rust: a `match`/`=>` arm keyed on a class, or an array/vec pairing classes with
/// queue ids.
#[test]
fn the_wiring_holds_no_domain_to_queue_table() {
    let src = common::code_only(&read("src/service/routing.rs"));

    // Cut the test region: a fixture naming a queue is not a production map.
    let production = match src.find("#[cfg(test)]") {
        Some(idx) => &src[..idx],
        None => src.as_str(),
    };

    // A `RoutingClass::X =>` arm is a class→queue mapping in the only shape the
    // core's own type permits.
    for variant in [
        "Technology",
        "BusinessProcess",
        "Compliance",
        "Finance",
        "Vendor",
        "Assessment",
        "Infrastructure",
        "DellSupport",
        "General",
    ] {
        let arm = format!("RoutingClass::{variant}");
        assert!(
            !production.contains(&arm),
            "the seam maps RoutingClass::{variant} somewhere - a class->queue table has been invented"
        );
    }

    // And no queue id is typed into the seam. The one id the system agrees on is
    // re-exported from the core, never restated.
    let typed_queue_literals = production.matches("\"Q-").count();
    assert_eq!(
        typed_queue_literals, 0,
        "the seam types {typed_queue_literals} queue id(s); the vocabulary is declared, not typed"
    );
}

/// The core's own law still holds: it holds no vocabulary of its own.
///
/// Stated from outside because the core cannot assert its own absence, and because
/// a second copy of the queue list is the exact defect the module doc warns about.
#[test]
fn the_routing_core_holds_no_queue_list_of_its_own() {
    let src = common::code_only(&read("src/workflow/routing.rs"));
    let production = match src.find("#[cfg(test)]") {
        Some(idx) => &src[..idx],
        None => src.as_str(),
    };
    // Exactly one queue id may appear in production: the escalation target.
    // Counted as an occurrence anywhere in the production region rather than by
    // line start — the core declares it as `pub const ESCALATION_QUEUE: &str =
    // "Q-OPS-ESCALATION";`, so a line-anchored scan reads zero and would pass a
    // core that had acquired a whole vocabulary. (The R8 prompt's line-start
    // warning is about backwards walks; here the lesson is the same one — a
    // detector that cannot see its own subject proves nothing.)
    let queue_literals = production.matches("\"Q-").count();
    assert_eq!(
        queue_literals, 1,
        "the core declares {queue_literals} queue ids; only the escalation target may be literal"
    );
}

/// Offers carry no assignment.
///
/// Asserted against the **type**, not a source scan: destructuring `Offer` into its
/// exact field list means adding an `assignee` field breaks this test at compile
/// time rather than passing a grep. An assignment is not something this seam can
/// express.
#[test]
fn routing_offers_never_assign() {
    use brain_server::workflow::routing::{Offer, OfferBasis};

    let Offer {
        principal,
        basis,
        basis_flags,
    } = Offer {
        principal: "agent-a".to_string(),
        basis: OfferBasis::Idle,
        basis_flags: vec!["Presence"],
    };
    assert_eq!(principal, "agent-a");
    assert_eq!(basis, OfferBasis::Idle);
    assert_eq!(basis_flags, vec!["Presence"]);

    // The escalation id is re-exported rather than restated, so there is one
    // declaration of it in the tree.
    assert_eq!(
        brain_server::service::routing::HUMAN_ESCALATION_QUEUE,
        brain_server::workflow::routing::ESCALATION_QUEUE
    );
}

/// The seam added no table and no route.
///
/// Read from the migration so the claim is about the shipped schema: a seam that
/// quietly created `crew_queue_declarations` would be a second way to declare a
/// queue, which is the drift the core's module doc warns against.
#[test]
fn the_routing_seam_created_no_table() {
    let migration = common::code_only(&read("src/migration.rs"));
    assert!(
        !migration.contains("crew_queue_declarations"),
        "the routing seam created a queue table; the vocabulary must ride the governed proposal surface"
    );
    assert!(
        !migration.contains("crew_shift_declarations"),
        "the routing seam created a shift table; shifts already have one"
    );
}

/// No new route and no wire change.
///
/// The seam is a service core plus a CLI-reachable path, so `openapi.yaml` and the
/// guard table must be untouched. Asserted rather than assumed: a route added
/// without its openapi row is the wire-contract discipline's one hard law.
#[test]
fn the_routing_seam_added_no_route() {
    let openapi = read("openapi.yaml");
    assert!(
        !openapi.contains("/ops/routing") && !openapi.contains("/routing"),
        "the routing seam appears in the openapi document but this round added no route"
    );
}
