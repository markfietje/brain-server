//! Effects have exactly one door: [`Effects`] wraps the hostcall dispatch.
//!
//! Every engine tool-effect (exec, http egress, event emission, suggestion
//! reads, log emits) serializes into its mediated body shape and rides
//! `HostCallContext::dispatch` — interceptor → canonicalize → audited
//! capability check → handler. Nothing in this module touches a process, a
//! socket, or storage directly. The HTTP transport client stays solely in
//! `remote_host.rs` (host TRANSPORT, authenticated by the agent token — not
//! a tool-effect; the self-grep deliberately does not name it).
//! `engine_has_no_direct_effect_paths` pins that boundary.

use brain_engine_sdk::hostcall::DispatchError;
use std::sync::Arc;

type Dispatcher = Arc<dyn Fn(&str, &str, &str) -> Result<String, DispatchError> + Send + Sync>;

/// The one effect door for an engine invocation. Constructed from the
/// server's assembled `HostCallContext` (its `dispatch` method).
#[derive(Clone)]
pub struct Effects {
    dispatch: Dispatcher,
}

impl Effects {
    pub fn new(
        f: impl Fn(&str, &str, &str) -> Result<String, DispatchError> + Send + Sync + 'static,
    ) -> Self {
        Effects {
            dispatch: Arc::new(f),
        }
    }

    fn call(&self, kind: &str, name: &str, body: &str) -> Result<String, HarnessEffectError> {
        (self.dispatch)(kind, name, body).map_err(HarnessEffectError::Dispatch)
    }

    /// Run an allowlisted command. Denials are loud values, never degraded.
    pub fn exec(&self, run: i64, argv: &[String]) -> Result<String, HarnessEffectError> {
        let body = serde_json::json!({ "argv": argv }).to_string();
        self.call("exec", &run.to_string(), &body)
    }

    /// Fetch from an allowlisted https host (loopback may speak http).
    pub fn http(&self, _run: i64, host: &str, path: &str) -> Result<String, HarnessEffectError> {
        let body = serde_json::json!({ "host": host, "path": path }).to_string();
        self.call("http", "fetch", &body)
    }

    /// Emit an outbox event under `workflow/*` with an idempotency key.
    pub fn event(
        &self,
        run: i64,
        topic: &str,
        payload_json: &str,
        idempotency_key: &str,
    ) -> Result<String, HarnessEffectError> {
        let body = serde_json::json!({
            "topic": topic,
            "payload": payload_json,
            "idempotency_key": idempotency_key,
        })
        .to_string();
        self.call("events", &run.to_string(), &body)
    }

    /// Domain-scoped, sanitized knowledge suggestions for the run.
    pub fn suggest(&self, run: i64, query: &str) -> Result<String, HarnessEffectError> {
        let body = serde_json::json!({ "run_id": run, "query": query }).to_string();
        self.call("tool", "knowledge_suggest", &body)
    }

    /// Structured log emit through the mediated log kind.
    pub fn log(&self, name: &str, line: &str) -> Result<String, HarnessEffectError> {
        self.call("log", name, line)
    }
}

/// An effect refused at any dispatch step. The crank folds this into a
/// finding row — never a silent skip (the gate-rejection posture).
#[derive(Debug)]
pub enum HarnessEffectError {
    Dispatch(DispatchError),
}

impl std::fmt::Display for HarnessEffectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessEffectError::Dispatch(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for HarnessEffectError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The include_str! self-grep (the main.rs house style): the transport
    /// client appears ONLY in remote_host.rs, never in effect/engine modules.
    /// Built at runtime so this test's own source never carries the literal.
    fn needle() -> String {
        format!("{}{}::", "req", "west")
    }

    #[test]
    fn engine_has_no_direct_effect_paths() {
        const FILES: &[(&str, &str)] = &[
            ("effects.rs", include_str!("effects.rs")),
            ("engine.rs", include_str!("engine.rs")),
            ("inmem.rs", include_str!("inmem.rs")),
            ("lib.rs", include_str!("lib.rs")),
            ("main.rs", include_str!("main.rs")),
        ];
        for (name, src) in FILES {
            assert!(
                !src.contains(&needle()) && !src.contains(&needle().replace("::", "")),
                "{name} must not touch transport directly — effects go through the dispatch door"
            );
        }
    }

    /// Every effect serializes into the exact body shape its server-side
    /// mediated handler validates.
    #[test]
    fn effects_serialize_mediated_body_shapes() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
        let tape = seen.clone();
        let fx = Effects::new(move |kind, name, body| {
            if let Ok(mut g) = tape.lock() {
                g.push((format!("{kind}/{name}"), body.to_string()));
            }
            Ok("ok".into())
        });
        fx.exec(7, &["/bin/ls".into(), "-a".into()]).unwrap();
        fx.http(7, "example.com", "/x").unwrap();
        fx.event(7, "workflow/log", "{\"a\":1}", "k-1").unwrap();
        fx.suggest(7, "runbook").unwrap();
        let calls = seen.lock().unwrap().clone();
        assert_eq!(
            calls[0],
            ("exec/7".into(), r#"{"argv":["/bin/ls","-a"]}"#.into())
        );
        assert!(calls[1].0 == "http/fetch" && calls[1].1.contains("example.com"));
        assert!(calls[2].0 == "events/7" && calls[2].1.contains("idempotency_key"));
        assert!(calls[3].0 == "tool/knowledge_suggest" && calls[3].1.contains("\"run_id\":7"));
    }

    /// A dispatch denial surfaces as a loud error value, never a default.
    #[test]
    fn effect_denials_are_loud() {
        let fx = Effects::new(|_, _, _| Err(DispatchError::Denied("capability denied".into())));
        assert!(matches!(
            fx.exec(7, &["/bin/ls".into()]),
            Err(HarnessEffectError::Dispatch(DispatchError::Denied(_)))
        ));
    }
}
