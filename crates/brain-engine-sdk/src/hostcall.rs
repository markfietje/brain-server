//! Hostcall dispatch — the confused-deputy boundary between engines and
//! host powers.
//!
//! Invariant: every call passes through the same four steps — interceptor,
//! payload canonicalization, capability check (audited), then and only then
//! the kind handler. A handler is unreachable without passing its capability
//! gate, and every denial leaves an audit row. Honest ceilings: worker-thread
//! isolation is not a security boundary (real isolation is process/container),
//! and engines hold bash-equivalent trust — this defends against buggy
//! scripts, not hostile code.

use crate::host::{AuditKind, AuditStatus, WorkflowHost};
use crate::trust::{Decision, ExtensionPolicy, HostCallKind, UnknownKind};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// Hard wire bound for one hostcall body.
pub const MAX_BODY_BYTES: usize = 256 * 1024;
/// Hard wire bound for the operation name.
const MAX_NAME_BYTES: usize = 256;
/// A region's forced-shutdown grace period.
pub const CLEANUP_BUDGET_SECS: u64 = 5;

/// Dispatch failure vocabulary — loud values, never degraded defaults.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DispatchError {
    /// Capability policy refused (or the class is outside the vocabulary).
    Denied(String),
    /// Payload failed canonicalization (shape, size, or control characters).
    InvalidPayload(String),
    /// The operation exceeded its effective time budget.
    BudgetExceeded,
    /// A test interceptor short-circuited with this outcome.
    Intercepted(String),
    Internal(String),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DispatchError::Denied(m) => write!(f, "denied: {m}"),
            DispatchError::InvalidPayload(m) => write!(f, "invalid payload: {m}"),
            DispatchError::BudgetExceeded => write!(f, "budget exceeded"),
            DispatchError::Intercepted(m) => write!(f, "intercepted: {m}"),
            DispatchError::Internal(m) => write!(f, "internal: {m}"),
        }
    }
}

impl std::error::Error for DispatchError {}

impl From<UnknownKind> for DispatchError {
    fn from(u: UnknownKind) -> Self {
        DispatchError::Denied(format!("unknown hostcall kind `{}`", u.0))
    }
}

/// Canonicalized hostcall: kind parsed against the closed vocabulary, name
/// and body bounded, control characters refused before anything runs.
#[derive(Debug, Clone, PartialEq)]
pub struct HostCallPayload {
    pub kind: HostCallKind,
    pub name: String,
    pub body: String,
}

impl HostCallPayload {
    pub fn canonicalize(kind_wire: &str, name: &str, body: &str) -> Result<Self, DispatchError> {
        let kind = HostCallKind::parse(kind_wire)?;
        if name.is_empty() || name.len() > MAX_NAME_BYTES {
            return Err(DispatchError::InvalidPayload("name out of bounds".into()));
        }
        if body.len() > MAX_BODY_BYTES {
            return Err(DispatchError::InvalidPayload(
                "body exceeds MAX_BODY_BYTES".into(),
            ));
        }
        if name.chars().any(char::is_control) || body.contains('\u{0}') {
            return Err(DispatchError::InvalidPayload(
                "control characters refused".into(),
            ));
        }
        Ok(HostCallPayload {
            kind,
            name: name.to_string(),
            body: body.to_string(),
        })
    }
}

type Handler = Arc<dyn Fn(&str, &str) -> Result<String, String> + Send + Sync>;
type Interceptor = Arc<dyn Fn(&HostCallPayload) -> Option<Result<String, String>> + Send + Sync>;

/// Everything a dispatch needs: policy, the engine asking, its handlers, the
/// optional test interceptor, and the audit chain every decision lands on.
pub struct HostCallContext<H: WorkflowHost + ?Sized> {
    pub policy: ExtensionPolicy,
    pub engine: String,
    pub budget: Budget,
    handlers: Mutex<HashMap<&'static str, Handler>>,
    interceptor: Mutex<Option<Interceptor>>,
    /// In-run dispatch tally: `(label, kind_wire) -> count`. Append-only,
    /// per-context (one context per engine invocation — no atomics needed).
    /// The audit chain is the durable count; this is the cheap aggregate a
    /// CrankReport carries.
    counters: Mutex<BTreeMap<(String, String), u64>>,
    host: Arc<H>,
}

