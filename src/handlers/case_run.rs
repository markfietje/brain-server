//! The operator case-launch boundary: `POST /workflow/cases/{id}/gdl`.
//!
//! The ONE authenticated boundary where an operator's authority maps to
//! case-run capability. Laws enforced HERE and nowhere else:
//! * agents do not self-launch cases — a `PrincipalKind::AgentLoopback`
//!   bearer is refused before anything else (the operator/agent split);
//! * an unknown run refuses before any provider contact (probe-blind 404);
//! * the provider endpoint is screened for SSRF at construction and key
//!   material rides the `secret_file` seam — never source, never logs;
//! * the launch drives the EXISTING `GdlDriver` with the deny-all scoped
//!   env, an empty (fail-closed) tool registry, and pass-through hooks:
//!   every outcome is the existing loop/GDL vocabulary (a pending capture
//!   PROPOSAL, a Handoff to a human, a route/escalation — nothing
//!   publishes automatically).

use axum::Json;
use axum::extract::{Path, State};
use std::sync::Arc;
use std::time::Duration;

use crate::agentloop::provider::{LlmProvider, ProviderError};
use crate::agentloop::provider_http::{HttpProvider, HttpProviderConfig};
use crate::auth::policy::PrincipalKind;
use crate::server::bootstrap::AppState;
use crate::workflow::gdl::{GdlDriver, GdlOutcome};
use crate::workflow::host::SqliteWorkflowHost;

use super::{HandlerError, authorize};

/// Per-launch provider + case configuration. The endpoint/model/secret
/// file are EXTERNALLY configured (operator-supplied per launch) — never
/// environment-read inside the loop.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct GdlLaunchRequest {
    /// The ticket verbatim (A1: the original words are evidence). Bounded:
    /// 1..=8192 bytes.
    pub ticket: String,
    pub base_url: String,
    pub model: String,
    /// Path to an owner-only (0600/0400) file carrying the bearer key.
    /// The value is read at the boundary and never logged or persisted.
    pub secret_file: String,
    pub connect_timeout_ms: Option<u64>,
    pub first_byte_timeout_ms: Option<u64>,
    pub max_response_bytes: Option<usize>,
}

/// Test-only provider injection: `#[cfg(test)]` tests drive the route
/// end-to-end with the real adapter pinned to an in-process SSE server.
/// In production builds the type is uninhabited-by-value (unit) and the
/// extension is never inserted, so the constructor path always runs.
#[cfg(test)]
#[derive(Clone)]
pub(crate) struct TestProvider(pub Arc<dyn LlmProvider>);
#[cfg(not(test))]
#[derive(Clone)]
pub(crate) struct TestProvider;

/// The coarse outcome summary: the existing GDL vocabulary, no content
/// echo (the case state itself is read through the existing run surface).
fn outcome_summary(run_id: i64, outcome: &GdlOutcome) -> serde_json::Value {
    let (kind, at): (&str, Option<String>) = match outcome {
        GdlOutcome::Resolved { .. } => ("resolved", None),
        GdlOutcome::Routed { at, .. } => ("routed", Some(at.as_str().to_string())),
        GdlOutcome::Escalated { at, .. } => ("escalated", Some(at.as_str().to_string())),
        GdlOutcome::VerifyFailed { at, .. } => ("verify_failed", Some(at.as_str().to_string())),
        GdlOutcome::Canceled => ("canceled", None),
        GdlOutcome::Capped { at, .. } => ("capped", Some(at.as_str().to_string())),
    };
    serde_json::json!({
        "run_id": run_id,
        "outcome": { "kind": kind, "at_phase": at },
        "capture": if matches!(outcome, GdlOutcome::Resolved { .. }) {
            "proposals queued pending human approval (or an audited no-proposal row); nothing published"
        } else {
            "none"
        },
    })
}

fn refused_from_provider(e: ProviderError) -> HandlerError {
    match e {
        // Transport-class failures: the provider could not be reached or
        // refused outright. No loopback fallback exists to fall back to.
        ProviderError::Unavailable(m) => HandlerError::unavailable(format!(
            "provider unavailable: {m} — no loopback fallback by law"
        )),
        ProviderError::Refused(m) => {
            HandlerError::unprocessable("provider_refused", format!("provider refused: {m}"))
        }
        ProviderError::Cancelled => {
            HandlerError::unavailable("provider cancelled the stream".to_string())
        }
    }
}

