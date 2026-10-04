//! The routing seam, asserted from OUTSIDE the module.
//
//! ## The claim this file exists to make
//
//! `workflow/routing.rs` shipped as a pure core with **zero production callers** —
//! a decision nothing could reach, so an unreadable file and a load-bearing one
//! looked identical. `service/routing.rs` is its first consumer, and `brain route`
//! is the production caller that consumer had none of. The pins below make that
//! visible to a reader of `tests/`, and hold the properties the wiring must not
//! trade away to become useful.
//
//! ## Why the pins that matter are shaped the way they are
//
//! 1. `the_routing_seam_has_a_production_caller` **walks `src/`**. The claim it
//!    makes is about the tree, so only a walk can prove it — and it deliberately
//!    excludes `tests/`, because a reachability pin satisfied by the test file
//!    beside it proves nothing. This is the pin whose absence let an earlier
//!    increment's claim stand.
//! 2. `the_routing_seam_routes_a_case_and_escalates_an_undeclared_one` drives the
//!    **real** service seam. R7's lesson: a pin that tests a reimplementation
//!    proves nothing, so this calls `service::routing::route_case` and asserts on
//!    its verdict. It was previously named `…_has_a_production_caller`, which
//!    claimed more than its body proved — it established *callable*, not *called*.
//! 3. `the_wiring_holds_no_domain_to_queue_table` scans the seam
//!    **comment-stripped**, through the one consolidated stripper in
//!    `tests/common/mod.rs`. A raw scan passes on a comment that merely names the
//!    thing it forbids — the false-pass class this repository treats as worse than
//!    no check. This is the anti-invention pin: it must stay green while the table
//!    is absent, and go red the moment one is planted.
//! 4. `the_routing_class_label_refuses_what_the_classifier_never_emitted` covers
//!    the label→class resolver the verb needs. Two `RoutingClass` types share the
//!    name in this tree, and a `From` between them would be the hand-typed mapping
//!    pin 3 exists to catch wearing a trait impl.
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

