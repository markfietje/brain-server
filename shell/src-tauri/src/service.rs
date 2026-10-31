//! W-1 — the install core for the shell's SERVICE posture (posture B).
//!
//! The mechanism decision is the CLASSIC per-user LaunchAgent (operator,
//! 2026-10-31), reopened by measurement: the SMAppService plist must be a
//! static bundle asset (W-0: relative program paths EX_CONFIG, and writing
//! the plist into the Developer-ID-sealed bundle would break the app's own
//! signature), but a static plist cannot carry the three user-absolute env
//! keys the kernel requires (`AUTH_TOKEN_FILE`, `BRAIN_DATA_ROOT`,
//! `BRAIN_DB_PATH` — launchd expands nothing, the kernel expands nothing,
//! relative roots are refused). The generated LaunchAgent plist carries
//! them; the evidence lives in the wizard round's W-0 record
//! (`EXECUTION_PROMPT_INSTALL_WIZARD_2026-10-31.md` §10).
//!
//! The contract, in one place:
//!
//! 1. **One label, ours only.** `com.stewardmem.shell.service` — a NEW
//!    identity. This module's write paths name no other service; the pin
//!    reads this file's own source and refuses the operator's label
//!    outright. The launchd verbs target `gui/<uid>/<OUR LABEL>` and
//!    nothing else.
//! 2. **The program is the bundle's own sealed sidecar** — the CURRENT
//!    absolute path at install time. No copy, no staging, nothing that can
//!    drift. An app update replaces the bundle in place at the same path,
//!    so the plist's path survives every update; a moved app is repaired by
//!    re-installing (the plist is regenerated from the current path).
//! 3. **The env is the spawn allowlist.** The plist's
//!    `EnvironmentVariables` is byte-for-byte the EIGHT keys that
//!    `sidecar::command_spec` derives — one allowlist implementation, two
//!    consumers (the spawned child and the launchd agent). No secret ever
//!    rides the plist: the token travels by reference (`AUTH_TOKEN_FILE`).
//! 4. **The service root is OUTSIDE the bundle** (bundles are replaced on
//!    update): `<app_data_dir>/service` (0700), token at
//!    `<service_root>/auth-token` (0600) — the existing
//!    `prepare_auth_file` mode law, reused, not re-implemented. Logs at
//!    `<service_root>/brain-server.log`.
//! 5. **Consent before storage.** No service root is created and no plist
//!    is written unless the consent marker exists (the wizard's gate runs
//!    first and stays first). `ConsentRefused` fires before any fs write.
//! 6. **Port exclusivity is physics.** Before anything is written, the
//!    probe decides: a listener that is not this app's own child REFUSES
//!    posture B naming what was found, and the wizard offers posture C. A
//!    held port is never spawned over (the same law the sidecar obeys).
//! 7. **The version gate is the belt.** After bootstrap, the bounded wait
//!    health-checks the SERVED version against the sealed provenance; a
//!    mismatch is refused with BOTH versions named and the failed agent is
//!    booted out — a half-installed service is never left flapping.
//! 8. **Every outcome is named.** `Installed` / `AlreadyInstalled` /
//!    `PortHeld { holder }` / `VersionMismatch { expected, observed }` /
//!    `Refused { kind }` — and uninstall returns the removal list and the
//!    residue enumeration, where "we did not delete your data" is stated as
//!    an OFFER with the exact path, never a silent keep or a silent delete.
//!
//! There is no `unsafe`, no shell (`sh -c` is never built; `/bin/launchctl`
//! is exec'd directly with argv), and no HTTP client here — the probe is
//! the sidecar's own raw loopback stream.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::sidecar::{self, ContractError, KernelState, ProbeOutcome, SidecarCommand, WaitOutcome};

// ── the Tauri edges ────────────────────────────────────────────────────────
//
// The only part that knows about `AppHandle`: path resolution and the real
// injected edges. Everything above is driven by the pins hermetically.

fn service_paths(app: &tauri::AppHandle) -> Result<ServicePaths, ServiceError> {
    use tauri::Manager;
    let resolve =
        |what: &str, value: Result<PathBuf, tauri::Error>| -> Result<PathBuf, ServiceError> {
            value.map_err(|error| ServiceError::Filesystem {
                path: what.to_string(),
                reason: error.to_string(),
            })
        };
    let data_dir = resolve("app_data_dir", app.path().app_data_dir())?;
    let config_dir = resolve("app_config_dir", app.path().app_config_dir())?;
    let home = resolve("home_dir", app.path().home_dir())?;
    let resource_dir = resolve("resource_dir", app.path().resource_dir())?;
    let executable = std::env::current_exe().map_err(|error| ServiceError::Filesystem {
        path: "current_exe".to_string(),
        reason: error.to_string(),
    })?;
    Ok(ServicePaths {
        service_root: data_dir.join("service"),
        launch_agents_dir: home.join("Library").join("LaunchAgents"),
        config_dir,
        sidecar: sidecar::resolve_sidecar_path(&executable),
        resource_dir,
    })
}

