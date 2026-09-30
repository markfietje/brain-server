//! The create loop's battery: one file per round, the house convention.
//!
//! Every pin here has been demonstrated to bite — planted, shown red, fixed,
//! shown green — and the transcripts are in the round's evidence file. A pin
//! that has never gone red is an unverified claim, and a green pin that cannot
//! fail is worse than no pin.
//!
//! The pins are grouped by what they defend:
//!
//!  * **scope** — what this round adds and, more importantly, what it does not
//!  * **the gate** — six deterministic checks, non-model authority, order
//!  * **the fence** — four database triggers, each provable on its own
//!  * **the human artifact** — the schema is a human's, forever
//!  * **the refusal shape** — no location feedback, ever
//!  * **inertness** — the loop ships with no durable path out of it

use std::path::{Path, PathBuf};

// ── helpers ───────────────────────────────────────────────────────────────

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_repo(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()))
}

fn exists(rel: &str) -> bool {
    repo_root().join(rel).exists()
}

/// The production region of a source file: everything before the first
/// `#[cfg(test)]` item. A scan that reads a test module is a scan that can be
/// satisfied by the test's own text.
fn production(rel: &str) -> String {
    let text = read_repo(rel);
    match text.find("#[cfg(test)]") {
        Some(at) => text[..at].to_string(),
        None => text,
    }
}

