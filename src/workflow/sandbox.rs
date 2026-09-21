//! The OS boundary for the exec path.
//!
//! The ONE process path stays `hostcalls::exec_effect_for`; when the
//! operator's knob (or the enterprise profile default) selects an OS
//! backend, the same screened argv runs under a real confinement boundary
//! instead of the same-user inherited posture. Three laws shape everything
//! here:
//!
//! 1. **Deny-default, compiled-in.** The Seatbelt profile is a `const` —
//!    deny everything, grant read + process-exec, scope writes to the
//!    realized workdir and the realized temp dir, deny network explicitly.
//!    The operator may SELECT a backend; the operator never supplies
//!    profile text. Allow-default profiles have documented escape vectors
//!    and are forbidden by construction here.
//! 2. **Realized paths.** Seatbelt evaluates canonical paths: a profile
//!    subpath written as `/tmp/...` does not authorize `/private/tmp/...`.
//!    Every subpath therefore comes from `fs::canonicalize`, and paths
//!    carrying profile metacharacters are refused, never rendered.
//! 3. **Fail-closed everywhere.** An unknown knob value is a named refusal,
//!    never a silent inherit; a requested backend that cannot enforce
//!    (landlock reporting partial or absent enforcement) is a named
//!    refusal, never unconfined execution; the worker thread panicking is
//!    a typed refusal, never a poisoned server. The handle's `Drop` kills
//!    and reaps on every path out.

use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use brain_engine_sdk::hostcall::CancellationToken;
use brain_engine_sdk::sandbox::{
    Backend, KillReason, SandboxOutcome, SandboxProvider, SandboxSpec, SandboxStatus,
};

use crate::workflow::hostcalls::EFFECT_OUTPUT_CAP;

// ── selection ────────────────────────────────────────────────────────────

/// The operator knob: which backend the exec path wraps its spawns in.
/// Unset + non-enterprise profile → `None` (the inherited posture, exactly
/// today's behavior). An unknown value is a named refusal, never a silent
/// inherit — the operator asked for something this build does not name.
pub(crate) fn selected_backend_for_exec() -> Result<Option<Backend>, String> {
    match std::env::var("BRAIN_ENGINE_SANDBOX_BACKEND") {
        Ok(raw) if !raw.trim().is_empty() => Backend::from_knob(&raw)
            .map(Some)
            .ok_or_else(|| format!("BRAIN_ENGINE_SANDBOX_BACKEND names no known backend: '{raw}'")),
        _ => {
            if crate::config::model_profile() == crate::config::PROFILE_ENTERPRISE {
                Ok(Some(Backend::platform_default()))
            } else {
                Ok(None)
            }
        }
    }
}

/// The effective wall-clock budget for a sandboxed spawn: the caller's spec
/// budget intersected with the exec handler's ceiling (whose value is the
/// SDK `Budget::effective_timeout()` default). Whichever is smaller kills.
pub(crate) fn effective_budget(spec_budget: Duration, handler_budget: Duration) -> Duration {
    spec_budget.min(handler_budget)
}

// ── the deny-default Seatbelt profile ────────────────────────────────────

/// The compiled-in profile. Deny default; read everywhere; exec allowed;
/// writes scoped to the two realized subpaths rendered by
/// [`seatbelt_profile`]; network denied explicitly (defense against
/// allow-creep — a future grant can never silently re-open it).
const SEATBELT_PROFILE: &str = r#"(version 1)
(deny default)
(allow file-read*)
(allow process-exec)
(allow file-write* (subpath "{WORKDIR}"))
(allow file-write* (subpath "{TMPDIR}"))
(deny network*)
"#;

/// Characters that would let a path carry profile syntax. A canonicalized
/// path containing any of them is a named refusal, never a rendered profile.
const PROFILE_METACHARACTERS: &str = "{}()\"\\#\n";

fn realized(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path).map_err(|e| format!("path not canonicalizable: {e}"))
}

fn check_profile_safe(path: &Path) -> Result<(), String> {
    let text = path.to_string_lossy();
    if let Some(c) = text.chars().find(|c| PROFILE_METACHARACTERS.contains(*c)) {
        return Err(format!(
            "workdir carries a profile metacharacter ({c}); refusing to render a profile from it"
        ));
    }
    Ok(())
}