/// The routing seam routes a case, and escalates one it cannot.
///
/// **The name says what the body proves.** This used to be called
/// `the_routing_core_has_a_production_caller`, which claimed more than it proved: it
/// *drove* `route_case` from a test, which establishes the seam is **callable**, not
/// that anything **calls** it. Those are different claims, and the stronger one was
/// the one nobody had earned. The claim that a production caller exists is a separate
/// pin below, and it walks the tree rather than calling anything.
#[test]
fn the_routing_seam_routes_a_case_and_escalates_an_undeclared_one() {
    use brain_server::service::routing::{HUMAN_ESCALATION_QUEUE, route_case};
    use brain_server::workflow::routing::RoutingClass;

    let conn = rusqlite::Connection::open_in_memory().expect("in-memory");
    // The REAL proposals schema, as `migration.rs` defines it. A fixture that
    // invents a column is a fixture that cannot catch a query against a column
    // nothing writes — which is exactly how this seam shipped broken.
    conn.execute_batch(
        "CREATE TABLE proposals(
             id INTEGER PRIMARY KEY, kind TEXT NOT NULL DEFAULT 'fact',
             content TEXT NOT NULL, source TEXT, authority REAL,
             observed_at INTEGER, novelty REAL NOT NULL, conflict_with INTEGER,
             salience REAL NOT NULL DEFAULT 0.5,
             status TEXT NOT NULL DEFAULT 'pending', created_at INTEGER NOT NULL,
             decided_at INTEGER, domain TEXT NOT NULL DEFAULT 'global');",
    )
    .expect("schema");
    // The queue id IS the proposal's content.
    conn.execute(
        "INSERT INTO proposals(kind, content, novelty, status, created_at, domain)
         VALUES ('queue_declaration', 'Q-CARE-INTAKE', 0.5, 'approved', 0, 'care')",
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
        "SubjectMatter",
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

/// The routing seam HAS a production caller.
///
/// **This is the pin whose absence let an earlier increment's claim stand.** The
/// claim it makes is a claim about the *tree*, not about behaviour, so it is proved
/// by walking the tree.
///
/// ## What it actually proves, and what it does not
///
/// A call site somewhere under `src/` is **not** enough, and this pin was wrong
/// about that until its own red-proof caught it: deleting the dispatch entry left
/// `fn cmd_route` in the file, complete with its call to the seam, and the walk
/// happily reported success against an **orphaned function the binary can never
/// invoke**. Presence of a call is the *callable* claim again, wearing a new name.
///
/// So the property proved here is **reachability from dispatch**: the call site must
/// sit inside a function that the `SUBCOMMANDS` table actually names in its `run:`
/// field. A function nothing dispatches is dead code no matter how loudly it calls.
///
/// ## The rest of the non-vacuity
///
/// - **It walks `src/`, not `tests/`.** A reachability pin satisfied by the test file
///   beside it would prove nothing — and that is how the earlier state arose:
///   `route_case` was reachable from `tests/` and nowhere else, which reads
///   identically to "reachable".
/// - **Comments are stripped first**, through the one consolidated stripper, so a
///   comment *naming* a call site cannot pass it.
/// - **Each file's `#[cfg(test)]` region is cut**, so a call that exists only under
///   test is not a production caller.
/// - **`axum::routing` is excluded** — the unrelated import that makes a naive grep
///   look busy.
/// - **The walker is self-checked** (`scanned > 100`): a walker that cannot see the
///   tree would otherwise report "no caller" for the wrong reason, and would go on
///   reporting it forever.
#[test]
fn the_routing_seam_has_a_production_caller() {
    // The dispatch table: every `run:` target it names is reachable from the
    // binary's argument dispatch, so a call inside one of these functions is a
    // call a user can actually make.
    let brain_src = common::code_only(&read("src/bin/brain.rs"));
    let table = brain_src
        .split("const SUBCOMMANDS")
        .nth(1)
        .expect("SUBCOMMANDS table missing from brain.rs — the dispatch law is broken");
    let table = &table[..table.find("\n];").expect("SUBCOMMANDS table never closes")];
    let dispatched: Vec<&str> = table
        .lines()
        .filter_map(|line| {
            let idx = line.find("run: ")?;
            let rest = &line[idx + "run: ".len()..];
            let end = rest.find(',')?;
            Some(&rest[..end])
        })
        .collect();
    assert!(
        dispatched.len() >= 40,
        "the dispatch table yielded {} run targets — the extractor is broken and this pin \
         would pass or fail for the wrong reason",
        dispatched.len()
    );

    // Walk `src/` for the seam's entry points, then keep only the call sites that
    // live inside a function the dispatch table names.
    let mut reachable_call_sites: Vec<String> = Vec::new();
    let mut any_call_sites: Vec<String> = Vec::new();
    let mut scanned = 0usize;

    let mut stack = vec![repo_root().join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            scanned += 1;
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let code = common::code_only(&raw);
            let production = match code.find("#[cfg(test)]") {
                Some(idx) => &code[..idx],
                None => code.as_str(),
            };
            let rel = path
                .strip_prefix(repo_root())
                .unwrap_or(&path)
                .display()
                .to_string();
            // The enclosing top-level function, so a call can be attributed to the
            // entry point that reaches it.
            let mut enclosing: Option<String> = None;
            for (n, line) in production.lines().enumerate() {
                if line.contains("axum::routing") {
                    continue;
                }
                let trimmed = line.trim_start();
                if trimmed.starts_with("fn ") || trimmed.starts_with("pub fn ") {
                    let after = trimmed
                        .strip_prefix("pub fn ")
                        .or_else(|| trimmed.strip_prefix("fn "))
                        .unwrap_or(trimmed);
                    let name: String = after
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    enclosing = Some(name);
                }
                let is_call = line.contains("route_case(")
                    || line.contains("offers_for(")
                    || line.contains("route_to_queue(");
                if !is_call {
                    continue;
                }
                // The seam's own definition is not a caller of itself.
                if rel == "src/service/routing.rs" {
                    continue;
                }
                let site = format!("{rel}:{}", n + 1);
                any_call_sites.push(site.clone());
                let reachable = enclosing
                    .as_deref()
                    .is_some_and(|name| dispatched.contains(&name));
                if reachable {
                    reachable_call_sites.push(site);
                }
            }
        }
    }

    assert!(
        scanned > 100,
        "the walk visited only {scanned} source file(s); a walker that cannot see the tree \
         passes everything vacuously"
    );
    assert!(
        !reachable_call_sites.is_empty(),
        "NO REACHABLE production caller of the routing seam exists. Call sites found in \
         production code: {any_call_sites:?} — but none of them sits inside a function the \
         SUBCOMMANDS table dispatches, so the seam is callable and nothing a user can run \
         calls it. Either the caller is an orphan (dead code), or the dispatch entry is \
         missing. Both read identically to a deleted file."
    );
}

