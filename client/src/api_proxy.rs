//! The api-proxy seam: the browser holds no capability, it only subscribes.
//!
//! Wire contract: a bidirectional envelope `{rpcId, kind, payload}` validated
//! in two layers — envelope shape first, then a per-kind payload schema. Every
//! violation is a typed [`HostError`], never a panic.
//!
//! Carriers:
//! - [`InProcessCarrier`] — test/deterministic fixture path (`host.handler.fetch`
//!   injection point; no port, no network).
//! - [`WebFetchCarrier`] — browser uplink on same origin via `fetch` (the
//!   request/response quadrant); SSE/WebSocket downlink stays server-pushed
//!   (`session/event` + `agent/*`) and is consumed by the conversation layer.
//!
//! Port note: semantics ported from the dual-process harness UI pattern
//! (apiproxy fetch carriers); implementation is original Rust.
//!
//! Truthful allow: this module is the envelope CONTRACT — the wasm shell still
//! speaks it through `ApiClient` (the web carrier), and the server-side proxy
//! handler consumes the same shape; nothing in the client tree calls these
//! items directly yet. Re-evaluated when a second carrier lands.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcEnvelope {
    pub rpc_id: String,
    pub kind: String,
    pub payload: serde_json::Value,
}

/// Typed proxy failure. `Envelope` = layer-1 violation, `Payload` = layer-2,
/// `Handler` = the proxied call itself failed (status/message preserved).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HostError {
    Envelope(String),
    Payload { kind: String, reason: String },
    Handler { status: u16, message: String },
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Envelope(e) => write!(f, "envelope invalid: {e}"),
            Self::Payload { kind, reason } => write!(f, "payload invalid for {kind}: {reason}"),
            Self::Handler { status, message } => {
                write!(f, "handler error {status}: {message}")
            }
        }
    }
}

impl std::error::Error for HostError {}

/// Layer 1: envelope shape. Bounded ids/kinds so a hostile payload cannot blow
/// up downstream formatting or smuggle kind strings past the dispatcher.
pub fn validate_envelope(v: &serde_json::Value) -> Option<RpcEnvelope> {
    let rpc_id = v.get("rpcId")?.as_str()?.to_string();
    if rpc_id.is_empty() || rpc_id.len() > 64 {
        return None;
    }
    let kind = v.get("kind")?.as_str()?.to_string();
    if kind.is_empty()
        || kind.len() > 128
        || kind.contains(|c: char| !c.is_ascii_alphanumeric() && c != '/' && c != '.' && c != '-')
    {
        return None;
    }
    Some(RpcEnvelope {
        rpc_id,
        kind,
        payload: v.get("payload").cloned().unwrap_or(serde_json::Value::Null),
    })
}

/// Layer 2: per-kind payload validation at the boundary. Unknown kinds are
/// denied by default (fail-closed) unless registered as open passthrough.
pub fn validate_payload(kind: &str, payload: &serde_json::Value) -> Result<(), HostError> {
    let bad = |reason: &str| HostError::Payload {
        kind: kind.to_string(),
        reason: reason.to_string(),
    };
    match kind {
        // Read-only queries: an optional bounded string filter.
        "health" | "recall" | "proposals/list" | "audit/verify" => {
            if let Some(q) = payload.get("q") {
                let s = q.as_str().ok_or_else(|| bad("q must be a string"))?;
                if s.len() > 4096 {
                    return Err(bad("q exceeds 4096 bytes"));
                }
            }
            Ok(())
        }
        // Mutations require an id.
        "review/approve" | "review/reject" => payload
            .get("id")
            .and_then(|v| v.as_i64())
            .filter(|i| *i > 0)
            .map(|_| ())
            .ok_or_else(|| bad("id must be a positive integer")),
        _ => Err(bad("unknown kind")),
    }
}

/// The host-side handler map: one entry per proxied kind. The server implements
/// this once over its real handlers; tests inject fixtures.
pub trait ApiProxy: Send + Sync {
    fn handle(&self, env: RpcEnvelope) -> Result<serde_json::Value, HostError>;
}

/// Shared dispatch: validate both layers, then delegate. `rpcId` echoes back in
/// the response wrapper so the caller can correlate (the four-quadrant rule).
pub fn dispatch(
    proxy: &dyn ApiProxy,
    raw: &serde_json::Value,
) -> Result<(String, serde_json::Value), HostError> {
    let env = validate_envelope(raw).ok_or_else(|| HostError::Envelope("malformed".into()))?;
    validate_payload(&env.kind, &env.payload)?;
    let out = proxy.handle(env)?;
    Ok((raw["rpcId"].as_str().unwrap_or_default().to_string(), out))
}