/// The uid for the launchd `gui/<uid>` domain. The crate forbids unsafe and
/// std exposes no uid, so the same source the bash installer uses —
/// `/usr/bin/id -u` — is exec'd directly (no shell, argv only).
fn uid() -> Result<u32, ServiceError> {
    let output = std::process::Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .map_err(|error| ServiceError::Launchctl {
            verb: "id -u".to_string(),
            stderr: error.to_string(),
        })?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|_| ServiceError::Launchctl {
            verb: "id -u".to_string(),
            stderr: "the uid was not a number".to_string(),
        })
}

/// `service_install`: the real edges — the sidecar's probe and bounded wait,
/// the shared token custody, the real launchctl. Every law above applies
/// unchanged; the edges are the only thing that is real here.
pub fn service_install(app: &tauri::AppHandle, state: &KernelState) -> InstallDto {
    let service_uid = match uid() {
        Ok(uid) => uid,
        Err(error) => return InstallDto::refused(error),
    };
    let paths = match service_paths(app) {
        Ok(paths) => paths,
        Err(error) => return InstallDto::refused(error),
    };
    let consent = sidecar::consent_granted(&paths.config_dir);
    let mut probe_once = || {
        sidecar::probe(
            sidecar::PORT,
            std::time::Duration::from_millis(sidecar::PROBE_TIMEOUT_MS),
        )
    };
    let mut wait = |expected: &str| {
        sidecar::wait_bounded(
            || {
                sidecar::probe(
                    sidecar::PORT,
                    std::time::Duration::from_millis(sidecar::PROBE_TIMEOUT_MS),
                )
            },
            expected,
            sidecar::WAIT_MS,
            sidecar::POLL_MS,
        )
    };
    install_with_paths(
        &paths,
        state,
        consent,
        &mut probe_once,
        &mut wait,
        &mut sidecar::resolve_token,
        &mut run_launchctl,
        service_uid,
    )
}

/// `service_uninstall`: bootout ours, remove ours, enumerate. The offered
/// data removal is its own command (`service_remove_data`) — never silent.
pub fn service_uninstall(app: &tauri::AppHandle) -> UninstallDto {
    let service_uid = match uid() {
        Ok(uid) => uid,
        Err(error) => {
            return UninstallDto {
                outcome: "refused".to_string(),
                removed: Vec::new(),
                offered: Vec::new(),
                residue: Vec::new(),
                kind: Some(error.kind().to_string()),
                error: Some(error.to_string()),
            };
        }
    };
    let paths = match service_paths(app) {
        Ok(paths) => paths,
        Err(error) => {
            return UninstallDto {
                outcome: "refused".to_string(),
                removed: Vec::new(),
                offered: Vec::new(),
                residue: Vec::new(),
                kind: Some(error.kind().to_string()),
                error: Some(error.to_string()),
            };
        }
    };
    uninstall_with_paths(&paths, &mut run_launchctl, service_uid)
}

/// The OFFERED removal, executed: the caller names the exact path it was
/// offered and this deletes exactly that — after the shape check above.
pub fn service_remove_data(app: &tauri::AppHandle) -> Result<(), ServiceError> {
    let paths = service_paths(app)?;
    remove_service_data(&paths.service_root)
}

/// `service_posture`: the marker's reading, beside the machine's reality
/// where the Settings surface needs it.
pub fn posture_load(app: &tauri::AppHandle) -> PostureDto {
    // An unresolvable path is the DEFAULT posture, never an error (the HIG
    // default-first law, the same fallthrough load_posture makes).
    let default = || PostureDto {
        posture: Posture::AppManaged.as_str().to_string(),
        url: None,
        chosen: false,
    };
    service_paths(app).map_or_else(|_| default(), |paths| load_posture(&paths.config_dir))
}

/// `store_service_posture`: the choice, validated and saved. There is no
/// token parameter — posture C never receives one through this app.
pub fn posture_store(
    app: &tauri::AppHandle,
    posture: &str,
    url: Option<&str>,
) -> Result<PostureDto, ServiceError> {
    let parsed = Posture::parse(posture, url)?;
    let paths = service_paths(app)?;
    save_posture(&paths.config_dir, &parsed)?;
    Ok(PostureDto {
        posture: parsed.as_str().to_string(),
        url: match parsed {
            Posture::External { url } => Some(url),
            _ => None,
        },
        chosen: true,
    })
}

/// The service's launchd label — a NEW identity, deliberately distinct from
/// every other service on the machine. Never a path under, near, or naming
/// any other label.
pub const SERVICE_LABEL: &str = "com.stewardmem.shell.service";

