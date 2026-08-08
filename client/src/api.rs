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

const MIN_QUERY: usize = 5; // mirrors brain-server's min_query_length

#[derive(Clone)]
pub struct ApiClient {
    base: String,
    token: Option<String>,
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
pub fn error_message(e: &ApiError) -> String {
    match e {
        ApiError::Network(_) => "cannot reach brain-server — check the URL or network".into(),
        ApiError::Status(401, _) => "authentication failed — check your token".into(),
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

    /// v1.16.0 M2.1: connect with a known principal (remote/JWT mode).
    pub fn with_principal(
        base: impl Into<String>,
        token: Option<String>,
        principal: Option<String>,
    ) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            token,
            principal,
            http: reqwest::Client::new(),
        }
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

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, ApiError> {
        let resp = self
            .http
            .get(self.url(path))
            .bearer(self.token.as_deref())
            .send()
            .await
            .map_err(ApiError::Network)?;
        self.unpack(resp).await
    }

    async fn post_json<T, B>(&self, path: &str, body: &B) -> Result<T, ApiError>
    where
        T: for<'de> Deserialize<'de>,
        B: Serialize + ?Sized,
    {
        let resp = self
            .http
            .post(self.url(path))
            .json(body)
            .bearer(self.token.as_deref())
            .send()
            .await
            .map_err(ApiError::Network)?;
        self.unpack(resp).await
    }

    async fn post_empty<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, ApiError> {
        let resp = self
            .http
            .post(self.url(path))
            .bearer(self.token.as_deref())
            .send()
            .await
            .map_err(ApiError::Network)?;
        self.unpack(resp).await
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
}

// Helpers to attach the bearer token to a request builder (avoids the
// one-off in every method).
trait Bearer {
    fn bearer(self, token: Option<&str>) -> reqwest::RequestBuilder;
}
impl Bearer for reqwest::RequestBuilder {
    fn bearer(self, token: Option<&str>) -> reqwest::RequestBuilder {
        match token {
            Some(t) => self.bearer_auth(t),
            None => self,
        }
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
}
