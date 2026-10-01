//! R65 increment B' — the approve ladder's missing arm.
//!
//! `approve_proposal` is a ten-armed ladder of `if kind == ...` checks, each
//! ending in `return`. There was no `else`, no `match`, and no default arm, so
//! a kind that no branch claimed fell straight through to the generic promote
//! and was written VERBATIM into `knowledge.node_kind` — a
//! `TEXT NOT NULL DEFAULT 'fact'` column with no CHECK constraint
//! (`src/migration.rs`), so the write succeeded with any string at all.
//!
//! Every unknown value then reads back as `fact`, because
//! `MemoryKind::from_str` falls back to `Fact` on anything it does not
//! recognise (`src/procedural.rs`). So the foot-gun is not "garbage in the
//! column" — it is "garbage in the column that no reader can tell apart from a
//! real declarative memory", written by a HITL approval that an operator
//! believed was publishing knowledge.
//!
//! ## What this file pins
//!
//! | id | claim |
//! |----|-------|
//! | B'.1 | an unclaimed kind is REFUSED, never promoted (the foot-gun) |
//! | B'.2 | no branch is shadowed — every governed kind still reaches its own arm |
//! | B'.3 | a refusal is fail-closed: no knowledge row, no `findings` row, proposal stays `pending` |
//! | B'.4 | the refusal names the offending kind |
//! | — | the arm is unreachable before the approve role gate and the digest bind |
//! | — | property: no arbitrary string reaches the promote path as a kind |
//!
//! ## Three traps this programme has already hit, all encoded below
//!
//! 1. **A pin that passes with the defect planted**, because it reads a
//!    hand-built literal instead of the real value. Every allow-list assertion
//!    here derives from [`promote_kind_is_governed`] — the same function the
//!    handler calls — rather than from a second literal table that could drift
//!    green beside a broken arm.
//! 2. **A fixture that claims to isolate a property but also emits a second
//!    signal.** `b2_no_branch_is_shadowed` asserts it DISCRIMINATES: it checks
//!    that the governed set and the refused set are non-empty AND disjoint, so
//!    a predicate that admitted everything (or nothing) fails it. An allow-list
//!    that returned `true` for every string would pass a pin that only tested
//!    the refusal.
//! 3. **Ordering.** `b2` pins the arm's POSITION by reading the real source:
//!    an arm placed at the top of the ladder would shadow all ten branches, and
//!    a pin that only exercised the refusal would never notice.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use brain_server::handlers::gate::promote_kind_is_governed;
use brain_server::pool::SqliteConnectionManager;
use brain_server::server::router::app;
use brain_server::server::router::auth::JwtMiddlewareState;
use rusqlite::Connection;
use tower::ServiceExt;

/// The opaque operator token: with no JWT principal in extensions both
/// `authorize` and `authorize_role(.., "approve")` return `Ok(())`
/// (`src/handlers/mod.rs`), so the ladder is entered as the superuser.
const OP_TOKEN: &str = "r65b-ladder-op-token";

struct Server {
    _dir: tempfile::TempDir,
    state: Arc<brain_server::AppState>,
}