/// The plist file name inside the user's `Library/LaunchAgents`.
pub const PLIST_FILE_NAME: &str = "com.stewardmem.shell.service.plist";

/// The posture marker (0600, OUTSIDE the data root — the consent marker's
/// own law, reused). Presence of the service is launchd's business; this
/// file only records WHICH posture the user chose so Settings can re-render
/// it and the gate can behave accordingly.
pub const POSTURE_FILE: &str = "posture-v1.json";

/// The bound on a posture-C URL: long enough for any loopback or LAN
/// address, short enough that a paste cannot smuggle a payload.
pub const MAX_URL_BYTES: usize = 2048;

/// The launchctl binary — exec'd directly, argv only, no shell.
const LAUNCHCTL: &str = "/bin/launchctl";

// ── the pure types ─────────────────────────────────────────────────────────

/// The three postures. `External` names a URL and NEVER a token: the
/// operator pastes their credential into the remote kernel's own auth file;
/// this app attaches and says nothing about credentials it does not hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Posture {
    AppManaged,
    BackgroundService,
    External { url: String },
}

impl Posture {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AppManaged => "app",
            Self::BackgroundService => "service",
            Self::External { .. } => "external",
        }
    }

    /// The shallow URL check a posture-C choice must pass: a bounded,
    /// absolute http(s) URL. Anything else is refused — the URL steers where
    /// the app's wire points, so a garbage URL is refused at the door.
    pub fn external(url: &str) -> Result<Self, ServiceError> {
        let url = url.trim();
        if url.is_empty() || url.len() > MAX_URL_BYTES {
            return Err(ServiceError::InvalidUrl {
                reason: "the URL is empty or beyond the length bound".to_string(),
            });
        }
        let ok = url.starts_with("http://") || url.starts_with("https://");
        let has_host = url
            .split_once("://")
            .and_then(|(_, rest)| rest.split(['/', '?', '#']).next())
            .is_some_and(|host| !host.is_empty());
        if !ok || !has_host {
            return Err(ServiceError::InvalidUrl {
                reason: "the URL must be an absolute http(s) address with a host".to_string(),
            });
        }
        Ok(Self::External {
            url: url.to_string(),
        })
    }

    pub fn parse(value: &str, url: Option<&str>) -> Result<Self, ServiceError> {
        match value {
            "app" => Ok(Self::AppManaged),
            "service" => Ok(Self::BackgroundService),
            "external" => Self::external(url.unwrap_or("")),
            other => Err(ServiceError::InvalidPosture {
                name: other.to_string(),
            }),
        }
    }
}

/// A service-posture refusal. Every variant names its path/value; none
/// carries a token, ever.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    ConsentRefused,
    InvalidPosture { name: String },
    InvalidUrl { reason: String },
    PortHeld { holder: String },
    VersionMismatch { expected: String, observed: String },
    ServiceDidNotAnswer { waited_ms: u64, last: String },
    Launchctl { verb: String, stderr: String },
    Filesystem { path: String, reason: String },
    Provenance { path: String, reason: String },
    TokenUnavailable,
    AgentStillRunning,
}

impl ServiceError {
    /// Stable machine codes for the webview — the DTO's `kind`, mapped to
    /// copy on the UI side.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ConsentRefused => "consentRefused",
            Self::InvalidPosture { .. } => "invalidPosture",
            Self::InvalidUrl { .. } => "invalidUrl",
            Self::PortHeld { .. } => "portHeld",
            Self::VersionMismatch { .. } => "versionMismatch",
            Self::ServiceDidNotAnswer { .. } => "serviceDidNotAnswer",
            Self::Launchctl { .. } => "launchctl",
            Self::Filesystem { .. } => "filesystem",
            Self::Provenance { .. } => "provenanceMissing",
            Self::TokenUnavailable => "tokenUnavailable",
            Self::AgentStillRunning => "agentStillRunning",
        }
    }

    /// The holder sentence for a refused port: what the probe found, never
    /// a guess about whose it is.
    fn holder(outcome: &ProbeOutcome) -> String {
        match outcome {
            ProbeOutcome::Probe(_) => {
                "a kernel that answers as a Brain kernel, not started by this app".to_string()
            }
            ProbeOutcome::Inconsistent { .. } => {
                "a Brain kernel whose versions disagree, not started by this app".to_string()
            }
            ProbeOutcome::Foreign => "a listener that did not answer as a Brain kernel".to_string(),
            ProbeOutcome::Unanswered => "a listener that answered nothing".to_string(),
            ProbeOutcome::NotRunning => "nothing".to_string(),
        }
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConsentRefused => write!(f, "storage consent was not granted"),
            Self::InvalidPosture { name } => write!(f, "unknown posture {name:?}"),
            Self::InvalidUrl { reason } => {
                write!(f, "the external kernel URL was refused: {reason}")
            }
            Self::PortHeld { holder } => write!(
                f,
                "the port {}:{} is held by {holder}; posture B was not installed — connect to the existing kernel instead",
                sidecar::BIND,
                sidecar::PORT
            ),
            Self::VersionMismatch { expected, observed } => write!(
                f,
                "staged app ships kernel {expected} (sealed provenance) but the service serves kernel {observed}"
            ),
            Self::ServiceDidNotAnswer { waited_ms, last } => write!(
                f,
                "the service did not answer within {waited_ms} ms; last: {last}"
            ),
            Self::Launchctl { verb, stderr } => {
                write!(f, "launchctl {verb} failed: {stderr}")
            }
            Self::Filesystem { path, reason } => {
                write!(f, "filesystem refusal at {path}: {reason}")
            }
            Self::Provenance { path, reason } => {
                write!(
                    f,
                    "kernel provenance missing or unreadable: {path} ({reason})"
                )
            }
            Self::TokenUnavailable => {
                write!(
                    f,
                    "the login keychain could not supply or store a service token"
                )
            }
            Self::AgentStillRunning => {
                write!(f, "the service agent is still running after bootout")
            }
        }
    }
}

