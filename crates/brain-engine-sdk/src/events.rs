//! Typed hooks/events: one [`Hooks`] registry owning registration,
//! provenance, and dispatch across four modes.
//!
//! Modes (the `@mode` contract): [`Hooks::emit`] is broadcast observe — every
//! listener runs even if one panics; each listener receives its own clone of
//! the payload. [`Hooks::waterfall`] is short-circuit policy — the first
//! denial wins, later listeners never run, and the denial cannot be
//! overturned (monotonic final denial). [`Hooks::serial`] applies ordered
//! mutations to shared state. [`Hooks::parallel`] fans out with independent
//! clones, aggregated deterministically in registration order.
//!
//! Listener panics are contained: a throwing subscriber is counted in the
//! report and never starves later listeners. Provenance is sidecar metadata
//! keyed by hook id.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, Mutex};

/// A listener's verdict for result-producing events.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Allow,
    Deny(String),
}

/// Per-listener outcome recorded by dispatch.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Ran,
    Denied(String),
    Panicked,
}

impl Outcome {
    pub fn is_panicked(&self) -> bool {
        matches!(self, Outcome::Panicked)
    }
}

/// Aggregated dispatch report.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Report {
    pub outcomes: Vec<(u64, Outcome)>,
}

impl Report {
    pub fn panicked(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, o)| o.is_panicked())
            .count()
    }
    pub fn ran(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, o)| matches!(o, Outcome::Ran))
            .count()
    }
}

type ErasedFn = Arc<dyn Fn(&dyn Any) -> Outcome + Send + Sync>;
type ErasedMutFn = Arc<dyn Fn(&mut dyn Any) -> Outcome + Send + Sync>;

enum Hook {
    /// `emit`/`waterfall`: value-producing listeners over cloned payloads.
    Observer(ErasedFn),
    /// `serial`: ordered mutation of shared state.
    Mutator(ErasedMutFn),
    /// `parallel`: fan-out job over a clone.
    Job(ErasedFn),
}

struct Entry {
    id: u64,
    event: &'static str,
    provenance: String,
    hook: Hook,
}

/// The single hooks registry.
#[derive(Default)]
pub struct Hooks {
    entries: Mutex<Vec<Entry>>,
    next_id: Mutex<u64>,
}

#[derive(Debug, PartialEq)]
pub enum HooksError {
    /// Registration-time validation refused the hook.
    Invalid(&'static str),
}

impl fmt::Display for HooksError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HooksError::Invalid(why) => write!(f, "hook invalid: {why}"),
        }
    }
}

impl std::error::Error for HooksError {}

fn contained<F: FnOnce() -> Outcome>(f: F) -> Outcome {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(o) => o,
        Err(_) => Outcome::Panicked,
    }
}

impl Hooks {
    pub fn new() -> Self {
        Self::default()
    }

    fn allocate(&self) -> u64 {
        let mut n = self.next_id.lock().unwrap_or_else(|p| p.into_inner());
        let id = *n;
        *n += 1;
        id
    }