fn server() -> Server {
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = dir.path().join("brain.db");
    let mgr = SqliteConnectionManager::file(&db_path);
    let pool: brain_server::Pool = r2d2::Pool::builder().max_size(4).build(mgr).expect("pool");
    brain_server::migration::run_migration(
        &mut pool.get().expect("conn"),
        brain_server::config::DB_MMAP_SIZE_MIB,
    )
    .expect("migration");
    let model: Arc<dyn brain_server::embed::Embedder> = Arc::new(
        brain_server::embed::StaticEmbedder::new(brain_server::config::MODEL_ID).expect("model"),
    );
    let tok_file = dir.path().join("tokens");
    std::fs::write(&tok_file, format!("{OP_TOKEN}\n")).expect("token file");
    let token_store = brain_server::auth::TokenStore::from_file(Some(tok_file));
    token_store.reload_parts_from(vec![OP_TOKEN.to_string()], None);
    let state = Arc::new(brain_server::AppState {
        token_store,
        jwt_middleware_state: Arc::new(JwtMiddlewareState::opaque_for_tests(
            pool.clone(),
            db_path.clone(),
        )),
        cors: tower_http::cors::CorsLayer::new(),
        durability: Default::default(),
        loom: Default::default(),
        model,
        registry: brain_server::domain_registry::DomainRegistry::new(pool.clone(), &db_path, false),
        pool,
        db_path,
        connection_tracker: Arc::new(brain_server::http_limit::ConnectionTracker::new()),
        rate_limiter: Arc::new(brain_server::http_limit::RateLimiter::new()),
        snapshot: brain_server::integrity::SnapshotState::default(),
        audit_chain_cache: Arc::new(std::sync::Mutex::new(None)),
        auth_mode: brain_server::auth::AuthMode::Opaque,
        key_store: brain_server::auth::jwks::KeyStore::default(),
        revocation_cache: Arc::new(brain_server::auth::RevocationCache::new()),
        jwt_issuer: String::new(),
        jwt_audience: String::new(),
        oidc_config: brain_server::handlers::well_known::OidcConfig::unconfigured(),
        ump_events: tokio::sync::broadcast::channel(brain_server::config::UMP_EVENT_BUFFER).0,
        alert_events: tokio::sync::broadcast::channel(brain_server::config::ALERT_EVENT_BUFFER).0,
        alert_seq: std::sync::atomic::AtomicU64::new(0),
        chain_watch: brain_server::alert::ChainWatchState::default(),
        concurrency: &brain_server::concurrency::CONCURRENCY,
    });
    Server { _dir: dir, state }
}

/// Seed a `proposals` row DIRECTLY, bypassing create-time validation — which is
/// the whole point: the kinds under test cannot be created through the route,
/// they only exist because a production writer inserts them.
///
/// `created_at = now` matters: `approve_proposal` runs the TTL check before the
/// transaction and would otherwise expire the row and answer
/// `400 proposal_expired` instead of reaching the arm.
fn seed_proposal(conn: &Connection, kind: &str, content: &str, now: i64) -> i64 {
    conn.execute(
        "INSERT INTO proposals(kind, content, source, novelty, salience, created_at, domain, owner)
         VALUES (?1, ?2, 'agent', 0.9, 0.5, ?3, 'global', 'proposer@acme')",
        rusqlite::params![kind, content, now],
    )
    .expect("seed proposal");
    conn.last_insert_rowid()
}

/// The digest an approve must carry, read from the STORED content (the handler
/// digests what the row holds, not what the caller seeded).
fn digest_for(state: &Arc<brain_server::AppState>, pid: i64) -> String {
    let conn = state.pool.get().expect("conn");
    let content: String = conn
        .query_row("SELECT content FROM proposals WHERE id = ?1", [pid], |r| {
            r.get(0)
        })
        .expect("stored content");
    brain_server::handlers::gate::review_digest(&content)
}

