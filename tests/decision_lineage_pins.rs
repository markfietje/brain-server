// R57b — the decision lineage: a closed, ref-only outbox record of every
//! operator decision the loop's three writers take.
//!
//! ## Why this file exists separately from the modules
//!
//! The two records a decision produces are written by different code for
//! different readers, and the gap between them is invisible from inside either
//! one. The session row carries the body; the lineage row carries the fact.
//! Before R57b a handoff decision, a back-referral release and a pipeline
//! advance each produced a session row and an audit row and NOTHING on the
//! run's event chain — so a reader of `/events`, or of the alert worker's
//! drain, could see that a run existed and never see that anybody decided
//! anything about it. A module cannot check its own absence, so the claim is
//! asserted from outside, by driving the real handlers and reading the rows
//! back.
//!
//! ## Run-scoped, always
//!
//! Every count below is `WHERE run_id = ?`. A global `COUNT(*)` would be a
//! measurement of the fixture rather than of the append, and it is the kind of
//! assertion that inverts the moment a second writer lands. (The one global
//! count in the tree — main_suite's `steering`/events forgery pins — is a
//! different claim: that a REFUSED enqueue writes nothing anywhere, which is
//! genuinely a whole-table statement and must stay one.)
//!
//! ## The read-as-text idiom
//!
//! One pin here reads source as text, the `r50_create.rs` / `r57_census_pins.rs`
//! precedent: the closed outcome vocabulary in `outbox.rs` is the union of two
//! enums that live in other files, and a compile-time check cannot see an
//! absent word. The scan cuts the region it scans — a whole-file grep passes on
//! the scanner's own literal strings, which is the vacuous-pass class this
//! repository treats as worse than no check at all.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use brain_server::handlers::accounts::{PipelineBody, post_pipeline};
use brain_server::handlers::auth::OptPrincipal;
use brain_server::handlers::workflow_decisions::{
    BackReferralReturnBody, HandoffDecisionBody, post_back_referral_return, post_handoff_decision,
};
use brain_server::workflow::outbox::{
    CASE_DECISION_KINDS, CASE_DECISION_OUTCOMES, topic_is_reserved,
};
use brain_server::{
    AppState, JwtMiddlewareState, alert, auth, concurrency, config, domain_registry, http_limit,
    integrity, migration, pool, register_sqlite_vec,
};

/// The lineage topic, spelled independently of the production constant. A pin
/// that read the subject's own constant would pass on a topic rename; this one
/// fails, which is the entire reason the value is written down twice.
const TOPIC: &str = "case/decision";

// ─────────────────────────────────────────────────────────────────────────────
// fixture — the composed state, one pool, the migration applied
// ─────────────────────────────────────────────────────────────────────────────

fn state(tmp: &tempfile::NamedTempFile) -> Arc<AppState> {
    register_sqlite_vec::register_sqlite_vec();
    let mgr = pool::SqliteConnectionManager::file(tmp.path());
    let pool: brain_server::Pool = r2d2::Pool::builder().max_size(4).build(mgr).expect("pool");
    migration::run_migration(&mut pool.get().unwrap(), config::DB_MMAP_SIZE_MIB)
        .expect("migration");
    let model: Arc<dyn brain_server::embed::Embedder> =
        Arc::new(brain_server::embed::StaticEmbedder::new(config::MODEL_ID).expect("model"));
    Arc::new(AppState {
        token_store: auth::TokenStore::new(),
        jwt_middleware_state: Arc::new(JwtMiddlewareState::opaque_for_tests(
            pool.clone(),
            tmp.path().to_path_buf(),
        )),
        cors: tower_http::cors::CorsLayer::new(),
        durability: Default::default(),
        loom: Default::default(),
        model,
        registry: domain_registry::DomainRegistry::new(pool.clone(), tmp.path(), false),
        pool,
        db_path: tmp.path().to_path_buf(),
        connection_tracker: Arc::new(http_limit::ConnectionTracker::new()),
        rate_limiter: Arc::new(http_limit::RateLimiter::new()),
        snapshot: integrity::SnapshotState::default(),
        audit_chain_cache: Arc::new(std::sync::Mutex::new(None)),
        auth_mode: auth::AuthMode::Opaque,
        key_store: auth::jwks::KeyStore::load(Path::new("/nonexistent")).expect("empty key store"),
        revocation_cache: Arc::new(auth::revocation::RevocationCache::new()),
        jwt_issuer: String::new(),
        jwt_audience: String::new(),
        oidc_config: brain_server::handlers::well_known::OidcConfig::unconfigured(),
        ump_events: tokio::sync::broadcast::channel(config::UMP_EVENT_BUFFER).0,
        alert_events: tokio::sync::broadcast::channel(config::ALERT_EVENT_BUFFER).0,
        alert_seq: std::sync::atomic::AtomicU64::new(0),
        chain_watch: alert::ChainWatchState::default(),
        concurrency: &concurrency::CONCURRENCY,
    })
}

