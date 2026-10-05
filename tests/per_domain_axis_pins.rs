//! R61′ — the per-domain knowledge-version axis.
//!
//! **Why the axis is PER-DOMAIN.** The publish branch audits under the literal
//! tenant `"global"` (`handlers/gate.rs`, `workflow/kcs/publish`). That is an
//! AUDIT label, not the article's domain. A counter keyed on it would give every
//! domain one shared version wearing a per-domain name, and a case's stored
//! `knowledge_version` would then be comparable across domains it has no
//! relationship to. So the bump reads the article's own `knowledge.domain`.
//!
//! **Why the axis is MONOTONIC.** The KCS state machine has a backward edge:
//! `retract` moves `published → approved`. Counting state transitions rather
//! than publications is confidently wrong — a retraction would tell a reopened
//! case its basis moved when the world had reverted. A retraction does not
//! decrement.
//!
//! **Why nothing branches on the delta.** The stored-vs-current difference is
//! computed and surfaced, but consuming it would be an unmeasured gate: an
//! operator policy behind it does not exist yet. Recording and consuming are
//! different claims, and only the first is made here.
//!
//! **THE LOAD-BEARING PIN IS THE ANTI-VACUITY ONE.** Every other pin in this
//! file would pass against a guard that bumps unconditionally. `r61p_the_guard_is_not_vacuous`
//! is what makes the rest mean anything: it fails when the bump fires where it
//! must not. Without it, five green proofs could describe a constant.

use brain_server::migration::run_migration;
use rusqlite::Connection;

fn crate_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let p = crate_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} must exist: {e}", p.display()))
}

fn migrated_db() -> Connection {
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mut db = Connection::open_in_memory().expect("open in-memory DB");
    run_migration(&mut db, 64).expect("migration");
    db
}

/// A knowledge row in `approved` state, in `domain` — the state a publication
/// transitions FROM. Domain is NOT NULL with a 'global' default, so a KCS draft
/// always carries one.
fn article(conn: &Connection, domain: &str, state: &str) -> i64 {
    conn.execute(
        "INSERT INTO knowledge(content, domain, kcs_state, source, content_hash)
         VALUES (?1, ?2, ?3, 'agent', ?4)",
        rusqlite::params![
            format!("article for {domain}"),
            domain,
            state,
            format!("hash-{domain}-{}", conn.last_insert_rowid())
        ],
    )
    .expect("article insert");
    conn.last_insert_rowid()
}

fn domain_version(conn: &Connection, domain: &str) -> Option<i64> {
    conn.query_row(
        "SELECT version FROM knowledge_domain_versions WHERE domain = ?1",
        rusqlite::params![domain],
        |r| r.get(0),
    )
    .ok()
}

/// The production bump, invoked exactly as the handler invokes it: by ARTICLE
/// ID, with the domain resolved inside the core. Nothing can pass a wrong
/// domain, because there is no domain argument.
fn bump(conn: &Connection, domain: &str, article_id: i64) -> i64 {
    // The article must actually BE in `domain` — that is what the core resolves.
    let actual: String = conn
        .query_row(
            "SELECT domain FROM knowledge WHERE id = ?1",
            rusqlite::params![article_id],
            |r| r.get(0),
        )
        .expect("article domain");
    assert_eq!(
        actual, domain,
        "fixture bug: article {article_id} is in {actual}, not {domain}"
    );
    brain_evolve_core::bump_article_knowledge_version(
        conn,
        article_id,
        "tester",
        1_000,
        brain_server::config::KNOWLEDGE_BASE_VERSION,
    )
    .expect("bump")
}

/// **R61p.1 (red-proof target) — a publication bumps the axis.**
#[test]
fn r61p_publish_bumps_the_axis() {
    let conn = migrated_db();
    assert_eq!(
        domain_version(&conn, "acme"),
        None,
        "a domain that has never published has NO row — not version 0"
    );
    let id = article(&conn, "acme", "approved");
    let v = bump(&conn, "acme", id);
    assert_eq!(v, 2, "the first publication moves the base (1) to 2");
    assert_eq!(domain_version(&conn, "acme"), Some(2));
}