/// The pre-write decision. `Proceed` is reachable ONLY from a free port
/// (after this app's own child was stopped by the caller's order below).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallDecision {
    Proceed,
    /// The kernel holding the port is THIS app's own child; stop it first.
    StopOurChild,
    /// Anyone else holds the port; posture B is refused and posture C is
    /// the offered alternative.
    Refuse(ServiceError),
}

/// The pure port decision. `held_but_ours` is true only when this app's
/// own child is alive AND the probe found it — anything else that holds
/// the port refuses.
pub fn decide_install(outcome: &ProbeOutcome, our_child_alive: bool) -> InstallDecision {
    match outcome {
        ProbeOutcome::NotRunning => InstallDecision::Proceed,
        _ if our_child_alive => InstallDecision::StopOurChild,
        other => InstallDecision::Refuse(ServiceError::PortHeld {
            holder: ServiceError::holder(other),
        }),
    }
}

// ── the plist generator (pure; the pins drive it) ──────────────────────────

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The LaunchAgent plist, GENERATED at install time. The env is exactly the
/// spawn allowlist's eight keys (`command_spec`'s output, verbatim), the
/// program is the bundle's own sidecar at its current absolute path, and no
/// secret rides the file — the token travels by reference.
pub fn plist_xml(label: &str, program: &Path, env: &[(String, String)], log_path: &Path) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n");
    out.push_str("<plist version=\"1.0\">\n<dict>\n");
    out.push_str(&format!(
        "    <key>Label</key>\n    <string>{}</string>\n",
        xml_escape(label)
    ));
    out.push_str("    <key>ProgramArguments</key>\n    <array>\n");
    out.push_str(&format!(
        "        <string>{}</string>\n",
        xml_escape(&program.display().to_string())
    ));
    out.push_str("    </array>\n");
    out.push_str("    <key>EnvironmentVariables</key>\n    <dict>\n");
    for (name, value) in env {
        out.push_str(&format!(
            "        <key>{}</key>\n        <string>{}</string>\n",
            xml_escape(name),
            xml_escape(value)
        ));
    }
    out.push_str("    </dict>\n");
    out.push_str("    <key>KeepAlive</key>\n    <true/>\n");
    out.push_str("    <key>RunAtLoad</key>\n    <true/>\n");
    let log = xml_escape(&log_path.display().to_string());
    out.push_str(&format!(
        "    <key>StandardOutPath</key>\n    <string>{log}</string>\n"
    ));
    out.push_str(&format!(
        "    <key>StandardErrorPath</key>\n    <string>{log}</string>\n"
    ));
    out.push_str("</dict>\n</plist>\n");
    out
}

/// The spawn allowlist for the SERVICE kernel: the same eight keys the
/// app-spawned child gets, pointed at the service root. One allowlist
/// implementation, two consumers.
pub fn service_command_spec(sidecar: &Path, service_root: &Path) -> SidecarCommand {
    sidecar::command_spec(
        sidecar,
        service_root,
        &service_root.join(sidecar::AUTH_FILE),
    )
}

// ── the launchd edge (thin; the pins record it) ────────────────────────────

/// What a launchctl invocation returned: `ok` is the exit status, `stderr`
/// the diagnostic a refusal names.
pub struct LaunchctlResult {
    pub ok: bool,
    pub stderr: String,
}

/// The injected runner: the real edge execs `/bin/launchctl` with argv; the
/// pins record the calls without touching the machine's launchd.
pub type LaunchctlRun<'a> = &'a mut dyn FnMut(&[String]) -> Result<LaunchctlResult, String>;