    /// Register an observer (`@mode emit`-eligible). Empty event names fail
    /// loud at registration.
    pub fn on<E, F>(&self, event: &'static str, provenance: &str, f: F) -> Result<u64, HooksError>
    where
        E: Any + Clone,
        F: Fn(E) -> Verdict + Send + Sync + 'static,
    {
        if event.is_empty() || provenance.is_empty() {
            return Err(HooksError::Invalid("event and provenance are required"));
        }
        let id = self.allocate();
        let listener = Arc::new(f);
        let hook = Hook::Observer(Arc::new(move |payload: &dyn Any| {
            contained(|| {
                let Some(e) = payload.downcast_ref::<E>() else {
                    return Outcome::Panicked;
                };
                match listener(e.clone()) {
                    Verdict::Allow => Outcome::Ran,
                    Verdict::Deny(reason) => Outcome::Denied(reason),
                }
            })
        }));
        self.push(id, event, provenance, hook);
        Ok(id)
    }

    /// Register an ordered mutator (`@mode serial`).
    pub fn on_mutate<S, F>(
        &self,
        event: &'static str,
        provenance: &str,
        f: F,
    ) -> Result<u64, HooksError>
    where
        S: Any,
        F: Fn(&mut S) + Send + Sync + 'static,
    {
        if event.is_empty() || provenance.is_empty() {
            return Err(HooksError::Invalid("event and provenance are required"));
        }
        let id = self.allocate();
        let listener = Arc::new(f);
        let hook = Hook::Mutator(Arc::new(move |state: &mut dyn Any| {
            contained(|| {
                let Some(s) = state.downcast_mut::<S>() else {
                    return Outcome::Panicked;
                };
                listener(s);
                Outcome::Ran
            })
        }));
        self.push(id, event, provenance, hook);
        Ok(id)
    }

    /// Register a fan-out job (`@mode parallel`).
    pub fn on_parallel<E, F>(
        &self,
        event: &'static str,
        provenance: &str,
        f: F,
    ) -> Result<u64, HooksError>
    where
        E: Any + Clone,
        F: Fn(E) + Send + Sync + 'static,
    {
        if event.is_empty() || provenance.is_empty() {
            return Err(HooksError::Invalid("event and provenance are required"));
        }
        let id = self.allocate();
        let listener = Arc::new(f);
        let hook = Hook::Job(Arc::new(move |payload: &dyn Any| {
            contained(|| {
                let Some(e) = payload.downcast_ref::<E>() else {
                    return Outcome::Panicked;
                };
                listener(e.clone());
                Outcome::Ran
            })
        }));
        self.push(id, event, provenance, hook);
        Ok(id)
    }

    fn push(&self, id: u64, event: &'static str, provenance: &str, hook: Hook) {
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries.push(Entry {
            id,
            event,
            provenance: provenance.to_string(),
            hook,
        });
    }

    /// Sidecar provenance lookup by hook id.
    pub fn provenance(&self, id: u64) -> Option<String> {
        let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.provenance.clone())
    }

    /// Broadcast observe: every listener runs with its own payload clone;
    /// a panic is contained and later listeners still run.
    pub fn emit<E: Any>(&self, event: &'static str, payload: &E) -> Report {
        self.dispatch(event, payload, Kind::Observers)
    }

    /// Short-circuit policy: first deny wins; later listeners do not run and
    /// the denial stands (monotonic final denial).
    pub fn waterfall<E: Any>(&self, event: &'static str, payload: &E) -> Result<Report, String> {
        let mut report = Report::default();
        for (id, hook) in self.matching(event) {
            let Hook::Observer(hook) = &hook else {
                continue;
            };
            let outcome = hook(payload as &dyn Any);
            let denied = match &outcome {
                Outcome::Denied(reason) => Some(reason.clone()),
                _ => None,
            };
            report.outcomes.push((id, outcome));
            if let Some(reason) = denied {
                // Monotonic: stop here, nothing downstream can overturn it.
                return Err(reason);
            }
        }
        Ok(report)
    }

    /// Ordered mutations over shared state (`&mut`), registration order.
    pub fn serial<S: Any>(&self, event: &'static str, state: &mut S) -> Report {
        let mut report = Report::default();
        for (id, hook) in self.matching(event) {
            let Hook::Mutator(hook) = &hook else {
                continue;
            };
            let outcome = hook(state as &mut dyn Any);
            report.outcomes.push((id, outcome));
        }
        report
    }

    /// Fan-out: each job gets an independent clone; outcomes aggregate in
    /// registration order so reports stay deterministic.
    pub fn parallel<E: Any + Clone>(&self, event: &'static str, payload: &E) -> Report {
        self.dispatch(event, payload, Kind::Jobs)
    }

    fn matching(&self, event: &'static str) -> Vec<(u64, Hook)> {
        let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries
            .iter()
            .filter(|e| e.event == event)
            .map(|e| {
                (
                    e.id,
                    match &e.hook {
                        Hook::Observer(h) => Hook::Observer(Arc::clone(h)),
                        Hook::Mutator(h) => Hook::Mutator(Arc::clone(h)),
                        Hook::Job(h) => Hook::Job(Arc::clone(h)),
                    },
                )
            })
            .collect()
    }

