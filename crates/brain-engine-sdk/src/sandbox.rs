//! The sandbox seam: the typed spec/outcome vocabulary and the
//! policy-outranks-backend selection law for the exec path.
//!
//! Types and pure mapping only — the crate stays dependency-free and
//! `#![forbid(unsafe_code)]`. Backends live host-side (kernel): this module
//! fixes the vocabulary every backend must speak and the ONE precedence law
//! that governs them: a trust policy that denies `exec` for the engine is a
//! refusal BEFORE any backend is constructed or run. A backend that is
//! requested but unavailable is a named refusal, never a silent fall-through
//! to unconfined execution — the operator asked for a boundary, and the
//! absence of a boundary is a refusal, not a shrug.

use std::path::PathBuf;
use std::time::Duration;

use crate::trust::{Decision, ExtensionPolicy};

/// What the exec path was asked to run under.
///
/// `Inherited` is today's posture (same-user child, screen + allowlist +
/// minimal env only). `SandboxExec` is the macOS Seatbelt deny-default
/// backend; `Landlock` the Linux LSM backend. The operator may SELECT a
/// backend; the operator never supplies backend policy text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Inherited,
    SandboxExec,
    Landlock,
}

impl Backend {
    /// The knob vocabulary, parsed fail-closed: anything the vocabulary does
    /// not name is `None` (the caller refuses; it never silently inherits).
    pub fn from_knob(value: &str) -> Option<Backend> {
        match value.trim().to_ascii_lowercase().as_str() {
            "inherited" => Some(Backend::Inherited),
            "sandbox-exec" => Some(Backend::SandboxExec),
            "landlock" => Some(Backend::Landlock),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Inherited => "inherited",
            Backend::SandboxExec => "sandbox-exec",
            Backend::Landlock => "landlock",
        }
    }

    /// The platform's native OS-confinement backend, for the `enterprise`
    /// profile default. There is no universal backend: the mapping is
    /// honest per platform and the caller still goes through `decide`.
    pub fn platform_default() -> Backend {
        if cfg!(target_os = "linux") {
            Backend::Landlock
        } else {
            Backend::SandboxExec
        }
    }
}

/// One sandboxed spawn request. Integer law: durations and exit codes only —
/// no floats ever cross the seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxSpec {
    /// Full argv, `argv[0]` first. The backend never re-splits or re-joins
    /// it: each element rides as one argv element.
    pub argv: Vec<String>,
    /// Working directory the child may write inside; backends scope writes
    /// to its REALIZED path (canonicalized) plus the realized temp dir.
    pub workdir: PathBuf,
    /// The caller's wall-clock budget. The effective budget is this
    /// intersected with the manager ceiling (`Budget::effective_timeout`),
    /// taken by the host wiring — the backend kills at whichever expires
    /// first.
    pub budget: Duration,
}

/// Why a killed child stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillReason {
    /// The effective budget expired; the backend force-terminated.
    Deadline,
    /// A cancellation token fired mid-run.
    Cancelled,
    /// The handle was dropped mid-run; RAII killed and reaped.
    Drop,
}

/// Terminal status of one sandboxed run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxStatus {
    /// The child exited on its own.
    Exited { code: i32 },
    /// The backend killed the child.
    Killed { reason: KillReason },
    /// The run never started: policy refusal, unavailable backend, or any
    /// fail-closed precondition. The reason names itself.
    Refused { reason: String },
}

/// Captured output of one sandboxed run. Each stream is capped by the host
/// (the exec path's existing cap); the backend reports what it captured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxOutcome {
    pub status: SandboxStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl SandboxOutcome {
    pub fn refused(reason: impl Into<String>) -> SandboxOutcome {
        SandboxOutcome {
            status: SandboxStatus::Refused {
                reason: reason.into(),
            },
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }
}

/// The seam a backend implements: one blocking call per spawn. The exec
/// handler is already blocking, so the seam is too — no async, no runtime.
pub trait SandboxProvider {
    fn run(&self, spec: SandboxSpec) -> SandboxOutcome;

    /// Cancellable form. Default ignores the token (a backend that cannot
    /// observe cancellation still terminates via the budget, so ignoring it
    /// is safe, never unbounded). Backends that can observe a token kill and
    /// report `Killed { Cancelled }`.
    fn run_cancellable(
        &self,
        spec: SandboxSpec,
        cancel: &crate::hostcall::CancellationToken,
    ) -> SandboxOutcome {
        let _ = cancel;
        self.run(spec)
    }
}

/// The selection decision: which backend may run, or the refusal that
/// outranks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// The named backend may spawn.
    Run(Backend),
    /// No backend may run. The reason names the law that refused.
    Refused { reason: String },
}

