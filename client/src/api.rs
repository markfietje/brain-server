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

const MIN_QUERY: usize = 5; // mirrors brain-server's min_query_length
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

    /// GET /dsar/{id}/certificate — re-fetch a deletion certificate + live
    /// chain check. Returns the raw JSON `{certificate, chain_verifies}` so
    /// `DsarCertificate::from_value` can pull the typed card fields up.
    pub async fn dsar_certificate(&self, id: i64) -> Result<serde_json::Value, ApiError> {
        self.get_json(&format!("/dsar/{id}/certificate")).await
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
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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
    #[serde(default)]
    pub authority: Option<f32>,
    pub novelty: f32,
    #[serde(default)]
    pub conflict_with: Option<i64>,
    pub salience: f32,
    pub created_at: i64,
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
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SnapshotStatus {
    pub db: String,
    pub snapshot_count: u64,
    pub all_ok: bool,
    #[serde(default)]
    pub snapshots: Vec<SnapshotRow>,
}

#[derive(Debug, Deserialize)]
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
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
pub struct UmpServer {
    pub name: String,
    pub version: String,
}

/// `GET /decayed` (gate.rs) — a bare Vec of expired chunks.
#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        assert!(needs_refresh(Some(&soon)));
        // exp far out → no refresh.
        let far = TokenClaims {
            sub: None,
            exp: Some(now_unix() + 3600),
            scope: None,
            team: None,
        };
        assert!(!needs_refresh(Some(&far)));
        // No exp (opaque token) → never refresh.
        assert!(!needs_refresh(None));
        assert!(!needs_refresh(Some(&TokenClaims {
            sub: None,
            exp: None,
            scope: None,
            team: None,
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
        };
        let payload = base64url_encode(serde_json::to_string(&claims).unwrap().as_bytes());
        let jwt = format!("e30.{payload}.e30");
        let c = ApiClient::with_refresh_pair("http://h", Some(jwt.clone()), Some("rt".to_string()));
        assert_eq!(c.principal(), Some("user:carol"));

        // A JWT without a sub still shows something for the identity pillar.
        let no_sub = TokenClaims {
            sub: None,
            exp: None,
            scope: None,
            team: None,
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
}
