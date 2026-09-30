// R57b — the decision trace's model CITATION, and the version-axis coupling
// that made it shippable.
//
// ## Why this file exists separately from the modules
//
// The two largest claims this round makes are about **things that are absent**:
// a second writer of the citation, and a probe that sits AT the ceiling instead
// of above it. A module cannot check its own absence. So the claims are asserted
// from outside, and the driving claims are asserted by RUNNING the shipped
// functions rather than by reading their source — a prior round shipped a
// string-scanning pin that passed with its own pathology planted, and a
// scanner that cannot fail is worse than no check at all.
//
// ## What is driven, and what is read as text
//
// The green path is driven end to end through the REAL router
// (`server::router::app`) against a REAL migrated database: the model is
// registered over the real registration route (which is also where the test
// gets its digest — the test never recomputes a canonical form by hand), the
// run is POSTed to the real decision-run route, the real registry resolver
// decides, and the real trace writer writes. The citation is then read back
// out of the row the writer actually committed and compared against the row
// the resolver actually returned.
//
// Two things are read as text, and each is a fact that has no runtime
// observation: the *only* writer of the citation (an absence), and the *probe
// literal* in the ceiling's own test (a literal in source, by construction
// unobservable from outside). The probe scan is not a hand-chosen string
// list: the string it looks for is DERIVED from the shipped ceiling by
// bumping the patch component, so a ceiling that moves without the probe
// moving is a failure rather than a coincidence.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use brain_server::pool::SqliteConnectionManager;
use brain_server::server::router::app;
use brain_server::server::router::auth::JwtMiddlewareState;
use rusqlite::Connection;
use tower::ServiceExt;

/// The opaque superuser: no principal, every action. The registry's lifecycle
/// transitions are operator actions, so the fixture is an operator.
const OP_TOKEN: &str = "r57b-citation-op-token";

/// A deterministic rules table with a declared identity — the same shape the
/// engine consumes, and the source of the digest under test. The test never
/// computes this table's digest: it asks the registration route for it.
const RULES_JSON: &str = r#"{
  "model_id": "rules-citation",
  "model_version": "1.0.0",
  "rules": [
    { "question_id": "needs_human", "min_evidence": 1, "min_tier": "vetted",
      "output": { "Choice": { "options": ["act", "reject"], "label": "act" } } }
  ]
}"#;

fn config_json(model_digest: &str) -> String {
    format!(
        r#"{{
          "config_schema": "harness.pipeline/v1",
          "pipeline_id": "citation-line",
          "stages": ["normalize","retrieve_context","candidate_generation","decision_model","rules_policy","threshold","action_escalation"],
          "retrieval": {{"rrf_k": 60, "limit": 5, "leg": "vector"}},
          "model": {{"key": "rules:rules-citation", "digest": "{model_digest}"}},
          "thresholds": {{
            "act_labels": ["act"],
            "reject_labels": ["reject"],
            "score_act_at_or_above": 50,
            "score_reject_at_or_below": 10,
            "noul_act_when": true,
            "fallback": "approve"
          }}
        }}"#
    )
}

fn run_body(config: &str, run_id: i64) -> String {
    format!(
        r#"{{
          "config": {config},
          "rules_config": {RULES_JSON},
          "run_id": {run_id},
          "mode": "deterministic",
          "request_id": "req-r57b-1",
          "question_id": "needs_human",
          "question_kind": "choice",
          "question_ids": ["needs_human"],
          "query": "which model produced this run"
        }}"#
    )
}

struct Server {
    _dir: tempfile::TempDir,
    state: Arc<brain_server::AppState>,
}

/// An opaque-mode server on a real file-backed, real migrated database — the
/// `authz_matrix` fixture shape. Every seam under test (the router, the
/// resolver, the writer) is the shipped one; nothing here is a stand-in.
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
        revocation_cache: Arc::new(brain_server::auth::revocation::RevocationCache::new()),
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

