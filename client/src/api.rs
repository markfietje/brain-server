//! Typed brain-server API client. Wraps `reqwest` with the bearer attached.
//! Works unchanged on web (WASM) + desktop + mobile — Dioxus abstracts fetch.
//!
//! The client holds NO memory cache: every call hits the backend (the source of
//! truth) and panels drive re-fetch via `use_resource` signal subscription.
//!
//! Wire types mirror `openapi.yaml` (the backend's test-guarded contract).
//! Unknown fields are ignored by serde; `#[serde(rename_all)]` matches the
//! backend's `lowercase`/`snake_case` JSON keys.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

pub(crate) const MIN_QUERY: usize = 5; // mirrors brain-server's min_query_length
/// v1.16.5 M3: refresh within this many seconds of `exp` to avoid the
/// 401→refresh→retry latency on every expiry boundary (M5 pre-emptive).
const REFRESH_AHEAD_SECS: i64 = 60;

/// v1.16.5 M1: the mutable access+refresh token pair. Shared behind
/// `Arc<RwLock>` across every `ApiClient` clone so a refresh-on-401 inside one
/// panel's request propagates to all panels + the root `Signal<ApiClient>`
/// (no per-panel rewiring). `refresh` is `None` in opaque-token mode.
#[derive(Debug, Default)]
struct TokenState {
    access: Option<String>,
    refresh: Option<String>,
}
/// v1.16.7 M7.2: the shared refresh single-flight guard. Held across a clone
/// boundary (inside the `Arc`) so two concurrent panel requests awaiting the
/// mutex can't both present the SAME refresh token — the loser would be burned
/// by brain-server's `refresh_reuse_detected` (v1.12.2) and logged out. The
/// winner rotates the pair; the waiter re-reads the NEW token and does a legit
/// rotation (no reuse). Serializing the read+POST is the whole fix.
struct SharedTokens {
    state: RwLock<TokenState>,
    refresh_lock: tokio::sync::Mutex<()>,
}
#[derive(Clone)]
pub struct ApiClient {
    base: String,
    tokens: Arc<SharedTokens>,
    /// v1.16.0 M2.1: the authenticated principal (identity pillar). `None` in
    /// loopback mode (no identity to claim); `Some(sub)` in remote/JWT mode.
    principal: Option<String>,
    http: reqwest::Client,
}
#[derive(Debug)]
pub enum ApiError {
    // ponytail: payloads are carried now so the error UI can render
    // them (status + server body); today panels only Debug-print the error.
    #[allow(dead_code)]
    Network(reqwest::Error),
    #[allow(dead_code)]
    Status(u16, String),
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Network(e) => write!(f, "network: {e}"),
            ApiError::Status(code, body) => write!(f, "http {code}: {body}"),
        }
    }
}
impl std::error::Error for ApiError {}
/// v1.20.8 M3: an alert feed signal (the `/events` SSE `data:` line). The
/// server's envelope is exactly `{kind, ts, seq, payload}`; the client parses
/// only `kind` + `seq` — **content/PII never leaves the wire** (the console
/// re-fetches detail from existing endpoints on demand).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertEvent {
    pub kind: String,
    pub seq: u64,
}
/// Parse one SSE `data:` JSON line into an `AlertEvent`. `None` on a malformed
/// line, a missing `kind`, or a missing `seq` — a dropped signal is safe
/// (the monotonic `seq` guard + polling re-sync cover any loss).
pub fn parse_alert_event(data: &str) -> Option<AlertEvent> {
    let v: serde_json::Value = serde_json::from_str(data).ok()?;
    Some(AlertEvent {
        kind: v.get("kind")?.as_str()?.to_string(),
        seq: v.get("seq")?.as_u64()?,
    })
}
/// v1.16.2 "Harden" M4.2: operator-facing error message from an `ApiError`.
/// Maps HTTP status codes to actionable hints. Never includes the bearer token
/// (it's not in the error payload).
/// v1.16.5 M6: revocation/reuse/expiry get specific, actionable messages
/// (DESIGN §3.2 — never a generic "something went wrong").
pub fn error_message(e: &ApiError) -> String {
    match e {
        ApiError::Network(_) => "cannot reach brain-server — check the URL or network".into(),
        ApiError::Status(401, body) if body.contains("refresh_reuse_detected") => {
            "session revoked (refresh token reuse detected) — reconnect".into()
        }
        ApiError::Status(401, _) => {
            "authentication failed — session may have expired; reconnect".into()
        }
        ApiError::Status(403, body) if body.contains("refresh_reuse_detected") => {
            "session revoked (refresh token reuse detected) — reconnect".into()
        }
        ApiError::Status(403, _) => {
            "permission denied — your token lacks the required scope".into()
        }
        ApiError::Status(404, body) => format!("not found — {body}"),
        ApiError::Status(429, _) => "rate limited — wait a moment and retry".into(),
        ApiError::Status(503, _) => "brain-server is unhealthy — check /health".into(),
        ApiError::Status(code, body) => format!("error {code}: {body}"),
    }
}
impl ApiClient {
    pub fn new(base: impl Into<String>, token: Option<String>) -> Self {
        Self::with_principal(base, token, None)
    }
    /// v1.16.7 M7.1: the pre-connect / signed-out state (empty base, no token,
    /// no principal). The logout flow sets this back into the shared signal so
    /// every panel stops using the old identity.
    pub fn unconfigured() -> Self {
        Self::new("", None)
    }
    /// v1.16.0 M2.1: connect with a known principal (remote/JWT mode). A
    /// v1.16.5 M2 refinement: when `principal` is `None` but the access token
    /// is a JWT, the principal is derived from its `sub` claim (the identity
    /// pillar — the operator sees who they are acting as). Opaque tokens stay
    /// `None` (loopback — no identity to claim).
    pub fn with_principal(
        base: impl Into<String>,
        token: Option<String>,
        principal: Option<String>,
    ) -> Self {
        let principal = principal.or_else(|| derive_principal(token.as_deref()));
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            tokens: Arc::new(SharedTokens {
                state: RwLock::new(TokenState {
                    access: token,
                    refresh: None,
                }),
                refresh_lock: tokio::sync::Mutex::new(()),
            }),
            principal,
            http: reqwest::Client::new(),
        }
    }
    /// v1.16.5 M1.4: connect in JWT-pair mode (access + refresh). Enables silent
    /// refresh-on-401 + pre-emptive expiry refresh. The principal is derived
    /// from the JWT `sub` claim (M2) for the identity pillar.
    pub fn with_refresh_pair(
        base: impl Into<String>,
        access: Option<String>,
        refresh: Option<String>,
    ) -> Self {
        let principal = derive_principal(access.as_deref());
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            tokens: Arc::new(SharedTokens {
                state: RwLock::new(TokenState { access, refresh }),
                refresh_lock: tokio::sync::Mutex::new(()),
            }),
            principal,
            http: reqwest::Client::new(),
        }
    }
    /// v1.16.5 M5.2: `true` when the current access token is a JWT within
    /// `REFRESH_AHEAD_SECS` of expiry AND a refresh token is available.
    pub fn should_preemptive_refresh(&self) -> bool {
        let state = self.tokens.state.read().expect("token lock poisoned");
        state.refresh.is_some()
            && state
                .access
                .as_deref()
                .and_then(decode_claims)
                .map(|c| needs_refresh(Some(&c)))
                .unwrap_or(false)
    }
    /// v1.16.0 M1.1: the probe skips an unconfigured client (empty base = the
    /// pre-connect state) so the failure counter doesn't accumulate before the
    /// operator has entered a backend URL.
    pub fn is_configured(&self) -> bool {
        !self.base.is_empty()
    }
    /// v1.16.0 M2.1: identity-pillar accessor for the top bar.
    pub fn principal(&self) -> Option<&str> {
        self.principal.as_deref()
    }
    /// v1.16.5 M3.2: the request core — wraps one HTTP call with pre-emptive
    /// refresh + a single 401→refresh→retry. `f` builds the request (so the
    /// bearer can be re-attached after rotation). Returns the deserialized
    /// result, or the last error.
    async fn request<T, F>(&self, f: F) -> Result<T, ApiError>
    where
        T: for<'de> Deserialize<'de>,
        F: Fn(Option<&str>) -> reqwest::RequestBuilder,
    {
        // M5 pre-emptive: if the JWT is near expiry and we hold a refresh
        // token, rotate before the request so we never burn a 401 on the
        // first call at the expiry boundary.
        if self.should_preemptive_refresh() {
            let _ = self.refresh().await;
        }
        let resp = {
            let tok = self.access_token();
            f(tok.as_deref()).send().await.map_err(ApiError::Network)?
        };
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            // One retry only (M3.1). If the refresh fails (reuse/expired) the
            // error propagates; the caller surfaces the specific reason.
            if self.refresh_token().is_some() && self.refresh().await.is_ok() {
                let tok = self.access_token();
                let resp = f(tok.as_deref()).send().await.map_err(ApiError::Network)?;
                return self.unpack(resp).await;
            }
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Status(401, body));
        }
        self.unpack(resp).await
    }
    fn access_token(&self) -> Option<String> {
        self.tokens
            .state
            .read()
            .expect("token lock poisoned")
            .access
            .clone()
    }
    /// v1.23.0 "Roles": the JWT `roles` claim of the current access token
    /// (empty for an opaque/loopback token or a non-JWT). Defense-in-depth UI
    /// source — brain-server verifies + enforces; the client only reads it for
    /// button/panel gating. Never used for authorization.
    pub fn roles(&self) -> Vec<String> {
        self.access_token()
            .as_deref()
            .and_then(decode_claims)
            .map(|c| c.roles)
            .unwrap_or_default()
    }
    /// v1.27.11 "Console": the client-domain allowlist of a `client-auditor`,
    /// derived from the JWT `scopes` claim — the client mirror of the server's
    /// `client_authorized_domains` seam. `None` = not a client-auditor (or no
    /// token): unrestricted, the register was not row-filtered. `Some(&[])` =
    /// a client-auditor whose scopes grant nothing → renders NO clients. The
    /// client-admin dashboard re-filters its `/clients` rows by this allowlist
    /// (defense-in-depth — never renders a client the scopes don't grant, even
    /// if a misconfigured server returned it).
    pub fn client_auditor_domains(&self) -> Option<Vec<String>> {
        let tok = self.access_token()?;
        let claims = decode_claims(tok.as_ref())?;
        let auditor = claims.roles.iter().any(|r| r == "client-auditor");
        if !auditor {
            return None;
        }
        Some(scope_client_domains(claims.scope.as_deref()))
    }
    fn refresh_token(&self) -> Option<String> {
        self.tokens
            .state
            .read()
            .expect("token lock poisoned")
            .refresh
            .clone()
    }
    /// v1.16.5 M3.2: `POST /auth/refresh` — rotates the chain, stores the new
    /// pair in the shared state. brain-server serializes concurrent rotations
    /// under `BEGIN IMMEDIATE`; the loser gets 403 `refresh_reuse_detected`.
    ///
    /// v1.16.7 M7.2: single-flight. The `refresh_lock` serializes the read+POST
    /// across every clone sharing this `Arc`. A concurrent caller awaits the
    /// mutex, then re-reads the ROTATED refresh token (the winner already wrote
    /// it) and does a fresh rotation — never presenting the same stale token, so
    /// the server never burns a legitimate session on a reuse race.
    async fn refresh(&self) -> Result<(), ApiError> {
        // Held across the await; clippy's `await_holding_lock` does not flag a
        // `tokio::sync::Mutex` (it's an async-aware guard, not a std RwLock).
        let _serialize = self.tokens.refresh_lock.lock().await;
        let refresh_tok = match self.refresh_token() {
            Some(t) => t,
            None => return Err(ApiError::Status(401, "no refresh token".into())),
        };
        let body = serde_json::json!({ "refresh_token": refresh_tok });
        let resp = self
            .http
            .post(format!("{}{}", self.base, "/auth/refresh"))
            .json(&body)
            .send()
            .await
            .map_err(ApiError::Network)?;
        if !resp.status().is_success() {
            let code = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Status(code, body));
        }
        let pair: TokenPair = resp.json().await.map_err(ApiError::Network)?;
        let mut state = self.tokens.state.write().expect("token lock poisoned");
        state.access = Some(pair.access_token);
        state.refresh = Some(pair.refresh_token);
        Ok(())
    }
    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, ApiError> {
        let base = self.base.clone();
        let http = self.http.clone();
        self.request(move |tok| {
            let mut rb = http.get(format!("{base}{path}"));
            if let Some(t) = tok {
                rb = rb.bearer_auth(t);
            }
            rb
        })
        .await
    }
    async fn post_json<T, B>(&self, path: &str, body: &B) -> Result<T, ApiError>
    where
        T: for<'de> Deserialize<'de>,
        B: Serialize + ?Sized,
    {
        let base = self.base.clone();
        let http = self.http.clone();
        let body: serde_json::Value =
            serde_json::to_value(body).map_err(|e| ApiError::Status(500, e.to_string()))?;
        self.request(move |tok| {
            let mut rb = http.post(format!("{base}{path}")).json(&body);
            if let Some(t) = tok {
                rb = rb.bearer_auth(t);
            }
            rb
        })
        .await
    }
    /// v1.17.7 M4: POST a raw text body (the `/ingest/memory` contract reads
    /// the body as a UTF-8 text block, not JSON).
    pub async fn post_raw(&self, path: &str, body: String) -> Result<serde_json::Value, ApiError> {
        let base = self.base.clone();
        let http = self.http.clone();
        self.request(move |tok| {
            let mut rb = http.post(format!("{base}{path}")).body(body.clone());
            if let Some(t) = tok {
                rb = rb.bearer_auth(t);
            }
            rb
        })
        .await
    }
    async fn post_empty<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, ApiError> {
        let base = self.base.clone();
        let http = self.http.clone();
        self.request(move |tok| {
            let mut rb = http.post(format!("{base}{path}"));
            if let Some(t) = tok {
                rb = rb.bearer_auth(t);
            }
            rb
        })
        .await
    }
    /// v1.17.8 M7.3: the console's raw GET — returns the response body JSON
    /// (the try-it history is rendered from this). 404s surface as errors like
    /// every other call, so a bad path shows red, not silence.
    pub async fn get_raw(&self, path: &str) -> Result<serde_json::Value, ApiError> {
        let base = self.base.clone();
        let http = self.http.clone();
        self.request(move |tok| {
            let mut rb = http.get(format!("{base}{path}"));
            if let Some(t) = tok {
                rb = rb.bearer_auth(t);
            }
            rb
        })
        .await
    }
    /// v1.20.8 M3: one bounded read of the `/events` SSE feed. Connects, drains
    /// the first chunk (the server's handshake + the alerts it broadcasts from
    /// its bounded ring to a new subscriber), then closes. The caller reconnects
    /// on an interval and the monotonic `seq` guard dedups. Works with or
    /// without a bearer — reqwest carries the header where a browser
    /// `EventSource` cannot. `bytes_stream()` (not `bytes()`) because the feed
    /// never EOFs; reading one chunk then dropping the stream closes the
    /// connection honestly.
    pub async fn alert_events(&self) -> Result<Vec<AlertEvent>, ApiError> {
        use futures_util::StreamExt;
        let mut rb = self.http.get(format!("{}/events", self.base));
        if let Some(t) = self.access_token() {
            rb = rb.bearer_auth(t);
        }
        let resp = rb.send().await.map_err(ApiError::Network)?;
        if !resp.status().is_success() {
            let code = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Status(code, body));
        }
        let mut events = Vec::new();
        if let Some(chunk) = resp.bytes_stream().next().await {
            let chunk = chunk.map_err(ApiError::Network)?;
            for line in String::from_utf8_lossy(&chunk).lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    if let Some(e) = parse_alert_event(data.trim()) {
                        events.push(e);
                    }
                }
            }
        }
        Ok(events)
    }
    /// v1.17.8 M7.3: the console's raw DELETE.
    pub async fn delete_raw(&self, path: &str) -> Result<serde_json::Value, ApiError> {
        let base = self.base.clone();
        let http = self.http.clone();
        self.request(move |tok| {
            let mut rb = http.delete(format!("{base}{path}"));
            if let Some(t) = tok {
                rb = rb.bearer_auth(t);
            }
            rb
        })
        .await
    }
    async fn unpack<T: for<'de> Deserialize<'de>>(
        &self,
        resp: reqwest::Response,
    ) -> Result<T, ApiError> {
        if !resp.status().is_success() {
            let code = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Status(code, body));
        }
        resp.json::<T>().await.map_err(ApiError::Network)
    }
    /// GET /health — the connect-first onboarding probe + capacity story.
    pub async fn health(&self) -> Result<Health, ApiError> {
        self.get_json("/health").await
    }
    /// POST /recall — the decision-path viewer data. Empty for short queries
    /// (mirrors the backend's min_query_length gate). When `trace` is true the
    /// response carries a `trace_id` (v1.15.0 M2) replayable via `recall_trace`.
    /// `min_relevance` (high|medium|low) is the post-fusion tier filter.
    pub async fn recall(
        &self,
        query: &str,
        trace: bool,
        min_relevance: Option<&str>,
        include_flagged: bool,
    ) -> Result<RecallResponse, ApiError> {
        let q = query.trim();
        if q.len() < MIN_QUERY {
            return Ok(RecallResponse::default());
        }
        let mut body = serde_json::json!({ "query": q, "k": 8 });
        if trace {
            body["trace"] = serde_json::json!(true);
        }
        if let Some(t) = min_relevance {
            body["min_relevance"] = serde_json::json!(t);
        }
        if include_flagged {
            body["include_flagged"] = serde_json::json!(true);
        }
        self.post_json("/recall", &body).await
    }
    /// GET /recall/{trace_id}/trace — replay the recorded decision path (the
    /// deep-linkable artifact, v1.15.0 M2 / DESIGN §4.2). Returns the raw JSON
    /// the server stored (query, decision, domains, per-hit id/score/source).
    pub async fn recall_trace(&self, trace_id: i64) -> Result<serde_json::Value, ApiError> {
        self.get_json(&format!("/recall/{trace_id}/trace")).await
    }
    /// GET /proposals?status= — the review queue (bare Vec, not wrapped).
    pub async fn proposals(&self, status: &str) -> Result<Vec<Proposal>, ApiError> {
        self.get_json(&format!("/proposals?status={status}")).await
    }
    /// v1.20.23 "Calibrate": GET /proposals?status=&since=<unix>&limit=200 —
    /// a `created_at`-bounded decision page for the review stats. `limit` is
    /// fetched at the server cap so a window is not chalked to the 50 default.
    pub async fn proposals_since(
        &self,
        status: &str,
        since: i64,
    ) -> Result<Vec<Proposal>, ApiError> {
        self.get_json(&format!(
            "/proposals?status={status}&since={since}&limit=200"
        ))
        .await
    }
    /// POST /proposals/{id}/approve[?supersedes=N]
    pub async fn approve_proposal(
        &self,
        id: i64,
        supersedes: Option<i64>,
    ) -> Result<ApproveResult, ApiError> {
        let path = match supersedes {
            Some(s) => format!("/proposals/{id}/approve?supersedes={s}"),
            None => format!("/proposals/{id}/approve"),
        };
        self.post_empty(&path).await
    }
    /// POST /proposals/{id}/reject[?reason=…] — v1.16.0 M3: optional reason
    /// recorded in the audit log so a rejection isn't a silent drop.
    pub async fn reject_proposal(
        &self,
        id: i64,
        reason: Option<&str>,
    ) -> Result<RejectResult, ApiError> {
        let path = match reason {
            Some(r) => format!("/proposals/{id}/reject?reason={}", url_encode(r)),
            None => format!("/proposals/{id}/reject"),
        };
        self.post_empty(&path).await
    }
    /// POST /ingest/proposal — seed a sample proposal (onboarding first-value).
    pub async fn propose(&self, content: &str) -> Result<ProposalResponse, ApiError> {
        let body = serde_json::json!({ "content": content });
        self.post_json("/ingest/proposal", &body).await
    }
    /// POST /proposals/{id}/edit — v1.20.14 "Steer" M1: rewrite a pending
    /// proposal's content; the server re-scores deterministically and stamps
    /// `edited_at`. Returns the re-scored proposal.
    pub async fn edit_proposal(&self, id: i64, content: &str) -> Result<Proposal, ApiError> {
        let body = serde_json::json!({ "content": content });
        self.post_json(&format!("/proposals/{id}/edit"), &body)
            .await
    }
    /// GET /audit — the hash-chain browser. v1.16.0 M6/M7: `kind` is forwarded
    /// server-side (the backend filters the `kind` column — auth/ingest/recall/
    /// …); principal/since stay client-side (the wire contract for those params
    /// is a v1.19.0 polish). `limit` caps the row count.
    pub async fn audit(&self) -> Result<AuditResponse, ApiError> {
        self.get_json("/audit").await
    }
    /// GET /audit?kind=… — filtered audit feed (M6 auth-failure feed uses
    /// `kind=auth`, then the client filters `status == "denied"`).
    pub async fn audit_kind(&self, kind: &str) -> Result<AuditResponse, ApiError> {
        self.get_json(&format!("/audit?kind={kind}")).await
    }
    /// v1.16.7 M4: GET /audit?limit=N&offset=N — one page of the hash-chain
    /// browser (newest-first). The audit panel accumulates pages via this.
    pub async fn audit_page(&self, offset: usize, limit: usize) -> Result<AuditResponse, ApiError> {
        self.get_json(&format!("/audit?limit={limit}&offset={offset}"))
            .await
    }
    /// GET /audit/verify — chain integrity.
    pub async fn audit_verify(&self) -> Result<ChainVerify, ApiError> {
        self.get_json("/audit/verify").await
    }
    /// GET /quarantine — injection suspects awaiting review.
    pub async fn quarantine(&self) -> Result<QuarantineResponse, ApiError> {
        self.get_json("/quarantine").await
    }
    /// POST /quarantine/{id}/release | /delete
    pub async fn quarantine_action(
        &self,
        id: i64,
        action: &str,
    ) -> Result<QuarantineAction, ApiError> {
        self.post_empty(&format!("/quarantine/{id}/{action}")).await
    }
    /// POST /dsar — the locate→export→purge workflow.
    pub async fn dsar(&self, subject: &str, action: &str) -> Result<DsarResponse, ApiError> {
        let body = serde_json::json!({ "subject": subject, "action": action });
        self.post_json("/dsar", &body).await
    }
    /// v1.20.21 M2: POST /dsar with `dry_run: true` — preview the would-be
    /// deletion footprint (locate + bundle build) with no writes. Thin wrapper
    /// over the same `/dsar` path; the live builder above omits `dry_run`.
    pub async fn dsar_preview(&self, subject: &str) -> Result<Footprint, ApiError> {
        let body = dsar_preview_body(subject);
        let resp: DsarResponse = self.post_json("/dsar", &body).await?;
        resp.footprint
            .ok_or_else(|| ApiError::Status(200, "no footprint in preview response".into()))
    }
    /// GET /dsar/{id}/certificate — re-fetch a deletion certificate + live
    /// chain check. Returns the raw JSON `{certificate, chain_verifies}` so
    /// `DsarCertificate::from_value` can pull the typed card fields up.
    pub async fn dsar_certificate(&self, id: i64) -> Result<serde_json::Value, ApiError> {
        self.get_json(&format!("/dsar/{id}/certificate")).await
    }
    /// v1.20.22 M2.1: GET /dsar — the DSAR ledger page. The Subjects-panel
    /// clock derives each open row's Art 17 deadline from `created_at`.
    pub async fn dsar_ledger(&self) -> Result<DsarLedger, ApiError> {
        self.get_json("/dsar").await
    }
    /// GET /stats — corpus counts for the Health/onboarding story.
    pub async fn stats(&self) -> Result<Stats, ApiError> {
        self.get_json("/stats").await
    }
    // --- v1.17.6 M2 — Overview status + alert resources ----------------------

    /// GET /snapshot/status — `VACUUM INTO` `.bak` snapshot integrity (Admin).
    pub async fn snapshot_status(&self) -> Result<SnapshotStatus, ApiError> {
        self.get_json("/snapshot/status").await
    }
    /// GET /retention — the effective per-kind retention policy + counts.
    pub async fn retention(&self) -> Result<RetentionStatus, ApiError> {
        self.get_json("/retention").await
    }
    /// GET /ump/capabilities — the UMP 1.0 negotiation handshake (public).
    pub async fn ump_capabilities(&self) -> Result<UmpCapabilities, ApiError> {
        self.get_json("/ump/capabilities").await
    }
    /// GET /decayed — chunks whose effective expiry has passed (bare Vec).
    pub async fn decayed(&self) -> Result<Vec<DecayedRow>, ApiError> {
        self.get_json("/decayed").await
    }
    /// POST /consolidate/propose — pure detection, zero mutation (no body).
    pub async fn consolidate_propose(&self) -> Result<ConsolidateProposal, ApiError> {
        self.post_empty("/consolidate/propose").await
    }
    /// GET /tombstones?limit=N — the deletion registry page.
    pub async fn tombstones(&self, limit: u32) -> Result<TombstonesResponse, ApiError> {
        self.get_json(&format!("/tombstones?limit={limit}")).await
    }
    // --- v1.17.7 M3 — Graph methods -----------------------------------------

    /// GET /graph/entity/{name} — browse one entity. Returns raw JSON so the
    /// panel can distinguish a real entity from the backend's 200 `{error}`
    /// not-found envelope via `parse_entity`.
    pub async fn graph_entity(&self, name: &str) -> Result<serde_json::Value, ApiError> {
        self.get_json(&format!("/graph/entity/{}", url_encode(name)))
            .await
    }
    /// GET /graph/traverse — bounded walk from a seed. `explain=true` is always
    /// set (the panel renders the `paths` chains); `kind` supports `causes:`
    /// prefix semantics, `at` is a bi-temporal date, `cross_domain` fans out.
    pub async fn graph_traverse(
        &self,
        start: &str,
        depth: u8,
        kind: &str,
        at: &str,
        cross_domain: bool,
    ) -> Result<TraverseResponse, ApiError> {
        let mut q = format!(
            "/graph/traverse?start={}&max_depth={depth}&explain=true",
            url_encode(start)
        );
        let kind = kind.trim();
        if !kind.is_empty() {
            q.push_str(&format!("&kind={}", url_encode(kind)));
        }
        let at = at.trim();
        if !at.is_empty() {
            q.push_str(&format!("&at={}", url_encode(at)));
        }
        if cross_domain {
            q.push_str("&cross_domain=true");
        }
        self.get_json(&q).await
    }
    // --- v1.17.7 M4 — Create methods ----------------------------------------

    /// POST /ingest — structured memory with entities/relations (v1.14/v1.0).
    /// `entities`/`relations` are pre-validated JSON arrays; the backend is the
    /// source of truth for shape (the free-text editor only guarantees JSON).
    pub async fn ingest_structured(
        &self,
        title: &str,
        content: &str,
        kind: &str,
        domain: &str,
        entities: &serde_json::Value,
        relations: &serde_json::Value,
    ) -> Result<serde_json::Value, ApiError> {
        let mut body = serde_json::json!({ "title": title, "content": content });
        if !kind.trim().is_empty() {
            body["memory_kind"] = serde_json::json!(kind.trim());
        }
        if !domain.trim().is_empty() {
            body["domain"] = serde_json::json!(domain.trim());
        }
        if let Some(a) = entities.as_array() {
            if !a.is_empty() {
                body["entities"] = entities.clone();
            }
        }
        if let Some(a) = relations.as_array() {
            if !a.is_empty() {
                body["relations"] = relations.clone();
            }
        }
        self.post_json("/ingest", &body).await
    }
    /// POST /ingest/markdown — markdown paste (source-path linkage when given).
    pub async fn ingest_markdown(
        &self,
        content: &str,
        title: Option<&str>,
        source_path: Option<&str>,
        domain: &str,
        replace: bool,
    ) -> Result<serde_json::Value, ApiError> {
        let mut body = serde_json::json!({ "content": content });
        if let Some(t) = title.filter(|t| !t.trim().is_empty()) {
            body["title"] = serde_json::json!(t.trim());
        }
        if let Some(sp) = source_path.filter(|s| !s.trim().is_empty()) {
            body["source_path"] = serde_json::json!(sp.trim());
        }
        if !domain.trim().is_empty() {
            body["domain"] = serde_json::json!(domain.trim());
        }
        if replace {
            body["replace"] = serde_json::json!(true);
        }
        self.post_json("/ingest/markdown", &body).await
    }
    /// POST /ingest/memory — a markdown-style block of `## [Title]` entries.
    /// The backend parses entries client-free (raw text body, not JSON).
    pub async fn ingest_memory(&self, content: &str) -> Result<serde_json::Value, ApiError> {
        self.post_raw("/ingest/memory", content.to_string()).await
    }
    /// POST /procedure — an ordered-step procedure in one tx.
    pub async fn procedure_create(
        &self,
        title: &str,
        content: &str,
        steps: &serde_json::Value,
        domain: &str,
    ) -> Result<ProcedureResponse, ApiError> {
        let mut body = serde_json::json!({ "title": title, "content": content });
        if let Some(a) = steps.as_array() {
            if !a.is_empty() {
                body["steps"] = steps.clone();
            }
        }
        if !domain.trim().is_empty() {
            body["domain"] = serde_json::json!(domain.trim());
        }
        self.post_json("/procedure", &body).await
    }
    /// GET /procedure/{id}/steps — the ordered steps view.
    pub async fn procedure_steps(&self, id: i64) -> Result<ProcedureStepsResponse, ApiError> {
        self.get_json(&format!("/procedure/{id}/steps")).await
    }
    /// POST /classify — deterministic keyword categorization.
    pub async fn classify(&self, text: &str) -> Result<ClassifyResponse, ApiError> {
        let body = serde_json::json!({ "text": text });
        self.post_json("/classify", &body).await
    }
    /// POST /decision/{id}/evaluate — the fired branch for numeric variables.
    pub async fn decision_evaluate(
        &self,
        id: i64,
        variables: &serde_json::Value,
    ) -> Result<DecisionOutcome, ApiError> {
        let body = serde_json::json!({ "variables": variables });
        self.post_json(&format!("/decision/{id}/evaluate"), &body)
            .await
    }
    /// POST /consolidate/apply — record one or more operator-chosen links
    /// (`kind = "supersedes"` expires the older chunk at retrieval time).
    pub async fn consolidate_apply(
        &self,
        links: &serde_json::Value,
    ) -> Result<ApplyResponse, ApiError> {
        let body = serde_json::json!({ "links": links });
        self.post_json("/consolidate/apply", &body).await
    }
    /// POST /consolidate/undo — reverse prior supersession resolutions.
    pub async fn consolidate_undo(&self, ids: &[i64]) -> Result<UndoResponse, ApiError> {
        let body = serde_json::json!({ "old_chunks": ids });
        self.post_json("/consolidate/undo", &body).await
    }
    // --- v1.17.8 M5 — Rights/Data methods ------------------------------------

    /// POST /purge — hard erasure of chunks by id or owner (Admin). `ids XOR
    /// owner`; the server tombstones + audits. Returns `{purged, reason}`.
    pub async fn purge(&self, ids: &[i64], owner: Option<&str>) -> Result<PurgeResult, ApiError> {
        let mut body = serde_json::json!({});
        if !ids.is_empty() {
            body["ids"] = serde_json::json!(ids);
        }
        if let Some(o) = owner.filter(|o| !o.trim().is_empty()) {
            body["owner"] = serde_json::json!(o.trim());
        }
        self.post_json("/purge", &body).await
    }
    /// GET /export?format= — the portable GDPR/UMP export body (Admin for
    /// `format != json`). `json` returns the full `{knowledge, entities,
    /// relationships, proposals}` object the panel can hand to a downloader.
    pub async fn export(&self, format: &str) -> Result<serde_json::Value, ApiError> {
        self.get_json(&format!("/export?format={}", url_encode(format)))
            .await
    }
    /// POST /retention — set an override (`{kind, days}`) or a full
    /// `{policy: {kind: days}}` map (Admin). Returns `{updated, set}`.
    pub async fn retention_set(
        &self,
        kind: &str,
        days: i64,
    ) -> Result<RetentionSetResult, ApiError> {
        let body = serde_json::json!({ "kind": kind, "days": days });
        self.post_json("/retention", &body).await
    }
    /// POST /retention — clear an override back to the code default.
    pub async fn retention_clear(&self, kind: &str) -> Result<RetentionSetResult, ApiError> {
        let body = serde_json::json!({ "kind": kind, "days": serde_json::Value::Null });
        self.post_json("/retention", &body).await
    }
    // --- v1.17.8 M6 — UMP ops methods ----------------------------------------

    /// GET /ump/memory/{id} — one UMP record by numeric id or `urn:ump:…`.
    pub async fn ump_memory(&self, id: &str) -> Result<serde_json::Value, ApiError> {
        self.get_json(&format!("/ump/memory/{}", url_encode(id)))
            .await
    }
    /// POST /ump/remember — lower a partial record (created|merged|rejected).
    pub async fn ump_remember(&self, record: &serde_json::Value) -> Result<UmpWrite, ApiError> {
        self.post_json("/ump/remember", record).await
    }
    /// POST /ump/revise — patch an existing record → new revision + supersede.
    pub async fn ump_revise(
        &self,
        id: &str,
        patch: &serde_json::Value,
    ) -> Result<UmpRevise, ApiError> {
        let body = serde_json::json!({ "id": id, "patch": patch });
        self.post_json("/ump/revise", &body).await
    }
    /// POST /ump/forget — soft (tombstoned) or hard (erased) deletion.
    pub async fn ump_forget(
        &self,
        id: &str,
        reason: Option<&str>,
        hard: bool,
    ) -> Result<UmpForget, ApiError> {
        let mut body = serde_json::json!({ "id": id, "hard": hard });
        if let Some(r) = reason.filter(|r| !r.trim().is_empty()) {
            body["reason"] = serde_json::json!(r.trim());
        }
        self.post_json("/ump/forget", &body).await
    }
    /// POST /ump/feedback — followed|overridden|ignored|contradicted.
    pub async fn ump_feedback(
        &self,
        id: &str,
        outcome: &str,
        session: Option<&str>,
    ) -> Result<UmpOk, ApiError> {
        let mut body = serde_json::json!({ "id": id, "outcome": outcome });
        if let Some(s) = session.filter(|s| !s.trim().is_empty()) {
            body["session"] = serde_json::json!(s.trim());
        }
        self.post_json("/ump/feedback", &body).await
    }
    /// POST /ump/recall — UMP §3.2 recall with the five signals per result.
    pub async fn ump_recall(
        &self,
        query: &str,
        limit: u32,
        kind: Option<&str>,
    ) -> Result<UmpRecall, ApiError> {
        let mut body = serde_json::json!({ "query": query, "limit": limit });
        if let Some(k) = kind.filter(|k| !k.trim().is_empty()) {
            body["filter"] = serde_json::json!({ "kind": vec![k.trim()] });
        }
        self.post_json("/ump/recall", &body).await
    }
    /// POST /ump/audit — the reference audit facility (Admin, tenant-scoped).
    pub async fn ump_audit(&self, kind: Option<&str>, limit: usize) -> Result<UmpAudit, ApiError> {
        let mut body = serde_json::json!({ "limit": limit });
        if let Some(k) = kind.filter(|k| !k.trim().is_empty()) {
            body["kind"] = serde_json::json!(k.trim());
        }
        self.post_json("/ump/audit", &body).await
    }
    /// GET /ump/audit/verify — authoritative full-chain verification.
    pub async fn ump_audit_verify(&self) -> Result<UmpOk, ApiError> {
        self.get_json("/ump/audit/verify").await
    }
    // --- v1.17.8 M7 — System methods -----------------------------------------

    /// GET /art30 — the Art 30 records-of-processing register (Admin).
    pub async fn art30(&self) -> Result<serde_json::Value, ApiError> {
        self.get_json("/art30").await
    }
    /// GET /domains — the domain registry (name + counts).
    pub async fn domains(&self) -> Result<DomainsResponse, ApiError> {
        self.get_json("/domains").await
    }
    /// v1.21.0 "Profiles": GET /profiles — the wizard's pick list (the 12
    /// seeded presets + operator clones), name-ordered.
    pub async fn profiles(&self) -> Result<Vec<ProfileView>, ApiError> {
        let v: serde_json::Value = self.get_json("/profiles").await?;
        Ok(v["profiles"]
            .as_array()
            .map(|a| a.iter().map(ProfileView::from_value).collect())
            .unwrap_or_default())
    }
    /// v1.21.0: GET /domains/{domain}/profile — the binding + effective knobs
    /// (Health panel + wizard eligibility).
    pub async fn domain_profile(&self, domain: &str) -> Result<DomainProfile, ApiError> {
        let v: serde_json::Value = self.get_json(&format!("/domains/{domain}/profile")).await?;
        Ok(DomainProfile::from_value(&v))
    }
    /// v1.21.0: POST /domains/{domain}/profile — bind (`Some`) or unbind
    /// (`None`). Takes effect at the next request (defaults, not a migration).
    pub async fn bind_profile(
        &self,
        domain: &str,
        profile: Option<&str>,
    ) -> Result<serde_json::Value, ApiError> {
        self.post_json(
            &format!("/domains/{domain}/profile"),
            &serde_json::json!({ "profile": profile }),
        )
        .await
    }
    /// GET /connectors — the registered connector ledger.
    pub async fn connectors(&self) -> Result<ConnectorsResponse, ApiError> {
        self.get_json("/connectors").await
    }
    /// POST /reindex — re-embed every chunk (Admin). Returns `{status,
    /// reembedded, skipped}`.
    pub async fn reindex(&self) -> Result<ReindexResult, ApiError> {
        self.post_empty("/reindex").await
    }
    /// POST /sources/reconcile — retire sources whose URI is no longer live.
    pub async fn sources_reconcile(
        &self,
        kind: &str,
        live_uris: &[String],
    ) -> Result<ReconcileResult, ApiError> {
        let body = serde_json::json!({ "kind": kind, "live_uris": live_uris });
        self.post_json("/sources/reconcile", &body).await
    }
    /// DELETE /sources/{id} — retire one source + sweep its chunks.
    pub async fn delete_source(&self, id: i64) -> Result<serde_json::Value, ApiError> {
        let base = self.base.clone();
        let http = self.http.clone();
        self.request(move |tok| {
            let mut rb = http.delete(format!("{base}/sources/{id}"));
            if let Some(t) = tok {
                rb = rb.bearer_auth(t);
            }
            rb
        })
        .await
    }
    /// v1.27.11 "Console": GET /clients — the BPO register. A `client-auditor`
    /// sees only its granted client-domain(s) (server-side R9 row filter); the
    /// client-admin dashboard renders exactly these rows and nothing else.
    pub async fn clients(&self) -> Result<Vec<ClientRow>, ApiError> {
        let v: serde_json::Value = self.get_json("/clients").await?;
        Ok(v["clients"]
            .as_array()
            .map(|a| a.iter().filter_map(ClientRow::from_value).collect())
            .unwrap_or_default())
    }
}
// --- v1.16.5 "Secure" helpers ------------------------------------------------