/// A minimal governed run row: `personal` domain, active.
fn seed_run(state: &Arc<AppState>) -> i64 {
    let conn = state.pool.get().unwrap();
    conn.execute(
        "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
         VALUES ('personal', 'intake', '{}', 0, 'active', 1, 1)",
        [],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// A minimal account row (kind `account`; the record itself is `state_json`).
fn seed_account(state: &Arc<AppState>, name: &str) -> i64 {
    let conn = state.pool.get().unwrap();
    let record = serde_json::json!({
        "account_id": "0",
        "name": name,
        "owner_principal": "hash:seed",
        "status": "active",
        "created_at": 1,
        "updated_at": 1,
    });
    conn.execute(
        "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
         VALUES ('acme', 'account', ?1, 0, 'active', 1, 1)",
        [record.to_string()],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// Arm a back-referral return contract, mirroring the core writer's row shape
/// (`run{id}:back_referral:{owner}` + a deadline epoch).
fn seed_contract(state: &Arc<AppState>, run_id: i64, armed_at: i64) -> String {
    let key = format!("run{run_id}:back_referral:owner");
    let contract = serde_json::json!({
        "referrer": "l1:steward-dpc",
        "receiver": "eng-storage",
        "clinical_question": "confirm the battery",
        "required_report": ["finding"],
        "status": "open",
    });
    let payload = serde_json::json!({
        "contract_key": key,
        "status": "open",
        "deadline_epoch": armed_at + 86_400,
        "contract": contract,
    });
    let conn = state.pool.get().unwrap();
    conn.execute(
        "INSERT INTO agent_session_events(run_id, seq, idempotency_key, kind, payload_json, created_at)
         VALUES (?1, 1, 'seed-back-referral', 'back_referral', ?2, ?3)",
        rusqlite::params![run_id, payload.to_string(), armed_at],
    )
    .unwrap();
    key
}

/// Seed a prior lineage event so the run has a tip BEFORE the decision. The
/// parent assertion is only meaningful when a parent can exist — without this
/// the `parent_id IS NULL` case would pass for the wrong reason.
fn seed_prior_event(state: &Arc<AppState>, run_id: i64) -> i64 {
    let conn = state.pool.get().unwrap();
    conn.execute(
        "INSERT INTO outbox(run_id, topic, payload_json, status, idempotency_key, created_at)
         VALUES (?1, 'workflow/log', '{}', 'pending', ?2, 1)",
        rusqlite::params![run_id, format!("seed-prior-{run_id}")],
    )
    .unwrap();
    conn.last_insert_rowid()
}

async fn decide_handoff(state: &Arc<AppState>, run_id: i64, transition: &str) {
    let _receipt = post_handoff_decision(
        State(state.clone()),
        OptPrincipal(None),
        AxumPath(run_id),
        Json(HandoffDecisionBody {
            transition: transition.to_string(),
            decision_ref: Some("op-2026-09-29#7".to_string()),
        }),
    )
    .await
    .unwrap_or_else(|e| panic!("handoff `{transition}` must land: {e:?}"));
}

fn lineage_rows(
    state: &Arc<AppState>,
    run_id: i64,
) -> Vec<(i64, String, String, String, Option<i64>)> {
    let conn = state.pool.get().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT id, topic, payload_json, idempotency_key, parent_id FROM outbox
              WHERE run_id = ?1 AND topic = ?2 ORDER BY id",
        )
        .expect("prepare");
    stmt.query_map(rusqlite::params![run_id, TOPIC], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<i64>>(4)?,
        ))
    })
    .expect("query")
    .collect::<Result<Vec<_>, _>>()
    .expect("rows")
}

fn count_for(state: &Arc<AppState>, run_id: i64) -> i64 {
    let conn = state.pool.get().unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM outbox WHERE run_id = ?1 AND topic = ?2",
        rusqlite::params![run_id, TOPIC],
        |r| r.get(0),
    )
    .expect("count")
}

// ─────────────────────────────────────────────────────────────────────────────
// R57b.1 — the handoff transition lands ONE lineage row, parented at the tip
// ─────────────────────────────────────────────────────────────────────────────