fn seatbelt_profile(workdir_real: &Path, tmpdir_real: &Path) -> String {
    SEATBELT_PROFILE
        .replace("{WORKDIR}", &workdir_real.to_string_lossy())
        .replace("{TMPDIR}", &tmpdir_real.to_string_lossy())
}

// ── the handle: RAII kill + reap on every path out ───────────────────────

pub(crate) struct SandboxHandle {
    child: std::process::Child,
    out_reader: Option<std::thread::JoinHandle<Vec<u8>>>,
    err_reader: Option<std::thread::JoinHandle<Vec<u8>>>,
}

impl SandboxHandle {
    fn spawn(command: &mut std::process::Command) -> Result<SandboxHandle, String> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn failed: {e}"))?;
        let out_reader = drain(child.stdout.take(), EFFECT_OUTPUT_CAP);
        let err_reader = drain(child.stderr.take(), EFFECT_OUTPUT_CAP);
        Ok(SandboxHandle {
            child,
            out_reader: Some(out_reader),
            err_reader: Some(err_reader),
        })
    }

    /// Kill + reap + collect. Every terminal path goes through here so the
    /// deadbolt law holds: no exit from the wait loop leaves a live child.
    fn collect(mut self) -> (Option<std::process::ExitStatus>, Vec<u8>, Vec<u8>) {
        let _ = self.child.kill();
        let out = take_drain(&mut self.out_reader);
        let err = take_drain(&mut self.err_reader);
        let status = self.child.wait().ok();
        (status, out, err)
    }
}

fn take_drain(slot: &mut Option<std::thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    slot.take().and_then(|h| h.join().ok()).unwrap_or_default()
}

impl Drop for SandboxHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // The drain threads hit EOF once the child dies and finish on their
        // own; detaching them here costs nothing and cannot hang the drop.
    }
}

fn drain<R: Read + Send + 'static>(
    pipe: Option<R>,
    cap: usize,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let mut chunk = [0u8; 8192];
            loop {
                match p.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let take = n.min(cap.saturating_sub(buf.len()));
                        buf.extend_from_slice(&chunk[..take]);
                        // Past the cap: keep draining so the child cannot
                        // block, but record nothing more.
                    }
                }
            }
        }
        buf
    })
}

// ── the wait loop: budget, cancellation, deadbolt ────────────────────────

fn wait_for_child(
    mut handle: SandboxHandle,
    budget: Duration,
    cancel: &CancellationToken,
) -> SandboxOutcome {
    let deadline = Instant::now() + budget;
    loop {
        match handle.child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code().unwrap_or(-1);
                let out = take_drain(&mut handle.out_reader);
                let err = take_drain(&mut handle.err_reader);
                return SandboxOutcome {
                    status: SandboxStatus::Exited { code },
                    stdout: out,
                    stderr: err,
                };
            }
            Ok(None) => {
                if cancel.is_cancelled() {
                    let (status, out, err) = handle.collect();
                    let _ = status;
                    return SandboxOutcome {
                        status: SandboxStatus::Killed {
                            reason: KillReason::Cancelled,
                        },
                        stdout: out,
                        stderr: err,
                    };
                }
                if Instant::now() >= deadline {
                    let (status, out, err) = handle.collect();
                    let _ = status;
                    return SandboxOutcome {
                        status: SandboxStatus::Killed {
                            reason: KillReason::Deadline,
                        },
                        stdout: out,
                        stderr: err,
                    };
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                let (_, out, err) = handle.collect();
                return SandboxOutcome {
                    status: SandboxStatus::Refused {
                        reason: format!("wait failed: {e}"),
                    },
                    stdout: out,
                    stderr: err,
                };
            }
        }
    }
}

// ── the minimal child environment (the secret law) ───────────────────────

/// The child gets the same minimal environment as the inherited posture:
/// never the server's env, only what a sane tool needs. Secrets and
/// configuration cannot be exfiltrated by any allowlisted program that
/// prints its environment.
fn minimial_child_env(command: &mut std::process::Command, workdir: &Path, tmpdir: &Path) {
    command.env_clear();
    command.env("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin");
    command.env("HOME", workdir);
    command.env("TMPDIR", tmpdir);
}

// ── the sandbox-exec (Seatbelt) worker ───────────────────────────────────