async fn json(
    state: &Arc<brain_server::AppState>,
    method: &str,
    uri: &str,
    body: String,
) -> (StatusCode, serde_json::Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {OP_TOKEN}"));
    let res = app(state.clone())
        .oneshot(builder.body(Body::from(body)).expect("request"))
        .await
        .expect("oneshot");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

// ── fixture steps (plain SQL over SHIPPED tables; the seams under test are
// never faked, only fed) ─────────────────────────────────────────────────────

/// One open case, so the route's run resolution has a row to find.
fn seed_case(conn: &Connection) -> i64 {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
         VALUES ('global', 'troubleshoot', '{}', 0, 'active', ?1, ?1)",
        [now],
    )
    .expect("seed case");
    conn.last_insert_rowid()
}

/// The registry's lifecycle transition, applied directly.
///
/// **Declared fixture, not a shortcut around the law.** Registration lands a
/// row as `candidate` and the tree exposes NO route that promotes one: the
/// promotion gate is deliberately not published. This is the operator's
/// approval record written straight to the row it belongs to, which is the
/// only way to reach a `promoted` model from outside the crate. The RESOLVER
/// is untouched — it still reads the row and still decides; the fixture only
/// supplies the state the human would have supplied.
fn set_status(conn: &Connection, id: &str, version: &str, status: &str) {
    let changed = conn
        .execute(
            "UPDATE decision_model_registry SET status = ?1, approved_by = 'r57b-operator'
             WHERE id = ?2 AND version = ?3",
            rusqlite::params![status, id, version],
        )
        .expect("set status");
    assert_eq!(
        changed, 1,
        "the lifecycle fixture must move exactly one row"
    );
}

fn remove_row(conn: &Connection, id: &str, version: &str) {
    conn.execute(
        "DELETE FROM decision_model_registry WHERE id = ?1 AND version = ?2",
        rusqlite::params![id, version],
    )
    .expect("remove row");
}