/// A handoff transition driven through the real handler writes exactly one
/// `case/decision` row under its run, parented at the run's PRIOR tip, keyed
/// by the decision itself.
///
/// The parent assertion is the load-bearing half. `append_lineage` reads
/// `MAX(id)` for the run and hands it to `enqueue_child` as the parent, so a
/// row that is merely PRESENT says nothing about whether the chain is welded
/// to the run — a row with `parent_id IS NULL`, or parented at another run's
/// tip, would satisfy a count-only pin and leave the lineage severed.
#[tokio::test]
async fn r57b1_handoff_transition_lands_one_lineage_row_parented_at_the_tip() {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let state = state(&tmp);
    let run_id = seed_run(&state);
    let prior_tip = seed_prior_event(&state, run_id);

    decide_handoff(&state, run_id, "delivered").await;

    assert_eq!(
        count_for(&state, run_id),
        1,
        "a delivered handoff must write exactly one case/decision row for run {run_id}"
    );

    let rows = lineage_rows(&state, run_id);
    let (id, topic, _payload, key, parent) = &rows[0];
    assert_eq!(topic, TOPIC);
    assert_eq!(
        *parent,
        Some(prior_tip),
        "the lineage row must parent at the run's prior tip, not start a new branch"
    );
    assert!(
        *id > prior_tip,
        "a child id is strictly greater than its parent's — the chain is append-only"
    );

    // The key is EXACTLY (run, kind, subject, outcome) and nothing else.
    // Stable means: reproducible from the four fields, with no clock and no
    // decision reference smuggled in. A key carrying `at` would mint a second
    // row for a replayed decision and silently turn exactly-once into
    // at-least-once.
    assert_eq!(
        key,
        &format!("run{run_id}:{TOPIC}:handoff_transition:loopback:delivered"),
        "the idempotency key must be (run, kind, subject, outcome) — no clock, no reference"
    );

    // The row is chain-well-formed: the parent exists, belongs to THIS run,
    // and carries a smaller id (the shipped `verify_outbox_lineage` law,
    // asserted here in its run-scoped form — the function itself is crate
    // internal, and a pin that cannot call its subject re-implements it).
    let (parent_run, parent_id): (i64, i64) = {
        let conn = state.pool.get().unwrap();
        conn.query_row(
            "SELECT p.run_id, p.id FROM outbox p WHERE p.id = ?1",
            rusqlite::params![prior_tip],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the parent row is readable")
    };
    assert_eq!(
        parent_run, run_id,
        "a parent from another run would stitch false ancestry across runs"
    );
    assert!(
        parent_id < *id,
        "the chain is append-only: a parent id is strictly smaller than its child"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// R57b.2 — exactly-once per decision, distinct per outcome
// ─────────────────────────────────────────────────────────────────────────────

/// Replaying the SAME decision adds no second row; a DIFFERENT outcome in the
/// same run IS a different row.
///
/// Both halves matter and they fail in opposite directions. A key that carried
/// only the run would collapse two real decisions into one — a handoff
/// delivered and then cancelled would leave a chain that says `delivered` and
/// nothing else, and the reader has no way to know a second decision was
/// taken. A key that carried the clock would turn every retry into a new
/// decision — the replay class `INSERT OR IGNORE` exists to refuse. The key is
/// the decision, and these are the two ways to get it wrong.
#[tokio::test]
async fn r57b2_replay_is_exactly_once_and_a_new_outcome_is_a_new_row() {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let state = state(&tmp);
    let run_id = seed_run(&state);

    decide_handoff(&state, run_id, "delivered").await;
    assert_eq!(count_for(&state, run_id), 1, "first decision, one row");

    // The same decision three more times — a client retry, a re-sent form, a
    // second caller's identical operator action.
    for _ in 0..3 {
        decide_handoff(&state, run_id, "delivered").await;
    }
    assert_eq!(
        count_for(&state, run_id),
        1,
        "a replayed decision is a no-op receipt, not a second event"
    );

    // A genuinely different decision by the same owner in the same run.
    decide_handoff(&state, run_id, "cancelled").await;
    assert_eq!(
        count_for(&state, run_id),
        2,
        "a different outcome is a different decision and must be its own row"
    );

    let rows = lineage_rows(&state, run_id);
    let outcomes: Vec<String> = rows
        .iter()
        .map(|(_, _, payload, _, _)| {
            serde_json::from_str::<serde_json::Value>(payload).expect("payload parses")["outcome"]
                .as_str()
                .expect("outcome is a string")
                .to_string()
        })
        .collect();
    assert_eq!(
        outcomes,
        vec!["delivered".to_string(), "cancelled".to_string()],
        "both decisions must survive, in the order they were taken"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// the payload grammar — CLOSED, and refs only
// ─────────────────────────────────────────────────────────────────────────────

/// The stored payload carries EXACTLY the five preregistered keys, across all
/// three writers.
///
/// The interesting half is the absence. Two of the three call sites write a
/// body into the session store in the very same transaction — the handoff's
/// `detail`, the back-referral's `contract` and the operator's `report` — and
/// both payloads are sitting there as a template to copy. They must not reach
/// the lineage, because the lineage is a broadcast surface: the alert worker's
/// drain republishes every `case/%` row to the workflow event stream and to a
/// webhook sink when one is configured. A clinical finding in a broadcast
/// payload is a disclosure the operator never consented to.
///
/// So the assertion is not "the keys we expect are present" — it is "the key
/// set is EXACTLY this, and none of these three names is in it".
#[tokio::test]
async fn case_decision_payload_carries_only_the_closed_key_set() {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let state = state(&tmp);
    let run_id = seed_run(&state);
    let now = chrono::Utc::now().timestamp();
    // Two runs, three writers: the handoff and the back-referral release both
    // live on runs, and keeping them apart is what makes the per-writer
    // "exactly one row" assertion mean something instead of counting the
    // other writer's decision.
    let referral_run = seed_run(&state);
    let contract_key = seed_contract(&state, referral_run, now);
    let account_id = seed_account(&state, "Acme Limited");

    // Writer 1 — handoff, whose session payload carries `detail`.
    decide_handoff(&state, run_id, "delivered").await;
    // Writer 2 — back-referral, whose session payload carries `contract` and
    // the operator's `report`.
    let _receipt = post_back_referral_return(
        State(state.clone()),
        OptPrincipal(None),
        AxumPath(referral_run),
        Json(BackReferralReturnBody {
            contract_key: Some(contract_key.clone()),
            report: Some(serde_json::json!({ "finding": "cell swelling at 40% SOC" })),
            decision_ref: Some("op-2026-09-29#8".to_string()),
        }),
    )
    .await
    .expect("the release must land");
    // Writer 3 — pipeline.
    let _receipt = post_pipeline(
        State(state.clone()),
        OptPrincipal(None),
        AxumPath(account_id),
        Json(PipelineBody {
            stage: "qualified".to_string(),
            decision_ref: Some("op-2026-09-29#9".to_string()),
        }),
    )
    .await
    .expect("the advance must land");

    const CLOSED: [&str; 5] = ["at", "decision_ref", "kind", "outcome", "subject"];
    const FORBIDDEN: [&str; 4] = ["detail", "contract", "report", "clinical_question"];

    let mut seen_kinds: Vec<String> = Vec::new();
    for (run, label) in [
        (run_id, "handoff"),
        (referral_run, "back_referral"),
        (account_id, "pipeline"),
    ] {
        let rows = lineage_rows(&state, run);
        assert_eq!(rows.len(), 1, "{label} must write exactly one lineage row");
        let value: serde_json::Value =
            serde_json::from_str(&rows[0].2).expect("payload parses as JSON");
        let object = value.as_object().expect("payload is an object");

        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys, CLOSED,
            "{label}: the lineage payload grammar is CLOSED and preregistered \
             (kind, decision_ref, subject, outcome, at) — a sixth key is a new \
             vocabulary decision, not an implementation detail"
        );
        for banned in FORBIDDEN {
            assert!(
                !object.contains_key(banned),
                "{label}: `{banned}` must never reach a broadcast lineage payload — \
                 it lives in the session row, which is not republished"
            );
        }
        // The clinical prose from writer 2 must be absent by VALUE too, not
        // only by key: a body smuggled into `subject` would pass a key-set
        // check and leak just the same.
        assert!(
            !rows[0].2.contains("cell swelling"),
            "{label}: no free text of any kind may appear in a lineage payload"
        );
        seen_kinds.push(
            value["kind"]
                .as_str()
                .expect("kind is a string")
                .to_string(),
        );
    }

    seen_kinds.sort();
    let mut expected = CASE_DECISION_KINDS.to_vec();
    expected.sort_unstable();
    assert_eq!(
        seen_kinds, expected,
        "each of the three writers stamps its own kind, and the vocabulary is closed"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// topic freedom — the topic is NOT reserved, and must stay that way
// ─────────────────────────────────────────────────────────────────────────────

/// `case/decision` is not a reserved topic.
///
/// The reserved list is a list of DISPATCHES — rows addressed to something
/// outside the run, each with a gate that re-verifies consent, role, window or
/// key at the enqueue. This topic is the run's own record, addressed to
/// nobody. Reserving it would make `topic_is_reserved` assert a capability the
/// topic does not have, and it would also mean the append had to mint a
/// `KernelOrigin` it has no business holding. The other direction is worse:
/// if the topic ever DOES start reading as reserved, `append_case_decision`
/// starts failing at runtime on every handoff — which is why the append would
/// then need a kernel token, and why this pin is worth a test rather than a
/// comment.
#[test]
fn case_decision_topic_is_free_in_the_reserved_vocabulary() {
    assert!(
        !topic_is_reserved(TOPIC),
        "`{TOPIC}` must not be reserved: it is a record of a decision already \
         taken, not a dispatch with a gate to re-verify"
    );
    // And the surrounding vocabulary is untouched — a prefix arm that swallowed
    // it would pass the assertion above and still break every append.
    for reserved in brain_server::workflow::outbox::RESERVED_OUTBOX_TOPICS {
        assert!(
            !TOPIC.starts_with(*reserved),
            "`{TOPIC}` must not fall under the reserved prefix `{reserved}`"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// the closed outcome vocabulary is not allowed to drift from its two sources
// ─────────────────────────────────────────────────────────────────────────────

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{} must be readable: {e}. A read-as-text pin that cannot read its \
             subject passes vacuously, which is worse than no pin.",
            path.display()
        )
    })
}

/// The body of one `impl` block: the header through the first line that is a
/// bare `}` at column zero. The cut is the control — scanning the whole file
/// would pick up this pin's own literals.
fn impl_block(src: &str, header: &str) -> String {
    let start = src
        .find(header)
        .unwrap_or_else(|| panic!("`{header}` must exist — this pin reads the source it guards"));
    let rest = &src[start + header.len()..];
    let end = rest
        .find("\n}\n")
        .unwrap_or_else(|| panic!("the `{header}` block must be closed"));
    rest[..end].to_string()
}

/// Every `=> "word"` arm in a block: the `as_str` mappings of a closed enum.
fn mapped_words(block: &str) -> Vec<String> {
    block
        .lines()
        .filter_map(|line| {
            let (_, tail) = line.split_once("=> \"")?;
            let (word, _) = tail.split_once('"')?;
            Some(word.to_string())
        })
        .collect()
}

/// The closed outcome list must cover EVERY word the two source enums can
/// produce — and the two source enums must produce nothing else.
///
/// This is the pin that makes the refusal in `append_case_decision` safe. The
/// append refuses an outcome outside [`CASE_DECISION_OUTCOMES`] rather than
/// writing a row no lineage reader can interpret, which is fail-closed and
/// correct — but it is a REFUSAL in a working handoff path if a new transition
/// is added upstream and this list is not widened with it. The alternative to
/// a CI failure there is an operator discovering it.
#[test]
fn case_decision_outcomes_cover_every_source_vocabulary() {
    let handoff = mapped_words(&impl_block(
        &read("src/workflow/gdl.rs"),
        "impl HandoffTransition {",
    ));
    let stages = mapped_words(&impl_block(
        &read("src/workflow/pipeline.rs"),
        "impl Stage {",
    ));

    assert!(
        !handoff.is_empty() && !stages.is_empty(),
        "both source vocabularies must be readable — an empty arm list is a \
         broken scan, not an empty vocabulary"
    );
    for (name, words) in [("HandoffTransition", &handoff), ("Stage", &stages)] {
        for word in words {
            assert!(
                CASE_DECISION_OUTCOMES.contains(&word.as_str()),
                "{name} can produce `{word}`, which is absent from \
                 CASE_DECISION_OUTCOMES — the append would refuse a live decision"
            );
        }
    }
    // And the reverse: a lineage outcome nobody can produce is a word no
    // reader of the chain can ever see resolve.
    for word in CASE_DECISION_OUTCOMES {
        assert!(
            handoff.iter().any(|w| w == word)
                || stages.iter().any(|w| w == word)
                || *word == "returned",
            "CASE_DECISION_OUTCOMES carries `{word}`, which no writer can \
             produce — an outcome word that can never appear is a false affordance"
        );
    }
    // The union, spelled out: five transitions, one release, five stages.
    assert_eq!(
        CASE_DECISION_OUTCOMES.len(),
        11,
        "the union is 5 handoff transitions + 1 back-referral release + 5 \
         pipeline stages; a count that moves without a source word moving is a \
         vocabulary nobody reviewed"
    );
}