/// A spawned-but-not-yet-waited child, for the handle laws' tests.
pub(crate) struct RunningSandbox {
    pub(crate) handle: SandboxHandle,
    pub(crate) budget: Duration,
}

struct PreparedSeatbelt {
    command: std::process::Command,
    budget: Duration,
}

fn prepare_seatbelt(spec: &SandboxSpec) -> Result<PreparedSeatbelt, SandboxOutcome> {
    let refuse = |reason: String| SandboxOutcome::refused(reason);
    let workdir_real = realized(&spec.workdir).map_err(refuse)?;
    let tmpdir = std::env::temp_dir();
    let tmpdir_real = realized(&tmpdir).map_err(refuse)?;
    check_profile_safe(&workdir_real).map_err(refuse)?;
    check_profile_safe(&tmpdir_real).map_err(refuse)?;
    let profile = seatbelt_profile(&workdir_real, &tmpdir_real);
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let profile_path = tmpdir_real.join(format!("brain-sbx-{}-{}.sb", std::process::id(), unique));
    let write_profile = || -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&profile_path)
            .map_err(|e| format!("profile file: {e}"))?;
        file.write_all(profile.as_bytes())
            .map_err(|e| format!("profile file: {e}"))
    };
    if let Err(reason) = write_profile() {
        let _ = std::fs::remove_file(&profile_path);
        return Err(refuse(reason));
    }

    let mut argv = spec.argv.iter();
    let argv0 = match argv.next() {
        Some(a) => a.clone(),
        None => {
            let _ = std::fs::remove_file(&profile_path);
            return Err(refuse("exec requires a non-empty argv".into()));
        }
    };
    let mut command = std::process::Command::new("/usr/bin/sandbox-exec");
    command.arg("-f").arg(&profile_path);
    command.arg(&argv0);
    command.args(argv);
    command.current_dir(&spec.workdir);
    minimial_child_env(&mut command, &spec.workdir, &tmpdir);
    Ok(PreparedSeatbelt {
        command,
        budget: spec.budget,
    })
}

fn spawn_seatbelt(spec: SandboxSpec) -> Result<RunningSandbox, SandboxOutcome> {
    let prepared = prepare_seatbelt(&spec)?;
    // The launcher parses the profile at spawn; the file's job is done the
    // moment the child exists.
    let mut command = prepared.command;
    let handle = match SandboxHandle::spawn(&mut command) {
        Ok(h) => h,
        Err(reason) => return Err(SandboxOutcome::refused(reason)),
    };
    Ok(RunningSandbox {
        handle,
        budget: prepared.budget,
    })
}

fn seatbelt_run(spec: SandboxSpec, cancel: CancellationToken) -> SandboxOutcome {
    let budget = spec.budget;
    spawn_seatbelt(spec)
        .map(|running| wait_for_child(running.handle, budget, &cancel))
        .unwrap_or_else(std::convert::identity)
}

// ── the landlock worker (Linux, target-gated) ────────────────────────────

/// The closed enforcement vocabulary the fail-closed refusal law speaks.
/// The Linux worker feeds the real `RulesetStatus` into this mapping; the
/// law itself is platform-independent and pinned on every platform.
pub(crate) enum Enforcement {
    Fully,
    Partially,
    Not,
}

/// Enforcement honesty: a requested boundary that the kernel cannot fully
/// enforce is a named refusal, never unconfined execution.
pub(crate) fn enforcement_decision(enforced: Enforcement) -> Result<(), String> {
    match enforced {
        Enforcement::Fully => Ok(()),
        Enforcement::Partially | Enforcement::Not => {
            Err("sandbox unavailable: landlock not fully enforced".to_string())
        }
    }
}

#[cfg(target_os = "linux")]
mod landlock {
    use super::*;

    use landlock::{
        ABI, Access, AccessFs, AccessNet, BitFlags, PathBeneath, PathFd, Ruleset, RulesetAttr,
        RulesetCreated, RulesetCreatedAttr, RulesetStatus,
    };

    // Newest fixed ABI: on older kernels the crate degrades best-effort and
    // the enforcement refusal above fires — honest, never half-confined.
    const ABI_V: ABI = ABI::V6;