/// `POST /workflow/cases/{id}/gdl` — launch one GDL case episode on a
/// fresh run through the real configured provider.
pub(crate) async fn run_gdl_case(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Path(run_id): Path<i64>,
    test_provider: Option<axum::Extension<TestProvider>>,
    axum::extract::Json(body): axum::extract::Json<GdlLaunchRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;

    // The operator/agent split, mapped at the boundary: agents do not
    // self-launch cases (before ANY other work — no probe oracle either).
    if let Some(p) = &principal
        && p.kind == PrincipalKind::AgentLoopback
    {
        return Err(HandlerError::forbidden(
            crate::auth::Action::Write,
            &p.tenant,
            "global",
        ));
    }

    // Unknown run refuses BEFORE any provider contact (probe-blind 404,
    // the get_run pattern). The freshness/revision columns ride the same
    // read so the launch law is one DB round-trip.
    let pool = state.pool.clone();
    let row = tokio::task::spawn_blocking(
        move || -> Result<Option<crate::workflow::state::LaunchRowTuple>, String> {
            let conn = pool.get().map_err(|e| format!("{e}"))?;
            crate::workflow::state::launch_row(&conn, run_id).map_err(|e| format!("{e}"))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?;
    let Some((domain, kind, status, state_json, revision)) = row else {
        return Err(HandlerError::not_found("workflow run not found"));
    };

    // The run's own domain authorization (operator-tier Write).
    authorize(&principal, crate::auth::Action::Write, "", &domain)?;

    // The launch law: a FRESH troubleshoot run only (the checkpoint admits
    // nothing else — pre-checked here for a cheap, honest refusal).
    if kind != "troubleshoot" {
        return Err(HandlerError::unprocessable(
            "run_kind_invalid",
            "GDL cases launch on kind='troubleshoot' runs only",
        ));
    }
    if status != "active" || revision != 0 || state_json.trim() != "{}" {
        return Err(HandlerError::conflict_with(
            "run_not_fresh",
            "GDL cases launch on fresh runs only (state '{}', revision 0, active)",
            serde_json::json!({}),
        ));
    }

    // Ticket bounds (the words ride the checkpoint and every prompt).
    let ticket = body.ticket.trim().to_string();
    if ticket.is_empty() || ticket.len() > 8192 {
        return Err(HandlerError::bad_request(
            "ticket_invalid",
            "ticket must be 1..=8192 bytes after trimming",
        ));
    }
    if body.model.trim().is_empty() || body.base_url.trim().is_empty() {
        return Err(HandlerError::bad_request(
            "provider_config_invalid",
            "base_url and model are required",
        ));
    }

    // Provider construction (secret-file read + SSRF screen + DNS pin +
    // bounds) happens HERE, at the boundary — inside the loop nothing
    // reads configuration. The test-only extension carries an already-
    // constructed adapter (pinned to the in-process SSE server); in test
    // builds the secret-file read is then skipped BY THE INJECTION, never
    // by the production path.
    #[cfg(test)]
    let provider: Arc<dyn LlmProvider> = if let Some(axum::Extension(injected)) = test_provider {
        injected.0
    } else {
        build_provider_from_request(&body).await?
    };
    #[cfg(not(test))]
    let provider: Arc<dyn LlmProvider> = {
        let _ = test_provider; // the injection seam is test-only
        build_provider_from_request(&body).await?
    };

    // Drive the EXISTING machine: deny-all scoped env, an empty (fail-
    // closed) tool registry — any tool call the model attempts is the
    // existing unknown-tool refusal — and pass-through hooks (hooks stay
    // constructor-injected policy).
    let host = Arc::new(SqliteWorkflowHost::new(state.pool.clone()));
    let driver = GdlDriver::new(
        state.pool.clone(),
        host,
        provider,
        vec![],
        brain_engine_sdk::env::ExecutionEnv::default(),
        crate::agentloop::run_loop::LoopConfig::default(),
    );
    let outcome = driver
        .run_case(run_id, &ticket, &tokio_util::sync::CancellationToken::new())
        .await
        .map_err(|e| match e {
            crate::agentloop::run_loop::LoopError::Provider(p) => refused_from_provider(p),
            other => HandlerError::unavailable(format!("case episode refused: {other}")),
        })?;
    Ok(Json(outcome_summary(run_id, &outcome)))
}

/// The secret-file seam: owner-only permissions are enforced BEFORE
/// reading; the value lives in this function's memory only (never logs,
/// never persists).
async fn read_secret_header(body: &GdlLaunchRequest) -> Result<String, HandlerError> {
    let path = std::path::PathBuf::from(&body.secret_file);
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        crate::secret_file::check_secret_permissions(&path)?;
        std::fs::read_to_string(&path)
            .map(|s| format!("Bearer {}", s.trim()))
            .map_err(|e| format!("secret file unreadable: {e}"))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(|e| HandlerError::bad_request("secret_file_invalid", e))
}

async fn build_provider_from_request(
    body: &GdlLaunchRequest,
) -> Result<Arc<dyn LlmProvider>, HandlerError> {
    let auth_header = read_secret_header(body).await?;
    build_provider(body, auth_header)
}

fn build_provider(
    body: &GdlLaunchRequest,
    auth_header: String,
) -> Result<Arc<dyn LlmProvider>, HandlerError> {
    let cfg = HttpProviderConfig {
        base_url: body.base_url.trim().to_string(),
        model: body.model.trim().to_string(),
        auth_header,
        connect_timeout: Duration::from_millis(body.connect_timeout_ms.unwrap_or(5_000)),
        first_byte_timeout: Duration::from_millis(body.first_byte_timeout_ms.unwrap_or(30_000)),
        max_response_bytes: body
            .max_response_bytes
            .unwrap_or(crate::agentloop::provider_http::DEFAULT_MAX_RESPONSE_BYTES),
    };
    let provider: Arc<dyn LlmProvider> = HttpProvider::new(cfg).map_err(refused_from_provider)?;
    Ok(provider)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::agentloop::provider_http::HttpProviderConfig;
    use crate::agentloop::provider_http::scripted_server::{spawn_turns, text_turn, tool_use_turn};
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;

    const OP_TOKEN: &str = "r10-op-token";

    const INTAKE_JSON: &str = r#"{"is_not":{"what":{"is":"PERC H740P write-cache write-through","is_not":"read cache"},"where":{"is":"node-042 RAID-10 VDs","is_not":"node-041"},"when":{"is":"since 03:12 during rebuild","is_not":"before 03:12"},"extent":{"is":"VD 5 only","is_not":"all VDs"}},"telemetry_refs":["tsr://node-042","sel://events"],"what_changed":"fw 2.10 flashed last week","known_good":"node-041 same fw"}"#;
    const TRIAGE_JSON: &str = r#"{"priority":"P3","stabilized":false,"search_hits":["P-STORAGE-0104"],"verdict":"accept"}"#;
    const HYPOTHESIZE_JSON: &str = r#"{"hypotheses":[{"statement":"PERC battery dead","prediction":"racadm battery state reports Failed","sources":["actual:SEL event 0x42","test:racadm get storageservices.battery"],"confidence":0.7}]}"#;
    const PLAN_JSON: &str = r#"{"steps":[{"order":1,"kind":"check","skill_gate":"L1","description":"query battery state","command":"racadm get storageservices.battery","expected":"Ready","fail_action":2,"invasiveness":0,"justification":null},{"order":2,"kind":"action","skill_gate":"L2","description":"replace battery ring 3","command":"hw replace battery","expected":"battery Ready","fail_action":null,"invasiveness":2,"justification":null}],"verify_step":{"re_run":"rebuild rate on VD 5 under the customer load","pass_condition":">10%/h"},"dead_end":{"escalate_to":"eng-storage","required_evidence":["TSR","test log"]}}"#;
    const ACT_JSON: &str = r#"{"rows":[{"order":1,"kind":"check","description":"query battery state","playbook_ref":"P-STORAGE-0104","variables":["battery state"],"expected":"Ready","actual":"Failed","verdict":"fail","evidence_ref":"TSR p.12","dtfvc":{"diagnose":"battery fault hypothesis","test":"racadm query","fix":null,"verify":"battery state readback matches Failed","capture":null},"invasiveness":0,"justification":null},{"order":2,"kind":"action","description":"replace battery ring 3","playbook_ref":"P-STORAGE-0104","variables":["battery"],"expected":"battery Ready","actual":"Ready","verdict":"pass","evidence_ref":"TSR p.13","dtfvc":{"diagnose":"battery fault confirmed by row 1","test":"racadm query post-replace","fix":"replaced battery ring 3","verify":"rebuild rate 14%/h","capture":"battery replacement row"},"invasiveness":2,"justification":null}],"complete":true}"#;
    const VERIFY_PASS_JSON: &str = r#"{"re_run":"rebuild rate on VD 5 under the customer load","pass":true,"stability_window_min":15,"negative_check":true}"#;
    const VERIFY_FAIL_JSON: &str = r#"{"re_run":"rebuild rate on VD 5 under the customer load","pass":false,"stability_window_min":15,"negative_check":true}"#;
    const HANDOFF_JSON: &str = r#"{"capture":{"resolution":"write-through during rebuild -> dead PERC battery -> replaced ring 3 -> verified 14%/h","bundle_hash":"h0"}}"#;

    fn happy_script() -> Vec<Vec<String>> {
        vec![
            text_turn(INTAKE_JSON),
            text_turn(TRIAGE_JSON),
            text_turn(HYPOTHESIZE_JSON),
            text_turn(PLAN_JSON),
            text_turn(ACT_JSON),
            text_turn(VERIFY_PASS_JSON),
            text_turn(HANDOFF_JSON),
        ]
    }

    /// Opaque-mode state (the twokeys fixture shape): the operator bearer
    /// is the None-superuser posture; the agent line exists but is unused
    /// here (agent refusal is pinned in the authz matrix).
    pub(crate) struct Fixture {
        pub(crate) _dir: tempfile::TempDir,
        pub(crate) state: Arc<AppState>,
    }

    pub(crate) fn fixture() -> Fixture {
        crate::register_sqlite_vec::register_sqlite_vec();
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("brain.db");
        let mgr = crate::pool::SqliteConnectionManager::file(&db_path);
        let pool: crate::Pool = r2d2::Pool::builder().max_size(4).build(mgr).unwrap();
        crate::migration::run_migration(&mut pool.get().unwrap(), 0).unwrap();
        let tok_file = dir.path().join("tokens");
        std::fs::write(&tok_file, format!("{OP_TOKEN}\nagent-line-unused\n")).unwrap();
        let token_store = crate::auth::TokenStore::from_file(Some(tok_file.clone()));
        // from_file seeds from the ENV; the explicit parts seam is the
        // test-honest load (the twokeys fixture's pattern).
        token_store.reload_parts_from(vec![OP_TOKEN.to_string()], None);
        let model: Arc<dyn crate::embed::Embedder> =
            Arc::new(crate::embed::StaticEmbedder::new(crate::config::MODEL_ID).expect("model"));
        let state = Arc::new(AppState {
            token_store,
            jwt_middleware_state: Arc::new(
                crate::server::router::auth::JwtMiddlewareState::opaque_for_tests(
                    pool.clone(),
                    db_path.clone(),
                ),
            ),
            cors: tower_http::cors::CorsLayer::new(),
            durability: Default::default(),
            loom: Default::default(),
            model,
            registry: crate::domain_registry::DomainRegistry::new(pool.clone(), &db_path, false),
            pool,
            db_path,
            connection_tracker: Arc::new(crate::http_limit::ConnectionTracker::new()),
            rate_limiter: Arc::new(crate::http_limit::RateLimiter::new()),
            snapshot: crate::integrity::SnapshotState::default(),
            audit_chain_cache: Arc::new(std::sync::Mutex::new(None)),
            auth_mode: crate::auth::AuthMode::Opaque,
            key_store: crate::auth::jwks::KeyStore::default(),
            revocation_cache: Arc::new(crate::auth::revocation::RevocationCache::new()),
            jwt_issuer: String::new(),
            jwt_audience: String::new(),
            oidc_config: crate::handlers::well_known::OidcConfig::unconfigured(),
            ump_events: tokio::sync::broadcast::channel(16).0,
            alert_events: tokio::sync::broadcast::channel(16).0,
            alert_seq: std::sync::atomic::AtomicU64::new(0),
            chain_watch: crate::alert::ChainWatchState::default(),
            concurrency: &crate::concurrency::CONCURRENCY,
        });
        Fixture { _dir: dir, state }
    }

    /// A fresh troubleshoot run + a resolved-repeater cluster in the
    /// domain (>=3/30d: the RCA capture proposal warrants on resolve).
    pub(crate) fn seed_run(state: &AppState, domain: &str) -> i64 {
        use crate::workflow::state::test_support::{
            insert_fresh_troubleshoot_run, insert_resolved_troubleshoot_run,
        };
        let conn = state.pool.get().unwrap();
        let now = chrono::Utc::now().timestamp();
        for _ in 0..3 {
            insert_resolved_troubleshoot_run(&conn, domain, now - 86400).unwrap();
        }
        insert_fresh_troubleshoot_run(&conn, domain, now).unwrap()
    }

    pub(crate) fn launch_body(ticket: &str, base_url: &str) -> String {
        format!(
            r#"{{"ticket":"{ticket}","base_url":"{base_url}","model":"pilot-model","secret_file":"/nonexistent-secret"}}"#
        )
    }

    /// POST the launch through the REAL composed app with the real
    /// (unscreened-pinned) adapter riding the test-only extension.
    async fn post_launch(
        state: &Arc<AppState>,
        run_id: i64,
        body: String,
        provider: Option<Arc<dyn LlmProvider>>,
    ) -> axum::response::Response {
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("/workflow/cases/{run_id}/gdl"))
            .header("authorization", format!("Bearer {OP_TOKEN}"))
            .header("content-type", "application/json");
        if let Some(p) = provider {
            builder = builder.extension(TestProvider(p));
        }
        crate::server::router::app(state.clone())
            .oneshot(builder.body(Body::from(body)).unwrap())
            .await
            .expect("oneshot")
    }

    async fn body_bytes(
        res: axum::response::Response,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        let v = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, v)
    }

    pub(crate) fn pinned_adapter(addr: std::net::SocketAddr) -> Arc<dyn LlmProvider> {
        let cfg = HttpProviderConfig {
            base_url: format!("http://{addr}/v1/stream"),
            model: "pilot-model".to_string(),
            auth_header: "Bearer test-key".to_string(),
            connect_timeout: std::time::Duration::from_secs(2),
            first_byte_timeout: std::time::Duration::from_secs(10),
            max_response_bytes: crate::agentloop::provider_http::DEFAULT_MAX_RESPONSE_BYTES,
        };
        HttpProvider::new_unscreened(cfg, addr) as Arc<dyn LlmProvider>
    }

    /// The full pilot path through the REAL route: a configured case
    /// reaches Resolved; the capture lands as PENDING proposals (RCA on
    /// the repeater cluster) on the existing HITL surface; knowledge
    /// stays byte-unchanged (no automatic publication); the audit chain
    /// stays green.
    #[tokio::test]
    async fn route_resolves_case_capture_lands_pending_only() {
        let f = fixture();
        let run_id = seed_run(&f.state, "acme");
        let addr = spawn_turns(happy_script()).await;
        let provider = pinned_adapter(addr);
        let (status, v) = body_bytes(
            post_launch(
                &f.state,
                run_id,
                launch_body(
                    "perc battery dead on node-042",
                    &format!("http://{addr}/v1/stream"),
                ),
                Some(provider),
            )
            .await,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "body: {v}");
        assert_eq!(v["outcome"]["kind"], "resolved", "body: {v}");
        assert_eq!(v["run_id"], run_id);

        use crate::workflow::state::test_support::{
            knowledge_and_vec_counts, pending_capture_proposals,
        };
        let conn = f.state.pool.get().unwrap();
        // The capture is a QUEUE: pending rows where a human decides.
        let pending = pending_capture_proposals(&conn).unwrap();
        assert!(
            pending
                .iter()
                .any(|(k, s)| k == "gdl_rca" && s == "pending"),
            "the repeater RCA proposal must queue pending, got {pending:?}"
        );
        assert!(
            pending.iter().all(|(_, s)| s == "pending"),
            "nothing leaves the queue without a human: {pending:?}"
        );
        // No-auto-publication, behaviorally.
        assert_eq!(
            knowledge_and_vec_counts(&conn).unwrap(),
            (0, 0),
            "no knowledge row was published"
        );
        // The chain carries the episode (denials AND the no-auto-publish row).
        assert!(crate::audit::verify_chain(&conn), "audit chain green");
    }

    /// Verify failure is a HUMAN handoff (the escalation bundle), never a
    /// silent retry and never a publication.
    #[tokio::test]
    async fn route_verify_failure_hands_off_to_a_human() {
        let f = fixture();
        let run_id = seed_run(&f.state, "acme");
        let mut script = vec![
            text_turn(INTAKE_JSON),
            text_turn(TRIAGE_JSON),
            text_turn(HYPOTHESIZE_JSON),
            text_turn(PLAN_JSON),
            text_turn(ACT_JSON),
            text_turn(VERIFY_FAIL_JSON),
        ];
        script.truncate(6);
        let addr = spawn_turns(script).await;
        let provider = pinned_adapter(addr);
        let (status, v) = body_bytes(
            post_launch(
                &f.state,
                run_id,
                launch_body("t", &format!("http://{addr}/v1/stream")),
                Some(provider),
            )
            .await,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "body: {v}");
        assert_eq!(v["outcome"]["kind"], "verify_failed", "body: {v}");
        assert_eq!(v["outcome"]["at_phase"], "verify", "body: {v}");
        use crate::workflow::state::test_support::knowledge_and_vec_counts;
        let conn = f.state.pool.get().unwrap();
        assert_eq!(
            knowledge_and_vec_counts(&conn).unwrap().0,
            0,
            "a failed verify publishes nothing"
        );
    }

    /// The scoped tool registry is EMPTY (fail-closed): a model tool call
    /// meets the existing unknown-tool refusal receipt, end-to-end through
    /// the route.
    #[tokio::test]
    async fn route_unknown_tool_fails_closed() {
        let f = fixture();
        let run_id = seed_run(&f.state, "acme");
        let script = vec![
            tool_use_turn("c1", "definitely_not_registered"),
            // After the fail-closed refusal the exchange continues; the
            // model answers with the intake artifact and the case
            // proceeds normally.
            text_turn(INTAKE_JSON),
            text_turn(TRIAGE_JSON),
            text_turn(HYPOTHESIZE_JSON),
            text_turn(PLAN_JSON),
            text_turn(ACT_JSON),
            text_turn(VERIFY_PASS_JSON),
            text_turn(HANDOFF_JSON),
        ];
        let addr = spawn_turns(script).await;
        let provider = pinned_adapter(addr);
        let (status, v) = body_bytes(
            post_launch(
                &f.state,
                run_id,
                launch_body("t", &format!("http://{addr}/v1/stream")),
                Some(provider),
            )
            .await,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "body: {v}");
        use crate::workflow::state::test_support::unknown_tool_receipts;
        let conn = f.state.pool.get().unwrap();
        let refused = unknown_tool_receipts(&conn, run_id).unwrap();
        assert!(refused >= 1, "the unknown-tool refusal receipt must exist");
    }

    /// Provider unreachable through the REAL route: the named Unavailable
    /// refusal (503) — never a silent loopback fallback.
    #[tokio::test]
    async fn route_provider_unreachable_is_named_refusal() {
        let f = fixture();
        let run_id = seed_run(&f.state, "acme");
        // A guaranteed-closed loopback port (adapter pinned unscreened —
        // the screen itself refuses loopback by design).
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead = listener.local_addr().unwrap();
        drop(listener);
        let provider = pinned_adapter(dead);
        let (status, v) = body_bytes(
            post_launch(
                &f.state,
                run_id,
                launch_body("t", "https://provider.invalid/v1/stream"),
                Some(provider),
            )
            .await,
        )
        .await;
        assert_eq!(
            status,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "body: {v}"
        );
        let text = v.to_string();
        assert!(
            text.contains("provider unavailable") && text.contains("no loopback fallback"),
            "the refusal must be named: {v}"
        );
    }

    /// Absent provider config (empty base_url) refuses BEFORE any
    /// provider contact.
    #[tokio::test]
    async fn route_absent_provider_config_is_400() {
        let f = fixture();
        let run_id = seed_run(&f.state, "acme");
        let (status, v) =
            body_bytes(post_launch(&f.state, run_id, launch_body("t", ""), None).await).await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "body: {v}");
        assert_eq!(v["error"]["code"], "provider_config_invalid", "body: {v}");
    }
}