/// v1.16.5 M1: decoded JWT claims the client reads for display + expiry
/// tracking. NO signature verification — brain-server verifies on receipt;
/// the client trusts the claims for UI only (never for authorization).
#[derive(Debug, Deserialize, Serialize)]
pub struct TokenClaims {
    #[serde(default)]
    pub sub: Option<String>,
    #[serde(default)]
    pub exp: Option<i64>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub roles: Vec<String>,
}
/// v1.16.5 M3: the `/auth/refresh` response pair. Mirrors openapi.yaml's
/// `TokenPair`; `token_type`/`expires_in` are read but unused by the client.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct TokenPair {
    access_token: String,
    refresh_token: String,
}
/// v1.16.5 M1.2: decode the JWT payload (middle segment) WITHOUT signature
/// verification. Ponytail: the client does NOT verify the JWT signature —
/// brain-server verifies on receipt. A forged JWT would be rejected by
/// brain-server on the next API call; the client reads claims for display +
/// expiry tracking only, never for authorization. Returns `None` on any
/// malformed input (not a 3-part JWT, bad base64url, non-JSON payload).
pub fn decode_claims(token: &str) -> Option<TokenClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let payload = base64url_decode(parts[1])?;
    serde_json::from_slice(&payload).ok()
}

/// v1.27.11 "Console": parse the JWT `scopes` claim (a whitespace-separated
/// `action:team/domain` list, mirroring the server `Scope`) and extract the
/// non-wildcard client-domains. The `global` operator root + `*` are never a
/// valid auditor grant (the min-necessary wedge). Mirrors the server
/// `client_authorized_domains` filter exactly, so the client dashboard and the
/// server row-filter agree. `None`/unknown → empty (deny-by-default).
pub fn scope_client_domains(scope: Option<&str>) -> Vec<String> {
    scope
        .unwrap_or("")
        .split_whitespace()
        .filter_map(|grant| {
            // Require the `action:team/domain` shape (like the server Scope::parse)
            // so a malformed grant with no '/' is dropped, not treated as a domain.
            let (_action, domain) = grant.split_once('/')?;
            let domain = domain.to_ascii_lowercase();
            if domain == "*" || domain == "global" {
                None
            } else {
                Some(domain)
            }
        })
        .collect()
}
/// v1.16.5 M1.2: is this a JWT-shaped token (3 dot-separated segments)?
/// Used to distinguish an opaque loopback token (no identity) from a JWT that
/// failed payload decode (still shown a neutral identity label).
fn is_jwt_shaped(token: Option<&str>) -> bool {
    token.map(|t| t.split('.').count() == 3).unwrap_or(false)
}
/// v1.16.5 M2: derive the identity-pillar principal from an access token.
/// `Some(sub)` when the token is a decodable JWT with a `sub`; a neutral
/// label for a JWT-shaped token without one; `None` for an opaque token
/// (loopback — the server trusts localhost, no identity to claim).
fn derive_principal(token: Option<&str>) -> Option<String> {
    match token.and_then(decode_claims) {
        Some(c) => c.sub.or(Some("token (no sub)".into())),
        None if is_jwt_shaped(token) => Some("token (no sub)".into()),
        None => None,
    }
}
/// v1.16.5 M1.2: base64url-decode (RFC 4648 §5, the JWT alphabet — `-`/`_`
/// instead of `+`/`/`, padding optional). Ponytail: no base64 dep — the few
/// lines beat a crate for one decode path. Returns `None` on invalid chars.
fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut rev = [255u8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        rev[c as usize] = i as u8;
    }
    // Reject the unpadded-length variants that can't encode bytes cleanly.
    let clean: String = s.chars().filter(|&c| c != '=').collect();
    let n = clean.len();
    if n % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(n * 3 / 4);
    let bytes: Vec<u8> = clean.bytes().collect();
    for chunk in bytes.chunks(4) {
        let mut acc: u32 = 0;
        let mut bits = 0u32;
        for &b in chunk {
            let v = *rev.get(b as usize)?;
            if v == 255 {
                return None;
            }
            acc = (acc << 6) | v as u32;
            bits += 6;
        }
        // Emit full bytes only; trailing bits beyond a byte boundary are the
        // JWT's zero-padding and dropped.
        while bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}
