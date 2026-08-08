//! Typed brain-server API client. Wraps `reqwest` with the bearer attached.
//! Works unchanged on web (WASM) + desktop + mobile — Dioxus abstracts fetch.
//!
//! The client holds NO memory cache: every call hits the backend (the source of
//! truth) and panels drive re-fetch via `use_resource` signal subscription.

use serde::Deserialize;

const MIN_QUERY: usize = 5; // mirrors brain-server's min_query_length

#[derive(Clone)]
pub struct ApiClient {
    base: String,
    token: Option<String>,
    http: reqwest::Client,
}

#[derive(Debug)]
pub enum ApiError {
    Network(reqwest::Error),
    Status(u16, String),
}

impl ApiClient {
    pub fn new(base: impl Into<String>, token: Option<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            token,
            http: reqwest::Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, ApiError> {
        let mut req = self.http.get(self.url(path));
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await.map_err(ApiError::Network)?;
        if !resp.status().is_success() {
            let code = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Status(code, body));
        }
        resp.json::<T>().await.map_err(ApiError::Network)
    }

    /// GET /health — the connect-first onboarding probe.
    pub async fn health(&self) -> Result<Health, ApiError> {
        self.get_json("/health").await
    }

    /// POST /recall — the decision-path viewer data. Empty for short queries
    /// (mirrors the backend's min_query_length gate).
    pub async fn recall(&self, query: &str) -> Result<Vec<Hit>, ApiError> {
        if query.trim().len() < MIN_QUERY {
            return Ok(Vec::new());
        }
        let body = serde_json::json!({ "query": query, "k": 8 });
        let mut req = self.http.post(self.url("/recall")).json(&body);
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await.map_err(ApiError::Network)?;
        if !resp.status().is_success() {
            let code = resp.status().as_u16();
            let b = resp.text().await.unwrap_or_default();
            return Err(ApiError::Status(code, b));
        }
        let parsed: RecallResponse = resp.json().await.map_err(ApiError::Network)?;
        Ok(parsed.hits)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct Health {
    pub status: String,
    pub version: String,
    pub capacity: Capacity,
}

#[derive(Debug, Deserialize)]
pub struct Capacity {
    pub docs: u64,
    pub max_docs: u64,
    pub rss_mib: u64,
    pub max_rss_mib: u64,
    pub status: String,
}

#[derive(Debug, Deserialize)]
struct RecallResponse {
    hits: Vec<Hit>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Hit {
    pub id: i64,
    pub content: String,
    pub score: f64,
}