#[cfg(test)]
mod conformance {
    //! The pilot conformance runner (C6c): executes the PRIVATE gold-case
    //! pack (read by path at test time — `GDL_R10_PACK_DIR`, defaulting to
    //! the private-repo sibling; never embedded, never a runtime knob)
    //! through the REAL route with the C3 scripted provider, records
    //! scrubbed per-case outcomes as JSONL, and pins their SHA-256 hashes
    //! for the evidence. Socket-bound: named in the sanitized-gate skip
    //! list; runs in the focused lane.
    //!
    //! κ honesty: this runner produces OUTCOMES, not agreement scores. A κ
    //! for this pack exists only after a real human labeling round against
    //! these retained outputs (the dataset-readiness gate — see the
    //! evidence runbook).

    use super::tests::{Fixture, fixture, launch_body};
    use crate::agentloop::provider_http::HttpProviderConfig;
    use crate::agentloop::provider_http::scripted_server::{spawn_turns, text_turn};
    use axum::body::Body;
    use axum::http::Request;
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;

    /// Resolve the pack directory (test-time input, never a runtime knob).
    fn pack_dir() -> std::path::PathBuf {
        match std::env::var("GDL_R10_PACK_DIR") {
            Ok(d) if !d.trim().is_empty() => std::path::PathBuf::from(d),
            _ => std::path::PathBuf::from("../brain-steward-ip/conformance"),
        }
    }