    fn add_path_rule(
        created: RulesetCreated,
        path: &Path,
        access: BitFlags<AccessFs>,
    ) -> Result<RulesetCreated, String> {
        let fd = PathFd::new(path).map_err(|e| format!("landlock rule: {e}"))?;
        created
            .add_rule(PathBeneath::new(fd, access))
            .map_err(|e| format!("landlock rule: {e}"))
    }

    pub(super) fn run_landlocked(spec: SandboxSpec, cancel: CancellationToken) -> SandboxOutcome {
        super::run_isolated(move || {
            let workdir_real = match realized(&spec.workdir) {
                Ok(p) => p,
                Err(reason) => return SandboxOutcome::refused(reason),
            };
            let tmpdir_real = match realized(&std::env::temp_dir()) {
                Ok(p) => p,
                Err(reason) => return SandboxOutcome::refused(reason),
            };

            let abi = ABI_V;
            let read_everywhere = AccessFs::from_read(abi) | AccessFs::Execute;
            let opened = Ruleset::default()
                .handle_access(AccessFs::from_all(abi))
                .and_then(|rs| rs.handle_access(AccessNet::from_all(abi)))
                .and_then(|rs| rs.create());
            let created = match opened {
                Ok(rs) => rs,
                Err(e) => return SandboxOutcome::refused(format!("landlock ruleset: {e}")),
            };
            let granted = add_path_rule(created, Path::new("/"), read_everywhere)
                .and_then(|rs| add_path_rule(rs, &workdir_real, AccessFs::from_write(abi)))
                .and_then(|rs| add_path_rule(rs, &tmpdir_real, AccessFs::from_write(abi)));
            let created = match granted {
                Ok(rs) => rs,
                Err(reason) => return SandboxOutcome::refused(reason),
            };
            let status = match created.restrict_self() {
                Ok(s) => s,
                Err(e) => return SandboxOutcome::refused(format!("landlock restrict: {e}")),
            };
            // Restrict-then-spawn: the refusal fires BEFORE any child exists,
            // so a refused run cannot leave a confined orphan. Children
            // created on this (now restricted) thread inherit the
            // restriction; the thread dies after reaping.
            if let Err(reason) = enforcement_decision(match status.ruleset {
                RulesetStatus::FullyEnforced => Enforcement::Fully,
                RulesetStatus::PartiallyEnforced => Enforcement::Partially,
                RulesetStatus::NotEnforced => Enforcement::Not,
            }) {
                return SandboxOutcome::refused(reason);
            }

            let mut argv = spec.argv.iter();
            let argv0 = match argv.next() {
                Some(a) => a.clone(),
                None => return SandboxOutcome::refused("exec requires a non-empty argv"),
            };
            let mut command = std::process::Command::new(&argv0);
            command.args(argv);
            command.current_dir(&spec.workdir);
            minimial_child_env(&mut command, &spec.workdir, &tmpdir_real);
            let handle = match SandboxHandle::spawn(&mut command) {
                Ok(h) => h,
                Err(reason) => return SandboxOutcome::refused(reason),
            };
            wait_for_child(handle, spec.budget, &cancel)
        })
    }

    // The tests below execute ONLY on the operator's Linux CI (kernel with
    // Landlock fully enforced); this darwin host cannot compile, let alone
    // run them — the named ceiling, mirroring the fuzz named gate.
    #[cfg(test)]
    mod linux_ci {
        use super::*;

        fn scratch(tag: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!("ll-{}-{}", std::process::id(), tag));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch dir");
            dir
        }

        fn spec(argv: &[&str], workdir: &Path) -> SandboxSpec {
            SandboxSpec {
                argv: argv.iter().map(|s| s.to_string()).collect(),
                workdir: workdir.to_path_buf(),
                budget: Duration::from_secs(10),
            }
        }

        #[test]
        fn landlock_write_inside_workdir_succeeds() {
            let workdir = scratch("in");
            let inside = workdir.join("inside.txt");
            let outcome = OsBackendProvider {
                backend: Backend::Landlock,
            }
            .run(spec(
                &[
                    "/bin/sh",
                    "-c",
                    &format!("echo inside > {}", inside.display()),
                ],
                &workdir,
            ));
            assert!(
                matches!(outcome.status, SandboxStatus::Exited { code: 0 }),
                "write inside must succeed under full enforcement: {:?} / {}",
                outcome.status,
                String::from_utf8_lossy(&outcome.stderr)
            );
            let _ = std::fs::remove_dir_all(&workdir);
        }