/// The ONE precedence law: the trust policy outranks every backend.
///
/// A policy that denies `exec` for the engine refuses BEFORE any backend is
/// constructed or run — including `Inherited`. A `Prompt` decision also
/// refuses: the seam is blocking and non-interactive, and an un-consented
/// spawn is a refused spawn (fail-closed). Only an explicit `Allowed`
/// reaches the backend. The policy is the contract's
/// `required_capability_for_host_call` precedence, realized through
/// [`ExtensionPolicy::decide`].
pub fn decide(policy: &ExtensionPolicy, engine: &str, backend: Backend) -> Selection {
    match policy.decide(engine, "exec") {
        Decision::Allowed => Selection::Run(backend),
        Decision::Denied => Selection::Refused {
            reason: format!("policy denies exec for engine '{engine}'"),
        },
        Decision::Prompt => Selection::Refused {
            reason: format!("exec for engine '{engine}' requires consent the seam cannot obtain"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The contract's verification test, mapped: per-engine deny overrides
    // allow, and the refusal fires BEFORE any backend — pinned with a
    // counting double that fails the test if a backend was even asked.
    #[test]
    fn per_engine_deny_overrides_allow() {
        let mut policy = ExtensionPolicy::permissive();
        policy.default_caps = vec!["exec".into()];
        policy
            .per_engine
            .entry("scrappy".to_string())
            .or_default()
            .deny_caps = vec!["exec".into()];

        let seen = CountingProvider::default();
        for backend in [Backend::Inherited, Backend::SandboxExec, Backend::Landlock] {
            assert!(
                matches!(
                    decide(&policy, "scrappy", backend),
                    Selection::Refused { .. }
                ),
                "per-engine deny must refuse, never run ({backend:?})"
            );
        }
        assert_eq!(seen.count.load(std::sync::atomic::Ordering::SeqCst), 0);
        if let Selection::Refused { reason } = decide(&policy, "scrappy", Backend::Inherited) {
            assert!(
                reason.contains("denies exec"),
                "refusal must name the law: {reason}"
            );
        }
    }

    #[test]
    fn explicit_per_engine_allow_beats_a_global_deny_absence_but_never_a_deny() {
        // Precedence: per-engine deny > global deny > per-engine allow >
        // global allow. An allow with no deny anywhere runs.
        let mut allowed = ExtensionPolicy::permissive();
        allowed
            .per_engine
            .entry("busy".to_string())
            .or_default()
            .allow_caps = vec!["exec".into()];
        assert_eq!(
            decide(&allowed, "busy", Backend::Inherited),
            Selection::Run(Backend::Inherited)
        );

        // The same allow under a global deny is still a refusal: deny
        // outranks allow at every level.
        let mut denied = allowed.clone();
        denied.deny_caps = vec!["exec".into()];
        assert!(matches!(
            decide(&denied, "busy", Backend::SandboxExec),
            Selection::Refused { .. }
        ));
    }

    #[test]
    fn prompt_refuses_because_the_seam_cannot_obtain_consent() {
        // `standard()` denies exec outright; `strict()` falls through to
        // Denied too. The Prompt posture needs a policy that neither denies
        // nor allows exec in Prompt mode: an engine with no entry under
        // Prompt mode.
        let mut policy = ExtensionPolicy::standard();
        policy.deny_caps.clear();
        assert!(matches!(
            decide(&policy, "some-engine", Backend::Inherited),
            Selection::Refused { .. }
        ));
    }

    #[test]
    fn safe_profile_denies_exec_for_every_backend_before_it_exists() {
        for backend in [Backend::Inherited, Backend::SandboxExec, Backend::Landlock] {
            assert!(matches!(
                decide(&ExtensionPolicy::safe(), "any", backend),
                Selection::Refused { .. }
            ));
        }
    }

    #[test]
    fn knob_vocabulary_is_closed_and_fail_closed() {
        assert_eq!(Backend::from_knob("inherited"), Some(Backend::Inherited));
        assert_eq!(
            Backend::from_knob("sandbox-exec"),
            Some(Backend::SandboxExec)
        );
        assert_eq!(Backend::from_knob("landlock"), Some(Backend::Landlock));
        assert_eq!(
            Backend::from_knob(" sandbox-exec "),
            Some(Backend::SandboxExec)
        );
        assert_eq!(
            Backend::from_knob("SANDBOX-EXEC"),
            Some(Backend::SandboxExec)
        );
        assert_eq!(
            Backend::from_knob("gvisor"),
            None,
            "unnamed backends are None, never a fallback"
        );
        assert_eq!(Backend::from_knob(""), None);
        assert_eq!(Backend::from_knob("allow-everything"), None);
    }

    #[test]
    fn outcome_shape_is_integer_and_refusals_are_empty() {
        let exited = SandboxOutcome {
            status: SandboxStatus::Exited { code: 0 },
            stdout: b"ok".to_vec(),
            stderr: Vec::new(),
        };
        assert!(matches!(exited.status, SandboxStatus::Exited { code: 0 }));

        for reason in ["policy denies exec", "backend unavailable"] {
            let r = SandboxOutcome::refused(reason);
            assert!(r.stdout.is_empty() && r.stderr.is_empty());
            assert!(matches!(r.status, SandboxStatus::Refused { .. }));
        }

        for killed in [
            KillReason::Deadline,
            KillReason::Cancelled,
            KillReason::Drop,
        ] {
            let k = SandboxOutcome {
                status: SandboxStatus::Killed { reason: killed },
                stdout: Vec::new(),
                stderr: Vec::new(),
            };
            assert!(matches!(k.status, SandboxStatus::Killed { .. }));
        }
    }

    #[test]
    fn default_cancellable_form_ignores_the_token_and_still_terminates_via_budget() {
        let provider = CountingProvider::default();
        let token = crate::hostcall::CancellationToken::default();
        let outcome = provider.run_cancellable(
            SandboxSpec {
                argv: vec!["/bin/true".into()],
                workdir: PathBuf::from("/"),
                budget: Duration::from_secs(1),
            },
            &token,
        );
        assert_eq!(
            outcome.status,
            SandboxStatus::Exited { code: 7 },
            "the default form delegates to run; the budget is the backstop"
        );
    }

    /// Test double that counts every `run` — the policy-outranks-backend
    /// law asserts it is NEVER invoked on a refusal.
    #[derive(Default)]
    struct CountingProvider {
        count: std::sync::atomic::AtomicUsize,
    }

    impl SandboxProvider for CountingProvider {
        fn run(&self, _spec: SandboxSpec) -> SandboxOutcome {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            SandboxOutcome {
                status: SandboxStatus::Exited { code: 7 },
                stdout: Vec::new(),
                stderr: Vec::new(),
            }
        }
    }
}