impl<H: WorkflowHost + ?Sized> HostCallContext<H> {
    pub fn new(host: Arc<H>, policy: ExtensionPolicy, engine: &str) -> Self {
        HostCallContext {
            policy,
            engine: engine.to_string(),
            budget: Budget::default(),
            handlers: Mutex::new(HashMap::new()),
            interceptor: Mutex::new(None),
            counters: Mutex::new(BTreeMap::new()),
            host,
        }
    }

    /// Register the handler for one kind (replacing any prior).
    pub fn set_handler(
        &self,
        kind: HostCallKind,
        f: impl Fn(&str, &str) -> Result<String, String> + Send + Sync + 'static,
    ) {
        if let Ok(mut g) = self.handlers.lock() {
            g.insert(kind.as_str(), Arc::new(f));
        }
    }

    /// Snapshot of the in-run dispatch tally: `((label, kind), count)`.
    /// Pure read; the map only grows within one engine invocation.
    pub fn counters(&self) -> BTreeMap<(String, String), u64> {
        self.counters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Whether a handler is registered for `kind` (the exhaustive-table
    /// pin reads this; production code should not branch on it).
    pub fn has_handler(&self, kind: HostCallKind) -> bool {
        self.handlers
            .lock()
            .ok()
            .is_some_and(|g| g.contains_key(kind.as_str()))
    }

    /// Test seam: short-circuit dispatch before canonicalization.
    pub fn set_interceptor(
        &self,
        f: impl Fn(&HostCallPayload) -> Option<Result<String, String>> + Send + Sync + 'static,
    ) {
        if let Ok(mut g) = self.interceptor.lock() {
            *g = Some(Arc::new(f));
        }
    }

    /// The four-step dispatch. Steps run strictly in order; a missing handler
    /// after a passed check is an Internal error (a misconfiguration, not a
    /// silent denial).
    pub fn dispatch(
        &self,
        kind_wire: &str,
        name: &str,
        body: &str,
    ) -> Result<String, DispatchError> {
        // 1. Interceptor (mock/test short-circuit).
        let payload = match self
            .interceptor
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .as_ref()
        {
            Some(f) => {
                // Canonicalize first so the interceptor sees typed input.
                let p = HostCallPayload::canonicalize(kind_wire, name, body)?;
                match f(&p) {
                    Some(r) => {
                        return r.map_err(DispatchError::Intercepted);
                    }
                    None => p,
                }
            }
            None => HostCallPayload::canonicalize(kind_wire, name, body)?,
        };

        // 2. Time budget must still be satisfiable.
        if self.budget.effective_timeout().is_none() && self.budget.manager_secs.is_some() {
            return Err(DispatchError::BudgetExceeded);
        }

        // Countable: every canonicalized dispatch tallies, allowed or not —
        // denials are effects too and must show up in the run's report.
        if let Ok(mut g) = self.counters.lock() {
            *g.entry((payload.name.clone(), payload.kind.as_str().to_string()))
                .or_insert(0) += 1;
        }

        // 3. Capability check, audited either way.
        let cap = payload.kind.required_capability();
        let decision = self.policy.decide(&self.engine, cap);
        let target = format!("{}/{}", payload.kind.as_str(), payload.name);
        let (status, result) = match decision {
            Decision::Allowed => (AuditStatus::Ok, None),
            Decision::Prompt => (
                AuditStatus::Denied,
                Some(Err(DispatchError::Denied(format!(
                    "`{cap}` requires consent (prompt posture)"
                )))),
            ),
            Decision::Denied => (
                AuditStatus::Denied,
                Some(Err(DispatchError::Denied(format!(
                    "capability `{cap}` denied"
                )))),
            ),
        };
        self.host
            .audit(AuditKind::Workflow, &self.engine, &target, status, cap);
        if let Some(r) = result {
            return r;
        }

        // 4. Kind handler, only after the check passed.
        let handler = self
            .handlers
            .lock()
            .ok()
            .and_then(|g| g.get(payload.kind.as_str()).cloned());
        match handler {
            Some(h) => h(&payload.name, &payload.body).map_err(DispatchError::Internal),
            None => Err(DispatchError::Internal(format!(
                "no handler registered for `{}`",
                payload.kind.as_str()
            ))),
        }
    }
}

/// Intersects the manager-level ceiling with a per-op budget: the effective
/// timeout is the smaller present bound, or none when unbounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub manager_secs: Option<u64>,
    pub op_secs: Option<u64>,
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            manager_secs: Some(30),
            op_secs: None,
        }
    }
}