/// The real runner: `/bin/launchctl` with argv. No shell, ever.
pub fn run_launchctl(args: &[String]) -> Result<LaunchctlResult, String> {
    let output = std::process::Command::new(LAUNCHCTL)
        .args(args)
        .output()
        .map_err(|error| format!("could not exec {LAUNCHCTL}: {error}"))?;
    Ok(LaunchctlResult {
        ok: output.status.success(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

/// `gui/<uid>/<label>` args for OUR label only. The label is a constant of
/// this module; a caller cannot steer it.
fn bootout_args(uid: u32) -> Vec<String> {
    vec!["bootout".to_string(), format!("gui/{uid}/{SERVICE_LABEL}")]
}

fn bootstrap_args(uid: u32, plist: &Path) -> Vec<String> {
    vec![
        "bootstrap".to_string(),
        format!("gui/{uid}"),
        plist.display().to_string(),
    ]
}

// ── paths ──────────────────────────────────────────────────────────────────

/// Every path the service posture needs, resolved once. `launch_agents_dir`
/// is the user's own `~/Library/LaunchAgents` (user-writable; no admin, no
/// root, no shared location).
#[derive(Debug, Clone)]
pub struct ServicePaths {
    pub service_root: PathBuf,
    pub launch_agents_dir: PathBuf,
    pub config_dir: PathBuf,
    pub sidecar: PathBuf,
    pub resource_dir: PathBuf,
}

impl ServicePaths {
    pub fn plist_path(&self) -> PathBuf {
        self.launch_agents_dir.join(PLIST_FILE_NAME)
    }

    pub fn auth_file(&self) -> PathBuf {
        self.service_root.join(sidecar::AUTH_FILE)
    }

    pub fn log_path(&self) -> PathBuf {
        self.service_root.join(sidecar::LOG_FILE)
    }
}

// ── the install edge ───────────────────────────────────────────────────────

/// The bounded start reuses the sidecar's wait (same port, same probe, same
/// poll cadence) — injected so the pins stay hermetic.
pub type WaitFn<'a> = &'a mut dyn FnMut(&str) -> WaitOutcome;

/// The named install outcome (the R129 law: never "saved" over a failure).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallDto {
    pub outcome: String,
    pub service_label: Option<String>,
    pub plist_path: Option<String>,
    /// True when the app's own app-managed kernel was stopped to free the
    /// port (the A→B transition's first named fact).
    pub stopped_app_kernel: bool,
    pub holder: Option<String>,
    pub expected: Option<String>,
    pub observed: Option<String>,
    pub kind: Option<String>,
    pub error: Option<String>,
}

impl InstallDto {
    pub(crate) fn refused(error: ServiceError) -> Self {
        let (holder, expected, observed) = match &error {
            ServiceError::PortHeld { holder } => (Some(holder.clone()), None, None),
            ServiceError::VersionMismatch { expected, observed } => {
                (None, Some(expected.clone()), Some(observed.clone()))
            }
            _ => (None, None, None),
        };
        Self {
            outcome: "refused".to_string(),
            service_label: Some(SERVICE_LABEL.to_string()),
            plist_path: None,
            stopped_app_kernel: false,
            holder,
            expected,
            observed,
            kind: Some(error.kind().to_string()),
            error: Some(error.to_string()),
        }
    }

    fn installed(plist_path: &Path, version: &str) -> Self {
        Self {
            outcome: "installed".to_string(),
            service_label: Some(SERVICE_LABEL.to_string()),
            plist_path: Some(plist_path.display().to_string()),
            stopped_app_kernel: false,
            holder: None,
            expected: Some(version.to_string()),
            observed: Some(version.to_string()),
            kind: None,
            error: None,
        }
    }
}

/// The service root, 0700 — created ONLY here, ONLY after consent.
fn prepare_service_root(root: &Path) -> Result<(), ServiceError> {
    fs::create_dir_all(root).map_err(|error| ServiceError::Filesystem {
        path: root.display().to_string(),
        reason: error.to_string(),
    })?;
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).map_err(|error| {
        ServiceError::Filesystem {
            path: root.display().to_string(),
            reason: error.to_string(),
        }
    })
}

/// The plist write: atomic (sibling temp, 0644, rename over the target —
/// a launchd plist carries no secrets, so the mode follows the convention
/// of every other plist in this directory).
fn write_plist_atomic(plist_path: &Path, contents: &str) -> Result<(), ServiceError> {
    let temp = plist_path.with_extension("plist.tmp");
    let written = (|| -> Result<(), ServiceError> {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o644)
            .open(&temp)
            .map_err(|error| ServiceError::Filesystem {
                path: temp.display().to_string(),
                reason: error.to_string(),
            })?;
        file.write_all(contents.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| ServiceError::Filesystem {
                path: temp.display().to_string(),
                reason: error.to_string(),
            })?;
        drop(file);
        fs::rename(&temp, plist_path).map_err(|error| ServiceError::Filesystem {
            path: plist_path.display().to_string(),
            reason: error.to_string(),
        })?;
        Ok(())
    })();
    if written.is_err() {
        // The temp file never outlives a failed write.
        fs::remove_file(&temp).ok();
    }
    written
}

/// The install path with everything resolved and injected. The Tauri edge
/// below is the only part that knows about `AppHandle`; this half a pin
/// drives with temp directories and recorded launchctl calls.
///
/// The ORDER is the law: consent → probe → (stop ours | refuse) →
/// provenance → root (0700) → token → auth file (0600) → plist → bootstrap
/// → bounded wait → named outcome. A refusal anywhere leaves NO plist and
/// NO bootstrap, and the wait's failure boots the agent back out so the
/// machine is never left with a flapping half-install.
#[allow(clippy::too_many_arguments)]
pub fn install_with_paths(
    paths: &ServicePaths,
    state: &KernelState,
    consent_granted: bool,
    probe_once: &mut dyn FnMut() -> ProbeOutcome,
    wait: WaitFn<'_>,
    resolve_token: &mut dyn FnMut() -> Result<String, ContractError>,
    run: LaunchctlRun<'_>,
    uid: u32,
) -> InstallDto {
    // 1. Consent before storage — before ANY write, before the root exists.
    if !consent_granted {
        return InstallDto::refused(ServiceError::ConsentRefused);
    }

    // 2. The port decision (physics before filesystem).
    let outcome = probe_once();
    let our_child_alive = sidecar::child_alive(state);
    let mut stopped_app_kernel = false;
    match decide_install(&outcome, our_child_alive) {
        InstallDecision::Refuse(error) => return InstallDto::refused(error),
        InstallDecision::Proceed => {}
        InstallDecision::StopOurChild => {
            sidecar::shutdown(state);
            stopped_app_kernel = true;
        }
    }

    // 3. The provenance gate: the service runs the SAME sealed sidecar the
    //    app ships, so the expected version comes from the seal, never a
    //    literal (the R127 rule, reused).
    let manifest = paths.resource_dir.join(sidecar::PROVENANCE_REL);
    let provenance = match sidecar::load_provenance(&manifest) {
        Ok(provenance) => provenance,
        Err(error) => {
            return InstallDto::refused(ServiceError::Provenance {
                path: manifest.display().to_string(),
                reason: error.to_string(),
            });
        }
    };

    // 4. The service root + token + auth file (the 0700/0600 mode laws ride
    //    on the reused prepare_auth_file).
    if let Err(error) = prepare_service_root(&paths.service_root) {
        return InstallDto::refused(error);
    }
    let token = match resolve_token() {
        Ok(token) => token,
        Err(_) => return InstallDto::refused(ServiceError::TokenUnavailable),
    };
    if let Err(error) = sidecar::prepare_auth_file(&paths.service_root, &token) {
        return InstallDto::refused(match error {
            ContractError::InsecureAuthFile { path, mode } => ServiceError::Filesystem {
                path,
                reason: format!("auth file is mode {mode:04o}"),
            },
            ContractError::TokenUnavailable => ServiceError::TokenUnavailable,
            other => ServiceError::Filesystem {
                path: paths.auth_file().display().to_string(),
                reason: other.to_string(),
            },
        });
    }

    // 5. The plist: generated from the CURRENT bundle path, atomically. The
    //    LaunchAgents directory exists on every macOS home, but a missing
    //    directory is created (a named write-set location), never assumed.
    if let Err(error) = fs::create_dir_all(&paths.launch_agents_dir) {
        return InstallDto::refused(ServiceError::Filesystem {
            path: paths.launch_agents_dir.display().to_string(),
            reason: error.to_string(),
        });
    }
    let spec = service_command_spec(&paths.sidecar, &paths.service_root);
    let xml = plist_xml(SERVICE_LABEL, &paths.sidecar, &spec.env, &paths.log_path());
    let plist_path = paths.plist_path();
    if let Err(error) = write_plist_atomic(&plist_path, &xml) {
        return InstallDto::refused(error);
    }

    // 6. Idempotency: an agent ALREADY bootstrapped under our label is
    //    named as such — the plist above was still rewritten (the repair
    //    semantics: re-installing fixes a stale program path), but no
    //    second bootstrap is attempted and no refusal is invented.
    let print_args = vec!["print".to_string(), format!("gui/{uid}/{SERVICE_LABEL}")];
    let already = run(&print_args).is_ok_and(|result| result.ok);
    if already {
        return InstallDto {
            outcome: "alreadyInstalled".to_string(),
            service_label: Some(SERVICE_LABEL.to_string()),
            plist_path: Some(plist_path.display().to_string()),
            stopped_app_kernel,
            holder: None,
            expected: None,
            observed: None,
            kind: None,
            error: None,
        };
    }

    // 7. Bootstrap the agent (the verbs install-service.sh proves daily).
    let bootstrap = bootstrap_args(uid, &plist_path);
    match run(&bootstrap) {
        Ok(result) if result.ok => {}
        Ok(result) => {
            return InstallDto::refused(ServiceError::Launchctl {
                verb: "bootstrap".to_string(),
                stderr: result.stderr,
            });
        }
        Err(error) => {
            return InstallDto::refused(ServiceError::Launchctl {
                verb: "bootstrap".to_string(),
                stderr: error,
            });
        }
    }

    // 8. The version gate is the belt: the SERVED version against the seal,
    //    both named on a mismatch, the failed agent booted out (best-effort)
    //    so the machine is not left flapping on a dead program path.
    let bootout = |run: LaunchctlRun<'_>| {
        let _ = run(&bootout_args(uid));
    };
    let refused = |error: ServiceError| {
        let mut dto = InstallDto::refused(error);
        dto.stopped_app_kernel = stopped_app_kernel;
        dto
    };
    match wait(&provenance.x_api_version) {
        WaitOutcome::Ready { version } => {
            let mut dto = InstallDto::installed(&plist_path, &version);
            dto.stopped_app_kernel = stopped_app_kernel;
            dto
        }
        WaitOutcome::Refused(ContractError::VersionMismatch { expected, observed }) => {
            bootout(run);
            refused(ServiceError::VersionMismatch { expected, observed })
        }
        WaitOutcome::Refused(error) => {
            bootout(run);
            refused(ServiceError::ServiceDidNotAnswer {
                waited_ms: 0,
                last: error.to_string(),
            })
        }
        WaitOutcome::Timeout { waited_ms, last } => {
            bootout(run);
            refused(ServiceError::ServiceDidNotAnswer { waited_ms, last })
        }
    }
}

