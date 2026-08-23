//! [`RemoteWorkflowHost`] — the SDK [`WorkflowHost`] seam over the
//! brain-server substrate routes. Each request is atomic server-side; `tx()`
//! maps to a no-compound journal (the single write lane lives in the server).
//!
//! Transport law: loopback plain-HTTP only (or HTTPS anywhere) — the plugin
//! 0.4.7 scheme rule. Bearer token via the extension config ladder:
//! `BRAIN_TOKEN_FILE` -> `BRAIN_TOKEN` -> the default install path.

use brain_engine_sdk::host::tx::{HostTx, HostTxHandle};
use brain_engine_sdk::host::{AuditKind, AuditStatus, CasError, HostError, WorkflowHost};
use reqwest::Client;
use serde_json::Value;

use crate::engine::SteeringReader;

pub const DEFAULT_BRAIN_URL: &str = "http://127.0.0.1:8765";

/// Resolve the base URL (`BRAIN_URL`, default loopback) and REFUSE a
/// non-loopback plain-HTTP target — credentials never ride cleartext off-host.
pub fn resolve_base_url(env_val: Option<String>) -> Result<String, String> {
    let base = env_val.unwrap_or_else(|| DEFAULT_BRAIN_URL.to_string());
    if let Some(rest) = base.strip_prefix("http://") {
        let host = rest.split('/').next().unwrap_or("");
        // strip the port (IPv6 literals keep their brackets as loopback keys)
        let host_only = host.split(':').next().unwrap_or(host);
        let loopback = matches!(host_only, "127.0.0.1" | "localhost" | "[::1]" | "::1");
        if !loopback {
            return Err(format!(
                "refusing non-loopback plain HTTP target '{base}' (use https:// or a loopback address)"
            ));
        }
    } else if !base.starts_with("https://") {
        return Err(format!("unsupported URL scheme in '{base}'"));
    }
    Ok(base.trim_end_matches('/').to_string())
}

/// The bearer-token ladder (mirrors the openclaw plugin config seam):
/// 1. `BRAIN_TOKEN_FILE` — path to a secret file,
/// 2. `BRAIN_TOKEN` — raw env (dev convenience),
/// 3. `~/.config/brain-server/auth-token` — default install path.
pub fn resolve_token() -> Option<String> {
    if let Ok(path) = std::env::var("BRAIN_TOKEN_FILE")
        && let Ok(s) = std::fs::read_to_string(path.trim())
        && !s.trim().is_empty()
    {
        return Some(s.trim().to_string());
    }
    if let Ok(t) = std::env::var("BRAIN_TOKEN")
        && !t.trim().is_empty()
    {
        return Some(t.trim().to_string());
    }
    let home = std::env::var("HOME").ok()?;
    let default_path = format!("{home}/.config/brain-server/auth-token");
    std::fs::read_to_string(default_path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// A journaling tx handle: each remote request is itself atomic, so the
/// local journal documents one-request units and commits as a no-op.
struct JournalHandle;

impl HostTxHandle for JournalHandle {
    fn finish(self: Box<Self>, _commit: bool) -> Result<(), HostError> {
        Ok(())
    }
}

pub struct RemoteWorkflowHost {
    base: String,
    token: Option<String>,
    client: Client,
}

/// Run one async request to completion on its own thread + runtime. The
/// WorkflowHost trait is synchronous by ABI design; this keeps a nested
/// runtime off the caller's executor threads.
fn tokio_block<T: Send>(
    fut: impl std::future::Future<Output = Result<T, HostError>> + Send,
) -> Result<T, HostError> {
    // The WorkflowHost trait is synchronous by ABI design; run each request
    // on its own scoped thread + single-thread runtime so a nested executor
    // never blocks the caller's.
    fn build_rt() -> Result<tokio::runtime::Runtime, HostError> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| HostError::Internal(e.to_string()))
    }
    let mut out: Option<Result<T, HostError>> = None;
    std::thread::scope(|s| -> Result<(), HostError> {
        std::thread::Builder::new()
            .name("harness-remote".into())
            .spawn_scoped(s, || {
                out = Some(build_rt().and_then(|rt| rt.block_on(fut)));
            })
            .map_err(|e| HostError::Internal(e.to_string()))?;
        Ok(())
    })?;
    out.unwrap_or_else(|| {
        Err(HostError::Internal(
            "remote worker terminated unexpectedly".into(),
        ))
    })
}