        #[test]
        fn landlock_write_outside_workdir_fails() {
            let workdir = scratch("out");
            let outside =
                std::env::temp_dir().join(format!("ll-escape-{}.txt", std::process::id()));
            let outcome = OsBackendProvider {
                backend: Backend::Landlock,
            }
            .run(spec(
                &[
                    "/bin/sh",
                    "-c",
                    &format!("echo pwned > {}", outside.display()),
                ],
                &workdir,
            ));
            assert!(
                !matches!(outcome.status, SandboxStatus::Exited { code: 0 }),
                "write outside must fail under full enforcement: {:?}",
                outcome.status
            );
            let _ = std::fs::remove_dir_all(&workdir);
            let _ = std::fs::remove_file(&outside);
        }
    }
}

// ── the worker-thread island: panic containment ──────────────────────────

fn run_isolated(worker: impl FnOnce() -> SandboxOutcome + Send + 'static) -> SandboxOutcome {
    let spawned = std::thread::Builder::new()
        .name("sandbox-worker".into())
        .spawn(worker)
        .map(|handle| {
            handle.join().unwrap_or_else(|_| {
                SandboxOutcome::refused(
                    "sandbox worker panicked; the child was killed and reaped by the handle's drop",
                )
            })
        })
        .map_err(|e| SandboxOutcome::refused(format!("sandbox worker could not start: {e}")));
    spawned.unwrap_or_else(std::convert::identity)
}

// ── the provider seam ────────────────────────────────────────────────────

/// Runs one spec under a named OS backend. The inherited posture is NOT a
/// provider backend: this provider refuses it rather than silently
/// reproducing an unconfined spawn — the inherited path lives in the exec
/// handler and stays byte-for-byte today's behavior.
pub(crate) struct OsBackendProvider {
    pub(crate) backend: Backend,
}

impl SandboxProvider for OsBackendProvider {
    fn run(&self, spec: SandboxSpec) -> SandboxOutcome {
        self.run_cancellable(spec, &CancellationToken::default())
    }

    fn run_cancellable(&self, spec: SandboxSpec, cancel: &CancellationToken) -> SandboxOutcome {
        let cancel = cancel.clone();
        match self.backend {
            Backend::Inherited => SandboxOutcome::refused(
                "the inherited posture is not an OS boundary; select a backend",
            ),
            Backend::SandboxExec => run_isolated(move || seatbelt_run(spec, cancel)),
            #[cfg(target_os = "linux")]
            Backend::Landlock => landlock::run_landlocked(spec, cancel),
            #[cfg(not(target_os = "linux"))]
            Backend::Landlock => SandboxOutcome::refused(
                "sandbox unavailable: landlock backend is not built on this platform",
            ),
        }
    }
}

/// The exec handler's entry into this module: one spec under one selected
/// backend, on the worker island.
pub(crate) fn exec_sandboxed(spec: SandboxSpec, backend: Backend) -> SandboxOutcome {
    OsBackendProvider { backend }.run(spec)
}