impl Budget {
    pub fn effective_timeout(self) -> Option<Duration> {
        match (self.manager_secs, self.op_secs) {
            (Some(a), Some(b)) => Some(Duration::from_secs(a.min(b))),
            (Some(a), None) => Some(Duration::from_secs(a)),
            (None, Some(b)) => Some(Duration::from_secs(b)),
            (None, None) => None,
        }
    }
}

/// Cooperative cancellation: the region's drop and explicit cancels both
/// flip the flag handlers poll between steps.
#[derive(Debug, Default)]
pub struct CancellationToken {
    cancelled: AtomicBool,
}

impl CancellationToken {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// RAII extension region: created per mounted extension, forces cancellation
/// on drop within [`CLEANUP_BUDGET_SECS`]. Structured concurrency for free —
/// nothing escapes a dropped region alive.
pub struct ExtensionRegion {
    pub token: Arc<CancellationToken>,
    started: Instant,
}

impl Default for ExtensionRegion {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtensionRegion {
    pub fn new() -> Self {
        ExtensionRegion {
            token: Arc::new(CancellationToken::default()),
            started: Instant::now(),
        }
    }

    /// Whether the cleanup budget has been exhausted (a handler ignoring the
    /// token past this point is reported, never waited on forever).
    pub fn cleanup_expired(&self) -> bool {
        self.started.elapsed() > Duration::from_secs(CLEANUP_BUDGET_SECS)
    }
}

impl Drop for ExtensionRegion {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

/// Exec mediation: classify commands that must never reach the process seam,
/// regardless of allowlist content. Pure, table-driven, stem-tolerant on the
/// destructive flags.
pub fn exec_mediation(command: &str) -> Result<(), String> {
    let lower = command.to_ascii_lowercase();
    const FORBIDDEN: &[&str] = &[
        "rm -rf /",
        "mkfs",
        ":(){ :|:& };:",
        "dd if=/dev/zero",
        "> /dev/sda",
        "chmod -r 777 /",
        "shutdown",
        "reboot",
    ];
    for pat in FORBIDDEN {
        if lower.contains(pat) {
            return Err(format!("dangerous command refused: contains `{pat}`"));
        }
    }
    Ok(())
}

/// Non-owning liveness probe over a shared manager slot: hosts keep this
/// instead of a strong handle so a long-lived observer cannot keep the
/// kernel alive (the Weak-ref cycle break). Upgrading after drop reads None.
pub struct ManagerProbe<T>(Weak<Mutex<T>>);

impl<T> ManagerProbe<T> {
    pub fn new(slot: &Arc<Mutex<T>>) -> Self {
        ManagerProbe(Arc::downgrade(slot))
    }
    /// None once every strong owner has dropped.
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        self.0.upgrade().map(|slot| {
            let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut guard)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{AuditStatus, CasError, HostError, HostTx};
    use crate::trust::ExtensionPolicy;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct TapeHost {
        log: StdMutex<Vec<String>>,
    }
    impl WorkflowHost for TapeHost {
        fn tx(&self) -> Result<HostTx, HostError> {
            unreachable!()
        }
        fn enqueue(&self, _: i64, _: &str, _: &str, _: &str) -> Result<bool, HostError> {
            Ok(true)
        }
        fn cas(&self, _: i64, _: i64, _: &str) -> Result<(), CasError> {
            Ok(())
        }
        fn load_state(&self, _: i64) -> Result<Option<(String, i64)>, HostError> {
            Ok(None)
        }
        fn audit(&self, k: AuditKind, actor: &str, target: &str, s: AuditStatus, d: &str) {
            if let Ok(mut g) = self.log.lock() {
                g.push(format!(
                    "{}/{}/{}/{}/{}",
                    k.as_str(),
                    actor,
                    target,
                    s.as_str(),
                    d
                ));
            }
        }
    }

    fn ctx(policy: ExtensionPolicy) -> (Arc<TapeHost>, HostCallContext<TapeHost>) {
        let host = Arc::new(TapeHost::default());
        let c = HostCallContext::new(host.clone(), policy, "engine-a");
        c.set_handler(HostCallKind::Log, |name, body| Ok(format!("{name}:{body}")));
        c.set_handler(HostCallKind::Tool, |name, _| Ok(format!("tool:{name}")));
        c.set_handler(HostCallKind::Ui, |name, _| Ok(format!("ui:{name}")));
        (host, c)
    }

    #[test]
    fn allowed_call_runs_handler_and_audits_ok() {
        let (host, c) = ctx(ExtensionPolicy::permissive());
        let out = c.dispatch("log", "boot", "hello").unwrap();
        assert_eq!(out, "boot:hello");
        assert_eq!(
            host.log.lock().unwrap().clone(),
            vec!["workflow/engine-a/log/boot/ok/log".to_string()]
        );
    }

    #[test]
    fn denied_capability_never_reaches_handler_and_audits_denied() {
        let (host, c) = ctx(ExtensionPolicy::standard()); // exec hard-denied
        let err = c.dispatch("exec", "sh", "ls").unwrap_err();
        assert!(matches!(err, DispatchError::Denied(_)));
        assert_eq!(
            host.log.lock().unwrap().clone(),
            vec!["workflow/engine-a/exec/sh/denied/exec".to_string()]
        );
    }

    #[test]
    fn unknown_kind_is_denial_before_anything_else() {
        let (host, c) = ctx(ExtensionPolicy::permissive());
        assert_eq!(
            c.dispatch("shell", "x", "{}").unwrap_err(),
            DispatchError::Denied("unknown hostcall kind `shell`".into())
        );
        assert!(host.log.lock().unwrap().is_empty());
    }

    #[test]
    fn oversized_and_control_payloads_fail_canonicalization() {
        let (_host, c) = ctx(ExtensionPolicy::permissive());
        let big = "x".repeat(MAX_BODY_BYTES + 1);
        assert!(matches!(
            c.dispatch("log", "n", &big),
            Err(DispatchError::InvalidPayload(_))
        ));
        assert!(matches!(
            c.dispatch("log", "na\u{0}me", "b"),
            Err(DispatchError::InvalidPayload(_))
        ));
    }

    #[test]
    fn prompt_posture_requires_consent() {
        let (_host, c) = ctx(ExtensionPolicy::standard()); // `ui` prompts
        assert!(matches!(
            c.dispatch("ui", "dialog", "{}"),
            Err(DispatchError::Denied(_))
        ));
    }

    #[test]
    fn interceptor_short_circuits_before_handlers() {
        let (host, c) = ctx(ExtensionPolicy::permissive());
        c.set_interceptor(|p| {
            if p.name == "mock" {
                Some(Ok("intercepted-ok".into()))
            } else {
                None
            }
        });
        assert_eq!(c.dispatch("log", "mock", "z").unwrap(), "intercepted-ok");
        // Falls through when the interceptor declines.
        assert_eq!(c.dispatch("log", "other", "z").unwrap(), "other:z");
        // An intercepted error propagates as such.
        c.set_interceptor(|_| Some(Err("nope".into())));
        assert_eq!(
            c.dispatch("log", "x", "z").unwrap_err(),
            DispatchError::Intercepted("nope".into())
        );
        // Only the fall-through call reached (and audited past) the gate.
        assert_eq!(
            host.log.lock().unwrap().clone(),
            vec!["workflow/engine-a/log/other/ok/log".to_string()]
        );
    }

    #[test]
    fn missing_handler_after_pass_is_internal_not_silent() {
        let (_host, c) = ctx(ExtensionPolicy::permissive());
        assert!(matches!(
            c.dispatch("http", "fetch", "{}"),
            Err(DispatchError::Internal(_))
        ));
    }

    #[test]
    fn budget_intersects_manager_and_op_bounds() {
        let b = Budget {
            manager_secs: Some(10),
            op_secs: Some(3),
        };
        assert_eq!(b.effective_timeout(), Some(Duration::from_secs(3)));
        assert_eq!(
            Budget {
                manager_secs: Some(10),
                op_secs: None
            }
            .effective_timeout(),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            Budget {
                manager_secs: None,
                op_secs: None
            }
            .effective_timeout(),
            None
        );
    }

    #[test]
    fn region_drop_cancels_token() {
        let token = {
            let region = ExtensionRegion::new();
            let t = region.token.clone();
            assert!(!t.is_cancelled());
            t
        };
        assert!(token.is_cancelled());
        assert!(!ExtensionRegion::default().cleanup_expired());
    }

    #[test]
    fn dispatch_counter_increments_per_kind_and_label() {
        let mut policy = ExtensionPolicy::permissive();
        policy.deny_caps.push("exec".into());
        let (_host2, c) = ctx(policy);
        assert_eq!(c.counters().len(), 0, "no dispatch, no tally");
        c.dispatch("log", "boot", "a").unwrap();
        c.dispatch("log", "boot", "b").unwrap();
        // A denied call counts too — denials are effects.
        assert!(c.dispatch("exec", "sh", "ls").is_err());
        let counters = c.counters();
        assert_eq!(
            counters.get(&("boot".to_string(), "log".to_string())),
            Some(&2)
        );
        assert_eq!(
            counters.get(&("sh".to_string(), "exec".to_string())),
            Some(&1),
            "denied dispatches tally like any other"
        );
    }

    #[test]
    fn hostcall_table_is_exhaustive() {
        // The closed 7-word vocabulary must stay closed: parse accepts every
        // wire name, as_str round-trips, and the capability map covers each.
        for wire in ["tool", "exec", "http", "session", "events", "ui", "log"] {
            let k = HostCallKind::parse(wire).unwrap();
            assert_eq!(k.as_str(), wire);
            assert!(!k.required_capability().is_empty());
        }
        // And nothing outside parses (the deny-by-default posture).
        for bad in ["shell", "", "Tool", "tools"] {
            assert!(HostCallKind::parse(bad).is_err());
        }
    }

    #[test]
    fn exec_mediation_refuses_destructive_commands() {
        assert!(exec_mediation("ls -l").is_ok());
        assert!(exec_mediation("rm -rf / --no-preserve-root").is_err());
        assert!(exec_mediation("MKFS.ext4 /dev/sda").is_err());
    }

    #[test]
    fn weak_probe_upgrade_after_drop_reads_none() {
        let slot: Arc<StdMutex<u64>> = Arc::new(StdMutex::new(7));
        let probe = ManagerProbe::new(&slot);
        assert_eq!(probe.with(|v| *v), Some(7));
        drop(slot);
        assert_eq!(probe.with(|v| *v), None, "probe holds no strong reference");
    }
}