impl RemoteWorkflowHost {
    pub fn new(base: String, token: Option<String>) -> Result<Self, String> {
        let base = resolve_base_url(Some(base))?;
        Ok(Self {
            base,
            token,
            client: Client::builder()
                .build()
                .map_err(|e| format!("http client: {e}"))?,
        })
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<(u16, Value), HostError> {
        let url = format!("{}{}", self.base, path);
        let mut req = match method {
            "GET" => self.client.get(&url),
            "PUT" => self.client.put(&url),
            _ => self.client.post(&url),
        };
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| HostError::Internal(e.to_string()))?;
        let status = resp.status().as_u16();
        let v = resp.json::<Value>().await.unwrap_or(Value::Null);
        Ok((status, v))
    }
}

fn err_from_status(status: u16, v: &Value) -> HostError {
    match status {
        404 => HostError::NotFound,
        _ => HostError::Internal(format!(
            "server {status}: {}",
            v.get("error").and_then(|e| e.as_str()).unwrap_or("?")
        )),
    }
}

impl WorkflowHost for RemoteWorkflowHost {
    fn tx(&self) -> Result<HostTx, HostError> {
        Ok(HostTx::new(Box::new(JournalHandle)))
    }

    fn enqueue(
        &self,
        run_id: i64,
        topic: &str,
        payload_json: &str,
        idempotency_key: &str,
    ) -> Result<bool, HostError> {
        let path = format!("/workflow/runs/{run_id}/events");
        let fut = self.call(
            "POST",
            &path,
            Some(serde_json::json!({
                "topic": topic,
                "payload_json": payload_json,
                "idempotency_key": idempotency_key,
            })),
        );
        let (status, v) = tokio_block(fut)?;
        if status == 200 {
            Ok(v["first"].as_bool().unwrap_or(true))
        } else {
            Err(err_from_status(status, &v))
        }
    }

    fn cas(&self, run_id: i64, expected_rev: i64, state_json: &str) -> Result<(), CasError> {
        let path = format!("/workflow/runs/{run_id}/state");
        let fut = self.call(
            "PUT",
            &path,
            Some(serde_json::json!({
                "expected_rev": expected_rev,
                "state_json": state_json,
            })),
        );
        let (status, v) = tokio_block(fut).map_err(|e| CasError::Database(e.to_string()))?;
        match status {
            200 => Ok(()),
            409 => Err(CasError::Stale {
                actual_revision: v["actual_revision"].as_i64().unwrap_or(-1),
            }),
            404 => Err(CasError::Gone),
            _ => Err(CasError::Database(format!("server {status}"))),
        }
    }

    fn load_state(&self, run_id: i64) -> Result<Option<(String, i64)>, HostError> {
        let path = format!("/workflow/runs/{run_id}/state");
        let fut = self.call("GET", &path, None);
        let (status, v) = tokio_block(fut)?;
        match status {
            200 => Ok(Some((
                v["state_json"].as_str().unwrap_or("").to_string(),
                v["revision"].as_i64().unwrap_or(0),
            ))),
            404 => Ok(None),
            _ => Err(err_from_status(status, &v)),
        }
    }

    fn audit(
        &self,
        _kind: AuditKind,
        _actor: &str,
        _target: &str,
        _status: AuditStatus,
        _detail: &str,
    ) {
        // Documented ceiling: every durable effect the engine makes (CAS
        // write, outbox event) is ALREADY audited server-side in the same
        // transaction — there is no separate remote audit row to write, so
        // this contract hook stays a deliberate no-op rather than forging a
        // second chain entry.
    }
}

impl SteeringReader for RemoteWorkflowHost {
    fn read_steering(&self, run_id: i64) -> Result<Vec<String>, HostError> {
        let path = format!("/workflow/runs/{run_id}/steering");
        let fut = self.call("GET", &path, None);
        let (status, v) = tokio_block(fut)?;
        if status != 200 {
            return Err(err_from_status(status, &v));
        }
        Ok(v["messages"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m["message"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }
}

impl RemoteWorkflowHost {
    /// `POST /workflow/runs` -> run_id.
    pub async fn open_run(
        &self,
        domain: String,
        kind: String,
        state_json: String,
    ) -> Result<i64, HostError> {
        let (status, v) = self
            .call(
                "POST",
                "/workflow/runs",
                Some(serde_json::json!({
                    "domain": domain,
                    "kind": kind,
                    "state_json": state_json,
                })),
            )
            .await?;
        if status == 200 || status == 201 {
            Ok(v["run_id"].as_i64().unwrap_or(0))
        } else {
            Err(err_from_status(status, &v))
        }
    }

    /// `POST /workflow/runs/{id}/answer` -> new revision.
    pub async fn answer(&self, run_id: i64, body: Value) -> Result<i64, HostError> {
        let (status, v) = self
            .call(
                "POST",
                &format!("/workflow/runs/{run_id}/answer"),
                Some(body),
            )
            .await?;
        if status == 200 {
            Ok(v["revision"].as_i64().unwrap_or(0))
        } else {
            Err(err_from_status(status, &v))
        }
    }
}