    #[derive(serde::Deserialize)]
    struct PackCase {
        case_id: String,
        #[allow(dead_code)]
        ticket: String,
        turns: Vec<String>,
        expected: Expected,
        #[allow(dead_code)]
        note: String,
    }

    #[derive(serde::Deserialize)]
    struct Expected {
        outcome_kind: String,
        at_phase: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct AmbiguityRow {
        case_id: String,
    }

    /// Execute the pack end-to-end and pin the hashes. Every non-
    /// ambiguous case must land its expected outcome; ambiguous cases are
    /// reported `excluded_ambiguous`, never scored.
    #[tokio::test]
    async fn gdl_conformance_pack_run() {
        let dir = pack_dir();
        let pack_path = dir.join("gdl_gold_pack_v1.jsonl");
        let register_path = dir.join("gdl_ambiguity_register_v1.jsonl");
        let pack_raw = std::fs::read_to_string(&pack_path).unwrap_or_else(|e| {
            panic!(
                "gold pack unreadable at {}: {e} (set GDL_R10_PACK_DIR)",
                pack_path.display()
            )
        });
        let register_raw = std::fs::read_to_string(&register_path).unwrap_or_else(|e| {
            panic!(
                "ambiguity register unreadable at {}: {e}",
                register_path.display()
            )
        });
        let cases: Vec<PackCase> = pack_raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("pack line is a valid case"))
            .collect();
        assert!(!cases.is_empty(), "an empty pack is not a run");
        let ambiguous: std::collections::HashSet<String> = register_raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                serde_json::from_str::<AmbiguityRow>(l)
                    .expect("register line")
                    .case_id
            })
            .collect();

        let mut out = String::new();
        let mut scored = 0usize;
        let mut excluded = 0usize;
        for case in &cases {
            if ambiguous.contains(&case.case_id) {
                out.push_str(&format!(
                    "{}\n",
                    serde_json::json!({"case_id": case.case_id, "verdict": "excluded_ambiguous"})
                ));
                excluded += 1;
                continue;
            }
            let Fixture { _dir, state } = fixture();
            use crate::workflow::state::test_support::insert_fresh_troubleshoot_run;
            let conn = state.pool.get().unwrap();
            let now = chrono::Utc::now().timestamp();
            let run_id = insert_fresh_troubleshoot_run(&conn, "acme", now).expect("seed fresh run");
            drop(conn);
            let addr = spawn_turns(case.turns.iter().map(|t| text_turn(t)).collect()).await;
            let provider: std::sync::Arc<dyn crate::agentloop::provider::LlmProvider> = {
                let cfg = HttpProviderConfig {
                    base_url: format!("http://{addr}/v1/stream"),
                    model: "pilot-model".to_string(),
                    auth_header: "Bearer test-key".to_string(),
                    connect_timeout: std::time::Duration::from_secs(2),
                    first_byte_timeout: std::time::Duration::from_secs(10),
                    max_response_bytes: crate::agentloop::provider_http::DEFAULT_MAX_RESPONSE_BYTES,
                };
                crate::agentloop::provider_http::HttpProvider::new_unscreened(cfg, addr)
            };
            let req = Request::builder()
                .method("POST")
                .uri(format!("/workflow/cases/{run_id}/gdl"))
                .header("authorization", "Bearer r10-op-token")
                .header("content-type", "application/json")
                .extension(super::TestProvider(provider))
                .body(Body::from(launch_body(
                    &case.ticket,
                    &format!("http://{addr}/v1/stream"),
                )))
                .unwrap();
            let res = crate::server::router::app(state.clone())
                .oneshot(req)
                .await
                .expect("oneshot");
            let status = res.status();
            let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
                .await
                .unwrap();
            let v: serde_json::Value =
                serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            let kind = v["outcome"]["kind"]
                .as_str()
                .unwrap_or("<none>")
                .to_string();
            let at = v["outcome"]["at_phase"].as_str().map(str::to_string);
            let pass = status == axum::http::StatusCode::OK
                && kind == case.expected.outcome_kind
                && at == case.expected.at_phase;
            out.push_str(&format!(
                "{}\n",
                serde_json::json!({
                    "case_id": case.case_id,
                    "verdict": if pass { "pass" } else { "fail" },
                    "expected": {"outcome_kind": case.expected.outcome_kind, "at_phase": case.expected.at_phase},
                    "actual": {"outcome_kind": kind, "at_phase": at, "http": status.as_u16()},
                })
            ));
            scored += 1;
            assert!(
                pass,
                "conformance case {} failed: expected {:?}/{:?} got {:?}/{:?} (http {})",
                case.case_id, case.expected.outcome_kind, case.expected.at_phase, kind, at, status
            );
        }
        assert_eq!(
            scored + excluded,
            cases.len(),
            "every pack case is reported"
        );

        // Persist the scrubbed outputs and pin the hashes for the evidence.
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S");
        let out_path = std::env::temp_dir().join(format!("r10_gdl_conformance_{stamp}.jsonl"));
        std::fs::write(&out_path, &out).expect("write conformance outputs");
        let pack_hash = hex(&Sha256::digest(pack_raw.as_bytes()));
        let out_hash = hex(&Sha256::digest(out.as_bytes()));
        println!("conformance outputs: {}", out_path.display());
        println!("sha256(gdl_gold_pack_v1.jsonl) = {pack_hash}");
        println!("sha256(outputs)                = {out_hash}");
        println!("cases: {} scored, {} excluded_ambiguous", scored, excluded);
    }

    fn hex(d: &[u8]) -> String {
        d.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// C7 — the live configured case. Runs ONLY with operator-provided
    /// configuration in the environment (endpoint + model + secret file +
    /// ticket); `#[ignore]` keeps it out of every lane. The endpoint is
    /// SSRF-screened by the production constructor — a private endpoint is
    /// refused here exactly as in production. This test is the runbook:
    ///   R10_LIVE_BASE_URL=https://<provider>/v1/stream \
    ///   R10_LIVE_MODEL=<model-id> \
    ///   R10_LIVE_SECRET_FILE=/path/to/0600-key \
    ///   cargo test --offline --locked --lib --features bench \
    ///     live_configured_case_real_provider -- --ignored --nocapture
    /// The evidence records model id, endpoint class, date, and the
    /// scrubbed trace hash from the printed line. Absent credentials,
    /// this stays the named dataset-readiness gate — never simulated.
    #[tokio::test]
    #[ignore = "live configured case: runs only with operator-provided R10_LIVE_* configuration (C7 runbook)"]
    async fn live_configured_case_real_provider() {
        let base_url = std::env::var("R10_LIVE_BASE_URL")
            .expect("R10_LIVE_BASE_URL must name the operator's provider endpoint");
        let model = std::env::var("R10_LIVE_MODEL")
            .expect("R10_LIVE_MODEL must name the operator's model id");
        let secret_file = std::env::var("R10_LIVE_SECRET_FILE")
            .expect("R10_LIVE_SECRET_FILE must name an owner-only key file");
        let ticket =
            std::env::var("R10_LIVE_TICKET").unwrap_or_else(|_| "live configured case".into());

        let f = fixture();
        use crate::workflow::state::test_support::insert_fresh_troubleshoot_run;
        let conn = f.state.pool.get().unwrap();
        let now = chrono::Utc::now().timestamp();
        let run_id = insert_fresh_troubleshoot_run(&conn, "acme", now).expect("seed fresh run");
        drop(conn);

        // The REAL constructor: SSRF screen + DNS pin + bounds. No
        // injection — this is the production path.
        let cfg = HttpProviderConfig {
            base_url: base_url.clone(),
            model: model.clone(),
            auth_header: {
                let path = std::path::PathBuf::from(&secret_file);
                crate::secret_file::check_secret_permissions(&path).expect("key file permissions");
                format!(
                    "Bearer {}",
                    std::fs::read_to_string(&path).expect("key file").trim()
                )
            },
            connect_timeout: std::time::Duration::from_secs(5),
            first_byte_timeout: std::time::Duration::from_secs(60),
            max_response_bytes: crate::agentloop::provider_http::DEFAULT_MAX_RESPONSE_BYTES,
        };
        let _screen_check: std::sync::Arc<dyn crate::agentloop::provider::LlmProvider> =
            match crate::agentloop::provider_http::HttpProvider::new(cfg) {
                Ok(p) => p,
                Err(e) => panic!("live endpoint refused at construction (screen): {e}"),
            };

        let req = Request::builder()
            .method("POST")
            .uri(format!("/workflow/cases/{run_id}/gdl"))
            .header("authorization", "Bearer r10-op-token")
            .header("content-type", "application/json")
            .body(Body::from(launch_body(&ticket, &base_url)))
            .unwrap();
        let res = crate::server::router::app(f.state.clone())
            .oneshot(req)
            .await
            .expect("oneshot");
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        let trace_hash = hex(&Sha256::digest(&bytes));
        println!(
            "LIVE CASE: model={model} endpoint_class=operator-provided date={} http={status} trace_sha256={trace_hash}",
            chrono::Utc::now().format("%Y-%m-%d")
        );
        println!("LIVE BODY: {}", String::from_utf8_lossy(&bytes));
        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "the live case must reach an existing GDL outcome through the real route"
        );
    }
}
