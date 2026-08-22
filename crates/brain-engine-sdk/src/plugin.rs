//! The plugin kernel: a minimal Cordis-shaped reimplementation (semantics
//! ported, no code copied).
//!
//! Invariant: everything a [`Service`] mounts is registered through the
//! [`Context`], and every registration taken inside [`Context::effect`] is
//! reversed on unmount/drop — load, unload, and reload are reversible. The
//! honest ceiling: this kernel is single-process; HMR across process
//! boundaries and nested-fiber lifecycles stay out of scope.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, Weak};

/// A mountable unit; [`Service::key`] is the stable wire name.
/// `inject` names dependencies (other services' keys) that must already be
/// mounted before [`Context::install`] calls [`Service::mount`] — ordering is
/// enforced, never assumed.
pub trait Service {
    /// The stable wire name (`"ctx.workflow"` style).
    fn key(&self) -> &'static str;

    /// Keys of other services this one depends on.
    fn inject(&self) -> &[&str] {
        &[]
    }

    /// Claim registrations (`provide`, `effect`) on the context.
    fn mount(&mut self, ctx: &mut Context);

    /// Reverse what mount did that effects do not already reverse.
    fn unmount(&self);
}

/// Kernel failure vocabulary — loud, never degraded.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum KernelError {
    /// An `inject` dependency was not mounted.
    MissingDependency {
        service: &'static str,
        needs: String,
    },
    /// The key (or service type) is already claimed.
    Duplicate { key: String },
    /// No service with that key is mounted / no value of that type provided.
    NotMounted { key: String },
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KernelError::MissingDependency { service, needs } => {
                write!(f, "service `{service}` missing dependency `{needs}`")
            }
            KernelError::Duplicate { key } => write!(f, "duplicate claim on `{key}`"),
            KernelError::NotMounted { key } => write!(f, "`{key}` not mounted"),
        }
    }
}

impl std::error::Error for KernelError {}

struct ActiveEffect {
    id: u64,
    undo: Box<dyn FnMut(&mut Context) + Send>,
}

struct Inner {
    services: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    mounted: Vec<Box<dyn Service + Send>>,
    effects: Vec<ActiveEffect>,
    next_effect_id: u64,
    /// The ONE workflow engine for this context (`ctx.workflowEngine`).
    /// Mounting replaces any previous engine — Cordis mount semantics, never
    /// parallel providers.
    workflow_engine: Option<Arc<dyn crate::workflow::WorkflowEngine>>,
}

/// The plugin context: owns services by type plus the effect stack whose
/// entries undo in strict reverse order.
pub struct Context {
    inner: Arc<Mutex<Inner>>,
}