// ── the uninstall edge ─────────────────────────────────────────────────────

/// The named uninstall outcome. `removed` is what this call removed;
/// `offered` is the data this call did NOT touch, with its exact path, for
/// the UI to offer (never silent, never automatic); `residue` is the full
/// enumeration of what still exists — the claim the pins hold.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallDto {
    pub outcome: String,
    pub removed: Vec<String>,
    pub offered: Vec<String>,
    pub residue: Vec<String>,
    pub kind: Option<String>,
    pub error: Option<String>,
}

impl UninstallDto {
    fn not_installed() -> Self {
        Self {
            outcome: "notInstalled".to_string(),
            removed: Vec::new(),
            offered: Vec::new(),
            residue: Vec::new(),
            kind: None,
            error: None,
        }
    }
}

/// What still exists of the service, enumerated. The list is FIXED — these
/// three locations and nothing else; no scan, no discovery, nothing that
/// could ever name another service's paths.
pub fn enumerate_residue(paths: &ServicePaths) -> Vec<String> {
    let mut residue = Vec::new();
    if paths.plist_path().exists() {
        residue.push(paths.plist_path().display().to_string());
    }
    if paths.service_root.exists() {
        residue.push(paths.service_root.display().to_string());
    }
    residue
}

/// Uninstall: bootout OUR label, remove OUR plist, enumerate. The data root
/// is OFFERED (returned in `offered` with its exact path), never silently
/// deleted — the W-4 law. Clearing the offered data is the separate
/// `remove_service_data` edge, itself a named act.
pub fn uninstall_with_paths(paths: &ServicePaths, run: LaunchctlRun<'_>, uid: u32) -> UninstallDto {
    let plist_path = paths.plist_path();
    let installed = plist_path.exists();

    // Bootout by OUR label only; "not bootstrapped" is fine (the agent may
    // have died or been removed by hand — uninstalling a ghost is success).
    let mut removed = Vec::new();
    match run(&bootout_args(uid)) {
        Ok(result) if result.ok => removed.push(format!("the launchd agent ({SERVICE_LABEL})")),
        Ok(_) => {}
        Err(error) => {
            return UninstallDto {
                outcome: "refused".to_string(),
                removed,
                offered: Vec::new(),
                residue: enumerate_residue(paths),
                kind: Some("launchctl".to_string()),
                error: Some(format!("launchctl bootout failed: {error}")),
            };
        }
    }

    // The plist is OURS (the constant name); a missing file is fine.
    match fs::remove_file(&plist_path) {
        Ok(()) => removed.push(plist_path.display().to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return UninstallDto {
                outcome: "refused".to_string(),
                removed,
                offered: Vec::new(),
                residue: enumerate_residue(paths),
                kind: Some("filesystem".to_string()),
                error: Some(
                    ServiceError::Filesystem {
                        path: plist_path.display().to_string(),
                        reason: error.to_string(),
                    }
                    .to_string(),
                ),
            };
        }
    }

    if !installed && removed.is_empty() {
        return UninstallDto::not_installed();
    }

    let offered = if paths.service_root.exists() {
        vec![paths.service_root.display().to_string()]
    } else {
        Vec::new()
    };
    UninstallDto {
        outcome: "uninstalled".to_string(),
        removed,
        offered,
        residue: enumerate_residue(paths),
        kind: None,
        error: None,
    }
}