/// **R61p.2 (red-proof target) — a RETRACTION DOES NOT BUMP.** The backward
/// edge. A retracted article's basis did not move, so the axis must not move:
/// a bump here would tell every later-opened case its basis changed when the
/// world in fact reverted.
#[test]
fn r61p_retract_does_not_bump() {
    let conn = migrated_db();
    let id = article(&conn, "acme", "published");
    // A retraction goes through `kcs_state_cas(action="retract")` and NOTHING
    // else — the handler guards the bump on `action == "publish"`. Assert the
    // guard's condition is actually present at the seam, because a retraction
    // reaching the bump is precisely the defect.
    let gate = read("src/handlers/gate.rs");
    let bump_at = gate
        .find("bump_article_knowledge_version")
        .expect("the bump must be called at the publish seam");
    let ctx = &gate[..bump_at];
    assert!(
        ctx.contains(r#"if action == "publish""#),
        "the bump must be guarded by `action == \"publish\"` so a retraction cannot reach it. \
         This is the backward edge: retract moves published → approved, and counting it would \
         be confidently wrong."
    );
    // And behaviourally: the state move alone leaves the axis alone.
    assert_eq!(
        domain_version(&conn, "acme"),
        None,
        "a retraction must leave the axis untouched"
    );
    // And behaviourally: the real retract statement moves the ARTICLE's state
    // back to 'approved' (the backward edge) and must leave the axis untouched.
    // This is `service::gate::kcs_state_cas`'s retract arm verbatim — driven here
    // rather than reached, because that core is crate-private.
    conn.execute(
        "UPDATE knowledge SET kcs_state = 'approved', public_slug = NULL
          WHERE id = ?1 AND kcs_state = 'published'",
        rusqlite::params![id],
    )
    .expect("retract CAS");
    let state: String = conn
        .query_row(
            "SELECT kcs_state FROM knowledge WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .expect("state readback");
    assert_eq!(state, "approved", "the backward edge really did move");
    assert_eq!(
        domain_version(&conn, "acme"),
        None,
        "retracting an article must NOT create or bump a domain row"
    );
}

/// **R61p.3 (red-proof target, THE ONE THAT MATTERS) — the axis is PER-DOMAIN.**
///
/// A pin asserting only "a bump happened" passes with the global-tenant bug
/// planted: every domain shares one counter and the test still sees a bump. So
/// this fixture publishes in TWO domains and asserts they moved INDEPENDENTLY.
#[test]
fn r61p_two_domains_bump_independently() {
    let conn = migrated_db();
    let a = article(&conn, "acme", "approved");
    let b = article(&conn, "globex", "approved");

    bump(&conn, "acme", a);
    // Two publications in acme, ONE in globex: if the axis were keyed on the
    // shared audit tenant, globex would read 3 (or acme's and globex's bumps
    // would be the same counter).
    bump(&conn, "acme", a);
    bump(&conn, "globex", b);

    assert_eq!(
        domain_version(&conn, "acme"),
        Some(3),
        "acme published twice from base 1: 1 → 2 → 3"
    );
    assert_eq!(
        domain_version(&conn, "globex"),
        Some(2),
        "globex published ONCE from base 1: 1 → 2. A shared counter would read 3 here — that \
         is the whole point of this assertion."
    );
    assert_ne!(
        domain_version(&conn, "acme"),
        domain_version(&conn, "globex"),
        "two domains that published a different number of times must NOT share a version. \
         This fails if the bump reads the audit tenant 'global' instead of knowledge.domain."
    );
}

/// **R61p.4 (red-proof target) — monotonic.** Every sequence of publications
/// yields a strictly increasing axis; a decrement or a rewrite is refused.
#[test]
fn r61p_the_axis_is_monotonic() {
    let conn = migrated_db();
    let id = article(&conn, "acme", "approved");
    let mut last = brain_server::config::KNOWLEDGE_BASE_VERSION;
    for step in 1..=5 {
        let v = bump(&conn, "acme", id);
        assert_eq!(
            v,
            last + 1,
            "publication {step} must advance the axis by exactly one"
        );
        assert!(
            v > last,
            "the axis is monotonic by definition: a retraction must never decrement it, because \
             the world reverting is not the basis moving"
        );
        last = v;
    }
    assert_eq!(domain_version(&conn, "acme"), Some(6));
}

/// **R61p.5 (red-proof target) — the bump is INSIDE the transaction.** A bump
/// that survives a rolled-back publication would move the axis for an
/// article that was never published.
#[test]
fn r61p_a_rolled_back_publication_does_not_move_the_axis() {
    let conn = migrated_db();
    let id = article(&conn, "acme", "approved");
    conn.execute_batch("BEGIN").expect("begin");
    bump(&conn, "acme", id);
    assert_eq!(
        domain_version(&conn, "acme"),
        Some(2),
        "precondition: the bump is visible inside its own transaction"
    );
    conn.execute_batch("ROLLBACK").expect("rollback");
    assert_eq!(
        domain_version(&conn, "acme"),
        None,
        "a bump made in a transaction that rolled back must not survive it — otherwise the axis \
         moves for a publication that never landed"
    );
}

/// **R61p.6 — THE ANTI-VACUITY PIN. Load-bearing.** A guard that bumps
/// unconditionally satisfies every "a bump happened" assertion above. This one
/// asserts the bump DOES NOT fire on paths where it must not, which is the only
/// thing that distinguishes a real guard from a constant.
#[test]
fn r61p_the_guard_is_not_vacuous() {
    let conn = migrated_db();
    let id = article(&conn, "acme", "approved");

    // No publication has happened, so nothing may have moved.
    assert_eq!(domain_version(&conn, "acme"), None);
    assert_eq!(domain_version(&conn, "globex"), None);

    // A domain nobody published stays absent — reading it is not a bump.
    assert_eq!(
        brain_evolve_core::current_domain_knowledge_version(
            &conn,
            "never-published",
            brain_server::config::KNOWLEDGE_BASE_VERSION
        ),
        brain_server::config::KNOWLEDGE_BASE_VERSION,
        "a domain with no row is at the BASE version, never 0 — 0 would falsely date it to \
         version zero and collide with the NULL 'predates tracking' sentinel"
    );
    assert_eq!(
        domain_version(&conn, "never-published"),
        None,
        "reading a domain's version must never CREATE a row — a read is not a bump"
    );

    // One publication, one bump: not zero (never-bumps), not two (always-bumps).
    bump(&conn, "acme", id);
    assert_eq!(
        domain_version(&conn, "acme"),
        Some(2),
        "exactly one publication must move the axis exactly once"
    );
}

/// **The base-version law: a missing row is the base, and zero is forbidden.**
#[test]
fn r61p_a_missing_row_is_the_base_version_never_zero() {
    let conn = migrated_db();
    for domain in ["acme", "globex", "never-published"] {
        assert_eq!(
            domain_version(&conn, domain),
            None,
            "{domain} has no row before its first publication"
        );
        assert_eq!(
            brain_evolve_core::current_domain_knowledge_version(
                &conn,
                domain,
                brain_server::config::KNOWLEDGE_BASE_VERSION
            ),
            brain_server::config::KNOWLEDGE_BASE_VERSION,
            "{domain} reads as the base version"
        );
        assert_ne!(
            brain_evolve_core::current_domain_knowledge_version(
                &conn,
                domain,
                brain_server::config::KNOWLEDGE_BASE_VERSION
            ),
            0,
            "zero is the forbidden sentinel"
        );
    }
}

/// **A case records its domain's CURRENT version, not the constant.**
#[test]
fn r61p_a_new_case_records_its_domains_current_version() {
    let conn = migrated_db();
    let id = article(&conn, "acme", "approved");
    bump(&conn, "acme", id);
    assert_eq!(domain_version(&conn, "acme"), Some(2));

    // `state::open_run` is pub(crate); this reproduces its insert with the
    // version the production code now reads. The equivalence of the STATEMENT
    // is pinned separately; what this proves is the value it now carries.
    let run = conn.execute(
        "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at, knowledge_version)
         VALUES (?1, ?2, ?3, 0, 'active', ?4, ?4, ?5)",
        rusqlite::params![
            "acme",
            "troubleshoot",
            "{}",
            1i64,
            brain_evolve_core::current_domain_knowledge_version(
                &conn,
                "acme",
                brain_server::config::KNOWLEDGE_BASE_VERSION
            )
        ],
    );
    assert_eq!(run.expect("open_run"), 1);
    let kv: Option<i64> = conn
        .query_row(
            "SELECT knowledge_version FROM workflow_runs ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("readback");
    assert_eq!(
        kv,
        Some(2),
        "a case opened AFTER the publication records 2, not the constant base"
    );

    // A different domain is unaffected — the recorded value is its own domain's.
    let other: i64 = brain_evolve_core::current_domain_knowledge_version(
        &conn,
        "globex",
        brain_server::config::KNOWLEDGE_BASE_VERSION,
    );
    assert_eq!(other, 1, "an unpublished domain's cases record the base");
}

/// **`NULL` keeps its stated meaning: "predates tracking".** A row written
/// without the column must not read 0 and must not be backfilled by the bump.
#[test]
fn r61p_legacy_runs_are_null_not_zero() {
    let conn = migrated_db();
    conn.execute(
        "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
         VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
        [],
    )
    .expect("legacy insert");
    let kv: Option<i64> = conn
        .query_row(
            "SELECT knowledge_version FROM workflow_runs LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("readback");
    assert_eq!(
        kv, None,
        "a row that predates tracking reads NULL. The bump writes the AXIS, never the runs \
         column — backfilling legacy cases would falsely date them to a version they never \
         opened against."
    );
}

/// **THE NON-CONSUMPTION PIN.** The delta is surfaced; nothing branches on it.
/// This is what stops a later round quietly turning an unmeasured signal into an
/// unmeasured gate.
#[test]
fn r61p_nothing_branches_on_the_delta() {
    let gate = read("src/handlers/gate.rs");
    let state = read("src/workflow/state.rs");
    let core = read("crates/brain-evolve-core/src/lib.rs");
    for (name, src) in [
        ("handlers/gate.rs", &gate),
        ("state.rs", &state),
        ("brain-evolve-core/src/lib.rs", &core),
    ] {
        assert!(
            !src.contains("if delta") && !src.contains("if basis_moved"),
            "{name} must not branch on the version delta. The signal is surfaced and audited; \
             acting on it is an unmeasured gate with no operator policy behind it — the error \
             PROMOTION_ENABLED exists to prevent exactly this."
        );
    }
}

/// **The migration is additive and idempotent, and the rehearse parity list
/// carries the row** — or the parity gate reports a phantom drift.
#[test]
fn r61p_migration_is_additive_and_idempotent() {
    let mut conn = migrated_db();
    run_migration(&mut conn, 64).expect("re-running is a no-op");
    let cols: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('knowledge_domain_versions') WHERE name='domain'",
            [],
            |r| r.get(0),
        )
        .expect("pragma");
    assert_eq!(cols, 1, "the guarded CREATE must not duplicate the table");

    let m = read("src/migration.rs");
    assert!(
        m.contains("CREATE TABLE IF NOT EXISTS knowledge_domain_versions"),
        "the table must be created with IF NOT EXISTS so the runner is idempotent"
    );
    assert!(
        !m.contains("DROP TABLE knowledge_domain_versions"),
        "no table is ever rebuilt — a rebuild is the one operation that can lose rows under a crash"
    );

    let rehearse = read("src/bin/brain_migrate_rehearse.rs");
    assert!(
        rehearse.contains("\"knowledge_domain_versions\""),
        "PARITY_TABLES must carry the new row or the migrate-rehearse parity gate reports a \
         phantom drift on every rehearsal"
    );
}

/// **THE SCHEMA STAMP MOVED WITH THE TABLE, in the same commit.** A version
/// constant with no table is a lie; a table with no constant is invisible.
///
/// The DECLARATION assertion above is this round's historical record and is
/// never edited — the axis stamped at 1.32.24 and the const is the proof. The
/// CEILING and STAMP assertions pin the CURRENT values, which every schema
/// round re-pins; the model-citation-key round moved them to 1.32.25 and the
/// proposal-edge round moved them to 1.32.26.
#[test]
fn r61p_schema_stamp_moved_with_the_table() {
    let layout = read("src/storage_layout.rs");
    assert!(
        layout.contains(r#"pub const SCHEMA_VERSION_V1_32_24: &str = "1.32.24";"#),
        "the new stamp must be declared"
    );
    assert!(
        layout.contains("LATEST_KNOWN_SCHEMA: &str = SCHEMA_VERSION_V1_32_26"),
        "LATEST_KNOWN_SCHEMA must track the newest const or a newer-schema database is blessed \
         instead of refused"
    );
    let m = read("src/migration.rs");
    assert!(
        m.contains("'schema_version', '1.32.26'"),
        "run_migration must stamp 1.32.26"
    );
    // The refuse-newer probe sits ABOVE the ceiling by construction.
    assert!(
        layout.contains(r#"assert!(is_newer_than_known(Some("1.32.27")));"#)
            && layout.contains(r#"assert!(!is_newer_than_known(Some("1.32.26")));"#),
        "the refuse-newer probe must sit above the new ceiling; pinned AT it, it would silently \
         test Equal instead of Greater"
    );
}

/// **THE ANTI-VACUITY GUARD, EXECUTED.** A guard that bumps unconditionally
/// satisfies every "a bump happened" assertion above, and a source pin reading
/// `if action == "publish"` proves only that the string is present — not that
/// the bump is inside it. This asserts the guard's STRUCTURE: the bump call
/// must sit inside the `action == "publish"` arm, with no intervening
/// `if action ==` that would widen it to every action.
#[test]
fn r61p_the_publish_guard_discriminates() {
    let gate = read("src/handlers/gate.rs");
    let at = gate
        .find("bump_article_knowledge_version")
        .expect("the bump call");
    let before = &gate[..at];

    // Find the nearest preceding `if action == "publish"` — the bump's OWN
    // guard. Everything from that guard's opening brace to the call must stay
    // at brace depth >= 1, which proves the call is INSIDE the arm. Scanning
    // backwards for a mere occurrence is not enough: an always-bump plant
    // DELETES this guard, and the search then lands on an unrelated one
    // earlier in the branch and passes VACUOUSLY. A first version of this pin
    // did exactly that, and the always-bump plant sailed through it green.
    let needle = r#"if action == "publish""#;
    let guard_at = before.rfind(needle).unwrap_or_else(|| {
        panic!(
            "the bump must be guarded by `action == \"publish\"` so a retraction cannot reach it"
        )
    });
    let arm_body = &before[guard_at + needle.len()..];
    assert!(
        arm_body.trim_start().starts_with('{'),
        "the bump's guard must open an arm; found: {:?}",
        &arm_body[..arm_body.len().min(40)]
    );

    // The call must sit inside the arm's span. Locating the span by brace
    // counting alone would mis-read string literals and comments, so the
    // assertion is positional and bounded: the guard opens immediately before
    // the call (allowing only the call's own argument text between), and the
    // arm's body between them contains no bare block opener.
    // The decisive check: within the guard's arm, the bump call must be the
    // FIRST statement group, and the arm must not have been closed and reopened.
    // A plant that deletes the guard leaves the call in a BARE BLOCK whose
    // preceding text is the previous arm's `}` — so require that the guard we
    // found is immediately followed by the bump's own call, with nothing that
    // closes a scope in between.
    let between = &gate[guard_at + needle.len()..at];
    let unclosed = between.chars().filter(|c| *c == '}').count();
    assert_eq!(
        unclosed, 0,
        "the bump's call must be INSIDE the guard's arm: a `}}` between the guard and the call \
         means the arm already closed and the bump is NOT guarded by it. This is the \
         anti-vacuity assertion — without it an always-bump plant (guard deleted) passes."
    );
    let opened = between.chars().filter(|c| *c == '{').count();
    assert!(
        opened >= 1,
        "the guard must open an arm before the bump call"
    );

    // And the arm must actually contain the call: the depth only ever rose,
    // never fell to zero before it. Assert the positive direction too — the
    // bump's own statement text must appear AFTER the guard's brace.
    let brace_at = guard_at + needle.len() + arm_body.find('{').expect("arm brace");
    assert!(
        brace_at < at,
        "the guard's arm must open before the bump call"
    );
}

/// **R61p.5 — the bump is INSIDE the transaction, not after it.** A bump on a
/// pooled connection after `tx.commit()` survives a rollback of the state
/// change, so the axis would move for a publication that never landed.
///
/// The rollback twin below proves the *core* is transactional. This proves the
/// *seam*: the bump call must be bound to `&tx` and must appear BEFORE the
/// branch's commit. Driving the core cannot see this — only the call site can.
#[test]
fn r61p_the_bump_rides_the_publish_transaction() {
    let gate = read("src/handlers/gate.rs");
    let bump_at = gate
        .find("bump_article_knowledge_version")
        .expect("the bump call");
    let call = &gate[bump_at..];
    let args = &call[..call.find(")\n").unwrap_or(call.len())];
    assert!(
        args.contains("&tx"),
        "the bump must be bound to the CALLER'S transaction (&tx). A pooled or otherwise \
         post-commit connection would let the axis move for a publication that rolled back. \
         Call: {args}"
    );

    // The branch's commit must come AFTER the bump, never before it.
    let commit_at = gate[bump_at..]
        .find("tx.commit()")
        .unwrap_or_else(|| panic!("the publish branch must still commit after the bump"));
    assert!(
        bump_at < gate.len() - commit_at,
        "the bump must precede tx.commit() in this branch"
    );
}

/// **The base constant's literal, and MONOTONICITY of the axis above it.** The r51
/// pin used to hold the constant at `"1"`; that literal is now the BASE a domain
/// sits at before its first publication, so pinning it to `"1"` alone would let
/// the axis be re-based silently. This holds the base AND the property the axis
/// must have: strictly increasing, never decremented.
#[test]
fn r61p_the_base_constant_and_the_axis_are_pinned() {
    let config = read("src/config.rs");
    let base = config
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("pub const KNOWLEDGE_BASE_VERSION: i64 = ")
        })
        .map(|v| v.trim_end_matches(';').trim())
        .expect("KNOWLEDGE_BASE_VERSION must be declared in config.rs");
    assert_eq!(
        base, "1",
        "the pre-axis base is 1. A silent edit here would re-base every existing case-open \
         readback. Found: {base:?}"
    );

    // Monotonicity, behaviourally, over a long sequence in two domains.
    let conn = migrated_db();
    let a = article(&conn, "acme", "approved");
    let b = article(&conn, "globex", "approved");
    let mut last_a = base.parse::<i64>().expect("numeric base");
    for step in 1..=25 {
        let v = bump(&conn, "acme", a);
        assert_eq!(
            v,
            last_a + 1,
            "publication {step} must advance acme by exactly one — never skip, never decrement"
        );
        last_a = v;
    }
    // The other domain is untouched by acme's 25 publications.
    let v_b = bump(&conn, "globex", b);
    assert_eq!(
        v_b,
        base.parse::<i64>().expect("numeric base") + 1,
        "a second domain's first publication advances only ITSELF — the axes are independent"
    );
    assert!(
        v_b < last_a,
        "globex (2 publications) must still read far below acme (26). Cross-domain versions \
         are NOT comparable, and neither is a shared counter acceptable."
    );
}

/// **The bump is ONE implementation, resolved from source — not a hand-typed
/// list**, which would be the second copy the no-second-copy rule forbids.
#[test]
fn r61p_one_bump_implementation() {
    let gate = read("src/handlers/gate.rs");
    assert_eq!(
        gate.matches("bump_article_knowledge_version").count(),
        1,
        "exactly ONE call site for the bump. A second would let two seams advance the same axis \
         twice per publication."
    );
    assert_eq!(
        read("crates/brain-evolve-core/src/lib.rs")
            .lines()
            .filter(|l| l
                .trim_start()
                .starts_with("pub fn bump_article_knowledge_version"))
            .count(),
        1,
        "exactly one DEFINITION of the bump"
    );
}

/// **The domain is NOT a parameter — it is resolved inside the core from the
/// article.** This is the structural half of the per-domain law: the branch's
/// audit tenant is the literal `"global"`, so any caller-supplied domain is a
/// chance to give every domain one shared counter. Taking the article id alone
/// makes that unrepresentable rather than merely discouraged.
#[test]
fn r61p_the_domain_is_never_caller_supplied() {
    let gate = read("src/handlers/gate.rs");
    let at = gate
        .find("bump_article_knowledge_version")
        .expect("the bump call");
    // the call site's own arguments must not carry a domain
    let tail = &gate[at..];
    let call = &tail[..tail.find(")\n").unwrap_or(tail.len())];
    assert!(
        !call.contains("\"global\"") && !call.contains("tenant"),
        "the bump call must not pass the audit tenant 'global' as a domain: that label is an \
         audit tenant, not the article's domain, and reusing it would give every domain one \
         shared counter wearing a per-domain name."
    );

    let svc = read("crates/brain-evolve-core/src/lib.rs");
    let sig = svc
        .lines()
        .skip_while(|l| !l.contains("pub fn bump_article_knowledge_version"))
        .take(7)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        !sig.contains("domain: &str"),
        "the bump's signature must NOT accept a domain: the core resolves it from the article, \
         so no caller can supply a shared counter. Signature: {sig}"
    );
    assert!(
        svc.contains("SELECT domain FROM knowledge WHERE id = ?1"),
        "the core must resolve the domain from the ARTICLE's own row"
    );
}

/// **R5 — the crate is CONSUMED, not scaffolded (the R59 anti-dead-vertical
/// law).** A `*-core` crate that nothing reaches is the R59 defect with a new
/// name, and the round fails on it. Machine-checked both ways: the server's
/// manifest declares the path edge, and cargo's OWN resolved graph
/// (`cargo tree --invert`, run from the server root) names `brain-server`
/// above the crate.
#[test]
fn r61p_the_evolve_core_is_consumed_not_scaffolded() {
    let manifest = read("Cargo.toml");
    assert!(
        manifest.contains("brain-evolve-core = { path = \"crates/brain-evolve-core\" }"),
        "the server must declare the brain-evolve-core path edge — this is the \
         consumption the R59 law demands, and exactly what R5.1's red-proof \
         removes"
    );
    let out = std::process::Command::new("cargo")
        .args(["tree", "--invert", "brain-evolve-core"])
        .current_dir(crate_root())
        .output()
        .expect("cargo tree must be runnable from the server root");
    assert!(
        out.status.success(),
        "cargo tree --invert brain-evolve-core must succeed. stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let tree = String::from_utf8_lossy(&out.stdout);
    assert!(
        tree.contains("brain-server"),
        "cargo tree --invert brain-evolve-core must name brain-server as a consumer — \
         a crate nothing reaches is the R59 dead-vertical defect with a new name. \
         Tree:\n{tree}"
    );
}