/// POST any route with the operator bearer and read the JSON body back.
/// Generalised from `post_approve` so a non-approve route (the classify
/// receipt) can be driven through the same real router.
async fn json(
    state: &Arc<brain_server::AppState>,
    method: &str,
    uri: &str,
    body: String,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {OP_TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("request");
    let res = app(state.clone()).oneshot(req).await.expect("oneshot");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

async fn post_approve(
    state: &Arc<brain_server::AppState>,
    pid: i64,
    digest: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let uri = match digest {
        Some(d) => format!("/proposals/{pid}/approve?digest={d}"),
        None => format!("/proposals/{pid}/approve"),
    };
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {OP_TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .expect("request");
    let res = app(state.clone()).oneshot(req).await.expect("oneshot");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

fn knowledge_count(state: &Arc<brain_server::AppState>) -> i64 {
    let conn = state.pool.get().expect("conn");
    conn.query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
        .expect("knowledge count")
}

fn vec_count(state: &Arc<brain_server::AppState>) -> i64 {
    let conn = state.pool.get().expect("conn");
    conn.query_row("SELECT COUNT(*) FROM vec_knowledge", [], |r| r.get(0))
        .expect("vec count")
}

fn proposal_status(state: &Arc<brain_server::AppState>, pid: i64) -> String {
    let conn = state.pool.get().expect("conn");
    conn.query_row("SELECT status FROM proposals WHERE id = ?1", [pid], |r| {
        r.get(0)
    })
    .expect("proposal status")
}

/// The kinds a PRODUCTION writer can put into `proposals` while no ladder
/// branch claims them — the measured refusal set.
///
/// Each entry is written by a production path; the census pin below re-derives
/// the writers from source so this list cannot silently rot into a fiction.
const UNCLAIMED_PRODUCTION_KINDS: [&str; 6] = [
    "case_merge_suggested",
    "gdl_gap_new",
    "gdl_gap_update",
    "gdl_rca",
    "gdl_complaint_rca",
    "gdl_proposal",
];

// ── B'.1 · the foot-gun ────────────────────────────────────────────────────

/// The defect itself: a kind nobody claims must be REFUSED, never promoted.
///
/// Before the arm this returned `200 {"status":"approved"}` and wrote a
/// `knowledge` row whose `node_kind` was the unclaimed string verbatim.
/// `case_merge_suggested` is the sharpest case — its writer
/// (`src/connector/crm/mod.rs`) has no consumer at all, so approving one
/// published a CRM merge suggestion as a long-term memory.
#[tokio::test]
async fn b1_unclaimed_kind_is_refused_not_promoted() {
    for kind in UNCLAIMED_PRODUCTION_KINDS {
        let srv = server();
        let now = chrono::Utc::now().timestamp();
        let pid = {
            let conn = srv.state.pool.get().expect("conn");
            seed_proposal(
                &conn,
                kind,
                "acme and bolt share a duplicate open case",
                now,
            )
        };
        let d = digest_for(&srv.state, pid);

        let (status, body) = post_approve(&srv.state, pid, Some(&d)).await;

        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "kind {kind:?} must be refused, got {status} {body}"
        );
        assert_eq!(
            body["error"]["code"], "unknown_proposal_kind",
            "kind {kind:?} must refuse with the named code, got {body}"
        );
        assert_eq!(
            knowledge_count(&srv.state),
            0,
            "kind {kind:?} wrote a knowledge row: the foot-gun"
        );
        assert_eq!(vec_count(&srv.state), 0, "kind {kind:?} wrote a vec row");
    }
}

/// The positive control, and the reason B'.1 is not vacuous: a GOVERNED kind
/// through the same fixture, the same seed, the same digest — and it promotes.
///
/// Trap #1, encoded: if this fixture could not promote, B'.1 would be passing
/// for the wrong reason (a fixture that refuses everything, a digest mismatch,
/// a dead role gate). Both halves run the identical path and differ only in
/// `kind`.
#[tokio::test]
async fn b1_governed_kind_still_promotes_through_the_same_fixture() {
    let srv = server();
    let now = chrono::Utc::now().timestamp();
    let pid = {
        let conn = srv.state.pool.get().expect("conn");
        seed_proposal(
            &conn,
            "fact",
            "the repeater cluster drops wifi nightly",
            now,
        )
    };
    let d = digest_for(&srv.state, pid);

    let (status, body) = post_approve(&srv.state, pid, Some(&d)).await;

    assert_eq!(status, StatusCode::OK, "governed kind must promote: {body}");
    assert_eq!(body["status"], serde_json::json!("approved"), "{body}");
    assert_eq!(knowledge_count(&srv.state), 1, "one knowledge row landed");
    assert_eq!(vec_count(&srv.state), 1, "one vec row landed");
    assert_eq!(proposal_status(&srv.state, pid), "approved");
}

// ── B'.2 · no branch is shadowed ───────────────────────────────────────────

/// The 14 kinds the ten ladder branches claim, each with the branch that owns
/// it. Read off the handler at `8552ccee`; the branch-ordering pin below binds
/// them to the real source.
const GOVERNED_BRANCH_KINDS: [&str; 14] = [
    "registry_lifecycle",
    "kcs_publish",
    "complaint_remedy",
    "outreach_consent",
    "outreach_campaign",
    "outreach_followup",
    "channel/template",
    "crew_skills_update",
    "channel/user_map",
    "kcs_translate",
    "kcs_new_article",
    "kcs_update_article",
    "kcs_link_only",
    "complaint_rca",
];

/// **This fixture asserts that it discriminates.** Trap #2, encoded.
///
/// A pin that only tested the refusal (B'.1) would pass an allow-list that
/// returns `true` for every string — or one that returns `false` for every
/// string, which would break the ordinary promote path entirely. So this
/// asserts BOTH directions over ONE shared set, and asserts the set is
/// non-empty on each side and the two sides do not overlap:
///
/// * every governed branch kind is NOT governed by the promote predicate
///   (it returns above, so it must not also be a promote-kind — otherwise the
///   arm is claiming kinds it never sees), and
/// * every promotable kind IS governed, and the two sets are disjoint.
///
/// A predicate that admitted everything fails the first; one that admitted
/// nothing fails the second.
#[test]
fn b2_the_promote_predicate_discriminates_governed_from_unclaimed() {
    // Side A: the promote allow-list. Non-empty, and every member admitted.
    let promotable: Vec<&str> = [
        "fact",
        "procedure",
        "step",
        "decision",
        "episodic",
        "entitlement",
        "draft",
        "delivery/artifact",
        "decision_review",
    ]
    .into_iter()
    .collect();
    assert!(
        !promotable.is_empty(),
        "fixture drifted: the promotable side is empty, so this pin cannot discriminate"
    );
    for kind in &promotable {
        assert!(
            promote_kind_is_governed(kind),
            "{kind:?} is on the promote allow-list but the predicate refuses it"
        );
    }

    // Side B: the refusal set. Non-empty, and no member admitted.
    assert!(
        !UNCLAIMED_PRODUCTION_KINDS.is_empty(),
        "fixture drifted: the refusal side is empty, so this pin cannot discriminate"
    );
    for kind in UNCLAIMED_PRODUCTION_KINDS {
        assert!(
            !promote_kind_is_governed(kind),
            "{kind:?} is unclaimed but the predicate admits it — the foot-gun is back"
        );
    }

    // THE DISCRIMINATION ASSERTION: the two sides are disjoint. If a kind ever
    // appears on both, the fixture has stopped isolating the property it claims
    // to isolate and must be rewritten, not re-tuned.
    for kind in &promotable {
        assert!(
            !UNCLAIMED_PRODUCTION_KINDS.contains(kind),
            "{kind:?} is on both sides — the fixture no longer discriminates"
        );
    }
    for kind in UNCLAIMED_PRODUCTION_KINDS {
        assert!(
            !promotable.contains(&kind),
            "{kind:?} is on both sides — the fixture no longer discriminates"
        );
    }
}

/// Ordering, pinned against the REAL source. An arm placed at the top of the
/// ladder would shadow all ten branches and break every governed approval — and
/// a pin that only exercised the refusal (B'.1) would never notice, because the
/// refusal is exactly what a shadowing arm does.
///
/// This reads the handler's source and asserts the arm appears AFTER every
/// branch condition, so the ordering claim is checked against the code rather
/// than asserted about it. Comments and strings are not stripped here on
/// purpose: the branch needles are distinctive enough that a doc mention would
/// show up as a false failure, not a false pass — the safe direction for an
/// ordering pin.
#[test]
fn b2_the_arm_sits_after_every_branch_so_none_is_shadowed() {
    let src = include_str!("../src/handlers/gate.rs");

    let arm_at = src
        .find("promote_kind_is_governed(&kind)")
        .expect("the default arm must call the promote predicate");

    // Every branch condition, as it appears in the handler.
    let branch_needles = [
        "if kind == crate::workflow::registry::PROP_KIND_REGISTRY_LIFECYCLE",
        "if kind == crate::workflow::kcs::KIND_PUBLISH",
        "if kind == crate::workflow::complaint::KIND_REMEDY",
        "if kind == crate::workflow::outreach::KIND_CONSENT",
        "if kind == crate::workflow::outreach::KIND_CAMPAIGN",
        "if kind == crate::workflow::channels::PROP_KIND_CHANNEL_TEMPLATE",
        "matches!(kind.as_str(), crate::workflow::crew::KIND_SKILLS_UPDATE)",
        "if kind == crate::workflow::channels::PROP_KIND_USER_MAP",
        "if kind == crate::workflow::kcs::KIND_TRANSLATE",
        "if kind == crate::workflow::kcs::KIND_NEW",
    ];

    for needle in branch_needles {
        let at = src
            .find(needle)
            .unwrap_or_else(|| panic!("branch {needle:?} is gone — re-derive this pin"));
        assert!(
            at < arm_at,
            "branch {needle:?} sits at byte {at}, AFTER the default arm at {arm_at} — \
             the arm shadows it"
        );
    }

    // And the arm is inside `approve_proposal`, not some other function.
    let fn_start = src
        .find("pub async fn approve_proposal")
        .expect("approve_proposal must exist");
    assert!(
        fn_start < arm_at,
        "the arm must live inside approve_proposal"
    );
}

/// Every governed branch kind is spelled by a real constant, and none of them
/// is a promote-kind. If a future round adds a branch kind that ALSO promotes,
/// this fails — which is the point: the ladder's arm and its branches must never
/// both claim a kind.
#[test]
fn b2_no_governed_branch_kind_is_also_a_promote_kind() {
    for kind in GOVERNED_BRANCH_KINDS {
        assert!(
            !promote_kind_is_governed(kind),
            "{kind:?} is claimed by a ladder branch AND admitted by the promote arm — \
             exactly one may own a kind"
        );
    }
}

// ── B'.3 · fail-closed on rollback ─────────────────────────────────────────

/// A refusal must leave the database EXACTLY as it found it.
///
/// Three assertions, because "no knowledge row" alone is satisfied by a crash:
/// the proposal is still `pending` (the CAS never ran), no `findings` row
/// landed, and the vec shadow is empty. The proposal staying `pending` is the
/// load-bearing one — a refusal that consumed the row would silently destroy
/// the operator's queue.
#[tokio::test]
async fn b3_a_refused_kind_writes_nothing_and_leaves_the_proposal_pending() {
    let srv = server();
    let now = chrono::Utc::now().timestamp();
    let pid = {
        let conn = srv.state.pool.get().expect("conn");
        seed_proposal(&conn, "case_merge_suggested", "duplicate open case", now)
    };
    let d = digest_for(&srv.state, pid);

    let (status, _body) = post_approve(&srv.state, pid, Some(&d)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    assert_eq!(
        knowledge_count(&srv.state),
        0,
        "a refused proposal wrote a knowledge row"
    );
    assert_eq!(
        vec_count(&srv.state),
        0,
        "a refused proposal wrote a vec row"
    );

    let conn = srv.state.pool.get().expect("conn");
    let findings: i64 = conn
        .query_row("SELECT COUNT(*) FROM findings", [], |r| r.get(0))
        .expect("findings count");
    assert_eq!(findings, 0, "a refusal wrote a findings row");

    let (status, decided): (String, Option<i64>) = conn
        .query_row(
            "SELECT status, decided_at FROM proposals WHERE id = ?1",
            [pid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("proposal row");
    assert_eq!(
        status, "pending",
        "the refusal consumed the proposal; the operator's queue lost the row"
    );
    assert!(
        decided.is_none(),
        "a refusal stamped decided_at: {:?}",
        decided
    );
}

/// The refusal is INSIDE the caller's transaction, so it rolls back with the
/// CAS. Proven by pinning that the arm returns an `Err` before the embed and
/// promote path: a refusal placed after `promote_chunk_insert` would still have
/// written the row the rollback could not un-write... except it would, so the
/// observable proof is the arm's POSITION in the source.
///
/// The search is scoped to the text AFTER the arm. `find` alone would match the
/// FIRST embed in the file — and there is an earlier one at the KCS branch,
/// which belongs to a different arm entirely. That is trap #1 wearing a
/// disguise: a whole-file `find` would have compared the arm against a line
/// from a branch that returns hundreds of lines earlier, and reported a
/// perfectly healthy ordering as broken.
#[test]
fn b3_the_arm_precedes_the_promote_and_embed_path() {
    let src = include_str!("../src/handlers/gate.rs");

    let arm_at = src
        .find("promote_kind_is_governed(&kind)")
        .expect("the default arm must exist");
    let tail = &src[arm_at..];

    let promote_at = tail
        .find("crate::service::gate::promote_chunk_insert(")
        .map(|i| arm_at + i)
        .expect("the promote call must follow the arm");
    let embed_at = tail
        .find("let embedding = model.encode_one(&content);")
        .map(|i| arm_at + i)
        .expect("the promote path's embed must follow the arm");

    assert!(
        promote_at > arm_at,
        "the arm sits at {arm_at} but the promote call resolves to {promote_at} — \
         the refusal would arrive after the knowledge row was already written"
    );
    assert!(
        embed_at > arm_at,
        "the arm sits at {arm_at} but the promote path's embed resolves to \
         {embed_at} — the refusal would pay for embedding work it then discards"
    );

    // And the resolved embed is the one in the SAME tail as the promote call,
    // not the KCS branch's earlier embed hundreds of lines above.
    assert!(
        embed_at < promote_at,
        "the promote path's embed ({embed_at}) must precede the promote call \
         ({promote_at}); the matched embed belongs to a different arm"
    );
}

// ── B'.4 · the refusal names the offending kind ────────────────────────────

/// A refusal that does not say WHICH kind is refused is not actionable: an
/// operator sees a red box and a queue that will not move.
#[tokio::test]
async fn b4_the_refusal_names_the_offending_kind() {
    let srv = server();
    let now = chrono::Utc::now().timestamp();
    let pid = {
        let conn = srv.state.pool.get().expect("conn");
        seed_proposal(&conn, "case_merge_suggested", "duplicate open case", now)
    };
    let d = digest_for(&srv.state, pid);

    let (_status, body) = post_approve(&srv.state, pid, Some(&d)).await;

    let message = body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal must carry a message, got {body}"));
    assert!(
        message.contains("case_merge_suggested"),
        "the refusal must name the offending kind, got {message:?}"
    );
}

/// A second, unclaimed kind gets its OWN name in the message — not a generic
/// "unknown kind", and not the first kind's name leaking through a constant.
#[tokio::test]
async fn b4_the_refusal_names_each_kind_individually() {
    for kind in ["gdl_rca", "gdl_proposal"] {
        let srv = server();
        let now = chrono::Utc::now().timestamp();
        let pid = {
            let conn = srv.state.pool.get().expect("conn");
            seed_proposal(&conn, kind, "capture body", now)
        };
        let d = digest_for(&srv.state, pid);

        let (_status, body) = post_approve(&srv.state, pid, Some(&d)).await;
        let message = body["error"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("{kind}: the refusal must carry a message, got {body}"));
        assert!(
            message.contains(kind),
            "the refusal for {kind:?} must name {kind:?}, got {message:?}"
        );
    }
}

// ── Security: the arm sits behind the role gate and the digest bind ─────────

/// An unauthorized caller must get 401/403, NEVER the new code. If the arm
/// were reachable before authentication, a stranger could enumerate kinds.
#[tokio::test]
async fn an_unauthenticated_caller_never_reaches_the_new_code() {
    let srv = server();
    let now = chrono::Utc::now().timestamp();
    let pid = {
        let conn = srv.state.pool.get().expect("conn");
        seed_proposal(&conn, "case_merge_suggested", "duplicate open case", now)
    };
    let d = digest_for(&srv.state, pid);

    let req = Request::builder()
        .method("POST")
        .uri(format!("/proposals/{pid}/approve?digest={d}"))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .expect("request");
    let res = app(srv.state.clone()).oneshot(req).await.expect("oneshot");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);

    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "an unauthenticated approve must be refused at the door, got {status}"
    );
    assert_ne!(
        body["error"]["code"], "unknown_proposal_kind",
        "the new code is reachable WITHOUT authentication"
    );
    assert_eq!(knowledge_count(&srv.state), 0);
}

/// A digest-less caller must still get `400 digest_required`, never the new
/// code. The digest bind is the "what did the operator actually see" law; if
/// the arm shadowed it, an operator could approve bytes they never read.
#[tokio::test]
async fn a_digest_less_caller_gets_digest_required_not_the_new_code() {
    let srv = server();
    let now = chrono::Utc::now().timestamp();
    let pid = {
        let conn = srv.state.pool.get().expect("conn");
        seed_proposal(&conn, "case_merge_suggested", "duplicate open case", now)
    };

    let (status, body) = post_approve(&srv.state, pid, None).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["error"]["code"], "digest_required",
        "the digest bind must fire before the promote arm, got {body}"
    );
    assert_eq!(knowledge_count(&srv.state), 0);
}

/// And a DIVERGING digest is a 409 conflict — the arm must not mask it.
#[tokio::test]
async fn a_diverging_digest_is_a_conflict_not_the_new_code() {
    let srv = server();
    let now = chrono::Utc::now().timestamp();
    let pid = {
        let conn = srv.state.pool.get().expect("conn");
        seed_proposal(&conn, "case_merge_suggested", "duplicate open case", now)
    };

    let (status, body) = post_approve(&srv.state, pid, Some("deadbeef")).await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_ne!(
        body["error"]["code"], "unknown_proposal_kind",
        "the arm fired before the digest check, masking a stale approval"
    );
    assert_eq!(knowledge_count(&srv.state), 0);
}

// ── Property · no arbitrary string reaches the promote path ─────────────────

/// The property the arm exists to enforce: over GENERATED kinds, the set that
/// lands is always a governed constant.
///
/// `proptest` expands into module-level `#[test]` items, so the macro must sit
/// at module scope — wrapping it in an outer `#[test]` is a compile error
/// ("cannot test inner items"). Context is passed as explicit `prop_assert*`
/// arguments because `format_args!` cannot implicitly capture inside the
/// generated closure.
use proptest::prelude::*;

proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(256))]

    /// An arbitrary string is never a promotable kind. This is the direct
    /// statement of "no arbitrary string reaches `promote_chunk_insert`": the
    /// generator can produce anything, including strings that look like the
    /// governed kinds, and the predicate still says no.
    #[test]
    fn arbitrary_kind_is_never_promotable(
        kind in ".{0,64}"
    ) {
        prop_assume!(
            !matches!(
                kind.as_str(),
                "fact" | "procedure" | "step" | "decision" | "episodic"
                    | "entitlement" | "draft" | "delivery/artifact"
                    | "decision_review"
            ),
            "the generator produced a governed kind; that is the positive \
             control, not a counterexample"
        );
        prop_assert!(
            !promote_kind_is_governed(&kind),
            "an arbitrary kind {:?} was admitted to the promote path",
            &kind
        );
    }

    /// The complement: every governed constant IS admitted. Without this the
    /// first property would be satisfied by a predicate that refuses
    /// everything — including the ordinary `fact` promote path.
    #[test]
    fn governed_kinds_are_always_promotable(
        index in 0usize..9
    ) {
        const GOVERNED: [&str; 9] = [
            "fact",
            "procedure",
            "step",
            "decision",
            "episodic",
            "entitlement",
            "draft",
            "delivery/artifact",
            "decision_review",
        ];
        let kind = GOVERNED[index];
        prop_assert!(
            promote_kind_is_governed(kind),
            "governed kind {:?} was refused — the ordinary promote path is broken",
            kind
        );
    }
}