/// The OFFERED data removal — a named act with the path in its outcome.
/// Refuses when the path is not exactly the service root (the write-set
/// law: this function deletes ONE directory, the module's own).
pub fn remove_service_data(service_root: &Path) -> Result<(), ServiceError> {
    let expected_suffix = Path::new("service");
    if service_root.file_name() != Some(expected_suffix.as_os_str()) {
        return Err(ServiceError::Filesystem {
            path: service_root.display().to_string(),
            reason: "refusing to remove a path that is not the service root".to_string(),
        });
    }
    fs::remove_dir_all(service_root).map_err(|error| ServiceError::Filesystem {
        path: service_root.display().to_string(),
        reason: error.to_string(),
    })
}

// ── the posture marker ─────────────────────────────────────────────────────

/// The posture DTO the Settings surface renders. Facts, never guesses: the
/// marker is the CHOICE; launchd and the probe are the REALITY, reported
/// beside it. `chosen` distinguishes a recorded choice from the default —
/// the gate shows the one-time decision only when nothing was chosen yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostureDto {
    pub posture: String,
    pub url: Option<String>,
    pub chosen: bool,
}

/// Read the posture marker. Absent/unreadable is the DEFAULT posture
/// (AppManaged — today's behavior), never an error: the app is fully
/// usable without answering anything (the HIG's default-first law).
pub fn load_posture(config_dir: &Path) -> PostureDto {
    let marker = config_dir.join(POSTURE_FILE);
    let default = || PostureDto {
        posture: Posture::AppManaged.as_str().to_string(),
        url: None,
        chosen: false,
    };
    let Ok(bytes) = fs::read(&marker) else {
        return default();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return default();
    };
    let posture = value
        .get("posture")
        .and_then(|v| v.as_str())
        .unwrap_or("app");
    let url = value
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    match Posture::parse(posture, url.as_deref()) {
        Ok(Posture::External { url }) => PostureDto {
            posture: "external".to_string(),
            url: Some(url),
            chosen: true,
        },
        Ok(Posture::BackgroundService) => PostureDto {
            posture: "service".to_string(),
            url: None,
            chosen: true,
        },
        Ok(Posture::AppManaged) => PostureDto {
            posture: "app".to_string(),
            url: None,
            chosen: true,
        },
        Err(_) => default(),
    }
}