/// The citation as it was COMMITTED: three independently nullable columns, so
/// a row that cited nothing and a row whose model carried no digest read
/// differently instead of collapsing into one sentinel.
fn committed_citation(
    conn: &Connection,
    trace_row_id: i64,
) -> (Option<String>, Option<String>, Option<String>) {
    conn.query_row(
        "SELECT model_registry_id, model_registry_version, model_registry_digest
         FROM decision_run_traces WHERE id = ?1",
        [trace_row_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .expect("read the committed citation")
}

/// The resolved identity, read from the row the resolver was entitled to read.
fn registry_identity(
    conn: &Connection,
    id: &str,
    version: &str,
) -> (String, String, Option<String>) {
    conn.query_row(
        "SELECT id, version, config_digest FROM decision_model_registry WHERE id = ?1 AND version = ?2",
        rusqlite::params![id, version],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .expect("read the registry row")
}

/// How many trace rows carry a citation at all. Boolean by construction: the
/// refusals below assert this does not MOVE.
fn cited_trace_count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM decision_run_traces WHERE model_registry_id IS NOT NULL",
        [],
        |r| r.get(0),
    )
    .expect("count cited traces")
}

/// Register the model over the real route and return `(id, version, digest)`.
///
/// The digest comes back from the registration handler, which derived it with
/// the shipped `RulesModel` — the test never canonicalizes the table itself,
/// because a hand-rolled second canonicalization is a second answer to a
/// question the engine already answers.
async fn register_via_route(state: &Arc<brain_server::AppState>) -> (String, String, String) {
    let (status, body) = json(
        state,
        "POST",
        "/workflow/model-registry/register",
        format!(r#"{{"kind": "deterministic-rules", "rules_config": {RULES_JSON}}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "registration body: {body}");
    (
        body["id"].as_str().expect("id").to_string(),
        body["version"].as_str().expect("version").to_string(),
        body["config_digest"]
            .as_str()
            .expect("a rules row always carries its config digest")
            .to_string(),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// R57b.3 — a persisted trace cites the model the RESOLVER returned
// ─────────────────────────────────────────────────────────────────────────────

/// **R57b.3.** Driven, not scanned: real router → real resolver → real writer →
/// the row that committed. The citation is compared against the resolved row's
/// own columns, so a writer that re-derived the identity from the request key,
/// or that stamped a digest the resolver never returned, fails here.
#[tokio::test]
async fn a_persisted_trace_cites_the_model_the_resolver_returned() {
    let s = server();
    let (id, version, digest) = register_via_route(&s.state).await;
    {
        let conn = s.state.pool.get().expect("conn");
        set_status(&conn, &id, &version, "promoted");
        let run_id = seed_case(&conn);
        let config = config_json(&digest);
        drop(conn);

        let (status, body) = json(
            &s.state,
            "POST",
            "/workflow/decision-runs",
            run_body(&config, run_id),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "body: {body}");
        let trace_row_id = body["trace_id"].as_i64().expect("trace id in the reply");

        let conn = s.state.pool.get().expect("conn");
        // What the resolver was entitled to return.
        let resolved = registry_identity(&conn, &id, &version);
        // What the writer actually committed.
        let cited = committed_citation(&conn, trace_row_id);
        assert_eq!(
            cited,
            (
                Some(resolved.0.clone()),
                Some(resolved.1.clone()),
                resolved.2.clone()
            ),
            "the trace must cite the resolved (id, version, config_digest) — the row the gate \
             cleared, not the key the request asked for and not a re-read of the table"
        );
        assert!(
            cited.2.is_some(),
            "a promoted rules row always resolved a digest, so a NULL here would mean the writer \
             dropped a value the resolver returned"
        );

        // The join pin: the same three names, so a trace and a registry row
        // meet with no translation layer between the two record types.
        let joined: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decision_run_traces t
                 JOIN decision_model_registry r
                   ON r.id = t.model_registry_id
                  AND r.version = t.model_registry_version
                  AND r.config_digest = t.model_registry_digest
                 WHERE t.id = ?1",
                [trace_row_id],
                |r| r.get(0),
            )
            .expect("join");
        assert_eq!(
            joined, 1,
            "a cited trace must join its registry row on the three columns, by name"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// R57b.4 — the refusals DO NOT MOVE
// ─────────────────────────────────────────────────────────────────────────────

/// **R57b.4.** The citation must not become a way around the gate. Each
/// lifecycle refusal is driven through the real resolver on the real route,
/// answers its own named 400, and — the load-bearing half — leaves the
/// citation-bearing trace count exactly where it was. A writer reached before
/// the refusal would leave a row here, and a test that only checked the status
/// code would call that a pass.
#[tokio::test]
async fn an_unresolved_model_is_refused_and_writes_no_cited_trace() {
    let s = server();
    let (id, version, digest) = register_via_route(&s.state).await;
    let config = config_json(&digest);
    {
        let conn = s.state.pool.get().expect("conn");
        let run_id = seed_case(&conn);
        drop(conn);

        // (1) Registered, never promoted: a deterministic run may not execute
        // a candidate. The row lands as `candidate` and stays there.
        let (status, body) = json(
            &s.state,
            "POST",
            "/workflow/decision-runs",
            run_body(&config, run_id),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
        assert_eq!(body["error"]["code"], "model_not_promoted");

        // (2) Promoted, then retired: the lifecycle moved backwards, and the
        // resolver must notice rather than reuse a stale clearance.
        {
            let conn = s.state.pool.get().expect("conn");
            set_status(&conn, &id, &version, "retired");
        }
        let (status, body) = json(
            &s.state,
            "POST",
            "/workflow/decision-runs",
            run_body(&config, run_id),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
        assert_eq!(body["error"]["code"], "model_retired");

        // (3) Retired AND withdrawn: nothing vouches for the binding at all.
        {
            let conn = s.state.pool.get().expect("conn");
            remove_row(&conn, &id, &version);
        }
        let (status, body) = json(
            &s.state,
            "POST",
            "/workflow/decision-runs",
            run_body(&config, run_id),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
        assert_eq!(body["error"]["code"], "model_not_registered");

        let conn = s.state.pool.get().expect("conn");
        assert_eq!(
            cited_trace_count(&conn),
            0,
            "three refusals, zero citation-bearing trace rows: the writer is downstream of the \
             gate, so a refused model can never leave a trace that cites one"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// R57b.5 — the version-axis coupling
// ─────────────────────────────────────────────────────────────────────────────

/// One patch component above the shipped ceiling, derived rather than written
/// down: a literal here would be a second place the version lives, which is
/// the exact failure this pin exists to catch.
fn one_above(ceiling: &str) -> String {
    let mut parts: Vec<u64> = ceiling
        .split('.')
        .map(|p| p.parse::<u64>().expect("a numeric schema version"))
        .collect();
    let patch = parts.last_mut().expect("a dotted version has a patch");
    *patch += 1;
    parts
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// **R57b.5, green half.** The refuse-newer probe must test `Greater`, not
/// `Equal`: a probe pinned AT the ceiling passes forever while blessing a
/// database this binary cannot migrate.
///
/// The pathology is demonstrated by construction in the red-proof transcript
/// (the probe literal was set equal to the ceiling, `is_newer_than_known`
/// stopped returning `Greater`, and the in-tree assertion failed). What is
/// pinned here is the coupling, and it is pinned in both directions: the real
/// function is asked about a real migrated database's stamp, and the source's
/// probe literal is required to be the ceiling's own successor.
#[test]
fn the_refuse_newer_probe_sits_strictly_above_the_ceiling() {
    use brain_server::storage_layout::{LATEST_KNOWN_SCHEMA, is_newer_than_known, schema_version};

    // The real migration, the real stamp, the real comparator: this binary's
    // own database is never "newer" than what this binary knows.
    // Registration BEFORE the open: the vec0 extension is applied when a
    // connection is created, so a migrated in-memory DB opened first fails
    // with `no such module: vec0` — which the rest of this file's fixtures
    // were silently masking by registering first.
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mut conn = Connection::open_in_memory().expect("open");
    brain_server::migration::run_migration(&mut conn, 0).expect("migration");
    let stamped = schema_version(&conn);
    assert_eq!(
        stamped.as_deref(),
        Some(LATEST_KNOWN_SCHEMA),
        "the migration's stamp and the ceiling are one unit; if these differ, refuse-newer lies \
         in one of two directions and no other gate can see it"
    );
    assert!(
        !is_newer_than_known(stamped.as_deref()),
        "a database this binary just migrated must open, not refuse"
    );
    assert!(
        !is_newer_than_known(Some(LATEST_KNOWN_SCHEMA)),
        "the ceiling is equal, never greater: a stamp at the ceiling is this binary's own"
    );

    // And strictly above it, the refusal must actually fire.
    let above = one_above(LATEST_KNOWN_SCHEMA);
    assert!(
        is_newer_than_known(Some(&above)),
        "a stamp one above the ceiling MUST read as newer, or a newer release's database opens \
         here and is then migrated DOWNWARD"
    );

    // The source literal is the coupling. Scanned as a DERIVED string, so the
    // ceiling moving without the probe moving fails here rather than silently
    // turning the probe into an `Equal` test.
    let layout = std::fs::read_to_string(format!(
        "{}/src/storage_layout.rs",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("read storage_layout.rs");
    let probe = format!("is_newer_than_known(Some(\"{above}\"))");
    assert!(
        layout.contains(&probe),
        "the refuse-newer probe must be pinned at `{above}` — strictly above LATEST_KNOWN_SCHEMA \
         ({LATEST_KNOWN_SCHEMA}). A probe at the ceiling silently tests `Equal` instead of \
         `Greater`, and this assertion is what makes that a failure."
    );
}

/// The citation cannot be omitted. `I57b.3` states the control as "a trace
/// row cannot be written without its citation" — and an `Option` parameter
/// would leave that true only of a convention, not of the code: a future
/// caller could pass `None` and the row would read exactly like one that
/// predates citation tracking.
///
/// So the control is carried by the **declared parameter type**, and this pin
/// reads the declaration rather than a hand-picked list of column names. That
/// distinction is the R57 lesson applied forward: a pin scanning a chosen
/// string list is only ever as good as the strings someone thought of, and R57
/// shipped one that passed with its pathology planted. Here the thing pinned
/// is the type itself.
///
/// Derived, not hand-written: the assertion compares the parsed parameter
/// list of the real signature, so renaming the parameter or reordering it
/// does not silently pass, and widening it back to `Option` fails.
#[test]
fn the_writer_cannot_be_called_without_a_citation() {
    let src = std::fs::read_to_string(format!(
        "{}/src/workflow/harness/trace.rs",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("read trace.rs");

    // The production region: a `#[cfg(test)]` fixture's call site is not a
    // production writer, and `persist_decision_run_trace` is the thin
    // `side: None` forwarder over the same writer.
    let production = src
        .split_once("#[cfg(test)]")
        .map_or(src.as_str(), |(head, _)| head);

    let mut checked = 0usize;
    for fn_name in [
        "fn persist_decision_run_trace_with(",
        "fn persist_decision_run_trace(",
        "fn persist_inner(",
    ] {
        let at = production
            .find(fn_name)
            .unwrap_or_else(|| panic!("{fn_name} is gone — the writer moved"));
        // The parameter list: from the opening paren to its match.
        let after = &production[at + fn_name.len()..];
        let close = after
            .find(')')
            .unwrap_or_else(|| panic!("unterminated parameter list in {fn_name}"));
        let params = &after[..close];
        assert!(
            params.contains("citation: &ModelCitation"),
            "{fn_name} must take `citation: &ModelCitation`. An `Option` here makes \
             `NULL`-on-a-new-row expressible, and a row that cites no model is then \
             indistinguishable from one written before citation tracking existed."
        );
        assert!(
            !params.contains("Option<&ModelCitation>"),
            "{fn_name} re-widened its citation to an Option — the control `I57b.3` claims \
             (a trace row cannot be written without its citation) is a convention again."
        );
        checked += 1;
    }
    assert_eq!(checked, 3, "all three writer signatures must be checked");

    // NOTE — what is deliberately NOT asserted here. An earlier draft of this
    // pin also string-scanned the handler for `registry_id: registry_row.…`,
    // to prove the citation is built from the resolver's row rather than the
    // request key. That check is exactly the R57 anti-pattern this round
    // exists to avoid: a hand-chosen list of source strings is only ever as
    // good as the strings someone thought of, and it re-derives a fact the
    // next `cargo fmt` can invalidate. It was deleted, and the first version
    // of it failed for precisely that reason (a formatting reflow, not a
    // real regression).
    //
    // The same property is proven where it is actually true — by RUNNING the
    // seam: `a_persisted_trace_cites_the_model_the_resolver_returned` drives
    // the real router and resolver, then compares the committed row against
    // the registry row's OWN columns read back from the database. A handler
    // that re-derived the identity from the request key fails there, and no
    // amount of source-matching would catch it if the derivation moved.
}

/// The one writer. The citation has exactly ONE production write seam, and a
/// second one would be a second answer to "which model ran this".
///
/// Scanned over the production region of the writer (a whole-file scan would
/// match this file's own literals and guard nothing), with a floor on the
/// number of files the walk saw so "found one" cannot mean "looked at none".
#[test]
fn the_citation_has_exactly_one_production_write_seam() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["src", "tests"] {
        walk_rs(&root.join(dir), &mut files);
    }
    assert!(
        files.len() >= 50,
        "anti-vacuous: the walk saw only {} files",
        files.len()
    );

    let mut writers: Vec<String> = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        // The production region only: a `#[cfg(test)]` seed is not a writer.
        let production = text
            .split_once("#[cfg(test)]")
            .map_or(text.as_str(), |(head, _)| head);
        for (n, line) in production.lines().enumerate() {
            if line.contains("INSERT INTO decision_run_traces") {
                writers.push(format!("{}:{}", path.display(), n + 1));
            }
        }
    }
    assert_eq!(
        writers.len(),
        1,
        "exactly ONE production INSERT into decision_run_traces may exist. A second one is a \
         second citation a reader must reconcile against the first; found: {writers:?}"
    );
    assert!(
        writers[0].ends_with(":263") || writers[0].contains("harness/trace.rs"),
        "the writer must be the trace writer (the persistence seam), not an engine module: {}",
        writers[0]
    );
}

fn walk_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}