/// The seam's declaration query runs against the SHIPPED schema.
///
/// **This is the pin whose absence let a broken seam ship green.** The read query
/// named `proposals.payload_json`, a column `migration.rs` never creates and
/// `service::review::insert_proposal` never writes. It was invisible because every
/// fixture that exercised the seam **invented the column** — so 3100+ tests agreed
/// with each other and none of them agreed with a real database.
///
/// The seam is now driven here against the actual `migration.rs` column list, and
/// the read must succeed and route. A fixture that restates the schema is a
/// fixture that can only ever confirm itself.
#[test]
fn the_declaration_read_runs_against_the_shipped_schema() {
    use brain_server::service::routing::{HUMAN_ESCALATION_QUEUE, route_case};
    use brain_server::workflow::routing::RoutingClass;

    // The columns `migration.rs` actually creates, read from the migration itself
    // rather than restated here — a second copy is what this pin exists to refuse.
    let migration = common::code_only(&read("src/migration.rs"));
    let start = migration
        .find("CREATE TABLE IF NOT EXISTS proposals")
        .expect("the proposals table must be declared in the migration");
    let body = &migration[start..];
    // The statement ends at its OWN closing paren — the column list contains none,
    // but slice to the terminator that closes the statement rather than the first
    // one in the file. The `)` is INCLUDED: the DDL is applied as SQL.
    let end = body
        .find("\n         );\",")
        .or_else(|| body.find("\n         );"))
        .expect("the proposals DDL must close");
    let ddl = format!("{})\n", &body[..end]);
    // `domain` arrives by a later ALTER, not in the base DDL — so its ABSENCE
    // here is the point, and asserting both halves keeps the fixture honest about
    // which columns come from where.
    assert!(
        ddl.contains("content") && !ddl.contains("domain"),
        "the proposals DDL shape moved: a base-DDL column set this fixture adds by ALTER has \
         moved into (or out of) the base statement, and the ALTER list below is now wrong"
    );

    let conn = rusqlite::Connection::open_in_memory().expect("in-memory");
    conn.execute_batch(&ddl)
        .expect("the shipped DDL must apply");
    for (col, def) in [
        ("lint_json", "TEXT"),
        ("domain", "TEXT NOT NULL DEFAULT 'global'"),
    ] {
        conn.execute(&format!("ALTER TABLE proposals ADD COLUMN {col} {def}"), [])
            .expect("the additive columns the migration adds");
    }
    conn.execute(
        "INSERT INTO proposals(kind, content, novelty, status, created_at, domain)
         VALUES ('queue_declaration', 'Q-CARE-INTAKE', 0.5, 'approved', 0, 'care')",
        [],
    )
    .expect("seed");

    // The read must SUCCEED — before the fix this raised
    // `no such column: payload_json` and the verb exited 1.
    let routed = route_case(
        &conn,
        "care",
        RoutingClass::BusinessProcess,
        Some("Q-CARE-INTAKE"),
        None,
    )
    .expect("the seam must read the shipped schema, not a fixture of its own imagining");
    assert_eq!(routed.queue(), "Q-CARE-INTAKE");
    assert!(!routed.is_escalated());
    assert_eq!(routed.declared_queues, vec!["Q-CARE-INTAKE".to_string()]);

    // And the escalation half, so the pin is not satisfied by a query that
    // always answers "declared".
    let escalated = route_case(
        &conn,
        "care",
        RoutingClass::BusinessProcess,
        Some("Q-NOT-DECLARED"),
        None,
    )
    .expect("escalation path");
    assert_eq!(escalated.queue(), HUMAN_ESCALATION_QUEUE);
}