/// v1.16.5 M1.2: base64url-encode (RFC 4648 §5, the JWT alphabet — `-`/`_`,
/// unpadded). Test-only mirror of the decode path (production never mints
/// tokens, so encode exists only to build round-trip fixtures).
fn base64url_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[n as usize & 63] as char);
        }
    }
    out
}
/// v1.16.5 M5.1: current unix time in seconds (the `exp` claim's unit).
/// v1.20.15 "Clock": delegates to the shared core (one implementation, no
/// per-module copies).
fn now_unix() -> i64 {
    crate::time_budget::now_unix()
}
/// v1.16.5 M5.1: pre-emptive expiry check, extracted pure for tests.
/// `exp` within `REFRESH_AHEAD_SECS` of now → refresh needed. `None` exp
/// (opaque token) → never.
fn needs_refresh(claims: Option<&TokenClaims>) -> bool {
    match claims.and_then(|c| c.exp) {
        Some(exp) => exp - now_unix() < REFRESH_AHEAD_SECS,
        None => false,
    }
}
/// v1.16.0 M3: percent-encode a query value for `?reason=…`. Ponytail: the
/// backend reads `reason` as a flat string; we encode the few reserved chars
/// that would break the query (`&`, `=`, `+`, `#`, ` `, `?`) rather than pull
/// a `urlencoding`/`percent-encoding` crate for one call site.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'-' | b'_' | b'.' | b'~' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
// --- Wire types (mirror openapi.yaml) ----------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct Health {
    pub status: String,
    pub version: String,
    pub capacity: Option<Capacity>, // omitted when the pool was momentarily exhausted
    #[serde(default)]
    pub hardening: Option<Hardening>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Capacity {
    pub docs: u64,
    pub max_docs: u64,
    pub rss_mib: u64,
    pub max_rss_mib: u64,
    pub status: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Hardening {
    pub unsafe_blocks: u64,
    pub panics_caught: u64,
    pub memory_leaks_detected: u64,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RecallResponse {
    #[serde(default)]
    pub hits: Vec<Hit>,
    #[serde(default)]
    pub decision: String,
    /// v1.15.0 M2: the audit row id for this recall's read event, present when
    /// `?trace=true` was requested AND read-event audit is enabled. Replayable
    /// via `ApiClient::recall_trace`.
    #[serde(default)]
    pub trace_id: Option<i64>,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Hit {
    pub id: i64,
    #[serde(default)]
    pub title: Option<String>,
    pub content: String,
    #[serde(default)]
    pub snippet: Option<String>,
    pub score: f64,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub conflict: Option<bool>,
    #[serde(default)]
    pub provenance: Option<HitProvenance>,
    /// v1.14.0 M3: `assertion_kind` (stated|observed|inferred).
    #[serde(default)]
    pub assertion_kind: Option<String>,
    /// v1.14.0 M3: deterministic stored confidence (0..1).
    #[serde(default)]
    pub confidence: Option<f32>,
    /// v1.14.0 M3: relevance tier (high|medium|low) derived from fused score.
    #[serde(default)]
    pub relevance: Option<String>,
    /// v1.14.0 M2: true when this chunk's `expires_at` is past (only present
    /// when the caller opted into decayed results).
    #[serde(default)]
    pub decayed: Option<bool>,
    /// v0.9.7 Guard: true when this chunk was quarantined by the injection
    /// screen (`flagged=1`). Only present on `include_flagged` recalls — the
    /// v1.20.6 `/ops` console's visible screen output.
    #[serde(default)]
    pub flagged: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HitProvenance {
    #[serde(default)]
    pub vector_rank: Option<i64>,
    #[serde(default)]
    pub fts_rank: Option<i64>,
    #[serde(default)]
    pub graph_rank: Option<i64>,
    #[serde(default)]
    pub fused_score: Option<f64>,
    #[serde(default)]
    pub rerank_score: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Proposal {
    pub id: i64,
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub source: Option<String>,
    /// v1.20.1 "Shield" M2: the caller-provided prompt that fed this capture,
    /// so a reviewer can context-check it before deciding.
    #[serde(default)]
    pub source_prompt: Option<String>,
    /// v1.20.3 "Classify" G5: the injection-screen verdict for this proposal
    /// (`clean`/`quarantine`), recomputed at read time. `None` for a legacy
    /// row; `Reject` is never persisted (reads as `quarantine` server-side).
    #[serde(default)]
    pub screen_verdict: Option<String>,
    #[serde(default)]
    pub authority: Option<f32>,
    pub novelty: f32,
    #[serde(default)]
    pub conflict_with: Option<i64>,
    pub salience: f32,
    pub created_at: i64,
    /// v1.20.14 "Steer" M1: unix ts of the last content rewrite, `None` if the
    /// pending proposal was never edited. Keys the edited badge.
    #[serde(default)]
    pub edited_at: Option<i64>,
    /// v1.20.15 "Clock" M1: when this proposal ages out of the review window
    /// (unix ts, `created_at + TTL`, server-derived). The client ticks its
    /// countdown against this absolute deadline — no client TTL mirror.
    #[serde(default)]
    pub expires_at: i64,
    /// v1.20.15 "Clock" M1: the SLA band boundaries (secs of remaining life),
    /// server-provided so the countdown color follows an operator threshold
    /// override with no rebuild.
    #[serde(default)]
    pub warn_secs: i64,
    #[serde(default)]
    pub critical_secs: i64,
    /// v1.20.23 "Calibrate": unix ts of the decision (approve/reject/expire),
    /// `None` while pending. The reviewer-latency input (`decided_at - created_at`).
    #[serde(default)]
    pub decided_at: Option<i64>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApproveResult {
    pub proposal_id: i64,
    pub chunk_id: i64,
    pub status: String,
    #[serde(default)]
    pub superseded: Option<i64>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RejectResult {
    pub proposal_id: i64,
    pub status: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProposalResponse {
    pub id: i64,
    pub status: String,
    pub novelty: f32,
    #[serde(default)]
    pub conflict_with: Option<i64>,
    pub salience: f32,
}
#[derive(Debug, Deserialize)]
pub struct AuditResponse {
    pub events: Vec<AuditRow>,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AuditRow {
    pub id: i64,
    pub ts: String,
    pub kind: String,
    pub actor: String,
    #[serde(default)]
    pub target_hash: String,
    pub status: String,
    #[serde(default)]
    pub detail_hash: String,
    #[serde(default)]
    pub tenant_id: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct ChainVerify {
    pub ok: bool,
}
#[derive(Debug, Deserialize)]
pub struct QuarantineResponse {
    pub quarantined: Vec<QuarantineRow>,
    pub count: u64,
}
#[derive(Debug, Clone, Deserialize)]
pub struct QuarantineRow {
    pub id: i64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct QuarantineAction {
    pub ok: bool,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DsarResponse {
    pub id: i64,
    pub subject: String,
    pub status: String,
    #[serde(default)]
    pub certificate: Option<serde_json::Value>,
    /// v1.20.21: present only on a dry-run — the would-be deletion footprint.
    #[serde(default)]
    pub footprint: Option<Footprint>,
}
/// v1.20.22 M1.2: `GET /dsar` — the bounded ledger page. The client clock
/// derives each open row's Art 17 deadline from `created_at` (server-window
/// stamp; there's no client mirror of `BRAIN_DSAR_WINDOW_DAYS`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DsarLedger {
    #[serde(default)]
    pub requests: Vec<DsarLedgerRow>,
    #[serde(default)]
    pub total: i64,
}
/// v1.20.22 M1.2: one DSAR request ledger row. `#[serde(default)]` timestamps
/// make a row with a missing `completed_at` (an open request) parse cleanly —
/// the countdown is `created_at`-keyed.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DsarLedgerRow {
    pub id: i64,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: Option<i64>,
    /// v1.20.22: the server-computed Art 17 deadline (`created_at + window`),
    /// so the client countdown ticks against the same number the POST response
    /// carries — no client mirror of `BRAIN_DSAR_WINDOW_DAYS`.
    #[serde(default)]
    pub deadline: Option<i64>,
    #[serde(default)]
    pub completed_at: Option<i64>,
}
/// v1.20.21: the would-be DSAR deletion footprint a dry-run returns — what a
/// live purge would locate + export + delete, without executing any write.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Footprint {
    pub roots: usize,
    pub derived: usize,
    pub export_rows: usize,
    pub tombstones: usize,
    pub dsar_rows: usize,
    pub dry_run: bool,
}
/// v1.20.21 M2: pure core — pull the `Footprint` out of a `/dsar` dry-run
/// response. `None` on shape drift (no `footprint` object). Mirrors the
/// `parse_purge_result`/`parse_ump_recall` decode pattern.
pub fn parse_footprint(value: &serde_json::Value) -> Option<Footprint> {
    value
        .get("footprint")
        .and_then(|f| serde_json::from_value(f.clone()).ok())
}
/// v1.20.21 M2: the dry-run request body builder. Pure so a wire test pins
/// that `dry_run: true` rides the `/dsar` body (the panic-risk of a preview
/// accidentally purging is a serialization detail, so it's worth a pin).
pub fn dsar_preview_body(subject: &str) -> serde_json::Value {
    serde_json::json!({ "subject": subject, "action": "both", "dry_run": true })
}
/// v1.16.0 M5: the deletion-certificate card fields, pulled up from the
/// raw `{certificate, chain_verifies}` envelope by `from_value`. Not derived
/// Deserialize directly: the typed fields live inside the nested `certificate`
/// object, so a single serde pass can't populate them — `from_value` walks both.
#[derive(Debug, Clone, PartialEq)]
pub struct DsarCertificate {
    /// The full certificate object (kept for unknown future fields).
    pub certificate: serde_json::Value,
    /// v1.15.0: live re-verification of the audit chain the cert anchored to.
    pub chain_verifies: bool,
    /// v1.16.0 M5: typed card fields read straight off the certificate object.
    pub found_count: u64,
    pub purged_ids: Vec<i64>,
    pub tombstone_root: Option<i64>,
    pub certified_at: String,
    pub chain_head: String,
}
impl DsarCertificate {
    /// v1.16.0 M5: the typed fields live inside `certificate` on the wire; pull
    /// them up so the card reads typed values. Unknown fields stay in the raw
    /// value. Called after deserialize via `from_value`.
    pub fn from_value(v: serde_json::Value) -> Self {
        let chain_verifies = v
            .get("chain_verifies")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let c = v.get("certificate").cloned().unwrap_or_default();
        let g = |k: &str| c.get(k);
        Self {
            found_count: g("found_count").and_then(|x| x.as_u64()).unwrap_or(0),
            purged_ids: g("purged_ids")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
                .unwrap_or_default(),
            tombstone_root: g("tombstone_root").and_then(|x| x.as_i64()),
            certified_at: g("certified_at")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            chain_head: g("chain_head")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            certificate: c,
            chain_verifies,
        }
    }
}
#[derive(Debug, Deserialize)]
pub struct Stats {
    pub count: u64,
    pub embeddings: u64,
    pub entities: u64,
    pub relationships: u64,
    pub model: String,
    pub version: String,
}
// --- v1.17.6 M2 — Overview wire types (mirror the confirmed handler shapes) --

/// `GET /snapshot/status` (govern.rs) — every `VACUUM INTO` `.bak` in the DB dir.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SnapshotStatus {
    pub db: String,
    pub snapshot_count: u64,
    pub all_ok: bool,
    #[serde(default)]
    pub snapshots: Vec<SnapshotRow>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SnapshotRow {
    pub file: String,
    pub exists: bool,
    pub size_bytes: u64,
    pub mode_0600: bool,
    pub integrity_check: bool,
    pub audit_chain_ok: bool,
    pub ok: bool,
}
/// `GET /retention` (govern.rs) — effective policy + per-kind counts.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RetentionStatus {
    pub enabled: bool,
    #[serde(default)]
    pub policy: std::collections::BTreeMap<String, i64>,
    #[serde(default)]
    pub counts: std::collections::BTreeMap<String, i64>,
    #[serde(default)]
    pub projection: String,
}
/// `GET /ump/capabilities` (ump_ops.rs) — the §3.1 negotiation handshake.
#[derive(Debug, Clone, Deserialize)]
pub struct UmpCapabilities {
    pub server: UmpServer,
    pub ump: String,
    pub conformance: String,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub bindings: Vec<String>,
    #[serde(default)]
    pub retrieval_signals: Vec<String>,
    pub max_recall: i64,
    pub writable: bool,
    pub audit: bool,
}
#[derive(Debug, Clone, Deserialize)]
pub struct UmpServer {
    pub name: String,
    pub version: String,
}
/// `GET /decayed` (gate.rs) — a bare Vec of expired chunks.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DecayedRow {
    pub id: i64,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub effective_expiry: Option<i64>,
    #[serde(default)]
    pub memory_kind: String,
    #[serde(default)]
    pub reason: String,
}
/// `POST /consolidate/propose` (consolidate.rs) — detection counts for the
/// alert list. Nested views (`conflicts`/`stale_sources`/`near_duplicates`)
/// are only counted here, so they stay raw `Value`s; the typed pair/id shapes
/// the Overview reads are fully typed.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConsolidateProposal {
    #[serde(default)]
    pub exact_duplicates: Vec<Vec<i64>>,
    #[serde(default)]
    pub conflicts: Vec<serde_json::Value>,
    #[serde(default)]
    pub unresolved_contradictions: Vec<(i64, i64)>,
    #[serde(default)]
    pub stale_sources: Vec<serde_json::Value>,
    #[serde(default)]
    pub near_duplicates: Vec<serde_json::Value>,
}
/// `GET /tombstones?limit=` (observe.rs) — the deletion-registry page.
#[derive(Debug, Deserialize)]
pub struct TombstonesResponse {
    #[serde(default)]
    pub tombstones: Vec<TombstoneRow>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TombstoneRow {
    pub knowledge_id: i64,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub purged_at: Option<i64>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub origin_id: Option<i64>,
}
// --- v1.17.7 M3/M4 — Graph + Create wire types (pinned to the real server) ---

/// `GET /graph/entity/{name}` — one entity + its typed relation rows. The
/// backend lowercases the name; a missing entity is a 200 `{error}` envelope
/// that `parse_entity` maps to `None`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EntityView {
    pub name: String,
    #[serde(rename = "type", default)]
    pub entity_type: String,
    #[serde(default)]
    pub relations: Vec<EntityRel>,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EntityRel {
    #[serde(rename = "to_entity")]
    pub other: String,
    pub relation_type: String,
    #[serde(rename = "direction")]
    pub dir: String,
}
/// `GET /graph/traverse` — the flat `traversal` rows (back-compat) + the
/// structured `paths` chains (v1.7.0 "Explain"). Field names pinned to the
/// actual server output (`from_entity` is the seed name; `path` is `id->id`,
/// `edge_path` is `rel|rel`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TraverseResponse {
    #[serde(default)]
    pub traversal: Vec<TraversalRow>,
    #[serde(default)]
    pub visited: usize,
    #[serde(default)]
    pub paths: Vec<PathChain>,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TraversalRow {
    pub entity: String,
    pub depth: i64,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub edge_path: String,
    #[serde(default)]
    pub from_entity: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PathChain {
    #[serde(default)]
    pub hops: Vec<Hop>,
    #[serde(default)]
    pub depth: Option<i64>,
    #[serde(default)]
    pub domain: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Hop {
    pub from: HopNode,
    pub relation: String,
    pub to: HopNode,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HopNode {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
}
/// `POST /procedure` — id + step ids (the steps are their own chunks).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProcedureResponse {
    pub id: i64,
    pub status: String,
    #[serde(default)]
    pub step_ids: Vec<i64>,
}
/// `GET /procedure/{id}/steps` — the ordered step view.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProcedureStepsResponse {
    pub procedure_id: i64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub steps: Vec<StepView>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StepView {
    pub step_index: i64,
    pub id: i64,
    #[serde(default)]
    pub title: Option<String>,
    pub content: String,
    #[serde(default)]
    pub memory_kind: String,
}
/// `POST /classify` — the winning category + its matched keywords + taxonomy.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ClassifyResponse {
    pub result: CategoryResult,
    #[serde(default)]
    pub categories: Vec<String>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CategoryResult {
    pub category: String,
    pub confidence: f32,
    #[serde(default)]
    pub matched_keywords: Vec<String>,
}
/// `POST /decision/{id}/evaluate` — the fired branch (or the default).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DecisionOutcome {
    pub result: String,
    #[serde(default)]
    pub matched_condition: Option<String>,
    #[serde(default)]
    pub citation: Option<i64>,
    pub used_default: bool,
}
/// `POST /consolidate/apply` — how many links were recorded vs rejected.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApplyResponse {
    pub recorded: usize,
    #[serde(default)]
    pub rejected: Vec<String>,
}
/// `POST /consolidate/undo` — how many prior supersessions were reversed.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UndoResponse {
    pub undone: usize,
    #[serde(default)]
    pub rejected: Vec<String>,
}
// --- v1.17.8 M5/M6/M7 — Rights, UMP, System wire types ------------------------

/// `POST /purge` (gate.rs) — how many chunks were erased + the tombstone reason.
#[derive(Debug, Deserialize)]
pub struct PurgeResult {
    pub purged: usize,
    #[serde(default)]
    pub reason: String,
}
/// `POST /retention` (govern.rs) — how many overrides were written.
#[derive(Debug, Deserialize)]
pub struct RetentionSetResult {
    pub updated: usize,
    #[serde(default)]
    pub set: Vec<serde_json::Value>,
}
/// `POST /ump/remember` (ump_ops.rs) — the content-addressed id + the §3.3
/// result word (`created | merged | rejected`).
#[derive(Debug, Deserialize)]
pub struct UmpWrite {
    pub id: String,
    pub result: String,
}
/// `POST /ump/revise` (ump_ops.rs) — the new revision id + the old id it
/// superseded.
#[derive(Debug, Deserialize)]
pub struct UmpRevise {
    pub id: String,
    #[serde(default)]
    pub supersedes: Vec<String>,
}
/// `POST /ump/forget` (ump_ops.rs) — `erased` (hard) vs `tombstoned` (soft).
#[derive(Debug, Deserialize)]
pub struct UmpForget {
    pub result: String,
}
/// `POST /ump/feedback` / `GET /ump/audit/verify` — the `{ok: true}` contract.
#[derive(Debug, Deserialize)]
pub struct UmpOk {
    pub ok: bool,
}
/// `POST /ump/recall` (ump_ops.rs) — §3.2 results: record + five signals.
#[derive(Debug, Deserialize)]
pub struct UmpRecall {
    #[serde(default)]
    pub results: Vec<UmpRecallResult>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct UmpRecallResult {
    pub record: serde_json::Value,
    #[serde(default)]
    pub signals: serde_json::Value,
    #[serde(default)]
    pub score: f32,
}
/// `POST /ump/audit` (ump_ops.rs) — `{rows, count}` (rows stay raw Values;
/// the exact `AuditRow` shape is already pinned by the `/audit` client type,
/// so the UMP panel just renders `rows.len()` + the JSON).
#[derive(Debug, Clone, Deserialize)]
pub struct UmpAudit {
    #[serde(default)]
    pub rows: Vec<serde_json::Value>,
    pub count: usize,
}
/// `GET /domains` (domains.rs) — the registry.
#[derive(Debug, Deserialize)]
pub struct DomainsResponse {
    #[serde(default)]
    pub domains: Vec<DomainInfo>,
}
/// v1.21.0 "Profiles": a preset bundle as the client renders it. Every knob
/// optional — absent = "this profile doesn't touch that knob".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfileView {
    pub name: String,
    pub description: Option<String>,
    pub default_access_scope: Option<String>,
    pub pii_mode: Option<String>,
    pub retention: Option<std::collections::BTreeMap<String, Option<i64>>>,
    pub audit_level: Option<String>,
    pub kinds: Option<Vec<String>>,
    pub connectors_allowed: Option<Vec<String>>,
    pub legal_hold_default: Option<bool>,
}
impl ProfileView {
    /// The one-line retention summary the wizard + Health panel render
    /// ("episodic=90d, fact=no-decay"; empty policy = "no decay").
    pub fn retention_label(&self) -> Option<String> {
        let map = self.retention.as_ref()?;
        if map.is_empty() {
            return Some("no decay".to_string());
        }
        let parts: Vec<String> = map
            .iter()
            .map(|(k, v)| {
                format!(
                    "{k}={}",
                    v.map(|d| format!("{d}d"))
                        .unwrap_or_else(|| "no-decay".into())
                )
            })
            .collect();
        Some(parts.join(", "))
    }
    pub fn from_value(v: &serde_json::Value) -> Self {
        let g = |k: &str| v[k].as_str().map(|s| s.to_string());
        Self {
            name: g("name").unwrap_or_default(),
            description: g("description"),
            default_access_scope: g("default_access_scope"),
            pii_mode: g("pii_mode"),
            retention: v["retention"]
                .as_object()
                .map(|o| o.iter().map(|(k, val)| (k.clone(), val.as_i64())).collect()),
            audit_level: g("audit_level"),
            kinds: v["kinds"].as_array().map(|a| {
                a.iter()
                    .filter_map(|k| k.as_str().map(String::from))
                    .collect()
            }),
            connectors_allowed: v["connectors_allowed"].as_array().map(|a| {
                a.iter()
                    .filter_map(|k| k.as_str().map(String::from))
                    .collect()
            }),
            legal_hold_default: v["legal_hold_default"].as_bool(),
        }
    }
}
/// v1.21.0: the domain binding + effective knobs (Health panel + wizard
/// eligibility). `profile` is null when unbound (server defaults).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DomainProfile {
    pub domain: String,
    pub profile: Option<String>,
    pub knobs: Option<ProfileView>,
    /// What recall will apply for the domain (null = the server-wide policy
    /// governs — the profile has no retention block).
    pub effective_retention: Option<std::collections::BTreeMap<String, i64>>,
}
impl DomainProfile {
    pub fn from_value(v: &serde_json::Value) -> Self {
        Self {
            domain: v["domain"].as_str().unwrap_or_default().to_string(),
            profile: v["profile"].as_str().map(|s| s.to_string()),
            knobs: if v["knobs"].is_object() {
                Some(ProfileView::from_value(&v["knobs"]))
            } else {
                None
            },
            effective_retention: v["effective"]["retention_days"].as_object().map(|o| {
                o.iter()
                    .filter_map(|(k, val)| val.as_i64().map(|d| (k.clone(), d)))
                    .collect()
            }),
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
pub struct DomainInfo {
    pub name: String,
    pub entries: i64,
    pub entities: i64,
    pub relations: i64,
    #[serde(default)]
    pub multi_db: bool,
}
/// `GET /connectors` (connectors.rs) — the ledger rows.
#[derive(Debug, Deserialize)]
pub struct ConnectorsResponse {
    #[serde(default)]
    pub connectors: Vec<ConnectorRow>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConnectorRow {
    pub id: i64,
    pub kind: String,
    pub instance: String,
    pub state: String,
    #[serde(default)]
    pub last_sync_at: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
}
/// v1.27.11 "Console": `GET /clients` — one BPO register row. Every field
/// optional so an older server body (or a redacted auditor view) still parses
/// cleanly; the client renders only the fields present.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClientRow {
    pub name: String,
    pub domain: String,
    pub jurisdiction: String,
    pub status: String,
    pub profile: Option<String>,
    pub created_at: Option<i64>,
    pub archived_at: Option<i64>,
}
impl ClientRow {
    /// Tolerant read of one `/clients` row. Unknown/absent fields degrade to
    /// ""/None so the dashboard still renders against a partial register.
    pub fn from_value(v: &serde_json::Value) -> Option<Self> {
        let name = v.get("name")?.as_str()?.to_string();
        Some(ClientRow {
            name,
            domain: v
                .get("domain")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            jurisdiction: v
                .get("jurisdiction")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            status: v
                .get("status")
                .and_then(|x| x.as_str())
                .unwrap_or("active")
                .to_string(),
            profile: v.get("profile").and_then(|x| x.as_str()).map(String::from),
            created_at: v.get("created_at").and_then(|x| x.as_i64()),
            archived_at: v.get("archived_at").and_then(|x| x.as_i64()),
        })
    }
}
/// `POST /reindex` (main.rs) — re-embed counts.
#[derive(Debug, Clone, Deserialize)]
pub struct ReindexResult {
    pub status: String,
    #[serde(default)]
    pub reembedded: usize,
    #[serde(default)]
    pub skipped: usize,
}
/// `POST /sources/reconcile` (sources.rs) — retired sources/chunks + orphans.
#[derive(Debug, Clone, Deserialize)]
pub struct ReconcileResult {
    pub kind: String,
    #[serde(default)]
    pub deleted_sources: usize,
    #[serde(default)]
    pub deleted_chunks: usize,
    #[serde(default)]
    pub orphan_uris: Vec<String>,
}
// --- v1.17.7 M3/M4 — pure cores (wire decode + display, testable) ------------

/// v1.17.7 M3.4: decode the entity response. `None` for the backend's 200
/// `{error}` not-found envelope or any non-entity shape.
pub fn parse_entity(json: &serde_json::Value) -> Option<EntityView> {
    if json.get("error").is_some() {
        return None;
    }
    serde_json::from_value(json.clone()).ok()
}
/// v1.17.7 M3.4: decode the traverse response (pins the field names the UI
/// reads). `None` on any shape drift — the panel then shows the error.
pub fn parse_traverse(json: serde_json::Value) -> Option<TraverseResponse> {
    serde_json::from_value(json).ok()
}
/// v1.17.7 M3.4: render a path chain as faithful `A --rel--> B --rel--> C`
/// text. Intermediate node names are best-effort (the server names only the
/// seed + leaf; an unnamed node falls back to its id) — mirror the v1.7.0
/// explanation contract.
pub fn render_path(p: &PathChain) -> String {
    let mut out = String::new();
    for (i, hop) in p.hops.iter().enumerate() {
        let from = if hop.from.name.is_empty() {
            &hop.from.id
        } else {
            &hop.from.name
        };
        let to = if hop.to.name.is_empty() {
            &hop.to.id
        } else {
            &hop.to.name
        };
        if i == 0 {
            out.push_str(from);
        }
        out.push_str(" --");
        out.push_str(&hop.relation);
        out.push_str("--> ");
        out.push_str(to);
    }
    if out.is_empty() {
        return String::new();
    }
    out
}
/// v1.17.7 M3.3: the `kind` filter input validator. Accepts relation-type
/// identifiers, optionally with a `causes:`-style prefix (a trailing `:`
/// selects the whole subgraph). Rejects anything that isn't a safe token.
pub fn kind_is_valid(kind: &str) -> bool {
    let k = kind.trim();
    !k.is_empty()
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        || (k.ends_with(':')
            && k[..k.len() - 1]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
}
/// v1.17.7 M4.1: reduce the three ingest response shapes to one honest outcome
/// (created / duplicate / error — the review panel's row pattern, no silent
/// failures). Reads the union of the contract keys each endpoint emits:
/// `/ingest` uses `status`; markdown/memory use `success` + an optional
/// `error`/`message` string.
#[derive(Debug, Clone, PartialEq)]
pub enum IngestOutcome {
    Created,
    Duplicate,
    Error(String),
}
pub fn parse_ingest_result(json: &serde_json::Value) -> IngestOutcome {
    let status = json.get("status").and_then(|s| s.as_str()).unwrap_or("");
    match status {
        "created" => IngestOutcome::Created,
        "duplicate" => IngestOutcome::Duplicate,
        _ => {
            let ok = json
                .get("success")
                .and_then(|s| s.as_bool())
                .unwrap_or(false);
            let err = json
                .get("error")
                .or_else(|| json.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("");
            if ok {
                IngestOutcome::Created
            } else if !err.is_empty() || status == "error" {
                IngestOutcome::Error(err.to_string())
            } else {
                IngestOutcome::Error("ingest returned no success signal".into())
            }
        }
    }
}
// --- v1.17.8 M5/M6/M7 — pure cores (Rights, UMP, System wire decode) ----------

/// v1.17.8 M5: reduce the `POST /purge` response to the tombstone count + the
/// reason string. `None` on any shape drift (the panel then shows the error).
pub fn parse_purge_result(json: &serde_json::Value) -> Option<PurgeResult> {
    serde_json::from_value(json.clone()).ok()
}
/// v1.17.8 M5: render the retention policy `BTreeMap` as a sortable Vec of
/// (kind, days) so the editor iterates deterministically (BTreeMap already
/// sorts by key; this is the display shape). Empty map → empty Vec.
pub fn retention_to_edits(policy: &std::collections::BTreeMap<String, i64>) -> Vec<(String, i64)> {
    policy.iter().map(|(k, v)| (k.clone(), *v)).collect()
}
/// v1.17.8 M6: decode one UMP record — the panel reads `{record: …}` (the
/// `/ump/memory/{id}` shape) OR a bare record (the `/ump/recall` result
/// record). Returns the record object or `None` on a non-object shape.
pub fn parse_ump_record(json: &serde_json::Value) -> Option<serde_json::Value> {
    let rec = json.get("record").unwrap_or(json);
    if rec.is_object() {
        Some(rec.clone())
    } else {
        None
    }
}
/// v1.17.8 M6: decode the `POST /ump/recall` §3.2 results envelope into the
/// typed results. `None` on shape drift.
pub fn parse_ump_recall(json: &serde_json::Value) -> Option<UmpRecall> {
    serde_json::from_value(json.clone()).ok()
}
/// v1.17.8 M6: the integrity/conformance badge — L3 when a key is configured,
/// L2 otherwise, "unknown" on any other value. Returns the badge token class +
/// the label, both i18n-free (the panel renders them; `t()` wraps the label).
pub fn ump_integrity_badge(conformance: &str) -> (&'static str, String) {
    let class = match conformance {
        "L3" => "badge-ok",
        "L2" => "badge-warn",
        _ => "badge-neutral",
    };
    (class, format!("UMP 1.0 · {conformance}"))
}
/// v1.17.8 M7.3: render a request as the wire line the console displays/sends —
/// `METHOD /path` + the (optional) JSON body pretty-printed. The token is NEVER
/// embedded; the caller attaches it as the bearer separately. Returns the line
/// for the history log + the display pane.
pub fn serialize_request(method: &str, path: &str, body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        format!("{method} {path}")
    } else {
        let pretty = serde_json::from_str::<serde_json::Value>(trimmed)
            .ok()
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
            .unwrap_or_else(|| trimmed.to_string());
        format!("{method} {path}\n{pretty}")
    }
}
/// v1.17.8 M7.3: strip PII-shaped values from a request body before it lands in
/// the persisted console history (localStorage is non-secret). Replaces values
/// under obvious secret-ish keys with `"[redacted]"`; the shape survives so the
/// operator sees what was sent without the secret. Never persisted at all if
/// the whole body is secret (single top-level secret key).
pub fn redact_for_history(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return String::new(); // not JSON → nothing persisted
    };
    const SECRET_KEYS: &[&str] = &[
        "token",
        "auth",
        "refresh_token",
        "access_token",
        "password",
        "secret",
        "key",
        "refresh_token",
    ];
    if let serde_json::Value::Object(map) = &mut v {
        for (k, val) in map.iter_mut() {
            let lk = k.to_ascii_lowercase();
            if SECRET_KEYS.iter().any(|s| lk.contains(s)) {
                *val = serde_json::json!("[redacted]");
            }
        }
    }
    serde_json::to_string(&v).unwrap_or_default()
}
/// v1.18.1 M1: a console history line + whether it is safe to persist.
/// `secret` lines are held in-memory only — never written to localStorage.
#[derive(Clone)]
pub struct StoredLine {
    pub text: String,
    pub secret: bool,
}
/// v1.18.1 M1: a line is `secret` when its request body was non-JSON.
/// `redact_for_history` returns "" for non-JSON (the body is dropped from the
/// line) but an opaque body is token-like — we can't prove the request is
/// clean, so the whole line stays in-memory. JSON bodies are redacted and safe.
pub fn line_is_secret(body: &str) -> bool {
    let b = body.trim();
    !b.is_empty() && serde_json::from_str::<serde_json::Value>(b).is_err()
}
/// v1.18.1 M1: the console-history subset safe to persist. Drops `secret` and
/// empty lines, keeps the last `cap` (newest), returns them in display order.
/// The `redact_for_history` output was already applied before the line existed;
/// this is the second gate (per-entry secret flag) before anything hits disk.
pub fn persist_history(entries: Vec<StoredLine>, cap: usize) -> Vec<String> {
    let clean: Vec<String> = entries
        .into_iter()
        .filter(|l| !l.secret && !l.text.trim().is_empty())
        .map(|l| l.text)
        .collect();
    let n = clean.len();
    clean.into_iter().skip(n.saturating_sub(cap)).collect()
}
#[cfg(test)]
mod tests {
    use super::*;

    /// v1.20.8 M3: the alert `data:` line parses kind+seq; malformed lines and
    /// anything missing `kind`/`seq` are dropped (content never parsed).
    #[test]
    fn alert_event_parses_kind_and_seq_only() {
        let e = parse_alert_event(
            r#"{"kind":"pending","ts":1700000000,"seq":3,"payload":{"proposal_id":1}}"#,
        )
        .unwrap();
        assert_eq!(e.kind, "pending");
        assert_eq!(e.seq, 3);
        assert!(parse_alert_event(r#"{"kind":"expiry","seq":4}"#).is_some());
        assert!(parse_alert_event("not-json").is_none());
        assert!(parse_alert_event(r#"{"kind":"pending"}"#).is_none()); // no seq
        assert!(parse_alert_event(r#"{"seq":3}"#).is_none()); // no kind
    }
    /// Wire-contract pin: representative JSON for every shape the panels read,
    /// deserialized through the same types the client uses. Mirrors openapi.yaml
    /// and the backend handlers (source of truth). Fails if a rename or field
    /// type drifts from what the server actually emits.
    #[test]
    fn health_with_capacity_parses() {
        let h: Health = serde_json::from_str(
            r#"{
                "status":"ok","version":"1.15.0",
                "capacity":{"docs":430,"max_docs":1000000,"rss_mib":120,"max_rss_mib":256,"status":"ok"}
            }"#,
        )
        .unwrap();
        assert_eq!(h.status, "ok");
        assert_eq!(h.version, "1.15.0");
        let c = h.capacity.unwrap();
        assert_eq!(c.docs, 430);
        assert_eq!(c.status, "ok");
        assert!(h.hardening.is_none());
    }
    #[test]
    fn health_capacity_optional() {
        // capacity is omitted when the pool was momentarily exhausted
        let h: Health = serde_json::from_str(r#"{"status":"ok","version":"1.15.0"}"#).unwrap();
        assert!(h.capacity.is_none());
    }
    #[test]
    fn recall_parses_hits_and_decision() {
        let r: RecallResponse = serde_json::from_str(
            r#"{
                "hits":[{
                    "id":1,"title":"note","content":"body",
                    "score":0.9,"domain":"global","source":"manual",
                    "conflict":false,
                    "provenance":{"vector_rank":0,"fts_rank":1,"fused_score":0.5},
                    "assertion_kind":"observed","confidence":0.8,
                    "relevance":"high","decayed":false
                }],
                "decision":"ok",
                "domains_searched":["global"],
                "trace_id":42
            }"#,
        )
        .unwrap();
        assert_eq!(r.decision, "ok");
        assert_eq!(r.trace_id, Some(42));
        assert_eq!(r.hits.len(), 1);
        let h = &r.hits[0];
        assert_eq!(h.id, 1);
        assert_eq!(h.title.as_deref(), Some("note"));
        assert_eq!(h.provenance.as_ref().unwrap().vector_rank, Some(0));
        assert_eq!(h.assertion_kind.as_deref(), Some("observed"));
        assert_eq!(h.confidence, Some(0.8));
        assert_eq!(h.relevance.as_deref(), Some("high"));
        assert_eq!(h.decayed, Some(false));
    }
    /// v1.16.0 M4: a server response missing the v1.14 fields still parses
    /// (every new field is `#[serde(default)]` → backward-safe).
    #[test]
    fn recall_hit_parses_without_v1_14_fields() {
        let h: Hit = serde_json::from_str(r#"{"id":2,"content":"c","score":0.1}"#).unwrap();
        assert_eq!(h.id, 2);
        assert!(h.assertion_kind.is_none());
        assert!(h.relevance.is_none());
    }
    #[test]
    fn proposals_is_a_bare_vec() {
        let v: Vec<Proposal> = serde_json::from_str(
            r#"[{
                "id":7,"kind":"new","content":"c",
                "novelty":0.8,"conflict_with":null,"salience":0.6,
                "created_at":1750000000
            }]"#,
        )
        .unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, 7);
        assert_eq!(v[0].novelty, 0.8);
        assert!(v[0].conflict_with.is_none());
    }
    #[test]
    fn audit_and_quarantine_parse() {
        let a: AuditResponse = serde_json::from_str(
            r#"{"events":[{
                "id":1,"ts":"2026-08-08T00:00:00Z","kind":"Recall","actor":"cli",
                "target_hash":"abc","status":"ok"
            }]}"#,
        )
        .unwrap();
        assert_eq!(a.events[0].kind, "Recall");
        assert_eq!(a.events[0].status, "ok");

        let q: QuarantineResponse =
            serde_json::from_str(r#"{"quarantined":[{"id":3,"title":"x"}],"count":1}"#).unwrap();
        assert_eq!(q.count, 1);
        assert_eq!(q.quarantined[0].id, 3);
    }
    #[test]
    fn dsar_certificate_and_stats_parse() {
        // v1.16.0 M5: the typed card fields are pulled up via `from_value` so
        // the certificate card reads typed values, not raw JSON indexing.
        let v: serde_json::Value = serde_json::from_str(
            r#"{
                "certificate":{
                    "subject":"u","action":"purge","found_count":2,
                    "purged_ids":[1,2],"tombstone_root":1,
                    "chain_head":"abc","certified_at":"2026-08-08T00:00:00Z"
                },
                "chain_verifies":true
            }"#,
        )
        .unwrap();
        let d = DsarCertificate::from_value(v);
        assert!(d.chain_verifies);
        assert_eq!(d.found_count, 2);
        assert_eq!(d.purged_ids, vec![1, 2]);
        assert_eq!(d.tombstone_root, Some(1));
        assert_eq!(d.chain_head, "abc");
        assert_eq!(d.certified_at, "2026-08-08T00:00:00Z");

        let s: Stats = serde_json::from_str(
            r#"{"count":5,"embeddings":5,"entities":2,"relationships":3,
               "model":"m","version":"1.15.0"}"#,
        )
        .unwrap();
        assert_eq!(s.count, 5);
        assert_eq!(s.version, "1.15.0");
    }
    /// v1.16.0 M5: an older server missing fields still renders (defaults).
    #[test]
    fn dsar_certificate_defaults_when_fields_absent() {
        let d = DsarCertificate::from_value(serde_json::json!({
            "certificate": {},
            "chain_verifies": false
        }));
        assert!(!d.chain_verifies);
        assert_eq!(d.found_count, 0);
        assert!(d.purged_ids.is_empty());
        assert!(d.tombstone_root.is_none());
    }
    #[test]
    fn propose_and_approve_shapes_parse() {
        let p: ProposalResponse = serde_json::from_str(
            r#"{"id":9,"status":"pending","novelty":0.7,"conflict_with":2,"salience":0.5}"#,
        )
        .unwrap();
        assert_eq!(p.status, "pending");
        assert_eq!(p.conflict_with, Some(2));

        let a: ApproveResult = serde_json::from_str(
            r#"{"proposal_id":9,"chunk_id":10,"status":"approved","superseded":2}"#,
        )
        .unwrap();
        assert_eq!(a.chunk_id, 10);
        assert_eq!(a.superseded, Some(2));
    }
    /// v1.16.0 M3: the reject reason is percent-encoded into the query string
    /// so a space / `&` / `#` in the reason can't break the URL.
    #[test]
    fn url_encode_reserved_chars() {
        assert_eq!(url_encode("plain"), "plain");
        assert_eq!(url_encode("a b"), "a%20b");
        assert_eq!(url_encode("a&b=c"), "a%26b%3Dc");
        assert_eq!(url_encode("a#b"), "a%23b");
    }
    /// v1.27.11 "Console": the client mirror of the server
    /// `client_authorized_domains` filter — never yields the wildcard or the
    /// operator `global` root, and lowercases so a mixed-case operator grant
    /// still matches the lowercase register (congruent with the server).
    #[test]
    fn scope_client_domains_is_deny_by_default_and_never_global() {
        // Full grant list: concrete + wildcard + global + mixed-case.
        let g = scope_client_domains(Some(
            "read:ops/acme-us read:ops/* admin:ops/global admin:ops/BETA-EU failed",
        ));
        assert_eq!(g, vec!["acme-us".to_string(), "beta-eu".to_string()]);
        // Only wildcard/global → nothing (min-necessary wedge never widens).
        assert!(scope_client_domains(Some("admin:*/* read:ops/global")).is_empty());
        // Absent/malformed scope → nothing (deny-by-default).
        assert!(scope_client_domains(None).is_empty());
        assert!(scope_client_domains(Some("")).is_empty());
    }
    /// v1.16.0 M2.1: principal accessor + is_configured.
    #[test]
    fn api_client_principal_and_configured() {
        let unconfigured = ApiClient::new("", None);
        assert!(!unconfigured.is_configured());
        assert!(unconfigured.principal().is_none());

        let remote = ApiClient::with_principal(
            "http://h",
            Some("t".to_string()),
            Some("user:alice".to_string()),
        );
        assert!(remote.is_configured());
        assert_eq!(remote.principal(), Some("user:alice"));
    }
    /// v1.16.2 "Harden" M4.2: operator-facing error messages map status codes
    /// to actionable hints and never leak the token (it's not in the error).
    #[test]
    fn error_message_maps_status_codes_to_actionable_hints() {
        // Status arms map to actionable hints.
        assert!(error_message(&ApiError::Status(401, "x".into())).contains("authentication failed"));
        assert!(error_message(&ApiError::Status(403, "x".into())).contains("permission denied"));
        assert!(error_message(&ApiError::Status(404, "missing".into())).contains("missing"));
        assert!(error_message(&ApiError::Status(429, "x".into())).contains("rate limited"));
        assert!(error_message(&ApiError::Status(503, "x".into())).contains("unhealthy"));
        assert!(error_message(&ApiError::Status(500, "boom".into())).contains("error 500"));
        // The Network arm is a fixed hint (constructing a reqwest::Error in a
        // test isn't possible — its constructor is pub(crate)); the hint text
        // is pinned by the Display arm, which is exercised above via the fallback.
    }
    // --- v1.16.5 "Secure" tests --------------------------------------------

    /// v1.16.5 M5.2: the pure pre-emptive refresh check — near expiry → true,
    /// far expiry → false, opaque (no exp) → false.
    #[test]
    fn needs_refresh_fires_only_near_expiry() {
        // exp = now + 30s (< 60) → refresh.
        let soon = TokenClaims {
            sub: Some("user:a".into()),
            exp: Some(now_unix() + 30),
            scope: None,
            team: None,
            roles: vec![],
        };
        assert!(needs_refresh(Some(&soon)));
        // exp far out → no refresh.
        let far = TokenClaims {
            sub: None,
            exp: Some(now_unix() + 3600),
            scope: None,
            team: None,
            roles: vec![],
        };
        assert!(!needs_refresh(Some(&far)));
        // No exp (opaque token) → never refresh.
        assert!(!needs_refresh(None));
        assert!(!needs_refresh(Some(&TokenClaims {
            sub: None,
            exp: None,
            scope: None,
            team: None,
            roles: vec![],
        })));
    }
    /// v1.16.5 M1.2: a real JWT payload decodes to its `sub`/`exp`. The header
    /// and signature segments are arbitrary — the client reads only the payload.
    #[test]
    fn decode_claims_roundtrips_jwt_payload() {
        let claims = TokenClaims {
            sub: Some("user:alice".into()),
            exp: Some(1750000000),
            scope: Some("read:global".into()),
            team: Some("alpha".into()),
            roles: vec!["dpo".into()],
        };
        let payload = serde_json::to_string(&claims).unwrap();
        let b64 = base64url_encode(payload.as_bytes());
        // header.ignore + payload + sig.ignore — a valid 3-part JWT shape.
        let token = format!("e30.{b64}.e30");
        let decoded = decode_claims(&token).unwrap();
        assert_eq!(decoded.sub.as_deref(), Some("user:alice"));
        assert_eq!(decoded.exp, Some(1750000000));
        assert_eq!(decoded.team.as_deref(), Some("alpha"));
    }
    /// v1.16.5 M1.2: malformed input is `None`, never a panic — a forged/garbage
    /// JWT falls through to the display fallback, and brain-server rejects it.
    #[test]
    fn decode_claims_rejects_malformed_input() {
        assert!(decode_claims("").is_none());
        assert!(decode_claims("not-a-jwt").is_none());
        assert!(decode_claims("a.b").is_none()); // 2 parts, not 3
        assert!(decode_claims("a.b.c").is_none()); // bad base64url
        assert!(decode_claims("e30.!!!.e30").is_none()); // invalid chars
                                                         // A valid-shaped JWT whose payload isn't JSON → None.
        let b64 = base64url_encode(b"not json");
        assert!(decode_claims(&format!("x.{b64}.x")).is_none());
    }
    /// v1.16.5 M1.2: base64url round-trip (the JWT alphabet + unpadded).
    #[test]
    fn base64url_encode_decode_roundtrip() {
        for s in ["", "a", "ab", "abc", "abcd", "hello world", "user:alice"] {
            let enc = base64url_encode(s.as_bytes());
            assert!(
                !enc.contains('+') && !enc.contains('/'),
                "JWT alphabet only"
            );
            assert_eq!(base64url_decode(&enc).unwrap(), s.as_bytes());
        }
        // Decode rejects length≡1 mod 4 (can't encode whole bytes).
        assert!(base64url_decode("A").is_none());
    }
    /// v1.16.5 M1.4: the JWT-pair constructor derives the principal from the
    /// access token's `sub`; opaque mode keeps `None`.
    #[test]
    fn with_refresh_pair_derives_principal_from_sub() {
        let claims = TokenClaims {
            sub: Some("user:carol".into()),
            exp: Some(now_unix() + 300),
            scope: None,
            team: None,
            roles: vec![],
        };
        let payload = base64url_encode(serde_json::to_string(&claims).unwrap().as_bytes());
        let jwt = format!("e30.{payload}.e30");
        let c = ApiClient::with_refresh_pair("http://h", Some(jwt.clone()), Some("rt".to_string()));
        assert_eq!(c.principal(), Some("user:carol"));
        assert!(c.roles().is_empty(), "no roles claim → empty");

        // A JWT without a sub still shows something for the identity pillar.
        let no_sub = TokenClaims {
            sub: None,
            exp: None,
            scope: None,
            team: None,
            roles: vec![],
        };
        let p2 = base64url_encode(serde_json::to_string(&no_sub).unwrap().as_bytes());
        let c2 = ApiClient::with_refresh_pair("http://h", Some(format!("e30.{p2}.e30")), None);
        assert_eq!(c2.principal(), Some("token (no sub)"));

        // Opaque (non-JWT) access token → principal None (loopback).
        let c3 = ApiClient::with_refresh_pair("http://h", Some("opaque-token".into()), None);
        assert!(c3.principal().is_none());
    }
    // --- v1.17.6 M2 — Overview wire-contract pins ----------------------------

    /// `GET /snapshot/status` — the `.bak` integrity envelope.
    #[test]
    fn snapshot_status_parses() {
        let s: SnapshotStatus = serde_json::from_str(
            r#"{
                "db":"/tmp/brain.db","snapshot_count":1,"all_ok":true,
                "snapshots":[{
                    "file":"brain.db.snapshot-1.bak","exists":true,"size_bytes":4096,
                    "mode_0600":true,"integrity_check":true,"audit_chain_ok":true,"ok":true
                }]
            }"#,
        )
        .unwrap();
        assert_eq!(s.snapshot_count, 1);
        assert!(s.all_ok);
        assert!(s.snapshots[0].mode_0600 && s.snapshots[0].ok);
    }
    /// `GET /retention` — enabled + policy/counts maps.
    #[test]
    fn retention_status_parses() {
        let r: RetentionStatus = serde_json::from_str(
            r#"{"enabled":true,"policy":{"fact":365},"counts":{"fact":10},"projection":"x"}"#,
        )
        .unwrap();
        assert!(r.enabled);
        assert_eq!(r.policy.get("fact"), Some(&365));
        assert_eq!(r.counts.get("fact"), Some(&10));
    }
    /// `GET /ump/capabilities` — the §3.1 handshake.
    #[test]
    fn ump_capabilities_parses() {
        let u: UmpCapabilities = serde_json::from_str(
            r#"{
                "server":{"name":"brain-server","version":"1.17.5"},
                "ump":"1.0","conformance":"L3",
                "kinds":["semantic","episodic"],"bindings":["http","mcp","file"],
                "retrieval_signals":["similarity","recency"],
                "max_recall":50,"writable":true,"audit":true
            }"#,
        )
        .unwrap();
        assert_eq!(u.server.name, "brain-server");
        assert_eq!(u.conformance, "L3");
        assert!(u.writable && u.audit);
    }
    /// `GET /decayed` — a bare Vec of expired rows.
    #[test]
    fn decayed_parses_bare_vec() {
        let v: Vec<DecayedRow> = serde_json::from_str(
            r#"[{
                "id":1,"content_hash":"h1","expires_at":1000,"effective_expiry":900,
                "memory_kind":"fact","reason":"per-kind default"
            }]"#,
        )
        .unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].memory_kind, "fact");
        assert_eq!(v[0].effective_expiry, Some(900));
    }
    /// `POST /consolidate/propose` — detection counts.
    #[test]
    fn consolidate_propose_parses_counts() {
        let p: ConsolidateProposal = serde_json::from_str(
            r#"{
                "exact_duplicates":[[1,2]],
                "conflicts":[{"from_chunk":1}],
                "unresolved_contradictions":[[3,4]],
                "stale_sources":[{"source_id":5}],
                "near_duplicates":[{"chunk_a":6,"chunk_b":7}]
            }"#,
        )
        .unwrap();
        assert_eq!(p.exact_duplicates, vec![vec![1, 2]]);
        assert_eq!(p.unresolved_contradictions, vec![(3, 4)]);
        assert_eq!(p.conflicts.len(), 1);
        assert_eq!(p.stale_sources.len(), 1);
        assert_eq!(p.near_duplicates.len(), 1);
    }
    /// `GET /tombstones` — the deletion-registry page envelope.
    #[test]
    fn tombstones_parse() {
        let t: TombstonesResponse = serde_json::from_str(
            r#"{"tombstones":[{
                "knowledge_id":9,"content_hash":"h","purged_at":1000,
                "reason":"owner:u","origin_id":8
            }]}"#,
        )
        .unwrap();
        assert_eq!(t.tombstones.len(), 1);
        assert_eq!(t.tombstones[0].reason.as_deref(), Some("owner:u"));
        assert_eq!(t.tombstones[0].origin_id, Some(8));
    }
    // --- v1.17.7 M3/M4 — Graph + Create wire pins ----------------------------

    /// `GET /graph/entity` — a real entity (spaces allowed) + typed relations.
    /// A missing entity is a 200 `{error}` → `parse_entity` → `None`.
    #[test]
    fn entity_parses_and_missing_entity_is_none() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{
                "name":"acme","type":"company",
                "relations":[
                    {"to_entity":"dave","relation_type":"employs","direction":"out"},
                    {"to_entity":"acme","relation_type":"ceo_of","direction":"in"}
                ]
            }"#,
        )
        .unwrap();
        let e = parse_entity(&v).unwrap();
        assert_eq!(e.name, "acme");
        assert_eq!(e.entity_type, "company");
        assert_eq!(e.relations.len(), 2);
        assert_eq!(e.relations[0].other, "dave");
        assert_eq!(e.relations[0].relation_type, "employs");
        assert_eq!(e.relations[0].dir, "out");

        let missing: serde_json::Value =
            serde_json::from_str(r#"{"error":"Entity not found"}"#).unwrap();
        assert!(parse_entity(&missing).is_none());
    }
    /// `GET /graph/traverse` — the flat rows + the structured chains, pinned to
    /// the real server field names (`from_entity` = seed, `path` = id->id,
    /// `edge_path` = rel|rel).
    #[test]
    fn traverse_parses_flat_rows_and_paths() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{
                "traversal":[
                    {"entity":"carol","depth":2,"path":"1->2->3","edge_path":"employs|ceo_of",
                     "from_entity":"dave","domain":"global"}
                ],
                "visited":1,
                "paths":[{
                    "hops":[
                        {"from":{"id":"1","name":"dave"},"relation":"employs","to":{"id":"2","name":""}},
                        {"from":{"id":"2","name":""},"relation":"ceo_of","to":{"id":"3","name":"carol"}}
                    ],
                    "depth":2,"domain":"global"
                }]
            }"#,
        )
        .unwrap();
        let t = parse_traverse(v).unwrap();
        assert_eq!(t.visited, 1);
        assert_eq!(t.traversal[0].entity, "carol");
        assert_eq!(t.traversal[0].depth, 2);
        assert_eq!(t.traversal[0].path, "1->2->3");
        assert_eq!(t.traversal[0].edge_path, "employs|ceo_of");
        assert_eq!(t.traversal[0].from_entity.as_deref(), Some("dave"));
        assert_eq!(t.paths.len(), 1);
        assert_eq!(t.paths[0].hops.len(), 2);
        assert_eq!(t.paths[0].hops[0].from.name, "dave");
        assert_eq!(t.paths[0].hops[0].relation, "employs");
    }
    /// v1.17.7 M3.4: the chain text renderer — empty hops → empty; multi-hop
    /// renders `A --rel--> B --rel--> C`; unnamed intermediates fall back to id.
    #[test]
    fn render_path_renders_faithful_chains() {
        let empty = PathChain {
            hops: vec![],
            depth: None,
            domain: None,
        };
        assert_eq!(render_path(&empty), "");

        let named = PathChain {
            hops: vec![
                Hop {
                    from: HopNode {
                        id: "1".into(),
                        name: "dave".into(),
                    },
                    relation: "employs".into(),
                    to: HopNode {
                        id: "2".into(),
                        name: String::new(),
                    },
                },
                Hop {
                    from: HopNode {
                        id: "2".into(),
                        name: String::new(),
                    },
                    relation: "ceo_of".into(),
                    to: HopNode {
                        id: "3".into(),
                        name: "carol".into(),
                    },
                },
            ],
            depth: Some(2),
            domain: Some("global".into()),
        };
        assert_eq!(render_path(&named), "dave --employs--> 2 --ceo_of--> carol");
    }
    /// v1.17.7 M3.3: the `kind` filter validator — exact tokens + `causes:`
    /// prefixes pass; empty/space/invalid chars fail.
    #[test]
    fn kind_validator_accepts_exact_and_prefix() {
        assert!(kind_is_valid("works_at"));
        assert!(kind_is_valid("causes:"));
        assert!(kind_is_valid("a_b-c"));
        assert!(!kind_is_valid(""));
        assert!(!kind_is_valid("  "));
        assert!(!kind_is_valid("has space"));
        assert!(!kind_is_valid("rel'"));
    }
    /// v1.17.7 M4.1: the ingest outcome reducer — `/ingest` status, markdown
    /// `success`, memory `error`, and the no-signal fallback.
    #[test]
    fn ingest_result_reduces_all_three_shapes() {
        let created: serde_json::Value =
            serde_json::json!({"id":3,"status":"created","domain":"global"});
        assert_eq!(parse_ingest_result(&created), IngestOutcome::Created);
        let dup: serde_json::Value = serde_json::json!({"id":3,"status":"duplicate"});
        assert_eq!(parse_ingest_result(&dup), IngestOutcome::Duplicate);
        let md: serde_json::Value = serde_json::json!({"success":true,"chunks_inserted":2});
        assert_eq!(parse_ingest_result(&md), IngestOutcome::Created);
        let mem_err: serde_json::Value =
            serde_json::json!({"success":false,"status":"error","error":"boom"});
        assert_eq!(
            parse_ingest_result(&mem_err),
            IngestOutcome::Error("boom".into())
        );
        let none: serde_json::Value = serde_json::json!({});
        assert_eq!(
            parse_ingest_result(&none),
            IngestOutcome::Error("ingest returned no success signal".into())
        );
    }
    /// v1.17.7 M4: procedure/classify/decision/apply/undo response pins.
    #[test]
    fn create_wire_types_parse() {
        let pr: ProcedureResponse =
            serde_json::from_str(r#"{"id":5,"status":"created","step_ids":[6,7]}"#).unwrap();
        assert_eq!(pr.id, 5);
        assert_eq!(pr.step_ids, vec![6, 7]);

        let sv: ProcedureStepsResponse = serde_json::from_str(
            r#"{"procedure_id":5,"title":"P","content":"c",
               "steps":[{"step_index":0,"id":6,"title":"S","content":"s","memory_kind":"step"}]}"#,
        )
        .unwrap();
        assert_eq!(sv.steps[0].step_index, 0);
        assert_eq!(sv.steps[0].memory_kind, "step");

        let cr: ClassifyResponse = serde_json::from_str(
            r#"{"result":{"category":"compliance","confidence":0.9,"matched_keywords":["hipaa","pii"]},
                "categories":["compliance","general"]}"#,
        )
        .unwrap();
        assert_eq!(cr.result.category, "compliance");
        assert_eq!(cr.result.matched_keywords, vec!["hipaa", "pii"]);

        let d: DecisionOutcome = serde_json::from_str(
            r#"{"result":"escalate","matched_condition":"employee_count >= 50","citation":9,"used_default":false}"#,
        )
        .unwrap();
        assert_eq!(d.result, "escalate");
        assert_eq!(d.citation, Some(9));
        assert!(!d.used_default);

        let a: ApplyResponse =
            serde_json::from_str(r#"{"recorded":1,"rejected":["self link"]}"#).unwrap();
        assert_eq!(a.recorded, 1);
        assert_eq!(a.rejected, vec!["self link"]);

        let u: UndoResponse = serde_json::from_str(r#"{"undone":1,"rejected":[]}"#).unwrap();
        assert_eq!(u.undone, 1);
    }
    #[test]
    fn decision_vars_parse_leniently() {
        let map = crate::panels::procedures::parse_decision_vars(r#"{"revenue":1200,"s":"x"}"#);
        assert_eq!(map.get("revenue"), Some(&1200.0));
        assert!(!map.contains_key("s"));
        assert!(crate::panels::procedures::parse_decision_vars("not json").is_empty());
    }
    // --- v1.17.8 M5/M6/M7 — Rights, UMP, System pure cores --------------------

    #[test]
    fn purge_result_parses() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"purged":3,"reason":"operator purge"}"#).unwrap();
        let r = parse_purge_result(&v).unwrap();
        assert_eq!(r.purged, 3);
        assert_eq!(r.reason, "operator purge");
    }
    #[test]
    fn retention_to_edits_sorts_by_kind() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("fact".into(), 90);
        m.insert("procedure".into(), 30);
        let edits = retention_to_edits(&m);
        assert_eq!(
            edits,
            vec![("fact".to_string(), 90), ("procedure".to_string(), 30)]
        );
        assert!(retention_to_edits(&std::collections::BTreeMap::new()).is_empty());
    }
    #[test]
    fn ump_record_parses_wrapped_and_bare() {
        let wrapped: serde_json::Value =
            serde_json::from_str(r#"{"record":{"id":"urn:ump:1","name":"x"}}"#).unwrap();
        assert_eq!(parse_ump_record(&wrapped).unwrap()["name"], "x");
        let bare: serde_json::Value =
            serde_json::from_str(r#"{"id":"urn:ump:1","name":"y"}"#).unwrap();
        assert_eq!(parse_ump_record(&bare).unwrap()["id"], "urn:ump:1");
        assert!(parse_ump_record(&serde_json::json!(42)).is_none());
    }
    #[test]
    fn ump_recall_parses_results_envelope() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"results":[{"record":{"id":"urn:ump:1"},"signals":{"kind":"fact"},"score":0.9}]}"#,
        )
        .unwrap();
        let r = parse_ump_recall(&v).unwrap();
        assert_eq!(r.results.len(), 1);
        assert_eq!(r.results[0].score, 0.9);
        assert_eq!(r.results[0].record["id"], "urn:ump:1");
    }
    #[test]
    fn ump_integrity_badge_maps_levels() {
        let (c3, l3) = ump_integrity_badge("L3");
        assert_eq!(c3, "badge-ok");
        assert_eq!(l3, "UMP 1.0 · L3");
        let (c2, _) = ump_integrity_badge("L2");
        assert_eq!(c2, "badge-warn");
        let (cn, ln) = ump_integrity_badge("X9");
        assert_eq!(cn, "badge-neutral");
        assert_eq!(ln, "UMP 1.0 · X9");
    }
    #[test]
    fn serialize_request_handles_json_and_plain() {
        let with_body = serialize_request("POST", "/ump/remember", r#"{"content":"x"}"#);
        assert!(with_body.starts_with("POST /ump/remember\n"));
        assert!(with_body.contains("\"content\": \"x\""));
        let no_body = serialize_request("GET", "/ump/capabilities", "");
        assert_eq!(no_body, "GET /ump/capabilities");
    }
    #[test]
    fn redact_for_history_strips_secrets_preserves_shape() {
        let red = redact_for_history(r#"{"content":"x","token":"abc","ok":true}"#);
        assert!(!red.contains("abc"));
        assert!(red.contains("\"[redacted]\""));
        assert!(red.contains("\"ok\":true"));
        assert_eq!(redact_for_history("not json"), "");
        assert_eq!(redact_for_history(""), "");
    }
    /// v1.18.1 M1: an opaque (non-JSON) body marks the line secret so it never
    /// persists; JSON bodies are redactable and persistable.
    #[test]
    fn line_is_secret_for_opaque_non_json_bodies() {
        assert!(line_is_secret(" raw-token-value "));
        assert!(line_is_secret("plain text body"));
        assert!(!line_is_secret(r#"{"content":"x"}"#));
        assert!(!line_is_secret(""));
        assert!(!line_is_secret("  "));
    }
    /// v1.18.1 M1: `persist_history` drops secret + empty lines and caps to the
    /// newest N in display order.
    #[test]
    fn persist_history_drops_secret_lines_and_caps() {
        let lines = vec![
            StoredLine {
                text: "GET /domains".into(),
                secret: false,
            },
            StoredLine {
                text: "POST /x\nopaque".into(),
                secret: true,
            },
            StoredLine {
                text: String::new(),
                secret: false,
            },
            StoredLine {
                text: "POST /y".into(),
                secret: false,
            },
            StoredLine {
                text: "POST /z".into(),
                secret: false,
            },
        ];
        assert_eq!(
            persist_history(lines, 2),
            vec!["POST /y".to_string(), "POST /z".to_string()]
        );
        assert_eq!(persist_history(Vec::new(), 100), Vec::<String>::new());
    }
    /// v1.20.21 M2.1: a dry-run response's `footprint` object parses into the
    /// typed counts + dry_run flag; an absent/non-footprint shape is `None`.
    #[test]
    fn parse_footprint_reads_counts_and_dry_run_flag() {
        let v = serde_json::json!({
            "id": 0,
            "subject": "alice@example.com",
            "status": "preview",
            "certificate": serde_json::Value::Null,
            "footprint": {
                "roots": 3,
                "derived": 1,
                "export_rows": 4,
                "tombstones": 2,
                "dsar_rows": 1,
                "dry_run": true
            }
        });
        let fp = parse_footprint(&v).unwrap();
        assert_eq!(fp.roots, 3);
        assert_eq!(fp.derived, 1);
        assert_eq!(fp.export_rows, 4);
        assert_eq!(fp.tombstones, 2);
        assert_eq!(fp.dsar_rows, 1);
        assert!(fp.dry_run);
        // No footprint object → None (a live response or shape drift).
        assert!(parse_footprint(&serde_json::json!({ "id": 1 })).is_none());
    }
    /// v1.20.21 M2.1: the dry-run wire carries `dry_run: true`; a live DSAR
    /// leaves it absent. Proves the request body builder is dry-run-safe (the
    /// panic-risk here — a preview silently purging — is worth a real pin).
    #[test]
    fn dsar_preview_request_carries_dry_run_true() {
        let body = dsar_preview_body("alice@example.com");
        assert_eq!(body["subject"], "alice@example.com");
        assert_eq!(body["action"], "both");
        assert_eq!(body["dry_run"], serde_json::json!(true));
        // The live builder's body has no dry_run key.
        let live = serde_json::json!({ "subject": "a", "action": "both" });
        assert!(live.get("dry_run").is_none());
    }
    /// v1.20.22 M2.1: the `/dsar` ledger page parses with a missing
    /// `completed_at` (an open request) — `#[serde(default)]` timestamps mean
    /// an absent clock field never panics the wire decode.
    #[test]
    fn dsar_ledger_parse_defaults_absent_timestamps() {
        let v: DsarLedger = serde_json::from_str(
            r#"{"total":2,"requests":[
                {"id":3,"subject":"new@x","action":"purge","status":"completed","created_at":3000,"completed_at":3001},
                {"id":2,"subject":"open@x","action":"both","status":"pending","created_at":2000,"deadline":2000}
            ]}"#,
        )
        .unwrap();
        assert_eq!(v.total, 2);
        assert_eq!(v.requests.len(), 2);
        // The open row has no `completed_at` key — it defaults to None, no panic.
        let open = &v.requests[1];
        assert_eq!(open.id, 2);
        assert_eq!(open.created_at, Some(2000));
        assert_eq!(open.deadline, Some(2000));
        assert_eq!(open.completed_at, None);
        assert_eq!(open.status, "pending");
        // A fully-populated row keeps both timestamps.
        assert_eq!(v.requests[0].completed_at, Some(3001));
    }
    /// v1.21.0 "Profiles": the wizard pick list parses (12 seeded presets,
    /// each with its description + knobs) and the retention label renders
    /// null-as-no-decay.
    #[test]
    fn profiles_parse_and_retention_label_handles_nulls() {
        let body = serde_json::json!({
            "profiles": [
                {"name": "health-hipaa",
                 "description": "Health/care posture",
                 "default_access_scope": "private",
                 "pii_mode": "strict",
                 "retention": {"fact": null, "episodic": 90},
                 "audit_level": "verbose",
                 "kinds": ["fact", "episodic"],
                 "connectors_allowed": ["ehr-readonly"],
                 "legal_hold_default": false},
                {"name": "smb-simple", "retention": {}}
            ]
        });
        let list: Vec<crate::api::ProfileView> = body["profiles"]
            .as_array()
            .map(|a| a.iter().map(crate::api::ProfileView::from_value).collect())
            .unwrap_or_default();
        assert_eq!(list.len(), 2);
        let hipaa = &list[0];
        assert_eq!(hipaa.pii_mode.as_deref(), Some("strict"));
        assert_eq!(hipaa.default_access_scope.as_deref(), Some("private"));
        assert_eq!(
            hipaa.retention.as_ref().unwrap().get("fact"),
            Some(&None),
            "explicit null retention = no decay"
        );
        assert_eq!(
            hipaa.retention_label().as_deref(),
            Some("episodic=90d, fact=no-decay")
        );
        // Empty policy = nothing decays.
        assert_eq!(list[1].retention_label().as_deref(), Some("no decay"));
        // Absent block = None (the server-wide policy governs).
        assert_eq!(list[1].pii_mode, None);
    }
    /// v1.21.0: the domain binding view parses both shapes — bound (knobs +
    /// effective retention) and unbound (null profile + null knobs, the
    /// pre-v1.21 posture the Health panel must render explicitly).
    #[test]
    fn domain_profile_parses_bound_and_unbound() {
        let bound = serde_json::json!({
            "domain": "global",
            "profile": "call-center",
            "knobs": {
                "default_access_scope": "private",
                "pii_mode": "standard",
                "retention": {"episodic": 90},
                "audit_level": "verbose",
                "kinds": ["fact", "episodic", "procedure"],
                "connectors_allowed": ["crm"],
                "legal_hold_default": false
            },
            "effective": {"retention_days": {"episodic": 90}}
        });
        let dp = crate::api::DomainProfile::from_value(&bound);
        assert_eq!(dp.domain, "global");
        assert_eq!(dp.profile.as_deref(), Some("call-center"));
        let knobs = dp.knobs.as_ref().expect("knobs present");
        assert_eq!(knobs.audit_level.as_deref(), Some("verbose"));
        assert_eq!(
            dp.effective_retention.as_ref().unwrap().get("episodic"),
            Some(&90)
        );

        let unbound = serde_json::json!({
            "domain": "global",
            "profile": null,
            "knobs": null
        });
        let dp2 = crate::api::DomainProfile::from_value(&unbound);
        assert_eq!(dp2.profile, None);
        assert_eq!(dp2.knobs, None);
        assert_eq!(dp2.effective_retention, None);
    }
}