/// Save the posture marker (0600, atomic rename — the rotation file's own
/// discipline). The external posture carries the URL and NOTHING else: the
/// command surface has no token parameter, so no caller can put one there.
pub fn save_posture(config_dir: &Path, posture: &Posture) -> Result<(), ServiceError> {
    fs::create_dir_all(config_dir).map_err(|error| ServiceError::Filesystem {
        path: config_dir.display().to_string(),
        reason: error.to_string(),
    })?;
    fs::set_permissions(config_dir, fs::Permissions::from_mode(0o700)).map_err(|error| {
        ServiceError::Filesystem {
            path: config_dir.display().to_string(),
            reason: error.to_string(),
        }
    })?;
    let (name, url) = match posture {
        Posture::AppManaged => ("app", None),
        Posture::BackgroundService => ("service", None),
        Posture::External { url } => ("external", Some(url.clone())),
    };
    let body = match &url {
        Some(url) => format!(
            "{{\"posture\":\"{name}\",\"url\":{}}}\n",
            serde_json::to_string(url).unwrap_or_else(|_| "\"\"".to_string())
        ),
        None => format!("{{\"posture\":\"{name}\"}}\n"),
    };
    let path = config_dir.join(POSTURE_FILE);
    let temp = config_dir.join(format!(".{POSTURE_FILE}.tmp"));
    let written = (|| -> Result<(), ServiceError> {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|error| ServiceError::Filesystem {
                path: temp.display().to_string(),
                reason: error.to_string(),
            })?;
        file.write_all(body.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| ServiceError::Filesystem {
                path: temp.display().to_string(),
                reason: error.to_string(),
            })?;
        drop(file);
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600)).map_err(|error| {
            ServiceError::Filesystem {
                path: temp.display().to_string(),
                reason: error.to_string(),
            }
        })?;
        fs::rename(&temp, &path).map_err(|error| ServiceError::Filesystem {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
        Ok(())
    })();
    if written.is_err() {
        fs::remove_file(&temp).ok();
    }
    written
}