    fn dispatch<E: Any>(&self, event: &'static str, payload: &E, kind: Kind) -> Report {
        let mut report = Report::default();
        for (id, hook) in self.matching(event) {
            let accepted = matches!(
                (&hook, kind),
                (Hook::Observer(_), Kind::Observers) | (Hook::Job(_), Kind::Jobs)
            );
            if !accepted {
                continue;
            }
            let outcome = match &hook {
                Hook::Observer(h) | Hook::Job(h) => h(payload as &dyn Any),
                Hook::Mutator(_) => continue,
            };
            report.outcomes.push((id, outcome));
        }
        report
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Observers,
    Jobs,
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn emit_broadcasts_with_clone_per_listener_and_contains_panics() {
        let h = Hooks::new();
        let calls = Arc::new(AtomicUsize::new(0));
        h.on::<String, _>("turn.start", "metrics", {
            let c = calls.clone();
            move |_e| {
                c.fetch_add(1, Ordering::SeqCst);
                Verdict::Allow
            }
        })
        .unwrap();
        // A throwing subscriber: panics are contained per listener.
        h.on::<String, _>("turn.start", "chaos", |_e| {
            if _e == "boom" {
                std::panic::panic_any("listener blew up")
            }
            Verdict::Allow
        })
        .unwrap();
        h.on::<String, _>("turn.start", "logger", {
            let c = calls.clone();
            move |_e| {
                c.fetch_add(1, Ordering::SeqCst);
                Verdict::Allow
            }
        })
        .unwrap();

        let report = h.emit("turn.start", &"boom".to_string());
        assert_eq!(report.panicked(), 1, "throwing subscriber contained");
        assert_eq!(report.ran(), 2, "later listeners never starve");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn waterfall_first_deny_wins_monotonically() {
        let h = Hooks::new();
        let later_ran = Arc::new(AtomicUsize::new(0));
        h.on::<String, _>("tool.call", "policy", |e| {
            if e.contains("rm") {
                Verdict::Deny("destructive".into())
            } else {
                Verdict::Allow
            }
        })
        .unwrap();
        // An overturning listener must never even run after a deny.
        h.on::<String, _>("tool.call", "optimist", {
            let r = later_ran.clone();
            move |_e| {
                r.fetch_add(1, Ordering::SeqCst);
                Verdict::Allow
            }
        })
        .unwrap();

        let ok = h.waterfall("tool.call", &"ls -l".to_string()).unwrap();
        assert_eq!(ok.ran(), 2);
        let err = h
            .waterfall("tool.call", &"rm -rf /".to_string())
            .unwrap_err();
        assert_eq!(err, "destructive");
        assert_eq!(later_ran.load(Ordering::SeqCst), 1, "only the allowed run");
    }

    #[test]
    fn serial_applies_mutations_in_registration_order() {
        let h = Hooks::new();
        h.on_mutate("state.fold", "append-a", |s: &mut String| s.push('a'))
            .unwrap();
        h.on_mutate("state.fold", "append-b", |s: &mut String| s.push('b'))
            .unwrap();
        let mut state = String::new();
        let report = h.serial("state.fold", &mut state);
        assert_eq!(report.ran(), 2);
        assert_eq!(state, "ab");
    }

    #[test]
    fn parallel_fans_out_clones_deterministically() {
        let h = Hooks::new();
        let total = Arc::new(AtomicUsize::new(0));
        for _ in 0..3 {
            let t = total.clone();
            h.on_parallel::<usize, _>("fan.out", "worker", move |n| {
                t.fetch_add(n, Ordering::SeqCst);
            })
            .unwrap();
        }
        let report = h.parallel("fan.out", &2usize);
        assert_eq!(report.ran(), 3);
        assert_eq!(total.load(Ordering::SeqCst), 6);
    }

    #[test]
    fn provenance_is_sidecar_metadata() {
        let h = Hooks::new();
        let id = h
            .on::<String, _>("e", "plugin:review@1.4", |_e| Verdict::Allow)
            .unwrap();
        assert_eq!(h.provenance(id).as_deref(), Some("plugin:review@1.4"));
        assert_eq!(h.provenance(9999), None);
    }

    #[test]
    fn registration_validates_loud() {
        let h = Hooks::new();
        assert_eq!(
            h.on::<String, _>("", "p", |_e| Verdict::Allow).unwrap_err(),
            HooksError::Invalid("event and provenance are required")
        );
        assert_eq!(
            h.on::<String, _>("e", "", |_e| Verdict::Allow).unwrap_err(),
            HooksError::Invalid("event and provenance are required")
        );
    }

    #[test]
    fn mode_mismatch_does_not_cross_dispatch() {
        let h = Hooks::new();
        h.on_parallel::<usize, _>("evt", "job", |_| {}).unwrap();
        // emit only fires observers; parallel only fires jobs.
        assert_eq!(h.emit("evt", &1usize).ran(), 0);
        assert_eq!(h.parallel("evt", &1usize).ran(), 1);
    }
}