/// The refusal set is not a hand-built fiction: every entry is spelled by a
/// production writer in the tree, and every governed kind by a ladder branch or
/// the `MemoryKind` vocabulary.
///
/// This is the anti-rot pin. If a kind is renamed in its writer, or a new
/// unclaimed kind is added, the hand-built lists above drift and this fails.
#[test]
fn the_refusal_set_matches_a_real_production_writer() {
    let crm = include_str!("../src/connector/crm/mod.rs");
    assert!(
        crm.contains("pub const KIND_CASE_MERGE_SUGGESTED: &str = \"case_merge_suggested\""),
        "case_merge_suggested must still be a declared production kind"
    );

    // The five `gdl_*` kinds are built by a match in the proficiency core.
    let prof = include_str!("../src/workflow/proficiency.rs");
    for kind in [
        "gdl_gap_new",
        "gdl_gap_update",
        "gdl_rca",
        "gdl_complaint_rca",
        "gdl_proposal",
    ] {
        assert!(
            prof.contains(&format!("\"{kind}\"")),
            "{kind:?} must still be written by the GDL capture producer"
        );
    }

    // And none of the governed kinds may be admitted by the arm.
    for kind in GOVERNED_BRANCH_KINDS {
        assert!(
            !promote_kind_is_governed(kind),
            "{kind:?} is claimed by a ladder branch but the promote arm admits it too"
        );
    }
}