/// Spawns without waiting — the handle laws' test seam.
#[cfg(test)]
pub(crate) fn spawn_sandboxed(
    spec: SandboxSpec,
    backend: Backend,
) -> Result<RunningSandbox, SandboxOutcome> {
    match backend {
        Backend::SandboxExec => spawn_seatbelt(spec),
        _ => Err(SandboxOutcome::refused(
            "spawn_sandboxed is the sandbox-exec test seam",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes every test that touches process-global env vars.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sbx-test-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn spec(argv: &[&str], workdir: &Path, budget: Duration) -> SandboxSpec {
        SandboxSpec {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            workdir: workdir.to_path_buf(),
            budget,
        }
    }

    fn seatbelt() -> OsBackendProvider {
        OsBackendProvider {
            backend: Backend::SandboxExec,
        }
    }

    #[test]
    fn knob_vocabulary_is_closed_and_never_silently_inherits() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("BRAIN_ENGINE_SANDBOX_BACKEND");
        }
        // Unset + non-enterprise profile → the inherited posture.
        assert_eq!(selected_backend_for_exec(), Ok(None));

        for named in ["inherited", "sandbox-exec", "landlock"] {
            unsafe { std::env::set_var("BRAIN_ENGINE_SANDBOX_BACKEND", named) };
            assert_eq!(
                selected_backend_for_exec(),
                Ok(Some(Backend::from_knob(named).unwrap()))
            );
        }
        for unnamed in ["gvisor", "allow-everything", "sandbox exec"] {
            unsafe { std::env::set_var("BRAIN_ENGINE_SANDBOX_BACKEND", unnamed) };
            let err = selected_backend_for_exec()
                .expect_err("an unnamed backend must refuse, never inherit");
            assert!(err.contains("names no known backend"), "{err}");
        }
        unsafe {
            std::env::remove_var("BRAIN_ENGINE_SANDBOX_BACKEND");
        }
    }

    #[test]
    fn enterprise_profile_defaults_to_the_platform_backend() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("BRAIN_ENGINE_SANDBOX_BACKEND");
            std::env::set_var("MODEL_PROFILE", "enterprise");
        }
        let selected =
            selected_backend_for_exec().expect("enterprise selects the platform backend");
        assert_eq!(selected, Some(Backend::platform_default()));
        unsafe {
            std::env::set_var("BRAIN_ENGINE_SANDBOX_BACKEND", "sandbox-exec");
        }
        assert_eq!(
            selected_backend_for_exec(),
            Ok(Some(Backend::SandboxExec)),
            "an explicit knob outranks the profile default"
        );
        unsafe {
            std::env::remove_var("BRAIN_ENGINE_SANDBOX_BACKEND");
            std::env::remove_var("MODEL_PROFILE");
        }
    }

    #[test]
    fn profile_is_deny_default_by_construction() {
        let rendered = seatbelt_profile(Path::new("/somewhere"), Path::new("/tmp-elsewhere"));
        assert!(rendered.starts_with("(version 1)"));
        assert!(rendered.contains("(deny default)"));
        assert!(rendered.contains("(deny network*)"));
        assert!(!rendered.contains("(allow network"));
        assert!(rendered.contains("(allow file-read*)"));
        assert!(rendered.contains("(allow process-exec)"));
        // Writes are scoped: the grants name the two realized subpaths.
        let scope_lines: Vec<&str> = rendered
            .lines()
            .filter(|l| l.contains("file-write*"))
            .collect();
        assert_eq!(scope_lines.len(), 2, "exactly the two write scopes");
        assert!(scope_lines.iter().all(|l| l.contains("subpath")));
        assert!(rendered.contains("/somewhere") && rendered.contains("/tmp-elsewhere"));
    }

    #[test]
    fn profile_render_refuses_metacharacter_paths() {
        let hostile = Path::new("/tmp/evil)(allow file-write* (subpath \"/\"))");
        let err = check_profile_safe(hostile).expect_err("metacharacters must refuse");
        assert!(err.contains("metacharacter"), "{err}");
        let clean = scratch_dir("clean");
        check_profile_safe(&realized(&clean).expect("realized")).expect("clean paths render");
        let _ = std::fs::remove_dir_all(&clean);
    }

    #[test]
    fn realized_paths_law_pinned_against_symlinked_temp() {
        // The law: every profile subpath is the REALIZED path. On macOS the
        // temp roots are symlinks (/tmp → /private/tmp, /var → /private/var),
        // so the realized form must differ from the alias; on kernels without
        // the alias the two coincide.
        let dir = scratch_dir("realized");
        let realized_dir = realized(&dir).expect("realized");
        assert!(realized_dir.is_absolute());
        #[cfg(target_os = "macos")]
        {
            assert!(
                realized_dir.starts_with("/private/"),
                "macOS temp must realize under /private: {realized_dir:?}"
            );
            assert_ne!(realized_dir, dir, "the alias must not survive");
        }
        let rendered = seatbelt_profile(&realized_dir, &realized_dir);
        assert!(rendered.contains(&realized_dir.to_string_lossy().to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enforcement_decision_refuses_partial_and_absent_enforcement() {
        assert_eq!(enforcement_decision(Enforcement::Fully), Ok(()));
        for refused in [Enforcement::Partially, Enforcement::Not] {
            let err = enforcement_decision(refused)
                .expect_err("a requested boundary that cannot enforce must refuse");
            assert_eq!(err, "sandbox unavailable: landlock not fully enforced");
        }
    }

    #[test]
    fn worker_thread_panic_is_contained_as_a_typed_refusal() {
        let outcome = run_isolated(|| std::panic::panic_any("worker blew up mid-run"));
        match outcome.status {
            SandboxStatus::Refused { reason } => {
                assert!(reason.contains("panicked"), "{reason}");
                assert!(reason.contains("killed and reaped"), "{reason}");
            }
            other => assert!(
                matches!(other, SandboxStatus::Refused { .. }),
                "a contained panic must be a typed refusal, got {other:?}"
            ),
        }
        // A well-behaved worker passes through untouched.
        let ok = run_isolated(|| SandboxOutcome {
            status: SandboxStatus::Exited { code: 0 },
            stdout: b"fine".to_vec(),
            stderr: Vec::new(),
        });
        assert!(matches!(ok.status, SandboxStatus::Exited { code: 0 }));
    }

    #[test]
    fn effective_budget_intersects_and_the_default_is_the_sdk_manager_ceiling() {
        use brain_engine_sdk::hostcall::Budget;
        // The handler ceiling IS the SDK Budget default's effective timeout.
        assert_eq!(
            Budget::default().effective_timeout(),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            effective_budget(Duration::from_secs(100), Duration::from_millis(250)),
            Duration::from_millis(250)
        );
        assert_eq!(
            effective_budget(Duration::from_millis(50), Duration::from_secs(100)),
            Duration::from_millis(50)
        );
    }

    #[test]
    fn the_provider_refuses_the_inherited_posture_instead_of_faking_it() {
        let provider = OsBackendProvider {
            backend: Backend::Inherited,
        };
        let outcome = provider.run(spec(
            &["/bin/echo", "x"],
            Path::new("/"),
            Duration::from_secs(1),
        ));
        match outcome.status {
            SandboxStatus::Refused { reason } => {
                assert!(reason.contains("not an OS boundary"), "{reason}");
            }
            other => assert!(
                matches!(other, SandboxStatus::Refused { .. }),
                "inherited must never ride the OS provider: {other:?}"
            ),
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn exec_route_wraps_the_sandbox_when_selected() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("BRAIN_ENGINE_SANDBOX_BACKEND", "sandbox-exec");
            std::env::set_var("BRAIN_ENGINE_EXEC_ALLOWLIST", "/bin/echo");
        }
        let out = crate::workflow::hostcalls::exec_effect_for(
            r#"{"argv":["/bin/echo","sandboxed-hello"]}"#,
            Duration::from_secs(10),
        )
        .expect("the allowlisted echo must run under the selected backend");
        assert_eq!(out.exit_code, 0);
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("sandboxed-hello"), "{text}");
        unsafe {
            std::env::remove_var("BRAIN_ENGINE_SANDBOX_BACKEND");
            std::env::remove_var("BRAIN_ENGINE_EXEC_ALLOWLIST");
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn escape_is_process_not_thread() {
        let workdir = scratch_dir("escape-in");
        let inside = workdir.join("inside.txt");
        let outside = PathBuf::from(format!("/var/tmp/sbx-escape-{}.txt", std::process::id()));
        let provider = seatbelt();

        // Inside the workdir: the identical write succeeds.
        let ok = provider.run(spec(
            &[
                "/bin/sh",
                "-c",
                &format!("echo inside > {}", inside.display()),
            ],
            &workdir,
            Duration::from_secs(10),
        ));
        assert!(
            matches!(ok.status, SandboxStatus::Exited { code: 0 }),
            "write inside the realized workdir must succeed: {:?} / {}",
            ok.status,
            String::from_utf8_lossy(&ok.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(&inside).unwrap_or_default(),
            "inside\n"
        );

        // Outside (a path outside BOTH scopes): the identical write is
        // refused by the OS, and no file is created.
        let deny = provider.run(spec(
            &[
                "/bin/sh",
                "-c",
                &format!("echo pwned > {}", outside.display()),
            ],
            &workdir,
            Duration::from_secs(10),
        ));
        assert!(
            !matches!(deny.status, SandboxStatus::Exited { code: 0 }),
            "write outside must fail under the backend: {:?}",
            deny.status
        );
        assert!(
            !outside.exists(),
            "the refused write must not create the file"
        );

        // The control: the same outside write through the INHERITED posture
        // succeeds — today's same-user posture has no OS boundary. This is
        // the boundary the backend adds, demonstrated on one machine.
        let inherited = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("echo pwned > {}", outside.display()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("inherited control spawn");
        assert!(
            inherited.status.success(),
            "the inherited posture permits the same write (the red): {:?}",
            inherited.status
        );
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&workdir);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn sandboxed_network_is_denied() {
        let workdir = scratch_dir("escape-net");
        let outcome = seatbelt().run(spec(
            &["/usr/bin/curl", "--max-time", "3", "http://example.com/"],
            &workdir,
            Duration::from_secs(10),
        ));
        match outcome.status {
            SandboxStatus::Exited { code } => {
                assert_ne!(code, 0, "network must be denied under the profile");
            }
            other => assert!(
                matches!(other, SandboxStatus::Exited { .. }),
                "curl must exit, not be killed: {other:?}"
            ),
        }
        assert!(outcome.stdout.is_empty(), "no body may arrive");
        let _ = std::fs::remove_dir_all(&workdir);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn secret_not_in_hostcall_payload() {
        let _g = ENV_LOCK.lock().unwrap();
        const KEY_MATERIAL: &str =
            "a7f3c1e9d4b25806fa2ce91b7d304c5581ee6b0aa1f2c3d4e5f60718293a4b5c";
        unsafe { std::env::set_var("BRAIN_AUDIT_CHAIN_KEY", KEY_MATERIAL) };
        let workdir = scratch_dir("escape-secret");
        let outcome = seatbelt().run(spec(&["/usr/bin/env"], &workdir, Duration::from_secs(10)));
        unsafe { std::env::remove_var("BRAIN_AUDIT_CHAIN_KEY") };
        assert!(
            matches!(outcome.status, SandboxStatus::Exited { code: 0 }),
            "env must run: {:?}",
            outcome.status
        );
        let text = String::from_utf8_lossy(&outcome.stdout);
        assert!(
            text.contains("PATH="),
            "the child must have printed its env: {text}"
        );
        assert!(
            !text.contains(KEY_MATERIAL),
            "the key must never cross the boundary: {text}"
        );
        let _ = std::fs::remove_dir_all(&workdir);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn harness_kill_within_budget() {
        let workdir = scratch_dir("kill-budget");
        let started = Instant::now();
        let outcome = seatbelt()
            .run(spec(&["/bin/sleep", "30"], &workdir, Duration::from_millis(250)).clone());
        let elapsed = started.elapsed();
        assert_eq!(
            outcome.status,
            SandboxStatus::Killed {
                reason: KillReason::Deadline
            }
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "the child must be killed at the budget, not awaited: {elapsed:?}"
        );
        let _ = std::fs::remove_dir_all(&workdir);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn sandbox_cancel_mid_run_kills_cancelled() {
        let workdir = scratch_dir("cancel-mid");
        let cancel = CancellationToken::default();
        let running = spawn_sandboxed(
            spec(&["/bin/sleep", "30"], &workdir, Duration::from_secs(10)),
            Backend::SandboxExec,
        )
        .expect("the sleep must spawn");
        std::thread::sleep(Duration::from_millis(300));
        cancel.cancel();
        let outcome = wait_for_child(running.handle, running.budget, &cancel);
        assert_eq!(
            outcome.status,
            SandboxStatus::Killed {
                reason: KillReason::Cancelled
            }
        );
        let _ = std::fs::remove_dir_all(&workdir);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn sandbox_handle_drop_kills_and_reaps_mid_run() {
        let workdir = scratch_dir("drop-mid");
        let started = Instant::now();
        {
            let _running = spawn_sandboxed(
                spec(&["/bin/sleep", "30"], &workdir, Duration::from_secs(30)),
                Backend::SandboxExec,
            )
            .expect("the sleep must spawn");
            // Dropping mid-run: RAII kills + reaps; the block cannot outlive
            // the child.
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "the drop must kill + reap promptly, not await the child: {elapsed:?}"
        );
        let _ = std::fs::remove_dir_all(&workdir);
    }
}
