// Truthful allow (workflow-substrate precedent): scaffold lands one release ahead of its UI consumer.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcEnvelope {
    pub rpc_id: String,
    pub kind: String,
    pub payload: serde_json::Value,
}

pub fn validate_envelope(v: &serde_json::Value) -> Option<RpcEnvelope> {
    let rpc_id = v.get("rpcId")?.as_str()?.to_string();
    if rpc_id.is_empty() || rpc_id.len() > 64 {
        return None;
    }
    let kind = v.get("kind")?.as_str()?.to_string();
    if kind.is_empty()
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

pub trait ApiProxy: Send + Sync {
    fn handle(&self, env: RpcEnvelope) -> Result<serde_json::Value, String>;
}

pub struct InProcessCarrier<P: ApiProxy> {
    pub inner: P,
}

impl<P: ApiProxy> InProcessCarrier<P> {
    pub fn call(&self, payload: serde_json::Value) -> Result<serde_json::Value, String> {
        let env = validate_envelope(&payload).ok_or_else(|| "invalid envelope".to_string())?;
        self.inner.handle(env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Echo;
    impl ApiProxy for Echo {
        fn handle(&self, env: RpcEnvelope) -> Result<serde_json::Value, String> {
            Ok(env.payload)
        }
    }
    #[test]
    fn envelope_validation() {
        assert!(
            validate_envelope(&serde_json::json!({"rpcId":"1","kind":"session/event"})).is_some()
        );
        assert!(validate_envelope(&serde_json::json!({"rpcId":"","kind":"x"})).is_none());
        assert!(validate_envelope(&serde_json::json!({"kind":"x"})).is_none());
    }
    #[test]
    fn in_process_carrier() {
        let c = InProcessCarrier { inner: Echo };
        let r = c
            .call(serde_json::json!({"rpcId":"a","kind":"health","payload":{"ok":true}}))
            .unwrap();
        assert_eq!(r, serde_json::json!({"ok":true}));
    }
}