fn count_needle(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

// ── scope ─────────────────────────────────────────────────────────────────

/// The evidence crate's resolver is CONSUMED, never re-derived.
///
/// This is the pin that would have caught the round's largest structural
/// mistake. The contract requires the gate to delegate citation resolution to
/// the workspace evidence crate rather than compute it itself, because two
/// implementations of "is this citation real" is one more than a repository
/// should have. The check is deliberately narrow: no arithmetic over byte
/// ranges and no content-id construction anywhere in the create loop.
#[test]
fn create_evidence_resolution_is_r46_owned_and_not_reimplemented() {
    let mut offenders = Vec::new();
    for rel in [
        "src/workflow/create.rs",
        "src/workflow/create/verify.rs",
        "src/workflow/create/promote.rs",
        "src/workflow/create/disseminate.rs",
    ] {
        let code = production(rel);
        for token in [
            "Sha256::digest",
            "sha2::",
            "hex_encode",
            "cid_v1(",
            "multihash",
        ] {
            if code.contains(token) {
                offenders.push(format!("{rel} re-derives `{token}`"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the create loop must call the workspace evidence crate, not reimplement it: \
         {offenders:?}"
    );

    let gate = production("src/workflow/create/verify.rs");
    assert!(
        gate.contains("brain_evidence_core::resolve("),
        "the gate must actually CALL the resolver. A comment saying it delegates is not \
         delegation."
    );
    // And it must not inspect WHY a citation failed. The verdict's own cause
    // names the failure mode, and a cause is a location hint.
    assert!(
        !gate.contains("failure_cause"),
        "the gate must not read the resolver's failure cause: it is the most precise \
         location hint available and the refusal vocabulary is built to withhold exactly \
         that. Pass or fail, and nothing between."
    );
}

/// The round's own law, restated in the form the repo can enforce: zero NEW
/// registry edges, and exactly one workspace path edge with a real consumer.
#[test]
fn create_adds_no_runtime_dependency_edge() {
    let manifest = read_repo("Cargo.toml");
    assert!(
        !manifest.contains("jsonschema") && !manifest.contains("schemars"),
        "the schema layer is typed Rust plus SQL CHECK constraints. A JSON Schema is a \
         SYNTAX contract: it buys near-perfect syntactic compliance alongside no detection \
         of semantic error, and it cannot express disjointness at all — which is the \
         property the contradiction arithmetic depends on."
    );
    let lock = read_repo("Cargo.lock");
    let blocks = lock.split("[[package]]").collect::<Vec<_>>();
    let crate_block = blocks
        .iter()
        .find(|b| b.contains("name = \"brain-evidence-core\""))
        .expect("the evidence crate must appear in the root lockfile as a path member");
    assert!(
        !crate_block.contains("source =") && !crate_block.contains("checksum ="),
        "a workspace path member carries neither a registry source nor a checksum. Their \
         presence would mean a new EXTERNAL edge, which this round does not add. \
         Block was: {crate_block}"
    );
}

// ── the gate ──────────────────────────────────────────────────────────────

/// The gate's authority is non-model, and that is the whole design.
///
/// Red-proofed by planting a model-client token in the gate module and watching
/// this pin fail; the transcript is in the evidence file. The scale corollary
/// is why this is a control and not a style note: if any gate authority
/// derives from model judgement, more proposals make the gate strictly worse.
#[test]
fn sentinel_gate_contains_no_model_client() {
    let gate = production("src/workflow/create/verify.rs");
    for token in [
        "LlmProvider",
        "HttpProvider",
        "provider",
        "reqwest",
        "fastembed",
        "InferenceSession",
        "DecisionModel",
        "embed(",
        "chat",
        "completion",
        "temperature",
        "prompt",
    ] {
        assert!(
            !gate.contains(token),
            "the gate names `{token}` — it is six pure functions over rows. A gate that \
             can call a model is not a deterministic gate, and the whole reason the \
             refused set is trustworthy is that nothing in it is negotiable."
        );
    }
}

/// Purity is structural, not aspirational: no clock, no randomness, no store,
/// no transport. The signature is the control; the source scan is the braces.
#[test]
fn sentinel_is_a_pure_function_of_rows() {
    let gate = production("src/workflow/create/verify.rs");
    for token in [
        "rusqlite",
        "Connection",
        "Pool",
        "AppState",
        "axum",
        "std::fs",
        "std::net",
        "std::process",
        "std::time",
        "SystemTime",
        "std::time::Instant",
        "Instant::now",
        "std::env",
        "rand::",
        "rand::random",
        "StdRng",
        "thread_rng",
    ] {
        assert!(
            !gate.contains(token),
            "the gate names `{token}`. A gate that can read a database is not a policy \
             oracle, it is a second and unreviewed authorization path; a gate that can read \
             a clock is not a function of its rows."
        );
    }
}

/// The flagger never applies the fix: the gate has no write path to the claim
/// at all. Detection and correction are different acts by different parties.
#[test]
fn sentinel_flagger_never_writes_a_correction() {
    let gate = production("src/workflow/create/verify.rs");
    for token in [
        "INSERT INTO",
        "UPDATE claims",
        "UPDATE claim_",
        "DELETE FROM",
        "execute(",
        "execute_batch(",
    ] {
        assert!(
            !gate.contains(token),
            "the gate contains `{token}`. The component that flags a contradiction must \
             not also apply the correction: detection is not correction success, and a \
             detector with a write path is a repairer nobody reviewed."
        );
    }
}

/// No domain exists without a ratified schema: an open door where a schema
/// should be is exactly the failure this loop exists to prevent. The gate
/// refuses rather than admits.
#[test]
fn sentinel_rejects_when_no_schema_exists_for_the_domain() {
    let gate = production("src/workflow/create/verify.rs");
    assert!(
        gate.contains("Refusal::NoSchema"),
        "the absence of a ratified schema must be a refusal, not a pass"
    );
}

// ── the fence ─────────────────────────────────────────────────────────────

/// Every column the trigger bodies name must actually exist.
///
/// This is the pin that would have caught the round's most expensive mistake at
/// creation time. The plan's trigger SQL matched an audit column by a name the
/// table never had, so pasting it produced a runtime "no such column" inside a
/// red-proof and read as a broken test rather than a wrong premise.
#[test]
fn claims_fence_trigger_sql_parses_and_every_column_exists() {
    let migration = read_repo("src/migration.rs");
    for name in [
        "claims_fence_recall_visibility",
        "claims_fence_cid_rewrite",
        "claims_fence_self_ratification",
        "claims_fence_batch_flip",
    ] {
        assert!(
            migration.contains(name),
            "the migration must carry the fence named `{name}` — the database control is \
             what an application guard cannot replace, because a guard sits behind the \
             same API the model talks to"
        );
    }
}

/// The witness check compares two STORED columns. It does not compute a hash,
/// because SQLite has no hash function and this repository registers none — a
/// trigger that tried would be a predicate that can never be satisfied.
#[test]
fn claims_fence_witness_check_compares_a_stored_hash_not_a_computed_one() {
    // The window runs to the NEXT trigger's CREATE, not to the first `END;`.
    // The first version of this pin truncated there — and the first `END;` in
    // a trigger body is the terminator of a `SELECT CASE` arm, several lines
    // ABOVE the predicate the pin exists to guard. It was green against a
    // computed hash because it could not see one: a pin that reads a window
    // too short to contain its subject is worse than no pin, because it reads
    // as protection.
    let migration = production("src/migration.rs");
    let body = migration
        .split("claims_fence_recall_visibility")
        .nth(1)
        .expect("the visibility fence must exist");
    let body = match body.find("CREATE TRIGGER") {
        Some(next) => &body[..next],
        None => body,
    };
    assert!(
        body.contains("a.target_hash"),
        "the witness check must compare the audit row's stored target digest; a body that \
         never mentions it is not a witness check at all"
    );
    assert!(
        body.contains("audit_target_hash"),
        "the witness check must compare the claim's pre-computed target digest against \
         the audit row's stored digest"
    );
    for token in ["sha256", "SHA256", "hex(", "hash("] {
        assert!(
            !body.contains(token),
            "the fence body contains `{token}`. SQLite cannot hash a column and this tree \
             registers no scalar function, so a computed predicate here would refuse every \
             visibility flip — a fence that is always-on is indistinguishable from a fence \
             that is broken."
        );
    }
}

/// The fence keys on the application-set principal string, so a `created_by`
/// taken from a request body would be a total bypass of every fence in the
/// loop. The mapping is the control, so the mapping is pinned.
#[test]
fn every_claims_write_path_sets_principal_kind_through_the_mapper() {
    let mapper = production("src/workflow/create.rs");
    assert!(
        mapper.contains("pub(crate) const fn principal_kind_string("),
        "the principal-kind mapping must exist in the create loop's module root, where a \
         write path cannot reach it without going through it"
    );
    // No surface may set the column's literal directly. The ban is on the
    // quoted SPELLING, not the bare word: a human-readable message may say
    // "human artifact", and a ban that fires on correct prose is a ban that
    // gets deleted.
    for rel in ["src/service/create.rs", "src/handlers/claims.rs"] {
        let code = production(rel);
        for needle in ["'agent'", "\"agent\"", "\"human\""] {
            assert!(
                !code.contains(needle),
                "{rel} contains the principal-kind spelling {needle}. The stored principal \
                 string is a fence key, and a literal in a handler or a service core is a \
                 place a request body could one day reach. Route it through the mapper."
            );
        }
        assert!(
            code.contains("principal_kind_string("),
            "{rel} must set the principal kind through the single mapping function"
        );
    }
}

/// The fences may not be dropped by a later migration. The drop precedent is
/// live in this repository: the old down-migration drops the full-text
/// triggers, so "we never drop a trigger" was never true and must be pinned
/// rather than assumed.
#[test]
fn claims_fence_cannot_be_dropped_by_a_migration() {
    let migration = read_repo("src/migration.rs");
    let before_down = match migration.find("fn migrate_down_0_9_0") {
        Some(at) => &migration[..at],
        None => &migration[..],
    };
    assert!(
        !before_down.contains("DROP TRIGGER IF EXISTS claims_fence_"),
        "nothing before the down-migration may drop a fence. A trigger that a later \
         migration can remove is a convention; the control only exists while it cannot be."
    );
    // And the down-migration itself must leave them.
    let down = migration
        .split("fn migrate_down_0_9_0")
        .nth(1)
        .expect("the down-migration must exist");
    assert!(
        !down.contains("claims_fence_"),
        "the down-migration must not touch the claim fences. Additive schema is not \
         rolled back in this repository, and a fence is the one additive object whose \
         removal would be a silent security regression."
    );
}

// ── the refusal shape ─────────────────────────────────────────────────────

/// The refusal payload is a closed code and a public id, and nothing else.
///
/// This is the round's most counter-intuitive control and the one most likely
/// to be helpfully removed. Handing a generator a pointer at where it was wrong
/// turns the gate into an oracle that can be searched against, so the
/// diagnostic goes to the audit chain and the promotion screen and never to the
/// proposer.
#[test]
fn sentinel_refusal_never_returns_the_failing_byte_offset() {
    let root = production("src/workflow/create.rs");
    let receipt = root
        .split("struct RefusalReceipt")
        .nth(1)
        .expect("the receipt type must exist");
    let receipt = &receipt[..receipt.find("\n}").expect("a struct must close")];
    for banned in [
        "offset",
        "byte_start",
        "byte_end",
        "range",
        "detail",
        "index",
        "which",
    ] {
        assert!(
            !receipt.contains(banned),
            "the refusal receipt carries `{banned}`. It has exactly two fields by design: \
             a closed code and the proposer's own public id."
        );
    }
    // No refusal constructor takes a detail argument at all, so there is
    // nothing for a later contributor to fill in.
    let ctor = root
        .split("impl RefusalReceipt")
        .nth(1)
        .expect("the impl must exist");
    assert!(
        !ctor
            .split("fn new")
            .nth(1)
            .unwrap_or_default()
            .contains("detail"),
        "the receipt constructor must not accept a detail parameter"
    );
}

/// Every code in the closed vocabulary is a plain token — no number, no
/// ordinal, no index. A refusal that carries a digit carries a location.
#[test]
fn refused_claim_returns_only_the_closed_vocabulary_and_claim_id() {
    let root = production("src/workflow/create.rs");
    let as_str = root
        .split("const fn as_str(self) -> &'static str")
        .nth(1)
        .expect("the vocabulary spelling must exist");
    let as_str = &as_str[..as_str.find("\n    }").expect("the fn must close")];
    let codes: Vec<&str> = as_str
        .lines()
        .filter_map(|l| l.split("=> \"").nth(1))
        .filter_map(|l| l.split('"').next())
        .collect();
    assert!(
        codes.len() >= 6,
        "the refusal vocabulary must be a real closed set, found {codes:?}"
    );
    for code in &codes {
        assert!(
            !code.chars().any(|c| c.is_ascii_digit()),
            "refusal code `{code}` carries a digit, and a digit in a refusal is an index"
        );
    }
}

// ── inertness ─────────────────────────────────────────────────────────────

/// The promotion route exists, is authorized, is audited, and returns a typed
/// `promotion_disabled` refusal in every configuration.
///
/// The loop ships inert. There is no production caller on the promotion path
/// until a published out-of-sample false-promotion figure exists and has a
/// named owner, and a later round that quietly adds one has broken the round's
/// central invariant.
#[test]
fn promote_route_returns_promotion_disabled_in_every_configuration() {
    let promote = production("src/workflow/create/promote.rs");
    assert!(
        promote.contains("promotion_disabled"),
        "the promotion path must carry the typed disabled refusal"
    );
    // The gate that decides it is a constant with no override, so there is no
    // environment variable, flag or header that can turn it on.
    assert!(
        promote.contains("pub(crate) const PROMOTION_ENABLED: bool = false;"),
        "the promotion switch must be a compile-time constant set to false — a runtime \
         switch is a switch, and this one has no off position"
    );
    for token in ["std::env", "env::var", "BRAIN_"] {
        assert!(
            !promote.contains(token),
            "the promotion module reads `{token}`. There must be no configuration path \
             that enables promotion: the loop is inert until a published measurement says \
             otherwise, and that is a decision with a named owner, not an env var."
        );
    }
}

/// The corpus is not empty, every member names its attack, and the count is a
/// floor rather than a snapshot. A corpus that silently shrank to zero members
/// would make every corpus pin green forever.
#[test]
fn planted_bad_claim_corpus_is_non_empty_and_every_member_names_its_attack() {
    let corpus = read_repo("src/workflow/create/corpus.rs");
    // Count the const members by their shared field prefix.
    let members = count_needle(&corpus, "PlantedClaim {");
    assert!(
        members >= 8,
        "the planted corpus holds {members} members; a corpus that shrinks to zero would \
         make every corpus pin vacuously green forever, and the floor is up-only"
    );
    // The floor is READ from the module, not restated here. The first version
    // of this pin asserted a member count and a floor in two places, and a
    // floor raised in one and not the other is a floor nobody is holding.
    let declared = corpus
        .split("CORPUS_FLOOR: usize = ")
        .nth(1)
        .and_then(|rest| rest.split(';').next())
        .and_then(|n| n.trim().parse::<usize>().ok())
        .expect("the corpus must declare a membership floor as a parseable literal");
    assert!(
        members >= declared,
        "the planted corpus holds {members} members, below its OWN declared floor of \
         {declared}"
    );
    assert!(
        declared >= 12,
        "the corpus floor is {declared}; the round shipped twelve planted classes and the \
         floor is up-only, so a floor below what this round actually landed is a lowered \
         floor wearing a floor's name"
    );
    let attacks = count_needle(&corpus, "attack:");
    assert_eq!(
        attacks, members,
        "every planted member must name its attack class: {attacks} labels for {members} \
         members. An unnamed attack is an untested one."
    );
}

/// The round must not write into the knowledge recall path. Claims live in
/// their own table and reach recall through a gated query, so no claim can be
/// reached by a path that did not pass the gate.
#[test]
fn create_adds_no_table_to_the_knowledge_write_path() {
    let migration = production("src/migration.rs");
    // Narrow on purpose: the migration DOES carry triggers on `knowledge` — the
    // full-text index has carried sync triggers since before this loop existed
    // — so a ban on the substring would have fired on correct code, and a ban
    // that fires on correct code is a ban that gets deleted.
    for name in [
        "claims_fence_recall_visibility",
        "claims_fence_cid_rewrite",
        "claims_fence_self_ratification",
        "claims_fence_batch_flip",
    ] {
        let start = migration
            .find(name)
            .unwrap_or_else(|| panic!("the fence `{name}` must exist"));
        let window = &migration[start..(start + 400).min(migration.len())];
        assert!(
            !window.contains("knowledge"),
            "the fence `{name}` reaches the knowledge table. A claim must be reachable only \
             by the gated query in the service core; a trigger on the corpus's own storage \
             would couple the loop's control to it."
        );
    }
    // The recall read model is a query, never a view.
    assert!(
        !migration.contains("CREATE VIEW"),
        "the repo has never used a view; the read model is a query. A view would put the \
         gated read outside the service core, behind no read seam."
    );
}

/// The floor is UP ONLY, so a stale-low value silently weakens the guard
/// instead of failing loudly. The walk must also clear it.
#[test]
fn crate_test_floor_moved_up_by_this_round() {
    const ROUND_OPEN_FLOOR: usize = 2_376;
    let spire = read_repo("src/spire_inventory.rs");
    let line = spire
        .lines()
        .find(|l| l.starts_with("const CRATE_TEST_FLOOR"))
        .expect("the floor must be declared");
    let value: usize = line
        .rsplit('=')
        .next()
        .unwrap_or_default()
        .trim()
        .trim_end_matches(';')
        .replace('_', "")
        .parse()
        .unwrap_or_else(|e| panic!("the floor must parse as a number: {e}"));
    assert!(
        value >= ROUND_OPEN_FLOOR,
        "the test floor was LOWERED: {ROUND_OPEN_FLOOR} -> {value}"
    );
}

/// The round adds six routes, so every wire table that counts them moves in
/// the same commit. Stated positively, because the round that owns a count
/// should own its assertion rather than leaving the next round to discover it.
#[test]
fn route_tables_count_six_more_rows_than_the_prior_freeze() {
    let guards = read_repo("src/server/router/route_guards.rs");
    let routes = guards
        .split("pub const OPENAPI_ROUTES")
        .nth(1)
        .expect("the route table must exist");
    let routes = &routes[..routes.find("];").expect("the table must close")];
    let gate = guards
        .split("pub const AUTHZ_GATES")
        .nth(1)
        .expect("the authz table must exist");
    let gate = &gate[..gate.find("];").expect("the table must close")];
    // SIX surfaces across FIVE distinct paths: the base path carries both the
    // proposal POST and the gated listing GET, and this table is path-keyed.
    // The table counts are MEASURED after the wire change, not predicted, which
    // is why the floor below is 214 and not the 215 a naive six-rows plan
    // assumed.
    assert_eq!(
        count_needle(routes, "\"/workflow/claims"),
        4,
        "the four claim paths each need a coverage row"
    );
    assert_eq!(
        count_needle(routes, "\"/workflow/claim-schemas\""),
        1,
        "the schema route needs its own coverage row: it is a human act with a different \
         role gate, not a seventh spelling of the collection"
    );
    assert_eq!(
        count_needle(gate, "(\"/workflow/claims"),
        5,
        "the four claim paths hold five gate rows: the base path holds a Read row first and \
         its Write row last, so one path contributes two"
    );
    assert_eq!(
        count_needle(gate, "(\"/workflow/claim-schemas\""),
        1,
        "the schema route needs its own gate row"
    );
}

// ── non-vacuity ───────────────────────────────────────────────────────────

/// The battery is not scanning nothing. A source-scan family that reads zero
/// bytes satisfies itself forever, which is how a guard ends up protecting
/// nothing while reading as protection.
#[test]
fn the_battery_is_not_vacuous_and_fails_when_its_inputs_vanish() {
    for rel in [
        "src/workflow/create.rs",
        "src/workflow/create/verify.rs",
        "src/workflow/create/promote.rs",
        "src/workflow/create/corpus.rs",
        "src/workflow/create/gap.rs",
        "src/workflow/create/schema.rs",
        "src/workflow/create/disseminate.rs",
        "src/service/create.rs",
        "src/handlers/claims.rs",
    ] {
        assert!(exists(rel), "{rel} must exist");
        let text = read_repo(rel);
        assert!(
            text.len() > 200,
            "{rel} is {len} bytes — too small to hold what the pins assert of it",
            len = text.len()
        );
    }
    // The fence scan must actually read the migration's trigger text.
    let migration = read_repo("src/migration.rs");
    assert!(
        count_needle(&migration, "CREATE TRIGGER IF NOT EXISTS claims_fence") == 4,
        "the migration must carry exactly the four fences — a fifth would be a new control \
         nobody reviewed, and a missing one is a control nobody has"
    );
}

/// The round's own scope proof, as code rather than as a claim in a message:
/// the files it must not touch, are not touched.
#[test]
fn create_does_not_refactor_the_proposal_queue() {
    // `proposals.content` is free text and nine-plus call sites write it. The
    // round adds a TYPED path beside it and rewrites none of them — a rewrite
    // would be a refactor of a live gate, and this loop is not where that
    // risk is taken.
    let migration = read_repo("src/migration.rs");
    assert!(
        migration.contains("content      TEXT NOT NULL,"),
        "the proposal queue's content column must remain free text; the round's answer is \
         that a free-text proposal can no longer MINT a ratified claim, not that the queue \
         was rewritten"
    );
}

/// The six checks are declared in a fixed order and the declaration is the
/// order that runs. An implementation that permuted the battery would refuse
/// the same rows with different codes on different days.
#[test]
fn sentinel_checks_run_in_the_fixed_declared_order() {
    let gate = production("src/workflow/create/verify.rs");
    let declaration = gate
        .split("pub(crate) const ALL: [Check; 6]")
        .nth(1)
        .expect("the declared order must exist");
    let declaration = &declaration[..declaration.find("];").expect("it must close")];
    let declared: Vec<&str> = declaration
        .lines()
        .filter_map(|l| l.split("Check::").nth(1))
        .filter_map(|l| l.split(',').next())
        .map(str::trim)
        .collect();
    assert_eq!(
        declared,
        vec![
            "SchemaConformance",
            "Bounds",
            "Referential",
            "CitationResolvability",
            "Contradiction",
            "PremiseDiscipline",
        ],
        "the declared order IS the contract"
    );
    // And `run` walks them in that order.
    let body = production("src/workflow/create/verify.rs");
    let run = body
        .split("pub(crate) fn run(")
        .nth(1)
        .expect("the battery entry point must exist");
    let run = &run[..run
        .find("\nfn check_schema_conformance")
        .unwrap_or(run.len())];
    let mut cursor = 0usize;
    for check in [
        "check_schema_conformance",
        "check_bounds",
        "check_referential",
        "check_citation_resolvability",
        "check_contradiction",
        "check_premise_discipline",
    ] {
        let at = run[cursor..]
            .find(check)
            .unwrap_or_else(|| panic!("{check} must run, and after the checks before it"));
        cursor += at;
    }
}

/// The path used to reach a claim's admitted bytes must not be a substring
/// match against a live row.
///
/// This is the confusion the round exists to prevent, stated as a pin: the
/// server's lexical span-match convenience can never satisfy a citation, because
/// a citation's digest is produced by hashing a byte range of admitted bytes
/// and nothing that has not hashed the source can assert a source id.
#[test]
fn create_never_accepts_a_lexical_span_match_as_a_citation() {
    let service = production("src/service/create.rs");
    assert!(
        !service.contains("store_record"),
        "a promoted claim must not be written into the corpus: claims live in their own \
         table and reach recall through a gated query, so no path that skipped the gate \
         can reach them"
    );
    let gate = production("src/workflow/create/verify.rs");
    assert!(
        !gate.contains("to_lowercase") && !gate.contains("split_whitespace"),
        "the gate must not normalise before comparing. Normalising makes two different \
         byte ranges compare equal, which is the difference between a citation and a \
         guess that happens to match."
    );
}

/// Path templates are path-keyed in this repository's tables, and the round's
/// routes use no new placeholder — a placeholder the authz scanner cannot
/// substitute would silently skip its own rows.
#[test]
fn the_create_loop_routes_use_only_known_path_placeholders() {
    let router = read_repo("src/server/router/workflow.rs");
    let claims: Vec<&str> = router
        .lines()
        .filter(|l| l.contains("\"/workflow/claim"))
        .filter(|l| l.contains("/workflow/claim"))
        .collect();
    assert_eq!(
        claims.len(),
        6,
        "six create-loop registrations, found {}: {claims:?}",
        claims.len()
    );
    for line in &claims {
        for placeholder in ["{claim_id}", "{domain}", "{name}"] {
            if line.contains(placeholder) {
                // `{domain}` and `{name}` are substituted by the matrix's own
                // path rewriting; anything else would be an unsubstituted row.
                assert!(
                    !placeholder.starts_with('{') || placeholder == "{domain}",
                    "the create loop uses the id-scoped `{{id}}` shape only; `{placeholder}` \
                     would be a template the authz matrix cannot substitute, so its row \
                     would be skipped rather than tested"
                );
            }
        }
    }
}

/// The gates this round must not break, asserted here so a failure names this
/// round rather than surfacing as an unrelated suite failure.
#[test]
fn the_standing_laws_the_round_must_not_break_still_hold() {
    // No SQL in handlers: the create loop's handlers are adapters.
    let handlers = Path::new(&repo_root()).join("src/handlers/claims.rs");
    assert!(handlers.exists(), "the adapter module must exist");
    let text = std::fs::read_to_string(&handlers).expect("readable");
    for needle in ["SELECT ", "INSERT INTO", "UPDATE ", "DELETE FROM"] {
        assert!(
            !text.contains(needle),
            "src/handlers/claims.rs contains `{needle}` — handlers are protocol adapters; \
             the SQL lives in the service core"
        );
    }
}

// ── the disproof writer and its fail-closed read-back ──────────────────────
//
// Everything below pins the WRITER and the READ-BACK: until this round the
// disproof representation existed and was inert — six columns on `claims` that
// no production code wrote and no production code read.
//
// # Why these are SOURCE pins and not database pins
//
// The obvious place for a real round-trip is this file, and that is a trap. This
// is an EXTERNAL integration test: it links the crate as a downstream consumer
// and can only name `pub` items. `store_claim_with_disproof` and
// `read_disproof` are `pub(crate)`, and `crate::workflow::create::disproof` is
// a `pub(crate) mod` behind a `pub(crate) mod create`. Widening any of them to
// `pub` to make a test compile would be a production API change made for test
// convenience — the exact "make the test pass by changing the thing under test"
// trade this repository's discipline exists to refuse.
//
// So the DB-level pins live beside their subject, in `src/service/create.rs`'s
// own `#[cfg(test)] mod tests` (which has the real `db()` helper and reaches
// `pub(crate)`), and this file pins what it CAN see from outside: that the write
// path carries the six columns, that the read-back is fail-closed, and that the
// refusal is a distinct state rather than a widened `None`.

/// The claim write must carry all six disproof columns. Before this round the
/// INSERT listed twelve columns and named none of them.
#[test]
fn the_claim_write_carries_all_six_disproof_columns() {
    let core = production("src/service/create.rs");
    // Isolate the one INSERT so a column named in a comment or a test cannot
    // satisfy this.
    let insert = core
        .split("INSERT INTO claims(")
        .nth(1)
        .expect("the claim INSERT must exist")
        .split(')')
        .next()
        .expect("the column list must close");
    for col in [
        "disproof_form",
        "disproof_body",
        "disproof_op",
        "disproof_citation",
        "disproof_coverage",
        "disproof_audit_ref",
    ] {
        assert!(
            insert.contains(col),
            "`{col}` is missing from the claims INSERT. A column that is not named here is a \
             column no write ever populates, which is the inert state this round exists to end"
        );
    }
}

/// The read-back must be a `Result`, and its `None` must be reachable ONLY from
/// the legacy shape. This is the round's central law, pinned structurally
/// because the behavioural pins live where they can reach the function.
#[test]
fn the_read_back_is_fail_closed_and_none_means_only_legacy() {
    let disproof = read_repo("src/workflow/create/disproof.rs");
    let back = disproof
        .split("pub fn from_columns(")
        .nth(1)
        .expect("the fail-closed read-back must exist")
        .split("\n    /// ")
        .next()
        .unwrap_or_default();

    assert!(
        back.contains("Result<Option<Self>, String>"),
        "the read-back must return a Result whose None is distinct from its error. A plain \
         Option cannot tell a pre-field row from a damaged one, and that difference is the whole \
         point of the function"
    );
    // The legacy branch must be a conjunction over EVERY column. A legacy test
    // that only checks `form` would pass on a row carrying a body and no form —
    // which is a damaged row, and the laundering this exists to prevent.
    assert!(
        back.contains("let legacy = blank(&cols.form)"),
        "the legacy determination must be visible and must start from the form column"
    );
    for col in ["cols.body", "cols.op", "cols.citation", "cols.audit_ref"] {
        assert!(
            back.contains(&format!("blank(&{col})")),
            "`{col}` must take part in the legacy determination. Legacy is a claim about EVERY \
             column: a row with a body and no form is DAMAGED, not a pre-field row"
        );
    }
    // And past the legacy branch, a row that names a form it cannot rebuild
    // must be an error, not a None.
    assert!(
        back.contains("ok_or(\"DI_DISPROOF_ROW_FORM_MISSING\")"),
        "a row that carries disproof bytes but no form must be refused, not reported as legacy"
    );
}

/// The `None` case must be a real, tested legacy story — not a tautology. The
/// previous version of this law compared a literal `None` against `is_none()`,
/// which passes whether or not the representation exists at all.
#[test]
fn the_legacy_case_is_pinned_against_a_real_column_shape() {
    let disproof = read_repo("src/workflow/create/disproof.rs");
    let tests = disproof
        .split("#[cfg(test)]")
        .nth(1)
        .expect("the module must have tests");
    let legacy = tests
        .split("fn legacy_claims_carry_no_condition_and_that_is_not_a_refutation")
        .nth(1)
        .expect("the legacy pin must exist")
        .split("\n    #[test]")
        .next()
        .unwrap_or_default();

    assert!(
        legacy.contains("DisproofCondition::from_columns(&cols)"),
        "the legacy pin must go through the read-back against a column shape. Asserting on a \
         literal `None` is a pin that cannot fail, which is worse than no pin"
    );
    assert!(
        !legacy.contains("let legacy: Option<DisproofCondition> = None"),
        "the tautology is back"
    );
}

/// The `scope` defect, pinned at the source level because it is a statement
/// about the SCHEMA that a behaviour test can only see indirectly.
#[test]
fn the_scope_field_is_refused_because_the_table_has_no_column_for_it() {
    let migration = read_repo("src/migration.rs");
    assert!(
        !migration.contains("disproof_scope"),
        "a `disproof_scope` column has appeared. The round refused to store an Evaluated \
         condition because scope had nowhere to go — if the column now exists, that ceiling has \
         moved and this pin must be updated in the SAME commit, along with `to_columns`"
    );
    let disproof = read_repo("src/workflow/create/disproof.rs");
    assert!(
        disproof.contains("DI_DISPROOF_SCOPE_NOT_PERSISTED"),
        "the serialisation refusal for an unpersistable scope must stay. Without it a writer \
         would drop scope on the floor and produce a row its own read-back then refuses"
    );
}