/// Deterministic carrier for tests + headless runs.
pub struct InProcessCarrier<P: ApiProxy> {
    pub inner: P,
}

impl<P: ApiProxy> InProcessCarrier<P> {
    pub fn call(&self, payload: serde_json::Value) -> Result<serde_json::Value, HostError> {
        let (_, v) = dispatch(&self.inner, &payload)?;
        Ok(v)
    }
}

/// Browser uplink: `ApiClient` IS the web fetch carrier (same-origin bearer
/// requests); this module owns the envelope/validation contract it must speak.
/// The [`InProcessCarrier`] covers tests + headless runs without a port.
///
/// Shared response decode: non-2xx → `Handler`, bad JSON → `Envelope`.
pub fn parse_response(status: u16, text: &str) -> Result<serde_json::Value, HostError> {
    if !(200..300).contains(&status) {
        return Err(HostError::Handler {
            status,
            message: text.chars().take(256).collect(),
        });
    }
    serde_json::from_str(text).map_err(|e| HostError::Envelope(format!("response not JSON: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Echo;
    impl ApiProxy for Echo {
        fn handle(&self, env: RpcEnvelope) -> Result<serde_json::Value, HostError> {
            Ok(serde_json::json!({"echo": env.payload, "kind": env.kind}))
        }
    }

    struct Deny;
    impl ApiProxy for Deny {
        fn handle(&self, _: RpcEnvelope) -> Result<serde_json::Value, HostError> {
            Err(HostError::Handler {
                status: 503,
                message: "down".into(),
            })
        }
    }

    #[test]
    fn envelope_validation() {
        assert!(
            validate_envelope(&serde_json::json!({"rpcId":"1","kind":"session/event"})).is_some()
        );
        assert!(validate_envelope(&serde_json::json!({"rpcId":"","kind":"x"})).is_none());
        assert!(validate_envelope(&serde_json::json!({"kind":"x"})).is_none());
        assert!(
            validate_envelope(&serde_json::json!({"rpcId":"1","kind":"bad kind!"})).is_none(),
            "kind charset is closed"
        );
        assert!(
            validate_envelope(&serde_json::json!({"rpcId":"x","kind":"a"})).is_some(),
            "minimal valid envelope"
        );
    }

    #[test]
    fn in_process_carrier_roundtrip() {
        let c = InProcessCarrier { inner: Echo };
        let r = c
            .call(serde_json::json!({"rpcId":"a","kind":"health","payload":{"ok":true}}))
            .unwrap();
        assert_eq!(r["echo"]["ok"], serde_json::json!(true));
    }

    #[test]
    fn unknown_kind_fails_closed_at_layer_two() {
        let c = InProcessCarrier { inner: Echo };
        let err = c
            .call(serde_json::json!({"rpcId":"a","kind":"totally/new"}))
            .unwrap_err();
        assert!(matches!(err, HostError::Payload { .. }), "{err:?}");
    }

    #[test]
    fn approve_requires_positive_id() {
        let good = serde_json::json!({"id": 3});
        assert!(validate_payload("review/approve", &good).is_ok());
        let bad = serde_json::json!({"id": -1});
        assert!(validate_payload("review/approve", &bad).is_err());
    }

    #[test]
    fn handler_errors_are_typed_not_panics() {
        let c = InProcessCarrier { inner: Deny };
        let err = c
            .call(serde_json::json!({"rpcId":"a","kind":"health"}))
            .unwrap_err();
        assert_eq!(
            err,
            HostError::Handler {
                status: 503,
                message: "down".into()
            }
        );
    }

    #[test]
    fn rpc_id_echoes_through_dispatch() {
        let (id, _) = dispatch(
            &Echo,
            &serde_json::json!({"rpcId":"corr-7","kind":"health"}),
        )
        .unwrap();
        assert_eq!(id, "corr-7");
    }

    #[test]
    fn response_decode_is_two_layer() {
        assert!(parse_response(200, "{\"ok\":true}").is_ok());
        assert_eq!(
            parse_response(500, "boom").unwrap_err(),
            HostError::Handler {
                status: 500,
                message: "boom".into()
            }
        );
        assert!(matches!(
            parse_response(200, "not json"),
            Err(HostError::Envelope(_))
        ));
    }
}