/// A declaration that is not a queue id is not a queue.
///
/// The queue id is operator- or agent-authored text that cleared the governed
/// surface, and it names a destination real work routes to — so the read side
/// refuses anything that is not a short graphic token. Without this, a declaration
/// carrying a newline or an unbounded string rides into the routing receipt.
#[test]
fn a_malformed_declaration_is_not_a_queue() {
    use brain_server::service::routing::route_case;
    use brain_server::workflow::routing::RoutingClass;

    let conn = rusqlite::Connection::open_in_memory().expect("in-memory");
    conn.execute_batch(
        "CREATE TABLE proposals(
             id INTEGER PRIMARY KEY, kind TEXT NOT NULL DEFAULT 'fact',
             content TEXT NOT NULL, source TEXT, authority REAL,
             observed_at INTEGER, novelty REAL NOT NULL, conflict_with INTEGER,
             salience REAL NOT NULL DEFAULT 0.5,
             status TEXT NOT NULL DEFAULT 'pending', created_at INTEGER NOT NULL,
             decided_at INTEGER, domain TEXT NOT NULL DEFAULT 'global');",
    )
    .expect("schema");
    for bad in [
        "Q-CARE\nINJECTED",  // a newline splits a receipt line
        "Q-CARE WITH SPACE", // not a token
        "",                  // the empty string is not a destination
        &"Q-".repeat(200),   // unbounded length
    ] {
        conn.execute(
            "INSERT INTO proposals(kind, content, novelty, status, created_at, domain)
             VALUES ('queue_declaration', ?1, 0.5, 'approved', 0, 'care')",
            rusqlite::params![bad],
        )
        .expect("seed");
    }

    let routed = route_case(
        &conn,
        "care",
        RoutingClass::General,
        Some("Q-CARE-INTAKE"),
        None,
    )
    .expect("routed");
    assert!(
        routed.declared_queues.is_empty(),
        "malformed declarations entered the vocabulary: {:?}",
        routed.declared_queues
    );
    assert!(
        routed.is_escalated(),
        "nothing was declared, so nothing routes"
    );

    // A well-formed declaration alongside the malformed ones still routes — one
    // bad row must not make the whole vocabulary unreadable.
    conn.execute(
        "INSERT INTO proposals(kind, content, novelty, status, created_at, domain)
         VALUES ('queue_declaration', 'Q-CARE-INTAKE', 0.5, 'approved', 0, 'care')",
        [],
    )
    .expect("seed");
    let routed = route_case(
        &conn,
        "care",
        RoutingClass::General,
        Some("Q-CARE-INTAKE"),
        None,
    )
    .expect("routed");
    assert_eq!(routed.declared_queues, vec!["Q-CARE-INTAKE".to_string()]);
    assert!(!routed.is_escalated());
}

/// The class label is resolved, or refused — never defaulted.
///
/// The verb that reached the seam needs a label→class resolver, and there are **two**
/// `RoutingClass` types in this tree with the same name. The tempting shape — a
/// `From<confidence::RoutingClass>` — is precisely the hand-typed mapping
/// `the_wiring_holds_no_domain_to_queue_table` exists to catch, wearing a trait impl.
///
/// So the resolver lives on the routing class, searches the vocabulary already pinned
/// to the classifier's, and answers `None` for anything else. This pin asserts the
/// refusal is real: every classifier category resolves, and the labels that must not
/// resolve — including the other enum's absence class — resolve to nothing.
#[test]
fn the_routing_class_label_refuses_what_the_classifier_never_emitted() {
    use brain_server::procedural::CATEGORIES;
    use brain_server::workflow::confidence::RoutingClass as ConfidenceClass;
    use brain_server::workflow::routing::RoutingClass;

    // Every category the classifier emits resolves to a class of the same name.
    for label in CATEGORIES {
        let class = RoutingClass::from_label(label)
            .unwrap_or_else(|| panic!("{label} is a classifier category with no class"));
        assert_eq!(class.as_str(), *label);
    }

    // The OTHER enum's absence class has no counterpart here, so it is no class —
    // and it resolves to nothing *without routing naming it anywhere*.
    assert_eq!(
        ConfidenceClass::HumanUnmeasured.as_str(),
        "human_unmeasured",
        "the absence label moved; this pin's premise is that it is unrecognised here"
    );
    assert_eq!(
        RoutingClass::from_label(ConfidenceClass::HumanUnmeasured.as_str()),
        None,
        "the classifier's absence class resolved to a ROUTABLE class — a case routed under a \
         class the classifier did not emit is routed on nothing"
    );

    // And the near misses a fuzzy matcher would swallow.
    for label in [
        "",
        "technolog",
        "Technology",
        "business process",
        "general ",
    ] {
        assert_eq!(
            RoutingClass::from_label(label),
            None,
            "{label:?} resolved to a class; an unrecognised label must be no class"
        );
    }
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