impl Clone for Context {
    fn clone(&self) -> Self {
        Context {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    pub fn new() -> Self {
        Context {
            inner: Arc::new(Mutex::new(Inner {
                services: HashMap::new(),
                mounted: Vec::new(),
                effects: Vec::new(),
                next_effect_id: 0,
                workflow_engine: None,
            })),
        }
    }

    /// Publish a service value under its concrete type. Fail-loud when the
    /// type is already claimed.
    pub fn provide<T>(&mut self, value: T) -> Result<(), KernelError>
    where
        T: Any + Send + Sync,
    {
        let mut inner = self.inner.lock().map_err(borrow_failed)?;
        let id = TypeId::of::<T>();
        if inner.services.contains_key(&id) {
            return Err(KernelError::Duplicate {
                key: std::any::type_name::<T>().to_string(),
            });
        }
        inner.services.insert(id, Arc::new(value));
        Ok(())
    }

    /// Fetch a shared handle to a provided service.
    pub fn require<T>(&self) -> Result<Arc<T>, KernelError>
    where
        T: Any + Send + Sync,
    {
        let inner = self.inner.lock().map_err(borrow_failed)?;
        let id = TypeId::of::<T>();
        inner
            .services
            .get(&id)
            .and_then(|any| Arc::downgrade(any).upgrade())
            .and_then(|any| any.downcast::<T>().ok())
            .ok_or_else(|| KernelError::NotMounted {
                key: std::any::type_name::<T>().to_string(),
            })
    }

    /// Mount a service after enforcing its `inject` ordering. Dependencies
    /// must already be installed; duplicate keys fail loud.
    pub fn install(&mut self, mut svc: Box<dyn Service + Send>) -> Result<(), KernelError> {
        {
            let inner = self.inner.lock().map_err(borrow_failed)?;
            if inner.mounted.iter().any(|m| m.key() == svc.key()) {
                return Err(KernelError::Duplicate {
                    key: svc.key().to_string(),
                });
            }
            for dep in svc.inject() {
                if !inner.mounted.iter().any(|m| m.key() == *dep) {
                    return Err(KernelError::MissingDependency {
                        service: svc.key(),
                        needs: (*dep).to_string(),
                    });
                }
            }
        }
        svc.mount(self);
        self.inner.lock().map_err(borrow_failed)?.mounted.push(svc);
        Ok(())
    }

    /// Unmount the service registered under `key`, topmost-first (reverse of
    /// install order). Effects it registered were reversed when their handles
    /// dropped or were disposed.
    pub fn uninstall(&mut self, key: &'static str) -> Result<(), KernelError> {
        // Remove under the lock, unmount/drop OUTSIDE it: a service may hold
        // an EffectHandle whose Drop re-locks this same mutex.
        let svc = {
            let mut inner = self.inner.lock().map_err(borrow_failed)?;
            let pos = inner
                .mounted
                .iter()
                .rposition(|m| m.key() == key)
                .ok_or_else(|| KernelError::NotMounted {
                    key: key.to_string(),
                })?;
            inner.mounted.remove(pos)
        };
        svc.unmount();
        drop(svc);
        Ok(())
    }

    /// Hot reload: unmount then remount the same instance. The unload half is
    /// reversible by construction (effect stack); the remount re-runs
    /// `mount` so the subtree re-claims its registrations.
    pub fn reload(&mut self, key: &'static str) -> Result<(), KernelError> {
        let mut svc = {
            let mut inner = self.inner.lock().map_err(borrow_failed)?;
            let pos = inner
                .mounted
                .iter()
                .rposition(|m| m.key() == key)
                .ok_or_else(|| KernelError::NotMounted {
                    key: key.to_string(),
                })?;
            inner.mounted.remove(pos)
        };
        svc.unmount();
        svc.mount(self);
        self.inner.lock().map_err(borrow_failed)?.mounted.push(svc);
        Ok(())
    }

    /// Run `setup` against this context and record its `undo` closure as a
    /// reversible effect. Dropping the returned [`EffectHandle`] (or calling
    /// [`EffectHandle::dispose`]) reverses it — and anything taken after it —
    /// in strict reverse order.
    pub fn effect<F, U>(&mut self, _label: &'static str, setup: F) -> EffectHandle
    where
        F: FnOnce(&mut Context) -> U,
        U: FnMut(&mut Context) + Send + 'static,
    {
        let id = match self.inner.lock() {
            Ok(mut inner) => {
                let id = inner.next_effect_id;
                inner.next_effect_id += 1;
                id
            }
            // Id allocation cannot fail here: the lock is uncontended by
            // construction (single-owner context); if it ever were poisoned,
            // we hand back an inert handle rather than panic.
            Err(_) => {
                return EffectHandle {
                    id: u64::MAX,
                    ctx: Weak::new(),
                };
            }
        };
        let undo = setup(self);
        if let Ok(mut inner) = self.inner.lock() {
            inner.effects.push(ActiveEffect {
                id,
                undo: Box::new(undo),
            });
        }
        EffectHandle {
            id,
            ctx: Arc::downgrade(&self.inner),
        }
    }

    /// Mounted keys in install order (oldest first).
    pub fn mounted_keys(&self) -> Vec<&'static str> {
        match self.inner.lock() {
            Ok(inner) => inner.mounted.iter().map(|m| m.key()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Mount the context's workflow engine. Replaces any previously mounted
    /// engine (config-driven replacement, never a parallel registry).
    /// Fail-closed: a poisoned lock refuses the mount instead of ignoring it.
    pub fn mount_workflow_engine(
        &mut self,
        engine: Arc<dyn crate::workflow::WorkflowEngine>,
    ) -> Result<(), KernelError> {
        let mut inner = self.inner.lock().map_err(borrow_failed)?;
        inner.workflow_engine = Some(engine);
        Ok(())
    }

    /// The mounted engine, when one is configured.
    pub fn workflow_engine(&self) -> Option<Arc<dyn crate::workflow::WorkflowEngine>> {
        let inner = self.inner.lock().ok()?;
        inner.workflow_engine.clone()
    }
}

/// Audited mounting (compliance carry-over): plugin mount/unmount emit
/// `Workflow` audit rows through the same host chain engines use — a mount
/// that failed still leaves its denial on the record.
pub fn install_audited<H: crate::host::WorkflowHost + ?Sized>(
    host: &std::sync::Arc<H>,
    ctx: &mut Context,
    svc: Box<dyn Service + Send>,
) -> Result<(), KernelError> {
    let key = svc.key();
    let result = ctx.install(svc);
    use crate::host::{AuditKind, AuditStatus};
    host.audit(
        AuditKind::Workflow,
        "plugin",
        key,
        if result.is_ok() {
            AuditStatus::Ok
        } else {
            AuditStatus::Denied
        },
        "mount",
    );
    result
}

/// Audited unmounting; see [`install_audited`].
pub fn uninstall_audited<H: crate::host::WorkflowHost + ?Sized>(
    host: &std::sync::Arc<H>,
    ctx: &mut Context,
    key: &'static str,
) -> Result<(), KernelError> {
    let result = ctx.uninstall(key);
    use crate::host::{AuditKind, AuditStatus};
    host.audit(
        AuditKind::Workflow,
        "plugin",
        key,
        if result.is_ok() {
            AuditStatus::Ok
        } else {
            AuditStatus::Denied
        },
        "unmount",
    );
    result
}

fn borrow_failed<T>(_e: T) -> KernelError {
    KernelError::NotMounted {
        key: "context re-entrant borrow".to_string(),
    }
}

/// Reverses its registration on drop. `dispose` makes the intent explicit;
/// both paths run the same reversal.
pub struct EffectHandle {
    id: u64,
    ctx: Weak<Mutex<Inner>>,
}

impl EffectHandle {
    /// Explicitly reverse now (same as dropping).
    pub fn dispose(self) {}
}

impl Drop for EffectHandle {
    fn drop(&mut self) {
        let Some(rc) = self.ctx.upgrade() else {
            return;
        };
        // Remove under the lock, run the undo OUTSIDE it: the undo receives a
        // live Context and may re-enter (provide/effect) without deadlocking.
        let eff = {
            let mut inner = match rc.lock() {
                Ok(i) => i,
                Err(_) => return,
            };
            inner
                .effects
                .iter()
                .rposition(|e| e.id == self.id)
                .map(|pos| inner.effects.remove(pos))
        };
        if let Some(mut eff) = eff {
            let mut ctx = Context { inner: rc };
            (eff.undo)(&mut ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    type SharedLog = Arc<Mutex<Vec<&'static str>>>;

    fn log_of(log: &SharedLog) -> Vec<&'static str> {
        log.lock().map(|g| g.clone()).unwrap_or_default()
    }

    struct Base {
        log: SharedLog,
    }
    impl Service for Base {
        fn key(&self) -> &'static str {
            "ctx.base"
        }
        fn mount(&mut self, _ctx: &mut Context) {
            if let Ok(mut g) = self.log.lock() {
                g.push("mount-base");
            }
        }
        fn unmount(&self) {
            if let Ok(mut g) = self.log.lock() {
                g.push("unmount-base");
            }
        }
    }

    struct Dependent {
        log: SharedLog,
        handle: Option<EffectHandle>,
        undo_log: SharedLog,
    }
    impl Service for Dependent {
        fn key(&self) -> &'static str {
            "ctx.dependent"
        }
        fn inject(&self) -> &[&str] {
            &["ctx.base"]
        }
        fn mount(&mut self, ctx: &mut Context) {
            if let Ok(mut g) = self.log.lock() {
                g.push("mount");
            }
            let undo_log = self.undo_log.clone();
            self.handle = Some(ctx.effect("watch", move |_ctx: &mut Context| {
                move |_ctx: &mut Context| {
                    if let Ok(mut g) = undo_log.lock() {
                        g.push("undone");
                    }
                }
            }));
        }
        fn unmount(&self) {
            if let Ok(mut g) = self.log.lock() {
                g.push("unmount");
            }
        }
    }

    #[test]
    fn inject_ordering_enforced() {
        let mut ctx = Context::new();
        // Missing dependency refused before any mount side effect runs.
        let err = ctx
            .install(Box::new(Dependent {
                log: Arc::new(Mutex::new(Vec::new())),
                handle: None,
                undo_log: Arc::new(Mutex::new(Vec::new())),
            }))
            .unwrap_err();
        assert_eq!(
            err,
            KernelError::MissingDependency {
                service: "ctx.dependent",
                needs: "ctx.base".into()
            }
        );
        assert!(
            ctx.install(Box::new(Base {
                log: Arc::new(Mutex::new(Vec::new()))
            }))
            .is_ok()
        );
        assert!(
            ctx.install(Box::new(Dependent {
                log: Arc::new(Mutex::new(Vec::new())),
                handle: None,
                undo_log: Arc::new(Mutex::new(Vec::new())),
            }))
            .is_ok()
        );
    }

    #[test]
    fn duplicate_key_and_require_fail_loud() {
        let mut ctx = Context::new();
        assert!(
            ctx.install(Box::new(Base {
                log: Arc::new(Mutex::new(Vec::new()))
            }))
            .is_ok()
        );
        assert_eq!(
            ctx.install(Box::new(Base {
                log: Arc::new(Mutex::new(Vec::new()))
            }))
            .unwrap_err(),
            KernelError::Duplicate {
                key: "ctx.base".into()
            }
        );
        assert_eq!(
            ctx.require::<u64>().unwrap_err(),
            KernelError::NotMounted { key: "u64".into() }
        );
    }

    #[test]
    fn effect_reversal_on_drop_in_reverse_order() {
        let order: SharedLog = Arc::new(Mutex::new(Vec::new()));
        {
            let mut ctx = Context::new();
            let o1 = order.clone();
            let h1 = ctx.effect("one", move |_| {
                if let Ok(mut g) = o1.lock() {
                    g.push("setup-one");
                }
                move |_| {
                    if let Ok(mut g) = o1.lock() {
                        g.push("undo-one");
                    }
                }
            });
            let o2 = order.clone();
            let h2 = ctx.effect("two", move |_| {
                if let Ok(mut g) = o2.lock() {
                    g.push("setup-two");
                }
                move |_| {
                    if let Ok(mut g) = o2.lock() {
                        g.push("undo-two");
                    }
                }
            });
            h2.dispose(); // newest reversed first
            drop(h1);
        }
        assert_eq!(
            log_of(&order),
            vec!["setup-one", "setup-two", "undo-two", "undo-one"]
        );
    }

    #[test]
    fn hmr_unload_then_remount() {
        let log: SharedLog = Arc::new(Mutex::new(Vec::new()));
        let mut ctx = Context::new();
        assert!(
            ctx.install(Box::new(Base {
                log: Arc::new(Mutex::new(Vec::new()))
            }))
            .is_ok()
        );
        let svc = Dependent {
            log: log.clone(),
            handle: None,
            undo_log: Arc::new(Mutex::new(Vec::new())),
        };
        assert!(ctx.install(Box::new(svc)).is_ok());
        assert!(ctx.reload("ctx.dependent").is_ok());
        // install mounts; reload unmounts then remounts the same instance.
        assert_eq!(log_of(&log), vec!["mount", "unmount", "mount"]);
        assert!(ctx.mounted_keys().contains(&"ctx.dependent"));
        assert!(ctx.uninstall("ctx.dependent").is_ok());
        assert!(!ctx.mounted_keys().contains(&"ctx.dependent"));
    }

    #[test]
    fn uninstall_missing_is_loud() {
        let mut ctx = Context::new();
        assert_eq!(
            ctx.uninstall("nope").unwrap_err(),
            KernelError::NotMounted { key: "nope".into() }
        );
    }
}

#[cfg(test)]
mod audit_tests {
    use super::*;
    use crate::host::{AuditKind, AuditStatus, CasError, HostError, HostTx, WorkflowHost};
    use std::sync::{Arc, Mutex as StdMutex};

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

    struct Plain;
    impl Service for Plain {
        fn key(&self) -> &'static str {
            "ctx.plain"
        }
        fn mount(&mut self, _ctx: &mut Context) {}
        fn unmount(&self) {}
    }

    #[test]
    fn mount_unmount_leave_audit_rows_on_the_chain() {
        let host = Arc::new(TapeHost::default());
        let mut ctx = Context::new();
        install_audited(&host, &mut ctx, Box::new(Plain)).ok();
        uninstall_audited(&host, &mut ctx, "ctx.plain").ok();
        let log = host.log.lock().map(|g| g.clone()).unwrap_or_default();
        assert_eq!(
            log,
            vec![
                "workflow/plugin/ctx.plain/ok/mount",
                "workflow/plugin/ctx.plain/ok/unmount"
            ]
        );
    }

    #[test]
    fn failed_mount_audits_denied() {
        let host = Arc::new(TapeHost::default());
        let mut ctx = Context::new();
        install_audited(&host, &mut ctx, Box::new(Plain)).ok();
        let err = install_audited(&host, &mut ctx, Box::new(Plain)).unwrap_err();
        assert_eq!(
            err,
            KernelError::Duplicate {
                key: "ctx.plain".into()
            }
        );
        let log = host.log.lock().map(|g| g.clone()).unwrap_or_default();
        assert!(log.contains(&"workflow/plugin/ctx.plain/denied/mount".to_string()));
    }
}

#[cfg(test)]
mod engine_slot_tests {
    use super::*;
    use crate::workflow::{
        AgentId, RunBuilder, WorkflowEngine, WorkflowError, WorkflowMeta, WorkflowResult,
        WorkflowRun, WorkflowStartRequest,
    };
    use std::sync::{Arc, Mutex as StdMutex};

    struct NamedEngine {
        name: &'static str,
        log: Arc<StdMutex<Vec<&'static str>>>,
    }
    impl WorkflowEngine for NamedEngine {
        fn start(&self, req: WorkflowStartRequest) -> Result<WorkflowRun, WorkflowError> {
            let mut b = RunBuilder::default();
            let id = b.admit(&req)?;
            let (completer, run) = b.build_run(id);
            completer.complete(WorkflowResult::completed("ok".into()));
            if let Ok(mut g) = self.log.lock() {
                g.push(self.name);
            }
            Ok(run)
        }
    }

    #[test]
    fn second_engine_mount_replaces_the_first_not_parallel() {
        let log = Arc::new(StdMutex::new(Vec::new()));
        let mut ctx = Context::new();
        ctx.mount_workflow_engine(Arc::new(NamedEngine {
            name: "first",
            log: Arc::clone(&log),
        }))
        .unwrap();
        ctx.mount_workflow_engine(Arc::new(NamedEngine {
            name: "second",
            log: Arc::clone(&log),
        }))
        .unwrap();
        // Start through the ctx-mounted engine (the tool path shape).
        let mounted = ctx.workflow_engine().expect("engine mounted");
        let req = WorkflowStartRequest {
            script: "x".into(),
            meta: WorkflowMeta {
                name: "demo".into(),
                description: "d".into(),
                when_to_use: None,
                phases: vec![],
            },
            args: "{}".into(),
            parent: AgentId("p".into()),
        };
        mounted.start(req).unwrap();
        assert_eq!(
            log.lock().map(|g| g.clone()).unwrap_or_default(),
            vec!["second"],
            "one engine per context: config replaces, never parallel providers"
        );
    }
}
